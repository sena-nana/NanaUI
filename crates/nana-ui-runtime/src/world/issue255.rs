//! Issue #255: the layout invalidation contract.
//!
//! A style write is classified once, against that node's own fields. A write
//! equal in effect queues nothing and counts as skipped; a real change queues
//! one typed cause that says why (`reason`, `changed_inputs`) and what may
//! follow from it (`kind`, `affected_axes`). Classification does not walk the
//! document or the node's descendants (Gate A), keeps one fixed slot per node
//! (Gate B), leaves paint, transform and accessibility writes out of layout
//! (Gate C) and skips repeated equivalent writes (Gate D). DevTools reads the
//! queued cause, and after a pass the cause the frontier admitted each node
//! with.

#![cfg(test)]

use std::sync::Arc;

use nana_ui_core::{
    BoxShadowSpec, InvalidationKind, InvalidationReason, LayoutDependencyFootprint,
    LayoutFieldMask, LayoutInvalidation, LayoutStyle, LengthSpec, PaintTransform,
    SemanticColorRole, VisibilitySpec, WorkCounters,
};

use super::reflow_oracle::{column, node, styled};
use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, LayoutViewport, MutationQueue, NodeStyle,
};

fn fixed(width: f32, height: f32) -> NodeStyle {
    styled(LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        ..LayoutStyle::default()
    })
}

/// A 240px column holding `rows` fixed 200x20 rows. Node 2 is the column,
/// nodes 3.. are the rows.
fn install_column(world: &mut UiWorld, rows: u64) -> DocumentId {
    let document = DocumentId::new(1).unwrap();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document, NodeKind::Document);
    queue.create(node(2), document, NodeKind::Element { tag: "div".into() });
    queue.insert(node(1), node(2), None);
    queue.set_style(node(2), styled(column(Some(240.0))));
    for index in 0..rows {
        let row = node(3 + index);
        queue.create(row, document, NodeKind::Element { tag: "div".into() });
        queue.insert(node(2), row, None);
        queue.set_style(row, fixed(200.0, 20.0));
    }
    world.commit(queue).unwrap();
    document
}

/// Run frames until the world has nothing left to lay out.
fn settle(context: &mut AppContext, document: DocumentId) {
    let viewport = LayoutViewport::new(320.0, 240.0);
    for _ in 0..8 {
        let work = context.take_system_work();
        if work.is_empty() {
            return;
        }
        context.resolve_styles(&work.style).unwrap();
        let mut seeds = work.layout_frontier_seeds.clone();
        seeds.extend(context.take_layout_frontier_seeds(document));
        if !seeds.is_empty() {
            context
                .layout_document_with_frontier(document, viewport, &seeds)
                .unwrap();
        }
    }
    panic!("frame did not settle");
}

fn commit_style(context: &mut AppContext, id: StableNodeId, edit: impl FnOnce(&mut NodeStyle)) {
    let mut next = context.world().node_style(id).unwrap().clone();
    edit(&mut next);
    let mut queue = MutationQueue::new();
    queue.set_style(id, next);
    context.compat_world_mut().commit(queue).unwrap();
}

fn rewrite_style(context: &mut AppContext, id: StableNodeId) {
    commit_style(context, id, |_| {});
}

/// Layout invalidations created, zero-delta writes and equivalent writes
/// skipped between two counter snapshots.
fn classified_counts(before: &WorkCounters, after: &WorkCounters) -> (usize, usize, usize) {
    (
        after.layout_invalidations_created - before.layout_invalidations_created,
        after.layout_invalidations_zero_delta - before.layout_invalidations_zero_delta,
        after.layout_equivalent_mutations_skipped - before.layout_equivalent_mutations_skipped,
    )
}

#[test]
fn issue255_equivalent_write_is_skipped_and_a_width_change_queues_one_typed_cause() {
    let mut context = AppContext::new();
    let document = install_column(context.compat_world_mut(), 1);
    settle(&mut context, document);
    let row = node(3);

    let before = context.last_work_counters();
    rewrite_style(&mut context, row);
    // The same width, spelled as a fresh style object.
    commit_style(&mut context, row, |style| {
        Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(200.0));
    });
    let after = context.last_work_counters();
    assert_eq!(classified_counts(&before, &after), (0, 2, 2));
    assert!(context.world().pending_layout_invalidation(row).is_empty());
    assert!(
        context.take_system_work().is_empty(),
        "an equivalent write schedules nothing"
    );

    let before = context.last_work_counters();
    commit_style(&mut context, row, |style| {
        Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(180.0));
    });
    let after = context.last_work_counters();
    assert_eq!(classified_counts(&before, &after), (1, 0, 0));
    let cause = context.world().pending_layout_invalidation(row);
    assert!(
        cause.reason.contains(InvalidationReason::STYLE),
        "{cause:?}"
    );
    assert!(
        cause.changed_inputs.contains(LayoutFieldMask::SIZING),
        "{cause:?}"
    );
    assert!(cause.kind.contains(InvalidationKind::MEASURE), "{cause:?}");
    assert!(
        cause
            .affected_axes
            .contains(LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE),
        "{cause:?}"
    );
}

/// DevTools explains a relayout: before the pass, the cause queued on the
/// node; after it, the cause the frontier admitted each node with, and
/// whether the node was the seed or a dependency edge reached it.
#[test]
fn issue255_devtools_explains_why_layout_reached_a_node() {
    let mut context = AppContext::new();
    let document = install_column(context.compat_world_mut(), 3);
    settle(&mut context, document);
    let row = node(3);
    let follower = node(4);

    commit_style(&mut context, row, |style| {
        Arc::make_mut(&mut style.layout).height = Some(LengthSpec::Px(30.0));
    });
    let queued = context.inspect(row).unwrap().layout.expect("queued cause");
    assert!(queued.pending && queued.seed);
    assert!(
        queued
            .invalidation
            .changed_inputs
            .contains(LayoutFieldMask::SIZING)
    );
    assert!(
        context
            .inspect(follower)
            .unwrap()
            .layout
            .is_none_or(|cause| !cause.pending),
        "nothing is queued on the follower itself"
    );

    settle(&mut context, document);
    let admitted = context.layout_cause(row).expect("the seed was laid out");
    assert!(!admitted.pending && admitted.seed);
    assert!(
        admitted
            .invalidation
            .kind
            .contains(InvalidationKind::MEASURE)
    );
    let reached = context
        .layout_cause(follower)
        .expect("the follower moved with the taller row");
    assert!(!reached.pending && !reached.seed);
    assert!(
        reached
            .invalidation
            .kind
            .contains(InvalidationKind::PLACEMENT),
        "{reached:?}"
    );
    assert!(
        !reached
            .invalidation
            .kind
            .contains(InvalidationKind::MEASURE),
        "the follower only moved: {reached:?}"
    );
}

/// What classifying one width change moves on the counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Classified {
    created: usize,
    zero_delta: usize,
    skipped: usize,
    footprints_read: usize,
    validation_nodes_scanned: Option<usize>,
}

fn classify_one_width_change(rows: u64) -> (Classified, LayoutInvalidation) {
    let mut context = AppContext::new();
    install_column(context.compat_world_mut(), rows);
    let column = node(2);
    assert_eq!(
        context.world().node(column).unwrap().children.len(),
        usize::try_from(rows).unwrap()
    );
    // Drain the install without laying out: only the write below is counted.
    let _ = context.take_system_work();
    let before = context.last_work_counters();
    commit_style(&mut context, column, |style| {
        Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(180.0));
    });
    let after = context.last_work_counters();
    let work = context.take_system_work();
    let (created, zero_delta, skipped) = classified_counts(&before, &after);
    (
        Classified {
            created,
            zero_delta,
            skipped,
            footprints_read: after.layout_dependency_edges_visited
                - before.layout_dependency_edges_visited,
            validation_nodes_scanned: work.validation_nodes_scanned,
        },
        work.layout_frontier_seeds
            .iter()
            .find(|seed| seed.node == column)
            .map(|seed| seed.invalidation)
            .unwrap_or_else(LayoutInvalidation::none),
    )
}

/// Gate A. The same width change on a column with 10k and with 100k rows is
/// classified with the same counts and the same typed cause, and reads no
/// dependency footprint: classification is the field diff of that one node.
#[test]
fn issue255_classifying_a_width_change_does_not_scale_with_the_document() {
    let (small, small_cause) = classify_one_width_change(10_000);
    let (large, large_cause) = classify_one_width_change(100_000);
    assert_eq!(small, large);
    assert_eq!(
        (
            small.created,
            small.zero_delta,
            small.skipped,
            small.footprints_read
        ),
        (1, 0, 0, 0)
    );
    assert_eq!(small_cause, large_cause);
    assert!(small_cause.changed_inputs.contains(LayoutFieldMask::SIZING));
}

/// Gate A, on published results. A layout field of the column does not hide
/// the results of its rows until the next pass: the write does not walk them.
#[test]
fn issue255_a_layout_write_leaves_descendant_results_published() {
    let mut context = AppContext::new();
    let document = install_column(context.compat_world_mut(), 2_000);
    settle(&mut context, document);
    let column = node(2);
    let last_row = node(3 + 1_999);
    assert!(context.world().layout_result(last_row).is_some());
    commit_style(&mut context, column, |style| {
        Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(220.0));
    });
    assert!(
        context.world().layout_result(column).is_none(),
        "the written node waits for its next result"
    );
    assert!(
        context.world().layout_result(last_row).is_some(),
        "a row result must not be hidden by a walk of the column's subtree"
    );
    settle(&mut context, document);
    assert!(context.world().layout_result(column).is_some());
    assert_eq!(context.world().layout_box(column).unwrap().width, 220.0);
}

fn assert_layout_uncharged(before: &WorkCounters, after: &WorkCounters, label: &str) {
    assert_eq!(
        after.layout_invalidations_created, before.layout_invalidations_created,
        "{label} must not create a layout invalidation"
    );
    assert_eq!(
        after.layout_dependency_edges_visited, before.layout_dependency_edges_visited,
        "{label} must not read a dependency footprint"
    );
    assert_eq!(
        after.layout_frontier_seeds, before.layout_frontier_seeds,
        "{label}"
    );
    assert_eq!(
        after.layout_measure_nodes, before.layout_measure_nodes,
        "{label}"
    );
    assert_eq!(
        after.layout_placement_nodes, before.layout_placement_nodes,
        "{label}"
    );
}

/// Gate C. Paint, colour, opacity, transform and accessibility writes on a
/// laid-out node queue no layout cause and read no footprint, through a
/// whole frame. A width change bundled with a colour still queues one.
#[test]
fn issue255_paint_opacity_transform_and_accessibility_stay_out_of_layout() {
    let mut context = AppContext::new();
    let document = install_column(context.compat_world_mut(), 1);
    settle(&mut context, document);
    let row = node(3);
    let laid_out = context.world().layout_box(row).unwrap();

    let writes: [(&str, Box<dyn Fn(&mut NodeStyle)>); 4] = [
        (
            "paint",
            Box::new(|style: &mut NodeStyle| {
                let layout = Arc::make_mut(&mut style.layout);
                layout.background = Some([0.1, 0.2, 0.3, 1.0]);
                layout.border_color = Some([0.2, 0.2, 0.2, 1.0]);
                layout.border_radius = Some(4.0);
                layout.paint.visibility = Some(VisibilitySpec::Hidden);
                layout.paint.box_shadows.push(BoxShadowSpec {
                    paint_color: None,
                    offset_x: 1.0,
                    offset_y: 1.0,
                    blur_radius: 2.0,
                    spread_radius: 0.0,
                    color: [0.0, 0.0, 0.0, 0.4],
                    inset: false,
                });
                style.foreground = Some(SemanticColorRole::Accent);
                style.background = Some(SemanticColorRole::Surface);
                style.border = Some(SemanticColorRole::Border);
            }),
        ),
        (
            "colour",
            Box::new(|style: &mut NodeStyle| {
                Arc::make_mut(&mut style.layout).color = Some([0.2, 0.4, 0.8, 1.0]);
            }),
        ),
        (
            "opacity",
            Box::new(|style: &mut NodeStyle| {
                Arc::make_mut(&mut style.layout).opacity = Some(0.4);
            }),
        ),
        (
            "transform",
            Box::new(|style: &mut NodeStyle| {
                Arc::make_mut(&mut style.layout).transform = Some(PaintTransform {
                    e: 8.0,
                    ..PaintTransform::default()
                });
            }),
        ),
    ];
    for (label, write) in writes {
        let before = context.last_work_counters();
        commit_style(&mut context, row, |style| write(style));
        assert!(
            context.world().pending_layout_invalidation(row).is_empty(),
            "{label}"
        );
        settle(&mut context, document);
        assert_layout_uncharged(&before, &context.last_work_counters(), label);
    }

    let before = context.last_work_counters();
    let mut queue = MutationQueue::new();
    queue.set_accessibility(
        row,
        AccessibilityState {
            role: AccessibilityRole::Button,
            label: Some(Arc::<str>::from("row")),
            disabled: true,
            ..AccessibilityState::default()
        },
    );
    context.compat_world_mut().commit(queue).unwrap();
    assert!(context.world().pending_layout_invalidation(row).is_empty());
    settle(&mut context, document);
    assert_layout_uncharged(&before, &context.last_work_counters(), "accessibility");
    assert_eq!(context.world().layout_box(row), Some(laid_out));

    let before = context.last_work_counters();
    commit_style(&mut context, row, |style| {
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(180.0));
        layout.color = Some([0.9, 0.1, 0.1, 1.0]);
    });
    let after = context.last_work_counters();
    assert_eq!(
        after.layout_invalidations_created,
        before.layout_invalidations_created + 1
    );
    assert_eq!(
        after.layout_dependency_edges_visited, before.layout_dependency_edges_visited,
        "classification reads no footprint; the frontier does, when layout runs"
    );
}

/// Gate D. One thousand effective-equivalent width writes on a 10k-row
/// column are each skipped: nothing is queued, nothing is laid out, and the
/// drain after them is empty.
#[test]
fn issue255_a_thousand_equivalent_writes_are_skipped_on_a_10k_tree() {
    let mut context = AppContext::new();
    install_column(context.compat_world_mut(), 10_000);
    let column = node(2);
    let _ = context.take_system_work();
    let before = context.last_work_counters();
    for _ in 0..1000 {
        commit_style(&mut context, column, |style| {
            Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(240.0));
        });
    }
    let after = context.last_work_counters();
    assert_eq!(classified_counts(&before, &after), (0, 1000, 1000));
    assert_eq!(after.layout_frontier_seeds, before.layout_frontier_seeds);
    assert_eq!(
        after.layout_frontier_nodes_measure,
        before.layout_frontier_nodes_measure
    );
    assert_eq!(
        after.layout_frontier_nodes_placement,
        before.layout_frontier_nodes_placement
    );
    assert_eq!(context.layout_invocations(), 0);
    let idle = context.take_system_work();
    assert!(idle.is_empty());
    assert_eq!(idle.validation_nodes_scanned, Some(0));
}

/// Bytes the pending-cause table holds: its buckets, each one id and one
/// [`LayoutInvalidation`] plus the control byte, whether full or not.
fn pending_table_bytes(world: &UiWorld) -> usize {
    world.pending_layout_invalidations.capacity()
        * (std::mem::size_of::<(StableNodeId, LayoutInvalidation)>() + 1)
}

/// Gate B. Dependency metadata is one fixed cause per node, at most 64 bytes
/// with the table that holds it, on a 100k-node tree where every node has a
/// cause pending. Two thousand further writes to one node add nothing.
#[test]
fn issue255_dependency_metadata_stays_one_bounded_slot_per_node_at_100k() {
    assert!(
        std::mem::size_of::<LayoutInvalidation>() <= 16,
        "a typed cause is a few masks, not a node list"
    );
    assert_eq!(std::mem::size_of::<LayoutDependencyFootprint>(), 2);

    let mut context = AppContext::new();
    install_column(context.compat_world_mut(), 100_000);
    let column = node(2);
    let nodes = context.world().len();
    let slots = context.world().pending_layout_invalidations.len();
    assert!(slots <= nodes);
    let bytes = pending_table_bytes(context.world());
    assert!(
        bytes <= nodes * 64,
        "{bytes} bytes of pending causes for {nodes} nodes is above 64 bytes a node"
    );

    for width in [180.0, 160.0].into_iter().cycle().take(1000) {
        commit_style(&mut context, column, |style| {
            Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(width));
        });
        rewrite_style(&mut context, column);
    }
    assert_eq!(
        context.world().pending_layout_invalidations.len(),
        slots,
        "repeat writes merge into the node's one slot"
    );
    assert_eq!(pending_table_bytes(context.world()), bytes);
    assert_eq!(context.layout_invocations(), 0);
}
