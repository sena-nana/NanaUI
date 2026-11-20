//! The layout recursions' stack (see `recursion`): one level stays within its
//! budget, and no tree is too deep for the thread laying it out.
#![cfg(test)]

use std::sync::Arc;

use nana_ui_core::{
    DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec, OverflowSpec, PositionSpec,
};

use crate::layout_engine::{LayoutExecutionStats, RetainedLayoutCache, RuntimeLayoutEngine};
use crate::{
    AppContext, DocumentId, LayoutViewport, MutationQueue, NodeKind, NodeStyle, StableNodeId,
    Stack, TextContent, UiWorld,
};

/// Most stack one level of each recursion may take, in bytes. Unoptimized
/// code keeps a slot for every local of a function, so a debug level costs
/// several times a release one.
///
/// Measured on x86_64 Windows (MSVC): a debug level was 25.8 KiB of placement
/// and 12.2 KiB of measure while `place_node_scoped` held the whole child walk
/// in one frame; with the walk and the plan paths in their own frames it is
/// 9.0 KiB of placement. Release: 4.8 KiB, then 3.2 KiB, and 2.5 KiB of
/// measure. Each budget leaves about a quarter of headroom over that.
///
/// On aarch64 macOS a release placement level was 2.7 KiB, and is 3.5 KiB
/// since a level splits into the head that writes its box and the body that
/// places its children (a boundary resumes at the body); debug placement is
/// 10.9 KiB and measure 11.9 KiB. Placement no longer runs deeper than
/// `recursion::SPAN` levels, so its level price bounds its whole stack.
///
/// The replays placed children from their own frames, so a replayed level
/// cost more than a cold one: on aarch64 macOS up to 12.0 KiB in debug and
/// 4.3 KiB in release (over this budget) for a child-list edit, against
/// 5.1 / 2.7 KiB cold. Since they hand their children back to the body as
/// `Descend` too, a replayed level is a cold one, 5.3 KiB in debug and
/// 2.8 KiB in release; one that also places a positioned child, which still
/// enters it from its own frame, 8.9 / 3.5 KiB.
struct Budget {
    measure: usize,
    placement: usize,
}

const BUDGET: Budget = if cfg!(debug_assertions) {
    Budget {
        measure: 16 << 10,
        placement: 12 << 10,
    }
} else {
    Budget {
        measure: 4 << 10,
        placement: 4 << 10,
    }
};

/// The containers a level can be: flex, grid, layers.
const DISPLAYS: [DisplaySpec; 3] = [DisplaySpec::Flex, DisplaySpec::Grid, DisplaySpec::Layers];

/// `depth` boxes inside one another, cycling through `displays`, laid out in
/// full.
fn lay_out_nested(depth: usize, displays: &[DisplaySpec]) -> LayoutExecutionStats {
    let document = DocumentId::new(7).unwrap();
    let mut cx = AppContext::new();
    let mut parent = cx
        .create_component(document, Stack::column(0.0).width(LengthSpec::Px(800.0)))
        .unwrap();
    for level in 0..depth {
        let display = displays[level % displays.len()];
        let mut layout = LayoutStyle {
            display: Some(display),
            direction: Some(FlexDirection::Column),
            padding: Some(LengthSpec::Px(0.25)),
            min_height: Some(LengthSpec::Px(1.0)),
            ..Default::default()
        };
        if display == DisplaySpec::Grid {
            layout.grid.columns = Some(vec![GridTrack::Fr(1.0)]);
        }
        let child = cx
            .create_detached_component(document, Stack::from_layout(layout))
            .unwrap();
        cx.append_child(parent, child).unwrap();
        parent = child;
    }
    cx.layout_document(document, LayoutViewport::new(800.0, 600.0))
        .unwrap();
    let stats = cx.layout_execution_stats(document);
    // Tear the tree down here: dropping it is as deep as building it.
    drop(cx);
    stats
}

#[test]
fn a_layout_level_stays_within_its_stack_budget() {
    if crate::stack::remaining().is_none() {
        // The platform cannot tell what is left, so nothing is priced.
        return;
    }
    for display in DISPLAYS {
        let stats = lay_out_nested(48, &[display]);
        let (measure, placement) = (
            stats.measure_stack_per_level,
            stats.placement_stack_per_level,
        );
        assert!(
            placement > 0,
            "{display:?} unpriced: {measure} / {placement}"
        );
        assert!(
            measure <= BUDGET.measure,
            "a {display:?} measure level takes {measure} bytes, over its {} budget",
            BUDGET.measure
        );
        assert!(
            placement <= BUDGET.placement,
            "a {display:?} placement level takes {placement} bytes, over its {} budget",
            BUDGET.placement
        );
    }
}

#[test]
fn a_tree_deeper_than_the_stack_still_lays_out() {
    // Unguarded, 400 levels need several MiB of stack in a debug build;
    // this thread has the 1 MiB of a Windows main thread.
    let stats = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| lay_out_nested(400, &DISPLAYS))
        .unwrap()
        .join()
        .expect("layout ran out of stack");
    assert!(stats.placement_nodes > 400);
}

#[test]
fn max_tree_depth_fits_in_384_kib_stack() {
    let stats = std::thread::Builder::new()
        .stack_size(384 * 1024)
        .spawn(|| lay_out_nested(511, &DISPLAYS))
        .unwrap()
        .join()
        .expect("512-node deep layout ran out of 384 KiB stack");
    assert!(stats.placement_nodes >= 512);
}

/// A level's price is reported, not only checked: `cargo test -p
/// nana-ui-runtime stack_budget -- --nocapture` prints it.
#[test]
fn report_layout_stack_per_level() {
    for display in DISPLAYS {
        let stats = lay_out_nested(48, &[display]);
        println!(
            "layout stack per level, {display:?} ({}): measure {} B, placement {} B",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            stats.measure_stack_per_level,
            stats.placement_stack_per_level,
        );
    }
}

/// How each level of an incremental tree is laid out, and what stands beside
/// the level nested in it.
#[derive(Clone, Copy)]
struct Route {
    name: &'static str,
    container: fn() -> LayoutStyle,
    /// An absolutely positioned child at every level.
    positioned: bool,
}

fn column() -> LayoutStyle {
    LayoutStyle {
        display: Some(DisplaySpec::Flex),
        direction: Some(FlexDirection::Column),
        padding: Some(LengthSpec::Px(0.25)),
        ..Default::default()
    }
}

/// The incremental routes a pass can take through a container whose child
/// changed: a sequential replay of its plan (forward and reversed), a grid
/// replay, a scroll container, and a plan that also places positioned
/// children.
const ROUTES: [Route; 6] = [
    Route {
        name: "column",
        container: column,
        positioned: false,
    },
    Route {
        name: "column-reverse",
        container: || LayoutStyle {
            flex_reverse: true,
            ..column()
        },
        positioned: false,
    },
    Route {
        name: "grid",
        container: || {
            let mut style = LayoutStyle {
                display: Some(DisplaySpec::Grid),
                ..column()
            };
            style.grid.columns = Some(vec![GridTrack::Fr(1.0)]);
            style
        },
        positioned: false,
    },
    Route {
        name: "layers",
        container: || LayoutStyle {
            display: Some(DisplaySpec::Layers),
            ..column()
        },
        positioned: false,
    },
    Route {
        name: "scroll",
        container: || LayoutStyle {
            overflow_y: OverflowSpec::Auto,
            ..column()
        },
        positioned: false,
    },
    Route {
        name: "absolute",
        container: || LayoutStyle {
            position: PositionSpec::Relative,
            ..column()
        },
        positioned: true,
    },
];

/// Levels of the incremental trees: many more than a span, so every level
/// below the first is placed by a parent that replayed.
const INCREMENTAL_DEPTH: u64 = 24;

fn node(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}

fn styled(layout: LayoutStyle) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        ..NodeStyle::default()
    }
}

fn container_id(level: u64) -> StableNodeId {
    node(10 + level * 4)
}

/// The text leaf at the bottom of the tree.
fn text_leaf() -> StableNodeId {
    node(10 + INCREMENTAL_DEPTH * 4)
}

/// The sized leaf beside the text leaf, whose height the edits change.
fn sized_leaf() -> StableNodeId {
    node(11 + INCREMENTAL_DEPTH * 4)
}

/// The leaf an edit adds to, and then takes from, the deepest container.
fn arriving_leaf() -> StableNodeId {
    node(12 + INCREMENTAL_DEPTH * 4)
}

fn leaf(height: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(40.0)),
        height: Some(LengthSpec::Px(height)),
        ..Default::default()
    }
}

/// `INCREMENTAL_DEPTH` containers of `route` inside one another, each with a
/// sized sibling after the level it holds (and a positioned one where the
/// route has it); the deepest holds a text leaf and a sized leaf.
fn incremental_tree(route: Route) -> (UiWorld, DocumentId) {
    let document = DocumentId::new(7).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document, NodeKind::Document);
    queue.create(node(2), document, NodeKind::Element { tag: "div".into() });
    queue.insert(node(1), node(2), None);
    queue.set_style(
        node(2),
        styled(LayoutStyle {
            width: Some(LengthSpec::Px(800.0)),
            direction: Some(FlexDirection::Column),
            ..Default::default()
        }),
    );
    let mut parent = node(2);
    for level in 0..INCREMENTAL_DEPTH {
        let container = container_id(level);
        queue.create(container, document, NodeKind::Element { tag: "div".into() });
        queue.insert(parent, container, None);
        queue.set_style(container, styled((route.container)()));
        if level > 0 {
            let sibling = node(11 + (level - 1) * 4);
            queue.create(sibling, document, NodeKind::Element { tag: "div".into() });
            queue.insert(parent, sibling, None);
            queue.set_style(sibling, styled(leaf(6.0)));
            if route.positioned {
                let positioned = node(12 + (level - 1) * 4);
                queue.create(
                    positioned,
                    document,
                    NodeKind::Element { tag: "div".into() },
                );
                queue.insert(parent, positioned, None);
                queue.set_style(
                    positioned,
                    styled(LayoutStyle {
                        position: PositionSpec::Absolute,
                        offset_bottom: Some(LengthSpec::Px(0.0)),
                        offset_right: Some(LengthSpec::Px(0.0)),
                        ..leaf(4.0)
                    }),
                );
            }
        }
        parent = container;
    }
    queue.create(text_leaf(), document, NodeKind::Text);
    queue.insert(parent, text_leaf(), None);
    queue.set_text(
        text_leaf(),
        TextContent {
            value: "leaf".into(),
        },
    );
    queue.create(
        sized_leaf(),
        document,
        NodeKind::Element { tag: "div".into() },
    );
    queue.insert(parent, sized_leaf(), None);
    queue.set_style(sized_leaf(), styled(leaf(10.0)));
    world.commit(queue).unwrap();
    (world, document)
}

/// Commits `queue` and lays the document out from the seeds it planted.
fn relayout(
    world: &mut UiWorld,
    document: DocumentId,
    retained: &mut RetainedLayoutCache,
    queue: MutationQueue,
) -> LayoutExecutionStats {
    let viewport = LayoutViewport::new(800.0, 600.0);
    world.commit(queue).unwrap();
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    // Shaping a changed text plants the seed its new size needs, as in a
    // frame.
    world
        .shape_text(&work.text, &mut crate::MeasureTextShaper)
        .unwrap();
    let mut seeds = work.layout_frontier_seeds.clone();
    seeds.extend(world.take_layout_frontier_seeds(document));
    let force_full = retained.documents.is_empty();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(world, document, viewport, &seeds, retained, force_full)
        .unwrap();
    let mut written = MutationQueue::new();
    for (id, layout) in emitted {
        if world.layout_box(id) != Some(layout) {
            written.write_layout(id, layout);
        }
    }
    world.commit(written).unwrap();
    let _ = world.take_system_work();
    retained.execution_stats(document)
}

/// An edit the incremental passes make at the bottom of the tree, which
/// every level above it replays.
struct Edit {
    name: &'static str,
    apply: fn(&mut MutationQueue, DocumentId),
}

const EDITS: [Edit; 4] = [
    Edit {
        name: "leaf resized",
        apply: |queue, _| queue.set_style(sized_leaf(), styled(leaf(14.0))),
    },
    Edit {
        name: "child added",
        apply: |queue, document| {
            queue.create(
                arriving_leaf(),
                document,
                NodeKind::Element { tag: "div".into() },
            );
            queue.insert(container_id(INCREMENTAL_DEPTH - 1), arriving_leaf(), None);
            queue.set_style(arriving_leaf(), styled(leaf(8.0)));
        },
    },
    Edit {
        name: "child removed",
        apply: |queue, _| queue.despawn_subtree(arriving_leaf()),
    },
    Edit {
        name: "text changed",
        apply: |queue, _| {
            queue.set_text(
                text_leaf(),
                TextContent {
                    value: "a longer leaf\nover two lines".into(),
                },
            )
        },
    },
];

/// Lays each route's tree out, then makes each edit incrementally, and
/// hands every incremental pass's stats to `check`.
fn each_incremental_pass(mut check: impl FnMut(Route, &Edit, LayoutExecutionStats)) {
    for route in ROUTES {
        let (mut world, document) = incremental_tree(route);
        let mut retained = RetainedLayoutCache::default();
        relayout(&mut world, document, &mut retained, MutationQueue::new());
        for edit in &EDITS {
            let mut queue = MutationQueue::new();
            (edit.apply)(&mut queue, document);
            let stats = relayout(&mut world, document, &mut retained, queue);
            check(route, edit, stats);
        }
    }
}

#[test]
fn an_incremental_layout_level_stays_within_its_stack_budget() {
    if crate::stack::remaining().is_none() {
        return;
    }
    each_incremental_pass(|route, edit, stats| {
        let placement = stats.placement_stack_per_level;
        // The pass replayed: its levels reused their plans rather than
        // walking their children, so the replay is what is priced.
        assert!(
            stats.placement_plans_reused > 0,
            "{} / {}: no plan was replayed",
            route.name,
            edit.name
        );
        assert!(placement > 0, "{} / {}: unpriced", route.name, edit.name);
        assert!(
            placement <= BUDGET.placement,
            "{} / {}: an incremental placement level takes {placement} bytes, over its {} budget",
            route.name,
            edit.name,
            BUDGET.placement
        );
    });
}

/// `cargo test -p nana-ui-runtime stack_budget -- --nocapture` prints what
/// an incremental level costs on each route.
#[test]
fn report_incremental_layout_stack_per_level() {
    each_incremental_pass(|route, edit, stats| {
        println!(
            "incremental placement per level, {} / {} ({}): {} B, plans reused {}, suffixes {}, boundaries {}",
            route.name,
            edit.name,
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            stats.placement_stack_per_level,
            stats.placement_plans_reused,
            stats.suffixes_replayed,
            stats.placement_boundaries,
        );
    });
}
