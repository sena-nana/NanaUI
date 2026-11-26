//! The cold reflow oracle the incremental-reflow gates share (Issue #259),
//! and the plumbing their fixtures share: ids and styles, a builder that
//! hands out ids in order, the product frame, and what the last layout pass
//! reached.
//!
//! An incremental frame is correct when the same canonical inputs laid out
//! from nothing give the same geometry. [`assert_matches_cold`] compares two
//! contexts built from the same inputs, one driven through incremental
//! frames and one laid out once: every box, every published result (bounds,
//! baselines, fragments, overflow, scroll extent, containing block and the
//! rest of [`crate::LayoutResult::geometry_eq`]), every hit entry and every
//! accessibility bound and clip.
//!
//! The per-pass guard in `layout_engine::verify` checks boxes after every
//! retained pass; this oracle checks what a pass publishes, against a world
//! that never ran an incremental frame, so state a pass keeps between frames
//! (shaped text, plans, retained sizes) cannot hide in both sides.

#![cfg(test)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nana_text::NativeTextEngine;
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily};
use nana_ui_core::{FlexDirection, InvalidationKind, LayoutStyle, LengthSpec};

use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::{
    AppContext, LayoutViewport, MutationQueue, NanaTextEngineShaper, NodeStyle, TextContent,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct FrameStats {
    pub(super) execution: crate::layout_engine::LayoutExecutionStats,
    pub(super) frontier: crate::LayoutFrontierStats,
    pub(super) text: nana_text::TextWorkCounters,
    pub(super) layout_passes: usize,
    pub(super) entities_spawned: usize,
    pub(super) entities_despawned: usize,
}

impl FrameStats {
    fn record_layout_pass(
        &mut self,
        execution: crate::layout_engine::LayoutExecutionStats,
        frontier: crate::LayoutFrontierStats,
    ) {
        self.layout_passes = self.layout_passes.saturating_add(1);
        self.execution.measure_nodes = self
            .execution
            .measure_nodes
            .saturating_add(execution.measure_nodes);
        self.execution.measure_cache_hits = self
            .execution
            .measure_cache_hits
            .saturating_add(execution.measure_cache_hits);
        self.execution.measure_cache_misses = self
            .execution
            .measure_cache_misses
            .saturating_add(execution.measure_cache_misses);
        self.execution.placement_nodes = self
            .execution
            .placement_nodes
            .saturating_add(execution.placement_nodes);
        self.execution.origin_only_updates = self
            .execution
            .origin_only_updates
            .saturating_add(execution.origin_only_updates);
        self.execution.moved_subtrees_kept = self
            .execution
            .moved_subtrees_kept
            .saturating_add(execution.moved_subtrees_kept);
        self.execution.placement_plans_reused = self
            .execution
            .placement_plans_reused
            .saturating_add(execution.placement_plans_reused);
        self.execution.measure_plans_reused = self
            .execution
            .measure_plans_reused
            .saturating_add(execution.measure_plans_reused);
        self.execution.suffixes_replayed = self
            .execution
            .suffixes_replayed
            .saturating_add(execution.suffixes_replayed);
        self.execution.children_measured = self
            .execution
            .children_measured
            .saturating_add(execution.children_measured);
        self.execution.inline_reflows = self
            .execution
            .inline_reflows
            .saturating_add(execution.inline_reflows);
        self.execution.inline_atomic_measure_requests = self
            .execution
            .inline_atomic_measure_requests
            .saturating_add(execution.inline_atomic_measure_requests);
        self.execution.containers_uncacheable = self
            .execution
            .containers_uncacheable
            .saturating_add(execution.containers_uncacheable);
        self.execution.plan_misses = self
            .execution
            .plan_misses
            .saturating_add(execution.plan_misses);
        self.execution.plan_rebuilds = self
            .execution
            .plan_rebuilds
            .saturating_add(execution.plan_rebuilds);
        self.execution.local_context_fallbacks = self
            .execution
            .local_context_fallbacks
            .saturating_add(execution.local_context_fallbacks);
        self.execution.retain_sweeps = self
            .execution
            .retain_sweeps
            .saturating_add(execution.retain_sweeps);
        self.execution.measure_stack_per_level = self
            .execution
            .measure_stack_per_level
            .max(execution.measure_stack_per_level);
        self.execution.placement_stack_per_level = self
            .execution
            .placement_stack_per_level
            .max(execution.placement_stack_per_level);
        self.execution.placement_boundaries = self
            .execution
            .placement_boundaries
            .saturating_add(execution.placement_boundaries);
        self.execution.line_boxes_settled = self
            .execution
            .line_boxes_settled
            .saturating_add(execution.line_boxes_settled);

        self.frontier.seeds = self.frontier.seeds.saturating_add(frontier.seeds);
        self.frontier.seed_merges = self
            .frontier
            .seed_merges
            .saturating_add(frontier.seed_merges);
        self.frontier.nodes_measure = self
            .frontier
            .nodes_measure
            .saturating_add(frontier.nodes_measure);
        self.frontier.nodes_placement = self
            .frontier
            .nodes_placement
            .saturating_add(frontier.nodes_placement);
        self.frontier.contexts = self.frontier.contexts.saturating_add(frontier.contexts);
        self.frontier.nodes_writing = self
            .frontier
            .nodes_writing
            .saturating_add(frontier.nodes_writing);
        self.frontier.dependency_edges_visited = self
            .frontier
            .dependency_edges_visited
            .saturating_add(frontier.dependency_edges_visited);
        self.frontier.propagations_stopped = self
            .frontier
            .propagations_stopped
            .saturating_add(frontier.propagations_stopped);
        self.frontier.local_subtree_fallbacks = self
            .frontier
            .local_subtree_fallbacks
            .saturating_add(frontier.local_subtree_fallbacks);
        self.frontier.full_document_fallbacks = self
            .frontier
            .full_document_fallbacks
            .saturating_add(frontier.full_document_fallbacks);
        self.frontier.graph_edges = self
            .frontier
            .graph_edges
            .saturating_add(frontier.graph_edges);
        self.frontier.scratch_entries = self
            .frontier
            .scratch_entries
            .saturating_add(frontier.scratch_entries);
        self.frontier.constraint.seeds = self
            .frontier
            .constraint
            .seeds
            .saturating_add(frontier.constraint.seeds);
        self.frontier.constraint.considered = self
            .frontier
            .constraint
            .considered
            .saturating_add(frontier.constraint.considered);
        self.frontier.constraint.remeasured = self
            .frontier
            .constraint
            .remeasured
            .saturating_add(frontier.constraint.remeasured);
    }
}

pub(super) fn node(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}

pub(super) fn styled(layout: LayoutStyle) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        ..NodeStyle::default()
    }
}

/// A column `width` wide, or as wide as what it holds.
pub(super) fn column(width: Option<f32>) -> LayoutStyle {
    LayoutStyle {
        width: width.map(LengthSpec::Px),
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    }
}

/// A fixed `width` x `height` row.
pub(super) fn fixed(width: f32, height: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        direction: Some(FlexDirection::Row),
        ..LayoutStyle::default()
    }
}

/// Cards in a group of [`Builder::filler`].
pub(super) const FILLER_CARDS: u64 = 10;
/// Nodes in a group of [`Builder::filler`]: the group, each card and its
/// label.
pub(super) const FILLER_GROUP_NODES: u64 = 1 + 2 * FILLER_CARDS;

/// One document's nodes, ids handed out in order.
pub(super) struct Builder {
    pub(super) queue: MutationQueue,
    document: DocumentId,
    next: u64,
}

impl Builder {
    /// A document node at id `first`; the nodes after it take the ids that
    /// follow.
    pub(super) fn new(document: DocumentId, first: u64) -> (Self, StableNodeId) {
        let mut builder = Self {
            queue: MutationQueue::new(),
            document,
            next: first,
        };
        let root = builder.take();
        builder.queue.create(root, document, NodeKind::Document);
        (builder, root)
    }

    /// [`Self::new`], and a page column `width` wide under the document.
    pub(super) fn page(document: DocumentId, first: u64, width: f32) -> (Self, StableNodeId) {
        let (mut builder, root) = Self::new(document, first);
        let page = builder.element(root, column(Some(width)));
        (builder, page)
    }

    fn take(&mut self) -> StableNodeId {
        self.next += 1;
        node(self.next - 1)
    }

    /// An element with no parent yet. Until it is inserted it is a root of
    /// the document, laid out and painted as one; one meant to stay out of
    /// the tree is detached as well.
    pub(super) fn detached(&mut self, layout: LayoutStyle) -> StableNodeId {
        let id = self.take();
        self.queue
            .create(id, self.document, NodeKind::Element { tag: "div".into() });
        self.queue.set_style(id, styled(layout));
        id
    }

    pub(super) fn element(&mut self, parent: StableNodeId, layout: LayoutStyle) -> StableNodeId {
        let id = self.take();
        self.queue
            .create(id, self.document, NodeKind::Element { tag: "div".into() });
        self.queue.insert(parent, id, None);
        self.queue.set_style(id, styled(layout));
        id
    }

    pub(super) fn label(&mut self, parent: StableNodeId, value: &str) -> StableNodeId {
        let id = self.take();
        self.queue.create(id, self.document, NodeKind::Text);
        self.queue.insert(parent, id, None);
        self.queue.set_text(
            id,
            TextContent {
                value: value.into(),
            },
        );
        id
    }

    /// `groups` groups of [`FILLER_CARDS`] fixed 200x40 cards, each with the
    /// label "one".
    pub(super) fn filler(&mut self, parent: StableNodeId, groups: u64) {
        for _ in 0..groups {
            let group = self.element(parent, column(None));
            for _ in 0..FILLER_CARDS {
                let card = self.element(group, fixed(200.0, 40.0));
                self.label(card, "one");
            }
        }
    }
}

/// The product text engine over the bundled UI face alone, so a label
/// measures the same on every machine.
pub(super) fn bundled_face_shaper() -> NanaTextEngineShaper {
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../nana-ui-core/assets/fonts/NotoSansSC-Variable.ttf"
            ),
            &FaceDescriptor::default(),
        )
        .expect("the bundled UI face registers");
    NanaTextEngineShaper::new(Arc::new(Mutex::new(NativeTextEngine::new(fonts))))
}

/// One product frame, in the order `RuntimeDocument::flush` runs it: shape
/// scheduled text, merge what shaping queued, lay out, re-shape the laid-out
/// scope. Returns the frame's layout and text stats; an idle frame returns zero.
pub(super) fn product_frame(
    context: &mut AppContext,
    document: DocumentId,
    viewport: LayoutViewport,
    shaper: &mut NanaTextEngineShaper,
) -> FrameStats {
    frame(context, document, viewport, shaper, false, None, None)
}

/// [`product_frame`], and the nodes every layout pass of it was seeded at.
pub(super) fn product_frame_seeded(
    context: &mut AppContext,
    document: DocumentId,
    viewport: LayoutViewport,
    shaper: &mut NanaTextEngineShaper,
) -> (FrameStats, HashSet<StableNodeId>) {
    let mut seeded = HashSet::new();
    let stats = frame(
        context,
        document,
        viewport,
        shaper,
        false,
        Some(&mut seeded),
        None,
    );
    (stats, seeded)
}

/// A product frame whose viewport changed: its first layout is the viewport
/// relayout, with the seeds the frame drained, as `RuntimeDocument::flush`
/// runs it.
pub(super) fn resize_frame(
    context: &mut AppContext,
    document: DocumentId,
    viewport: LayoutViewport,
    shaper: &mut NanaTextEngineShaper,
) -> FrameStats {
    frame(context, document, viewport, shaper, true, None, None)
}

/// A [`product_frame`] (after a viewport change when `resized`, as
/// [`resize_frame`]), and the accessibility work it drained: what a host
/// hands [`UiWorld::project_accessibility_delta`] to update its tree.
pub(super) fn accessible_frame(
    context: &mut AppContext,
    document: DocumentId,
    viewport: LayoutViewport,
    shaper: &mut NanaTextEngineShaper,
    resized: bool,
) -> crate::SystemWork {
    let mut drained = crate::SystemWork::default();
    frame(
        context,
        document,
        viewport,
        shaper,
        resized,
        None,
        Some(&mut drained),
    );
    drained
}

fn frame(
    context: &mut AppContext,
    document: DocumentId,
    viewport: LayoutViewport,
    shaper: &mut NanaTextEngineShaper,
    mut resized: bool,
    mut seeded: Option<&mut HashSet<StableNodeId>>,
    mut accessible: Option<&mut crate::SystemWork>,
) -> FrameStats {
    context.world_mut().observe_text_shaper(shaper);
    context.begin_frame_profile();
    let mut drained_any = false;
    let mut stats = FrameStats::default();
    // Nodes some pass marked INPUT for more than a layout move, as
    // `RuntimeDocument::flush` keeps them apart.
    let mut seeded_input = HashSet::new();
    for _ in 0..8 {
        let work = context.take_system_work_for(document);
        stats.entities_spawned = stats.entities_spawned.saturating_add(work.entities_spawned);
        stats.entities_despawned = stats
            .entities_despawned
            .saturating_add(work.entities_despawned);
        drained_any |= !work.is_empty() || resized;
        if let Some(drained) = accessible.as_deref_mut() {
            drained.generation = work.generation;
            drained.accessibility.extend(&work.accessibility);
            drained
                .accessibility_removals
                .extend(&work.accessibility_removals);
            drained.input_hit_test.extend(&work.input_hit_test);
            drained.transform.extend(&work.transform);
            drained.transform_kept.extend(&work.transform_kept);
            drained.layout_moved.extend(&work.layout_moved);
            drained.layout_reshaped.extend(&work.layout_reshaped);
            for id in &work.input_hit_test {
                if work.accessibility_moved.binary_search(id).is_ok() {
                    drained.accessibility_moved.push(*id);
                } else if work.accessibility_scrolled.binary_search(id).is_ok() {
                    drained.accessibility_scrolled.push(*id);
                } else {
                    seeded_input.insert(*id);
                }
            }
        }
        if work.is_empty() && !resized {
            break;
        }
        context.resolve_styles(&work.style).unwrap();
        context.shape_text(&work.text, shaper).unwrap();
        let mut seeds = work.layout_frontier_seeds.clone();
        seeds.extend(context.take_layout_frontier_seeds(document));
        if let Some(seeded) = seeded.as_deref_mut() {
            seeded.extend(seeds.iter().map(|seed| seed.node));
        }
        if resized {
            context
                .layout_document_for_viewport(document, viewport, &seeds)
                .unwrap();
            resized = false;
            stats.record_layout_pass(
                context.layout_execution_stats(document),
                context.layout_frontier_stats(document),
            );
        } else if seeds.is_empty() {
            continue;
        } else {
            context
                .layout_document_with_frontier(document, viewport, &seeds)
                .unwrap();
            stats.record_layout_pass(
                context.layout_execution_stats(document),
                context.layout_frontier_stats(document),
            );
        }
        let mut scope = context.take_last_layout_scope();
        for _ in 0..4 {
            if !context
                .shape_text_for_layout_scoped(&scope, shaper)
                .unwrap()
            {
                break;
            }
            let seeds = context.take_layout_frontier_seeds(document);
            if seeds.is_empty() {
                break;
            }
            if let Some(seeded) = seeded.as_deref_mut() {
                seeded.extend(seeds.iter().map(|seed| seed.node));
            }
            context
                .layout_document_with_frontier(document, viewport, &seeds)
                .unwrap();
            stats.record_layout_pass(
                context.layout_execution_stats(document),
                context.layout_frontier_stats(document),
            );
            scope = context.take_last_layout_scope();
        }
    }
    context.finish_frame_profile();
    if let Some(drained) = accessible {
        drained
            .accessibility_moved
            .retain(|id| !seeded_input.contains(id));
        drained.accessibility_moved.sort_unstable();
        drained.accessibility_moved.dedup();
        drained.accessibility_scrolled.sort_unstable();
        drained.accessibility_scrolled.dedup();
        // A box one pass moved and another scrolled is neither alone: its
        // subtree projects again.
        let moved = std::mem::take(&mut drained.accessibility_moved);
        let scrolled = std::mem::take(&mut drained.accessibility_scrolled);
        drained.accessibility_scrolled = scrolled
            .iter()
            .copied()
            .filter(|id| !seeded_input.contains(id) && moved.binary_search(id).is_err())
            .collect();
        drained.accessibility_moved = moved
            .into_iter()
            .filter(|id| scrolled.binary_search(id).is_err())
            .collect();
        drained.layout_moved.sort_unstable();
        drained.layout_moved.dedup();
        drained.layout_reshaped.sort_unstable();
        drained.layout_reshaped.dedup();
        drained.transform.sort_unstable();
        drained.transform.dedup();
        drained.transform_kept.sort_unstable();
        drained.transform_kept.dedup();
    }
    if drained_any {
        stats.text = context.world().last_text_work_counters();
    }
    stats
}

/// Whether the last layout pass of `id`'s document admitted it.
pub(super) fn admitted(context: &AppContext, id: StableNodeId) -> bool {
    context.layout_cause(id).is_some_and(|cause| !cause.pending)
}

/// Whether the last layout pass of `id`'s document measured it.
pub(super) fn measured(context: &AppContext, id: StableNodeId) -> bool {
    context
        .layout_cause(id)
        .filter(|cause| !cause.pending)
        .is_some_and(|cause| {
            cause
                .invalidation
                .kind
                .intersects(InvalidationKind::MEASURE)
        })
}

/// Nodes the last layout pass of `document` admitted to its frontier.
pub(super) fn admitted_nodes(context: &AppContext, document: DocumentId) -> HashSet<StableNodeId> {
    context
        .world()
        .document_order(document)
        .into_iter()
        .filter(|id| admitted(context, *id))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct HitGeometry {
    layout: crate::LayoutBox,
    transform: [f32; 6],
    /// Where the entry's own coordinates stand from the window.
    offset: [f32; 2],
}

fn hit_geometry(world: &UiWorld, document: DocumentId, id: StableNodeId) -> Option<HitGeometry> {
    let index = world.hit_index.hit_test_index.get(&document)?;
    let hit = index.entries.get(&id)?;
    Some(HitGeometry {
        layout: hit.entry.layout,
        transform: super::hit_test::find_hit_transform(index, id)?,
        offset: index.entry_offset(id),
    })
}

/// A node's accessible bounds and what its clip chain leaves of them.
fn accessibility_bounds(
    world: &UiWorld,
    id: StableNodeId,
) -> Option<(crate::LayoutBox, Option<crate::LayoutBox>)> {
    world
        .project_accessibility_nodes(&[id])
        .into_iter()
        .find(|node| node.id == id)
        .map(|node| (node.bounds, node.clip))
}

/// Every box, published result, hit entry and accessibility bound of
/// `incremental` equals `cold`, the same inputs laid out once.
pub(super) fn assert_matches_cold(
    incremental: &mut AppContext,
    cold: &mut AppContext,
    document: DocumentId,
) {
    incremental.world_mut().rebuild_hit_test(document);
    cold.world_mut().rebuild_hit_test(document);
    let ours = incremental.world();
    let theirs = cold.world();
    let order = ours.document_order(document);
    assert_eq!(order, theirs.document_order(document));
    for id in order {
        // As kept, from each box's frame, and on the page.
        assert_eq!(
            (ours.local_layout_box(id), ours.layout_frame(id)),
            (theirs.local_layout_box(id), theirs.layout_frame(id)),
            "local box of {id:?}"
        );
        assert_eq!(ours.layout_box(id), theirs.layout_box(id), "box of {id:?}");
        match (ours.local_layout_result(id), theirs.local_layout_result(id)) {
            (Some(ours), Some(theirs)) => assert!(
                ours.geometry_eq(&theirs),
                "local result of {id:?}: {ours:?} != {theirs:?}"
            ),
            (None, None) => {}
            (ours, theirs) => panic!("local result of {id:?}: {ours:?} != {theirs:?}"),
        }
        match (ours.layout_result(id), theirs.layout_result(id)) {
            (Some(ours), Some(theirs)) => assert!(
                ours.geometry_eq(&theirs),
                "result of {id:?}: {ours:?} != {theirs:?}"
            ),
            (None, None) => {}
            (ours, theirs) => panic!("result of {id:?}: {ours:?} != {theirs:?}"),
        }
        let context = |world: &UiWorld| {
            world
                .layout_node(id)
                .map(|node| (node.established, node.parent_context, node.participation))
        };
        assert_eq!(context(ours), context(theirs), "layout context of {id:?}");
        assert_eq!(
            hit_geometry(ours, document, id),
            hit_geometry(theirs, document, id),
            "hit geometry of {id:?}"
        );
        assert_eq!(
            accessibility_bounds(ours, id),
            accessibility_bounds(theirs, id),
            "accessibility bounds of {id:?}"
        );
    }
}
