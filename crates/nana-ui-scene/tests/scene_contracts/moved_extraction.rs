//! A node layout only moved reuses the record the scene holds for it
//! (`UiWorld::extract_nodes_reusing`). Whatever else changed in the same
//! frame -- its colour, its text, the theme -- must still reach the scene:
//! after every frame, each record the scene holds is what a cold
//! extraction gives.

use std::sync::Arc;

use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, SemanticColorRole, ThemeAppearance};
use nana_ui_runtime::{
    DocumentId, LayoutViewport, MeasureTextShaper, MutationQueue, NodeKind, NodeStyle,
    StableNodeId, TextContent,
};
use nana_ui_scene::{AffineTransform, RuntimeDocument, SceneRect, UiScene};

const ROWS: usize = 8;
const ROOT: u64 = 1;
const COLUMN: u64 = 2;

fn id(raw: u64) -> StableNodeId {
    StableNodeId::new(raw).unwrap()
}

fn row_id(row: usize) -> StableNodeId {
    id(10 + row as u64 * 3)
}

fn label_id(row: usize) -> StableNodeId {
    id(11 + row as u64 * 3)
}

fn chip_id(row: usize) -> StableNodeId {
    id(12 + row as u64 * 3)
}

fn column_style(padding: f32) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(LayoutStyle {
            width: Some(LengthSpec::Px(300.0)),
            height: Some(LengthSpec::Fill),
            direction: Some(FlexDirection::Column),
            padding_top: Some(LengthSpec::Px(padding)),
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    }
}

fn row_style() -> Arc<LayoutStyle> {
    Arc::new(LayoutStyle {
        width: Some(LengthSpec::Px(300.0)),
        height: Some(LengthSpec::Px(20.0)),
        direction: Some(FlexDirection::Row),
        ..LayoutStyle::default()
    })
}

fn label_style(foreground: SemanticColorRole) -> NodeStyle {
    NodeStyle {
        foreground: Some(foreground),
        ..NodeStyle::default()
    }
}

fn chip_style(background: SemanticColorRole) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(LayoutStyle {
            width: Some(LengthSpec::Px(12.0)),
            height: Some(LengthSpec::Px(12.0)),
            ..LayoutStyle::default()
        }),
        background: Some(background),
        ..NodeStyle::default()
    }
}

/// Root → column → rows × (row → label, chip). Moving the column's
/// padding moves every row, and every label and chip with its row.
fn document() -> RuntimeDocument {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let mut queue = MutationQueue::new();
    queue.create(id(ROOT), document, NodeKind::Document);
    queue.create(
        id(COLUMN),
        document,
        NodeKind::Element { tag: "div".into() },
    );
    queue.insert(id(ROOT), id(COLUMN), None);
    queue.set_style(id(COLUMN), column_style(0.0));
    for row in 0..ROWS {
        queue.create(
            row_id(row),
            document,
            NodeKind::Element { tag: "div".into() },
        );
        queue.create(label_id(row), document, NodeKind::Text);
        queue.create(
            chip_id(row),
            document,
            NodeKind::Element { tag: "div".into() },
        );
        queue.insert(id(COLUMN), row_id(row), None);
        queue.insert(row_id(row), label_id(row), None);
        queue.insert(row_id(row), chip_id(row), None);
        queue.set_text(
            label_id(row),
            TextContent {
                value: format!("row {row}").into(),
            },
        );
        queue.set_style(
            row_id(row),
            NodeStyle {
                layout: row_style(),
                ..NodeStyle::default()
            },
        );
        queue.set_style(label_id(row), label_style(SemanticColorRole::Text));
        queue.set_style(chip_id(row), chip_style(SemanticColorRole::Accent));
    }
    runtime.context_mut().commit_mutations(queue).unwrap();
    runtime
}

fn flush(runtime: &mut RuntimeDocument) {
    runtime
        .flush(LayoutViewport::new(300.0, 800.0), &mut MeasureTextShaper)
        .unwrap();
}

/// `rect` through `transform`, as its bounding box.
fn drawn(rect: SceneRect, transform: AffineTransform) -> [f32; 4] {
    let [a, b, c, d, e, f] = transform.0;
    let corners = [
        (rect.x, rect.y),
        (rect.x + rect.width, rect.y),
        (rect.x, rect.y + rect.height),
        (rect.x + rect.width, rect.y + rect.height),
    ]
    .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
    let left = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let top = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let right = corners
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = corners
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max);
    [left, top, right, bottom]
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 1e-3)
}

/// Every record the scene holds is a cold extraction of its node, and it
/// paints what a scene built from a cold extraction paints, to the bit.
///
/// A record a layout move kept -- the root of the moved subtree and every
/// record under it -- is in its node's own coordinates, as a cold one is, and
/// is drawn at its node's origin, summed down its frames. So every primitive
/// is the cold one, put at the cold origin, through the cold transform, cut
/// by the cold clips: a painter that snaps the transform's translation and
/// takes text phase from where the geometry lands draws the cold pixels.
///
/// The culling half holds with a popover open too: its trigger is outside
/// the fade its hanging surface opens with, so the retained index bounds the
/// trigger as a cold one does.
fn assert_scene_is_cold(runtime: &RuntimeDocument, frame: &str) {
    let world = runtime.context().world();
    let document = DocumentId::new(1).unwrap();
    let scene = runtime.scene();
    let mut cold = UiScene::new();
    cold.apply_delta(world.extract_document(document), []);
    let ids: Vec<_> = scene.primitives().map(|primitive| primitive.id).collect();
    let cold_ids: Vec<_> = cold.primitives().map(|primitive| primitive.id).collect();
    assert_eq!(
        ids, cold_ids,
        "{frame}: the scene paints other primitives than a cold one"
    );
    let rect_bits = |rect: SceneRect| [rect.x, rect.y, rect.width, rect.height].map(f32::to_bits);
    let transform_bits =
        |transform: AffineTransform| (transform.0.map(f32::to_bits), transform.1.map(f32::to_bits));
    for (held, fresh) in scene.primitives().zip(cold.primitives()) {
        let (draw, cold_draw) = (
            scene.draw_primitive(held.id).unwrap(),
            cold.draw_primitive(fresh.id).unwrap(),
        );
        assert!(
            near(
                drawn(draw.placed_bounds(), draw.transform),
                drawn(cold_draw.placed_bounds(), cold_draw.transform)
            ),
            "{frame}: {:?} draws at {:?}, a cold scene at {:?}",
            held.id,
            drawn(draw.placed_bounds(), draw.transform),
            drawn(cold_draw.placed_bounds(), cold_draw.transform),
        );
        assert_eq!(
            draw.origin.map(f32::to_bits),
            cold_draw.origin.map(f32::to_bits),
            "{frame}: {:?} is put at {:?}, a cold one at {:?}",
            held.id,
            draw.origin,
            cold_draw.origin
        );
        // A translation is what a painter puts on the grid, and the same
        // one to the bit. Anything else is rebased through an inverse, an ulp
        // off a fresh composition whether or not anything moved.
        let translation = |transform: AffineTransform| {
            transform.0[..4] == [1.0, 0.0, 0.0, 1.0] && transform.1 == [0.0, 0.0]
        };
        if translation(cold_draw.transform) {
            assert_eq!(
                transform_bits(draw.transform),
                transform_bits(cold_draw.transform),
                "{frame}: {:?} is drawn through {:?}, a cold one through {:?}",
                held.id,
                draw.transform,
                cold_draw.transform
            );
        } else {
            assert!(
                draw.transform
                    .0
                    .iter()
                    .zip(cold_draw.transform.0)
                    .all(|(a, b)| (a - b).abs() <= 1e-3),
                "{frame}: {:?} is drawn through {:?}, a cold one through {:?}",
                held.id,
                draw.transform,
                cold_draw.transform
            );
        }
        assert_eq!(
            draw.clips.len(),
            cold_draw.clips.len(),
            "{frame}: {:?} clips",
            held.id
        );
        for (clip, cold_clip) in draw.clips.iter().zip(cold_draw.clips.iter()) {
            assert_eq!(
                rect_bits(clip.bounds),
                rect_bits(cold_clip.bounds),
                "{frame}: {:?} is cut by {clip:?}, a cold one by {cold_clip:?}",
                held.id
            );
            assert!(
                near(
                    drawn(clip.bounds, clip.transform),
                    drawn(cold_clip.bounds, cold_clip.transform)
                ) && (!translation(cold_clip.transform)
                    || transform_bits(clip.transform) == transform_bits(cold_clip.transform)),
                "{frame}: {:?} is cut by {clip:?}, a cold one by {cold_clip:?}",
                held.id
            );
        }
        // Where it draws and what cuts it are checked above: a move or a
        // scroll rebases both without rebuilding the primitive.
        let mut held = held.clone();
        held.clips = fresh.clips.clone();
        held.transform = fresh.transform;
        assert_eq!(
            &held, fresh,
            "{frame}: {:?} is not a cold primitive",
            held.id
        );
    }
    // What a painter culls to: a scene that keeps its culling bounds through
    // moves (it built them on the first frame, below) answers what a fresh
    // one does, at the viewport edges included.
    for viewport in [
        SceneRect {
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 800.0,
        },
        SceneRect {
            x: 0.0,
            y: 60.0,
            width: 300.0,
            height: 40.0,
        },
        SceneRect {
            x: 40.0,
            y: 133.0,
            width: 20.0,
            height: 7.0,
        },
    ] {
        assert_eq!(
            scene.visible_operations(viewport).unwrap(),
            cold.visible_operations(viewport).unwrap(),
            "{frame}: a painter culling to {viewport:?} draws other primitives than in a cold scene"
        );
    }
    for node in world.document_order(document) {
        let cold = world.extract_nodes(&[node]);
        let (Some(held), Some(cold)) = (scene.extracted_node(node), cold.first()) else {
            assert_eq!(
                scene.extracted_node(node).is_some(),
                !cold.is_empty(),
                "{frame}: {node:?} is extracted in one scene only"
            );
            continue;
        };
        let placed = scene.node_layout_bounds(node).unwrap();
        assert_eq!(
            [placed.x, placed.y, placed.width, placed.height].map(f32::to_bits),
            [
                cold.layout.x,
                cold.layout.y,
                cold.layout.width,
                cold.layout.height
            ]
            .map(f32::to_bits),
            "{frame}: {node:?} stands other than where a cold extraction does"
        );
        let mut held = held.clone();
        held.layout = cold.layout;
        assert_eq!(
            &held, cold,
            "{frame}: the scene's record for {node:?} is not a cold extraction"
        );
    }
}

#[test]
fn a_moved_node_changed_in_the_same_frame_reaches_the_scene() {
    type Edit = (&'static str, fn(&mut MutationQueue, bool));
    let edits: [Edit; 5] = [
        ("move only", |_, _| {}),
        ("label colour", |queue, on| {
            let role = if on {
                SemanticColorRole::Accent
            } else {
                SemanticColorRole::Text
            };
            queue.set_style(label_id(2), label_style(role));
        }),
        ("chip fill", |queue, on| {
            let role = if on {
                SemanticColorRole::Danger
            } else {
                SemanticColorRole::Accent
            };
            queue.set_style(chip_id(5), chip_style(role));
        }),
        ("same-length text", |queue, on| {
            let value = if on { "wor 3" } else { "row 3" };
            queue.set_text(
                label_id(3),
                TextContent {
                    value: value.into(),
                },
            );
        }),
        ("theme", |queue, on| {
            queue.set_preset_theme(if on {
                ThemeAppearance::Dark
            } else {
                ThemeAppearance::Light
            });
        }),
    ];
    let mut runtime = document();
    flush(&mut runtime);
    assert_scene_is_cold(&runtime, "first frame");
    let mut padding = 0.0;
    for (name, edit) in edits {
        for on in [true, false] {
            padding += 3.0;
            let mut queue = MutationQueue::new();
            queue.set_style(id(COLUMN), column_style(padding));
            edit(&mut queue, on);
            runtime.context_mut().commit_mutations(queue).unwrap();
            flush(&mut runtime);
            assert_scene_is_cold(&runtime, name);
        }
    }
}

/// A button below a list moves when a row is inserted above it: its label
/// must move with its plate.
#[test]
fn a_button_moved_by_an_inserted_row_paints_like_a_cold_scene() {
    use nana_ui_runtime::Button;
    let document = DocumentId::new(1).unwrap();
    let mut runtime = self::document();
    let button = runtime
        .context_mut()
        .create_detached_component(document, Button::new("选择范围"))
        .unwrap()
        .stable_id();
    // `actions_side_by_side`: a wrapping row of filling width, inside another.
    let wrap_row = || NodeStyle {
        layout: Arc::new(LayoutStyle {
            width: Some(LengthSpec::Fill),
            direction: Some(FlexDirection::Row),
            flex_wrap: nana_ui_core::FlexWrap::Wrap,
            align_items: nana_ui_core::AlignSpec::Center,
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    };
    let mut queue = MutationQueue::new();
    for (bar, parent) in [(id(800), id(COLUMN)), (id(801), id(800))] {
        queue.create(bar, document, NodeKind::Element { tag: "div".into() });
        queue.insert(parent, bar, None);
        queue.set_style(bar, wrap_row());
    }
    queue.insert(id(801), button, None);
    runtime.context_mut().commit_mutations(queue).unwrap();
    flush(&mut runtime);
    assert_scene_is_cold(&runtime, "first frame");
    for extra in 0..3u64 {
        let row = id(900 + extra);
        let mut queue = MutationQueue::new();
        queue.create(row, document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(COLUMN), row, Some(row_id(0)));
        queue.set_style(
            row,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(300.0)),
                    height: Some(LengthSpec::Px(20.0)),
                    ..LayoutStyle::default()
                }),
                background: Some(SemanticColorRole::Accent),
                ..NodeStyle::default()
            },
        );
        runtime.context_mut().commit_mutations(queue).unwrap();
        flush(&mut runtime);
        assert_scene_is_cold(&runtime, "row inserted");
    }
}

fn boxed(layout: LayoutStyle, background: Option<SemanticColorRole>) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        background,
        ..NodeStyle::default()
    }
}

fn px(value: f32) -> Option<LengthSpec> {
    Some(LengthSpec::Px(value))
}

/// The rows below the column's padding move under a transformed column, and
/// what they hold reads more than its own offset: a `fixed` box and an
/// absolute one whose containing block is the page stay where they are, an
/// absolute one measured from its row's grandparent frame moves with it, a
/// scrolled container and the sticky box in it move with their row, and so
/// do a text field and a button. After every frame the scene paints what a
/// cold one does.
#[test]
fn a_moved_subtree_that_reads_the_page_paints_like_a_cold_scene() {
    use nana_ui_core::{OverflowSpec, PaintTransform, PositionSpec};
    use nana_ui_runtime::{Button, ScrollOffset, TextInput};
    let document_id = DocumentId::new(1).unwrap();
    let mut runtime = document();
    let mut queue = MutationQueue::new();
    let element = |queue: &mut MutationQueue, node: u64, parent: StableNodeId, style| {
        queue.create(
            id(node),
            document_id,
            NodeKind::Element { tag: "div".into() },
        );
        queue.insert(parent, id(node), None);
        queue.set_style(id(node), style);
    };
    let column_style = |padding: f32| {
        let mut style = self::column_style(padding);
        let layout = Arc::make_mut(&mut style.layout);
        layout.transform = Some(PaintTransform {
            a: 0.9,
            b: 0.1,
            c: -0.1,
            d: 0.9,
            e: 4.0,
            f: 2.0,
        });
        style
    };
    queue.set_style(id(COLUMN), column_style(0.0));
    element(
        &mut queue,
        700,
        row_id(4),
        boxed(
            LayoutStyle {
                position: PositionSpec::Fixed,
                offset_top: px(5.0),
                offset_left: px(200.0),
                width: px(10.0),
                height: px(10.0),
                ..LayoutStyle::default()
            },
            Some(SemanticColorRole::Danger),
        ),
    );
    element(
        &mut queue,
        701,
        row_id(4),
        boxed(
            LayoutStyle {
                position: PositionSpec::Absolute,
                offset_top: px(300.0),
                offset_left: px(220.0),
                width: px(10.0),
                height: px(10.0),
                ..LayoutStyle::default()
            },
            Some(SemanticColorRole::Accent),
        ),
    );
    // Row 5 is the containing block of a box inside a static wrapper.
    let mut relative = (*row_style()).clone();
    relative.position = PositionSpec::Relative;
    queue.set_style(row_id(5), boxed(relative, None));
    element(
        &mut queue,
        702,
        row_id(5),
        boxed(
            LayoutStyle {
                width: px(30.0),
                height: px(10.0),
                ..LayoutStyle::default()
            },
            None,
        ),
    );
    element(
        &mut queue,
        703,
        id(702),
        boxed(
            LayoutStyle {
                position: PositionSpec::Absolute,
                offset_top: px(2.0),
                offset_left: px(150.0),
                width: px(8.0),
                height: px(8.0),
                ..LayoutStyle::default()
            },
            Some(SemanticColorRole::Accent),
        ),
    );
    // A scrolled container in row 6, with a sticky box in it.
    element(
        &mut queue,
        704,
        row_id(6),
        boxed(
            LayoutStyle {
                width: px(60.0),
                height: px(16.0),
                direction: Some(FlexDirection::Column),
                overflow_y: OverflowSpec::Auto,
                ..LayoutStyle::default()
            },
            Some(SemanticColorRole::Text),
        ),
    );
    element(
        &mut queue,
        705,
        id(704),
        boxed(
            LayoutStyle {
                position: PositionSpec::Sticky,
                offset_top: px(0.0),
                width: px(40.0),
                height: px(4.0),
                ..LayoutStyle::default()
            },
            Some(SemanticColorRole::Danger),
        ),
    );
    element(
        &mut queue,
        706,
        id(704),
        boxed(
            LayoutStyle {
                width: px(50.0),
                height: px(60.0),
                ..LayoutStyle::default()
            },
            Some(SemanticColorRole::Accent),
        ),
    );
    runtime.context_mut().commit_mutations(queue).unwrap();
    let field = runtime
        .context_mut()
        .create_detached_component(document_id, TextInput::new("moved text"))
        .unwrap()
        .stable_id();
    let button = runtime
        .context_mut()
        .create_detached_component(document_id, Button::new("Go"))
        .unwrap()
        .stable_id();
    let mut queue = MutationQueue::new();
    queue.insert(row_id(7), field, None);
    queue.insert(row_id(7), button, None);
    runtime.context_mut().commit_mutations(queue).unwrap();
    flush(&mut runtime);
    let mut queue = MutationQueue::new();
    queue.set_scroll_offset(id(704), ScrollOffset { x: 0.0, y: 7.0 });
    runtime.context_mut().commit_mutations(queue).unwrap();
    flush(&mut runtime);
    assert_scene_is_cold(&runtime, "first frame");
    for (frame, padding, scroll) in [
        ("moved", 3.0, None),
        ("moved again", 7.5, None),
        ("moved and scrolled", 2.25, Some(12.0)),
        ("back", 0.0, Some(0.0)),
    ] {
        let mut queue = MutationQueue::new();
        queue.set_style(id(COLUMN), column_style(padding));
        if let Some(y) = scroll {
            queue.set_scroll_offset(id(704), ScrollOffset { x: 0.0, y });
        }
        runtime.context_mut().commit_mutations(queue).unwrap();
        flush(&mut runtime);
        assert_scene_is_cold(&runtime, frame);
    }
}

/// Controls whose geometry reads little of the page move with their rows and
/// keep everything: a closed select (its menu, fitted to the viewport, is
/// the part that reads the page), a closed popover (its items are placed on
/// the page), an icon centred on its parent's text (it reads its offset in
/// its parent's coordinates) and, with rich text, a Markdown document with
/// a diagram. A pure move rebuilds no primitive, and after every frame the
/// scene paints what a cold one does -- the select and the popover opened,
/// their records on the page and extracted again under every move, too.
#[test]
fn moved_controls_that_read_little_of_the_page_keep_what_they_built() {
    use nana_ui_core::Icon;
    use nana_ui_runtime::{Popover, Select, SelectOption, StandardVisual};
    let document_id = DocumentId::new(1).unwrap();
    let mut runtime = document();
    let mut select = Select::new(Some("a"));
    select.options = vec![
        SelectOption::new("a", "Alpha"),
        SelectOption::new("b", "Beta"),
    ];
    let select = runtime
        .context_mut()
        .create_detached_component(document_id, select)
        .unwrap();
    let popover = runtime
        .context_mut()
        .create_detached_component(document_id, Popover::new().trigger("More"))
        .unwrap();
    let mut queue = MutationQueue::new();
    queue.insert(row_id(1), select.stable_id(), None);
    queue.insert(row_id(2), popover.stable_id(), None);
    // A row of text with a leading icon its own size, centred on that text.
    let (host, icon) = (id(710), id(711));
    for node in [host, icon] {
        queue.create(node, document_id, NodeKind::Element { tag: "div".into() });
    }
    queue.insert(row_id(3), host, None);
    queue.insert(host, icon, None);
    queue.set_text(host, TextContent { value: "Ab".into() });
    let host_style = |padding: f32| {
        boxed(
            LayoutStyle {
                width: px(120.0),
                height: px(18.0),
                padding_left: px(3.25),
                padding_top: px(padding),
                ..LayoutStyle::default()
            },
            None,
        )
    };
    queue.set_style(host, host_style(2.5));
    queue.set_style(
        icon,
        boxed(
            LayoutStyle {
                width: px(12.0),
                height: px(12.0),
                ..LayoutStyle::default()
            },
            None,
        ),
    );
    queue.set_standard_visual(
        icon,
        Some(StandardVisual::Icon {
            icon: Icon::Add,
            size: 12.0,
            tooltip: None,
        }),
    );
    runtime.context_mut().commit_mutations(queue).unwrap();
    #[cfg(feature = "rich-text")]
    {
        let markdown = runtime
            .context_mut()
            .create_detached_component(
                document_id,
                nana_ui_runtime::NativeMarkdown::from_source(
                    "# Title\n\nSome *body* text.\n\n```mermaid\ngraph LR\n  A --> B\n```\n",
                ),
            )
            .unwrap();
        let mut queue = MutationQueue::new();
        queue.insert(row_id(6), markdown.stable_id(), None);
        runtime.context_mut().commit_mutations(queue).unwrap();
    }
    flush(&mut runtime);
    assert_scene_is_cold(&runtime, "first frame");
    let mut padding = 0.0;
    let mut step = |runtime: &mut RuntimeDocument, frame: &str| {
        padding += 1.37;
        let mut queue = MutationQueue::new();
        queue.set_style(id(COLUMN), column_style(padding));
        runtime.context_mut().commit_mutations(queue).unwrap();
        let update = runtime
            .flush(LayoutViewport::new(300.0, 800.0), &mut MeasureTextShaper)
            .unwrap();
        assert_scene_is_cold(runtime, frame);
        update.scene.stats
    };
    for frame in ["moved", "moved again", "moved a third time"] {
        let stats = step(&mut runtime, frame);
        assert_eq!(
            stats.rebuilt_primitives, 0,
            "{frame}: a pure move rebuilt primitives ({stats:?})"
        );
    }
    // The icon itself moves, inside its row: it is built again, where it
    // now stands from its parent's text.
    let mut queue = MutationQueue::new();
    queue.set_style(host, host_style(1.75));
    runtime.context_mut().commit_mutations(queue).unwrap();
    flush(&mut runtime);
    assert_scene_is_cold(&runtime, "icon moved in its row");
    runtime.context_mut().toggle_select(select).unwrap();
    runtime
        .context_mut()
        .update_component(popover, |popover, _| popover.open = true)
        .unwrap();
    flush(&mut runtime);
    assert_scene_is_cold(&runtime, "opened");
    for frame in ["opened and moved", "opened and moved again"] {
        step(&mut runtime, frame);
    }
}

/// The rows stand in a scroller scrolled by an uneven amount, and the first
/// row grows by an uneven step again and again: the rows below move inside
/// the scroll, between scrolls of their own and with one in the same
/// frame. A move rebuilds no primitive, and after every frame the scene
/// paints, and culls, what a cold one does.
#[test]
fn rows_moved_in_a_scrolled_column_paint_like_a_cold_scene() {
    use nana_ui_core::OverflowSpec;
    use nana_ui_runtime::ScrollOffset;
    let scroller = |padding: f32| {
        let mut style = column_style(padding);
        let layout = Arc::make_mut(&mut style.layout);
        layout.height = Some(LengthSpec::Px(90.0));
        layout.overflow_y = OverflowSpec::Auto;
        style
    };
    let mut runtime = document();
    let mut queue = MutationQueue::new();
    queue.set_style(id(COLUMN), scroller(0.0));
    runtime.context_mut().commit_mutations(queue).unwrap();
    flush(&mut runtime);
    assert_scene_is_cold(&runtime, "first frame");
    let mut scroll = 30.37;
    let commit = |runtime: &mut RuntimeDocument, queue: MutationQueue| {
        runtime.context_mut().commit_mutations(queue).unwrap();
        runtime
            .flush(LayoutViewport::new(300.0, 800.0), &mut MeasureTextShaper)
            .unwrap()
            .scene
            .stats
    };
    let mut queue = MutationQueue::new();
    queue.set_scroll_offset(id(COLUMN), ScrollOffset { x: 0.0, y: scroll });
    commit(&mut runtime, queue);
    assert_scene_is_cold(&runtime, "scrolled");
    let mut height = 20.0;
    let grow = |queue: &mut MutationQueue, height: f32| {
        let mut layout = (*row_style()).clone();
        layout.height = Some(LengthSpec::Px(height));
        queue.set_style(
            row_id(0),
            NodeStyle {
                layout: Arc::new(layout),
                ..NodeStyle::default()
            },
        );
    };
    for round in 0..2 {
        for step in 0..4 {
            height += 1.37;
            let frame = format!("round {round}, step {step}");
            let mut queue = MutationQueue::new();
            grow(&mut queue, height);
            let stats = commit(&mut runtime, queue);
            assert_eq!(
                stats.rebuilt_primitives, 0,
                "{frame}: a pure move rebuilt primitives ({stats:?})"
            );
            assert_scene_is_cold(&runtime, &frame);
        }
        scroll += 11.13;
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(id(COLUMN), ScrollOffset { x: 0.0, y: scroll });
        commit(&mut runtime, queue);
        assert_scene_is_cold(&runtime, &format!("round {round}: scrolled"));
        scroll -= 5.29;
        height += 1.37;
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(id(COLUMN), ScrollOffset { x: 0.0, y: scroll });
        grow(&mut queue, height);
        commit(&mut runtime, queue);
        assert_scene_is_cold(&runtime, &format!("round {round}: scrolled and moved"));
    }
}

/// An open select below a row that grows by an uneven step, again and again,
/// in a scroller that cuts the room its menu opens into. While its menu opens
/// the same way, uncapped, its record is kept: the menu stands where it stood
/// in the field's own coordinates. The field moves down until the menu no
/// longer fits below and flips above, and back up until it flips down again;
/// between the moves the scroller scrolls and the field's row is resized.
/// After every frame the scene paints what a cold one does. The field is
/// itself a moved box (a child of the column), and then a box under one (in
/// a wrapper the column moves).
#[test]
fn an_open_select_moved_keeps_its_menu_until_it_flips() {
    open_select_moved(false);
    open_select_moved(true);
}

fn open_select_moved(wrapped: bool) {
    use nana_ui_core::OverflowSpec;
    use nana_ui_runtime::{ComponentGeometry, ScrollOffset, Select, SelectOption};
    let document_id = DocumentId::new(1).unwrap();
    let scroller = |height: f32| {
        let mut style = column_style(0.0);
        let layout = Arc::make_mut(&mut style.layout);
        layout.height = Some(LengthSpec::Px(height));
        layout.overflow_y = OverflowSpec::Auto;
        style
    };
    let mut runtime = document();
    let mut select = Select::new(Some("a"));
    select.options = ["Alpha", "Beta", "Gamma"]
        .iter()
        .map(|label| SelectOption::new(label.to_lowercase(), *label))
        .collect();
    let select = runtime
        .context_mut()
        .create_detached_component(document_id, select)
        .unwrap();
    let mut queue = MutationQueue::new();
    if wrapped {
        let wrapper = id(720);
        queue.create(
            wrapper,
            document_id,
            NodeKind::Element { tag: "div".into() },
        );
        queue.set_style(
            wrapper,
            boxed(
                LayoutStyle {
                    width: px(300.0),
                    padding_top: px(0.5),
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                },
                None,
            ),
        );
        queue.insert(id(COLUMN), wrapper, Some(row_id(6)));
        queue.insert(wrapper, select.stable_id(), None);
    } else {
        queue.insert(id(COLUMN), select.stable_id(), Some(row_id(6)));
    }
    queue.set_style(id(COLUMN), scroller(400.0));
    runtime.context_mut().commit_mutations(queue).unwrap();
    flush(&mut runtime);
    runtime.context_mut().toggle_select(select).unwrap();
    flush(&mut runtime);
    // The field's box and its menu's surface on the page.
    let menu = |runtime: &RuntimeDocument| {
        let world = runtime.context().world();
        let field = world.layout_box(select.stable_id()).unwrap();
        match world.component_geometry(select.stable_id()) {
            Some(ComponentGeometry::Select {
                menu: Some(menu), ..
            }) => (field, menu.surface),
            other => panic!("expected an open menu, got {other:?}"),
        }
    };
    let (field, surface) = menu(&runtime);
    assert!(surface.y > field.y, "the menu opens down first");
    let natural = surface.height;
    // A scroller that ends 6 px below the menu: four more steps and it
    // no longer fits below; above, the rows leave it room.
    let bottom = surface.y + surface.height + 6.0;
    let mut queue = MutationQueue::new();
    queue.set_style(id(COLUMN), scroller(bottom));
    runtime.context_mut().commit_mutations(queue).unwrap();
    flush(&mut runtime);
    assert_scene_is_cold(&runtime, "first frame");
    let commit = |runtime: &mut RuntimeDocument, queue: MutationQueue| {
        runtime.context_mut().commit_mutations(queue).unwrap();
        runtime
            .flush(LayoutViewport::new(300.0, 800.0), &mut MeasureTextShaper)
            .unwrap()
            .scene
            .stats
    };
    let grow = |queue: &mut MutationQueue, height: f32| {
        let mut layout = (*row_style()).clone();
        layout.height = Some(LengthSpec::Px(height));
        queue.set_style(
            row_id(0),
            NodeStyle {
                layout: Arc::new(layout),
                ..NodeStyle::default()
            },
        );
    };
    let mut height = 20.0;
    let mut scroll = 0.0;
    let (mut kept, mut flips) = (0, 0);
    let mut opened_down = true;
    for round in 0..3 {
        // Down past the flip, then back up past it.
        for (leg, step) in [(0, 1.37), (1, -1.37)] {
            for index in 0..7 {
                height += step;
                let frame = format!("wrapped {wrapped}, round {round}, leg {leg}, step {index}");
                let mut queue = MutationQueue::new();
                grow(&mut queue, height);
                let stats = commit(&mut runtime, queue);
                assert_scene_is_cold(&runtime, &frame);
                let (field, surface) = menu(&runtime);
                let down = surface.y > field.y;
                // Fits one way or the other: never capped.
                assert_eq!(surface.height, natural, "{frame}: capped");
                if down == opened_down {
                    assert_eq!(
                        stats.rebuilt_primitives, 0,
                        "{frame}: a move that keeps the menu's side rebuilt primitives ({stats:?})"
                    );
                    kept += 1;
                } else {
                    assert!(
                        stats.rebuilt_primitives > 0,
                        "{frame}: the menu flipped and nothing was built"
                    );
                    flips += 1;
                }
                opened_down = down;
            }
        }
        // A scroll moves the room: the menu is fitted again.
        scroll += 0.83;
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(id(COLUMN), ScrollOffset { x: 0.0, y: scroll });
        commit(&mut runtime, queue);
        assert_scene_is_cold(
            &runtime,
            &format!("wrapped {wrapped}, round {round}: scrolled"),
        );
        // The field's row is resized with a move in the same frame.
        let mut queue = MutationQueue::new();
        queue.set_style(
            row_id(5),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: px(290.0 - round as f32 * 3.1),
                    ..(*row_style()).clone()
                }),
                ..NodeStyle::default()
            },
        );
        height += 1.37;
        grow(&mut queue, height);
        commit(&mut runtime, queue);
        assert_scene_is_cold(
            &runtime,
            &format!("wrapped {wrapped}, round {round}: resized and moved"),
        );
        height -= 1.37;
        let mut queue = MutationQueue::new();
        grow(&mut queue, height);
        commit(&mut runtime, queue);
        assert_scene_is_cold(
            &runtime,
            &format!("wrapped {wrapped}, round {round}: moved back"),
        );
        opened_down = {
            let (field, surface) = menu(&runtime);
            surface.y > field.y
        };
    }
    assert!(flips >= 6, "the menu flipped {flips} times");
    assert!(kept >= 24, "kept the menu {kept} times");
}
