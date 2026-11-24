mod retained_cache;
pub use retained_cache::RetainedLayoutCache;
#[cfg(test)]
pub(crate) use retained_cache::RetainedLayoutFootprint;
use retained_cache::*;
mod plans;
#[allow(unused_imports)]
pub use plans::plan_stats;
use plans::*;
mod input_map;
use crate::world::LayoutStyleRef as StyleRef;
use input_map::*;
pub(crate) use input_map::{
    AffectedIndex, LayoutExecutionStats, Size, StyleFacts, may_move_line_boxes,
    moves_line_boxes_below,
};

mod context;
use context::*;
mod grid;
use grid::*;
mod dynamic;
/// Segments one line's cost class may share before its solve shares the
/// class by capacity.
#[cfg(test)]
pub(crate) fn dynamic_line_max_states() -> usize {
    dynamic::LINE_MAX_STATES
}
mod flow;
use flow::*;
mod measure;
use measure::*;
pub(crate) use measure::{
    depends_on_used_basis, reads_offered_block_extent, sizes_own_height,
    spec_tracks_containing_block,
};
mod placement;
mod recursion;
use placement::*;
pub use recursion::LayoutSliceBudget;
pub(crate) use recursion::SliceLimit;
mod overlay;
use overlay::*;
mod inline;
use inline::*;
mod flex;
#[cfg(test)]
mod inline_scope;
pub(crate) mod parts;
use flex::*;
// These caches use internal numeric identities/constraint bits, not external text keys.
use hashbrown::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::box_layout::text_line_box_height_px;
use nana_ui_core::{
    AlignSpec, BoxSizing, DisplaySpec, FlexDirection, FlexWrap, FontSizeContext, GridAutoFlow,
    GridLine, GridPlacement, GridRepeatAuto, GridTemplateAreas, GridTrack, JustifySpec,
    LayoutStyle, LengthSpec, PositionSpec, TextAlignSpec, resolve_grid_track_sizes,
};

use crate::layout_frontier::{LayoutFrontier, LayoutFrontierSeed, LayoutFrontierStats};
use crate::{
    DocumentId, LayoutBox, MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld, UiWorldError,
};

/// Logical viewport supplied by the platform host to the retained layout system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutViewport {
    pub width: f32,
    pub height: f32,
}

impl LayoutViewport {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: finite_extent(width),
            height: finite_extent(height),
        }
    }
}

/// The viewport as far as one box's own layout reads it: each side the box
/// resolves against (a `vw` length the width, a `vh` one the height,
/// `position: fixed` both), zero for a side it does not read. Memo keys and
/// plans compare this, so a viewport resize keeps every memo and plan of a
/// box that does not read the side that moved. A box whose size moves with a
/// descendant that does is on the frontier the resize seeds, which retires
/// its memo and checks its plan.
fn viewport_basis(style: &LayoutStyle, viewport: LayoutViewport) -> LayoutViewport {
    let reads = style.viewport_axes();
    LayoutViewport::new(
        if reads.width { viewport.width } else { 0.0 },
        if reads.height { viewport.height } else { 0.0 },
    )
}

/// Backend-neutral layout owner used by canonical Runtime applications.
///
/// Consumes the same `LayoutStyle` and shaped text metrics stored in `UiWorld`
/// (flex wrap / `display:grid` 2D tracks·repeat·areas·placement via
/// `uses_2d_grid` / percent / calc / absolute / fixed / float / IFC subset
/// including shrink-to-avoid-float line boxes beside sibling floats)
/// and returns atomic layout writeback. css-parity calls
/// [`Self::layout_style_tree`] so its fixtures share this algorithm.
#[derive(Debug, Default, Clone, Copy)]
pub struct RuntimeLayoutEngine;

/// Style-only tree accepted by [`RuntimeLayoutEngine::layout_style_tree`].
///
/// css-parity fixtures adapt onto this type; they do not keep a second layout
/// algorithm.
#[derive(Debug, Clone, Default)]
pub struct StyleLayoutNode {
    pub id: String,
    pub style: LayoutStyle,
    pub children: Vec<StyleLayoutNode>,
    pub text: Option<String>,
}

impl RuntimeLayoutEngine {
    /// Every box of `document`, laid out from nothing, on the page.
    pub fn layout_document(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
    ) -> Result<Vec<(StableNodeId, LayoutBox)>, UiWorldError> {
        let local = self.layout_document_local(world, document, viewport)?;
        let frames: crate::NodeMap<_> = local
            .iter()
            .map(|&(id, layout, frame)| (id, (layout, frame)))
            .collect();
        let ids: Vec<_> = local.iter().map(|&(id, ..)| id).collect();
        Ok(page_boxes(world, &ids, |id| frames.get(&id).copied()))
    }

    /// [`Self::layout_document`] with each box measured from its frame's
    /// origin (see [`crate::LayoutFrame`]).
    pub(crate) fn layout_document_local(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
    ) -> Result<Vec<(StableNodeId, LayoutBox, crate::LayoutFrame)>, UiWorldError> {
        let order = world.document_order(document);
        let mut nodes = LayoutInputMap::new(world);
        nodes.prefetch(&order)?;
        let roots = world.document_roots(document);
        let mut output = PassOutput::with_highest_of(&order);
        let mut intrinsic = PassIntrinsicCache::with_capacity(nodes.len());
        let available = Size::new(viewport.width, viewport.height);
        for root in roots {
            let root_size = intrinsic_size(
                root,
                available,
                None,
                viewport,
                ROOT_FONT_PX,
                &mut nodes,
                &mut intrinsic,
            )?;
            place_node(
                root,
                Point::ZERO,
                root_size,
                available,
                viewport,
                ROOT_FONT_PX,
                &mut nodes,
                &mut intrinsic,
                &mut output,
            )?;
        }
        Ok(order
            .into_iter()
            .map(|id| {
                let frame = nodes.frames.get(&id).copied().unwrap_or_default();
                (id, output.remove(&id).unwrap_or_default(), frame)
            })
            .collect())
    }

    /// Incremental retained layout driven by typed invalidation seeds: the
    /// boxes the pass wrote, on the page.
    ///
    /// `force_full` disables pruning (viewport semantics changed) and
    /// rebuilds the retained cache. Otherwise the dependency graph expands
    /// each typed seed only across edges whose footprint consumes its metric.
    pub fn layout_document_with_frontier(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
        typed_seeds: &[LayoutFrontierSeed],
        retained: &mut RetainedLayoutCache,
        force_full: bool,
    ) -> Result<Vec<(StableNodeId, LayoutBox)>, UiWorldError> {
        let emitted = self.layout_document_with_frontier_local(
            world,
            document,
            viewport,
            typed_seeds,
            retained,
            force_full,
        )?;
        let ids: Vec<_> = emitted.iter().map(|&(id, _)| id).collect();
        let cache = retained.documents.get(&document);
        Ok(page_boxes(world, &ids, |id| {
            let cache = cache?;
            Some((
                cache.boxes.get(&id).copied()?,
                cache.frames.get(&id).copied().unwrap_or_default(),
            ))
        }))
    }

    /// [`Self::layout_document_with_frontier`] with each box measured from
    /// its frame's origin, as the retained cache keeps it: what the frame
    /// writes back.
    pub(crate) fn layout_document_with_frontier_local(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
        typed_seeds: &[LayoutFrontierSeed],
        retained: &mut RetainedLayoutCache,
        force_full: bool,
    ) -> Result<Vec<(StableNodeId, LayoutBox)>, UiWorldError> {
        let emitted = self.layout_document_with_frontier_unverified(
            world,
            document,
            viewport,
            typed_seeds,
            retained,
            force_full,
        )?;
        #[cfg(any(test, feature = "layout-verify"))]
        verify::retained_line_boxes_are_fresh(world, document, retained);
        #[cfg(any(test, feature = "layout-verify"))]
        verify::retained_matches_full_layout(
            self,
            world,
            document,
            viewport,
            retained,
            typed_seeds,
            force_full,
        );
        Ok(emitted)
    }

    fn layout_document_with_frontier_unverified(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
        typed_seeds: &[LayoutFrontierSeed],
        retained: &mut RetainedLayoutCache,
        force_full: bool,
    ) -> Result<Vec<(StableNodeId, LayoutBox)>, UiWorldError> {
        let roots = world.document_roots(document);
        // The budget is this pass's alone, whichever document it lays out.
        let slice_limit = retained.slice.take();
        if roots.is_empty() {
            retained.remove_document(document);
            return Ok(Vec::new());
        }
        let retained = retained.documents.entry(document).or_default();
        // A document with nothing retained -- its first layout, or one whose
        // cache was let go -- has nothing a scoped pass could reuse: every box
        // is computed either way. The full pass computes them without
        // expanding a frontier over the whole tree or materializing inputs one
        // miss at a time, and leaves the same retained cache behind. Only a
        // caller's full pass records or replays a full-page snapshot.
        let requested_full = force_full;
        let force_full = force_full || (retained.viewport.is_none() && retained.boxes.is_empty());
        // Reset before the full-layout reuse return, which runs no measure or placement.
        retained.execution_stats = LayoutExecutionStats::default();
        retained.inline_objects.clear();
        retained.last_frontier = LayoutFrontier::default();
        // A viewport the cache was not laid out against: plant the resize's
        // seeds, whichever entry asked for this pass. Memos and plans of boxes
        // that do not read the viewport no longer miss on it, so these seeds
        // are what reaches the boxes that consume it.
        let resized: Vec<LayoutFrontierSeed>;
        let typed_seeds = match retained.viewport.replace(viewport) {
            Some(previous) if !force_full && previous != viewport => {
                resized = typed_seeds
                    .iter()
                    .copied()
                    .chain(world.viewport_resize_seeds(document, Some(previous), viewport))
                    .collect();
                resized.as_slice()
            }
            _ => typed_seeds,
        };
        if requested_full && world.layout_source_reusable() {
            let viewport_width = viewport.width.to_bits();
            let viewport_height = viewport.height.to_bits();
            let epoch = world.layout_source_epoch();
            let reused = retained.full_snapshots.iter().find_map(|snapshot| {
                let snapshot = snapshot.as_ref()?;
                (snapshot.epoch == epoch
                    && snapshot.viewport_width == viewport_width
                    && snapshot.viewport_height == viewport_height)
                    .then(|| {
                        (
                            snapshot.emitted.clone(),
                            snapshot.used_padding.clone(),
                            snapshot.omitted_boxes.clone(),
                            snapshot.far_start.clone(),
                            snapshot.contexts.clone(),
                            snapshot.frames.clone(),
                        )
                    })
            });
            if let Some((emitted, used_padding, omitted_boxes, far_start, contexts, frames)) =
                reused
            {
                retained.boxes.clear();
                retained.boxes.extend(emitted.iter().copied());
                retained.frames = frames;
                retained.used_padding = used_padding;
                retained.omitted_boxes = omitted_boxes;
                retained.far_start = far_start;
                retained.contexts = contexts;
                retained.intrinsics.clear();
                retained.placements.clear();
                retained.container_plans.clear();
                retained.remote_overlays.clear();
                retained.measure_plans.clear();
                retained.line_boxes.clear();
                retained.deferred.clear();
                retained.materialized_inputs = emitted.len();
                return Ok(emitted);
            }
        }
        if force_full {
            retained.clear();
        }
        let mut nodes = LayoutInputMap::new(world);
        #[cfg(feature = "benchmark")]
        let mut phase = plan_stats::PhaseClock::start();
        let full_order = if force_full {
            world.document_order(document)
        } else {
            Vec::new()
        };
        if force_full {
            nodes.prefetch(&full_order)?;
        }
        #[cfg(feature = "benchmark")]
        phase.lap(0);
        let mut frontier = if force_full {
            LayoutFrontier::default()
        } else {
            // Typed mutation authority path: use the retained dependency index
            // so parent constraints, containing blocks and local formatting
            // contexts participate in one deduplicated closure.
            let graph = world.layout_dependency_graph_for_seeds(document, typed_seeds);
            let mut frontier = LayoutFrontier::from_dependency_graph(
                typed_seeds
                    .iter()
                    .copied()
                    .filter(|seed| world.document_of(seed.node) == Some(document)),
                &graph,
            );
            // A forced subtree rooted at the document is the whole document.
            if typed_seeds.iter().any(|seed| {
                seed.invalidation.is_unknown_forced_subtree()
                    && world.document_of(seed.node) == Some(document)
                    && world
                        .parent_id(seed.node)
                        .is_none_or(|parent| world.is_document_node(parent))
            }) {
                frontier.note_full_document_fallback();
            }
            frontier
        };
        let affected = if force_full {
            crate::NodeSet::default()
        } else {
            frontier.nodes().clone()
        };
        // Facts change at a mutation and its metric-owning ancestors. Nodes
        // reached only because their parent's context/constraint changed keep
        // compatible intrinsic facts; their used size is still recomputed.
        let mut metrics_changed = crate::NodeSet::default();
        let mut metric_edges = 0usize;
        for seed in typed_seeds.iter().filter(|seed| {
            frontier.is_seed(seed.node) && frontier.measure_nodes().contains(&seed.node)
        }) {
            let mut cursor = Some(seed.node);
            while let Some(id) = cursor {
                if !metrics_changed.insert(id) {
                    break;
                }
                cursor = world
                    .parent_id(id)
                    .filter(|parent| frontier.measure_nodes().contains(parent));
                metric_edges += usize::from(cursor.is_some());
            }
        }
        frontier.note_dependency_walk(metric_edges);
        retained.frontier_stats = LayoutFrontierStats::from_frontier(&frontier);
        // Whether a box holds a line box changes with it or with something
        // inside it: drop the answer up the ancestors of every node where
        // what it reads may have changed. A seed that only sized, spaced,
        // aligned, scrolled or turned a box changed nothing it reads
        // (`may_move_line_boxes`), and neither did a node the frontier
        // reached through a dependency edge from it. A structural seed
        // (`moves_line_boxes_below`) may hide or show a whole subtree
        // through state the subtree's boxes derive from their ancestors
        // (mounted, an overlay branch, a menu open), so a pass holding one
        // drops the answers up every affected node, as for any seed before.
        // The walk does not stop at a box with no answer, since one holding
        // a line box of its own settles without asking its children; it
        // stops where an earlier walk already went up.
        let seeds = typed_seeds
            .iter()
            .filter(|seed| world.document_of(seed.node) == Some(document));
        let structural = seeds
            .clone()
            .any(|seed| moves_line_boxes_below(seed.invalidation));
        let mut cleared = crate::NodeSet::default();
        let mut clear_up = |id: StableNodeId| {
            let mut cursor = Some(id);
            while let Some(node) = cursor.filter(|node| cleared.insert(*node)) {
                retained.line_boxes.remove(&node);
                cursor = world.parent_id(node);
            }
        };
        if structural {
            affected.iter().copied().for_each(&mut clear_up);
        }
        seeds
            .filter(|seed| may_move_line_boxes(seed.invalidation))
            .for_each(|seed| clear_up(seed.node));
        nodes.line_boxes = std::cell::RefCell::new(std::mem::take(&mut retained.line_boxes));
        // Used-size memos on the measure frontier are stale. Intrinsic facts
        // stay until publication so an identical recompute does not bump generation.
        for id in frontier.measure_nodes() {
            retained.intrinsics.remove_used(*id);
        }
        // A full pass places every node: size its table for the document
        // once. A scoped pass places a few, wherever they are.
        let mut output = if force_full {
            PassOutput::with_highest_of(&full_order)
        } else {
            PassOutput::default()
        };
        // A full pass measures most containers' children twice (against the
        // offered and the used content box), so its used sizes and facts
        // outnumber its nodes: room for two each, or the tables double and
        // copy themselves near the end of the pass.
        let mut intrinsic = PassIntrinsicCache::with_capacity(nodes.len().saturating_mul(2));
        let highest = full_order.iter().max().copied();
        if let Some(highest) = highest {
            intrinsic.reserve_through(highest, nodes.len());
        }
        // A full pass a caller asked for leaves its plans to nobody (the
        // retained cache was just cleared, or the map is thrown away). The
        // first layout of a document is the base of the scoped passes after
        // it, and records them as a scoped pass would.
        intrinsic.records_measure_plans = !requested_full;
        intrinsic.slice.limit = slice_limit;
        intrinsic.slice.viewport = Size::new(viewport.width, viewport.height);
        // What earlier passes deferred: placed where this pass reaches it,
        // or after the roots from what was recorded. A full pass cleared it
        // with the rest, and places everything anew.
        intrinsic.slice.pending = std::mem::take(&mut retained.deferred);
        let available = Size::new(viewport.width, viewport.height);
        let islands = if force_full {
            Vec::new()
        } else {
            let mut islands = Vec::new();
            for id in affected.iter().copied() {
                // A dependency boundary may be a fixed-size ordinary ancestor
                // as well as an explicit isolation context. Use the retained
                // placement as the local layout root so a child can be
                // recomputed without pulling that stable ancestor into the
                // measure frontier. A fixed inline-block has the same role
                // for an inline formatting context: its border box is the
                // island, and the outer line is not packed again.
                let Some(parent) = world.parent_id(id) else {
                    continue;
                };
                if affected.contains(&parent) {
                    continue;
                }
                if retained.placements.contains_key(&id) && retained.boxes.contains_key(&id) {
                    let (origin, containing, font) = retained.placements[&id];
                    islands.push((id, origin, containing, font));
                    continue;
                }
                let (Some(border), Some(parent_box)) = (
                    retained.boxes.get(&id).copied(),
                    retained.boxes.get(&parent).copied(),
                ) else {
                    continue;
                };
                if let Some((origin, containing, font)) = fixed_inline_block_island(
                    world,
                    id,
                    border,
                    parent_box,
                    retained.used_padding_of(parent),
                ) {
                    islands.push((id, origin, containing, font));
                }
            }
            islands
        };
        // An island whose own border box changed moves its siblings, which only
        // its parent places. Such a pass also walks from the island's document
        // root, where the parent's retained plan replays the shift.
        let mut resized_roots = HashSet::new();
        let reach = if force_full {
            AffectedIndex::default()
        } else {
            let covered: HashSet<StableNodeId> = islands.iter().map(|island| island.0).collect();
            AffectedIndex::new(world, &affected, &covered)
        };
        let affected_children = if force_full {
            AffectedChildren::default()
        } else {
            AffectedChildren::new(world, &affected)
        };
        let scope = ScopeContext {
            frontier: &frontier,
            affected: &affected,
            affected_children: &affected_children,
            measure: frontier.measure_nodes(),
            metrics_changed: &metrics_changed,
            retained: &*retained,
            reach: &reach,
        };
        let scope_ref = (!force_full).then_some(&scope);
        for &(root, origin, containing, font) in &islands {
            // An island boundary normally has a stable used size, but an
            // affected content-sized child can grow inside a fixed ancestor.
            // Re-measure the island itself so its retained box does not freeze
            // the new intrinsic size before placement reaches its descendants.
            let root_size = intrinsic_size_scoped(
                root,
                containing,
                None,
                viewport,
                font,
                &mut nodes,
                &mut intrinsic,
                scope_ref,
            )?;
            if retained.boxes.get(&root).is_none_or(|previous| {
                previous.width.to_bits() != root_size.width.to_bits()
                    || previous.height.to_bits() != root_size.height.to_bits()
            }) {
                let mut top = root;
                while let Some(parent) = world.parent_id(top) {
                    top = parent;
                }
                resized_roots.insert(top);
            }
            place_node_scoped(
                root,
                origin,
                root_size,
                containing,
                viewport,
                font,
                None,
                &mut nodes,
                &mut intrinsic,
                &mut output,
                scope_ref,
            )?;
        }
        for root in roots {
            // With islands laid out on their own, a root pass runs only for
            // what leads to an affected node outside every island, or for the
            // root of an island that resized and moves its siblings.
            if !force_full
                && !islands.is_empty()
                && !resized_roots.contains(&root)
                && !reach.reaches(root)
            {
                continue;
            }
            let root_size = intrinsic_size_scoped(
                root,
                available,
                None,
                viewport,
                ROOT_FONT_PX,
                &mut nodes,
                &mut intrinsic,
                scope_ref,
            )?;
            #[cfg(feature = "benchmark")]
            phase.lap(1);
            place_node_scoped(
                root,
                Point::ZERO,
                root_size,
                available,
                viewport,
                ROOT_FONT_PX,
                None,
                &mut nodes,
                &mut intrinsic,
                &mut output,
                scope_ref,
            )?;
            #[cfg(feature = "benchmark")]
            phase.lap(2);
        }
        placement::resume_deferred(
            document,
            viewport,
            &mut nodes,
            &mut intrinsic,
            &mut output,
            scope_ref,
        )?;
        if let Some(scope) = scope_ref {
            place_remote_overlays(viewport, &mut nodes, &mut intrinsic, &mut output, scope)?;
        }
        if !force_full {
            // An affected node that no longer generates a box (hidden, or in
            // a closed branch) is skipped by its container rather than
            // placed: collapse what it kept, and everything under it, and
            // drop the contexts they established.
            collapse_omitted_boxes(&affected, &mut nodes, retained, &mut output);
        }
        // Publish recomputed boxes from the placed set; no document_order walk.
        let emitted = output.sorted();
        // A first layout's table of boxes is the pass's, whole.
        output.move_into(&mut retained.boxes);
        // A first layout's retained tables are empty: they take the pass's
        // tables whole instead of copying them entry by entry.
        for id in nodes.cleared_padding.drain() {
            retained.used_padding.remove(&id);
        }
        retained.used_padding.absorb(&mut nodes.used_padding);
        for id in nodes.unomitted_boxes.drain() {
            retained.omitted_boxes.remove(&id);
        }
        for id in nodes.omitted_boxes.drain() {
            retained.used_padding.remove(&id);
            retained.omitted_boxes.insert(id);
        }
        retained.line_boxes = nodes.line_boxes.take();
        retained.far_start.absorb(&mut nodes.far_start);
        retained.contexts.absorb(&mut nodes.contexts);
        retained.inline_objects.extend(nodes.inline_objects.drain());
        for (id, frame) in nodes.frames.drain() {
            if frame == crate::LayoutFrame::Parent {
                retained.frames.remove(&id);
            } else {
                retained.frames.insert(id, frame);
            }
        }
        retained.placements.absorb(&mut nodes.placements);
        std::mem::take(&mut nodes.container_plans)
            .merge_into(&mut retained.container_plans, &mut retained.remote_overlays);
        // The measure plans this pass answered from or recorded, by container
        // and the constraint each holds.
        let plan_key = |id: StableNodeId, available: Size| {
            (id, available.width.to_bits(), available.height.to_bits())
        };
        // Only the walk over the affected children below reads which plans
        // this pass consulted; a pass with none affected (a full one) keeps
        // no record of them.
        let tracks_consulted = !affected.is_empty();
        let mut consulted: hashbrown::HashSet<(StableNodeId, u32, u32)> = if tracks_consulted {
            std::mem::take(&mut intrinsic.consulted_plans)
                .into_iter()
                .map(|(id, available)| plan_key(id, available))
                .collect()
        } else {
            hashbrown::HashSet::new()
        };
        if retained.measure_plans.is_empty() && !tracks_consulted {
            // Nothing retained to merge into (a first layout): each holder
            // goes over as it is, its plans in the slots the pass put them
            // in, which is where inserting them one by one, oldest first,
            // into an empty holder puts them.
            nodes.measure_plans.retain(|_, plans| {
                for plan in plans.slots.iter_mut().flatten() {
                    bound_measure_plan(plan);
                }
                !plans.is_empty()
            });
            retained.measure_plans.absorb(&mut nodes.measure_plans);
        }
        for (id, plans) in nodes.measure_plans.drain() {
            if plans.is_empty() {
                retained.measure_plans.remove(&id);
                continue;
            }
            // Merge rather than replace. A container is commonly measured under
            // two constraints per pass but only RE-measured under one of them:
            // the other is answered from its cached plan, which records nothing.
            // Replacing would drop that constraint's plan, so the two slots
            // would alternate instead of holding both. The slot array evicts
            // any older constraint for this same container.
            let slots = retained.measure_plans.entry(id).or_default();
            for mut plan in plans.into_plans().rev() {
                if tracks_consulted {
                    consulted.insert(plan_key(id, plan.available));
                }
                bound_measure_plan(&mut plan);
                slots.insert(plan);
            }
        }
        // A child that changed this pass reached only the plans this pass
        // consulted. Its container's other plan, under a constraint this pass
        // did not measure it at, still holds the child as it was; re-checking
        // the children a later pass reaches would not find the change, and the
        // plan would hand back a size from before it.
        for &child in &affected {
            let Some(container) = world.parent_id(child) else {
                continue;
            };
            let Some(slots) = retained.measure_plans.get_mut(&container) else {
                continue;
            };
            for plan in slots.slots.iter_mut().flatten() {
                if !plan.stale
                    && !consulted.contains(&plan_key(container, plan.available))
                    && plan_missed_change(plan, child, &nodes, frontier.measure_nodes())
                {
                    plan.stale = true;
                }
            }
        }
        retained
            .envelopes
            .extend(intrinsic.dynamic.take_envelopes());
        for (id, solve) in intrinsic.dynamic.line_solves.drain() {
            match solve {
                Some(solve) => {
                    retained.line_solves.insert(id, solve);
                }
                None => {
                    retained.line_solves.remove(&id);
                }
            }
        }
        for (id, applied) in intrinsic.dynamic.applied.drain() {
            match applied {
                Some(applied) => {
                    retained.applied_adjustments.insert(id, applied);
                }
                None => {
                    retained.applied_adjustments.remove(&id);
                }
            }
        }
        intrinsic.execution_stats.dynamic = intrinsic.dynamic.counters;
        // Pending for the next pass: what this pass deferred, and what
        // earlier passes deferred that it did not reach.
        let slice = std::mem::take(&mut intrinsic.slice);
        intrinsic.execution_stats.placement_slices = usize::from(!slice.deferred.is_empty());
        intrinsic.execution_stats.placement_deferrals_cancelled = slice.cancelled;
        retained.deferred = slice.pending;
        retained.deferred.extend(slice.deferred);
        intrinsic.execution_stats.placement_boundaries_deferred = retained.deferred.len();
        intrinsic.execution_stats.measure_stack_per_level = 0;
        intrinsic.execution_stats.placement_stack_per_level = intrinsic.placement_levels.per_level;
        let intrinsic_counters = intrinsic.counters();
        intrinsic.execution_stats.line_boxes_settled = nodes.line_boxes_settled.get();
        retained.execution_stats = intrinsic.execution_stats;
        let universe = if force_full { nodes.len() } else { world.len() };
        if let Some(highest) = highest {
            retained.intrinsics.reserve_through(highest, universe);
        }
        for (key, size) in std::mem::take(&mut intrinsic.used_order) {
            retained.intrinsics.record_used(key.id, key.stored(), size);
        }
        // Four constraint variants a live node, never a fixed count: a cap
        // below the document's working set evicts facts the next frame reads,
        // and every large document then measured again what it had evicted --
        // work that grew with the document, not with the edit.
        const DEFAULT_RETAINED_METRIC_FLOOR: usize = 4096;
        let retained_metric_budget = universe
            .saturating_mul(4)
            .max(DEFAULT_RETAINED_METRIC_FLOOR);
        // A patched measurement leaves the facts measured before it behind:
        // a later constraint its used sizes do not hold would read them and
        // miss the patch. Facts this pass measured come back in below.
        for id in std::mem::take(&mut intrinsic.retired_facts) {
            retained.intrinsics.remove_metrics(id);
        }
        // A node in `metrics_changed` that was omitted or hidden this pass
        // never published fresh intrinsic facts, so its prior metrics and
        // measure plans would stay behind and answer once it opens again.
        if !metrics_changed.is_empty() {
            for &id in &metrics_changed {
                if intrinsic.cache.has_recorded(id) || intrinsic.unchanged_plan_nodes.contains(&id)
                {
                    continue;
                }
                retained.intrinsics.remove_metrics(id);
                if let Some(slots) = retained.measure_plans.get_mut(&id) {
                    for plan in slots.slots.iter_mut().flatten() {
                        if !plan.stale && !consulted.contains(&plan_key(id, plan.available)) {
                            plan.stale = true;
                        }
                    }
                }
            }
        }
        retained.retain_intrinsic_metrics(&intrinsic.cache, retained_metric_budget);
        retained.materialized_inputs = nodes.materialized;
        retained.record_intrinsic_counters(intrinsic_counters);
        // Despawned ids linger in the retained maps; keep them bounded.
        // Scoped passes only materialize a subset, so membership is the live
        // world, not the partial input map. Each map is swept by its own
        // size: one that outgrows the others (the line-box memo holds an
        // answer for a node whose box is already gone) would otherwise wait
        // for them and keep an entry for every node ever removed.
        let crowded = universe.saturating_mul(2);
        let mut swept = false;
        macro_rules! sweep {
            ($($map:ident),* $(,)?) => {$(
                if retained.$map.len() > crowded {
                    retained.$map.retain(|id, _| world.contains(*id));
                    swept = true;
                }
            )*};
        }
        sweep!(
            boxes,
            frames,
            placements,
            used_padding,
            far_start,
            contexts,
            container_plans,
            measure_plans,
            envelopes,
            line_solves,
            applied_adjustments,
            line_boxes,
        );
        if retained.remote_overlays.len() > crowded {
            retained.remote_overlays.retain(|id| world.contains(*id));
            swept = true;
        }
        if retained.omitted_boxes.len() > crowded {
            retained.omitted_boxes.retain(|id| world.contains(*id));
            swept = true;
        }
        if swept {
            retained.execution_stats.retain_sweeps =
                retained.execution_stats.retain_sweeps.saturating_add(1);
        }
        retained.intrinsics.retain_live(world, universe);
        #[cfg(feature = "benchmark")]
        phase.lap(3);
        // A sliced layout is not the full page.
        if requested_full && world.layout_source_reusable() && retained.deferred.is_empty() {
            let snapshot = FullLayoutSnapshot {
                viewport_width: viewport.width.to_bits(),
                viewport_height: viewport.height.to_bits(),
                epoch: world.layout_source_epoch(),
                emitted: emitted.clone(),
                used_padding: retained.used_padding.clone(),
                omitted_boxes: retained.omitted_boxes.clone(),
                far_start: retained.far_start.clone(),
                contexts: retained.contexts.clone(),
                frames: retained.frames.clone(),
            };
            retained.full_snapshots[1] = retained.full_snapshots[0].take();
            retained.full_snapshots[0] = Some(snapshot);
        }
        // Kept for diagnostics until the next pass replaces it: why each node
        // this pass reached was admitted. It holds the frontier, not the
        // dependency graph, so it is bounded by this pass's own work.
        retained.last_frontier = frontier;
        Ok(emitted)
    }

    /// Layout a style tree with the same algorithm as [`Self::layout_document`].
    ///
    /// Hidden / `display:none` nodes are omitted from the result (css-parity
    /// measure contract). Product `UiWorld` still records a zero box.
    ///
    /// Text leaves are measured by `shaper`, so a host passes the same one its
    /// frames flush with: a style tree measured by a second shaper would give
    /// boxes the painted text does not fit.
    pub fn layout_style_tree(
        self,
        root: &StyleLayoutNode,
        viewport: LayoutViewport,
        shaper: &mut impl crate::TextShaper,
    ) -> Vec<(String, LayoutBox)> {
        let document = DocumentId::new(1).expect("document 1 is nonzero");
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        let mut names = HashMap::new();
        let mut omitted = HashSet::new();
        let mut next = 1u64;
        fn add(
            node: &StyleLayoutNode,
            parent: Option<StableNodeId>,
            parent_omitted: bool,
            document: DocumentId,
            queue: &mut MutationQueue,
            names: &mut HashMap<StableNodeId, String>,
            omitted: &mut HashSet<StableNodeId>,
            next: &mut u64,
        ) -> StableNodeId {
            let id = StableNodeId::new(*next).expect("style-tree ids start at 1");
            *next += 1;
            queue.create(id, document, NodeKind::Element { tag: "div".into() });
            if let Some(parent) = parent {
                queue.insert(parent, id, None);
            }
            // `display:none` / hidden omit self and descendants. `display:contents`
            // omits only self from the name→box map; descendants still layout.
            let omit_descendants = parent_omitted || node.style.omits_box();
            if omit_descendants || !node.style.generates_box() {
                omitted.insert(id);
            }
            queue.set_style(
                id,
                NodeStyle {
                    layout: Arc::new(node.style.clone()),
                    ..NodeStyle::default()
                },
            );
            names.insert(id, node.id.clone());
            if let Some(text) = node.text.as_deref() {
                queue.set_text(id, crate::TextContent { value: text.into() });
            }
            for child in &node.children {
                add(
                    child,
                    Some(id),
                    omit_descendants,
                    document,
                    queue,
                    names,
                    omitted,
                    next,
                );
            }
            id
        }
        add(
            root,
            None,
            false,
            document,
            &mut queue,
            &mut names,
            &mut omitted,
            &mut next,
        );
        world
            .commit(queue)
            .expect("style-tree mutations are well-formed");
        let order = world.document_order(document);
        world
            .resolve_styles(&order)
            .expect("style-tree style resolve is infallible");
        world
            .shape_text(&order, shaper)
            .expect("style-tree text shaping is infallible");
        let layouts = self
            .layout_document(&world, document, viewport)
            .expect("style-tree layout is infallible");
        layouts
            .into_iter()
            .filter_map(|(id, box_)| {
                if omitted.contains(&id) {
                    return None;
                }
                names.get(&id).cloned().map(|name| (name, box_))
            })
            .collect()
    }
}

/// `ids`' boxes on the page, from what `local` reads for each box: its
/// local box and frame. Each origin is its frame's plus its local one,

impl Size {
    pub(crate) fn new(width: f32, height: f32) -> Self {
        Self {
            width: finite_extent(width),
            height: finite_extent(height),
        }
    }
}

fn gap_containing_block(style: &LayoutStyle, content: Size) -> nana_ui_core::ParentBox {
    // Auto-height wrap: row-gap % falls back to width (T-W05/W06). A Fill/px
    // height is a definite CB and must not use the parent/viewport leftover.
    let height = match style.height {
        None
        | Some(LengthSpec::Auto)
        | Some(LengthSpec::Shrink)
        | Some(LengthSpec::MinContent)
        | Some(LengthSpec::MaxContent)
        | Some(LengthSpec::FitContent) => None,
        Some(LengthSpec::Fill) => Some(content.height).filter(|value| *value > 0.0),
        Some(_) => Some(content.height).filter(|value| *value > 0.0),
    };
    nana_ui_core::ParentBox::new(Some(content.width).filter(|value| *value > 0.0), height)
}

/// Whether placement lays the container's main and cross axes out from their
/// far page end — the right or the bottom: an RTL inline axis (the right, or
/// the bottom of a vertical one), `vertical-rl`'s block axis from the right,
/// or `flex-direction: *-reverse`. A 2D grid places on the page as authored.
fn flow_axes_reversed(
    style: &LayoutStyle,
    writing: nana_ui_core::WritingContext,
    direction: FlexDirection,
    ifc: bool,
    grid_2d: bool,
) -> (bool, bool) {
    if grid_2d {
        return (false, false);
    }
    let cross = if direction.is_row() {
        FlexDirection::Column
    } else {
        FlexDirection::Row
    };
    let main = if ifc {
        writing.physical_axis_reversed(direction)
    } else {
        style.flex_reverse != writing.physical_axis_reversed(direction)
    };
    (main, writing.physical_axis_reversed(cross))
}

/// Which page axes of container `id` start at their far end: `[horizontal,
/// vertical]`, true where the content starts at the right / bottom and
/// overflows toward the left / top. The scroll origin sits on that start edge.
///
/// Placement records it as it lays the children out, so this is what the
/// boxes actually did. Before a first layout it is read off the container's
/// own style, which differs only for a reversed non-flex container that turns
/// out to be an inline formatting context (whose lines ignore `*-reverse`).
pub(crate) fn far_start_axes(world: &UiWorld, id: StableNodeId) -> [bool; 2] {
    if let Some(placed) = world.layout_far_start(id) {
        return placed;
    }
    let Some(style) = world.layout_style(id) else {
        return [false; 2];
    };
    let writing = world.layout_writing(id);
    let grid = style.display.is_some_and(DisplaySpec::is_grid_container);
    let direction = used_flow_direction(&style, writing, false);
    let reversed = flow_axes_reversed(&style, writing, direction, false, grid);
    page_far_start(writing, direction, grid, reversed)
}

/// The page axes `[horizontal, vertical]` a placement starting its main /
/// cross axes at their far ends (`reversed`) turns into. A grid turns its
/// tracks onto the page by the writing context alone.
fn page_far_start(
    writing: nana_ui_core::WritingContext,
    direction: FlexDirection,
    grid_2d: bool,
    (main, cross): (bool, bool),
) -> [bool; 2] {
    if grid_2d {
        [
            writing.physical_axis_reversed(FlexDirection::Row),
            writing.physical_axis_reversed(FlexDirection::Column),
        ]
    } else if direction.is_row() {
        [main, cross]
    } else {
        [cross, main]
    }
}

/// Physical main axis for this formatting context.
///
/// IFC always follows the writing-mode inline axis. Flex `row`/`column` are
/// remapped through writing-mode; block containers without an explicit
/// `flex-direction` stack along the block axis.
fn used_flow_direction(
    style: &LayoutStyle,
    context: nana_ui_core::WritingContext,
    ifc: bool,
) -> FlexDirection {
    if ifc {
        return context.inline_flex_direction();
    }
    let css = style.direction.unwrap_or(FlexDirection::Column);
    context.physical_flex_direction(css)
}

/// `text-align` as the flow-relative justification of an inline formatting
/// context's line: `start` / `end` are the inline axis's own, and the physical
/// `left` / `right` land on whichever end of it the page puts there — the end
/// of an RTL line for `left`, its start for `right`.
fn ifc_justify(align: TextAlignSpec, context: nana_ui_core::WritingContext) -> JustifySpec {
    let reversed = context.inline_reversed();
    let physical = align.to_justify(reversed);
    if reversed {
        flip_justify_for_reverse(physical)
    } else {
        physical
    }
}

fn flip_justify_for_reverse(justify: JustifySpec) -> JustifySpec {
    match justify {
        JustifySpec::Start => JustifySpec::End,
        JustifySpec::End => JustifySpec::Start,
        other => other,
    }
}

fn demote_fill_spec(spec: Option<LengthSpec>) -> Option<LengthSpec> {
    match spec {
        Some(s) if s.is_full_percent_fill() => None,
        other => other,
    }
}

/// `100%` / `Fill` against a definite grid CB must not become the auto-track
/// contribution. Measure that axis as indefinite (same as auto tracks).
fn grid_item_measure_available(style: &LayoutStyle, content: Size) -> Size {
    let width = if style.width.is_some() && demote_fill_spec(style.width).is_none() {
        0.0
    } else {
        content.width
    };
    let height = if style.height.is_some() && demote_fill_spec(style.height).is_none() {
        0.0
    } else {
        content.height
    };
    Size::new(width, height)
}

#[allow(clippy::too_many_arguments)]
fn packing_main_size(
    style: &LayoutStyle,
    intrinsic: Size,
    direction: FlexDirection,
    content_main: f32,
    // What percentage paddings resolve against: the containing block's inline
    // size, which is `content_main` only for a row in `horizontal-tb`.
    edge_percent_base: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    track: Option<GridTrack>,
) -> f32 {
    // A line takes an item by the size it has before it grows (CSS Flexbox
    // §9.3, the hypothetical main size): its basis, else its main size, else
    // its content, within its min and max. Growth is shared out once the
    // line is known (`distribute_flex_main`), so a growing item no longer
    // claims a whole line and pushes siblings that fit beside it onto the
    // next.
    let spec = if style.grows() {
        style
            .flex_basis
            .filter(|basis| !matches!(basis, LengthSpec::Auto))
            .or(match direction {
                FlexDirection::Row => style.width,
                FlexDirection::Column => style.height,
            })
    } else {
        style.child_main_length(direction)
    }
    .or_else(|| track.map(GridTrack::as_row_main_length));
    let fonts = fonts_of(style, parent_font_px);
    let vp = Some((viewport.width, viewport.height));
    let (min, max) = match direction {
        FlexDirection::Row => (
            style.resolved_min_width_fonts(Some(content_main), vp, fonts),
            style.resolved_max_width_fonts(Some(content_main), vp, fonts),
        ),
        FlexDirection::Column => (
            style.resolved_min_height_fonts(Some(content_main), vp, fonts),
            style.resolved_max_height_fonts(Some(content_main), vp, fonts),
        ),
    };
    let clamp = |value: f32| {
        let value = value.max(min);
        max.map_or(value, |max| value.min(max))
    };
    match resolve_child_main(spec, content_main, viewport, fonts) {
        Some(value) => content_box_main_border_size(
            style,
            direction,
            Some(edge_percent_base),
            clamp(value),
            fonts,
        ),
        None if matches!(spec, Some(LengthSpec::Fill)) => content_main,
        None => clamp(main_extent(intrinsic, direction)),
    }
}

/// CSS initial `medium` ≈ 16px. Root `rem` and the em base when no ancestor
/// set `font-size`.
const ROOT_FONT_PX: f32 = nana_ui_core::FontSizeContext::DEFAULT_ROOT_PX;

fn fonts_of(style: &LayoutStyle, parent_font_px: f32) -> FontSizeContext {
    FontSizeContext::new(ROOT_FONT_PX, style.font_size.unwrap_or(parent_font_px))
}

fn resolve_child_main(
    spec: Option<LengthSpec>,
    percent_base: f32,
    viewport: LayoutViewport,
    fonts: FontSizeContext,
) -> Option<f32> {
    match spec {
        None
        | Some(LengthSpec::Fill)
        | Some(LengthSpec::Shrink)
        | Some(LengthSpec::Auto)
        | Some(LengthSpec::MinContent)
        | Some(LengthSpec::MaxContent)
        | Some(LengthSpec::FitContent) => None,
        Some(other) => other
            .resolve_with_fonts(
                Some(percent_base),
                Some((viewport.width, viewport.height)),
                fonts,
            )
            .map(|value| value.max(0.0)),
    }
}

fn content_box_main_border_size(
    style: &LayoutStyle,
    direction: FlexDirection,
    margin_percent_base: Option<f32>,
    content_main: f32,
    fonts: FontSizeContext,
) -> f32 {
    if !matches!(style.box_sizing, BoxSizing::ContentBox) {
        return content_main;
    }
    let pad = style.resolved_padding_against_fonts(margin_percent_base, fonts);
    let border = style.resolved_border_edges();
    content_main
        + match direction {
            FlexDirection::Row => pad.left + pad.right + border.left + border.right,
            FlexDirection::Column => pad.top + pad.bottom + border.top + border.bottom,
        }
}

fn resolve_axis(
    spec: Option<LengthSpec>,
    percent_base: f32,
    fill_base: f32,
    viewport: LayoutViewport,
    fonts: FontSizeContext,
) -> Option<f32> {
    spec.and_then(|value| {
        if value == LengthSpec::Fill {
            Some(fill_base)
        } else {
            value
                .resolve_with_fonts(
                    Some(percent_base),
                    Some((viewport.width, viewport.height)),
                    fonts,
                )
                .map(|value| value.max(0.0))
        }
    })
}

fn demote_fill_spec_if_indefinite(spec: Option<LengthSpec>, base: f32) -> Option<LengthSpec> {
    if base > 0.5 {
        spec
    } else {
        demote_fill_spec(spec)
    }
}

fn aspect_ratio_is_usable(style: &nana_ui_core::LayoutStyle) -> bool {
    style.aspect_ratio.is_some_and(|r| r.is_finite() && r > 0.0)
}

/// Whether `child`, which names no alignment of its own, keeps the size
/// its content gives it across a block container that would otherwise
/// stretch it. Block layout fills an `auto`-width block box along the
/// container's inline axis, but not every child: a replaced element takes
/// the width its natural size gives it (CSS 2.1 §10.3.4, §10.3.2), and a
/// child in an orthogonal writing mode has its block size there, which its
/// content gives (CSS Writing Modes 3 §7.3). Flex and grid stretch both.
///
/// `natural_ratio` is [`has_natural_ratio`] for `child`. A child inherits
/// its container's writing mode unless it declares one.
fn block_keeps_content_size(
    container: &nana_ui_core::LayoutStyle,
    container_writing: nana_ui_core::WritingContext,
    child_style: &nana_ui_core::LayoutStyle,
    natural_ratio: bool,
) -> bool {
    child_style.align_self.is_none()
        && container.stacks_as_block()
        && (natural_ratio
            || child_style
                .writing_mode
                .is_some_and(|mode| mode.is_vertical() != container_writing.is_vertical()))
}

/// Whether `id` shows replaced content whose resource reported a natural
/// size: a preferred aspect ratio of its own, which stretch alignment keeps
/// as it keeps a declared `aspect-ratio` (Issue #202).
fn has_natural_ratio(world: &UiWorld, id: StableNodeId) -> bool {
    world
        .replaced_natural_size(id)
        .is_some_and(|natural| natural.width > 0.0 && natural.height > 0.0)
}

/// After stretch (or a flexed used width), fill `height:auto` from the used
/// width; the height then meets the box's own `min-height` / `max-height`
/// (CSS Sizing 4 §5.1). `percent_base` resolves padding, `height_base` the
/// height limits, as measurement resolved them.
fn fill_auto_height_from_aspect_ratio(
    style: &nana_ui_core::LayoutStyle,
    size: &mut Size,
    percent_base: Option<f32>,
    height_base: Option<f32>,
    viewport: LayoutViewport,
    fonts: FontSizeContext,
) {
    if !aspect_ratio_is_usable(style) || style.height.is_some() {
        return;
    }
    let padding = style.resolved_padding_against_fonts(percent_base, fonts);
    let border = style.resolved_border_edges();
    let chrome_w = padding.left + padding.right + border.left + border.right;
    let chrome_h = padding.top + padding.bottom + border.top + border.bottom;
    let mut width = Some(size.width);
    let mut height = None;
    style.apply_aspect_ratio_used(&mut width, &mut height, (chrome_w, chrome_h));
    if let Some(height) = height {
        let vp = Some((viewport.width, viewport.height));
        let mut height = height.max(style.resolved_min_height_fonts(height_base, vp, fonts));
        if let Some(max) = style.resolved_max_height_fonts(height_base, vp, fonts) {
            height = height.min(max);
        }
        size.height = height;
    }
}

/// Whether the item's cross size is its own, so `align-items: stretch`
/// leaves it: any size it declares but `auto`, which stretches as unset
/// does (CSS). A content-sized keyword (`Shrink`, `fit-content`) keeps the
/// content's size.
/// `natural_ratio` is [`has_natural_ratio`]: a replaced box's own ratio
/// transfers its block size as a declared `aspect-ratio` does.
fn cross_axis_is_definite(
    style: &nana_ui_core::LayoutStyle,
    direction: FlexDirection,
    natural_ratio: bool,
) -> bool {
    let declared = |spec: Option<LengthSpec>| spec.is_some_and(|spec| spec != LengthSpec::Auto);
    match direction {
        // Transferred block size from a definite used width + `aspect-ratio`.
        FlexDirection::Row => {
            declared(style.height) || aspect_ratio_is_usable(style) || natural_ratio
        }
        FlexDirection::Column => declared(style.width),
    }
}

fn main_extent(size: Size, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => size.width,
        FlexDirection::Column => size.height,
    }
}

fn cross_extent(size: Size, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => size.height,
        FlexDirection::Column => size.width,
    }
}

fn set_main_extent(size: &mut Size, direction: FlexDirection, value: f32) {
    match direction {
        FlexDirection::Row => size.width = finite_extent(value),
        FlexDirection::Column => size.height = finite_extent(value),
    }
}

fn set_cross_extent(size: &mut Size, direction: FlexDirection, value: f32) {
    match direction {
        FlexDirection::Row => size.height = finite_extent(value),
        FlexDirection::Column => size.width = finite_extent(value),
    }
}

fn main_start_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.left,
        FlexDirection::Column => margin.top,
    }
}

fn main_end_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.right,
        FlexDirection::Column => margin.bottom,
    }
}

fn cross_start_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.top,
        FlexDirection::Column => margin.left,
    }
}

fn cross_end_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.bottom,
        FlexDirection::Column => margin.right,
    }
}

fn cross_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    cross_start_margin(margin, direction) + cross_end_margin(margin, direction)
}

/// Drop a repeated child row. Plans store one entry per direct participant;
/// a later record for the same child replaces the stale one.

fn finite_extent(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

mod stack_budget;
#[cfg(test)]
mod tests;

pub(crate) mod verify;

/// Which nodes a layout actually measured: past the pass memo and the
/// retained intrinsics, into computing a size. A gate that asks "was this
/// box measured" reads this instead of the frontier, which does not see a
/// box measured again because its memo key moved. Off until a test begins
/// a trace on its thread.
#[cfg(test)]
pub(crate) mod measure_trace {
    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;

    use crate::StableNodeId;

    thread_local! {
        static MEASURED: RefCell<Option<HashSet<StableNodeId>>> = const { RefCell::new(None) };
        static PAUSED: Cell<bool> = const { Cell::new(false) };
    }

    pub(crate) fn record(id: StableNodeId) {
        if PAUSED.with(Cell::get) {
            return;
        }
        MEASURED.with(|measured| {
            if let Some(measured) = measured.borrow_mut().as_mut() {
                measured.insert(id);
            }
        });
    }

    /// Stops recording until the returned value drops: the layout-verify
    /// guard's own full layout is the test's check, not the frame's work.
    pub(crate) fn pause() -> Paused {
        Paused(PAUSED.with(|paused| paused.replace(true)))
    }

    pub(crate) struct Paused(bool);

    impl Drop for Paused {
        fn drop(&mut self) {
            PAUSED.with(|paused| paused.set(self.0));
        }
    }

    /// Start, or restart, tracing on this thread.
    pub(crate) fn begin() {
        MEASURED.with(|measured| *measured.borrow_mut() = Some(HashSet::new()));
    }

    /// What was measured since the trace began or was last taken.
    pub(crate) fn take() -> HashSet<StableNodeId> {
        MEASURED.with(|measured| {
            measured
                .borrow_mut()
                .as_mut()
                .map(std::mem::take)
                .unwrap_or_default()
        })
    }
}
