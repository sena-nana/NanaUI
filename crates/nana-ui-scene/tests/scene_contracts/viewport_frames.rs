//! Boxes placed from the viewport -- the page's root, a `fixed` and a
//! `sticky` box, an absolute box escaping a scroller's clip -- turn again and
//! again, and boxes scale by other factors (one nested in another, one all
//! but flat), with moves by an uneven step and scrolls between and in the
//! same frames. Each hands its own frame (`AccessibilityNode::transform`),
//! so what it holds is placed by its offsets inside it. After every frame
//! the accessibility tree the deltas handed out equals, to the bit, the one
//! a twin page projects whole from scratch, and placing it through the
//! transforms it hands puts every box where the twin's viewport projection
//! does.

use std::collections::BTreeMap;
use std::sync::Arc;

use nana_ui_core::{
    FlexDirection, LayoutStyle, LengthSpec, OverflowSpec, PaintTransform, PositionSpec,
};
use nana_ui_runtime::{
    AccessibilityNode, AccessibilityPlacements, DocumentId, LayoutViewport, MeasureTextShaper,
    MutationQueue, NodeKind, NodeStyle, ScrollOffset, StableNodeId,
};
use nana_ui_scene::{RuntimeDocument, RuntimeFrameUpdate};

/// The page's root: no document node above it, as an application's page.
const ROOT: u64 = 1;
/// A scroller between the rows.
const LIST: u64 = 2;
/// A box scaled by `diag(a, d)`, holding rows and [`NESTED`].
const SCALED: u64 = 3;
const NESTED: u64 = 4;
const STICKY: u64 = 30;
const FIXED: u64 = 31;
const ESCAPING: u64 = 32;

fn document_id() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn id(raw: u64) -> StableNodeId {
    StableNodeId::new(raw).unwrap()
}

fn px(value: f32) -> Option<LengthSpec> {
    Some(LengthSpec::Px(value))
}

fn styled(layout: LayoutStyle) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        ..NodeStyle::default()
    }
}

fn turn(kind: &str, angle: f32) -> PaintTransform {
    let (sin, cos) = angle.sin_cos();
    match kind {
        "rotated" => PaintTransform {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 3.25,
            f: 1.5,
        },
        "skewed" => PaintTransform {
            c: angle.tan(),
            e: 1.5,
            ..PaintTransform::default()
        },
        "flipped" => PaintTransform {
            a: -1.0,
            f: 2.75,
            ..PaintTransform::default()
        },
        other => panic!("no turn {other}"),
    }
}

fn scale(a: f32, d: f32) -> PaintTransform {
    PaintTransform {
        a,
        d,
        e: 2.25,
        f: -1.5,
        ..PaintTransform::default()
    }
}

fn root_style(transform: Option<PaintTransform>) -> NodeStyle {
    styled(LayoutStyle {
        width: px(360.0),
        direction: Some(FlexDirection::Column),
        position: PositionSpec::Relative,
        padding_left: px(1.3),
        padding_top: px(2.1),
        transform,
        ..LayoutStyle::default()
    })
}

fn scaled_style(height: f32, transform: Option<PaintTransform>) -> NodeStyle {
    styled(LayoutStyle {
        width: px(300.0),
        height: px(height),
        direction: Some(FlexDirection::Column),
        flex_shrink: Some(0.0),
        padding_top: px(0.7),
        transform,
        ..LayoutStyle::default()
    })
}

fn placed_style(position: PositionSpec, transform: PaintTransform) -> NodeStyle {
    styled(LayoutStyle {
        position,
        offset_top: px(1.0),
        offset_left: px(7.0),
        width: px(12.0),
        height: px(6.0),
        transform: Some(transform),
        ..LayoutStyle::default()
    })
}

fn row_style(height: f32) -> NodeStyle {
    styled(LayoutStyle {
        width: px(300.0),
        height: px(height),
        direction: Some(FlexDirection::Row),
        flex_shrink: Some(0.0),
        ..LayoutStyle::default()
    })
}

fn element(queue: &mut MutationQueue, node: StableNodeId, parent: StableNodeId, style: NodeStyle) {
    queue.create(node, document_id(), NodeKind::Element { tag: "div".into() });
    queue.insert(parent, node, None);
    queue.set_style(node, style);
}

/// A row holding a padded cell holding a leaf; returns the row and cell.
fn row(queue: &mut MutationQueue, at: u64, parent: StableNodeId) -> (StableNodeId, StableNodeId) {
    let (row, cell, leaf) = (id(at), id(at + 1), id(at + 2));
    element(queue, row, parent, row_style(20.0));
    element(
        queue,
        cell,
        row,
        styled(LayoutStyle {
            width: px(100.0),
            height: px(14.0),
            padding_left: px(1.3),
            padding_top: px(2.5),
            ..LayoutStyle::default()
        }),
    );
    element(
        queue,
        leaf,
        cell,
        styled(LayoutStyle {
            width: px(50.0),
            height: px(5.0),
            ..LayoutStyle::default()
        }),
    );
    (row, cell)
}

fn placed(
    queue: &mut MutationQueue,
    node: u64,
    parent: StableNodeId,
    position: PositionSpec,
    transform: PaintTransform,
) {
    element(queue, id(node), parent, placed_style(position, transform));
    element(
        queue,
        id(node + 100),
        id(node),
        styled(LayoutStyle {
            width: px(4.0),
            height: px(3.0),
            ..LayoutStyle::default()
        }),
    );
}

struct Page {
    runtime: RuntimeDocument,
    /// What the deltas handed out, applied one over the other.
    accessibility: BTreeMap<StableNodeId, AccessibilityNode>,
    /// The first row of the root and of the scaled box.
    rows: [StableNodeId; 3],
}

/// Root (turned, relative) → six rows, the list (a scroller of eight
/// rows), the scaled box (three rows, then the nested scaled box of two).
/// The root's second row holds a turned sticky and a turned fixed box; the
/// list's last row a turned absolute box measured from the root, which
/// escapes the list's clip (#201).
fn build() -> Page {
    let document = document_id();
    let mut runtime = RuntimeDocument::new(document);
    let mut queue = MutationQueue::new();
    queue.create(id(ROOT), document, NodeKind::Element { tag: "div".into() });
    queue.set_style(id(ROOT), root_style(Some(turn("rotated", 0.1))));
    let mut cells = Vec::new();
    for index in 0..6 {
        cells.push(row(&mut queue, 100 + index * 4, id(ROOT)));
    }
    placed(
        &mut queue,
        STICKY,
        cells[1].1,
        PositionSpec::Sticky,
        turn("rotated", 0.25),
    );
    placed(
        &mut queue,
        FIXED,
        cells[1].1,
        PositionSpec::Fixed,
        turn("skewed", 0.3),
    );
    element(
        &mut queue,
        id(LIST),
        id(ROOT),
        styled(LayoutStyle {
            width: px(300.0),
            height: px(60.0),
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            overflow_y: OverflowSpec::Auto,
            ..LayoutStyle::default()
        }),
    );
    let mut listed = Vec::new();
    for index in 0..8 {
        listed.push(row(&mut queue, 200 + index * 4, id(LIST)));
    }
    placed(
        &mut queue,
        ESCAPING,
        listed[7].1,
        PositionSpec::Absolute,
        turn("rotated", -0.4),
    );
    element(
        &mut queue,
        id(SCALED),
        id(ROOT),
        scaled_style(140.0, Some(scale(1.5, 1.25))),
    );
    let mut scaled = Vec::new();
    for index in 0..3 {
        scaled.push(row(&mut queue, 300 + index * 4, id(SCALED)));
    }
    element(
        &mut queue,
        id(NESTED),
        id(SCALED),
        scaled_style(50.0, Some(scale(0.8, 1.1))),
    );
    let mut nested = Vec::new();
    for index in 0..2 {
        nested.push(row(&mut queue, 400 + index * 4, id(NESTED)));
    }
    runtime.context_mut().commit_mutations(queue).unwrap();
    let mut page = Page {
        runtime,
        accessibility: BTreeMap::new(),
        rows: [cells[0].0, scaled[0].0, nested[0].0],
    };
    let update = flush(&mut page.runtime);
    take(&mut page, update);
    page
}

fn flush(runtime: &mut RuntimeDocument) -> RuntimeFrameUpdate {
    runtime
        .flush(LayoutViewport::new(400.0, 600.0), &mut MeasureTextShaper)
        .unwrap()
}

fn take(page: &mut Page, update: RuntimeFrameUpdate) -> Vec<StableNodeId> {
    for removed in &update.accessibility.removed {
        page.accessibility.remove(removed);
    }
    let updated = update
        .accessibility
        .updated
        .iter()
        .map(|node| node.id)
        .collect();
    for node in update.accessibility.updated {
        page.accessibility.insert(node.id, node);
    }
    updated
}

/// The tree the page's deltas add up to is the twin's, projected whole, to
/// the bit; and every box of it, placed through the transforms it hands,
/// stands where the twin's viewport projection puts it.
#[track_caller]
fn same(page: &Page, twin: &Page, at: &str) {
    let full: BTreeMap<_, _> = twin
        .runtime
        .context()
        .world()
        .project_accessibility(document_id())
        .into_iter()
        .map(|node| (node.id, node))
        .collect();
    let world = page.runtime.context().world();
    for (id, node) in &full {
        assert_eq!(
            page.accessibility.get(id),
            Some(node),
            "{at}: the deltas left {id:?} unlike a full projection"
        );
    }
    assert!(
        page.accessibility
            .keys()
            .all(|id| full.contains_key(id) || !world.contains(*id)),
        "{at}: the deltas left nodes a full projection does not have"
    );
    let placements = AccessibilityPlacements::new(page.accessibility.values());
    for node in twin
        .runtime
        .context()
        .world()
        .project_accessibility_in_viewport(document_id())
    {
        let placed = placements.viewport_bounds(node.id).unwrap();
        let want = node.bounds;
        assert!(
            [
                placed.x - want.x,
                placed.y - want.y,
                placed.width - want.width,
                placed.height - want.height
            ]
            .iter()
            .all(|delta| delta.abs() <= 2e-3),
            "{at}: {:?} placed at {placed:?}, in the viewport at {want:?}",
            node.id
        );
    }
}

/// Commit what `change` writes on both pages, run a frame on each and
/// compare them ([`same`]); returns what the page's delta projected.
fn edit(
    page: &mut Page,
    twin: &mut Page,
    at: &str,
    change: impl Fn(&mut MutationQueue),
) -> Vec<StableNodeId> {
    let mut updated = Vec::new();
    for (index, page) in [&mut *page, &mut *twin].into_iter().enumerate() {
        let mut queue = MutationQueue::new();
        change(&mut queue);
        page.runtime.context_mut().commit_mutations(queue).unwrap();
        let done = flush(&mut page.runtime);
        let done = take(page, done);
        if index == 0 {
            updated = done;
        }
    }
    same(page, twin, at);
    updated
}

/// `node` and everything under it in what `page` handed out.
fn subtree(page: &Page, node: u64) -> Vec<StableNodeId> {
    let mut out = Vec::new();
    let mut pending = vec![id(node)];
    while let Some(at) = pending.pop() {
        out.push(at);
        pending.extend(page.accessibility[&at].children.iter().copied());
    }
    out
}

#[test]
fn boxes_placed_from_the_viewport_turn_and_scale_as_a_full_projection_does() {
    let mut page = build();
    let mut twin = build();
    same(&page, &twin, "built");
    for node in [ROOT, STICKY, FIXED, ESCAPING, SCALED, NESTED] {
        assert!(
            page.accessibility[&id(node)].transform.is_some(),
            "{node} hands its own frame"
        );
    }
    // A turn of the root projects it and the pins under it, with what they
    // hold: the placed boxes and the list, whose clip is in viewport
    // coordinates.
    let mut pinned = vec![id(ROOT)];
    for node in [STICKY, FIXED, ESCAPING, LIST] {
        pinned.extend(subtree(&page, node));
    }
    let mut heights = [20.0_f32; 3];
    let mut scroll = 0.0;
    let scales = [
        (scale(1.37, 1.37), scale(0.85, 1.1)),
        (scale(0.5, 1.37), scale(2.0, 0.5)),
        (scale(1e-3, 1e-3), scale(1e-3, 2.0)),
        (scale(2.0, 1e-4), scale(0.9, 0.9)),
        (scale(1.25, 0.75), scale(1.75, 1.5)),
    ];
    for round in 0..2 {
        for step in 0..5 {
            let at = format!("round {round}, step {step}");
            let angle = 0.1 + 0.137 * (round * 5 + step + 1) as f32;
            let updated = edit(
                &mut page,
                &mut twin,
                &format!("{at}: root turned"),
                |queue| {
                    queue.set_style(id(ROOT), root_style(Some(turn("rotated", angle))));
                },
            );
            assert!(updated.contains(&id(ROOT)), "{at}: the root projects again");
            assert!(
                updated.iter().all(|node| pinned.contains(node)),
                "{at}: a turn of the root projected {updated:?}"
            );
            for (node, position) in [
                (STICKY, PositionSpec::Sticky),
                (FIXED, PositionSpec::Fixed),
                (ESCAPING, PositionSpec::Absolute),
            ] {
                let kind = ["rotated", "skewed", "flipped"][(node as usize + step) % 3];
                let updated = edit(
                    &mut page,
                    &mut twin,
                    &format!("{at}: {node} turned"),
                    |queue| {
                        queue.set_style(id(node), placed_style(position, turn(kind, angle * 0.7)));
                    },
                );
                assert_eq!(
                    updated,
                    vec![id(node)],
                    "{at}: a turn of {node} projects it alone"
                );
            }
            let (outer, inner) = scales[step];
            let updated = edit(&mut page, &mut twin, &format!("{at}: scaled"), |queue| {
                queue.set_style(id(SCALED), scaled_style(140.0, Some(outer)));
                queue.set_style(id(NESTED), scaled_style(50.0, Some(inner)));
            });
            assert_eq!(
                updated,
                vec![id(SCALED), id(NESTED)],
                "{at}: a change of factors projects the scaled boxes alone"
            );
            // Moves by an uneven step under the root and under the scale,
            // then a move, a scroll and a turn in one frame.
            let which = step % 3;
            heights[which] += 1.37;
            let (moved, height) = (page.rows[which], heights[which]);
            edit(&mut page, &mut twin, &format!("{at}: moved"), |queue| {
                queue.set_style(moved, row_style(height));
            });
            scroll += 3.37;
            let list_scroll = scroll;
            edit(&mut page, &mut twin, &format!("{at}: scrolled"), |queue| {
                queue.set_scroll_offset(
                    id(LIST),
                    ScrollOffset {
                        x: 0.0,
                        y: list_scroll,
                    },
                );
            });
            heights[0] += 1.37;
            let (first, height) = (page.rows[0], heights[0]);
            edit(
                &mut page,
                &mut twin,
                &format!("{at}: moved, scrolled and turned"),
                |queue| {
                    queue.set_style(first, row_style(height));
                    queue.set_scroll_offset(
                        id(LIST),
                        ScrollOffset {
                            x: 0.0,
                            y: list_scroll * 0.5,
                        },
                    );
                    queue.set_style(id(ROOT), root_style(Some(turn("skewed", -angle * 0.5))));
                    queue.set_style(
                        id(ESCAPING),
                        placed_style(PositionSpec::Absolute, turn("rotated", angle)),
                    );
                },
            );
        }
    }
    // The root goes upright and turns again; the scaled box goes to a
    // scale of one and back: each changes the space, and everything under
    // it projects again.
    for (name, root, outer) in [
        ("upright", None, Some(scale(1.0, 1.0))),
        (
            "turned again",
            Some(turn("flipped", 0.0)),
            Some(scale(1.2, 0.9)),
        ),
    ] {
        let updated = edit(&mut page, &mut twin, name, |queue| {
            queue.set_style(id(ROOT), root_style(root));
            queue.set_style(id(SCALED), scaled_style(140.0, outer));
        });
        assert!(updated.contains(&id(302)), "{name}: the spaces changed");
        assert_eq!(
            page.accessibility[&id(ROOT)].transform.is_some(),
            root.is_some()
        );
        assert_eq!(
            page.accessibility[&id(SCALED)].transform.is_some(),
            outer.is_some_and(|scale| (scale.a, scale.d) != (1.0, 1.0)),
            "{name}: a scale of one hands no transform"
        );
    }
}
