//! Cross-frame retained layout state and unified per-document intrinsic cache.

use super::*;

#[derive(Default)]
pub(super) struct NodeIntrinsics {
    pub(super) used: Option<RetainedIntrinsic>,
    pub(super) metrics: Option<ContentMetrics>,
}

/// Unified per-document retained intrinsic store backed by a single [`DenseNodeMap`].
///
/// Holds both retained used-size memos (up to two constraint variants per node)
/// and content-derived [`ContentMetrics`] with generation tracking.
#[derive(Default)]
pub(super) struct DocumentIntrinsics {
    pub(super) by_node: crate::store::DenseNodeMap<NodeIntrinsics>,
    pub(super) metrics_len: usize,
}

impl DocumentIntrinsics {
    pub(super) fn reserve_through(&mut self, highest: StableNodeId, expected: usize) {
        self.by_node.reserve_through(highest, expected);
    }

    #[cfg(test)]
    pub(super) fn node_count(&self) -> usize {
        self.by_node.len()
    }

    #[cfg(test)]
    pub(super) fn used_variant_count(&self) -> usize {
        self.by_node
            .values()
            .filter_map(|entry| entry.used.as_ref())
            .map(RetainedIntrinsic::len)
            .sum()
    }

    #[cfg(test)]
    pub(super) fn metrics_len(&self) -> usize {
        self.metrics_len
    }

    #[cfg(test)]
    pub(super) fn contains_metrics(&self, id: StableNodeId) -> bool {
        self.by_node
            .get(&id)
            .is_some_and(|entry| entry.metrics.is_some())
    }

    pub(super) fn clear(&mut self) {
        self.by_node.clear();
        self.metrics_len = 0;
    }

    pub(super) fn remove_node(&mut self, id: StableNodeId) {
        if let Some(removed) = self.by_node.remove(&id)
            && let Some(metrics) = removed.metrics
        {
            self.metrics_len -= metrics.variant_count();
        }
    }

    pub(super) fn remove_used(&mut self, id: StableNodeId) {
        if let Some(entry) = self.by_node.get_mut(&id) {
            entry.used = None;
            if entry.metrics.is_none() {
                self.by_node.remove(&id);
            }
        }
    }

    pub(super) fn remove_metrics(&mut self, id: StableNodeId) -> Option<u64> {
        let entry = self.by_node.get_mut(&id)?;
        let removed = entry.metrics.take()?;
        self.metrics_len -= removed.variant_count();
        let max_gen = removed.max_generation();
        if entry.used.is_none() {
            self.by_node.remove(&id);
        }
        Some(max_gen)
    }

    pub(super) fn used(&self, id: StableNodeId) -> Option<&RetainedIntrinsic> {
        self.by_node.get(&id)?.used.as_ref()
    }

    pub(super) fn record_used(&mut self, id: StableNodeId, key: StoredMeasurementKey, size: Size) {
        match self.by_node.get_mut(&id) {
            Some(entry) => match entry.used.as_mut() {
                Some(used) => used.insert(key, size),
                None => entry.used = Some(RetainedIntrinsic::new(key, size)),
            },
            None => {
                self.by_node.insert(
                    id,
                    NodeIntrinsics {
                        used: Some(RetainedIntrinsic::new(key, size)),
                        metrics: None,
                    },
                );
            }
        }
    }

    pub(super) fn metrics(
        &self,
        key: &crate::IntrinsicCacheKey,
    ) -> Option<crate::IntrinsicMetrics> {
        let id = StableNodeId::new(key.content)?;
        self.by_node.get(&id)?.metrics.as_ref()?.get(key)
    }

    pub(super) fn max_generation(&self, id: StableNodeId) -> Option<u64> {
        self.by_node
            .get(&id)?
            .metrics
            .as_ref()
            .map(ContentMetrics::max_generation)
    }

    pub(super) fn latest_baseline(&self, id: StableNodeId, first_only: bool) -> Option<f32> {
        self.by_node
            .get(&id)?
            .metrics
            .as_ref()?
            .latest_baseline(first_only)
    }

    pub(super) fn retain_live(&mut self, world: &UiWorld, universe: usize) {
        if self.by_node.len() <= universe.saturating_mul(2)
            && self.metrics_len <= universe.saturating_mul(2)
        {
            return;
        }
        let mut dropped_metrics = 0usize;
        self.by_node.retain(|id, entry| {
            let kept = world.contains(*id);
            if !kept && let Some(metrics) = &entry.metrics {
                dropped_metrics += metrics.variant_count();
            }
            kept
        });
        self.metrics_len -= dropped_metrics;
    }

    fn insert_metric(
        &mut self,
        id: StableNodeId,
        key: &crate::IntrinsicCacheKey,
        metrics: crate::IntrinsicMetrics,
    ) {
        let added = match self.by_node.get_mut(&id) {
            Some(entry) => match entry.metrics.as_mut() {
                Some(existing) => existing.insert(key, metrics),
                None => {
                    entry.metrics = Some(ContentMetrics::new(key, metrics));
                    1
                }
            },
            None => {
                self.by_node.insert(
                    id,
                    NodeIntrinsics {
                        used: None,
                        metrics: Some(ContentMetrics::new(key, metrics)),
                    },
                );
                1
            }
        };
        self.metrics_len += added;
    }

    fn evict_metrics_to(&mut self, limit: usize) -> usize {
        let excess = self.metrics_len.saturating_sub(limit);
        if excess == 0 {
            return 0;
        }
        let mut victims = Vec::new();
        let mut covered = 0usize;
        for (id, entry) in self.by_node.iter() {
            if covered >= excess {
                break;
            }
            if let Some(metrics) = &entry.metrics {
                covered += metrics.variant_count();
                victims.push(id);
            }
        }
        for id in victims {
            self.remove_metrics(id);
        }
        covered
    }

    pub(super) fn commit_facts(
        &mut self,
        facts: &crate::intrinsic_cache::PassIntrinsicFacts,
        max_entries: usize,
    ) -> (usize, usize) {
        let mut generation_bumps = 0usize;
        let cold = self.metrics_len == 0;
        facts.for_each_node_recorded(|id, iter| {
            let mut has_any = false;
            let mut changed = cold;
            if !cold {
                let existing = self.by_node.get(&id).and_then(|e| e.metrics.as_ref());
                for (key, incoming) in iter.clone() {
                    has_any = true;
                    if existing
                        .and_then(|m| m.get(&key))
                        .is_none_or(|old| intrinsic_facts_changed(old, incoming))
                    {
                        changed = true;
                        break;
                    }
                }
            } else {
                has_any = iter.clone().next().is_some();
            }
            if !has_any || !changed {
                return;
            }
            let prev = if cold {
                0
            } else {
                self.remove_metrics(id).unwrap_or(0)
            };
            let next_gen = prev.saturating_add(1).max(1);
            for (key, mut incoming) in iter {
                incoming.generation = next_gen;
                self.insert_metric(id, &key, incoming);
            }
            generation_bumps += 1;
        });
        let evicted = self.evict_metrics_to(max_entries);
        (generation_bumps, evicted)
    }
}

/// summed down from the top as [`UiWorld::layout_box`] sums them, and
/// summed once for every box below it.
pub(super) fn page_boxes(
    world: &UiWorld,
    ids: &[StableNodeId],
    local: impl Fn(StableNodeId) -> Option<(LayoutBox, crate::LayoutFrame)>,
) -> Vec<(StableNodeId, LayoutBox)> {
    let mut origins: crate::NodeMap<[f32; 2]> = crate::NodeMap::default();
    let mut chain: Vec<(StableNodeId, [f32; 2])> = Vec::new();
    let mut origin_of = |id: StableNodeId| -> [f32; 2] {
        if let Some(origin) = origins.get(&id) {
            return *origin;
        }
        chain.clear();
        let mut base = None;
        let mut cursor = Some(id);
        while let Some(node) = cursor {
            if world.is_document_node(node) {
                break;
            }
            if let Some(origin) = origins.get(&node) {
                base = Some(*origin);
                break;
            }
            let (layout, frame) = local(node).unwrap_or_else(|| {
                (
                    world.local_layout_box(node).unwrap_or_default(),
                    world.layout_frame(node).unwrap_or_default(),
                )
            });
            chain.push((node, [layout.x, layout.y]));
            cursor = match frame {
                crate::LayoutFrame::Parent => world.parent_id(node),
                crate::LayoutFrame::Block(block) => Some(block),
                crate::LayoutFrame::Page => None,
            };
        }
        let mut sum = base;
        while let Some((node, [x, y])) = chain.pop() {
            let next = match sum {
                Some([sx, sy]) => [sx + x, sy + y],
                None => [x, y],
            };
            origins.insert(node, next);
            sum = Some(next);
        }
        sum.unwrap_or([0.0, 0.0])
    };
    ids.iter()
        .map(|&id| {
            let [x, y] = origin_of(id);
            let layout = local(id)
                .map(|(layout, _)| layout)
                .or_else(|| world.local_layout_box(id))
                .unwrap_or_default();
            (id, LayoutBox { x, y, ..layout })
        })
        .collect()
}

/// Cross-frame layout memo for scoped relayout: last published boxes, used-size
/// resolutions, and content-derived intrinsic facts. Each document owns its
/// entries, so a full pass cannot invalidate another window's layout.
#[derive(Default)]
pub struct RetainedLayoutCache {
    pub(super) documents: HashMap<DocumentId, DocumentLayoutCache>,
    /// What is left of the frame's slice budget for the next pass; taken by
    /// that pass. See `recursion::LayoutSliceBudget`.
    pub(super) slice: Option<recursion::SliceLimit>,
}

impl RetainedLayoutCache {
    /// What is left of the frame's slice budget, for the next pass only.
    pub(crate) fn set_slice_limit(&mut self, limit: Option<recursion::SliceLimit>) {
        self.slice = limit;
    }

    /// Whether a sliced pass left boundaries of `document` unplaced, which
    /// the next pass places.
    pub(crate) fn layout_pending(&self, document: DocumentId) -> bool {
        self.documents
            .get(&document)
            .is_some_and(|cache| !cache.deferred.is_empty())
    }

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
            cache.inline_objects.remove(&id);
            cache.intrinsics.remove_node(id);
            cache.boxes.remove(&id);
            cache.frames.remove(&id);
            cache.placements.remove(&id);
            cache.used_padding.remove(&id);
            cache.omitted_boxes.remove(&id);
            cache.far_start.remove(&id);
            cache.contexts.remove(&id);
            cache.container_plans.remove(&id);
            cache.remote_overlays.remove(&id);
            cache.measure_plans.remove(&id);
            cache.envelopes.remove(&id);
            cache.line_solves.remove(&id);
            cache.applied_adjustments.remove(&id);
            cache.line_boxes.remove(&id);
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
        self.documents.get(&document)?.intrinsics.max_generation(id)
    }

    /// The first or last baseline `id` published, the one an atom sits on its line by.
    pub(crate) fn intrinsic_baseline(
        &self,
        document: DocumentId,
        id: StableNodeId,
        first_only: bool,
    ) -> Option<f32> {
        self.documents
            .get(&document)?
            .intrinsics
            .latest_baseline(id, first_only)
    }
    /// What the box the last pass gave `id` is measured from.
    pub(crate) fn frame(&self, document: DocumentId, id: StableNodeId) -> crate::LayoutFrame {
        self.documents
            .get(&document)
            .and_then(|cache| cache.frames.get(&id).copied())
            .unwrap_or_default()
    }

    pub(crate) fn take_inline_objects(
        &mut self,
        document: DocumentId,
    ) -> HashMap<StableNodeId, Arc<[nana_text::InlineObject]>> {
        self.documents
            .get_mut(&document)
            .map_or_else(HashMap::new, |cache| {
                std::mem::take(&mut cache.inline_objects)
            })
    }

    pub(crate) fn used_padding(
        &self,
        document: DocumentId,
        id: StableNodeId,
    ) -> Option<nana_ui_core::PaddingSpec> {
        self.documents.get(&document)?.used_padding_of(id)
    }
}

/// Two used-size variants per node allow measure/place reuse without
/// accumulating a new entry for every pixel of an interactive resize. The
/// content-derived facts live in the generation-aware authority beside it.
pub(super) struct RetainedIntrinsic {
    pub(super) first: (StoredMeasurementKey, Size),
    pub(super) second: Option<Box<(StoredMeasurementKey, Size)>>,
}

impl RetainedIntrinsic {
    pub(super) fn new(key: StoredMeasurementKey, size: Size) -> Self {
        Self {
            first: (key, size),
            second: None,
        }
    }

    pub(super) fn get(&self, key: MeasurementKey) -> Option<Size> {
        let key = key.stored();
        if self.first.0 == key {
            return Some(self.first.1);
        }
        self.second
            .as_deref()
            .filter(|(held, _)| *held == key)
            .map(|(_, size)| *size)
    }

    pub(super) fn insert(&mut self, key: StoredMeasurementKey, size: Size) {
        if self.first.0 == key {
            self.first.1 = size;
            return;
        }
        match self.second.as_deref_mut() {
            Some(second) => *second = self.first,
            None => self.second = Some(Box::new(self.first)),
        }
        self.first = (key, size);
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        1 + usize::from(self.second.is_some())
    }
}

#[derive(Default)]
pub(super) struct DocumentLayoutCache {
    pub(super) inline_objects: HashMap<StableNodeId, Arc<[nana_text::InlineObject]>>,
    pub(super) intrinsics: DocumentIntrinsics,
    pub(super) intrinsic_counters: crate::IntrinsicCacheCounters,
    /// Each box from its frame's origin (see [`crate::LayoutFrame`]).
    pub(super) boxes: crate::store::DenseNodeMap<LayoutBox>,
    /// The frame of each box not measured from its parent's origin.
    pub(super) frames: crate::NodeMap<crate::LayoutFrame>,
    pub(super) materialized_inputs: usize,
    pub(super) placements: crate::store::DenseNodeMap<(Point, Size, f32)>,
    pub(crate) used_padding: crate::store::DenseNodeMap<nana_ui_core::PaddingSpec>,
    pub(super) omitted_boxes: crate::NodeSet,
    /// Per container, the page axes placement starts at the far end.
    pub(super) far_start: crate::store::DenseNodeMap<[bool; 2]>,
    /// Per container, the formatting context its last placement ran.
    pub(super) contexts: crate::store::DenseNodeMap<crate::FormattingContextKind>,
    /// Cached in-flow child placement per container. See [`ContainerPlan`].
    pub(super) container_plans: HashMap<StableNodeId, ContainerPlan>,
    /// The containers whose plan holds an absolute child hung from an
    /// ancestor or from the initial containing block: a pass that does not
    /// place the container checks those children against their block.
    pub(super) remote_overlays: crate::NodeSet,
    /// Cached intrinsic measurement per content-sized container. See
    /// [`MeasurePlan`].
    pub(super) measure_plans: crate::store::DenseNodeMap<MeasurePlanSlots>,
    /// Dynamic Layout envelopes, by box (Issue #213).
    pub(super) envelopes: HashMap<StableNodeId, dynamic::RetainedEnvelope>,
    /// Where each overflowing line's solve stopped, by container.
    pub(super) line_solves: HashMap<StableNodeId, dynamic::RetainedLineSolve>,
    /// What each box a line shrank resolved inside itself.
    pub(super) applied_adjustments: HashMap<StableNodeId, dynamic::AppliedAdjustment>,
    /// Which boxes hold an in-flow line box ([`UiWorld::holds_line_box_in`]),
    /// kept across passes. The answer reads only a box and what is inside
    /// it, so a pass drops it for its affected nodes and their ancestors and
    /// keeps the rest: asking it of the document's root after one edit would
    /// otherwise walk the whole tree again.
    pub(super) line_boxes: crate::NodeMap<bool>,
    pub(super) frontier_stats: LayoutFrontierStats,
    /// The frontier of the most recent pass, so diagnostics can say why a
    /// node was laid out again. Empty after a full pass.
    pub(super) last_frontier: LayoutFrontier,
    pub(super) execution_stats: LayoutExecutionStats,
    /// Last two full-layout results for this document, keyed by viewport and
    /// layout-input epoch. `clear` keeps them: a full pass is what consults
    /// them, and clearing first would drop the hit.
    pub(super) full_snapshots: [Option<FullLayoutSnapshot>; 2],
    /// The viewport the cache was last laid out against. A pass against
    /// another one plants the resize's seeds itself.
    pub(super) viewport: Option<LayoutViewport>,
    /// Boundaries a sliced pass left unplaced, by root: nothing retained is
    /// placed under them, and the next pass places them (see
    /// `recursion::LayoutSliceBudget`).
    pub(super) deferred: crate::NodeMap<recursion::DeferredBoundary>,
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

pub(super) struct FullLayoutSnapshot {
    pub(super) viewport_width: u32,
    pub(super) viewport_height: u32,
    pub(super) epoch: u64,
    pub(super) emitted: Vec<(StableNodeId, LayoutBox)>,
    pub(super) used_padding: crate::store::DenseNodeMap<nana_ui_core::PaddingSpec>,
    pub(super) omitted_boxes: crate::NodeSet,
    pub(super) far_start: crate::store::DenseNodeMap<[bool; 2]>,
    pub(super) contexts: crate::store::DenseNodeMap<crate::FormattingContextKind>,
    pub(super) frames: crate::NodeMap<crate::LayoutFrame>,
}

#[derive(Clone, Copy)]
pub(super) struct PackedOptF32(f32);

impl PackedOptF32 {
    pub(super) const NONE: Self = Self(f32::NAN);

    #[inline]
    pub(super) fn from_opt(value: Option<f32>) -> Self {
        match value {
            Some(v) if !v.is_nan() => Self(v),
            _ => Self::NONE,
        }
    }

    #[inline]
    pub(super) fn get(self) -> Option<f32> {
        (!self.0.is_nan()).then_some(self.0)
    }

    #[inline]
    pub(super) fn is_some(self) -> bool {
        !self.0.is_nan()
    }
}

#[derive(Clone, Copy)]
pub(super) struct PackedOptSize(Size);

impl PackedOptSize {
    pub(super) const NONE: Self = Self(Size {
        width: f32::NAN,
        height: 0.0,
    });

    #[inline]
    pub(super) fn from_size(size: Size) -> Self {
        debug_assert!(!size.width.is_nan());
        Self(size)
    }

    #[inline]
    pub(super) fn from_opt(value: Option<Size>) -> Self {
        match value {
            Some(s) if !s.width.is_nan() => Self(s),
            _ => Self::NONE,
        }
    }

    #[inline]
    pub(super) fn get(self) -> Option<Size> {
        (!self.0.width.is_nan()).then_some(self.0)
    }

    #[inline]
    pub(super) fn is_none(self) -> bool {
        self.0.width.is_nan()
    }

    #[inline]
    pub(super) fn is_some(self) -> bool {
        !self.0.width.is_nan()
    }
}

#[derive(Clone, Copy)]
pub(super) struct StoredIntrinsicMetricVariant {
    pub(super) style: u64,
    pub(super) generation: u64,
    pub(super) constraint_a: u32,
    pub(super) constraint_b: u32,
    pub(super) min_inline: f32,
    pub(super) max_inline: f32,
    pub(super) min_block: f32,
    pub(super) preferred_inline: f32,
    pub(super) preferred_block: f32,
    pub(super) max_block: PackedOptF32,
    pub(super) first_baseline: PackedOptF32,
    pub(super) last_baseline: PackedOptF32,
    pub(super) aspect_ratio: PackedOptF32,
    pub(super) constraint_tag: u8,
    pub(super) truncatable: bool,
}

impl StoredIntrinsicMetricVariant {
    pub(super) fn from_key_metrics(
        key: &crate::IntrinsicCacheKey,
        metrics: crate::IntrinsicMetrics,
    ) -> Self {
        let (constraint_tag, (constraint_a, constraint_b)) = key.constraints.canonical();
        Self {
            style: key.style,
            generation: metrics.generation,
            constraint_a,
            constraint_b,
            min_inline: metrics.min_inline,
            max_inline: metrics.max_inline,
            min_block: metrics.min_block,
            preferred_inline: metrics.preferred.inline,
            preferred_block: metrics.preferred.block,
            max_block: PackedOptF32::from_opt(metrics.max_block),
            first_baseline: PackedOptF32::from_opt(metrics.first_baseline),
            last_baseline: PackedOptF32::from_opt(metrics.last_baseline),
            aspect_ratio: PackedOptF32::from_opt(metrics.aspect_ratio),
            constraint_tag,
            truncatable: metrics.truncatable,
        }
    }

    pub(super) fn set_metrics(&mut self, metrics: crate::IntrinsicMetrics) {
        self.generation = metrics.generation;
        self.min_inline = metrics.min_inline;
        self.max_inline = metrics.max_inline;
        self.min_block = metrics.min_block;
        self.preferred_inline = metrics.preferred.inline;
        self.preferred_block = metrics.preferred.block;
        self.max_block = PackedOptF32::from_opt(metrics.max_block);
        self.first_baseline = PackedOptF32::from_opt(metrics.first_baseline);
        self.last_baseline = PackedOptF32::from_opt(metrics.last_baseline);
        self.aspect_ratio = PackedOptF32::from_opt(metrics.aspect_ratio);
        self.truncatable = metrics.truncatable;
    }

    pub(super) fn metrics(self) -> crate::IntrinsicMetrics {
        crate::IntrinsicMetrics {
            min_inline: self.min_inline,
            max_inline: self.max_inline,
            min_block: self.min_block,
            max_block: self.max_block.get(),
            preferred: crate::UsedSize {
                inline: self.preferred_inline,
                block: self.preferred_block,
            },
            first_baseline: self.first_baseline.get(),
            last_baseline: self.last_baseline.get(),
            aspect_ratio: self.aspect_ratio.get(),
            truncatable: self.truncatable,
            generation: self.generation,
        }
    }

    pub(super) fn matches(self, key: &crate::IntrinsicCacheKey) -> bool {
        self.style == key.style
            && (self.constraint_tag, (self.constraint_a, self.constraint_b))
                == key.constraints.canonical()
    }
}

pub(super) struct ContentMetrics {
    pub(super) first: StoredIntrinsicMetricVariant,
    #[allow(clippy::box_collection)] // Keeps `Option<ContentMetrics>` <= 72 bytes.
    pub(super) rest: Option<Box<Vec<StoredIntrinsicMetricVariant>>>,
}

const _: () = assert!(std::mem::size_of::<Option<ContentMetrics>>() <= 72);

impl ContentMetrics {
    pub(super) fn new(key: &crate::IntrinsicCacheKey, metrics: crate::IntrinsicMetrics) -> Self {
        Self {
            first: StoredIntrinsicMetricVariant::from_key_metrics(key, metrics),
            rest: None,
        }
    }

    pub(super) fn variant_count(&self) -> usize {
        1 + self.rest.as_deref().map_or(0, Vec::len)
    }

    pub(super) fn get(&self, key: &crate::IntrinsicCacheKey) -> Option<crate::IntrinsicMetrics> {
        if self.first.matches(key) {
            return Some(self.first.metrics());
        }
        self.rest
            .as_deref()?
            .iter()
            .find(|variant| variant.matches(key))
            .map(|variant| variant.metrics())
    }

    pub(super) fn insert(
        &mut self,
        key: &crate::IntrinsicCacheKey,
        metrics: crate::IntrinsicMetrics,
    ) -> usize {
        if self.first.matches(key) {
            self.first.set_metrics(metrics);
            return 0;
        }
        let rest = self.rest.get_or_insert_with(|| Box::new(Vec::new()));
        if let Some(slot) = rest.iter_mut().find(|variant| variant.matches(key)) {
            slot.set_metrics(metrics);
            return 0;
        }
        // Most contents hold one or two variants: grow by one, not to
        // the four a first push reserves.
        rest.reserve_exact(1);
        rest.push(StoredIntrinsicMetricVariant::from_key_metrics(key, metrics));
        1
    }

    pub(super) fn max_generation(&self) -> u64 {
        self.rest
            .as_deref()
            .into_iter()
            .flatten()
            .fold(self.first.generation, |acc, variant| {
                acc.max(variant.generation)
            })
    }

    pub(super) fn latest_baseline(&self, first_only: bool) -> Option<f32> {
        let latest =
            self.rest
                .as_deref()
                .into_iter()
                .flatten()
                .fold(self.first, |best, &variant| {
                    if variant.generation >= best.generation {
                        variant
                    } else {
                        best
                    }
                });
        if first_only {
            latest.first_baseline.get().or(latest.last_baseline.get())
        } else {
            latest.last_baseline.get().or(latest.first_baseline.get())
        }
    }
}

pub(super) fn intrinsic_facts_changed(
    mut existing: crate::IntrinsicMetrics,
    mut incoming: crate::IntrinsicMetrics,
) -> bool {
    existing.generation = 0;
    incoming.generation = 0;
    existing != incoming
}

impl DocumentLayoutCache {
    pub(super) fn used_padding_of(&self, id: StableNodeId) -> Option<nana_ui_core::PaddingSpec> {
        if !self.boxes.contains_key(&id) || self.omitted_boxes.contains(&id) {
            return None;
        }
        Some(self.used_padding.get(&id).copied().unwrap_or_default())
    }

    pub(super) fn record_intrinsic_counters(&mut self, counters: crate::IntrinsicCacheCounters) {
        self.intrinsic_counters.accumulate(counters);
    }

    pub(super) fn retain_intrinsic_metrics(
        &mut self,
        facts: &crate::intrinsic_cache::PassIntrinsicFacts,
        max_entries: usize,
    ) {
        let (bumps, evicted) = self.intrinsics.commit_facts(facts, max_entries);
        self.intrinsic_counters.generation_bumps = self
            .intrinsic_counters
            .generation_bumps
            .saturating_add(bumps);
        self.intrinsic_counters.evictions =
            self.intrinsic_counters.evictions.saturating_add(evicted);
    }

    /// Direct participants and tracks stored on this document's context plans.
    ///
    /// Mutation history does not add entries: a later pass replaces the plan
    /// for the same container. See [`bound_container_plan`].
    #[cfg(test)]
    pub(super) fn retained_plan_entries(&self) -> usize {
        let mut count = 0usize;
        for plan in self.container_plans.values() {
            count = count.saturating_add(plan.entries.len());
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

    /// Every retained box on the page.
    #[cfg(test)]
    pub(super) fn page_boxes(&self, world: &UiWorld) -> crate::store::DenseNodeMap<LayoutBox> {
        let ids: Vec<_> = self.boxes.iter().map(|(id, _)| id).collect();
        page_boxes(world, &ids, |id| {
            Some((
                self.boxes.get(&id).copied()?,
                self.frames.get(&id).copied().unwrap_or_default(),
            ))
        })
        .into_iter()
        .collect()
    }

    #[cfg(test)]
    pub(super) fn footprint(&self) -> RetainedLayoutFootprint {
        RetainedLayoutFootprint {
            boxes: self.boxes.len(),
            placements: self.placements.len(),
            intrinsics: self.intrinsics.used_variant_count(),
            intrinsic_metrics: self.intrinsics.metrics_len(),
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

    pub(super) fn clear(&mut self) {
        self.intrinsics.clear();
        self.intrinsic_counters = crate::IntrinsicCacheCounters::default();
        self.placements.clear();
        self.boxes.clear();
        self.frames.clear();
        self.used_padding.clear();
        self.omitted_boxes.clear();
        self.far_start.clear();
        self.contexts.clear();
        self.container_plans.clear();
        self.remote_overlays.clear();
        self.measure_plans.clear();
        self.envelopes.clear();
        self.line_solves.clear();
        self.applied_adjustments.clear();
        self.line_boxes.clear();
        self.frontier_stats = LayoutFrontierStats::default();
        self.last_frontier = LayoutFrontier::default();
        self.execution_stats = LayoutExecutionStats::default();
        self.materialized_inputs = 0;
        self.deferred.clear();
    }
}
