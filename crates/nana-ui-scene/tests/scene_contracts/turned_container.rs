//! A container's `transform` changes again and again -- its turn, its scale,
//! its origin -- with moves, scrolls and inserts between and in the same
//! frames. Only the container is marked (`SystemWork::transform_kept` when
//! what it holds keeps its space): its own hit entry is built again, its own
//! accessible node and those placed from the viewport under it project
//! again, and the scene re-projects what it holds. After every frame the
//! page answers the pointer, hands out an accessibility tree and paints as
//! a twin page does whose hit index, accessibility tree and scene are all
//! built from scratch.

use std::collections::BTreeMap;
use std::sync::Arc;

use nana_ui_core::{
    FlexDirection, LayoutStyle, LengthSpec, OverflowSpec, PaintTransform, PositionSpec,
    SemanticColorRole, TransformOrigin,
};
use nana_ui_runtime::{
    AccessibilityNode, DocumentId, Entity, LayoutViewport, MeasureTextShaper, MutationQueue,
    NodeKind, NodeStyle, ScrollOffset, Select, SelectOption, StableNodeId, TextContent,
};
use nana_ui_scene::{AffineTransform, RuntimeDocument, RuntimeFrameUpdate, SceneRect, UiScene};

const ROOT: u64 = 1;
/// The page: a scroller, and the containing block of every absolute box.
const PAGE: u64 = 2;
/// The turned container whose transform the frames change.
const OUTER: u64 = 3;
/// A turned box inside it, after its rows.
const INNER: u64 = 4;
/// A scroller inside it, between its rows and the inner box.
const LIST: u64 = 5;
const STICKY: u64 = 30;
const FIXED: u64 = 31;
const ABSOLUTE: u64 = 32;
const ESCAPING: u64 = 33;

fn document_id() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn id(raw: u64) -> StableNodeId {
    StableNodeId::new(raw).unwrap()
}

fn px(value: f32) -> Option<LengthSpec> {
    Some(LengthSpec::Px(value))
}

fn boxed(layout: LayoutStyle, background: Option<SemanticColorRole>) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        background,
        ..NodeStyle::default()
    }
}

fn turn(angle: f32, scale: f32) -> PaintTransform {
    let (sin, cos) = angle.sin_cos();
    PaintTransform {
        a: scale * cos,
        b: scale * sin,
        c: -scale * sin,
        d: scale * cos,
        e: 3.25,
        f: 1.5,
    }
}

/// The outer container: static, so that the absolute boxes it holds are
/// measured from the page above it.
fn outer_style(transform: Option<PaintTransform>, origin: Option<TransformOrigin>) -> NodeStyle {
    boxed(
        LayoutStyle {
            width: px(340.0),
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            padding_left: px(1.3),
            padding_top: px(2.1),
            transform,
            transform_origin: origin,
            ..LayoutStyle::default()
        },
        Some(SemanticColorRole::Surface),
    )
}

/// The inner box clips what it holds: a turn of it cuts its subtree anew.
fn inner_style(angle: f32) -> NodeStyle {
    boxed(
        LayoutStyle {
            width: px(300.0),
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            overflow_x: OverflowSpec::Hidden,
            overflow_y: OverflowSpec::Hidden,
            transform: Some(PaintTransform {
                c: angle.tan(),
                e: 1.5,
                ..PaintTransform::default()
            }),
            ..LayoutStyle::default()
        },
        None,
    )
}

fn row_style(height: f32) -> NodeStyle {
    boxed(
        LayoutStyle {
            width: px(300.0),
            height: px(height),
            direction: Some(FlexDirection::Row),
            flex_shrink: Some(0.0),
            ..LayoutStyle::default()
        },
        Some(SemanticColorRole::Accent),
    )
}

struct Page {
    runtime: RuntimeDocument,
    /// What the deltas handed out, applied one over the other.
    accessibility: BTreeMap<StableNodeId, AccessibilityNode>,
    select: Entity<Select>,
    /// The first row of the outer container, of the list and of the inner
    /// box: growing them moves the rows after them.
    rows: [StableNodeId; 3],
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
        boxed(
            LayoutStyle {
                width: px(100.0),
                height: px(14.0),
                padding_left: px(1.3),
                padding_top: px(2.5),
                ..LayoutStyle::default()
            },
            Some(SemanticColorRole::Text),
        ),
    );
    element(
        queue,
        leaf,
        cell,
        boxed(
            LayoutStyle {
                width: px(50.0),
                height: px(5.0),
                ..LayoutStyle::default()
            },
            Some(SemanticColorRole::Danger),
        ),
    );
    (row, cell)
}

/// A box of `width` × `height` placed by `position` from `top`, `left`,
/// with a plain box inside.
fn placed(
    queue: &mut MutationQueue,
    node: u64,
    parent: StableNodeId,
    position: PositionSpec,
    transform: Option<PaintTransform>,
) {
    element(
        queue,
        id(node),
        parent,
        boxed(
            LayoutStyle {
                position,
                offset_top: px(1.0),
                offset_left: px(7.0),
                width: px(12.0),
                height: px(6.0),
                transform,
                ..LayoutStyle::default()
            },
            Some(SemanticColorRole::Danger),
        ),
    );
    element(
        queue,
        id(node + 100),
        id(node),
        boxed(
            LayoutStyle {
                width: px(4.0),
                height: px(3.0),
                ..LayoutStyle::default()
            },
            Some(SemanticColorRole::Accent),
        ),
    );
}

/// Root → page (a scroller, relative) → outer (turned, static) → six rows,
/// the list (a scroller of eight rows), the inner box (skewed, three rows).
/// The outer's second row holds a turned sticky box, a fixed box and an
/// absolute box measured from the page; the list's last row an absolute box
/// measured from the page, which escapes the list's clip (#201). A select
/// stands in the third row and, with rich text, a Markdown document in the
/// fourth.
fn build() -> Page {
    let document = document_id();
    let mut runtime = RuntimeDocument::new(document);
    let mut queue = MutationQueue::new();
    queue.create(id(ROOT), document, NodeKind::Document);
    element(
        &mut queue,
        id(PAGE),
        id(ROOT),
        boxed(
            LayoutStyle {
                width: px(400.0),
                height: px(600.0),
                direction: Some(FlexDirection::Column),
                overflow_y: OverflowSpec::Auto,
                position: PositionSpec::Relative,
                // Clear of the page's left edge, turned: a culling bound
                // near zero compared by ulps would read the rounding of
                // the terms it cancels as drift (see `bounds_match`).
                padding_left: px(40.0),
                ..LayoutStyle::default()
            },
            None,
        ),
    );
    element(
        &mut queue,
        id(OUTER),
        id(PAGE),
        outer_style(Some(turn(0.2, 1.0)), None),
    );
    let mut cells = Vec::new();
    for index in 0..6 {
        cells.push(row(&mut queue, 100 + index * 4, id(OUTER)));
    }
    placed(
        &mut queue,
        STICKY,
        cells[1].1,
        PositionSpec::Sticky,
        Some(turn(0.25, 1.0)),
    );
    placed(&mut queue, FIXED, cells[1].1, PositionSpec::Fixed, None);
    placed(
        &mut queue,
        ABSOLUTE,
        cells[1].1,
        PositionSpec::Absolute,
        None,
    );
    element(
        &mut queue,
        id(LIST),
        id(OUTER),
        boxed(
            LayoutStyle {
                width: px(300.0),
                height: px(60.0),
                direction: Some(FlexDirection::Column),
                flex_shrink: Some(0.0),
                overflow_y: OverflowSpec::Auto,
                ..LayoutStyle::default()
            },
            None,
        ),
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
        None,
    );
    element(&mut queue, id(INNER), id(OUTER), inner_style(0.3));
    let mut inner = Vec::new();
    for index in 0..3 {
        inner.push(row(&mut queue, 300 + index * 4, id(INNER)));
    }
    runtime.context_mut().commit_mutations(queue).unwrap();
    let mut select = Select::new(Some("a"));
    select.options = vec![
        SelectOption::new("a", "Alpha"),
        SelectOption::new("b", "Beta"),
    ];
    let select = runtime
        .context_mut()
        .create_detached_component(document, select)
        .unwrap();
    let mut queue = MutationQueue::new();
    queue.insert(cells[2].1, select.stable_id(), None);
    runtime.context_mut().commit_mutations(queue).unwrap();
    #[cfg(feature = "rich-text")]
    {
        let markdown = runtime
            .context_mut()
            .create_detached_component(
                document,
                nana_ui_runtime::NativeMarkdown::from_source(
                    "# Title\n\nSome *body* text.\n\n```mermaid\ngraph LR\n  A --> B\n```\n",
                ),
            )
            .unwrap();
        let mut queue = MutationQueue::new();
        queue.insert(cells[3].1, markdown.stable_id(), None);
        runtime.context_mut().commit_mutations(queue).unwrap();
    }
    let mut page = Page {
        runtime,
        accessibility: BTreeMap::new(),
        select,
        rows: [cells[0].0, listed[0].0, inner[0].0],
    };
    let update = flush(&mut page.runtime);
    take(&mut page, update);
    page
}

fn flush(runtime: &mut RuntimeDocument) -> RuntimeFrameUpdate {
    let update = runtime
        .flush(LayoutViewport::new(400.0, 600.0), &mut MeasureTextShaper)
        .unwrap();
    // The painter culls each frame, so the scene keeps its culling index
    // and pays for keeping it current.
    let viewport = SceneRect {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 600.0,
    };
    runtime.scene().visible_operations(viewport).unwrap();
    update
}

/// Apply `update`'s accessibility delta to what `page` handed out.
fn take(page: &mut Page, update: RuntimeFrameUpdate) -> RuntimeFrameUpdate {
    for removed in &update.accessibility.removed {
        page.accessibility.remove(removed);
    }
    for node in &update.accessibility.updated {
        page.accessibility.insert(node.id, node.clone());
    }
    update
}

/// One point of [`grid`]: the topmost hit, every candidate, and where the
/// point lands in each candidate's layout coordinates.
type Point = (
    Option<StableNodeId>,
    Vec<StableNodeId>,
    Vec<Option<(f32, f32)>>,
);

fn grid(runtime: &RuntimeDocument) -> Vec<Point> {
    let world = runtime.context().world();
    let mut points = Vec::new();
    for row in 0..60 {
        let y = row as f32 * 9.7 + 0.71;
        for column in 0..36 {
            let x = column as f32 * 11.3 + 0.43;
            let candidates = world.hit_test_candidates(document_id(), x, y);
            let positions = candidates
                .iter()
                .map(|&id| world.pointer_layout_position(id, x, y))
                .collect();
            points.push((world.hit_test(document_id(), x, y), candidates, positions));
        }
    }
    points
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
    let fold = |pick: fn(&(f32, f32)) -> f32, start: f32, op: fn(f32, f32) -> f32| {
        corners.iter().map(pick).fold(start, op)
    };
    [
        fold(|p| p.0, f32::INFINITY, f32::min),
        fold(|p| p.1, f32::INFINITY, f32::min),
        fold(|p| p.0, f32::NEG_INFINITY, f32::max),
        fold(|p| p.1, f32::NEG_INFINITY, f32::max),
    ]
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 1e-3)
}

/// The page and its twin, whose hit index is built again whole after every
/// flush and whose accessibility tree and scene are built from scratch,
/// agree: the pointer finds the same boxes at every point of a grid and
/// lands at the same place in each; the deltas handed out add up to the
/// full projection; and the scene draws every primitive of a cold one, the
/// same, at the same place, through the same transform and clips, and
/// culls as it does.
#[track_caller]
fn same(page: &Page, twin: &mut Page, at: &str) {
    twin.runtime.context_mut().rebuild_hit_test(document_id());
    let (ours, theirs) = (grid(&page.runtime), grid(&twin.runtime));
    let differs = |(a, b): (&Point, &Point)| {
        a.0 != b.0
            || a.1 != b.1
            || a.2.iter().zip(&b.2).any(|pair| match pair {
                (Some(a), Some(b)) => (a.0 - b.0).abs() > 1e-3 || (a.1 - b.1).abs() > 1e-3,
                (a, b) => a.is_some() != b.is_some(),
            })
    };
    if let Some(point) = (0..ours.len()).find(|&point| differs((&ours[point], &theirs[point]))) {
        panic!(
            "{at}: point {point} (row {}, column {}): {:?} != {:?}",
            point / 36,
            point % 36,
            ours[point],
            theirs[point]
        );
    }

    let world = page.runtime.context().world();
    let full: BTreeMap<_, _> = twin
        .runtime
        .context()
        .world()
        .project_accessibility(document_id())
        .into_iter()
        .map(|node| (node.id, node))
        .collect();
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

    let scene = page.runtime.scene();
    let mut cold = UiScene::new();
    cold.apply_delta(
        twin.runtime
            .context()
            .world()
            .extract_document(document_id()),
        [],
    );
    let ids: Vec<_> = scene.primitives().map(|primitive| primitive.id).collect();
    let cold_ids: Vec<_> = cold.primitives().map(|primitive| primitive.id).collect();
    assert_eq!(ids, cold_ids, "{at}: other primitives than a cold scene");
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
            "{at}: {:?} draws at {:?}, a cold scene at {:?}",
            held.id,
            drawn(draw.placed_bounds(), draw.transform),
            drawn(cold_draw.placed_bounds(), cold_draw.transform),
        );
        assert_eq!(
            draw.origin.map(f32::to_bits),
            cold_draw.origin.map(f32::to_bits),
            "{at}: {:?} is put at {:?}, a cold one at {:?}",
            held.id,
            draw.origin,
            cold_draw.origin
        );
        assert!(
            draw.transform
                .0
                .iter()
                .zip(cold_draw.transform.0)
                .all(|(a, b)| (a - b).abs() <= 1e-3),
            "{at}: {:?} is drawn through {:?}, a cold one through {:?}",
            held.id,
            draw.transform,
            cold_draw.transform
        );
        assert_eq!(draw.clips.len(), cold_draw.clips.len(), "{at}: clips");
        for (clip, cold_clip) in draw.clips.iter().zip(cold_draw.clips.iter()) {
            assert!(
                near(
                    drawn(clip.bounds, clip.transform),
                    drawn(cold_clip.bounds, cold_clip.transform)
                ),
                "{at}: {:?} is cut by {clip:?}, a cold one by {cold_clip:?}",
                held.id
            );
        }
        let mut held = held.clone();
        held.clips = fresh.clips.clone();
        held.transform = fresh.transform;
        assert_eq!(&held, fresh, "{at}: {:?} is not a cold primitive", held.id);
    }
    for viewport in [
        SceneRect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 600.0,
        },
        SceneRect {
            x: 30.0,
            y: 41.0,
            width: 90.0,
            height: 33.0,
        },
        SceneRect {
            x: 180.0,
            y: 133.0,
            width: 20.0,
            height: 7.0,
        },
    ] {
        assert_eq!(
            scene.visible_operations(viewport).unwrap(),
            cold.visible_operations(viewport).unwrap(),
            "{at}: a painter culling to {viewport:?} draws other primitives than in a cold scene"
        );
    }
}

/// What one frame of [`edit`] did on the page.
struct Done {
    update: RuntimeFrameUpdate,
    hit_built: usize,
}

/// Commit what `change` writes on both pages, run a frame on each and
/// compare them ([`same`]).
fn edit(
    page: &mut Page,
    twin: &mut Page,
    at: &str,
    change: impl Fn(&mut Page, &mut MutationQueue),
) -> Done {
    let mut built = 0;
    let mut update = None;
    for (index, page) in [&mut *page, &mut *twin].into_iter().enumerate() {
        let mut queue = MutationQueue::new();
        change(page, &mut queue);
        page.runtime.context_mut().commit_mutations(queue).unwrap();
        let done = flush(&mut page.runtime);
        let done = take(page, done);
        if index == 0 {
            built = page
                .runtime
                .context()
                .world()
                .last_work_counters()
                .hit_test_nodes_rebuilt
                .unwrap_or_default();
            update = Some(done);
        }
    }
    same(page, twin, at);
    Done {
        update: update.unwrap(),
        hit_built: built,
    }
}

fn set_outer(
    queue: &mut MutationQueue,
    transform: Option<PaintTransform>,
    origin: Option<TransformOrigin>,
) {
    queue.set_style(id(OUTER), outer_style(transform, origin));
}

fn scroll(queue: &mut MutationQueue, scroller: u64, y: f32) {
    queue.set_scroll_offset(id(scroller), ScrollOffset { x: 0.0, y });
}

/// Turn, scale and move the origin of the outer container, again and
/// again, with moves, scrolls and inserts between and in the same frames,
/// and turn the inner box under it; open the select, and change what the
/// outer one hands its children (turn to scale, scale to a move, and back).
/// Every frame agrees with the twin built from scratch ([`same`]); a change
/// that keeps the space builds one hit entry and projects only the outer
/// box and what is placed from the viewport under it.
#[test]
fn a_turned_container_changed_again_and_again_keeps_what_it_holds() {
    let mut page = build();
    let mut twin = build();
    let mut nodes = page.runtime.context().world().len();
    same(&page, &mut twin, "built");
    let mut heights = [20.0_f32; 3];
    // What a turn that keeps the outer box's space projects with the box:
    // the boxes placed from the viewport under it (sticky, fixed, absolute,
    // and what they hold), and the list, whose clip in viewport coordinates
    // cuts everything it holds.
    let viewport_placed: Vec<StableNodeId> = [OUTER, LIST, STICKY, FIXED, ABSOLUTE, ESCAPING]
        .into_iter()
        .flat_map(|node| [node, node + 100])
        .chain(200..232)
        .chain(std::iter::once(INNER))
        .chain(300..312)
        .map(id)
        .collect();
    // Its own hit entry and the pinned ones under it: the sticky, the
    // absolute and the escaping box, each with the box it holds.
    // A box that only moves hands its children no layout coordinates of its
    // own, so a hit test patches its branch whole (`hit` false).
    let kept_with = |done: &Done, at: &str, nodes: usize, hit: bool| {
        let projected: Vec<_> = done
            .update
            .accessibility
            .updated
            .iter()
            .map(|node| node.id)
            .collect();
        assert!(
            projected.contains(&id(OUTER))
                && projected.iter().all(|node| viewport_placed.contains(node)),
            "{at}: a turn that keeps the space projected {projected:?} of {nodes} nodes"
        );
        assert!(
            !hit || done.hit_built <= 7,
            "{at}: a turn that keeps the space built {} hit entries",
            done.hit_built
        );
    };
    let kept = |done: &Done, at: &str, nodes: usize| kept_with(done, at, nodes, true);
    let steps: [(&str, Option<PaintTransform>, Option<TransformOrigin>); 6] = [
        ("turned", Some(turn(0.35, 1.0)), None),
        ("turned and scaled", Some(turn(0.35, 1.1)), None),
        (
            "origin moved",
            Some(turn(0.35, 1.1)),
            Some(TransformOrigin {
                x: LengthSpec::Px(12.5),
                y: LengthSpec::Percent(30.0),
            }),
        ),
        ("turned back", Some(turn(-0.15, 0.9)), None),
        (
            "flipped",
            Some(PaintTransform {
                a: -1.0,
                f: 2.75,
                ..PaintTransform::default()
            }),
            None,
        ),
        ("turned again", Some(turn(0.2, 1.0)), None),
    ];
    for round in 0..2 {
        for (step, &(name, transform, origin)) in steps.iter().enumerate() {
            let at = format!("round {round}, {name}");
            let done = edit(&mut page, &mut twin, &at, |_, queue| {
                set_outer(queue, transform, origin)
            });
            kept(&done, &at, nodes);
            // A move under it, then a move and a turn in one frame.
            heights[step % 3] += 1.37;
            let (row, height) = (page.rows[step % 3], heights[step % 3]);
            edit(&mut page, &mut twin, &format!("{at}: moved"), |_, queue| {
                queue.set_style(row, row_style(height));
            });
            heights[0] += 1.37;
            let (row, height) = (page.rows[0], heights[0]);
            let again = turn(0.1 * step as f32 - 0.25, 1.0);
            edit(
                &mut page,
                &mut twin,
                &format!("{at}: moved and turned"),
                |_, queue| {
                    queue.set_style(row, row_style(height));
                    set_outer(queue, Some(again), origin);
                },
            );
        }
        // Scrolls: the page's (the sticky box sticks to it) and the
        // list's, alone, then with a turn in the same frame.
        let (page_scroll, list_scroll) = (7.31 + 5.0 * round as f32, 13.7 - 3.0 * round as f32);
        edit(
            &mut page,
            &mut twin,
            &format!("round {round}: scrolled"),
            |_, queue| {
                scroll(queue, PAGE, page_scroll);
                scroll(queue, LIST, list_scroll);
            },
        );
        let done = edit(
            &mut page,
            &mut twin,
            &format!("round {round}: turned scrolled"),
            |_, queue| {
                set_outer(queue, Some(turn(0.27, 1.05)), None);
            },
        );
        kept(&done, &format!("round {round}: turned scrolled"), nodes);
        edit(
            &mut page,
            &mut twin,
            &format!("round {round}: scrolled and turned"),
            |_, queue| {
                scroll(queue, PAGE, page_scroll + 2.5);
                scroll(queue, LIST, list_scroll + 4.25);
                set_outer(queue, Some(turn(0.31, 1.0)), None);
            },
        );
        // A row inserted into the outer box, then a turn of the inner one,
        // alone and with the outer's.
        let added = id(900 + round as u64 * 4);
        edit(
            &mut page,
            &mut twin,
            &format!("round {round}: inserted"),
            |_, queue| {
                row(queue, 900 + round as u64 * 4, id(OUTER));
            },
        );
        nodes = page.runtime.context().world().len();
        assert!(page.runtime.context().world().contains(added));
        let done = edit(
            &mut page,
            &mut twin,
            &format!("round {round}: inner turned"),
            |_, queue| {
                queue.set_style(id(INNER), inner_style(0.45 - 0.2 * round as f32));
            },
        );
        // It clips, so its subtree (it and three rows of three) projects
        // again, and nothing else.
        assert!(
            done.update
                .accessibility
                .updated
                .iter()
                .all(|node| node.id == id(INNER) || (300..312).contains(&node.id.get())),
            "{round}: a turn of the inner box projected {:?}",
            done.update
                .accessibility
                .updated
                .iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        );
        edit(
            &mut page,
            &mut twin,
            &format!("round {round}: both turned"),
            |_, queue| {
                queue.set_style(id(INNER), inner_style(0.3));
                set_outer(queue, Some(turn(-0.22, 1.2)), None);
            },
        );
    }
    // The select opened: its list box reads the page, and projects again
    // with every turn.
    let select = page.select;
    for page in [&mut page, &mut twin] {
        page.runtime.context_mut().toggle_select(select).unwrap();
    }
    edit(&mut page, &mut twin, "opened", |_, _| {});
    for (step, angle) in [0.4_f32, -0.3].into_iter().enumerate() {
        heights[0] += 1.37;
        let (row, height) = (page.rows[0], heights[0]);
        edit(
            &mut page,
            &mut twin,
            &format!("opened, turned {step}"),
            |_, queue| {
                set_outer(queue, Some(turn(angle, 1.0)), None);
            },
        );
        edit(
            &mut page,
            &mut twin,
            &format!("opened, moved {step}"),
            |_, queue| {
                queue.set_style(row, row_style(height));
            },
        );
    }
    for page in [&mut page, &mut twin] {
        page.runtime.context_mut().toggle_select(select).unwrap();
    }
    edit(&mut page, &mut twin, "closed", |_, _| {});
    // What the outer box hands its children changes kind: a turn becomes a
    // scale, the scale another, a move, nothing, and a turn again. Two
    // scales alike but for the move keep the space.
    let scale = |factor: f32, e: f32| PaintTransform {
        a: factor,
        d: factor,
        e,
        f: 1.25,
        ..PaintTransform::default()
    };
    let kinds = [
        ("to a scale", Some(scale(1.2, 2.0)), None),
        ("the scale moved", Some(scale(1.2, 5.5)), Some(true)),
        ("another scale", Some(scale(0.8, 5.5)), None),
        ("to a move", Some(scale(1.0, 3.0)), None),
        ("to nothing", None, Some(false)),
        ("to a turn", Some(turn(0.2, 1.0)), None),
    ];
    for (name, transform, keeps) in kinds {
        let done = edit(&mut page, &mut twin, name, |_, queue| {
            set_outer(queue, transform, None)
        });
        if let Some(hit) = keeps {
            kept_with(&done, name, nodes, hit);
        }
        heights[1] += 1.37;
        let (row, height) = (page.rows[1], heights[1]);
        edit(
            &mut page,
            &mut twin,
            &format!("{name}: moved"),
            |_, queue| {
                queue.set_style(row, row_style(height));
            },
        );
    }
}

/// Text written into a row under a turned container is ordinary work: it
/// does not stand in for a turn.
#[test]
fn text_under_a_turned_container_projects_as_before() {
    let mut page = build();
    let mut twin = build();
    let label = id(950);
    edit(&mut page, &mut twin, "labelled", |page, queue| {
        queue.create(label, document_id(), NodeKind::Text);
        queue.insert(page.rows[0], label, None);
        queue.set_text(
            label,
            TextContent {
                value: "turned".into(),
            },
        );
    });
    edit(&mut page, &mut twin, "labelled and turned", |_, queue| {
        queue.set_text(
            label,
            TextContent {
                value: "turned again".into(),
            },
        );
        set_outer(queue, Some(turn(0.5, 1.0)), None);
    });
}

/// A popover opened on the page beside the turned container: its open
/// motion is a presentation overlay, so no box is placed by its offset and
/// every accessible box comes from its viewport box. The first row grown
/// in place by an uneven step resizes the container, whose turn about its
/// centre moves everything it holds on the viewport: the cells and leaves
/// of the rows that did not move project again, as a cold projection does,
/// not only the container's children.
#[test]
fn rows_moved_under_a_turn_with_a_popover_open_keep_a_cold_projection() {
    use nana_ui_runtime::Popover;
    let mut pages = [build(), build()];
    let popovers = pages.each_mut().map(|page| {
        let popover = page
            .runtime
            .context_mut()
            .create_detached_component(document_id(), Popover::new().trigger("More"))
            .unwrap();
        let mut queue = MutationQueue::new();
        queue.insert(id(PAGE), popover.stable_id(), None);
        page.runtime.context_mut().commit_mutations(queue).unwrap();
        popover
    });
    let [mut page, mut twin] = pages;
    edit(&mut page, &mut twin, "popover", |_, _| {});
    for (page, popover) in [&mut page, &mut twin].into_iter().zip(popovers) {
        page.runtime
            .context_mut()
            .update_component(popover, |popover, _| popover.open = true)
            .unwrap();
    }
    edit(&mut page, &mut twin, "opened", |_, _| {});
    let mut height = 20.0;
    for step in 0..6 {
        height += 1.37;
        let row = page.rows[0];
        edit(
            &mut page,
            &mut twin,
            &format!("opened, moved {step}"),
            |_, queue| {
                queue.set_style(row, row_style(height));
            },
        );
    }
}

/// The container turned in depth instead (a `transform_3d` with a
/// perspective): the first row grown in place moves its centre, so every
/// box it holds moves on the viewport and is the box around projected
/// corners, checked box by box.
#[test]
fn rows_grown_under_a_3d_turn_keep_a_cold_projection() {
    use nana_ui_core::PaintMat4;
    let deep = || {
        let mut style = outer_style(None, None);
        Arc::make_mut(&mut style.layout).transform_3d = Some(
            PaintMat4::perspective(800.0)
                .unwrap()
                .then(PaintMat4::rotate_y(0.5)),
        );
        style
    };
    let mut page = build();
    let mut twin = build();
    edit(&mut page, &mut twin, "turned in depth", |_, queue| {
        queue.set_style(id(OUTER), deep());
    });
    let mut height = 20.0;
    for step in 0..6 {
        height += 1.37;
        let row = page.rows[0];
        edit(
            &mut page,
            &mut twin,
            &format!("grown {step}"),
            |_, queue| {
                queue.set_style(row, row_style(height));
            },
        );
    }
}
