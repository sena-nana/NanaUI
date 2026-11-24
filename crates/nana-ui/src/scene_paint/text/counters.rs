//! Monotonic counters for text layout, glyph rasterization, atlas packing,
//! and retained GPU entry updates.

/// The counters Issue #97 asks the text path to answer with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextGlyphCounters {
    pub glyph_resolve_requests: u64,
    pub glyph_rasterized: u64,
    pub glyph_raster_cache_hit: u64,
    pub glyph_raster_cache_miss: u64,
    pub glyph_raster_cache_evict: u64,
    pub glyph_raster_cache_bytes: u64,
    pub glyph_atlas_hit: u64,
    pub glyph_atlas_miss: u64,
    pub glyph_atlas_evict: u64,
    pub glyph_atlas_pages: u32,
    pub glyph_atlas_bytes: u64,
    /// Live glyph area per mille of page area; the complement is gutters plus
    /// fragmentation.
    pub glyph_atlas_occupancy_permille: u32,
    pub glyph_upload_regions: u64,
    pub glyph_upload_bytes: u64,
    pub atlas_relocations: u64,
    pub atlas_stale_handle_rejects: u64,
    pub text_pipeline_draws: u64,
    /// Index slots those draws span: the quads the vertex stage runs, gap and
    /// slack slots included, which cull without reading an instance but are
    /// still run.
    pub text_index_slots_drawn: u64,
    /// Retained `TextGpuEntry` count and its lifecycle.
    pub text_gpu_entries_active: u64,
    pub text_gpu_entries_created: u64,
    pub text_gpu_entries_destroyed: u64,
    /// Entries answered from the store rather than resolved again.
    pub text_gpu_entries_reused: u64,
    /// Live instances the entries hold.
    pub text_gpu_entry_glyphs: u64,
    /// Entries whose glyphs had to be resolved from a shaped paragraph.
    pub text_instance_rebuilds: u64,
    /// Entries repaired in place: an atlas relocation or a new run index,
    /// never a reshape or a rasterize.
    pub text_instance_patches: u64,
    pub text_instance_upload_bytes: u64,
    /// The draw-order index table: four bytes per slot of an entry whose
    /// range was placed or whose block moved. A frame whose draw order and
    /// blocks stood still writes none.
    pub text_index_upload_bytes: u64,
    /// The run and presentation tables, which is where a move, a fade or a
    /// recolor lands instead of in the instances.
    pub text_presentation_upload_bytes: u64,
    pub text_prepare_nodes_considered: u64,
    /// Nodes that reached a draw without resolving a glyph.
    pub text_prepare_nodes_skipped: u64,
    /// Nodes that could not reach a pixel, so they cost no entry and no draw.
    pub text_prepare_nodes_culled: u64,
    /// Paragraphs resolved straight from the Runtime's retained layout.
    pub text_retained_layouts_drawn: u64,
}

/// The monotonic counters one render target accumulates.
///
/// Kept per target because that is where the work happens, and summed across
/// every live target — plus the ones already closed — when read, so a window
/// that is not the one painted last still shows up in the totals and closing
/// it does not make them run backwards.
#[derive(Clone, Copy, Default)]
pub(super) struct TargetCounters {
    pub(super) instance_rebuilds: u64,
    pub(super) instance_patches: u64,
    pub(super) instance_upload_bytes: u64,
    pub(super) index_upload_bytes: u64,
    pub(super) presentation_upload_bytes: u64,
    pub(super) nodes_considered: u64,
    pub(super) nodes_skipped: u64,
    pub(super) nodes_culled: u64,
    pub(super) entries_created: u64,
    pub(super) entries_destroyed: u64,
    pub(super) entries_reused: u64,
}

impl TargetCounters {
    pub(super) fn add(&mut self, other: Self) {
        self.instance_rebuilds += other.instance_rebuilds;
        self.instance_patches += other.instance_patches;
        self.instance_upload_bytes += other.instance_upload_bytes;
        self.index_upload_bytes += other.index_upload_bytes;
        self.presentation_upload_bytes += other.presentation_upload_bytes;
        self.nodes_considered += other.nodes_considered;
        self.nodes_skipped += other.nodes_skipped;
        self.nodes_culled += other.nodes_culled;
        self.entries_created += other.entries_created;
        self.entries_destroyed += other.entries_destroyed;
        self.entries_reused += other.entries_reused;
    }
}
