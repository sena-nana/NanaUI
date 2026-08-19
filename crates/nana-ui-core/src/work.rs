//! Algorithm-level work counters and named frame stages for the Performance
//! Contract (Issue #8).
//!
//! Runtime fills these from dirty system work. GPU upload bytes are omitted
//! rather than estimated: this crate does not observe renderer uploads.

/// Per-frame algorithm counts. Timing stays on the Runtime profiler; these
/// fields are the stable CI signals.
///
/// Runtime dirty-bit mapping (Issue #8 §6.2). PAINT stays folded into RENDER.
/// STATE and TRANSFORM are independent Runtime mask bits:
///
/// - STATE → Runtime `SystemWork::state` (hover/press/focus/`SetInteraction`).
///   STYLE is added only when interaction paints need resolving.
/// - STYLE → `style_processed`
/// - TEXT → `text_shaped` (scheduled text nodes) plus `text_shaped_runs` /
///   `text_layout_cache_*` recorded on the shaping hot path
/// - LAYOUT → `layout_nodes`
/// - TRANSFORM → Runtime `SystemWork::transform` (`PaintTransform`). INPUT and
///   RENDER are added because hit-test and extract consume the matrix; LAYOUT
///   is not. STYLE is not set for transform-only `SetStyle`.
/// - INPUT → `hit_test_candidates`
/// - FOCUS_IME is tracked on Runtime `SystemWork::focus_ime`, not a dedicated
///   counter field
/// - RENDER → `render_nodes_extracted` / `render_nodes_changed`. **PAINT is folded
///   into RENDER**; paint-only mutations (hover color, opacity) schedule RENDER
///   without LAYOUT
/// - ACCESSIBILITY → `accessibility_nodes_updated`
///
/// STATE and TRANSFORM follow FOCUS_IME: they are scheduled on Runtime
/// `SystemWork`, not dedicated `WorkCounters` fields.
///
/// `input_targets` counts live pointer hover/press/capture plus focus.
///
/// `allocations` / `allocated_bytes` are **CPU hot-path** Vec/slot/string
/// clones Runtime can observe (drain lists, layout input children, text-shape
/// temps). They are not a process-wide malloc hook.
///
/// `text_layout_cache_*` come from Runtime `TextLayoutCache` lookup/insert.
/// `glyph_cache_*` are `None` until a shaping pass consults Runtime
/// `GlyphCache` (hosts without a glyph backend never do). `cache_eviction`
/// is `Some` after a shaping pass that consulted the layout cache (including
/// 0). GPU upload / draw-batch are `None` until a renderer that actually
/// encodes/submits records them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkCounters {
    pub entities_total: usize,
    pub entities_changed: usize,
    pub entities_spawned: usize,
    pub entities_despawned: usize,
    pub style_processed: usize,
    pub text_shaped: usize,
    pub layout_nodes: usize,
    pub hit_test_candidates: usize,
    /// Unique live pointer hover, press, capture, and focus nodes this drain.
    pub input_targets: usize,
    pub accessibility_nodes_updated: usize,
    /// Nodes whose render extraction was invalidated this drain.
    pub render_nodes_changed: usize,
    /// Nodes actually produced by extract. Zero on last-work snapshots until
    /// Runtime `record_extract` runs.
    pub render_nodes_extracted: usize,
    /// Theme-resolved text spans on extracted nodes. Zero until extract is
    /// recorded; not a GPU batch count.
    pub extracted_text_spans: usize,
    /// Observed CPU hot-path heap events this drain/frame (Issue #8 §5 / §7).
    pub allocations: usize,
    /// Payload bytes of those observed events. Not allocator slack, not VRAM.
    pub allocated_bytes: usize,
    /// `TextShaper::shape` invocations (Issue #8 §3.5 shaping calls/frame).
    pub text_shaped_runs: usize,
    /// `TextLayoutCache::lookup` hits. Not “metrics left unchanged”.
    pub text_layout_cache_hits: usize,
    /// `TextLayoutCache::insert` after a lookup miss.
    pub text_layout_cache_misses: usize,
    /// Shape calls that requested wrapping (`TextShapeConstraints.wrap`).
    pub text_wrap_layouts: usize,
    /// `GlyphCache::lookup` hits. `None` until a glyph backend consults the
    /// cache this pass — omitted, never a fake 0.
    pub glyph_cache_hits: Option<usize>,
    /// `GlyphCache::insert` after a lookup miss. `None` until consulted.
    pub glyph_cache_misses: Option<usize>,
    /// `TextLayoutCache` FIFO evictions this shaping pass. `None` until the
    /// cache is consulted. Glyph FIFO trim is not folded into this field.
    pub cache_eviction: Option<usize>,
    /// Coalesced GPU batches rebuilt this frame. `None` until a host encodes.
    pub batch_rebuilds: Option<usize>,
    /// GPU batches issued this frame. `None` until a host encodes.
    pub draw_batches: Option<usize>,
    /// `draw` / `draw_indexed` invocations this frame. `None` until a host encodes.
    pub draw_calls: Option<usize>,
    /// Observed `queue.write_buffer` bytes this frame. `None` until a host
    /// encodes/submits. Not an estimate; missing stays omitted, never a fake 0.
    pub gpu_upload_bytes: Option<usize>,
    /// GPU buffer reallocations observed this frame. `None` until a host encodes.
    pub gpu_buffer_reallocations: Option<usize>,
}

/// GPU work a renderer observed while encoding or submitting a real frame.
///
/// Recording this (including zeros) means the host ran encode/submit/upload.
/// Do not construct it to invent a quiet CPU-only drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuWorkObservation {
    pub batch_rebuilds: usize,
    pub draw_batches: usize,
    pub draw_calls: usize,
    pub gpu_upload_bytes: usize,
    pub gpu_buffer_reallocations: usize,
}

impl GpuWorkObservation {
    pub fn record_upload(&mut self, bytes: usize) {
        self.gpu_upload_bytes = self.gpu_upload_bytes.saturating_add(bytes);
    }

    pub fn record_realloc(&mut self) {
        self.gpu_buffer_reallocations = self.gpu_buffer_reallocations.saturating_add(1);
    }

    pub fn record_batch_rebuild(&mut self) {
        self.batch_rebuilds = self.batch_rebuilds.saturating_add(1);
    }

    pub fn record_draw_batch(&mut self) {
        self.draw_batches = self.draw_batches.saturating_add(1);
    }

    pub fn record_draw_call(&mut self) {
        self.draw_calls = self.draw_calls.saturating_add(1);
    }
}

impl WorkCounters {
    /// Fold another drain into this frame snapshot. `entities_total` is the
    /// latest live count; extract fields are added only when the other snapshot
    /// recorded them.
    pub fn accumulate(&mut self, other: Self) {
        self.entities_total = other.entities_total;
        self.entities_changed = self.entities_changed.saturating_add(other.entities_changed);
        self.entities_spawned = self.entities_spawned.saturating_add(other.entities_spawned);
        self.entities_despawned = self
            .entities_despawned
            .saturating_add(other.entities_despawned);
        self.style_processed = self.style_processed.saturating_add(other.style_processed);
        self.text_shaped = self.text_shaped.saturating_add(other.text_shaped);
        self.layout_nodes = self.layout_nodes.saturating_add(other.layout_nodes);
        self.hit_test_candidates = self
            .hit_test_candidates
            .saturating_add(other.hit_test_candidates);
        self.input_targets = self.input_targets.saturating_add(other.input_targets);
        self.accessibility_nodes_updated = self
            .accessibility_nodes_updated
            .saturating_add(other.accessibility_nodes_updated);
        self.render_nodes_changed = self
            .render_nodes_changed
            .saturating_add(other.render_nodes_changed);
        self.render_nodes_extracted = self
            .render_nodes_extracted
            .saturating_add(other.render_nodes_extracted);
        self.extracted_text_spans = self
            .extracted_text_spans
            .saturating_add(other.extracted_text_spans);
        self.allocations = self.allocations.saturating_add(other.allocations);
        self.allocated_bytes = self.allocated_bytes.saturating_add(other.allocated_bytes);
        self.text_shaped_runs = self.text_shaped_runs.saturating_add(other.text_shaped_runs);
        self.text_layout_cache_hits = self
            .text_layout_cache_hits
            .saturating_add(other.text_layout_cache_hits);
        self.text_layout_cache_misses = self
            .text_layout_cache_misses
            .saturating_add(other.text_layout_cache_misses);
        self.text_wrap_layouts = self
            .text_wrap_layouts
            .saturating_add(other.text_wrap_layouts);
        fold_optional_count(&mut self.glyph_cache_hits, other.glyph_cache_hits);
        fold_optional_count(&mut self.glyph_cache_misses, other.glyph_cache_misses);
        fold_optional_count(&mut self.cache_eviction, other.cache_eviction);
        fold_optional_count(&mut self.batch_rebuilds, other.batch_rebuilds);
        fold_optional_count(&mut self.draw_batches, other.draw_batches);
        fold_optional_count(&mut self.draw_calls, other.draw_calls);
        fold_optional_count(&mut self.gpu_upload_bytes, other.gpu_upload_bytes);
        fold_optional_count(
            &mut self.gpu_buffer_reallocations,
            other.gpu_buffer_reallocations,
        );
    }

    /// Record observed CPU hot-path heap events. Zero-count/zero-byte calls
    /// are ignored so empty `Vec::new()` is not a fake allocation.
    pub fn record_hot_path_allocation(&mut self, count: usize, bytes: usize) {
        if count == 0 && bytes == 0 {
            return;
        }
        self.allocations = self.allocations.saturating_add(count);
        self.allocated_bytes = self.allocated_bytes.saturating_add(bytes);
    }

    /// Record shaping-path stats after Runtime calls the host `TextShaper`.
    pub fn record_text_shape(
        &mut self,
        shaped_runs: usize,
        cache_hits: usize,
        cache_misses: usize,
        wrap_layouts: usize,
    ) {
        self.text_shaped_runs = self.text_shaped_runs.saturating_add(shaped_runs);
        self.text_layout_cache_hits = self.text_layout_cache_hits.saturating_add(cache_hits);
        self.text_layout_cache_misses = self.text_layout_cache_misses.saturating_add(cache_misses);
        self.text_wrap_layouts = self.text_wrap_layouts.saturating_add(wrap_layouts);
    }

    /// Record `TextLayoutCache` FIFO evictions. Does not invent glyph evictions.
    pub fn record_cache_eviction(&mut self, evictions: usize) {
        self.cache_eviction = Some(self.cache_eviction.unwrap_or(0).saturating_add(evictions));
    }

    /// Record `GlyphCache` lookup/insert from a shaping pass that consulted it.
    /// Zeros are stored as `Some(0)` only because that pass ran, not because a
    /// non-glyph host guessed quiet glyph work.
    pub fn record_glyph_cache(&mut self, hits: usize, misses: usize) {
        self.glyph_cache_hits = Some(self.glyph_cache_hits.unwrap_or(0).saturating_add(hits));
        self.glyph_cache_misses = Some(self.glyph_cache_misses.unwrap_or(0).saturating_add(misses));
    }

    /// Fold GPU work observed on a real encode/submit path. Zeros are stored as
    /// `Some(0)` only because the host ran that path, not because a CPU drain
    /// guessed quiet GPU work.
    pub fn record_gpu_work(&mut self, observed: GpuWorkObservation) {
        self.batch_rebuilds = Some(
            self.batch_rebuilds
                .unwrap_or(0)
                .saturating_add(observed.batch_rebuilds),
        );
        self.draw_batches = Some(
            self.draw_batches
                .unwrap_or(0)
                .saturating_add(observed.draw_batches),
        );
        self.draw_calls = Some(
            self.draw_calls
                .unwrap_or(0)
                .saturating_add(observed.draw_calls),
        );
        self.gpu_upload_bytes = Some(
            self.gpu_upload_bytes
                .unwrap_or(0)
                .saturating_add(observed.gpu_upload_bytes),
        );
        self.gpu_buffer_reallocations = Some(
            self.gpu_buffer_reallocations
                .unwrap_or(0)
                .saturating_add(observed.gpu_buffer_reallocations),
        );
    }
}

fn fold_optional_count(slot: &mut Option<usize>, other: Option<usize>) {
    *slot = match (*slot, other) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0).saturating_add(right.unwrap_or(0))),
    };
}

/// Named CPU stages from Issue #8 §4. Runtime times the stages it owns;
/// Batch / GPU Upload / Encode / Submit stay `runtime_unsupported` until a
/// GPU host that actually encodes/submits times them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameStage {
    Input,
    Reconcile,
    Style,
    TextShape,
    Layout,
    HitTest,
    Accessibility,
    Animation,
    Extract,
    Batch,
    GpuUpload,
    Encode,
    Submit,
}

impl FrameStage {
    pub const ALL: [Self; 13] = [
        Self::Input,
        Self::Reconcile,
        Self::Style,
        Self::TextShape,
        Self::Layout,
        Self::HitTest,
        Self::Accessibility,
        Self::Animation,
        Self::Extract,
        Self::Batch,
        Self::GpuUpload,
        Self::Encode,
        Self::Submit,
    ];

    /// Stages the retained Runtime does not own. A profiler should report these
    /// as unsupported with zero duration rather than pretending they ran.
    pub const fn runtime_unsupported(self) -> bool {
        self.gpu_host_owned()
    }

    /// Stages a Scene/WGPU host owns: Batch, GPU Upload, Encode, Submit.
    /// Measurable only when that host actually encodes/submits.
    pub const fn gpu_host_owned(self) -> bool {
        matches!(
            self,
            Self::Batch | Self::GpuUpload | Self::Encode | Self::Submit
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_counters_are_zero_and_gpu_stages_are_explicitly_unsupported() {
        assert_eq!(WorkCounters::default(), WorkCounters::default());
        assert_eq!(WorkCounters::default().allocations, 0);
        assert_eq!(WorkCounters::default().allocated_bytes, 0);
        assert_eq!(WorkCounters::default().text_shaped_runs, 0);
        assert_eq!(WorkCounters::default().text_layout_cache_hits, 0);
        assert_eq!(WorkCounters::default().text_layout_cache_misses, 0);
        assert_eq!(WorkCounters::default().text_wrap_layouts, 0);
        assert_eq!(WorkCounters::default().glyph_cache_hits, None);
        assert_eq!(WorkCounters::default().glyph_cache_misses, None);
        assert_eq!(WorkCounters::default().cache_eviction, None);
        assert_eq!(WorkCounters::default().batch_rebuilds, None);
        assert_eq!(WorkCounters::default().draw_batches, None);
        assert_eq!(WorkCounters::default().draw_calls, None);
        assert_eq!(WorkCounters::default().gpu_upload_bytes, None);
        assert_eq!(WorkCounters::default().gpu_buffer_reallocations, None);
        assert!(!FrameStage::Style.runtime_unsupported());
        assert!(FrameStage::GpuUpload.runtime_unsupported());
        assert!(FrameStage::GpuUpload.gpu_host_owned());
        assert!(!FrameStage::Extract.gpu_host_owned());
        assert_eq!(FrameStage::ALL.len(), 13);
    }

    #[test]
    fn accumulate_folds_hot_path_allocation_and_text_shape_fields() {
        let mut total = WorkCounters {
            allocations: 1,
            allocated_bytes: 32,
            text_shaped_runs: 2,
            text_layout_cache_hits: 1,
            text_layout_cache_misses: 1,
            text_wrap_layouts: 1,
            cache_eviction: Some(1),
            input_targets: 2,
            render_nodes_changed: 3,
            ..WorkCounters::default()
        };
        total.accumulate(WorkCounters {
            entities_total: 9,
            allocations: 3,
            allocated_bytes: 16,
            text_shaped_runs: 1,
            text_layout_cache_hits: 2,
            text_layout_cache_misses: 0,
            text_wrap_layouts: 1,
            cache_eviction: Some(2),
            input_targets: 1,
            render_nodes_changed: 4,
            ..WorkCounters::default()
        });
        assert_eq!(total.entities_total, 9);
        assert_eq!(total.allocations, 4);
        assert_eq!(total.allocated_bytes, 48);
        assert_eq!(total.text_shaped_runs, 3);
        assert_eq!(total.text_layout_cache_hits, 3);
        assert_eq!(total.text_layout_cache_misses, 1);
        assert_eq!(total.text_wrap_layouts, 2);
        assert_eq!(total.cache_eviction, Some(3));
        assert_eq!(total.glyph_cache_hits, None);
        assert_eq!(total.gpu_upload_bytes, None);
        assert_eq!(total.input_targets, 3);
        assert_eq!(total.render_nodes_changed, 7);
    }

    #[test]
    fn glyph_cache_fields_stay_none_until_a_backend_records_them() {
        let counters = WorkCounters::default();
        assert!(counters.glyph_cache_hits.is_none());
        assert!(counters.glyph_cache_misses.is_none());
        assert!(counters.cache_eviction.is_none());
        let mut recorded = WorkCounters::default();
        recorded.record_cache_eviction(0);
        assert_eq!(recorded.cache_eviction, Some(0));
        assert!(recorded.glyph_cache_hits.is_none());
        recorded.record_glyph_cache(2, 1);
        assert_eq!(recorded.glyph_cache_hits, Some(2));
        assert_eq!(recorded.glyph_cache_misses, Some(1));
        recorded.record_glyph_cache(0, 3);
        assert_eq!(recorded.glyph_cache_hits, Some(2));
        assert_eq!(recorded.glyph_cache_misses, Some(4));
        let mut total = recorded;
        total.accumulate(WorkCounters {
            glyph_cache_hits: Some(1),
            glyph_cache_misses: Some(0),
            ..WorkCounters::default()
        });
        assert_eq!(total.glyph_cache_hits, Some(3));
        assert_eq!(total.glyph_cache_misses, Some(4));
    }

    #[test]
    fn gpu_work_fields_are_queryable_as_unsupported_until_a_host_encodes() {
        let counters = WorkCounters::default();
        assert!(counters.batch_rebuilds.is_none());
        assert!(counters.draw_batches.is_none());
        assert!(counters.draw_calls.is_none());
        assert!(counters.gpu_upload_bytes.is_none());
        assert!(counters.gpu_buffer_reallocations.is_none());
        let mut recorded = WorkCounters::default();
        recorded.record_gpu_work(GpuWorkObservation::default());
        assert_eq!(recorded.gpu_upload_bytes, Some(0));
        assert_eq!(recorded.draw_calls, Some(0));
        assert!(recorded.glyph_cache_hits.is_none());
        recorded.record_gpu_work(GpuWorkObservation {
            gpu_upload_bytes: 64,
            draw_calls: 2,
            draw_batches: 1,
            batch_rebuilds: 1,
            gpu_buffer_reallocations: 0,
        });
        assert_eq!(recorded.gpu_upload_bytes, Some(64));
        assert_eq!(recorded.draw_calls, Some(2));
        assert_eq!(recorded.draw_batches, Some(1));
        assert_eq!(recorded.batch_rebuilds, Some(1));
        assert_eq!(recorded.gpu_buffer_reallocations, Some(0));
    }
}
