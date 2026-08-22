use bevy_ecs::component::Component;
use nana_ui_core::WorkCounters;

use crate::{ExtractedNode, StableNodeId};

/// Per-entity invalidation mask. Widened from `u8` because Issue #8 §6.2 STATE
/// and TRANSFORM are independent bits; packing them into the last unused `u8`
/// lane would leave no room for TRANSFORM.
#[derive(Component, Debug, Clone, Copy, Default)]
pub(crate) struct DirtyMask(u16);

impl DirtyMask {
    pub(crate) const STYLE: u16 = 1 << 0;
    pub(crate) const TEXT: u16 = 1 << 1;
    pub(crate) const LAYOUT: u16 = 1 << 2;
    pub(crate) const INPUT: u16 = 1 << 3;
    pub(crate) const FOCUS_IME: u16 = 1 << 4;
    /// Render extraction and paint. PAINT is not an independent bit; paint-only
    /// mutations (hover color, opacity) set RENDER without LAYOUT.
    pub(crate) const RENDER: u16 = 1 << 5;
    pub(crate) const ACCESSIBILITY: u16 = 1 << 6;
    /// Hover / press / focus / interaction authority. Does not imply STYLE.
    pub(crate) const STATE: u16 = 1 << 7;
    /// Paint transform (and unsupported transform diagnostics). Does not imply
    /// STYLE or LAYOUT. RENDER/INPUT are set only when extract or hit-test
    /// consume the matrix.
    pub(crate) const TRANSFORM: u16 = 1 << 8;
    pub(crate) const ALL: u16 = Self::STYLE
        | Self::TEXT
        | Self::LAYOUT
        | Self::INPUT
        | Self::FOCUS_IME
        | Self::RENDER
        | Self::ACCESSIBILITY
        | Self::STATE
        | Self::TRANSFORM;

    pub(crate) const fn all() -> Self {
        Self(Self::ALL)
    }

    pub(crate) fn insert(&mut self, bits: u16) -> bool {
        let before = self.0;
        self.0 |= bits;
        self.0 != before
    }

    pub(crate) fn take(&mut self) -> u16 {
        std::mem::take(&mut self.0)
    }

    /// Whether every bit in `bits` is set.
    pub(crate) fn has(&self, bits: u16) -> bool {
        self.0 & bits == bits
    }
}

/// Deterministic per-system work produced from entity dirty components.
///
/// PAINT is folded into `RENDER`. STATE and TRANSFORM are independent lists so
/// hover/transform invalidation is not counted as style or layout. Mapping onto
/// Issue #8 dirty bits lives on [`nana_ui_core::WorkCounters`]; STATE/TRANSFORM
/// follow `FOCUS_IME` and stay on this type, not WorkCounters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemWork {
    pub generation: u64,
    pub style: Vec<StableNodeId>,
    /// Hover, press, focus, and `SetInteraction` targets. Empty style is valid.
    pub state: Vec<StableNodeId>,
    pub text: Vec<StableNodeId>,
    pub layout: Vec<StableNodeId>,
    /// Paint-transform targets. Empty layout is valid; extract still uses RENDER.
    pub transform: Vec<StableNodeId>,
    pub input_hit_test: Vec<StableNodeId>,
    pub focus_ime: Vec<StableNodeId>,
    pub accessibility: Vec<StableNodeId>,
    pub accessibility_removals: Vec<StableNodeId>,
    pub render_extraction: Vec<StableNodeId>,
    pub render_removals: Vec<StableNodeId>,
    /// Live retained entity count at drain time.
    pub entities_total: usize,
    /// Mounted entities that contributed dirty bits this drain.
    pub entities_changed: usize,
    /// Entities created since the previous drain.
    pub entities_spawned: usize,
    /// Entities despawned since the previous drain.
    pub entities_despawned: usize,
    /// Unique live pointer hover, press, capture, and focus nodes.
    pub input_targets: usize,
    /// Nodes whose render extraction was invalidated this drain. Not overwritten
    /// by [`Self::record_extract`].
    pub render_nodes_changed: usize,
    /// Nodes named for render extraction. Overwritten by
    /// [`Self::record_extract`] with the number actually produced.
    pub render_nodes_extracted: usize,
    /// Theme-resolved text spans after [`Self::record_extract`]. Zero until then.
    pub extracted_text_spans: usize,
    /// Observed CPU hot-path heap events while draining dirty lists.
    pub allocations: usize,
    /// Payload bytes of those drain-list events.
    pub allocated_bytes: usize,
    /// `TextShaper::shape` invocations. Zero until Runtime records shaping.
    pub text_shaped_runs: usize,
    /// `TextLayoutCache` lookup hits. Zero until shaping records the cache.
    pub text_layout_cache_hits: usize,
    /// `TextLayoutCache` inserts after a miss. Zero until shaping.
    pub text_layout_cache_misses: usize,
    /// Shape calls that requested wrapping. Zero until shaping.
    pub text_wrap_layouts: usize,
    /// `GlyphCache` lookup hits. `None` until a glyph backend consults it.
    pub glyph_cache_hits: Option<usize>,
    /// `GlyphCache` inserts after a miss. `None` until consulted.
    pub glyph_cache_misses: Option<usize>,
    /// `TextLayoutCache` evictions. `None` until a shaping pass consults it.
    pub cache_eviction: Option<usize>,
}

impl SystemWork {
    pub fn is_empty(&self) -> bool {
        self.style.is_empty()
            && self.state.is_empty()
            && self.text.is_empty()
            && self.layout.is_empty()
            && self.transform.is_empty()
            && self.input_hit_test.is_empty()
            && self.focus_ime.is_empty()
            && self.accessibility.is_empty()
            && self.accessibility_removals.is_empty()
            && self.render_extraction.is_empty()
            && self.render_removals.is_empty()
    }

    /// Algorithm-level snapshot for Performance Contract assertions.
    pub fn counters(&self) -> WorkCounters {
        WorkCounters {
            entities_total: self.entities_total,
            entities_changed: self.entities_changed,
            entities_spawned: self.entities_spawned,
            entities_despawned: self.entities_despawned,
            style_processed: self.style.len(),
            text_shaped: self.text.len(),
            layout_nodes: self.layout.len(),
            hit_test_candidates: self.input_hit_test.len(),
            input_targets: self.input_targets,
            accessibility_nodes_updated: self.accessibility.len(),
            render_nodes_changed: self.render_nodes_changed,
            render_nodes_extracted: self.render_nodes_extracted,
            extracted_text_spans: self.extracted_text_spans,
            allocations: self.allocations,
            allocated_bytes: self.allocated_bytes,
            text_shaped_runs: self.text_shaped_runs,
            text_layout_cache_hits: self.text_layout_cache_hits,
            text_layout_cache_misses: self.text_layout_cache_misses,
            text_wrap_layouts: self.text_wrap_layouts,
            glyph_cache_hits: self.glyph_cache_hits,
            glyph_cache_misses: self.glyph_cache_misses,
            cache_eviction: self.cache_eviction,
            batch_rebuilds: None,
            draw_batches: None,
            draw_calls: None,
            gpu_upload_bytes: None,
            gpu_buffer_reallocations: None,
        }
    }

    /// Record extract output. Draw batches and GPU upload bytes are not
    /// available from node extraction and are not fabricated here.
    pub fn record_extract(&mut self, extracted: &[ExtractedNode]) {
        self.render_nodes_extracted = extracted.len();
        self.extracted_text_spans = extracted.iter().map(|node| node.text_spans.len()).sum();
    }

    /// Record observed drain/layout/shape heap events. Does not invent GPU bytes.
    pub fn record_hot_path_allocation(&mut self, count: usize, bytes: usize) {
        if count == 0 && bytes == 0 {
            return;
        }
        self.allocations = self.allocations.saturating_add(count);
        self.allocated_bytes = self.allocated_bytes.saturating_add(bytes);
    }

    /// Record shaping-path stats after the host `TextShaper` ran.
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

    pub fn record_cache_eviction(&mut self, evictions: usize) {
        self.cache_eviction = Some(self.cache_eviction.unwrap_or(0).saturating_add(evictions));
    }

    pub fn record_glyph_cache(&mut self, hits: usize, misses: usize) {
        self.glyph_cache_hits = Some(self.glyph_cache_hits.unwrap_or(0).saturating_add(hits));
        self.glyph_cache_misses = Some(self.glyph_cache_misses.unwrap_or(0).saturating_add(misses));
    }
}

pub(crate) fn push_work(work: &mut SystemWork, id: StableNodeId, bits: u16) {
    if bits & DirtyMask::STYLE != 0 {
        work.style.push(id);
    }
    if bits & DirtyMask::STATE != 0 {
        work.state.push(id);
    }
    if bits & DirtyMask::TEXT != 0 {
        work.text.push(id);
    }
    if bits & DirtyMask::LAYOUT != 0 {
        work.layout.push(id);
    }
    if bits & DirtyMask::TRANSFORM != 0 {
        work.transform.push(id);
    }
    if bits & DirtyMask::INPUT != 0 {
        work.input_hit_test.push(id);
    }
    if bits & DirtyMask::FOCUS_IME != 0 {
        work.focus_ime.push(id);
    }
    if bits & DirtyMask::ACCESSIBILITY != 0 {
        work.accessibility.push(id);
    }
    if bits & DirtyMask::RENDER != 0 {
        work.render_extraction.push(id);
    }
}
