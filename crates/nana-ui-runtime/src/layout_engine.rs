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
use placement::*;
mod inline;
use inline::*;
mod flex;
#[cfg(test)]
mod inline_scope;
use flex::*;
// These caches use internal numeric identities/constraint bits, not external text keys.
use hashbrown::HashMap;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::box_layout::text_line_box_height_px;
use nana_ui_core::{
    AlignSpec, BoxSizing, ClearSpec, DisplaySpec, FlexDirection, FlexWrap, FloatSpec,
    FontSizeContext, GridAutoFlow, GridLine, GridPlacement, GridRepeatAuto, GridTemplateAreas,
    GridTrack, JustifySpec, LayoutStyle, LengthSpec, PositionSpec, TextAlignSpec,
    resolve_grid_track_sizes,
};

use crate::layout_frontier::{LayoutFrontier, LayoutFrontierSeed, LayoutFrontierStats};
use crate::{
    DocumentId, LayoutBox, LayoutInput, MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld,
    UiWorldError,
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
/// and returns atomic layout writeback. Vue `measure_layout` and css-parity
/// call [`Self::layout_style_tree`] so mixed trees and fixtures share this
/// algorithm.
#[derive(Debug, Default, Clone, Copy)]
pub struct RuntimeLayoutEngine;

/// Style-only tree accepted by [`RuntimeLayoutEngine::layout_style_tree`].
///
/// Vue `LayoutNode` and css-parity fixtures adapt onto this type; they do not
/// keep a second layout algorithm.
#[derive(Debug, Clone, Default)]
pub struct StyleLayoutNode {
    pub id: String,
    pub style: LayoutStyle,
    pub children: Vec<StyleLayoutNode>,
    pub text: Option<String>,
}

impl RuntimeLayoutEngine {
    pub fn layout_document(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
    ) -> Result<Vec<(StableNodeId, LayoutBox)>, UiWorldError> {
        let order = world.document_order(document);
        let mut nodes = LayoutInputMap::new(world);
        nodes.prefetch(&order)?;
        let roots = world.document_roots(document);
        let mut output = HashMap::with_capacity(nodes.len());
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
            .map(|id| (id, output.remove(&id).unwrap_or_default()))
            .collect())
    }

    /// Incremental retained layout driven by typed invalidation seeds.
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
        let emitted = self.layout_document_with_frontier_unverified(
            world,
            document,
            viewport,
            typed_seeds,
            retained,
            force_full,
        )?;
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
        if roots.is_empty() {
            retained.remove_document(document);
            return Ok(Vec::new());
        }
        let retained = retained.documents.entry(document).or_default();
        // Reset before the full-layout reuse return, which runs no measure or placement.
        retained.execution_stats = LayoutExecutionStats::default();
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
        if force_full && world.layout_source_reusable() {
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
                            snapshot.far_start.clone(),
                            snapshot.contexts.clone(),
                        )
                    })
            });
            if let Some((emitted, used_padding, far_start, contexts)) = reused {
                retained.boxes.clear();
                retained.boxes.extend(emitted.iter().copied());
                retained.used_padding = used_padding;
                retained.far_start = far_start;
                retained.contexts = contexts;
                retained.intrinsics.clear();
                retained.intrinsic_metrics.clear();
                retained.placements.clear();
                retained.container_plans.clear();
                retained.measure_plans.clear();
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
        if force_full {
            let order = world.document_order(document);
            nodes.prefetch(&order)?;
        }
        #[cfg(feature = "benchmark")]
        phase.lap(0);
        let frontier = if force_full {
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
            HashSet::new()
        } else {
            frontier.nodes().clone()
        };
        retained.frontier_stats = LayoutFrontierStats::from_frontier(&frontier);
        // Used-size memos on the measure frontier are stale. Intrinsic facts
        // stay until publication so an identical recompute does not bump generation.
        for id in frontier.measure_nodes() {
            retained.intrinsics.remove(id);
        }
        let mut output = HashMap::with_capacity(nodes.len());
        let mut intrinsic = PassIntrinsicCache::with_capacity(nodes.len());
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
                    retained.used_padding.get(&parent).copied(),
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
        let scope = ScopeContext {
            affected: &affected,
            measure: frontier.measure_nodes(),
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
                &mut nodes,
                &mut intrinsic,
                &mut output,
                scope_ref,
                None,
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
                &mut nodes,
                &mut intrinsic,
                &mut output,
                scope_ref,
                None,
            )?;
            #[cfg(feature = "benchmark")]
            phase.lap(2);
        }
        if !force_full {
            // An affected node that no longer generates a box (hidden, or in
            // a closed branch) is skipped by its container rather than
            // placed: collapse what it kept, and everything under it.
            collapse_omitted_boxes(&affected, &mut nodes, &*retained, &mut output);
        }
        // Publish recomputed boxes from the placed set; no document_order walk.
        let mut emitted = output.into_iter().collect::<Vec<_>>();
        emitted.sort_unstable_by_key(|(id, _)| *id);
        for (id, box_) in &emitted {
            retained.boxes.insert(*id, *box_);
        }
        retained.used_padding.extend(nodes.used_padding.drain());
        retained.far_start.extend(nodes.far_start.drain());
        retained.contexts.extend(nodes.contexts.drain());
        retained.placements.extend(nodes.placements.drain());
        for (id, plan) in nodes.container_plans.drain() {
            match plan {
                Some(mut plan) => {
                    // Gate E: one plan per container, entries are the direct
                    // participants recorded this pass. A repeat edit replaces
                    // the previous plan instead of appending another.
                    bound_container_plan(&mut plan);
                    retained.container_plans.insert(id, plan);
                }
                None => {
                    retained.container_plans.remove(&id);
                }
            }
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
            for mut plan in plans.into_plans().collect::<Vec<_>>().into_iter().rev() {
                bound_measure_plan(&mut plan);
                slots.insert(plan);
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
        let intrinsic_counters = intrinsic.counters();
        retained.execution_stats = intrinsic.execution_stats;
        let universe = if force_full { nodes.len() } else { world.len() };
        for (key, size) in std::mem::take(&mut intrinsic.used_order) {
            retained
                .intrinsics
                .entry(key.id)
                .or_default()
                .insert(key, size);
        }
        // Four constraint variants a live node, never a fixed count: a cap
        // below the document's working set evicts facts the next frame reads,
        // and every large document then measured again what it had evicted --
        // work that grew with the document, not with the edit.
        let retained_metric_budget = universe
            .saturating_mul(4)
            .max(crate::IntrinsicCacheBudget::default().max_entries);
        // A patched measurement leaves the facts measured before it behind:
        // a later constraint its used sizes do not hold would read them and
        // miss the patch. Facts this pass measured come back in below.
        for id in std::mem::take(&mut intrinsic.retired_facts) {
            retained.intrinsic_metrics.remove_content(id.get());
        }
        retained.retain_intrinsic_metrics(intrinsic.new_metrics, retained_metric_budget);
        retained.materialized_inputs = nodes.materialized;
        retained.record_intrinsic_counters(intrinsic_counters);
        // Despawned ids linger in the retained maps; keep them bounded.
        // Scoped passes only materialize a subset, so membership is the live
        // world, not the partial input map.
        if retained.boxes.len() > universe.saturating_mul(2) {
            retained.execution_stats.retain_sweeps =
                retained.execution_stats.retain_sweeps.saturating_add(1);
            retained.boxes.retain(|id, _| world.contains(*id));
            retained.placements.retain(|id, _| world.contains(*id));
            retained.used_padding.retain(|id, _| world.contains(*id));
            retained.far_start.retain(|id, _| world.contains(*id));
            retained.contexts.retain(|id, _| world.contains(*id));
            retained.container_plans.retain(|id, _| world.contains(*id));
            retained.measure_plans.retain(|id, _| world.contains(*id));
            retained.envelopes.retain(|id, _| world.contains(*id));
            retained.line_solves.retain(|id, _| world.contains(*id));
            retained
                .applied_adjustments
                .retain(|id, _| world.contains(*id));
        }
        if retained.intrinsics.len() > universe.saturating_mul(2) {
            retained.intrinsics.retain(|id, _| world.contains(*id));
        }
        if retained.intrinsic_metrics.len() > universe.saturating_mul(2) {
            retained.intrinsic_metrics.retain_contents(|content| {
                StableNodeId::new(content).is_some_and(|id| world.contains(id))
            });
        }
        #[cfg(feature = "benchmark")]
        phase.lap(3);
        if force_full && world.layout_source_reusable() {
            let snapshot = FullLayoutSnapshot {
                viewport_width: viewport.width.to_bits(),
                viewport_height: viewport.height.to_bits(),
                epoch: world.layout_source_epoch(),
                emitted: emitted.clone(),
                used_padding: retained.used_padding.clone(),
                far_start: retained.far_start.clone(),
                contexts: retained.contexts.clone(),
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
    /// Hidden / `display:none` nodes are omitted from the result (css-parity /
    /// Vue measure contract). Product `UiWorld` still records a zero box.
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

/// Cross-frame layout memo for scoped relayout: last published boxes, used-size
/// resolutions, and content-derived intrinsic facts. Each document owns its
/// entries, so a full pass cannot invalidate another window's layout.
#[derive(Default)]
pub struct RetainedLayoutCache {
    documents: HashMap<DocumentId, DocumentLayoutCache>,
}

impl RetainedLayoutCache {
    /// Cumulative intrinsic measurement work observed by scoped layout.  The
    /// snapshot is cumulative across documents and remains available to the
    /// frame driver until the next read/reset.
    pub fn intrinsic_cache_counters(&self) -> crate::IntrinsicCacheCounters {
        let mut counters = crate::IntrinsicCacheCounters::default();
        for document in self.documents.values() {
            let snapshot = document.intrinsic_counters;
            let entries = counters.entries.saturating_add(snapshot.entries);
            let bytes = counters.bytes.saturating_add(snapshot.bytes);
            counters.accumulate(snapshot);
            counters.entries = entries;
            counters.bytes = bytes;
        }
        counters
    }

    pub(crate) fn take_intrinsic_counters(&mut self) -> crate::IntrinsicCacheCounters {
        let mut counters = crate::IntrinsicCacheCounters::default();
        for document in self.documents.values_mut() {
            let snapshot = std::mem::take(&mut document.intrinsic_counters);
            let entries = counters.entries.saturating_add(snapshot.entries);
            let bytes = counters.bytes.saturating_add(snapshot.bytes);
            counters.accumulate(snapshot);
            counters.entries = entries;
            counters.bytes = bytes;
        }
        counters
    }

    /// Release all layout state owned by a closed document.
    pub fn remove_document(&mut self, document: DocumentId) {
        self.documents.remove(&document);
    }

    /// Release one deleted node without scanning cached nodes or constraints.
    pub fn remove_node(&mut self, document: DocumentId, id: StableNodeId) {
        if let Some(cache) = self.documents.get_mut(&document) {
            cache.intrinsics.remove(&id);
            cache.intrinsic_metrics.remove_content(id.get());
            cache.boxes.remove(&id);
            cache.placements.remove(&id);
            cache.used_padding.remove(&id);
            cache.far_start.remove(&id);
            cache.contexts.remove(&id);
            cache.container_plans.remove(&id);
            cache.measure_plans.remove(&id);
            cache.envelopes.remove(&id);
            cache.line_solves.remove(&id);
            cache.applied_adjustments.remove(&id);
        }
    }

    pub(crate) fn execution_stats(&self, document: DocumentId) -> LayoutExecutionStats {
        self.documents
            .get(&document)
            .map(|cache| cache.execution_stats)
            .unwrap_or_default()
    }

    /// What the retained cache holds for `document`.
    #[cfg(test)]
    pub(crate) fn footprint(&self, document: DocumentId) -> RetainedLayoutFootprint {
        self.documents
            .get(&document)
            .map(DocumentLayoutCache::footprint)
            .unwrap_or_default()
    }

    /// What Dynamic Layout kept for `id`: its envelope and the adjustment
    /// its line last assigned it, for devtools.
    pub(crate) fn dynamic_inspection(
        &self,
        document: DocumentId,
        id: StableNodeId,
    ) -> Option<crate::view::DynamicInspection> {
        let cache = self.documents.get(&document)?;
        let envelope = cache.envelopes.get(&id);
        let applied = cache.applied_adjustments.get(&id);
        if envelope.is_none() && applied.is_none() {
            return None;
        }
        Some(crate::view::DynamicInspection {
            generation: envelope.map_or(0, |envelope| envelope.shape.generation),
            segments: envelope.map_or_else(Vec::new, |envelope| envelope.shape.segments().to_vec()),
            applied: applied.map(|applied| crate::view::AppliedInspection {
                inline: applied.inline,
                amount: applied.amount,
                padding: applied.padding,
            }),
        })
    }

    /// Structural counters from the most recent pass for `document`.
    pub(crate) fn frontier_stats(&self, document: DocumentId) -> LayoutFrontierStats {
        self.documents
            .get(&document)
            .map(|cache| cache.frontier_stats)
            .unwrap_or_default()
    }

    /// The frontier of the most recent pass for `document`: why each node it
    /// reached was laid out again. Empty after a full pass.
    pub(crate) fn last_frontier(&self, document: DocumentId) -> Option<&LayoutFrontier> {
        self.documents
            .get(&document)
            .map(|cache| &cache.last_frontier)
    }

    /// Which page axes the last placement of `id` laid its children out
    /// from the far (right / bottom) end: `[horizontal, vertical]`.
    pub(crate) fn far_start(&self, document: DocumentId, id: StableNodeId) -> Option<[bool; 2]> {
        self.documents.get(&document)?.far_start.get(&id).copied()
    }

    /// The formatting context the last placement of `id` ran for its
    /// children. Leaves record none.
    pub(crate) fn formatting_context(
        &self,
        document: DocumentId,
        id: StableNodeId,
    ) -> Option<crate::FormattingContextKind> {
        self.documents.get(&document)?.contexts.get(&id).copied()
    }

    /// The newest intrinsic generation layout holds for `id`'s content.
    pub(crate) fn intrinsic_generation(
        &self,
        document: DocumentId,
        id: StableNodeId,
    ) -> Option<u64> {
        self.documents
            .get(&document)?
            .intrinsic_metrics
            .by_content
            .get(&id.get())?
            .iter()
            .map(|(_, metrics)| metrics.generation)
            .max()
    }

    pub(crate) fn used_padding(
        &self,
        document: DocumentId,
        id: StableNodeId,
    ) -> Option<nana_ui_core::PaddingSpec> {
        self.documents
            .get(&document)?
            .used_padding
            .get(&id)
            .copied()
    }
}

/// Two used-size variants per node allow measure/place reuse without
/// accumulating a new entry for every pixel of an interactive resize. The
/// content-derived facts live in the generation-aware authority beside it.
#[derive(Default)]
struct RetainedIntrinsic {
    measurements: [Option<(MeasurementKey, Size)>; 2],
}

impl RetainedIntrinsic {
    fn get(&self, key: MeasurementKey) -> Option<Size> {
        self.measurements
            .iter()
            .flatten()
            .find(|(held, _)| *held == key)
            .map(|(_, size)| *size)
    }

    fn insert(&mut self, key: MeasurementKey, size: Size) {
        let next = Some((key, size));
        if self.measurements[0].is_some_and(|(held, _)| held == key) {
            self.measurements[0] = next;
            return;
        }
        self.measurements[1] = self.measurements[0];
        self.measurements[0] = next;
    }
}

#[derive(Default)]
struct DocumentLayoutCache {
    intrinsics: HashMap<StableNodeId, RetainedIntrinsic>,
    /// Content-derived intrinsic facts, independent from retained used sizes.
    intrinsic_metrics: RetainedIntrinsicMetrics,
    intrinsic_counters: crate::IntrinsicCacheCounters,
    boxes: HashMap<StableNodeId, LayoutBox>,
    materialized_inputs: usize,
    placements: HashMap<StableNodeId, (Point, Size, f32)>,
    pub(crate) used_padding: HashMap<StableNodeId, nana_ui_core::PaddingSpec>,
    /// Per container, the page axes placement starts at the far end.
    far_start: HashMap<StableNodeId, [bool; 2]>,
    /// Per container, the formatting context its last placement ran.
    contexts: HashMap<StableNodeId, crate::FormattingContextKind>,
    /// Cached in-flow child placement per container. See [`ContainerPlan`].
    container_plans: HashMap<StableNodeId, ContainerPlan>,
    /// Cached intrinsic measurement per content-sized container. See
    /// [`MeasurePlan`].
    measure_plans: HashMap<StableNodeId, MeasurePlanSlots>,
    /// Dynamic Layout envelopes, by box (Issue #213).
    envelopes: HashMap<StableNodeId, dynamic::RetainedEnvelope>,
    /// Where each overflowing line's solve stopped, by container.
    line_solves: HashMap<StableNodeId, dynamic::RetainedLineSolve>,
    /// What each box a line shrank resolved inside itself.
    applied_adjustments: HashMap<StableNodeId, dynamic::AppliedAdjustment>,
    frontier_stats: LayoutFrontierStats,
    /// The frontier of the most recent pass, so diagnostics can say why a
    /// node was laid out again. Empty after a full pass.
    last_frontier: LayoutFrontier,
    execution_stats: LayoutExecutionStats,
    /// Last two full-layout results for this document, keyed by viewport and
    /// layout-input epoch. `clear` keeps them: a full pass is what consults
    /// them, and clearing first would drop the hit.
    full_snapshots: [Option<FullLayoutSnapshot>; 2],
    /// The viewport the cache was last laid out against. A pass against
    /// another one plants the resize's seeds itself.
    viewport: Option<LayoutViewport>,
}

/// Entries one document's retained layout cache holds. The memory gate of
/// Issue #259 reads it: every count follows the live tree and the cache
/// budgets, never how many mutations came before.
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RetainedLayoutFootprint {
    pub boxes: usize,
    pub placements: usize,
    /// Retained used sizes, two constraints per node at most.
    pub intrinsics: usize,
    pub intrinsic_metrics: usize,
    pub container_plans: usize,
    pub measure_plans: usize,
    /// Direct participants and tracks the plans store.
    pub plan_entries: usize,
    /// Full-layout results kept for a viewport round trip. Only a full pass
    /// records one; the product frame never runs one.
    pub full_snapshots: usize,
    /// Nodes of the last pass's frontier, kept for diagnostics.
    pub last_frontier: usize,
    /// Dynamic Layout envelopes, line solves and the participants they
    /// hold, and resolved adjustments.
    pub envelopes: usize,
    pub line_solves: usize,
    pub line_solve_entries: usize,
    pub applied_adjustments: usize,
}

struct FullLayoutSnapshot {
    viewport_width: u32,
    viewport_height: u32,
    epoch: u64,
    emitted: Vec<(StableNodeId, LayoutBox)>,
    used_padding: HashMap<StableNodeId, nana_ui_core::PaddingSpec>,
    far_start: HashMap<StableNodeId, [bool; 2]>,
    contexts: HashMap<StableNodeId, crate::FormattingContextKind>,
}

/// Retained intrinsic facts, grouped by the content they describe. A node has
/// a few constraint variants; replacing or dropping one node's facts touches
/// those variants and nothing else, so neither costs the document.
#[derive(Default)]
struct RetainedIntrinsicMetrics {
    by_content: HashMap<u64, Vec<(crate::IntrinsicCacheKey, crate::IntrinsicMetrics)>>,
    len: usize,
}

impl RetainedIntrinsicMetrics {
    fn get(&self, key: &crate::IntrinsicCacheKey) -> Option<crate::IntrinsicMetrics> {
        self.by_content
            .get(&key.content)?
            .iter()
            .find(|(held, _)| held == key)
            .map(|(_, metrics)| *metrics)
    }

    fn len(&self) -> usize {
        self.len
    }

    fn clear(&mut self) {
        self.by_content.clear();
        self.len = 0;
    }

    fn insert(&mut self, key: crate::IntrinsicCacheKey, metrics: crate::IntrinsicMetrics) {
        let variants = self.by_content.entry(key.content).or_default();
        if let Some(slot) = variants.iter_mut().find(|(held, _)| *held == key) {
            slot.1 = metrics;
        } else {
            variants.push((key, metrics));
            self.len += 1;
        }
    }

    /// Drop every variant of one content's facts.
    fn remove_content(
        &mut self,
        content: u64,
    ) -> Option<Vec<(crate::IntrinsicCacheKey, crate::IntrinsicMetrics)>> {
        let removed = self.by_content.remove(&content)?;
        self.len -= removed.len();
        Some(removed)
    }

    fn retain_contents(&mut self, mut keep: impl FnMut(u64) -> bool) {
        let mut dropped = 0;
        self.by_content.retain(|content, variants| {
            let kept = keep(*content);
            if !kept {
                dropped += variants.len();
            }
            kept
        });
        self.len -= dropped;
    }

    /// Evict whole contents, first in table order, until at most `limit`
    /// entries remain; returns how many entries went.
    fn evict_to(&mut self, limit: usize) -> usize {
        let excess = self.len.saturating_sub(limit);
        if excess == 0 {
            return 0;
        }
        // Collected in one walk: taking the first key again after every
        // removal rescans the emptied front of the table each time.
        let mut victims = Vec::new();
        let mut covered = 0;
        for (content, variants) in &self.by_content {
            if covered >= excess {
                break;
            }
            covered += variants.len();
            victims.push(*content);
        }
        for content in victims {
            self.remove_content(content);
        }
        covered
    }
}

fn intrinsic_facts_changed(
    mut existing: crate::IntrinsicMetrics,
    mut incoming: crate::IntrinsicMetrics,
) -> bool {
    existing.generation = 0;
    incoming.generation = 0;
    existing != incoming
}

impl DocumentLayoutCache {
    fn record_intrinsic_counters(&mut self, counters: crate::IntrinsicCacheCounters) {
        self.intrinsic_counters.accumulate(counters);
    }

    fn retain_intrinsic_metrics(
        &mut self,
        metrics: HashMap<crate::IntrinsicCacheKey, crate::IntrinsicMetrics>,
        max_entries: usize,
    ) {
        // A changed content drops every variant it held; the facts this pass
        // produced for it carry the next generation. Dropping one content
        // leaves every other content's comparison as it was.
        let mut next_generation = HashMap::new();
        for (key, incoming) in &metrics {
            if next_generation.contains_key(&key.content)
                || self
                    .intrinsic_metrics
                    .get(key)
                    .is_some_and(|existing| !intrinsic_facts_changed(existing, *incoming))
            {
                continue;
            }
            let generation = self
                .intrinsic_metrics
                .remove_content(key.content)
                .and_then(|variants| variants.iter().map(|(_, held)| held.generation).max())
                .unwrap_or(0);
            next_generation.insert(key.content, generation);
        }
        for (key, mut incoming) in metrics {
            let Some(&previous) = next_generation.get(&key.content) else {
                continue;
            };
            incoming.generation = previous.saturating_add(1).max(1);
            self.intrinsic_metrics.insert(key, incoming);
        }
        self.intrinsic_counters.generation_bumps = self
            .intrinsic_counters
            .generation_bumps
            .saturating_add(next_generation.len());
        // The per-pass authority enforces its byte budget. The retained mirror
        // holds fixed-size entries for the live tree, so its entry budget,
        // proportional to that tree, is its byte budget: it does not let every
        // viewport or constraint variant accumulate across frames.
        let evicted = self.intrinsic_metrics.evict_to(max_entries);
        self.intrinsic_counters.evictions =
            self.intrinsic_counters.evictions.saturating_add(evicted);
    }

    /// Direct participants and tracks stored on this document's context plans.
    ///
    /// Mutation history does not add entries: a later pass replaces the plan
    /// for the same container. See [`bound_container_plan`].
    #[cfg(test)]
    fn retained_plan_entries(&self) -> usize {
        let mut count = 0usize;
        for plan in self.container_plans.values() {
            count = count.saturating_add(plan.entries.borrow().len());
            count = count.saturating_add(plan.overlay.len());
            if let Some(grid) = &plan.grid {
                count = count.saturating_add(grid.items.len());
                count = count.saturating_add(grid.col_sizes.len());
                count = count.saturating_add(grid.row_sizes.len());
            }
        }
        for slots in self.measure_plans.values() {
            for plan in slots.slots.iter().flatten() {
                count = count.saturating_add(plan.entries.len());
                if let Some(grid) = &plan.grid {
                    count = count.saturating_add(grid.items.len());
                    count = count.saturating_add(grid.col_sizes.len());
                    count = count.saturating_add(grid.row_sizes.len());
                }
            }
        }
        count
    }

    #[cfg(test)]
    fn footprint(&self) -> RetainedLayoutFootprint {
        RetainedLayoutFootprint {
            boxes: self.boxes.len(),
            placements: self.placements.len(),
            intrinsics: self
                .intrinsics
                .values()
                .map(|slots| slots.measurements.iter().flatten().count())
                .sum(),
            intrinsic_metrics: self.intrinsic_metrics.len(),
            container_plans: self.container_plans.len(),
            measure_plans: self.measure_plans.len(),
            plan_entries: self.retained_plan_entries(),
            full_snapshots: self.full_snapshots.iter().flatten().count(),
            last_frontier: self.last_frontier.nodes().len(),
            envelopes: self.envelopes.len(),
            line_solves: self.line_solves.len(),
            line_solve_entries: self
                .line_solves
                .values()
                .map(dynamic::RetainedLineSolve::participants)
                .sum(),
            applied_adjustments: self.applied_adjustments.len(),
        }
    }

    fn clear(&mut self) {
        self.intrinsics.clear();
        self.intrinsic_metrics.clear();
        self.intrinsic_counters = crate::IntrinsicCacheCounters::default();
        self.placements.clear();
        self.boxes.clear();
        self.used_padding.clear();
        self.far_start.clear();
        self.contexts.clear();
        self.container_plans.clear();
        self.measure_plans.clear();
        self.envelopes.clear();
        self.line_solves.clear();
        self.applied_adjustments.clear();
        self.frontier_stats = LayoutFrontierStats::default();
        self.last_frontier = LayoutFrontier::default();
        self.execution_stats = LayoutExecutionStats::default();
        self.materialized_inputs = 0;
    }
}

/// Coarse phase clocks for `--profile-layout`.
#[cfg(feature = "benchmark")]
pub mod plan_stats {
    use std::cell::Cell;

    /// Coarse clocks for `--profile-layout`. Slots:
    /// prefetch, root measure, root place, engine tail,
    /// large-container child measure, large-container measure fold,
    /// large-container place remeasure, large-container place pack,
    /// result build, result store,
    /// writeback compare, view scan, commit, publish,
    /// plain-leaf content, plain-leaf baseline, plain-leaf metric record.
    const PHASES: usize = 17;

    thread_local! {
        static PHASE_NS: Cell<[u64; 17]> = const { Cell::new([0; 17]) };
    }

    pub(crate) fn add_phase(slot: usize, elapsed: std::time::Duration) {
        PHASE_NS.with(|cell| {
            let mut slots = cell.get();
            slots[slot] = slots[slot].saturating_add(elapsed.as_nanos() as u64);
            cell.set(slots);
        });
    }

    pub fn take_phases_ns() -> [u64; PHASES] {
        PHASE_NS.with(|cell| cell.replace([0; PHASES]))
    }

    pub(crate) struct PhaseClock {
        start: std::time::Instant,
    }

    impl PhaseClock {
        pub(crate) fn start() -> Self {
            Self {
                start: std::time::Instant::now(),
            }
        }

        pub(crate) fn lap(&mut self, slot: usize) {
            let now = std::time::Instant::now();
            add_phase(slot, now.saturating_duration_since(self.start));
            self.start = now;
        }
    }
}

/// One positioned child of a cached container. The list is the direct
/// participants of that positioned formatting context, replaced when the
/// container is recorded again.
#[derive(Clone)]
struct PlannedOverlay {
    child: StableNodeId,
    /// Retained style and used box for this participant. A later pass replaces
    /// the record; the fields are the cached placement, not a second authority.
    #[allow(dead_code)]
    style: Arc<nana_ui_core::LayoutStyle>,
    fixed: bool,
    /// The used box reads the containing block's size (percentage, fill,
    /// or both insets). A fixed child reads the viewport instead.
    tracks_containing_block: bool,
    #[allow(dead_code)]
    base: Size,
    #[allow(dead_code)]
    base_origin: Point,
    #[allow(dead_code)]
    origin: Point,
    #[allow(dead_code)]
    size: Size,
}

/// One child's contribution to a cached container placement.
#[derive(Clone)]
struct PlannedChild {
    child: StableNodeId,
    /// The child's own layout style at plan time, compared by pointer. This is
    /// what catches a style edit that moves a child without resizing it --
    /// `margin`, `align_self`, `order`, `flex_grow`.
    style: Arc<nana_ui_core::LayoutStyle>,
    /// Intrinsic size measured BEFORE flex distribution: the pure input the
    /// rest of the container's placement is a function of.
    intrinsic: Size,
    /// The main size the line gave this child when it differs from the
    /// intrinsic one, and the child measured at it: its cross size is then
    /// the line's input, and an edit can change it while the intrinsic stays.
    at_main: Option<(f32, Size)>,
    origin: Point,
    size: Size,
    /// Main-axis cursor before this child, i.e. the prefix sum of every
    /// preceding child's outer main extent plus gaps. Lets a suffix replay
    /// start at any index in O(1) instead of re-accumulating from zero.
    cursor_before: f32,
    /// The first baseline the child's line aligned it by, when it aligns by
    /// baseline. A child whose baseline moves moves the rest of its line,
    /// even when no size changed: its text grew inside a fixed box.
    baseline: Option<f32>,
}

/// One grid item's occupied tracks and the intrinsic contribution those
/// tracks were solved from. Spans are the placement result, not a second
/// copy of the child's style.
#[derive(Clone)]
struct GridItemPlan {
    child: StableNodeId,
    col: u32,
    row: u32,
    col_span: u32,
    row_span: u32,
    contribution: Size,
}

/// Resolved grid tracks plus the contribution that produced them.
///
/// A later pass re-solves every track from these contributions. Items whose
/// contribution and cell constraint are unchanged are not measured again.
/// Subgrid, a child entering or leaving flow, and a container whose own
/// content-sized keywords this record cannot finish fall back to measuring
/// this grid, not the document.
#[derive(Clone)]
struct GridTrackPlan {
    items: Vec<GridItemPlan>,
    col_sizes: Vec<f32>,
    row_sizes: Vec<f32>,
    col_gap: f32,
    row_gap: f32,
}

impl GridTrackPlan {
    fn from_layout(grid: &Grid2DLayout) -> Self {
        Self {
            items: grid
                .items
                .iter()
                .map(|item| GridItemPlan {
                    child: item.id,
                    col: item.col as u32,
                    row: item.row as u32,
                    col_span: item.col_span as u32,
                    row_span: item.row_span as u32,
                    contribution: item.intrinsic,
                })
                .collect(),
            col_sizes: grid.col_sizes.clone(),
            row_sizes: grid.row_sizes.clone(),
            col_gap: grid.col_gap,
            row_gap: grid.row_gap,
        }
    }

    fn cell(&self, item: &GridItemPlan) -> (f32, f32) {
        (
            grid_span_extent(
                &self.col_sizes,
                item.col as usize,
                item.col_span as usize,
                self.col_gap,
            ),
            grid_span_extent(
                &self.row_sizes,
                item.row as usize,
                item.row_span as usize,
                self.row_gap,
            ),
        )
    }
}

fn grid_child_in_flow(style: &LayoutStyle) -> bool {
    !style.omits_box()
        && !style.position.is_out_of_flow()
        && !style.display.is_some_and(DisplaySpec::is_contents)
}

/// Both axes are a length that does not read the grid content box, so a
/// sibling-driven change of that box does not change this item's contribution.
fn grid_contribution_ignores_content_box(style: &LayoutStyle) -> bool {
    fn fixed(spec: Option<LengthSpec>) -> bool {
        matches!(
            spec,
            Some(
                LengthSpec::Px(_)
                    | LengthSpec::Em(_)
                    | LengthSpec::Rem(_)
                    | LengthSpec::CalcEmOffset { .. }
                    | LengthSpec::CalcRemOffset { .. }
            )
        )
    }
    style.aspect_ratio.is_none()
        && fixed(style.width)
        && fixed(style.height)
        && style.min_width.is_none_or(|spec| fixed(Some(spec)))
        && style.min_height.is_none_or(|spec| fixed(Some(spec)))
        && style.max_width.is_none_or(|spec| fixed(Some(spec)))
        && style.max_height.is_none_or(|spec| fixed(Some(spec)))
}

fn sort_ids_with_sizes(ids: &mut [StableNodeId], sizes: &mut [Size], nodes: &LayoutInputMap<'_>) {
    if ids.len() != sizes.len() {
        return;
    }
    let order_of = |id: StableNodeId| nodes.style(id).map(|style| style.order).unwrap_or(0);
    if ids.iter().copied().all(|id| order_of(id) == 0) {
        return;
    }
    let mut order: Vec<usize> = (0..ids.len()).collect();
    order.sort_by_key(|&index| order_of(ids[index]));
    let old_ids = ids.to_vec();
    let old_sizes = sizes.to_vec();
    for (slot, index) in order.into_iter().enumerate() {
        ids[slot] = old_ids[index];
        sizes[slot] = old_sizes[index];
    }
}

/// Where a container's child list differs from the one a plan recorded: the
/// children between the prefix and the suffix the two lists share left it
/// (`old[start..old_end]`) and arrived (`new[start..new_end]`). An insertion,
/// a removal, or a move within the list is one such range; a child that moved
/// within it both left and arrived.
///
/// A sequential plan takes the edit instead of walking every child again: a
/// child that left takes its share out, one that arrived is measured and puts
/// its share in, and the children after the edit move by the difference.
/// Finding the range compares ids; it measures nothing.
#[derive(Debug, Clone, Copy)]
struct ChildListEdit {
    start: usize,
    old_end: usize,
    new_end: usize,
}

impl ChildListEdit {
    fn between(old: &[StableNodeId], new: &[StableNodeId]) -> Self {
        let start = old
            .iter()
            .zip(new)
            .take_while(|(old, new)| old == new)
            .count();
        let shared = old.len().min(new.len()) - start;
        let suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(shared)
            .take_while(|(old, new)| old == new)
            .count();
        Self {
            start,
            old_end: old.len() - suffix,
            new_end: new.len() - suffix,
        }
    }

    fn left<'a>(&self, old: &'a [StableNodeId]) -> &'a [StableNodeId] {
        &old[self.start..self.old_end]
    }

    fn arrivals<'a>(&self, new: &'a [StableNodeId]) -> &'a [StableNodeId] {
        &new[self.start..self.new_end]
    }

    /// Whether `child`, one of `container`'s children now, arrived in this
    /// edit, by its index in the new list.
    fn arrived(&self, world: &UiWorld, container: StableNodeId, child: StableNodeId) -> bool {
        world
            .child_index(container, child)
            .is_some_and(|index| (self.start..self.new_end).contains(&index))
    }
}

/// A container's placement of its in-flow children, cached across passes.
///
/// The whole point of scoped layout is to charge by the change, but a flex
/// container still had to walk every child to discover that none of them
/// moved: `subtree_unchanged` prunes a child's SUBTREE, not the parent's scan
/// of its siblings. So a one-row edit in an N-row list cost O(N).
///
/// The placement of in-flow children is a pure function of the container's own
/// inputs plus, in order, each child's layout style and intrinsic size. When
/// all of those are unchanged the previous result still holds, so the pass can
/// skip straight to the children the change closure actually reaches.
///
/// Only children in that closure need re-checking: a layout-affecting
/// `set_style` marks the node LAYOUT-dirty (`mark_subtree`), which is what puts
/// it in the closure. `UiWorld::children_layout_style_is_local` guards the
/// cases where an ancestor could move a child's style without touching it.
#[derive(Clone)]
struct ContainerPlan {
    origin: Point,
    size: Size,
    containing: Size,
    parent_font_px: f32,
    viewport: LayoutViewport,
    /// The container's effective style, compared by pointer.
    style: Arc<nana_ui_core::LayoutStyle>,
    /// The container's child list, compared by pointer. A structural edit
    /// copy-on-writes this `Arc` (the cache holds a reference, so the world's
    /// `Arc::make_mut` cannot mutate it in place), so a different pointer is a
    /// different list.
    children: Arc<Vec<StableNodeId>>,
    /// Containing block handed to each child.
    content: Size,
    child_font_px: f32,
    /// Available size each child's intrinsic measurement was taken against.
    child_available: Size,
    main_direction: FlexDirection,
    /// The writing mode and direction the container laid out in, inherited.
    /// An ancestor can change it without touching this container's own style,
    /// so the plan compares it with the other inputs.
    writing: nana_ui_core::WritingContext,
    /// The main and cross axes run from their far page edge — the right or
    /// the bottom. Placement is flow-relative (every cursor and margin is read
    /// from the start edge) and only turned onto the page where an origin is
    /// written; the replay turns it the same way.
    main_reversed: bool,
    cross_reversed: bool,
    /// Origin of the container's content box.
    content_origin: Point,
    /// Main-axis gap between children.
    gap: f32,
    /// True when this container placed its children as a plain accumulation
    /// from the main-start edge, so child `i`'s position depends only on the
    /// children before it — on a reversed axis too, where it is measured back
    /// from the far edge. Everything that would couple siblings is excluded:
    /// wrapping, a `justify-content` that distributes free space, grid tracks,
    /// auto main margins, baseline or center/end cross alignment, and any flex
    /// grow/shrink redistribution (detected from the data -- every child's
    /// used main size equalled its intrinsic).
    ///
    /// Under that shape a resized child shifts exactly the children after it,
    /// so the pass can keep the prefix and replay only the suffix.
    sequential: bool,
    /// In placement order.
    entries: RefCell<Vec<PlannedChild>>,
    /// Main extent of the content box the entries' origins were placed in.
    /// On a reversed main axis an origin is measured back from the far edge,
    /// so when the container's main size moves, every child before the first
    /// changed one moves with that edge. The replay shifts them by the
    /// difference and records the new extent here.
    placed_main: Cell<f32>,
    /// `(child, index into entries)`, sorted by child, so the pass can ask
    /// "which of my children are in the change closure?" without walking every
    /// entry. Scanning the entries instead would leave the fast path O(number
    /// of children), which is the cost it exists to remove.
    by_child: Vec<(StableNodeId, u32)>,
    /// Occupied tracks, contributions, and the resolved track sizes. `None`
    /// on every flex and block plan.
    grid: Option<GridTrackPlan>,
    /// No in-flow child reads this container's content box on either axis,
    /// so a change of this container's own size refreshes positioned children
    /// only.
    cross_independent: bool,
    /// Some in-flow child reads this container's main extent: a main size,
    /// limit or basis against the containing block, say. Its entries hold
    /// the sizes the old extent gave, so a new main size retires the plan.
    main_dependent: bool,
    /// Positioned participants of this container. Empty on a flow-only plan.
    overlay: Vec<PlannedOverlay>,
}

impl ContainerPlan {
    /// Everything the container's own placement depends on, other than its
    /// child list, its children and its used border-box size. A mismatch
    /// here means the plan is about a different layout.
    fn flow_identity_holds(
        &self,
        origin: Point,
        containing: Size,
        parent_font_px: f32,
        viewport: LayoutViewport,
        style: &Arc<nana_ui_core::LayoutStyle>,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        self.writing == writing
            && self.origin == origin
            && self.containing_compatible(containing)
            && self.parent_font_px == parent_font_px
            && viewport_basis(&self.style, self.viewport) == viewport_basis(style, viewport)
            && Arc::ptr_eq(&self.style, style)
    }

    /// Everything the container's own placement depends on, other than its
    /// child list and its children. A mismatch here means the plan is about
    /// a different layout.
    #[allow(clippy::too_many_arguments)]
    fn inputs_match(
        &self,
        origin: Point,
        size: Size,
        containing: Size,
        parent_font_px: f32,
        viewport: LayoutViewport,
        style: &Arc<nana_ui_core::LayoutStyle>,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        self.flow_identity_holds(origin, containing, parent_font_px, viewport, style, writing)
            && self.size_compatible(size)
    }

    /// In-flow start edges stay put when this container's cross size changes
    /// and no child stretches to it. Positioned children that read the
    /// containing block are refreshed separately.
    fn flow_stable_under_own_size(&self, content_origin: Point) -> bool {
        self.cross_independent
            && self.sequential
            && self.grid.is_none()
            && !self.main_reversed
            && !self.cross_reversed
            && self.content_origin == content_origin
    }

    /// Flow reuse, including a containing-block size change that does not
    /// move in-flow children. The child list is the caller's to compare: the
    /// same list replays the children the change closure reaches, a new one
    /// the edit (see [`ChildListEdit`]).
    #[allow(clippy::too_many_arguments)]
    fn can_reuse_flow(
        &self,
        origin: Point,
        size: Size,
        containing: Size,
        content_origin: Point,
        parent_font_px: f32,
        viewport: LayoutViewport,
        style: &Arc<nana_ui_core::LayoutStyle>,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        // The content box's origin is not only the box's origin and its
        // style's padding: a line that shrank the box may have closed its
        // padding up (Issue #212), which moves every child.
        (self.content_origin == content_origin
            && self.inputs_match(
                origin,
                size,
                containing,
                parent_font_px,
                viewport,
                style,
                writing,
            ))
            || (self.flow_identity_holds(
                origin,
                containing,
                parent_font_px,
                viewport,
                style,
                writing,
            ) && self.flow_stable_under_own_size(content_origin))
    }

    /// A sequential plan places from the start edge. Its main size can grow
    /// with a child without moving the cross axis or, on a forward main axis,
    /// the prefix. On a reversed one the replay moves the prefix with the far
    /// edge; see [`Self::placed_main`].
    fn size_compatible(&self, size: Size) -> bool {
        if self.size == size {
            return true;
        }
        // Track sizes are solved again from the recorded contributions, so
        // the used border box may grow with a row or a column. A pass whose
        // children did not change still re-solves when this size moved.
        if self.grid.is_some() {
            return true;
        }
        if self.sequential {
            let held = |extent: fn(Size, FlexDirection) -> f32| {
                extent(self.size, self.main_direction).to_bits()
                    == extent(size, self.main_direction).to_bits()
            };
            return held(cross_extent) && (!self.main_dependent || held(main_extent));
        }
        // A wrap container's cross size is the sum of its line cross sizes.
        // The main size is the line budget; if that moved, line membership
        // has to be solved again by the formatting context.
        if !flex_line_local_style(self.style.as_ref()) || self.main_reversed || self.cross_reversed
        {
            return false;
        }
        match self.main_direction {
            FlexDirection::Row => {
                self.style.height.is_none() && self.size.width.to_bits() == size.width.to_bits()
            }
            FlexDirection::Column => {
                self.style.width.is_none() && self.size.height.to_bits() == size.height.to_bits()
            }
        }
    }

    /// The line budget is this container's main size. The parent's cross size
    /// can grow with this container without changing that budget.
    fn containing_compatible(&self, containing: Size) -> bool {
        if self.containing == containing {
            return true;
        }
        if self.grid.is_some() {
            return true;
        }
        // A container reads its containing block for percentage padding and
        // margins, which resolve against its inline size, and for
        // percentage relative offsets. A content-sized ancestor growing
        // along the block axis changes neither: the plan still holds.
        let inline = |size: Size| self.writing.inline_size(size.width, size.height).to_bits();
        if inline(self.containing) == inline(containing)
            && !reads_containing_block_size(&self.style)
        {
            return true;
        }
        if !flex_line_local_style(self.style.as_ref()) || self.main_reversed || self.cross_reversed
        {
            return false;
        }
        match self.main_direction {
            FlexDirection::Row => self.containing.width.to_bits() == containing.width.to_bits(),
            FlexDirection::Column => {
                self.containing.height.to_bits() == containing.height.to_bits()
            }
        }
    }

    fn child_count(&self) -> usize {
        self.by_child.len()
    }

    /// The index in `entries` of `child`'s recorded placement.
    fn entry_index(&self, child: StableNodeId) -> Option<u32> {
        self.by_child
            .binary_search_by_key(&child, |(child, _)| *child)
            .ok()
            .map(|slot| self.by_child[slot].1)
    }

    /// Every affected direct child still has the role this plan recorded:
    /// in flow, positioned, or omitted. A child that enters or leaves flow, or
    /// becomes positioned, changes the participant lists themselves, which no
    /// replay can patch. Driven from the closure, like [`Self::affected_entries`].
    /// A child that arrived in a child-list `edit` has no role here yet; the
    /// edit places it.
    fn flow_membership_holds(
        &self,
        container: StableNodeId,
        edit: Option<&ChildListEdit>,
        scope: &ScopeContext<'_>,
        nodes: &LayoutInputMap<'_>,
    ) -> bool {
        scope.affected.iter().all(|&child| {
            if nodes.world.parent_id(child) != Some(container)
                || edit.is_some_and(|edit| edit.arrived(nodes.world, container, child))
            {
                return true;
            }
            let Some(style) = nodes.style(child) else {
                return false;
            };
            let was_in_flow = self
                .by_child
                .binary_search_by_key(&child, |(entry, _)| *entry)
                .is_ok();
            let was_positioned = self.overlay.iter().any(|entry| entry.child == child);
            if style.omits_box() {
                !was_in_flow && !was_positioned
            } else if style.position.is_out_of_flow() {
                was_positioned
            } else {
                was_in_flow
            }
        })
    }

    /// Entry indices for the children the change closure reaches, in placement
    /// order. Driven from the closure (small) rather than the child list.
    fn affected_entries(&self, scope: &ScopeContext<'_>) -> Vec<u32> {
        let mut indices: Vec<u32> = scope
            .affected
            .iter()
            .filter_map(|&id| self.entry_index(id))
            .collect();
        indices.sort_unstable();
        indices
    }
}

/// Whether two layout styles are the same INPUT to layout, as opposed to the
/// same spelling of one.
///
/// `direction` is the one field the engine never reads directly: every use goes
/// through [`used_flow_direction`], which is `unwrap_or(Column)`. So `None` and
/// `Some(Column)` are the same layout, and `Some(Row)` is not.
///
/// That distinction is not academic. Three writers disagree about how to spell
/// a default column: `MessageBridge::register` seeds `direction` from the
/// widget kind, the CSS cascade republishes a style that leaves it `None`, and
/// the Runtime's own `Stack` projection writes `Some(Column)` back. On a
/// 2,000-row Vue list all three run every pointer event, so a plain `==`
/// retires the cached plan on every frame while nothing about the layout has
/// changed. That was the whole of the measure plan's benefit: on that
/// benchmark, `==` left settle at 0.786 ms and the container re-measuring
/// 2,000 children per event; this comparison takes it to 0.630 ms and 5.4.
///
/// The equality is exact, not a tolerance: it accepts exactly the pairs that
/// `used_flow_direction` maps to the same axis, and every other field still has
/// to match outright.
/// Whether a flex item's `current` style is its `cached` one but for its
/// cross size: the one change a line-local replay recomputes. Anything else
/// (a margin, `align-self`, `flex-grow`) moves the line or its neighbours.
fn same_but_cross(
    current: &nana_ui_core::LayoutStyle,
    cached: &nana_ui_core::LayoutStyle,
    direction: FlexDirection,
) -> bool {
    let mut aligned = current.clone();
    match direction {
        FlexDirection::Row => aligned.height = cached.height,
        FlexDirection::Column => aligned.width = cached.width,
    }
    layout_inputs_equal(&aligned, cached)
}

/// Whether `style` resolves anything against its containing block's block
/// size: a percentage offset (relative positioning) or a percentage size.
fn reads_containing_block_size(style: &nana_ui_core::LayoutStyle) -> bool {
    let percent = |spec: Option<LengthSpec>| matches!(spec, Some(LengthSpec::Percent(_)));
    percent(style.offset_top)
        || percent(style.offset_bottom)
        || percent(style.offset_left)
        || percent(style.offset_right)
        || percent(style.width)
        || percent(style.height)
        || percent(style.min_width)
        || percent(style.min_height)
        || percent(style.max_width)
        || percent(style.max_height)
}

/// [`layout_inputs_equal`] for a measurement, which no inset reaches: insets
/// place a positioned box in its containing block. What they do to its size
/// -- two opposite insets stretch it -- arrives as the constraint it is
/// measured against, which a plan compares on its own. A virtual row moved
/// down by the row above it keeps its measurement.
fn measure_inputs_equal(a: &nana_ui_core::LayoutStyle, b: &nana_ui_core::LayoutStyle) -> bool {
    if layout_inputs_equal(a, b) {
        return true;
    }
    let insets = |style: &nana_ui_core::LayoutStyle| {
        [
            style.offset_top,
            style.offset_right,
            style.offset_bottom,
            style.offset_left,
        ]
    };
    if insets(a) == insets(b) {
        return false;
    }
    let mut probe = a.clone();
    probe.offset_top = b.offset_top;
    probe.offset_right = b.offset_right;
    probe.offset_bottom = b.offset_bottom;
    probe.offset_left = b.offset_left;
    layout_inputs_equal(&probe, b)
}

fn layout_inputs_equal(a: &nana_ui_core::LayoutStyle, b: &nana_ui_core::LayoutStyle) -> bool {
    if a == b {
        return true;
    }
    if a.direction.unwrap_or(FlexDirection::Column) != b.direction.unwrap_or(FlexDirection::Column)
    {
        return false;
    }
    // Only reached when the styles differ, so the clone is off the hot path:
    // once per closure child that failed the cheap compare.
    let mut probe = a.clone();
    probe.direction = b.direction;
    probe == *b
}

/// Wrap flex whose line breaks depend only on each item's main size.
fn flex_line_local_style(style: &LayoutStyle) -> bool {
    style
        .display
        .is_some_and(|display| display.is_flex_container())
        && matches!(style.flex_wrap, FlexWrap::Wrap)
        && style.justify_content == JustifySpec::Start
        && style.align_items == AlignSpec::Start
        && style.align_content == JustifySpec::Start
        && style.aspect_ratio.is_none()
        && !style.flex_reverse
}

fn child_blocks_flex_line_local(style: &LayoutStyle) -> bool {
    style.flex_grow.unwrap_or(0.0) > 0.0
        || style.flex_shrink.unwrap_or(0.0) > 0.0
        || style.order != 0
        || style.aspect_ratio.is_some()
        || style.clear != ClearSpec::None
        || style
            .align_self
            .is_some_and(|align| align != AlignSpec::Start)
        || matches!(
            style.margin_left,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
        || matches!(
            style.margin_right,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
        || matches!(
            style.margin_top,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
        || matches!(
            style.margin_bottom,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
}

/// One child's contribution to a cached container measurement.
#[derive(Clone)]
struct MeasuredChild {
    child: StableNodeId,
    /// The child's effective layout style at plan time, or `None` when the node
    /// was missing. Compared by pointer with a value fallback, for the same
    /// reason as in [`ContainerPlan`]: a host that rebuilds its style objects
    /// every frame hands back a fresh `Arc` holding an identical style.
    style: Option<Arc<nana_ui_core::LayoutStyle>>,
    /// The intrinsic size measured for this child, or `None` for a child the
    /// flow collection dropped (`display:none`, out of flow). A dropped child
    /// contributes nothing to the container's measurement, and it cannot start
    /// contributing without its own style changing -- which the style compare
    /// above catches.
    intrinsic: Option<Size>,
    /// The main size the line gave this child when it differs from the
    /// measured one, and the child measured at it (its cross size is the
    /// container's input then).
    at_main: Option<(f32, Size)>,
}

/// A container's own intrinsic measurement, cached across passes.
///
/// The measure-side twin of [`ContainerPlan`], and the same shape of hole.
/// [`measure::intrinsic_size_scoped`] short-circuits a node whose width and
/// height both resolve from its own style, which is what stops a dirty frame
/// from re-measuring the whole document. A CONTENT-SIZED container has no such
/// short circuit: its own size is a function of its children, so every affected
/// container re-measured every child -- each one only to hit the retained memo
/// and return the value it already had. Layout invalidation propagates to
/// ancestors, so a single edit puts every content-sized container above it on
/// that path, and the frame is O(number of children) again.
///
/// The measurement is a pure function of the container's own inputs (style,
/// child list, text metrics, available size, viewport, inherited font size,
/// parent flow direction) plus, per child, that child's layout style and its
/// intrinsic size under the recorded available size. When all of those are
/// unchanged the previous result still holds.
///
/// Only children the change closure reaches need re-checking, and the check is
/// driven FROM the closure: a container looks each affected id up in its own
/// sorted entries, rather than walking its children looking for affected ones.
/// Walking the children would leave the fast path O(number of children), which
/// is the cost this exists to remove.
///
/// Entries are sorted by child id rather than kept in flow order: the container
/// is either reusing the whole cached measurement or recomputing it from
/// scratch, and neither needs the order.
///
/// This is a cache of its own, NOT a relaxation of the
/// `retained.intrinsics.remove` in `layout_document_with_frontier`. That removal is
/// still right and still happens: an affected node's memo holds entries for
/// constraint combinations this frame will not measure, and those really are
/// stale. What a plan caches is the container's result under ONE recorded
/// constraint, re-validated against the closure before it is used, so the two
/// do not overlap.
#[derive(Clone)]
struct MeasurePlan {
    /// Constraint the container was measured against. This is the part of the
    /// per-pass cache key that the plan, keyed by id alone, has to carry.
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    /// The container's effective style, compared by pointer with a value
    /// fallback.
    style: Arc<nana_ui_core::LayoutStyle>,
    /// The writing mode and direction the container measured in, inherited;
    /// see [`ContainerPlan::writing`].
    writing: nana_ui_core::WritingContext,
    /// The container's child list, compared by pointer. A structural edit
    /// copy-on-writes this `Arc`, so a different pointer is a different list.
    children: Arc<Vec<StableNodeId>>,
    /// The container's own shaped text, which competes with the children for
    /// the content size.
    text_metrics: Option<crate::TextMetrics>,
    /// That text's unwrapped width, when it wrapped narrower.
    text_natural_width: Option<f32>,
    /// The width that text was last wrapped against.
    text_wrap_limit: Option<f32>,
    /// What the container's visual draws beside that text.
    visual: VisualContent,
    /// Available size in-flow children were measured against. Flex uses one
    /// value for every child. A grid stores the content box here; each item's
    /// contribution lives on [`GridTrackPlan`], and a fill axis is applied
    /// only when that contribution is remeasured.
    child_available: Size,
    /// Flow direction handed to each child as its `parent_direction`.
    child_direction: FlexDirection,
    /// Sorted by child id.
    entries: Vec<MeasuredChild>,
    /// What the measurement produced.
    size: Size,
    /// Main size is the sum of child border boxes, margins, and gaps.
    sequential: bool,
    /// On a sequential plan, that sum, exact: see `sequential_main_term`.
    main_sum: f64,
    /// On a sequential plan, the in-flow children the sum holds, one gap
    /// between each two.
    flow_len: usize,
    /// On a sequential plan, the widest in-flow child's cross extent with its
    /// margins: a child arriving within it leaves the cross size where it
    /// was, and so does one leaving while another is as wide.
    children_cross: f32,
    /// How many in-flow children are that wide.
    children_at_cross: usize,
    /// The cross-axis size the measurement handed its final step, before the
    /// container's own size rules. A sequential patch keeps it.
    default_cross: f32,
    /// Present when this measurement is a grid track solution.
    grid: Option<GridTrackPlan>,
}

/// The measure plans retained for one container.
///
/// A container is commonly measured TWICE per pass under two different
/// constraints: once from its parent's own intrinsic measurement, against the
/// parent's available content box, and once from its parent's placement,
/// against the parent's USED content box. Under an auto-height ancestor those
/// two differ, so a single slot is written by one call and missed by the other,
/// every pass, forever. Two slots is the same answer -- and the same reason --
/// as [`RetainedIntrinsic`].
#[derive(Default)]
struct MeasurePlanSlots {
    slots: [Option<MeasurePlan>; 2],
}

impl MeasurePlanSlots {
    fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    /// Picks the slot recorded under this constraint. Selection only, not a
    /// correctness check: `MeasurePlan::inputs_match` compares `available`
    /// again, so handing back the wrong slot costs a recompute, never a stale
    /// answer.
    fn get(&self, available: Size) -> Option<&MeasurePlan> {
        self.slots
            .iter()
            .flatten()
            .find(|plan| plan.available == available)
    }

    fn insert(&mut self, plan: MeasurePlan) {
        if self.slots[0]
            .as_ref()
            .is_some_and(|held| held.available == plan.available)
        {
            self.slots[0] = Some(plan);
            return;
        }
        self.slots.swap(0, 1);
        self.slots[0] = Some(plan);
    }

    fn clear(&mut self) {
        self.slots = [None, None];
    }

    /// The plans this holder carries, most recent first.
    fn into_plans(self) -> impl Iterator<Item = MeasurePlan> {
        self.slots.into_iter().flatten()
    }
}

impl MeasurePlan {
    /// Everything the measurement depends on other than the child list and
    /// the children. The caller compares the list: the same one re-checks the
    /// children the change closure reaches, a new one takes the edit (see
    /// [`ChildListEdit`]).
    #[allow(clippy::too_many_arguments)]
    fn inputs_match(
        &self,
        available: Size,
        parent_direction: Option<FlexDirection>,
        viewport: LayoutViewport,
        parent_font_px: f32,
        style: &Arc<nana_ui_core::LayoutStyle>,
        text_metrics: Option<crate::TextMetrics>,
        text_natural_width: Option<f32>,
        text_wrap_limit: Option<f32>,
        visual: VisualContent,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        self.writing == writing
            && self.available == available
            && self.parent_direction == parent_direction
            && viewport_basis(&self.style, self.viewport) == viewport_basis(style, viewport)
            && self.parent_font_px == parent_font_px
            && self.text_metrics == text_metrics
            && self.text_natural_width == text_natural_width
            && self.text_wrap_limit == text_wrap_limit
            && self.visual == visual
            && (Arc::ptr_eq(&self.style, style) || measure_inputs_equal(&self.style, style))
    }

    fn entry(&self, child: StableNodeId) -> Option<&MeasuredChild> {
        self.entries
            .binary_search_by_key(&child, |entry| entry.child)
            .ok()
            .map(|slot| &self.entries[slot])
    }
}

/// On-demand `LayoutInput` cache. A miss loads exactly that id from `UiWorld`.
struct LayoutInputMap<'a> {
    world: &'a UiWorld,
    nodes: HashMap<StableNodeId, LayoutInput>,
    /// Effective styles for nodes this pass never materialized into `nodes`.
    ///
    /// A scoped pass prefetches nothing, so an unchanged sibling is reached
    /// only through [`Self::style`] -- and reached repeatedly: flex main-axis
    /// distribution, the baseline fold, and the cross-axis fold each ask for
    /// the same child's style. `UiWorld::effective_layout_style` is not a
    /// field read; it is several hash lookups plus an `Arc` clone, and a
    /// hidden or overlay-hosted node also pays an `Arc::make_mut` clone of the
    /// whole `LayoutStyle`. Resolving that once per node per pass is exact,
    /// not an approximation: `layout_document_with_frontier` borrows the world
    /// immutably for the entire pass, so no resolution can change underneath
    /// this map.
    styles: RefCell<HashMap<StableNodeId, Option<Arc<nana_ui_core::LayoutStyle>>>>,
    materialized: usize,
    placements: HashMap<StableNodeId, (Point, Size, f32)>,
    used_padding: HashMap<StableNodeId, nana_ui_core::PaddingSpec>,
    far_start: HashMap<StableNodeId, [bool; 2]>,
    /// Formatting context each container placed this pass ran.
    contexts: HashMap<StableNodeId, crate::FormattingContextKind>,
    /// Container plans rebuilt this pass. Merged into the retained cache at the
    /// end; containers that took the fast path record nothing, so their
    /// existing plan simply stays. `None` retires a plan recorded when the
    /// container was still on the cacheable path.
    container_plans: HashMap<StableNodeId, Option<ContainerPlan>>,
    /// Measure plans rebuilt this pass, merged the same way. An entry that
    /// ends the pass empty retires the container's retained plans.
    measure_plans: HashMap<StableNodeId, MeasurePlanSlots>,
}

#[derive(Debug, Clone, Copy, Default)]
struct BaselineMetrics {
    first: Option<f32>,
    last: Option<f32>,
}

impl<'a> LayoutInputMap<'a> {
    fn new(world: &'a UiWorld) -> Self {
        Self {
            world,
            nodes: HashMap::new(),
            styles: RefCell::new(HashMap::new()),
            materialized: 0,
            placements: HashMap::new(),
            used_padding: HashMap::new(),
            far_start: HashMap::new(),
            contexts: HashMap::new(),
            container_plans: HashMap::new(),
            measure_plans: HashMap::new(),
        }
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }

    fn prefetch(&mut self, ids: &[StableNodeId]) -> Result<(), UiWorldError> {
        // Full passes prefetch once, immediately after constructing this map.
        // Fill the final cache directly: materializing a temporary Vec and
        // then moving every input into a HashMap doubles the container work on
        // the cold path that large documents pay most often.
        debug_assert!(self.nodes.is_empty());
        if !ids.is_empty() {
            self.world.record_hot_path_allocation(
                1,
                ids.len().saturating_mul(std::mem::size_of::<LayoutInput>()),
            );
        }
        let mut nodes = HashMap::with_capacity(ids.len());
        for &id in ids {
            let input = self.world.layout_input(id)?;
            nodes.insert(id, input);
        }
        self.materialized = nodes.len();
        self.nodes = nodes;
        Ok(())
    }

    fn get(&mut self, id: StableNodeId) -> Result<Option<&LayoutInput>, UiWorldError> {
        match self.nodes.entry(id) {
            hashbrown::hash_map::Entry::Occupied(entry) => Ok(Some(entry.into_mut())),
            hashbrown::hash_map::Entry::Vacant(entry) => {
                let input = match self.world.layout_input(id) {
                    Ok(input) => input,
                    Err(UiWorldError::MissingNode(_)) => return Ok(None),
                    Err(error) => return Err(error),
                };
                self.materialized = self.materialized.saturating_add(1);
                Ok(Some(entry.insert(input)))
            }
        }
    }

    /// Style for classifying / measuring siblings without assembling `LayoutInput`.
    fn style(&self, id: StableNodeId) -> Option<Arc<nana_ui_core::LayoutStyle>> {
        if let Some(node) = self.nodes.get(&id) {
            return Some(Arc::clone(&node.style));
        }
        if let Some(cached) = self.styles.borrow().get(&id) {
            return cached.clone();
        }
        let resolved = self.world.layout_style(id);
        self.styles.borrow_mut().insert(id, resolved.clone());
        resolved
    }

    /// Shared baseline authority for all formatting contexts. Retained
    /// `nana-text` layouts provide first/last line baselines; host-shaped text
    /// falls back to its first-line ascent. Replaced/custom content has an
    /// explicit bottom-edge fallback so it never accidentally inherits text's
    /// approximate ascent.
    fn baseline_metrics(
        &self,
        id: StableNodeId,
        fallback_font_px: f32,
        inline_base: Option<f32>,
        used: Option<Size>,
    ) -> BaselineMetrics {
        let Some(style) = self.style(id) else {
            return BaselineMetrics::default();
        };
        let font = fonts_of(&style, fallback_font_px).element_px;
        let chrome_top =
            style.resolved_padding_against(inline_base).top + style.resolved_border_width();
        let replaced = self.world.custom_render(id).is_some()
            || style.paint.content_image.is_some()
            || style.paint.skipped_replaced.is_some()
            || {
                #[cfg(feature = "image-viewer")]
                {
                    matches!(
                        self.world.standard_visual_ref(id),
                        Some(crate::StandardVisual::ImageViewer { .. })
                    )
                }
                #[cfg(not(feature = "image-viewer"))]
                {
                    false
                }
            };
        if replaced {
            // Replaced content's baseline is its border-box bottom. The
            // measure caller supplies the used extent when it is available;
            // an absent extent remains an explicit fallback for a later query.
            let block = used.and_then(|used| {
                let (_, block) = self
                    .nodes
                    .get(&id)
                    .map(|node| node.writing.logical_size(used.width, used.height))?;
                Some(block.max(0.0))
            });
            return BaselineMetrics {
                first: block,
                last: block,
            };
        }
        if let Some((_, layout)) = self.world.text_layout(id)
            && !layout.is_vertical()
            && !layout.lines.is_empty()
        {
            let first = layout
                .lines
                .first()
                .map(|line| chrome_top + line.metrics.baseline_y_px);
            let last = layout
                .lines
                .last()
                .map(|line| chrome_top + line.metrics.baseline_y_px);
            return BaselineMetrics { first, last };
        }
        if matches!(
            self.world.standard_visual_ref(id),
            Some(crate::StandardVisual::Button { label, .. }) if label.is_empty()
        ) {
            let block = used.map(|used| {
                let writing = self
                    .nodes
                    .get(&id)
                    .map_or_else(Default::default, |node| node.writing);
                writing.logical_size(used.width, used.height).1.max(0.0)
            });
            return BaselineMetrics {
                first: block,
                last: block,
            };
        }
        let button_with_label = matches!(
            self.world.standard_visual_ref(id),
            Some(crate::StandardVisual::Button { label, .. }) if !label.is_empty()
        );
        let ascent = self
            .nodes
            .get(&id)
            .and_then(|node| node.text_metrics)
            .and_then(|metrics| metrics.ascent)
            .filter(|value| value.is_finite() && *value >= 0.0);
        let ascent = ascent.unwrap_or(font * nana_ui_core::TEXT_APPROX_ASCENT_EM);
        let first = Some(chrome_top + ascent);
        if button_with_label {
            // A labelled button's baseline belongs to its internal label
            // content. The shaped text metrics above are authoritative; the
            // approximation is only the same explicit host fallback used
            // when shaping has not produced a line yet.
            return BaselineMetrics { first, last: first };
        }
        BaselineMetrics { first, last: first }
    }

    fn baseline(&self, id: StableNodeId, fallback_font_px: f32, used: Size) -> f32 {
        let writing = self
            .nodes
            .get(&id)
            .map_or_else(Default::default, |node| node.writing);
        let (_, block_extent) = writing.logical_size(used.width, used.height);
        let replaced = self.world.custom_render(id).is_some()
            || self.style(id).is_some_and(|style| {
                style.paint.content_image.is_some() || style.paint.skipped_replaced.is_some()
            })
            || {
                #[cfg(feature = "image-viewer")]
                {
                    matches!(
                        self.world.standard_visual_ref(id),
                        Some(crate::StandardVisual::ImageViewer { .. })
                    )
                }
                #[cfg(not(feature = "image-viewer"))]
                {
                    false
                }
            };
        if replaced {
            // Replaced/custom nodes align to their bottom edge by default.
            return block_extent.max(0.0);
        }
        let metrics = self.baseline_metrics(
            id,
            fallback_font_px,
            Some(writing.inline_size(used.width, used.height)),
            Some(used),
        );
        // Read both ends here so callers share one retained result even when
        // the current parent only asks for first baseline alignment.
        let _last = metrics.last;
        metrics.first.unwrap_or(block_extent.max(0.0))
    }
}

/// Zero-size every kept box at or under an affected node that omits its box
/// and was not placed this pass.
fn collapse_omitted_boxes(
    affected: &HashSet<StableNodeId>,
    nodes: &mut LayoutInputMap<'_>,
    retained: &DocumentLayoutCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
) {
    let collapse = |id: StableNodeId, output: &mut HashMap<StableNodeId, LayoutBox>| {
        if let Some(kept) = retained.boxes.get(&id)
            && (kept.width != 0.0 || kept.height != 0.0)
        {
            // Collapsed where it stood: its origin, no extent.
            output.entry(id).or_insert(LayoutBox {
                x: kept.x,
                y: kept.y,
                ..LayoutBox::default()
            });
        }
    };
    for &id in affected {
        // A container may write the omitted box itself, as a zero box, and
        // stop there: its descendants still keep the boxes they had.
        if !nodes.style(id).is_some_and(|style| style.omits_box()) {
            continue;
        }
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            collapse(node, output);
            if let Some(record) = nodes.world.node(node) {
                stack.extend(record.children.iter().copied());
            }
        }
    }
}

struct ScopeContext<'a> {
    affected: &'a HashSet<StableNodeId>,
    measure: &'a HashSet<StableNodeId>,
    retained: &'a DocumentLayoutCache,
    /// Which boxes lead to an affected node, built once per pass.
    reach: &'a AffectedIndex,
}

/// Every node an affected node is in, and per container the direct children
/// that are affected or contain an affected node. A fixed box may stay out of
/// the frontier while a node inside it changes; every place that prunes a
/// subtree or replays part of a container asks this, so the path down to that
/// node is never cut.
#[derive(Default)]
pub(crate) struct AffectedIndex {
    reach: HashSet<StableNodeId>,
    reaching: HashMap<StableNodeId, Vec<StableNodeId>>,
}

impl AffectedIndex {
    /// From `affected`, leaving out the nodes inside `covered` (island roots,
    /// laid out on their own).
    fn new<'a>(
        world: &UiWorld,
        affected: impl IntoIterator<Item = &'a StableNodeId>,
        covered: &HashSet<StableNodeId>,
    ) -> Self {
        let mut index = Self::default();
        for &id in affected {
            let inside_island = std::iter::successors(Some(id), |id| world.parent_id(*id))
                .any(|node| covered.contains(&node));
            if inside_island || !index.reach.insert(id) {
                continue;
            }
            let mut child = id;
            while let Some(parent) = world.parent_id(child) {
                index.reaching.entry(parent).or_default().push(child);
                if !index.reach.insert(parent) {
                    break;
                }
                child = parent;
            }
        }
        index
    }

    /// Whether `id` is affected or contains an affected node.
    pub(crate) fn reaches(&self, id: StableNodeId) -> bool {
        self.reach.contains(&id)
    }

    /// The direct children of `container` that lead to an affected node.
    pub(crate) fn children(&self, container: StableNodeId) -> &[StableNodeId] {
        self.reaching.get(&container).map_or(&[], Vec::as_slice)
    }
}

/// Prune a child recursion when the child is outside the affected closure and
/// its recomputed entry box is bit-identical to the retained one.
fn subtree_unchanged(
    child: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    child_style: &nana_ui_core::LayoutStyle,
    child_fonts: FontSizeContext,
    // The parent's: it is the child's containing block.
    writing: nana_ui_core::WritingContext,
    scope: Option<&ScopeContext<'_>>,
) -> bool {
    let Some(scope) = scope else {
        return false;
    };
    if scope.affected.contains(&child) || scope.reach.reaches(child) {
        return false;
    }
    let Some(cached) = scope.retained.boxes.get(&child) else {
        return false;
    };
    let (relative_x, relative_y) = child_style.relative_offset_against_fonts(
        Some(containing.width),
        Some(containing.height),
        child_fonts,
    );
    scope.retained.used_padding.get(&child).copied()
        == Some(child_style.resolved_padding_against_fonts(
            Some(writing.inline_size(containing.width, containing.height)),
            child_fonts,
        ))
        && cached.x == origin.x + relative_x
        && cached.y == origin.y + relative_y
        && cached.width == size.width
        && cached.height == size.height
}

fn sort_by_order(ids: &mut [StableNodeId], nodes: &LayoutInputMap<'_>) {
    let order_of = |id: StableNodeId| nodes.style(id).map(|style| style.order).unwrap_or(0);
    // Every key costs a map lookup plus an `Arc` clone, so resolve each one at
    // most once. Siblings almost always keep the default order, and a stable
    // sort on all-equal keys is a no-op, so scan for that case and skip.
    if ids.iter().all(|id| order_of(*id) == 0) {
        return;
    }
    // `sort_by_key` re-evaluates the key on every comparison; `sort_by_cached_key`
    // is likewise stable but evaluates it once per element.
    ids.sort_by_cached_key(|id| order_of(*id));
}

fn uses_2d_grid(style: &LayoutStyle, flow: &[StableNodeId], nodes: &LayoutInputMap<'_>) -> bool {
    if !style.display.is_some_and(DisplaySpec::is_grid_container) {
        return false;
    }
    if style.active_grid_columns().is_some()
        || style.active_grid_rows().is_some()
        || style.grid_columns_repeat.is_some()
        || style.grid_rows_repeat.is_some()
        || style.is_subgrid_columns()
        || style.is_subgrid_rows()
        || style.grid_auto_flow.is_some()
        || style
            .grid_auto_columns
            .as_ref()
            .is_some_and(|tracks| !tracks.is_empty())
        || style
            .grid_auto_rows
            .as_ref()
            .is_some_and(|tracks| !tracks.is_empty())
        || style
            .grid_template_areas
            .as_ref()
            .is_some_and(|areas| !areas.cells.is_empty())
    {
        return true;
    }
    flow.iter().any(|id| {
        nodes
            .style(*id)
            .is_some_and(|child| !child.grid_placement.is_auto())
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Point {
    x: f32,
    y: f32,
}

impl Point {
    const ZERO: Self = Self { x: 0.0, y: 0.0 };
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Size {
    width: f32,
    height: f32,
}

/// All non-content inputs that can change a used measurement.  The containing
/// block size remains in the key, while font, viewport, writing mode, and
/// parent flow are carried alongside it so a compatible constraint cannot
/// accidentally reuse a result from another layout environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MeasurementKey {
    id: StableNodeId,
    width: u32,
    height: u32,
    parent_direction: u8,
    viewport_width: u32,
    viewport_height: u32,
    parent_font: u32,
    writing: nana_ui_core::WritingContext,
    containing_writing: nana_ui_core::WritingContext,
    constraint: crate::ConstraintClass,
    direction_sensitive: bool,
}

impl MeasurementKey {
    fn new(
        id: StableNodeId,
        available: Size,
        parent_direction: Option<FlexDirection>,
        viewport: LayoutViewport,
        parent_font_px: f32,
        writing: nana_ui_core::WritingContext,
        containing_writing: nana_ui_core::WritingContext,
        constraint: crate::ConstraintClass,
        direction_sensitive: bool,
    ) -> Self {
        Self {
            id,
            width: available.width.to_bits(),
            height: available.height.to_bits(),
            parent_direction: match parent_direction {
                None => 0,
                Some(FlexDirection::Column) => 1,
                Some(FlexDirection::Row) => 2,
            },
            viewport_width: viewport.width.to_bits(),
            viewport_height: viewport.height.to_bits(),
            parent_font: parent_font_px.to_bits(),
            writing,
            containing_writing,
            constraint: constraint.normalized(),
            direction_sensitive,
        }
    }
}

fn measurement_constraint_class(
    style: &nana_ui_core::LayoutStyle,
    available: Size,
) -> crate::ConstraintClass {
    let extents = (available.width.max(0.0), available.height.max(0.0));
    if style
        .aspect_ratio
        .is_some_and(|ratio| ratio.is_finite() && ratio > 0.0)
    {
        crate::ConstraintClass::aspect_ratio(extents.0, extents.1)
    } else if style
        .width
        .is_some_and(nana_ui_core::LengthSpec::is_full_percent_fill)
        || style
            .height
            .is_some_and(nana_ui_core::LengthSpec::is_full_percent_fill)
    {
        crate::ConstraintClass::fill(extents.0, extents.1)
    } else {
        crate::ConstraintClass::percentage_cb(extents.0, extents.1)
    }
}

/// Work a layout pass ran. Frontier membership is recorded separately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LayoutExecutionStats {
    pub measure_nodes: usize,
    pub measure_cache_hits: usize,
    pub measure_cache_misses: usize,
    pub placement_nodes: usize,
    pub origin_only_updates: usize,
    /// Containers placed from their retained [`ContainerPlan`].
    pub placement_plans_reused: usize,
    /// Containers measured from a retained [`MeasurePlan`], without a walk of
    /// their children.
    pub measure_plans_reused: usize,
    /// Sequential containers that kept their prefix and replayed only the
    /// children from the first changed one on.
    pub suffixes_replayed: usize,
    /// Children a container walk measured, in its measure and in its
    /// placement: the scan that makes a dirty frame O(N) when a plan misses.
    pub children_measured: usize,
    /// Containers that placed their children but could not record a plan,
    /// so they walk them again on every later pass.
    pub containers_uncacheable: usize,
    /// Containers that had a retained plan for this pass's inputs and still
    /// walked their children: the plan could not answer. A query is a hit
    /// (one of the two `*_plans_reused`) or one of these.
    pub plan_misses: usize,
    /// Plans a walk recorded over an earlier plan for the same container
    /// and constraint. A container's first plan is not a rebuild.
    pub plan_rebuilds: usize,
    /// Positioned contexts laid out whole because their plan was stale.
    pub local_context_fallbacks: usize,
    /// Retained-cache sweeps that dropped despawned ids.
    pub retain_sweeps: usize,
    /// What Dynamic Layout did this pass.
    pub dynamic: nana_ui_core::DynamicLayoutCounters,
}

/// Per-pass used-size memo.  The public [`crate::IntrinsicCache`] owns the
/// generation-aware intrinsic contract; this tiny adapter keeps the existing
/// layout algorithm's physical-size representation while exposing the same
/// hit/miss accounting.  Its key deliberately has no formatting-context id,
/// so a child measured by flex/grid/inline can share the result in a pass.
#[derive(Default)]
struct PassIntrinsicCache {
    /// Intrinsic facts are kept separately from used sizes.  A percentage,
    /// fill, or stretch result is a resolution against one containing block;
    /// it must never become the preferred value in the shared authority.
    cache: crate::IntrinsicCache,
    /// Used-size memo for this pass. Its key contains the concrete available
    /// size because that is exactly what the placement algorithm resolves.
    used: HashMap<MeasurementKey, Size>,
    /// Every entry of `used`, in the order this pass measured it: the
    /// writeback merged into the retained cache at the end. A node keeps two
    /// retained variants; writing them back in measuring order keeps the two
    /// it measured last, where the table's own order would keep whichever two
    /// its hashing visits last, a different pair on every run.
    used_order: Vec<(MeasurementKey, Size)>,
    new_metrics: HashMap<crate::IntrinsicCacheKey, crate::IntrinsicMetrics>,
    /// Containers a measure plan answered by patching what it recorded: the
    /// intrinsic facts retained for them were measured from content the
    /// patch changed, and no longer describe it under any constraint. They
    /// leave the retained cache with this pass.
    retired_facts: Vec<StableNodeId>,
    latest_intrinsic_keys: HashMap<StableNodeId, crate::IntrinsicCacheKey>,
    seeded_intrinsic: HashSet<crate::IntrinsicCacheKey>,
    extra_counters: crate::IntrinsicCacheCounters,
    execution_stats: LayoutExecutionStats,
    /// Dynamic Layout state for the pass.
    dynamic: dynamic::DynamicPass,
}

impl PassIntrinsicCache {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            cache: crate::IntrinsicCache::new(crate::IntrinsicCacheBudget {
                // A node may be measured at its parent width and again at a
                // flex/grid used width. Keep a few constraint classes alive
                // for baseline and cross-context consumers in this pass.
                max_entries: capacity.saturating_mul(4).max(1),
                max_bytes: usize::MAX,
            }),
            used: HashMap::with_capacity(capacity),
            used_order: Vec::with_capacity(capacity),
            new_metrics: HashMap::with_capacity(capacity),
            retired_facts: Vec::new(),
            latest_intrinsic_keys: HashMap::with_capacity(capacity),
            seeded_intrinsic: HashSet::with_capacity(capacity),
            extra_counters: crate::IntrinsicCacheCounters::default(),
            execution_stats: LayoutExecutionStats::default(),
            dynamic: dynamic::DynamicPass::default(),
        }
    }

    fn note_measure_cache_hit(&mut self) {
        self.execution_stats.measure_cache_hits =
            self.execution_stats.measure_cache_hits.saturating_add(1);
    }

    fn note_measure_cache_miss(&mut self) {
        self.execution_stats.measure_cache_misses =
            self.execution_stats.measure_cache_misses.saturating_add(1);
    }

    fn note_measure_node(&mut self) {
        self.execution_stats.measure_nodes = self.execution_stats.measure_nodes.saturating_add(1);
    }

    fn note_placement_plan_reused(&mut self) {
        self.execution_stats.placement_plans_reused = self
            .execution_stats
            .placement_plans_reused
            .saturating_add(1);
    }

    fn note_measure_plan_reused(&mut self) {
        self.execution_stats.measure_plans_reused =
            self.execution_stats.measure_plans_reused.saturating_add(1);
    }

    /// `id`'s measure plan answered by a patch: see `retired_facts`.
    fn retire_intrinsic_facts(&mut self, id: StableNodeId) {
        self.retired_facts.push(id);
    }

    fn note_suffix_replayed(&mut self) {
        self.execution_stats.suffixes_replayed =
            self.execution_stats.suffixes_replayed.saturating_add(1);
    }

    fn note_child_measured(&mut self) {
        self.execution_stats.children_measured =
            self.execution_stats.children_measured.saturating_add(1);
    }

    fn note_container_uncacheable(&mut self) {
        self.execution_stats.containers_uncacheable = self
            .execution_stats
            .containers_uncacheable
            .saturating_add(1);
    }

    fn note_plan_miss(&mut self) {
        self.execution_stats.plan_misses = self.execution_stats.plan_misses.saturating_add(1);
    }

    fn note_plan_rebuilt(&mut self) {
        self.execution_stats.plan_rebuilds = self.execution_stats.plan_rebuilds.saturating_add(1);
    }

    fn note_local_context_fallback(&mut self) {
        self.execution_stats.local_context_fallbacks = self
            .execution_stats
            .local_context_fallbacks
            .saturating_add(1);
    }

    fn note_placement_node(&mut self) {
        self.execution_stats.placement_nodes =
            self.execution_stats.placement_nodes.saturating_add(1);
    }

    fn note_origin_only(&mut self) {
        self.execution_stats.origin_only_updates =
            self.execution_stats.origin_only_updates.saturating_add(1);
    }

    fn get(&mut self, key: &MeasurementKey) -> Option<Size> {
        // A hit answers the query. Intrinsic accounting stays on the miss
        // path; this memo must not increment those counters.
        self.used.get(key).copied()
    }

    fn insert(&mut self, key: MeasurementKey, size: Size) {
        self.used.insert(key, size);
        self.used_order.push((key, size));
    }

    /// Publish content-derived facts. `preferred` is the natural border-box
    /// result before resolving the current containing block. Callers pass the
    /// final used value separately to [`Self::insert`].
    fn insert_intrinsic_bounds(
        &mut self,
        key: MeasurementKey,
        min_inline: f32,
        max_inline: f32,
        min_block: f32,
        max_block: Option<f32>,
        preferred: Size,
        first_baseline: Option<f32>,
        last_baseline: Option<f32>,
        aspect_ratio: Option<f32>,
    ) {
        let cache_key = Self::intrinsic_key(key);
        let metrics = crate::IntrinsicMetrics::new(
            min_inline,
            max_inline,
            min_block,
            max_block,
            crate::UsedSize::new(preferred.width, preferred.height),
        )
        .with_baselines(first_baseline, last_baseline)
        .with_aspect_ratio(aspect_ratio);
        let context = match key.parent_direction {
            1 => Some(crate::FormattingContextId::new(1)),
            2 => Some(crate::FormattingContextId::new(2)),
            _ => None,
        };
        self.seeded_intrinsic.insert(cache_key);
        self.cache.insert(cache_key, metrics, context);
        self.new_metrics.insert(cache_key, metrics);
        self.latest_intrinsic_keys.insert(key.id, cache_key);
    }

    /// Record facts for the retained cache without the in-pass LRU.
    ///
    /// Placement reads a baseline from that LRU only for `align-items:
    /// baseline`; every other alignment recomputes from the node. A plain
    /// leaf still publishes the same metrics the general insert would have
    /// written to `new_metrics`.
    fn remember_intrinsic_bounds(
        &mut self,
        key: MeasurementKey,
        min_inline: f32,
        max_inline: f32,
        min_block: f32,
        max_block: Option<f32>,
        preferred: Size,
        first_baseline: Option<f32>,
        last_baseline: Option<f32>,
        aspect_ratio: Option<f32>,
    ) {
        let cache_key = Self::intrinsic_key(key);
        let metrics = crate::IntrinsicMetrics::new(
            min_inline,
            max_inline,
            min_block,
            max_block,
            crate::UsedSize::new(preferred.width, preferred.height),
        )
        .with_baselines(first_baseline, last_baseline)
        .with_aspect_ratio(aspect_ratio);
        self.new_metrics.insert(cache_key, metrics);
    }

    fn seed_intrinsic(
        &mut self,
        key: crate::IntrinsicCacheKey,
        metrics: crate::IntrinsicMetrics,
        context: Option<crate::FormattingContextId>,
    ) {
        if !self.seeded_intrinsic.insert(key) {
            if let Some(id) = StableNodeId::new(key.content) {
                self.latest_intrinsic_keys.insert(id, key);
            }
            return;
        }
        self.cache.insert(key, metrics, context);
        if let Some(id) = StableNodeId::new(key.content) {
            self.latest_intrinsic_keys.insert(id, key);
        }
    }

    fn get_intrinsic(
        &mut self,
        key: MeasurementKey,
        context: Option<crate::FormattingContextId>,
    ) -> Option<crate::IntrinsicMetrics> {
        let key = Self::intrinsic_key(key);
        self.cache.get(&key, context)
    }

    fn record_full_subtree(&mut self) {
        self.extra_counters.full_subtrees = self.extra_counters.full_subtrees.saturating_add(1);
        self.extra_counters.intrinsic_measure_full_subtrees = self
            .extra_counters
            .intrinsic_measure_full_subtrees
            .saturating_add(1);
    }

    fn baseline(&mut self, id: StableNodeId, which: crate::Baseline) -> Option<f32> {
        let Some(key) = self.latest_intrinsic_keys.get(&id).copied() else {
            self.extra_counters.baseline_queries =
                self.extra_counters.baseline_queries.saturating_add(1);
            return None;
        };
        self.cache.baseline(&key, None, which)
    }

    fn counters(&self) -> crate::IntrinsicCacheCounters {
        let mut counters = self.cache.counters();
        let mut extra = self.extra_counters;
        extra.entries = counters.entries;
        extra.bytes = counters.bytes;
        counters.accumulate(extra);
        counters
    }

    fn intrinsic_key(key: MeasurementKey) -> crate::IntrinsicCacheKey {
        // Natural contributions for wrapping and percentage descendants can
        // depend on the containing block. Keep that relevant basis in the
        // constraint class and carry the remaining environment in the style
        // identity; the formatting-context name itself is still absent.
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        // Parent flow direction selects the *used-size* resolution path, but
        // it is not part of ordinary intrinsic facts. Aspect-ratio stretch
        // transfer is the deliberate exception: its used width depends on
        // whether the parent is a row, so keep that relevant dependency in
        // the identity while all other content still crosses contexts.
        if key.direction_sensitive
            || matches!(key.constraint, crate::ConstraintClass::AspectRatio { .. })
        {
            key.parent_direction.hash(&mut hasher);
        }
        key.viewport_width.hash(&mut hasher);
        key.viewport_height.hash(&mut hasher);
        key.parent_font.hash(&mut hasher);
        key.writing.hash(&mut hasher);
        key.containing_writing.hash(&mut hasher);
        crate::IntrinsicCacheKey::new(key.id.get(), hasher.finish(), key.constraint)
    }
}

impl Size {
    fn new(width: f32, height: f32) -> Self {
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
const ROOT_FONT_PX: f32 = nana_ui_core::type_scale::LINE;

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

/// After stretch (or a flexed used width), fill `height:auto` from the used width.
fn fill_auto_height_from_aspect_ratio(
    style: &nana_ui_core::LayoutStyle,
    size: &mut Size,
    percent_base: Option<f32>,
    fonts: FontSizeContext,
) {
    if !aspect_ratio_is_usable(style) || style.height.is_some() {
        return;
    }
    let padding = style.resolved_padding_against_fonts(percent_base, fonts);
    let border = style.resolved_border_edges();
    let chrome_w = padding.left + padding.right + border.left + border.right;
    let chrome_h = padding.top + padding.bottom + border.top + border.bottom;
    let mut content_w = Some((size.width - chrome_w).max(0.0));
    let mut content_h = None;
    style.apply_aspect_ratio_used(&mut content_w, &mut content_h);
    if let Some(h) = content_h {
        size.height = h + chrome_h;
    }
}

/// Whether the item's cross size is its own, so `align-items: stretch`
/// leaves it: any size it declares but `auto`, which stretches as unset
/// does (CSS). A content-sized keyword (`Shrink`, `fit-content`) keeps the
/// content's size.
fn cross_axis_is_definite(style: &nana_ui_core::LayoutStyle, direction: FlexDirection) -> bool {
    let declared = |spec: Option<LengthSpec>| spec.is_some_and(|spec| spec != LengthSpec::Auto);
    match direction {
        // Transferred block size from a definite used width + `aspect-ratio`.
        FlexDirection::Row => declared(style.height) || aspect_ratio_is_usable(style),
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
fn dedupe_plan_rows<T: Clone>(rows: &mut Vec<T>, child_of: impl Fn(&T) -> StableNodeId) {
    let mut seen = HashSet::with_capacity(rows.len());
    if rows.iter().all(|row| seen.insert(child_of(row))) {
        return;
    }
    seen.clear();
    let mut kept = Vec::with_capacity(rows.len());
    for row in rows.drain(..) {
        if seen.insert(child_of(&row)) {
            kept.push(row);
        }
    }
    *rows = kept;
}

fn bound_container_plan(plan: &mut ContainerPlan) {
    dedupe_plan_rows(&mut plan.entries.borrow_mut(), |entry| entry.child);
    dedupe_plan_rows(&mut plan.overlay, |entry| entry.child);
    if let Some(grid) = plan.grid.as_mut() {
        dedupe_plan_rows(&mut grid.items, |item| item.child);
    }
}

fn bound_measure_plan(plan: &mut MeasurePlan) {
    dedupe_plan_rows(&mut plan.entries, |entry| entry.child);
    if let Some(grid) = plan.grid.as_mut() {
        dedupe_plan_rows(&mut grid.items, |item| item.child);
    }
}

fn finite_extent(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

mod issue258;
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
