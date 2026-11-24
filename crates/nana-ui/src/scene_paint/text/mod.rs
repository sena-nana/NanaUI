//! `SceneWgpuPainter::text`: the renderer's own glyph subsystem.
//!
//! ```text
//! shaped paragraph
//!         ↓  resolve
//!     NanaGlyphRun / PlacedGlyph        glyph.rs
//!         ↓  GlyphRasterKey
//!     GlyphRasterCache ── GlyphRasterizer   raster_cache.rs / raster.rs
//!         ↓  GlyphImage
//!     GlyphAtlasManager ── GlyphUploadQueue atlas.rs / upload.rs
//!         ↓  GlyphAtlasEntryId
//!     TextPipeline ──────────────────────── pipeline.rs
//!         ↓
//!        WGPU
//! ```
//!
//! Nothing below the resolver knows what laid the paragraph out. Since #99
//! that is `nana-text`: this module asks the process-wide engine for an
//! immutable [`nana_text::TextLayout`] and resolves it into glyph runs. No
//! cosmic-text type reaches the paint path.
//!
//! Three lifetimes meet here and are deliberately not the same:
//!
//! - **Shaped paragraphs** are CPU state of one painter, keyed by content and
//!   style. A repaint of unchanged text reuses them.
//! - **Glyph bitmaps** are CPU state too, keyed by face instance, size,
//!   subpixel bin and synthesis — never by color, opacity or transform.
//! - **Atlas placements** are device state, shared by every window on one
//!   device and reachable only through a generational handle, so eviction and
//!   compaction cannot be observed as the wrong glyph.

mod atlas;
mod builder;
mod counters;
mod entry;
mod gamma;
mod glyph;
mod ordinals;
mod pipeline;
#[cfg(test)]
pub(super) use self::pipeline::precompiled_inputs;
#[cfg(test)]
pub(super) use self::pipeline::text_shader;
mod raster;
mod raster_cache;
#[cfg(windows)]
mod raster_dwrite;
mod shape_cache;
mod upload;

use nana_text::{TextEngine as _, TextLayout};
use nana_ui_core::sort::CompactSort as _;
use nana_ui_core::{LineHeightSpec, TextAlignSpec};
use nana_ui_runtime::{TextHorizontalAlignment, TextShaping, TextVerticalAlignment};
use nana_ui_scene::{SceneGlyphPresentation, SceneRichPaint, SceneTextOpenType, SceneTextSpan};
use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use self::atlas::{AtlasPageKind, GlyphAtlasLimits, GlyphAtlasManager};
use self::builder::{
    EffectSource, ResolvedSpanColor, SolidQuad, SpanColors, combine_fingerprints, decoration_quads,
    layer_effects, legacy_effects, legacy_shadow_fingerprint, measure, pack_glyph_fx,
    place_solid_quads, push_entry_segment, run_color, text_box_origin, transformed_ink,
};
use self::counters::TargetCounters;
pub use self::counters::TextGlyphCounters;
pub(in crate::scene_paint) use self::entry::EntryKey;
use self::entry::{
    EntrySegment, EntryStore, RunSlots, SegmentBuilder, SlotArena, VACANT_INDEX, range_keeps,
};
use self::glyph::{
    GlyphRenderMode, GlyphRole, GlyphSynthesis, NanaGlyphBuffer, PlacedGlyph, size_bits,
};
use self::ordinals::GraphemeOrdinals;
use self::pipeline::{
    ArenaWrite, CONTENT_COLOR, CONTENT_MASK, DrawSegment, FrameUpload, GlyphInstance, MotionClock,
    TextGpu, TextPresentationGpu, TextRunGpu, TextTargetGpu,
};
use self::shape_cache::{
    PARAGRAPH_IS_RETAINED, ShapeCache, ShapeKeyRef, opentype_disc, opentype_kern_disc,
    opentype_line_disc, retained_paragraph_id,
};
/// The glyph rasterizer this platform draws with: DirectWrite on Windows,
/// so text there matches every native app; swash elsewhere.
#[cfg(windows)]
type PlatformRasterizer = self::raster_dwrite::DWriteGlyphRasterizer;
#[cfg(not(windows))]
type PlatformRasterizer = self::raster::SwashGlyphRasterizer;
use self::raster_cache::GlyphRasterCache;
use self::upload::GlyphUploadQueue;

use super::clip::{self, LogicalRect};
use crate::PhysicalRect;

/// What a caller passes when it cannot say whether the primitive changed.
///
/// Such a frame assembles the shape key and hashes the paragraph, which is
/// what every frame used to do. It never matches a retained entry, including
/// one built by another untracked call.
pub(super) const UNTRACKED_REVISION: u64 = u64::MAX;

/// What a presentation row is derived from: the paint transform, its
/// perspective, the fragment clip and the device scale.
type PresentationInputs = ([f32; 6], [f32; 2], clip::FragmentClip, u32);

/// Frames between retirement sweeps, and how long an entry survives without
/// being drawn. A tab switch that flips back and forth must not pay for
/// either direction.
const RETIRE_INTERVAL: u64 = 64;
const RETIRE_AFTER_FRAMES: u64 = 240;

/// The raster factor a transform's scale earns a paragraph: half-octave steps
/// from 1/4 (step -4) to 4 (step 4), step 0 the device scale. Shrinking earns
/// smaller bitmaps as magnifying earns larger ones, so a glyph's edge stays
/// about one device pixel either way; bilinear sampling a bitmap twice its
/// quad's size would halve the edge and alias thin stems. A press or a
/// zoom-in from 0.9 stays within step 0.
fn raster_factor(step: i8) -> f32 {
    2f32.powf(f32::from(step) * 0.5)
}
/// How far past the midpoint between two steps a magnification must travel
/// before the step an entry holds gives way, in steps. A zoom that settles
/// near a boundary does not flip between two bitmaps as it jitters, and a zoom
/// animation rasterizes each step it passes once rather than once per frame.
const RASTER_HYSTERESIS: f32 = 0.25;
/// Largest em a raster step may ask for, in raster px. Past this a magnified
/// glyph is not worth an atlas region the size of a tile.
const MAX_RASTER_EM_PX: f32 = 256.0;

/// The raster step for text under `affine`, given the step it held.
///
/// The device scale is not an input: it is the DPI policy, applied whatever
/// this says. This is the *scene* scale policy — separate, bucketed and
/// hysteretic, so a transform animation cannot open a raster size per frame.
fn raster_step(affine: [f32; 6], held: Option<i8>, em_px: f32) -> i8 {
    let [a, b, c, d, _, _] = affine;
    let magnification = (a * a + b * b).max(c * c + d * d).sqrt();
    let ceiling = if em_px > 0.0 && em_px.is_finite() {
        ((2.0 * (MAX_RASTER_EM_PX / em_px).log2()).floor()).clamp(0.0, 4.0) as i8
    } else {
        0
    };
    if !magnification.is_finite() || magnification <= 0.0 {
        return held.unwrap_or(0).min(ceiling);
    }
    let wanted = 2.0 * magnification.log2();
    let step = match held {
        Some(held) if (wanted - f32::from(held)).abs() <= 0.5 + RASTER_HYSTERESIS => held,
        _ => wanted.round().clamp(-4.0, 4.0) as i8,
    };
    step.min(ceiling)
}

/// How a paragraph reaches the screen.
///
/// Deliberately everything the retained instances do *not* carry: where the
/// text sits, what color it paints, how opaque it is and what transform it is
/// under. An animation that only touches these writes one 48-byte row.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RunPresentation {
    /// Whole physical pixels the entry's instances are relative to.
    origin: [f32; 2],
    /// Linear RGB with its own alpha; opacity is applied separately so a fade
    /// never has to be folded into a color a glyph was resolved with.
    color: [f32; 4],
    opacity: f32,
    flags: u32,
    presentation: u32,
    /// Physical px per logical px the entry's instances are in.
    raster: f32,
    /// Word offset of the paragraph's per-glyph presentation in this frame's
    /// fx table; zero for none.
    fx: u32,
    transitions: [u64; 2],
}

/// One text draw command's head, or one run folded into an earlier one.
struct TextRun {
    entry: u32,
    presentation: RunPresentation,
    /// Next run in the same draw command. `NO_RUN` ends the chain.
    next: u32,
    /// Chain tail, so folding another run in is O(1).
    last: u32,
    /// Set once this run belongs to an earlier run's command.
    folded: bool,
    /// Filled by [`TextPipeline::flush_runs`] for the chain head.
    segments: Range<u32>,
    /// Device pixels the run paints; the chain head's covers the chain.
    pixels: u64,
}

const NO_RUN: u32 = u32::MAX;

pub(super) struct PreparedText {
    pub index: usize,
    /// Local-space rectangle the glyphs can cover, `bounds` overflow included.
    pub ink: LogicalRect,
}

/// Per-target text state: the arena this target's glyphs are drawn from, the
/// entries that own ranges in it, and the draw commands that name them.
///
/// The atlas, the raster cache and the shaped paragraphs are **not** here.
/// They belong to the device context, so a second window on the same device
/// reuses every glyph the first one faulted in rather than filling a second
/// atlas with the same shell chrome.
pub(super) struct TextPipelineTarget {
    gpu: TextTargetGpu,
    entries: EntryStore,
    runs: Vec<TextRun>,
    live_runs: usize,
    flushed: usize,
    segments: Vec<DrawSegment>,
    /// Indexed by [`entry::TextGpuEntry::slot`], not by draw order, and kept
    /// between frames: a paragraph that did not move, recolor or fade leaves
    /// its row alone even when the frame around it changed completely.
    run_table: Vec<TextRunGpu>,
    /// Rows that differ from what the GPU holds, as one span. Two rows far
    /// apart cost the rows between them, which is cheaper than a write per row
    /// and far cheaper than comparing the whole table.
    run_dirty: Option<Range<u32>>,
    run_slots: RunSlots,
    presentations: Vec<TextPresentationGpu>,
    uploaded_presentations: Vec<TextPresentationGpu>,
    /// This frame's per-glyph presentation table, word 0 reserved so an
    /// offset of zero means none; and what the GPU holds.
    fx: Vec<u32>,
    uploaded_fx: Vec<u32>,
    presentation_index: HashMap<[u32; 40], u32>,
    /// The presentation the label before this one asked for, by the values it
    /// was derived from. See [`TextPipeline::presentation_index`].
    last_presentation: Option<(PresentationInputs, u32)>,
    /// Where each entry's block sits in this target's instance buffer. The
    /// bytes come straight from the entry that owns the block; only the
    /// offsets live here.
    arena: SlotArena,
    /// Where each entry's range sits in the draw-order index table. What
    /// batches is adjacency here, not in the arena.
    order: SlotArena,
    /// Blocks whose arena bytes are no longer what the GPU holds, coalesced
    /// into as few writes as their offsets allow.
    writes: Vec<ArenaWrite>,
    staging: Vec<GlyphInstance>,
    /// Index ranges that were placed or whose block moved, likewise.
    index_writes: Vec<ArenaWrite>,
    index_staging: Vec<u32>,
    /// Scratch for the blocks and ranges a flush is about to place.
    fresh_blocks: Vec<u32>,
    fresh_ranges: Vec<u32>,
    /// Which entry owns the index range starting at each offset, so a range
    /// that grows can find the ones after it.
    order_owners: OrderOwners,
    /// Drawn entries whose block outgrew their range, found by the first walk
    /// so growing them in place does not walk every entry a second time.
    outgrown: Vec<u32>,
    /// Entries this frame leaves undrawn, sorted, because with them the
    /// frame's glyphs would not fit the device's storage binding.
    skipped: Vec<u32>,
    /// Whether the last order repack left gaps, and what has moved in the
    /// order since: ranges placed, and how many of those because their
    /// paragraph grew. What decides whether the next repack leaves gaps.
    order_gapped: bool,
    order_moves: u32,
    growth_moves: u32,
    /// Frames this target drew in a row without a range outgrowing its
    /// place. Past [`SETTLE_FRAMES`] the text has stopped growing, and a
    /// layout with gaps is repacked without them.
    quiet_frames: u32,
    physical_size: [u32; 2],
    frame: u64,
    font_generation: u64,
    subpixel: GlyphRenderMode,
    corner_exponent: f32,
    frame_gpu_allocations: usize,
    /// Everything but the entry lifecycle, which the store counts itself.
    counters: TargetCounters,
    /// What this target's instance and index buffers hold, rebuilt from the
    /// writes the frame sent them. See [`TextPipeline::audit_draw_order`].
    #[cfg(test)]
    shadow: GpuShadow,
}

/// A CPU copy of one target's two GPU buffers, kept only in tests.
#[cfg(test)]
#[derive(Default)]
struct GpuShadow {
    instances: Vec<GlyphInstance>,
    indices: Vec<u32>,
}

impl TextPipelineTarget {
    fn counters(&self) -> TargetCounters {
        let entries = self.entries.counters();
        TargetCounters {
            entries_created: entries.created,
            entries_destroyed: entries.destroyed,
            entries_reused: entries.reused,
            ..self.counters
        }
    }

    /// Forget every range of the draw order, and with them the gaps and what
    /// moved since the last repack: the next layout has not left anything
    /// room, and nothing placed before it says whether text is growing.
    fn forget_order(&mut self) {
        self.order.reset();
        self.entries.invalidate_order();
        self.order_owners.clear(None);
        self.order_gapped = false;
        self.order_moves = 0;
        self.growth_moves = 0;
    }

    fn new(gpu: TextTargetGpu, font_generation: u64) -> Self {
        Self {
            gpu,
            entries: EntryStore::default(),
            runs: Vec::new(),
            live_runs: 0,
            flushed: 0,
            segments: Vec::new(),
            run_table: Vec::new(),
            run_dirty: None,
            run_slots: RunSlots::default(),
            presentations: Vec::new(),
            uploaded_presentations: Vec::new(),
            fx: vec![0],
            uploaded_fx: Vec::new(),
            presentation_index: HashMap::new(),
            last_presentation: None,
            arena: SlotArena::default(),
            order: SlotArena::default(),
            writes: Vec::new(),
            staging: Vec::new(),
            index_writes: Vec::new(),
            index_staging: Vec::new(),
            fresh_blocks: Vec::new(),
            fresh_ranges: Vec::new(),
            order_owners: OrderOwners::default(),
            outgrown: Vec::new(),
            skipped: Vec::new(),
            order_gapped: false,
            order_moves: 0,
            growth_moves: 0,
            quiet_frames: 0,
            physical_size: [0; 2],
            frame: 0,
            font_generation,
            subpixel: GlyphRenderMode::Mask,
            corner_exponent: 2.0,
            frame_gpu_allocations: 0,
            counters: TargetCounters::default(),
            #[cfg(test)]
            shadow: GpuShadow::default(),
        }
    }

    /// Resolve upright text on an opaque backdrop with per-subpixel coverage
    /// for a panel of this order, or fall back to grayscale on `None` or on
    /// devices without dual-source blending.
    pub(super) fn set_subpixel(
        &mut self,
        device: &wgpu::Device,
        order: Option<SubpixelOrder>,
    ) -> bool {
        let mode = match order {
            Some(_) if !TextGpu::supports_subpixel(device) => GlyphRenderMode::Mask,
            Some(SubpixelOrder::Rgb) => GlyphRenderMode::SubpixelRgb,
            Some(SubpixelOrder::Bgr) => GlyphRenderMode::SubpixelBgr,
            None => GlyphRenderMode::Mask,
        };
        if mode == self.subpixel {
            return false;
        }
        self.subpixel = mode;
        true
    }

    /// The scene's corner curve exponent, for the clips of the text drawn
    /// this frame.
    pub(super) fn set_corner_exponent(&mut self, exponent: f32) {
        self.corner_exponent = exponent;
    }

    /// This target is drawn from last frame's batch: nothing was prepared,
    /// so nothing grew. Counted only when that batch draws text, as a flush
    /// is: a frame without any draws no gap.
    pub(super) fn note_reused_frame(&mut self) {
        if self.live_runs > 0 {
            self.quiet_frames = self.quiet_frames.saturating_add(1);
        }
    }

    /// Whether a flush this frame would give back the order's gaps.
    pub(super) fn order_settle_due(&self) -> bool {
        self.order_gapped
            && self.live_runs > 0
            && self.quiet_frames.saturating_add(1) >= SETTLE_FRAMES
    }

    #[cfg(test)]
    pub(super) fn assume_gapped_order(&mut self) {
        self.order_gapped = true;
        self.quiet_frames = 0;
    }

    #[cfg(test)]
    pub(super) fn order_gapped(&self) -> bool {
        self.order_gapped
    }

    /// GPU allocations this frame's text could not reuse.
    pub(super) fn take_frame_gpu_allocations(&mut self) -> usize {
        std::mem::take(&mut self.frame_gpu_allocations)
    }

    /// Fold the run just opened by `next` into `previous`, so both draw as one
    /// command.
    pub(super) fn can_merge_runs(&self, previous: &PreparedText, next: &PreparedText) -> bool {
        // `next` must be the run just opened, so folding it away is an append.
        next.index + 1 == self.live_runs
            && previous.index < next.index
            && previous.index >= self.flushed
            && self.runs.get(previous.index).is_some_and(|run| !run.folded)
    }

    pub(super) fn merge_runs(&mut self, previous: &PreparedText, next: &PreparedText) {
        debug_assert!(self.can_merge_runs(previous, next));
        let tail = self.runs[previous.index].last;
        self.runs[tail as usize].next = next.index as u32;
        self.runs[previous.index].last = next.index as u32;
        self.runs[next.index].folded = true;
        self.runs[previous.index].pixels += self.runs[next.index].pixels;
    }

    /// Record the device pixels `prepared` paints, for the material policy.
    pub(super) fn set_painted(&mut self, prepared: &PreparedText, pixels: u64) {
        if let Some(run) = self.runs.get_mut(prepared.index) {
            run.pixels = pixels;
        }
    }
}

/// The order of a panel's color subpixels, left to right.
///
/// [`SubpixelOrder::system`] reports it on Windows. A host that paints onto
/// a surface of its own passes it in [`ScenePaintOptions`](crate::scene_paint::ScenePaintOptions).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SubpixelOrder {
    Rgb,
    Bgr,
}

impl SubpixelOrder {
    /// The order the system draws its own text with, when it draws it with
    /// subpixel coverage: ClearType's panel order on Windows while the user
    /// has it on, `None` everywhere else. Read once per process.
    pub fn system() -> Option<Self> {
        #[cfg(windows)]
        {
            static ORDER: std::sync::OnceLock<Option<SubpixelOrder>> = std::sync::OnceLock::new();
            *ORDER.get_or_init(raster_dwrite::system_subpixel_order)
        }
        #[cfg(not(windows))]
        None
    }
}

/// The part of a [`TextPipeline`] a working format changes: its programs
/// and the materials compiled for them. Everything else is format-free.
pub(super) struct TextProgram(TextGpu);

impl TextProgram {
    /// A program for targets of `format`, over the atlas pages a pipeline's
    /// [`TextPipeline::atlas_layout`] binds.
    pub(super) fn try_new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas_layout: &wgpu::BindGroupLayout,
        policy: &nana_gpu::GpuDeviceState,
        precompiled: bool,
    ) -> Result<Self, super::validate::ScenePaintError> {
        Ok(Self(TextGpu::try_new_with_policy(
            device,
            format,
            atlas_layout,
            Some(policy),
            precompiled,
        )?))
    }
}

pub(super) struct TextPipeline {
    /// The `nana-text` engine this painter lays paragraphs out through, and
    /// the one whose faces its rasterizer scales. One per process, so a
    /// paragraph shaped for one window is already shaped for the next.
    engine: nana_text::SharedTextEngine,
    rasterizer: PlatformRasterizer,
    raster: GlyphRasterCache,
    atlas: GlyphAtlasManager,
    uploads: GlyphUploadQueue,
    /// This paint follows another of the same painter in one frame, whose
    /// passes the frame's queue writes would land ahead of.
    follows_in_frame: bool,
    /// Atlas regions were copied in an encoder since the last take; see
    /// [`Self::take_ordered_flush`].
    ordered_flushed: bool,
    gpu: TextGpu,
    /// Shaped paragraphs reused across frames. Shaping is the dominant CPU
    /// cost of a text-heavy frame and identical text+style+box repeats on
    /// every repaint (scroll, hover, unrelated animations), so the shaped
    /// `Buffer` is cached and only glyph placement is redone per frame.
    shape_cache: ShapeCache,
    /// Resolver scratch, reused so a paragraph's runs cost no allocation.
    resolved: NanaGlyphBuffer,
    /// The same glyphs in paint order with their shadow and stroke layers,
    /// for a paragraph that has effects. Swapped with `resolved` once built.
    layered: NanaGlyphBuffer,
    /// Underlines and strikeouts of the paragraph being built.
    solids: Vec<SolidQuad>,
    /// What the next [`Self::prepare`] draws besides its fill. One-shot,
    /// like the colour overrides.
    effects: Option<Arc<SceneRichPaint>>,
    legacy_shadow: Option<nana_ui_core::TextShadowSpec>,
    /// What the next [`Self::prepare`] presents per glyph. One-shot.
    glyph_presentation: Option<Arc<SceneGlyphPresentation>>,
    /// The font-set generation this painter's caches were filled at. A
    /// `@font-face` registration reissues faces, so both the shaped paragraphs
    /// and the glyph bitmaps stop meaning what they meant.
    font_generation: u64,
    resolve_requests: u64,
    /// Paragraphs drawn straight from the Runtime's retained layout, i.e. not
    /// laid out a second time here. The counter exists so "measurement and
    /// paint are the same paragraph" is a number a gate can read rather than a
    /// claim in a comment.
    retained_layouts_drawn: u64,
    draws: Cell<u64>,
    slots_drawn: Cell<u64>,
    /// What closed render targets did before they closed.
    closed: TargetCounters,
    /// One-shot linear presentation color used by a paint-only pass such as
    /// an explicit CSS text shadow. Text's retained glyph entries continue
    /// to use their compatibility sRGB colors; only the run row changes.
    linear_color_override: Option<[f32; 4]>,
    paint_color_override: Option<nana_ui_core::PaintColor>,
}

impl TextPipeline {
    #[cfg(test)]
    pub(super) fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self::try_with_atlas_limits_policy(device, format, GlyphAtlasLimits::default(), None, false)
            .expect("text test pipeline")
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(super) fn new_with_policy(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        policy: &nana_gpu::GpuDeviceState,
        precompiled: bool,
    ) -> Self {
        Self::try_new_with_policy(device, queue, format, policy, precompiled)
            .expect("text test pipeline")
    }

    pub(super) fn try_new_with_policy(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        policy: &nana_gpu::GpuDeviceState,
        precompiled: bool,
    ) -> Result<Self, super::validate::ScenePaintError> {
        Self::try_with_atlas_limits_policy(
            device,
            format,
            GlyphAtlasLimits::default(),
            Some(policy),
            precompiled,
        )
    }

    /// A pipeline whose atlas is one `page_edge` page within `byte_budget`,
    /// for painter tests that need eviction within a few labels. Its layouts
    /// come from `policy`, as a painter's do: the device's pipeline registry
    /// keys text programs by those layouts.
    #[cfg(test)]
    pub(super) fn with_atlas_budget(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        page_edge: u32,
        byte_budget: usize,
        policy: &nana_gpu::GpuDeviceState,
    ) -> Self {
        Self::try_with_atlas_limits_policy(
            device,
            format,
            GlyphAtlasLimits {
                page_edge,
                byte_budget,
            },
            Some(policy),
            false,
        )
        .expect("text test pipeline")
    }

    #[cfg(test)]
    fn with_atlas_limits(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        limits: GlyphAtlasLimits,
    ) -> Self {
        Self::try_with_atlas_limits_policy(device, format, limits, None, false)
            .expect("text test pipeline")
    }

    fn try_with_atlas_limits_policy(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        limits: GlyphAtlasLimits,
        policy: Option<&nana_gpu::GpuDeviceState>,
        precompiled: bool,
    ) -> Result<Self, super::validate::ScenePaintError> {
        let raster = GlyphRasterCache::default();
        let atlas = GlyphAtlasManager::new(device, raster.generation(), limits, policy);
        let gpu =
            TextGpu::try_new_with_policy(device, format, atlas.layout(), policy, precompiled)?;
        Ok(Self {
            engine: crate::text_engine::nana_text_engine(),
            rasterizer: PlatformRasterizer::new(crate::text_engine::nana_text_engine()),
            raster,
            atlas,
            uploads: GlyphUploadQueue::default(),
            follows_in_frame: false,
            ordered_flushed: false,
            gpu,
            shape_cache: ShapeCache::default(),
            resolved: NanaGlyphBuffer::default(),
            layered: NanaGlyphBuffer::default(),
            solids: Vec::new(),
            effects: None,
            legacy_shadow: None,
            glyph_presentation: None,
            font_generation: crate::text_engine::engine_font_generation(),
            resolve_requests: 0,
            retained_layouts_drawn: 0,
            draws: Cell::new(0),
            slots_drawn: Cell::new(0),
            closed: TargetCounters::default(),
            linear_color_override: None,
            paint_color_override: None,
        })
    }

    pub(super) fn new_target(&self, device: &wgpu::Device) -> TextPipelineTarget {
        TextPipelineTarget::new(self.gpu.new_target(device), self.font_generation)
    }

    /// Set the linear color for the next [`Self::prepare`] call. This keeps
    /// the existing text preparation API (and its retained-entry behavior)
    /// while allowing CSS authoring spaces to reach the shader without an
    /// sRGB round-trip. The value is consumed even when preparation culls the
    /// paragraph, so it cannot leak into the following text node.
    pub(super) fn set_linear_color_override(&mut self, color: Option<[f32; 4]>) {
        self.linear_color_override = color;
    }

    pub(super) fn set_material_waker(&mut self, wake: super::material::MaterialWaker) {
        self.gpu.set_material_waker(wake);
    }

    /// What the next [`Self::prepare`] presents per glyph. Not part of what
    /// its instances are built from: a new presentation is a new fx row, not
    /// a rebuilt entry.
    pub(super) fn set_glyph_presentation(
        &mut self,
        presentation: Option<&Arc<SceneGlyphPresentation>>,
    ) {
        if let Some(p) = presentation {
            for t in [&p.transitions.entrance, &p.transitions.exit]
                .into_iter()
                .flatten()
            {
                self.gpu.register_material(&t.material);
            }
        }
        self.glyph_presentation = presentation.cloned();
    }

    /// The motion clock glyph presentation samples, written when it moved.
    pub(super) fn write_motion(
        &self,
        target: &mut TextPipelineTarget,
        queue: &wgpu::Queue,
        now: std::time::Duration,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        self.gpu
            .write_motion(queue, &mut target.gpu, MotionClock::at(now), work);
    }

    /// What the next [`Self::prepare`] draws besides its fill: the scene's
    /// resolved effects, or — for a primitive that carries none — its legacy
    /// single `text_shadow`. Consumed by that call, like the colour
    /// overrides, so it cannot leak into the next paragraph.
    pub(super) fn set_effects(
        &mut self,
        rich: Option<&Arc<SceneRichPaint>>,
        legacy_shadow: Option<nana_ui_core::TextShadowSpec>,
    ) {
        if let Some(rich) = rich {
            for effect in std::iter::once(&rich.base).chain(rich.runs.iter().map(|r| &r.effects)) {
                if let Some(material) = &effect.material {
                    self.gpu.register_material(material);
                }
            }
        }
        self.effects = rich.cloned();
        self.legacy_shadow = if rich.is_some() { None } else { legacy_shadow };
    }

    pub(super) fn set_paint_color_override(&mut self, color: Option<nana_ui_core::PaintColor>) {
        self.paint_color_override = color;
    }

    /// Whether the dual-source subpixel program was compiled ahead.
    #[cfg(test)]
    pub(super) fn subpixel_prewarmed(&self) -> bool {
        self.gpu.subpixel_prewarmed()
    }

    /// See [`TextGpu::try_prewarm_subpixel`].
    #[cfg(test)]
    #[allow(dead_code)]
    pub(super) fn prewarm_subpixel(&mut self, device: &wgpu::Device) {
        self.try_prewarm_subpixel(device)
            .expect("text subpixel test prewarm");
    }

    pub(super) fn try_prewarm_subpixel(
        &mut self,
        device: &wgpu::Device,
    ) -> Result<(), super::validate::ScenePaintError> {
        self.gpu.try_prewarm_subpixel(device)
    }

    /// Where a text program for another working format binds this
    /// pipeline's atlas pages; see [`TextGpu::new_with_policy`].
    pub(super) fn atlas_layout(&self) -> &wgpu::BindGroupLayout {
        self.atlas.layout()
    }

    /// Draw through `program`, built for another working format, handing
    /// back the one in use. The engine, shape and raster caches, the atlas and
    /// every target's entries stay: they are the same whatever the format.
    pub(super) fn swap_program(&mut self, program: &mut TextProgram) {
        std::mem::swap(&mut self.gpu, &mut program.0);
    }

    /// Whether this paint follows another of the same painter in one frame:
    /// its atlas regions are then copied in order, see `upload.rs`.
    pub(super) fn set_follows_in_frame(&mut self, follows: bool) {
        self.follows_in_frame = follows;
    }

    /// Whether this paint copied atlas regions in its encoder. The atlas
    /// already counts them as placed, so a frame dropped with them must be
    /// followed by [`Self::forget_atlas_placements`].
    pub(super) fn take_ordered_flush(&mut self) -> bool {
        std::mem::take(&mut self.ordered_flushed)
    }

    pub(super) fn forget_atlas_placements(&mut self) {
        self.atlas.forget_every_placement();
    }

    pub(super) fn begin_frame(&mut self, target: &mut TextPipelineTarget, physical_size: [u32; 2]) {
        let generation = crate::text_engine::engine_font_generation();
        if generation != self.font_generation {
            // Faces were added, replaced or removed. Shaped paragraphs named
            // the old face set and glyph bitmaps were scaled from it, so both
            // are dropped rather than left to be keyed around — and with them
            // every entry, whose instances were resolved through those ids.
            self.font_generation = generation;
            self.shape_cache.clear();
            self.raster.invalidate();
        }
        if target.font_generation != generation {
            target.font_generation = generation;
            Self::drop_every_entry(&mut self.atlas, target);
        }
        // Evicted variation ids are never reissued, so what is keyed by one
        // goes cold instead of wrong, and ages out of the raster cache.
        let _ = self.rasterizer.begin_frame();
        self.atlas.begin_frame(self.raster.generation());
        self.shape_cache.begin_frame();
        target.frame = target.frame.wrapping_add(1);
        target.live_runs = 0;
        target.flushed = 0;
        target.segments.clear();
        // What the GPU holds is last frame's table, so it becomes the
        // comparison baseline by swapping rather than by copying.
        std::mem::swap(
            &mut target.presentations,
            &mut target.uploaded_presentations,
        );
        target.presentations.clear();
        std::mem::swap(&mut target.fx, &mut target.uploaded_fx);
        target.fx.clear();
        target.fx.push(0);
        target.presentation_index.clear();
        target.last_presentation = None;
        target.writes.clear();
        target.staging.clear();
        target.index_writes.clear();
        target.index_staging.clear();
        target.frame_gpu_allocations = 0;
        target.physical_size = physical_size;
        // Entries a shell stopped drawing — a closed panel, a scrolled-away
        // row — keep their block and their claim on the glyphs in it. Retiring
        // them on a schedule rather than on every frame keeps a tab switch
        // that flips back and forth from paying for either direction.
        if target.frame.is_multiple_of(RETIRE_INTERVAL) {
            let atlas = &mut self.atlas;
            let horizon = target.frame.saturating_sub(RETIRE_AFTER_FRAMES);
            let TextPipelineTarget {
                entries,
                run_slots,
                arena,
                order,
                order_owners,
                ..
            } = target;
            entries.retire(
                horizon,
                |handle| atlas.release(handle),
                |slot| run_slots.release(slot),
                |generation, offset, capacity| arena.release(generation, offset, capacity),
                |generation, offset, capacity| {
                    if generation == order.generation() {
                        order_owners.remove(offset);
                    }
                    order.release(generation, offset, capacity)
                },
            );
        }
    }

    /// Drop every retained entry and everything it held.
    ///
    /// The blocks and run rows go back too: without that, the arena and the
    /// run table would grow past every `@font-face` registration a session
    /// sees, because the entries that owned them are gone and can no longer
    /// give them back.
    fn drop_every_entry(atlas: &mut GlyphAtlasManager, target: &mut TextPipelineTarget) {
        target.entries.clear(|handle| atlas.release(handle));
        target.arena.reset();
        target.forget_order();
        target.run_slots.reset();
        target.run_table.clear();
        target.run_dirty = None;
    }

    /// Bumped whenever a placement moved or died. A render target that kept
    /// draw commands from an earlier frame must rebuild them when this
    /// changes: its instances name rectangles that are no longer that glyph's.
    pub(super) fn placement_epoch(&self) -> u64 {
        self.atlas.placement_epoch()
    }

    /// Shape-cache counters for tests: (hits, misses, evictions). None until
    /// consulted, mirroring the Runtime text cache contract.
    pub(super) fn shape_cache_stats(&self) -> (usize, usize, usize) {
        (
            self.shape_cache.hits,
            self.shape_cache.misses,
            self.shape_cache.evictions,
        )
    }

    /// Counters over every render target and the ones already closed.
    pub(super) fn glyph_counters_across<'a>(
        &'a self,
        all_targets: impl Iterator<Item = &'a TextPipelineTarget>,
    ) -> TextGlyphCounters {
        let raster = self.raster.counters();
        let atlas = self.atlas.counters();
        let uploads = self.uploads.counters();
        let mut targets = self.closed;
        let (mut active, mut glyphs) = (0, 0);
        for target in all_targets {
            targets.add(target.counters());
            let entries = target.entries.counters();
            active += entries.active;
            glyphs += entries.glyphs;
        }
        TextGlyphCounters {
            glyph_resolve_requests: self.resolve_requests,
            glyph_rasterized: raster.rasterized,
            glyph_raster_cache_hit: raster.hits,
            glyph_raster_cache_miss: raster.misses,
            glyph_raster_cache_evict: raster.evictions,
            glyph_raster_cache_bytes: self.raster.bytes() as u64,
            glyph_atlas_hit: atlas.hits,
            glyph_atlas_miss: atlas.misses,
            glyph_atlas_evict: atlas.evictions,
            glyph_atlas_pages: atlas.pages,
            glyph_atlas_bytes: atlas.bytes,
            glyph_atlas_occupancy_permille: atlas.occupancy_permille,
            glyph_upload_regions: uploads.regions,
            glyph_upload_bytes: uploads.bytes,
            atlas_relocations: atlas.relocations,
            atlas_stale_handle_rejects: atlas.stale_handle_rejects,
            text_pipeline_draws: self.draws.get(),
            text_index_slots_drawn: self.slots_drawn.get(),
            text_gpu_entries_active: active,
            text_gpu_entries_created: targets.entries_created,
            text_gpu_entries_destroyed: targets.entries_destroyed,
            text_gpu_entries_reused: targets.entries_reused,
            text_gpu_entry_glyphs: glyphs,
            text_instance_rebuilds: targets.instance_rebuilds,
            text_instance_patches: targets.instance_patches,
            text_instance_upload_bytes: targets.instance_upload_bytes,
            text_index_upload_bytes: targets.index_upload_bytes,
            text_presentation_upload_bytes: targets.presentation_upload_bytes,
            text_prepare_nodes_considered: targets.nodes_considered,
            text_prepare_nodes_skipped: targets.nodes_skipped,
            text_prepare_nodes_culled: targets.nodes_culled,
            text_retained_layouts_drawn: self.retained_layouts_drawn,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
        target: &mut TextPipelineTarget,
        device: &wgpu::Device,
        bounds: LogicalRect,
        clip: LogicalRect,
        scale_factor: f32,
        content: &str,
        color: Option<[f32; 4]>,
        size: f32,
        weight: Option<u16>,
        family: Option<&str>,
        line_height: Option<LineHeightSpec>,
        wrap: bool,
        wrap_break: nana_ui_core::TextWrapBreak,
        italic: bool,
        ellipsis: bool,
        max_lines: Option<u16>,
        shaping: TextShaping,
        horizontal: TextHorizontalAlignment,
        vertical: TextVerticalAlignment,
        spans: &[SceneTextSpan],
        letter_spacing: f32,
        font_features: &[nana_ui_core::FontFeatureSetting],
        opentype: &SceneTextOpenType,
        affine: [f32; 6],
        persp: [f32; 2],
        fragment_clip: clip::FragmentClip,
        opacity: f32,
        paint_offset: [f32; 2],
        entry_key: EntryKey,
        revision: u64,
        text_layout: Option<&nana_ui_runtime::RetainedTextLayout>,
        // Nothing between this text and the window's own opaque surface:
        // not inside an offscreen group, whose transparent texture would
        // take subpixel coverage as colored alpha.
        opaque_backdrop: bool,
    ) -> Option<PreparedText> {
        let linear_color_override = self.linear_color_override.take();
        let paint_color_override = self.paint_color_override.take();
        let effects = self.effects.take();
        let legacy_shadow = self.legacy_shadow.take();
        let glyph_presentation = self.glyph_presentation.take();
        if content.is_empty() || bounds.width <= 0.0 || bounds.height <= 0.0 {
            return None;
        }
        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let size = size.max(f32::MIN_POSITIVE);
        let line_height = match line_height {
            Some(LineHeightSpec::Relative(value)) => size * value,
            Some(LineHeightSpec::Absolute(value)) => value,
            None => size * 1.2,
        }
        .max(f32::MIN_POSITIVE);
        // Laid out in **logical** px, which is what makes a layout reusable
        // across device scales and identical to the one Runtime measured the
        // same node with. The device scale enters at resolve time, where it
        // picks the raster size and the sub-pixel bin; advances are linear in
        // the size, so scaling the result is the same text at a different
        // scale, and a DPI change re-rasterizes without laying anything out.
        let box_width = bounds.width.max(0.0);
        let box_height = bounds.height.max(line_height);
        // Opacity rides on the run, not on the color the glyphs were
        // resolved with: a fade must not be a reason to reshape rich text or
        // to rebuild a single instance.
        let default_color = color.unwrap_or([0.0, 0.0, 0.0, 1.0]);
        // Built before the retained lookup because its fingerprint is half of
        // what decides that lookup: the layout is not keyed by colour, so for
        // rich text this is the only thing that can tell one paint from
        // another. Solid text costs an empty `Vec` and a zero.
        let colors = SpanColors::new(content, spans, default_color, paint_color_override);
        // Shadows, strokes and decorations are instances of this same entry,
        // so what they are is part of what its instances were built from —
        // and nothing else: a paragraph without them keeps the solid case's
        // zero.
        let effects_fingerprint = match (&effects, legacy_shadow) {
            (Some(effects), _) => effects.revision,
            (None, Some(shadow)) => legacy_shadow_fingerprint(shadow),
            (None, None) => 0,
        };
        let fingerprint = combine_fingerprints(colors.fingerprint, effects_fingerprint);
        let reach = match (&effects, legacy_shadow) {
            (Some(effects), _) => effects.reach,
            (None, Some(shadow)) => {
                shadow.offset_x.abs().max(shadow.offset_y.abs()) + shadow.blur_radius.max(0.0)
            }
            (None, None) => 0.0,
        };
        // Every path shapes with the full OpenType machinery. The scene still
        // carries the old `Auto | Advanced` distinction; `nana-text` has no
        // reduced mode to select, and a reduced one changed advances enough to
        // wrap text that fit the Runtime content box.
        let (TextShaping::Auto | TextShaping::Advanced) = shaping;
        let align = match horizontal {
            TextHorizontalAlignment::Start => TextAlignSpec::Start,
            TextHorizontalAlignment::Center => TextAlignSpec::Center,
            TextHorizontalAlignment::End => TextAlignSpec::End,
        };
        // The layout Runtime *measured* this node with, when it measured into
        // the box this paints into.
        //
        // Taking it is the difference between measurement and paint being the
        // same paragraph and being two paragraphs that agree by construction:
        // every constraint the Runtime decided — wrap, ellipsis, max-lines,
        // `white-space`, direction — arrives already decided instead of being
        // rebuilt from the scene's description of it. It also costs one float
        // compare where laying out again costs a hash of the whole string.
        //
        // The one thing that has to hold is the alignment box: the glyphs are
        // placed inside the line budget — `max_width_px`, or `max_height_px`
        // for vertical text — and this paints them at the box's start edge,
        // so a node whose paint box is not the box it was measured into (a
        // switch with a trailing control, a list item whose content geometry
        // overrides the text box) falls back to laying out for itself rather
        // than drawing centred text off-centre.
        let retained_paragraph = text_layout
            .filter(|retained| {
                let layout = &retained.layout;
                let (budget, line_box) = if layout.is_vertical() {
                    (layout.constraints.max_height_px, bounds.height.max(0.0))
                } else {
                    (layout.constraints.max_width_px, box_width)
                };
                budget == Some(line_box)
            })
            .map(|retained| Arc::clone(&retained.layout));
        // Nothing the shape key is made of can have changed: the scene has not
        // rewritten this primitive since these glyphs were resolved, and
        // neither the device scale nor the font set has moved. Assembling the
        // key and hashing the paragraph would only prove that again, once per
        // label per frame.
        //
        // Rich text is included: since #99 the span colors are not part of the
        // layout key, and what a span paints is baked into the instances this
        // entry already holds under the same primitive revision.
        //
        // Text under a scaling transform is resolved at the scale it is seen,
        // in steps — see [`raster_step`] — so the scale an entry is compared
        // at is the raster one. A translation never scales, and costs no
        // lookup to find that out.
        let translation = clip::is_translation_projective(affine, persp);
        let step = if translation {
            0
        } else {
            let held = target
                .entries
                .lookup(entry_key)
                .and_then(|id| target.entries.get(id))
                .map(|entry| entry.raster_step);
            raster_step(affine, held, size * scale)
        };
        let raster = scale * raster_factor(step);
        let retained = target
            .entries
            .lookup(entry_key)
            .and_then(|id| Some((id, target.entries.get(id)?)))
            .filter(|(_, entry)| {
                !entry.damaged
                    && revision != UNTRACKED_REVISION
                    && entry.revision == revision
                    && entry.colors == fingerprint
                    && entry.scale_bits == raster.to_bits()
                    && entry.font_generation == self.font_generation
            })
            .map(|(id, entry)| (id, entry.layout, entry.measured))
            // A paragraph the Runtime retains is kept alive by the scene, so
            // only a painter-owned one has to still be in the cache.
            .filter(|(_, hash, _)| {
                *hash & PARAGRAPH_IS_RETAINED != 0 || self.shape_cache.holds(*hash)
            });
        let hash = match (retained, &retained_paragraph) {
            (Some((_, hash, _)), _) => hash,
            // The Runtime's handle *is* the identity: two u32s of a
            // generational slot, never reissued at the same generation. No
            // string is read and nothing is hashed.
            (None, Some(paragraph)) => {
                self.retained_layouts_drawn += 1;
                retained_paragraph_id(paragraph.id)
            }
            (None, None) => {
                // Width, height and requested ellipsis uniquely determine the
                // result; the cache lookup happens before layout so a repaint
                // of unchanged text never reaches the engine.
                let key = ShapeKeyRef {
                    content,
                    family,
                    weight,
                    font_size_bits: size.to_bits(),
                    line_height_bits: line_height.to_bits(),
                    wrap,
                    wrap_break,
                    italic,
                    ellipsis,
                    max_lines,
                    letter_spacing_bits: letter_spacing.to_bits(),
                    word_break: opentype_disc(opentype.word_break),
                    line_break: opentype_line_disc(opentype.line_break),
                    kerning: opentype_kern_disc(opentype.kerning),
                    features: &opentype.features,
                    variations: &opentype.variations,
                    width_bits: box_width.to_bits(),
                    height_bits: box_height.to_bits(),
                    align: match horizontal {
                        TextHorizontalAlignment::Start => 0,
                        TextHorizontalAlignment::Center => 1,
                        TextHorizontalAlignment::End => 2,
                    },
                    direction: if opentype.direction.is_rtl() { 1 } else { 0 },
                    writing_mode: match opentype.writing_mode {
                        nana_ui_core::WritingModeSpec::HorizontalTb => 0,
                        nana_ui_core::WritingModeSpec::VerticalRl => 1,
                        nana_ui_core::WritingModeSpec::VerticalLr => 2,
                    },
                    text_orientation: match opentype.text_orientation {
                        nana_ui_core::TextOrientationSpec::Mixed => 0,
                        nana_ui_core::TextOrientationSpec::Upright => 1,
                        nana_ui_core::TextOrientationSpec::Sideways => 2,
                    },
                    preserve_lines: opentype.preserve_lines,
                    font_features,
                    typography: opentype.typography,
                };
                let hash = key.hash64();
                if self.shape_cache.get(hash, &key).is_none() {
                    let layout = self.lay_out(
                        content,
                        family,
                        weight,
                        size,
                        line_height,
                        letter_spacing,
                        italic,
                        wrap,
                        wrap_break,
                        ellipsis,
                        max_lines,
                        box_width,
                        box_height,
                        align,
                        opentype,
                    );
                    self.shape_cache.insert(hash, key.to_owned_key(), layout);
                }
                hash
            }
        };
        // The widest line and the laid-out height are the layout's, in logical
        // px, and the layout is the one this entry was built from, so a steady
        // frame does not walk its lines again to find that out.
        let (measured_width, laid_out_height) = match (retained, &retained_paragraph) {
            (Some((_, _, measured)), _) => (measured[0], measured[1]),
            (None, Some(paragraph)) => measure(paragraph),
            (None, None) => {
                let layout = self.shape_cache.layout(hash).expect("laid out above");
                measure(layout)
            }
        };
        // Vertical text is aligned inside its own layout, down the column's
        // line budget, so the box origin is where it starts. A `vertical-rl`
        // paragraph is drawn right-aligned against its own column stack (see
        // `build_entry`), and the box adds whatever it has beyond that stack
        // on the left.
        let writing = nana_ui_core::WritingContext::new(opentype.writing_mode, opentype.direction);
        let mut aligned = if !writing.is_vertical() {
            text_box_origin(bounds, vertical, laid_out_height)
        } else if writing.block_reversed() {
            [bounds.x + bounds.width - measured_width, bounds.y]
        } else {
            [bounds.x, bounds.y]
        };
        aligned[0] += paint_offset[0];
        aligned[1] += paint_offset[1];
        // The line box's top lands on a whole pixel of the grid its glyphs are
        // placed on: the device grid for a translation, the entry's own raster
        // grid, before the homography, for anything else.
        //
        // The second is the same snap in the space a projected run is
        // resolved in, and that is what keeps an entry across the switch: at
        // an identity transform both put the paragraph at the same sub-pixel
        // phase, so a label whose container starts or stops turning, scaling
        // or pressing keeps its glyphs — its run row changes its flags and
        // nothing else. Unsnapped, every such transition rebuilt every label
        // under the container, and a press made the label jump by a fraction
        // of a pixel on its first frame.
        //
        // The snapped top is kept as the whole pixel it is. Carried back
        // through logical px and out again it comes back as 1.9999999 as
        // often as 2.0, and `floor` turns that into a line one pixel higher
        // with a phase of 0.9999999: a label that jumps a pixel on some
        // frames of a scroll and is re-resolved on each of them.
        let line_logical = laid_out_height;
        let top_px = if translation {
            let wy = clip::on_grid(aligned[1], affine[5], scale);
            let (top_px, _) =
                clip::snap_centered_origin(wy + line_logical * 0.5, line_logical, scale);
            aligned[1] += top_px / scale - wy;
            top_px
        } else {
            let (top_px, _) =
                clip::snap_centered_origin(aligned[1] + line_logical * 0.5, line_logical, raster);
            aligned[1] = top_px / raster;
            top_px
        };
        if fragment_clip == clip::FragmentClip::REJECT {
            return None;
        }
        // What the glyphs can actually cover, in the same local space as
        // `bounds`. Not the content box: `overflow: visible` text (a fixed
        // height holding three lines, `wrap: false` in a narrow box) paints
        // outside it, and the caller uses this to decide whether reordering a
        // batch would cross this text.
        //
        // The laid-out box is the union of the line boxes. Ink leaves it
        // vertically exactly when the requested line height is shorter than
        // what the face needs, so pad by that shortfall against a 1.25em
        // natural height — nothing at a normal line height, a few pixels at
        // `line-height: 1`. Horizontally the pad covers side bearings, which
        // an italic or a swash can push past the advance box.
        // Shadows and outlines reach past the glyphs by what they say.
        let pad_y = (size * 1.25 - line_height).max(0.0) + reach;
        let pad_x = size * 0.25 + reach;
        let ink = LogicalRect::from_xywh(
            aligned[0] - pad_x,
            aligned[1] - pad_y,
            bounds.width.max(measured_width) + pad_x * 2.0,
            laid_out_height + pad_y * 2.0,
        );
        // Where the paragraph starts, in the whole pixels its run row carries
        // and the sub-pixel phase its glyphs are rasterized for. A translated
        // run's whole pixels are relative to the translation's own, which the
        // presentation row carries: a scroll that lands on whole pixels then
        // moves the one row every label under it shares, and not a run.
        //
        // The phase is the paragraph's own position's alone. The painter
        // draws a translation on whole device pixels (`clip::snap_translation`,
        // #223), so a scroll by any amount, fraction or not, keeps every
        // label's glyphs; what the translation adds past its whole pixels is
        // the ulp `k / scale * scale` can come back with, and that is no phase.
        // Vertically there is no phase at all: the line box top is a whole
        // pixel.
        let (x, y) = if translation {
            let [_, ty] = pipeline::whole_translation(affine, scale);
            (aligned[0] * scale, top_px - ty)
        } else {
            (aligned[0] * raster, top_px)
        };
        let whole = [x.floor(), y];
        let phase = [(x - whole[0]).to_bits(), 0f32.to_bits()];
        // An axis-aligned run is clipped by the batch's scissor. Rotated or
        // projective text carries the same homography as Quad, applied per
        // glyph corner in the vertex stage, and a rounded or polygonal clip
        // needs the fragment test the scissor cannot express — neither of
        // which is a reason to resolve the paragraph differently.
        let mut flags = 0;
        if !translation {
            // The corners no longer land on the texel grid, so nearest
            // sampling would alias them.
            flags |= pipeline::RUN_PROJECT | pipeline::RUN_LINEAR;
        }
        if fragment_clip != clip::FragmentClip::PASS {
            flags |= pipeline::RUN_CLIP;
        }
        // Text that cannot reach a pixel costs nothing: no entry, no run, no
        // draw. The same predicate the scissor would apply, one rectangle at a
        // time instead of one glyph at a time.
        // `ink` is in the node's own space and `clip` is in paint space, so
        // the homography has to be applied before they can be compared — a
        // scrolled viewport folds its offset into `affine`, and comparing the
        // two spaces directly would cull exactly the text that scrolled into
        // view.
        let reachable = transformed_ink(ink, affine, persp)
            .intersection(clip)
            .is_some();
        target.counters.nodes_considered += 1;
        if !reachable {
            target.counters.nodes_culled += 1;
            return None;
        }
        // Subpixel coverage only where the blend lands on the surface it was
        // rasterized for: an upright run, straight onto the window.
        let mode = if translation && opaque_backdrop {
            target.subpixel
        } else {
            GlyphRenderMode::Mask
        };
        let index = target.live_runs;
        if index == target.runs.len() {
            target.runs.push(TextRun {
                entry: 0,
                presentation: RunPresentation {
                    origin: [0.0; 2],
                    color: [0.0; 4],
                    opacity: 1.0,
                    flags: 0,
                    presentation: 0,
                    raster: 1.0,
                    fx: 0,
                    transitions: [0; 2],
                },
                next: NO_RUN,
                last: index as u32,
                folded: false,
                segments: 0..0,
                pixels: 0,
            });
        }
        let epoch = self.placement_epoch();
        let reusable = retained
            .map(|(id, _, _)| id)
            .or_else(|| target.entries.lookup(entry_key))
            .filter(|id| {
                target.entries.get(*id).is_some_and(|entry| {
                    entry.valid(
                        hash,
                        fingerprint,
                        phase,
                        raster.to_bits(),
                        self.font_generation,
                        mode,
                    )
                })
            })
            .filter(|id| {
                // The atlas moved since this entry read its rectangles. Repair
                // them through the handles it kept — no shaping, no
                // rasterizing, no atlas traffic. Repairing here rather than at
                // flush is what lets a handle the atlas has reissued be
                // *resolved* again in the same frame instead of drawing a
                // paragraph with holes in it for one frame.
                let stale = target
                    .entries
                    .get(*id)
                    .is_some_and(|entry| entry.atlas_epoch != epoch);
                if !stale {
                    return true;
                }
                target.counters.instance_patches += 1;
                target
                    .entries
                    .repair(*id, epoch, |handle| self.atlas.entry(handle).copied())
            });
        let entry = match reusable {
            Some(id) => {
                // The paragraph, its sub-pixel phase and the face set are all
                // the ones these instances were resolved from. Nothing below
                // this line reads a glyph.
                target.entries.note_reuse();
                target.counters.nodes_skipped += 1;
                id
            }
            None => self.build_entry(
                target,
                device,
                entry_key,
                hash,
                phase,
                (raster, step),
                mode,
                default_color,
                &colors,
                fingerprint,
                EffectSource {
                    rich: effects.as_deref(),
                    legacy_shadow,
                },
                content,
                revision,
                retained_paragraph.as_deref(),
            )?,
        };
        let frame = target.frame;
        if let Some(entry) = target.entries.get_mut(entry) {
            entry.last_used = frame;
            entry.measured = [measured_width, laid_out_height];
        }
        // After the entry, so a paragraph that resolves to nothing does not
        // leave a row in the table nobody names.
        let presentation = Self::presentation_index(target, affine, persp, fragment_clip, scale);
        let run = &mut target.runs[index];
        run.entry = entry;
        let fx = glyph_presentation.as_deref().map_or(0, |presentation| {
            let mut presentation = presentation.clone();
            if presentation.transitions.viewport.is_none() {
                presentation.transitions.viewport = Some(nana_ui_core::LogicalRect::new(
                    (clip.x - bounds.x).max(0.),
                    (clip.y - bounds.y).max(0.),
                    bounds.width.min(clip.width),
                    bounds.height.min(clip.height),
                ));
            }
            pack_glyph_fx(&mut target.fx, &presentation)
        });
        run.presentation = RunPresentation {
            origin: whole,
            color: linear_color_override.unwrap_or_else(|| run_color(default_color)),
            opacity: opacity.clamp(0.0, 1.0),
            flags,
            presentation,
            raster,
            fx,
            transitions: glyph_presentation.as_ref().map_or([0; 2], |p| {
                [
                    p.transitions
                        .entrance
                        .as_ref()
                        .map_or(0, |t| t.material.key()),
                    p.transitions.exit.as_ref().map_or(0, |t| t.material.key()),
                ]
            }),
        };
        run.next = NO_RUN;
        run.last = index as u32;
        run.folded = false;
        run.segments = 0..0;
        target.live_runs += 1;
        Some(PreparedText { index, ink })
    }

    /// The row `affine`/`persp`/`clip` present through, adding it if this
    /// frame has not seen that combination yet.
    fn presentation_index(
        target: &mut TextPipelineTarget,
        affine: [f32; 6],
        persp: [f32; 2],
        fragment_clip: clip::FragmentClip,
        scale: f32,
    ) -> u32 {
        // A shell's labels nearly all share one transform and one clip. Ask
        // that question of the four values the row is derived from, not of the
        // row: building it is a clip inversion and a hundred and sixty bytes,
        // and hashing it is forty words, to find out they are the ones the
        // label before this one already had.
        let inputs = (affine, persp, fragment_clip, scale.to_bits());
        if let Some((held, index)) = target.last_presentation
            && held == inputs
        {
            return index;
        }
        let row = TextPresentationGpu::new(
            affine,
            persp,
            &fragment_clip.for_physical_pixels(scale),
            scale,
        );
        let bits = row.to_bits();
        if let Some(last) = target.presentations.last()
            && *last == row
        {
            let index = (target.presentations.len() - 1) as u32;
            target.last_presentation = Some((inputs, index));
            return index;
        }
        if let Some(index) = target.presentation_index.get(&bits) {
            let index = *index;
            target.last_presentation = Some((inputs, index));
            return index;
        }
        let index = target.presentations.len() as u32;
        target.presentations.push(row);
        target.presentation_index.insert(bits, index);
        target.last_presentation = Some((inputs, index));
        index
    }

    /// Lay one paragraph out through the process-wide `nana-text` engine.
    ///
    /// Everything is passed in **logical** px and the layout scale is left at
    /// 1, so the result does not depend on the device scale: the same layout
    /// serves 1x and 2x, and it is the same one Runtime measured this node
    /// with. The scale is applied to the *positions* when the glyphs are
    /// resolved.
    #[allow(clippy::too_many_arguments)]
    fn lay_out(
        &mut self,
        content: &str,
        family: Option<&str>,
        weight: Option<u16>,
        font_size_px: f32,
        line_height_px: f32,
        letter_spacing_px: f32,
        italic: bool,
        wrap: bool,
        wrap_break: nana_ui_core::TextWrapBreak,
        ellipsis: bool,
        max_lines: Option<u16>,
        max_width_px: f32,
        max_height_px: f32,
        align: TextAlignSpec,
        opentype: &SceneTextOpenType,
    ) -> Arc<TextLayout> {
        let source = nana_text::TextSource::new(content);
        let style = nana_text::TextStyle {
            // The same family rule Runtime measured with, from the same
            // function: a named family that says `mono` falls back to
            // `monospace`. Two spellings of it would be two font selections
            // for one node.
            font_family: family.map(nana_ui_runtime::nana_font_family),
            font_size_px,
            font_weight: weight.unwrap_or(400),
            // As Runtime's base style: only a rich span stretches, and rich
            // text paints from its retained layout.
            font_stretch: 100.0,
            italic,
            line_height: Some(LineHeightSpec::Absolute(line_height_px)),
            // A non-finite tracking would poison every advance after it.
            letter_spacing_px: if letter_spacing_px.is_finite() {
                letter_spacing_px
            } else {
                0.0
            },
            features: opentype.features.clone(),
            variations: opentype.variations.clone(),
            kerning: opentype.kerning,
            language: opentype.language.clone(),
        };
        // The box dimension lines stack along — the height, or the width of
        // vertical text — is a truncation budget, so it only goes to the
        // engine when truncation was asked for: the same rule
        // `nana_text_constraints` applies to the layout Runtime keeps, so the
        // fallback here and the handle cannot disagree about it. A box too
        // short is an overflow the scissor clips, not a shorter paragraph.
        let vertical = opentype.writing_mode.is_vertical();
        let constraints = nana_text::TextConstraints {
            max_width_px: (!vertical || ellipsis).then_some(max_width_px),
            max_height_px: (vertical || ellipsis).then_some(max_height_px),
            wrap: wrap.then_some(wrap_break),
            word_break: opentype.word_break,
            line_break: opentype.line_break,
            max_lines,
            ellipsis,
            // What the Runtime measured this node with. `white-space: normal`
            // folds an authored newline into a space, and measuring one line
            // while painting two is the whole reason this rides on the scene
            // rather than being assumed here.
            preserve_lines: opentype.preserve_lines,
            base_direction: opentype.direction,
            align,
            writing_mode: opentype.writing_mode,
            text_orientation: opentype.text_orientation,
            scale: nana_text::TextScale {
                px_per_logical: 1.0,
            },
            ..nana_text::TextConstraints::default()
        };
        let mut constraints = constraints;
        nana_ui_runtime::apply_used_typography(&mut constraints, opentype.typography);
        let kind = if wrap {
            nana_text::TextKind::Paragraph
        } else {
            nana_text::TextKind::Label
        };
        let mut counters = nana_text::TextWorkCounters::default();
        let engine = Arc::clone(&self.engine);
        let mut engine = crate::text_engine::lock_engine(&engine);
        engine.layout(kind, &source, &style, &constraints, &mut counters)
    }

    /// Turn one shaped paragraph into placed, atlas-resident glyphs, and keep
    /// them.
    ///
    /// This is the only function that knows how the paragraph was laid out.
    /// Everything it stores is in the renderer's own terms: an instance per
    /// glyph in the run's own space, and the atlas handle it was read from so
    /// a later relocation can be repaired instead of re-resolved.
    #[allow(clippy::too_many_arguments)]
    fn build_entry(
        &mut self,
        target: &mut TextPipelineTarget,
        device: &wgpu::Device,
        key: EntryKey,
        hash: u64,
        phase: [u32; 2],
        // Logical-to-raster px, applied to the layout's coordinates here and
        // nowhere else: it is not in the layout key, so 1x and 2x resolve the
        // same paragraph into different glyphs. The device scale times the
        // raster step, which is kept on the entry for the next frame's
        // hysteresis.
        (scale, step): (f32, i8),
        mode: GlyphRenderMode,
        default_color: [f32; 4],
        colors: &SpanColors,
        // `colors` and the effects together, which is what the entry is
        // stamped with.
        fingerprint: u64,
        effects: EffectSource<'_>,
        // The text the paragraph lays out: what a glyph's grapheme ordinal,
        // which its per-glyph presentation is looked up by, counts in.
        content: &str,
        revision: u64,
        // The Runtime's own layout for this node, when it has one. `None`
        // means the painter laid this paragraph out itself and it is in the
        // shape cache under `hash`.
        paragraph: Option<&TextLayout>,
    ) -> Option<u32> {
        let origin = [f32::from_bits(phase[0]), f32::from_bits(phase[1])];
        let Self {
            shape_cache,
            resolved,
            layered,
            solids,
            rasterizer,
            font_generation,
            ..
        } = self;
        let layout = match paragraph {
            Some(paragraph) => paragraph,
            None => shape_cache.layout(hash).expect("laid out above"),
        };
        resolved.clear();
        let generation = *font_generation as u32;
        let vertical = layout.is_vertical();
        // Lines, then ruby above their bases, emphasis marks beside their
        // characters and object labels inside their objects: each glyph that
        // stands for a character takes that character's effects and reveal
        // turn, and its colour by what part of the paragraph it draws.
        for placed in layout.glyph_placements(scale, origin) {
            let (font, variation, mut synthesis) = rasterizer.intern_instance(placed.instance);
            if placed.sideways {
                synthesis = synthesis.with(GlyphSynthesis::ROTATE_CW);
            }
            let cluster = placed.cluster as usize;
            let color = match placed.source {
                nana_text::GlyphSource::Line | nana_text::GlyphSource::Ruby => {
                    colors.color_at(cluster)
                }
                // The mark colour, or the character's own fill.
                nana_text::GlyphSource::Emphasis => match effects
                    .rich
                    .and_then(|rich| rich.effects_at(cluster).emphasis_color)
                {
                    Some(paint) => ResolvedSpanColor {
                        srgb: paint.to_srgb(),
                        paint_color: Some(paint),
                    },
                    None => colors.color_at(cluster),
                },
                // A light ink unless the label names one.
                nana_text::GlyphSource::Label(label) => ResolvedSpanColor {
                    srgb: layout.labels[label].color.unwrap_or([1.0, 1.0, 1.0, 1.0]),
                    paint_color: None,
                },
            };
            // A vertical column draws plain coverage.
            let mode = if vertical && placed.source == nana_text::GlyphSource::Line {
                GlyphRenderMode::Mask
            } else {
                mode
            };
            resolved.push(
                font,
                generation,
                variation,
                size_bits(placed.font_size_px),
                synthesis,
                mode,
                color.srgb,
                color.paint_color,
                GlyphRole::Fill,
                PlacedGlyph {
                    glyph: placed.glyph_id,
                    x: placed.position[0],
                    y: placed.position[1],
                    cluster: placed.cluster,
                },
            );
        }
        if resolved.is_empty() {
            return None;
        }
        // Effects: the same glyphs again under their shadows and outlines,
        // and the lines along their runs. Built here, where the layout is in
        // hand; a legacy single shadow is resolved into the same form.
        solids.clear();
        let synthesized;
        let rich = match (effects.rich, effects.legacy_shadow) {
            (Some(rich), _) => Some(rich),
            (None, Some(shadow)) => {
                synthesized = legacy_effects(shadow);
                synthesized.as_deref()
            }
            (None, None) => None,
        };
        let fill_start = match rich {
            Some(rich) => {
                let fill_start = layer_effects(resolved, rich, scale, layered);
                std::mem::swap(resolved, layered);
                if rich.has_decoration() && !layout.is_vertical() {
                    decoration_quads(layout, rich, colors, scale, origin, solids);
                }
                fill_start
            }
            None => 0,
        };
        let Self {
            resolved,
            solids,
            rasterizer,
            raster,
            atlas,
            uploads,
            resolve_requests,
            ..
        } = self;
        let pages_before = atlas.pages_opened();
        // Read before the first placement, not after the last. Faulting a
        // glyph in can repack the atlas, and a repack moves glyphs this same
        // build already wrote instances for; stamping the entry with the
        // epoch from *after* that would declare those rectangles current and
        // nothing would ever repair them. Stamped with this one, the flush
        // re-reads them through the handles.
        let epoch = atlas.placement_epoch();
        // A glyph the atlas could not place this frame. The entry draws what
        // it has and asks again next frame, when the frame that crowded it
        // out may be gone; without this it would be missing for good, since
        // nothing else about the paragraph would change.
        let mut unplaced = false;
        let inherited = pipeline::pack_srgb(default_color);
        let placeholders = [
            atlas.placeholder_page(AtlasPageKind::Mask),
            atlas.placeholder_page(AtlasPageKind::Color),
        ];
        let id = target.entries.begin_build(
            key,
            (resolved.glyphs.len() + solids.len()) as u32,
            |handle| atlas.release(handle),
        );
        target.counters.instance_rebuilds += 1;
        let mut placed = 0u32;
        let mut segments: Vec<EntrySegment> = Vec::new();
        // Paint order: shadows and under-strokes, underlines, fills and
        // over-strokes, strikeouts.
        let ordinals = GraphemeOrdinals::new(content);
        let place_solids = |line_through: bool,
                            entries: &mut EntryStore,
                            placed: &mut u32,
                            segments: &mut Vec<EntrySegment>| {
            place_solid_quads(
                entries,
                id,
                solids,
                line_through,
                inherited,
                placeholders,
                placed,
                segments,
                &ordinals,
            );
        };
        let underlines_at = if solids.is_empty() {
            usize::MAX
        } else {
            fill_start
        };
        for (index, run) in resolved.runs.iter().enumerate() {
            if index == underlines_at {
                place_solids(false, &mut target.entries, &mut placed, &mut segments);
            }
            let role = match run.role {
                GlyphRole::Fill => pipeline::ROLE_FILL,
                GlyphRole::Shadow => pipeline::ROLE_SHADOW,
                GlyphRole::Stroke => pipeline::ROLE_STROKE,
            };
            // The run's color is the same for every glyph under it, so it is
            // packed once rather than per glyph — and compared once against
            // the paragraph's own color, because a glyph that paints it can
            // inherit the run row instead of carrying four bytes that a
            // recolor would then have to rewrite.
            let color = pipeline::pack_srgb(run.color);
            let own = if run.paint_color.is_some() {
                pipeline::INSTANCE_OWN_COLOR | pipeline::INSTANCE_LINEAR_COLOR
            } else if color == inherited {
                0
            } else {
                pipeline::INSTANCE_OWN_COLOR
            };
            let linear_color = run
                .paint_color
                .map(super::color::pack_paint_color)
                .unwrap_or([0.0; 4]);
            for glyph in resolved.glyphs_of(run) {
                *resolve_requests += 1;
                let (raster_key, pen) = run.raster_key(glyph);
                let (handle, placement) = match atlas.lookup(&raster_key) {
                    Some(placed) => placed,
                    None => {
                        let Some(image) = raster.get_or_rasterize(rasterizer, raster_key) else {
                            continue;
                        };
                        match atlas.insert(device, raster_key, &image, raster, uploads) {
                            Some(placed) => placed,
                            None => {
                                // Waiting for room, not too big for any page:
                                // one that can never fit would only fail again,
                                // and rebuild its paragraph every frame doing so.
                                unplaced |= atlas.could_hold(&image);
                                continue;
                            }
                        }
                    }
                };
                atlas.retain(handle);
                let content = match placement.kind {
                    AtlasPageKind::Mask => CONTENT_MASK,
                    AtlasPageKind::Color if placement.subpixel => {
                        CONTENT_COLOR | pipeline::INSTANCE_SUBPIXEL
                    }
                    AtlasPageKind::Color => CONTENT_COLOR,
                };
                let (mask_page, color_page) = match placement.kind {
                    AtlasPageKind::Mask => (placement.page, placeholders[1]),
                    AtlasPageKind::Color => (placeholders[0], placement.page),
                };
                let material = if role == pipeline::ROLE_FILL
                    && (placement.kind == AtlasPageKind::Mask || placement.subpixel)
                {
                    rich.and_then(|r| r.effects_at(glyph.cluster as usize).material.as_ref())
                        .map_or(0, |m| m.key())
                } else {
                    0
                };
                push_entry_segment(
                    &mut segments,
                    placeholders,
                    mask_page,
                    color_page,
                    placed,
                    material,
                );
                target.entries.push_glyph(
                    id,
                    placed,
                    handle,
                    GlyphInstance::new(
                        [pen[0] + placement.left, pen[1] - placement.top],
                        placement.size,
                        placement.origin,
                        color,
                        content | own,
                        linear_color,
                    )
                    .with_role(role)
                    .with_ordinal(ordinals.of(glyph.cluster as usize)),
                );
                placed += 1;
            }
        }
        if !solids.is_empty() {
            if underlines_at >= resolved.runs.len() {
                place_solids(false, &mut target.entries, &mut placed, &mut segments);
            }
            place_solids(true, &mut target.entries, &mut placed, &mut segments);
        }
        target.frame_gpu_allocations += atlas.pages_opened() - pages_before;
        target.entries.finish_build(id, placed);
        let fonts = self.font_generation;
        let entry = target.entries.get_mut(id).expect("just built");
        entry.layout = hash;
        entry.colors = fingerprint;
        entry.phase = phase;
        entry.font_generation = fonts;
        entry.mode = mode;
        entry.revision = revision;
        entry.scale_bits = scale.to_bits();
        entry.raster_step = step;
        entry.atlas_epoch = epoch;
        entry.segments = segments;
        entry.run = NO_RUN;
        entry.damaged = unplaced;
        if placed == 0 {
            return None;
        }
        Some(id)
    }

    /// Give every still-open run an arena block, an index range and a run
    /// row, and turn the entries under it into draw segments. Must run before
    /// `upload` and before `draw`.
    ///
    /// This is where retention pays: an entry whose block and range are
    /// already placed, under the run index it already names, is passed over
    /// without reading a single instance or writing a single index. An entry
    /// that outgrew its block moves in the arena and writes its own bytes
    /// there; what its neighbours pay for it is at most a rewrite of the index
    /// table, never of their instances.
    pub(super) fn flush_runs(&mut self, target: &mut TextPipelineTarget) {
        if target.flushed >= target.live_runs {
            return;
        }
        let Self { atlas, gpu, .. } = self;
        // The table persists, and a row is only written when it actually
        // changed, so an unchanged frame neither copies it nor compares it.
        target.run_dirty = None;
        let rows = target.run_table.len();
        target
            .run_table
            .resize(target.run_slots.len(), TextRunGpu::VACANT);
        if target.run_table.len() > rows {
            target.run_dirty = Some(rows as u32..target.run_table.len() as u32);
        }
        target.segments.clear();
        target.writes.clear();
        target.staging.clear();
        target.index_writes.clear();
        target.index_staging.clear();
        // What the frame needs, and what of it the arena and the order do not
        // already hold, so a repack happens instead of running out of room.
        let mut outgrown = std::mem::take(&mut target.outgrown);
        let mut fresh = std::mem::take(&mut target.fresh_blocks);
        let mut fresh_order = std::mem::take(&mut target.fresh_ranges);
        target.skipped.clear();
        let limit = gpu.instance_slots();
        target.arena.set_limit(limit);
        let mut total = Self::measure_runs(target, &mut fresh, &mut fresh_order, &mut outgrown);
        if total > limit {
            // More glyphs than this device can bind as one storage buffer —
            // a whole log file set as one paragraph. The largest paragraphs
            // are left out until the rest fits with room to spare:
            // everything else on screen still draws, and a frame that would
            // have failed validation says so instead.
            let asked = total;
            Self::skip_largest(target, total, limit);
            total = Self::measure_runs(target, &mut fresh, &mut fresh_order, &mut outgrown);
            use nana_diagnostics::framework::text;
            static EXCEEDED: nana_diagnostics::Throttle = nana_diagnostics::Throttle::new();
            nana_diagnostics::metric!(text::INSTANCE_LIMIT_FRAMES);
            if nana_diagnostics::enabled(text::INSTANCE_LIMIT_EXCEEDED.severity)
                && EXCEEDED.allow(std::time::Duration::from_secs(10))
            {
                nana_diagnostics::event!(
                    text::INSTANCE_LIMIT_EXCEEDED,
                    slots = u64::from(asked),
                    limit = u64::from(limit),
                    skipped = target.skipped.len() as u64
                );
            }
        }
        // A range outgrowing its place is text still growing. Once it has
        // stopped for long enough, gaps left for it are quads every frame
        // runs for nothing, and the order is repacked without them.
        target.quiet_frames = if outgrown.is_empty() {
            target.quiet_frames.saturating_add(1)
        } else {
            0
        };
        let settled = target.quiet_frames >= SETTLE_FRAMES;
        // Independent: the arena repacks only to make room or close holes,
        // and then rewrites instances; the order repacks when fragmentation
        // costs too many draws, or to give back gaps, and then rewrites only
        // indices. Never twice in a frame: one decision covers both reasons.
        let arena_repack = target.arena.should_repack(total, &fresh);
        let order_repack =
            target.order.should_repack(total, &fresh_order) || (target.order_gapped && settled);
        target.fresh_blocks = fresh;
        target.fresh_ranges = fresh_order;
        if arena_repack {
            target.arena.repack(total);
            target.entries.invalidate_arena();
        }
        if order_repack {
            target.order.repack(total);
            target.entries.invalidate_order();
            // Most of what moved since the last repack moved because its
            // paragraph grew: this text keeps changing length, so the new
            // layout leaves room for it to grow where it is. Text that only
            // comes and goes gets none — a gap would not keep it from
            // splitting the draws, and every gap slot is a quad the vertex
            // stage still runs. Nor does text that grew once but has since
            // stood still.
            target.order_gapped = !settled && target.growth_moves * 2 > target.order_moves;
            // Only a layout with gaps grows ranges in place, which is the
            // one thing that asks who owns an offset.
            target
                .order_owners
                .clear(target.order_gapped.then(|| target.order.capacity()));
            target.growth_moves = 0;
            target.order_moves = 0;
        } else if target.order_gapped && !outgrown.is_empty() {
            Self::grow_ranges_in_place(target, &outgrown);
        }
        target.outgrown = outgrown;
        let epoch = atlas.placement_epoch();
        let placeholders = [
            atlas.placeholder_page(AtlasPageKind::Mask),
            atlas.placeholder_page(AtlasPageKind::Color),
        ];
        let generation = target.arena.generation();
        let order_generation = target.order.generation();
        let mut breaks = 0u32;
        let mut group_start = 0u32;
        for index in 0..target.live_runs {
            if target.runs[index].folded {
                continue;
            }
            let first_segment = target.segments.len() as u32;
            let mut builder = SegmentBuilder::new(placeholders);
            let mut member = index as u32;
            let mut previous_end = None;
            loop {
                let entry_id = target.runs[member as usize].entry;
                let slot = match target.entries.get(entry_id).and_then(|entry| entry.slot) {
                    Some(slot) => slot,
                    None => {
                        let slot = target.run_slots.alloc();
                        if let Some(entry) = target.entries.get_mut(entry_id) {
                            entry.slot = Some(slot);
                        }
                        slot
                    }
                };
                let row = target.runs[member as usize].presentation.to_gpu();
                if target.run_table.len() <= slot as usize {
                    target
                        .run_table
                        .resize(slot as usize + 1, TextRunGpu::VACANT);
                }
                if target.run_table[slot as usize] != row {
                    target.run_table[slot as usize] = row;
                    target.run_dirty = Some(match target.run_dirty.take() {
                        Some(range) => range.start.min(slot)..range.end.max(slot + 1),
                        None => slot..slot + 1,
                    });
                }
                let Some(entry) = target.entries.get(entry_id) else {
                    break;
                };
                if target.skipped.binary_search(&entry_id).is_ok() {
                    let next = target.runs[member as usize].next;
                    if next == NO_RUN {
                        break;
                    }
                    member = next;
                    continue;
                }
                let capacity = entry.capacity;
                let mut dirty = entry.atlas_epoch != epoch;
                let placed =
                    entry.arena_generation == Some(generation) && entry.arena_capacity == capacity;
                if !placed {
                    if entry.arena_generation == Some(generation) {
                        let (offset, held) = (entry.arena_offset, entry.arena_capacity);
                        target.arena.release(generation, offset, held);
                    }
                    let offset = target.arena.alloc(capacity);
                    let entry = target.entries.get_mut(entry_id).expect("looked up above");
                    entry.indices_stale |=
                        entry.arena_offset != offset || entry.arena_capacity != capacity;
                    entry.arena_offset = offset;
                    entry.arena_capacity = capacity;
                    entry.arena_generation = Some(generation);
                    dirty = true;
                }
                if entry_needs_repair(&target.entries, entry_id, epoch) {
                    // A glyph moved inside the atlas while a later paragraph
                    // was faulting its own in. Rectangles are re-read through
                    // the handles the entry kept; nothing is reshaped,
                    // re-rasterized or re-uploaded.
                    let intact = target
                        .entries
                        .repair(entry_id, epoch, |handle| atlas.entry(handle).copied());
                    target.counters.instance_patches += 1;
                    if !intact && let Some(entry) = target.entries.get_mut(entry_id) {
                        entry.damaged = true;
                    }
                }
                if target.entries.bind_run(entry_id, slot) {
                    target.counters.instance_patches += 1;
                    dirty = true;
                }
                let entry = target.entries.get(entry_id).expect("looked up above");
                let offset = entry.arena_offset;
                if dirty {
                    stage_write(&mut target.writes, target.staging.len(), offset, capacity);
                    let block = target.entries.instances(entry);
                    target.staging.extend_from_slice(block);
                }
                // Exactly the block's size. Room to grow here would let a
                // paragraph lengthen without moving in the draw order, but
                // every spare index is a quad the vertex stage still runs —
                // and on the GPU that costs more than the draws and the
                // index rewrites it would save.
                let ordered = entry.order_generation == Some(order_generation)
                    && range_keeps(entry.order_capacity, capacity);
                if !ordered {
                    if !order_repack {
                        target.order_moves += 1;
                    }
                    if entry.order_generation == Some(order_generation) {
                        let (at, held) = (entry.order_offset, entry.order_capacity);
                        target.order.release(order_generation, at, held);
                        target.order_owners.remove(at);
                        if capacity > held {
                            target.growth_moves += 1;
                        }
                    }
                    let at = target.order.alloc(capacity);
                    target.order_owners.insert(at, entry_id);
                    let entry = target.entries.get_mut(entry_id).expect("looked up above");
                    entry.order_offset = at;
                    entry.order_capacity = capacity;
                    entry.order_generation = Some(order_generation);
                    if order_repack
                        && target.order_gapped
                        && at + capacity - group_start >= GAP_EVERY
                        && let Some(gap) = target.order.reserve_gap(GAP_SLOTS)
                    {
                        stage_write(
                            &mut target.index_writes,
                            target.index_staging.len(),
                            gap,
                            GAP_SLOTS,
                        );
                        target
                            .index_staging
                            .extend(std::iter::repeat_n(VACANT_INDEX, GAP_SLOTS as usize));
                        group_start = gap + GAP_SLOTS;
                    }
                }
                let entry = target.entries.get_mut(entry_id).expect("looked up above");
                let order = entry.order_offset;
                let span = entry.order_capacity;
                // The range names the block slot for slot, slack included:
                // a vacant slot's instance covers nothing, so a draw that
                // spans it paints nothing there.
                if !ordered || entry.indices_stale {
                    entry.indices_stale = false;
                    stage_write(
                        &mut target.index_writes,
                        target.index_staging.len(),
                        order,
                        span,
                    );
                    target.index_staging.extend(offset..offset + capacity);
                    target.index_staging.extend(std::iter::repeat_n(
                        VACANT_INDEX,
                        (span - capacity) as usize,
                    ));
                }
                let entry = target.entries.get(entry_id).expect("looked up above");
                let adjacent = previous_end
                    .is_none_or(|end| end == order || target.order.clean_between(end, order));
                if !adjacent {
                    breaks += 1;
                }
                for segment in &entry.segments {
                    let mut segment = *segment;
                    let t = target.runs[member as usize].presentation.transitions;
                    segment.material = self.gpu.combination([segment.material, t[0], t[1]]);
                    builder.push(&mut target.segments, segment, order, adjacent);
                }
                previous_end = Some(order + span);
                let next = target.runs[member as usize].next;
                if next == NO_RUN {
                    break;
                }
                member = next;
            }
            builder.finish(&mut target.segments);
            target.runs[index].segments = first_segment..target.segments.len() as u32;
        }
        target.order.note_breaks(breaks);
        target.flushed = target.live_runs;
        #[cfg(test)]
        self.audit_placements(target);
    }

    /// What the frame's drawn entries need: their slots in total, the blocks
    /// and ranges the arena and the order do not already hold, and the ranges
    /// whose block outgrew them. Entries in `target.skipped` are not drawn.
    fn measure_runs(
        target: &mut TextPipelineTarget,
        fresh: &mut Vec<u32>,
        fresh_order: &mut Vec<u32>,
        outgrown: &mut Vec<u32>,
    ) -> u32 {
        fresh.clear();
        fresh_order.clear();
        outgrown.clear();
        let mut total = 0u32;
        Self::walk_runs(target, |target, _, entry_id| {
            let Some(entry) = target.entries.get(entry_id) else {
                return;
            };
            if target.skipped.binary_search(&entry_id).is_ok() {
                return;
            }
            let capacity = entry.capacity;
            total = total.saturating_add(capacity);
            if entry.arena_generation != Some(target.arena.generation())
                || entry.arena_capacity != capacity
            {
                fresh.push(capacity);
            }
            let keeps = range_keeps(entry.order_capacity, capacity);
            if entry.order_generation != Some(target.order.generation()) || !keeps {
                fresh_order.push(capacity);
                if entry.order_generation == Some(target.order.generation())
                    && capacity > entry.order_capacity
                {
                    outgrown.push(entry_id);
                }
            }
        });
        total
    }

    /// Leave out the largest drawn entries until the rest need at most two
    /// thirds of `limit` slots. Fills `target.skipped`, sorted.
    ///
    /// Not just until they fit: an arena pinned at the limit has no room for
    /// the next block that moves, and would repack — rewrite every instance —
    /// on every frame. With a third free it repacks as rarely as any other.
    fn skip_largest(target: &mut TextPipelineTarget, total: u32, limit: u32) {
        // Slots per entry, counting an entry two commands draw twice, as the
        // total did.
        let mut drawn: HashMap<u32, u32> = HashMap::new();
        Self::walk_runs(target, |target, _, entry_id| {
            if let Some(entry) = target.entries.get(entry_id) {
                let slots = drawn.entry(entry_id).or_default();
                *slots = slots.saturating_add(entry.capacity);
            }
        });
        let mut largest = drawn.into_iter().collect::<Vec<_>>();
        largest.compact_sort_by_key(|&(entry, slots)| (std::cmp::Reverse(slots), entry));
        let room = limit - limit / 3;
        let mut left = total;
        for (entry, slots) in largest {
            if left <= room {
                break;
            }
            left = left.saturating_sub(slots);
            target.skipped.push(entry);
        }
        target.skipped.sort_unstable();
    }

    /// Let every drawn range whose block outgrew it grow where it is: into
    /// the clean gap right after it, or by shifting the ranges between it and
    /// the next gap along by as much. A range that cannot is left to the walk,
    /// which places it elsewhere.
    ///
    /// Before the walk, so the offsets the walk turns into draws are final. A
    /// range that moves here has its indices rewritten by the walk if it is
    /// drawn this frame, and whenever it is next drawn otherwise.
    fn grow_ranges_in_place(target: &mut TextPipelineTarget, outgrown: &[u32]) {
        const WINDOW: u32 = 4096;
        'entries: for &entry_id in outgrown {
            let Some(entry) = target.entries.get(entry_id) else {
                continue;
            };
            // Listed twice when two commands draw it; the first grew it.
            if entry.capacity <= entry.order_capacity {
                continue;
            }
            let grow = entry.capacity - entry.order_capacity;
            let after = entry.order_offset + entry.order_capacity;
            let mut chain = Vec::new();
            let mut at = after;
            let gap = loop {
                let Some(owner) = target.order_owners.get(at) else {
                    match target.order.free_at(at) {
                        Some((len, true)) if len >= grow => break at,
                        _ => continue 'entries,
                    }
                };
                let Some(next) = target.entries.get(owner) else {
                    continue 'entries;
                };
                chain.push(owner);
                at += next.order_capacity;
                if at - after > WINDOW {
                    continue 'entries;
                }
            };
            target.order.grow_into(gap, grow);
            target.growth_moves += 1;
            target.order_moves += 1;
            for owner in chain.into_iter().rev() {
                let moved = target.entries.get_mut(owner).expect("owned");
                target.order_owners.remove(moved.order_offset);
                moved.order_offset += grow;
                moved.indices_stale = true;
                target.order_owners.insert(moved.order_offset, owner);
            }
            let entry = target.entries.get_mut(entry_id).expect("looked up above");
            entry.order_capacity = entry.capacity;
            entry.indices_stale = true;
        }
    }

    /// Every rectangle a drawn entry samples is the one its handle names now.
    ///
    /// Recomputed from the atlas rather than trusted from the epoch stamps,
    /// so every test that draws text also tests the rule that decides when a
    /// retained instance has to be repaired: an entry stamped current while
    /// holding a rectangle the atlas has since moved would sample some other
    /// glyph, and no counter would say so.
    #[cfg(test)]
    fn audit_placements(&self, target: &TextPipelineTarget) {
        for run in &target.runs[..target.live_runs] {
            let Some(entry) = target.entries.get(run.entry) else {
                continue;
            };
            if target.skipped.binary_search(&run.entry).is_ok() {
                continue;
            }
            for (index, (instance, handle)) in target.entries.live_glyphs(entry).enumerate() {
                let Some(drawn) = instance.placement().filter(|_| !instance.is_solid()) else {
                    continue;
                };
                let current = self
                    .atlas
                    .entry(handle)
                    .map(|placed| (placed.origin, placed.size));
                assert_eq!(
                    Some(drawn),
                    current,
                    "glyph {index} of entry {} samples a rectangle its handle no \
                     longer names (entry epoch {}, atlas epoch {})",
                    run.entry,
                    entry.atlas_epoch,
                    self.placement_epoch(),
                );
            }
        }
    }

    /// Visit every run of every open command, in draw order.
    fn walk_runs(
        target: &mut TextPipelineTarget,
        mut visit: impl FnMut(&mut TextPipelineTarget, usize, u32),
    ) {
        for index in 0..target.live_runs {
            if target.runs[index].folded {
                continue;
            }
            let mut member = index as u32;
            loop {
                let entry = target.runs[member as usize].entry;
                visit(target, index, entry);
                let next = target.runs[member as usize].next;
                if next == NO_RUN {
                    break;
                }
                member = next;
            }
        }
    }

    /// [`Self::upload_with`] in an encoder of its own, submitted at once:
    /// for tests that draw without a painter around them.
    #[cfg(test)]
    pub(super) fn upload(
        &mut self,
        target: &mut TextPipelineTarget,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui.scene.text.upload"),
        });
        self.upload_with(target, device, queue, &mut encoder, work);
        queue.submit([encoder.finish()]);
    }

    /// Write this frame's atlas regions, arena blocks, index ranges and
    /// presentation tables. Blocks and ranges are copied in `encoder`, so it
    /// must be the one the passes that draw them are recorded in, or one
    /// submitted before it.
    pub(super) fn upload_with(
        &mut self,
        target: &mut TextPipelineTarget,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let dual_source = target.subpixel != GlyphRenderMode::Mask;
        if dual_source {
            self.gpu.ensure_subpixel(device);
        }
        self.gpu
            .prepare_materials(device, queue, &self.atlas, work, dual_source);
        let ordered = self.follows_in_frame.then_some(&mut *encoder);
        self.ordered_flushed |= self
            .uploads
            .flush(device, queue, &self.atlas, work, ordered);
        // Bind groups are created here, where the atlas is still `&mut`, so
        // `draw` only has to look one up. Nearly every frame's segments name
        // the same pair, so the pair is only looked up when it changes.
        let mut last = None;
        for segment in target.segments.iter() {
            let pair = (segment.mask_page, segment.color_page);
            if last != Some(pair) {
                last = Some(pair);
                self.atlas.bind_group(device, pair.0, pair.1);
            }
        }
        let bytes = self.gpu.upload(
            device,
            queue,
            encoder,
            &mut target.gpu,
            &FrameUpload {
                physical_size: target.physical_size,
                arena_capacity: target.arena.capacity(),
                writes: &target.writes,
                staging: &target.staging,
                order_capacity: target.order.capacity(),
                index_writes: &target.index_writes,
                index_staging: &target.index_staging,
                runs: &target.run_table,
                run_dirty: target.run_dirty.clone(),
                presentations: &target.presentations,
                uploaded_presentations: &target.uploaded_presentations,
                fx: &target.fx,
                uploaded_fx: &target.uploaded_fx,
                corner_exponent: target.corner_exponent,
            },
            work,
        );
        if bytes.lost {
            // Nothing this frame placed reached the GPU. Forget every
            // placement, so the next frame places and writes everything it
            // draws again instead of trusting buffers that never got it.
            target.arena.reset();
            target.entries.invalidate_arena();
            target.forget_order();
        }
        target.counters.instance_upload_bytes += bytes.instances as u64;
        target.counters.index_upload_bytes += bytes.indices as u64;
        target.counters.presentation_upload_bytes += bytes.presentation as u64;
        target.frame_gpu_allocations += target.gpu.take_allocations();
        #[cfg(test)]
        Self::audit_draw_order(target);
    }

    /// Every index a draw spans resolves, through what the GPU buffers hold,
    /// to the instance its entry holds, and every glyph an entry holds is under
    /// a draw.
    #[cfg(test)]
    fn audit_draw_order(target: &mut TextPipelineTarget) {
        let shadow = &mut target.shadow;
        // A replaced buffer starts zeroed, which is what a vacant glyph is.
        let capacity = target.arena.capacity() as usize;
        if capacity != 0 && capacity != shadow.instances.len() {
            shadow.instances = vec![GlyphInstance::VACANT; capacity];
        }
        let capacity = target.order.capacity() as usize;
        if capacity != 0 && capacity != shadow.indices.len() {
            shadow.indices = vec![0; capacity];
        }
        for write in &target.writes {
            let staged = &target.staging[write.staged.start as usize..write.staged.end as usize];
            shadow.instances[write.offset as usize..][..staged.len()].copy_from_slice(staged);
        }
        for write in &target.index_writes {
            let staged =
                &target.index_staging[write.staged.start as usize..write.staged.end as usize];
            shadow.indices[write.offset as usize..][..staged.len()].copy_from_slice(staged);
        }
        for head in 0..target.live_runs {
            if target.runs[head].folded {
                continue;
            }
            let mut expected = HashMap::new();
            let mut glyphs = Vec::new();
            let mut member = head as u32;
            loop {
                let run = &target.runs[member as usize];
                if let Some(entry) = target
                    .entries
                    .get(run.entry)
                    .filter(|_| target.skipped.binary_search(&run.entry).is_err())
                {
                    let block = target.entries.instances(entry);
                    assert!(
                        entry.order_capacity >= entry.capacity,
                        "entry {}'s index range is smaller than its block",
                        run.entry
                    );
                    for index in 0..entry.order_capacity {
                        let want = block.get(index as usize).copied();
                        let previous =
                            expected.insert(entry.order_offset + index, (run.entry, want));
                        assert!(
                            previous.is_none(),
                            "two entries of one command share index {}",
                            entry.order_offset + index
                        );
                    }
                    for segment in &entry.segments {
                        glyphs.extend(
                            (segment.first..segment.first + segment.count)
                                .map(|index| entry.order_offset + index),
                        );
                    }
                }
                if run.next == NO_RUN {
                    break;
                }
                member = run.next;
            }
            let run = &target.runs[head];
            let drawn = &target.segments[run.segments.start as usize..run.segments.end as usize];
            for segment in drawn {
                for position in segment.first..segment.first + segment.count {
                    let slot = shadow.indices[position as usize];
                    let Some((entry, want)) = expected.get(&position).copied() else {
                        assert!(
                            target.order.clean_at(position),
                            "a draw spans index {position}, which no entry of its command \
                             owns and no gap covers"
                        );
                        assert_eq!(slot, VACANT_INDEX, "gap index {position} must be vacant");
                        continue;
                    };
                    match want {
                        None => assert_eq!(
                            slot, VACANT_INDEX,
                            "index {position} runs past entry {entry}'s block and must be vacant"
                        ),
                        Some(want) => assert_eq!(
                            shadow.instances.get(slot as usize).copied(),
                            Some(want),
                            "index {position} of entry {entry} names slot {slot}, which does \
                             not hold the glyph the entry holds there"
                        ),
                    }
                }
            }
            for position in glyphs {
                assert!(
                    drawn
                        .iter()
                        .any(|segment| (segment.first..segment.first + segment.count)
                            .contains(&position)),
                    "index {position} holds a glyph no draw spans"
                );
            }
        }
    }

    pub(super) fn draw(
        &self,
        target: &TextPipelineTarget,
        pass: &mut wgpu::RenderPass<'_>,
        prepared: &PreparedText,
        scissor: PhysicalRect,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let Some(run) = target
            .runs
            .get(prepared.index)
            .filter(|_| prepared.index < target.live_runs)
        else {
            return;
        };
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        let start = run.segments.start as usize;
        let end = (run.segments.end as usize).min(target.segments.len());
        let dual_source = target.subpixel != GlyphRenderMode::Mask;
        let mut drawn = 0u64;
        let mut slots = 0u64;
        for segment in &target.segments[start.min(end)..end] {
            let Some(bind_group) = self
                .atlas
                .cached_bind_group(segment.mask_page, segment.color_page)
            else {
                continue;
            };
            self.gpu
                .draw_segment(pass, &target.gpu, segment, bind_group, dual_source);
            drawn += 1;
            slots += u64::from(segment.count);
        }
        self.gpu.charge_materials(
            &target.segments[start.min(end)..end],
            run.pixels
                .min(u64::from(scissor.width) * u64::from(scissor.height)),
        );
        self.draws.set(self.draws.get() + drawn);
        self.slots_drawn.set(self.slots_drawn.get() + slots);
        if let Some(work) = gpu_work {
            work.record_draw_batch();
            for _ in 0..drawn {
                work.record_draw_call();
            }
        }
    }

    /// A window or viewport closed. Its entries' claims on atlas glyphs end
    /// here — the glyphs stay placed for the windows still open, they just
    /// stop being counted as in use by one that is gone — and its counters
    /// are kept so the totals do not run backwards.
    pub(super) fn close_target(&mut self, mut target: TextPipelineTarget) {
        let atlas = &mut self.atlas;
        target.entries.clear(|handle| atlas.release(handle));
        self.closed.add(target.counters());
    }
}

/// Where the order leaves a gap when it repacks around text that keeps
/// growing: after every [`GAP_EVERY`] slots, [`GAP_SLOTS`] of room. Measured
/// on ten thousand labels changing length (#224): a quarter of room per 512
/// slots took the draws from 90 to 14 and the index uploads to a third; an
/// eighth, or a group half the size, left two to three times the draws.
const GAP_EVERY: u32 = 512;
const GAP_SLOTS: u32 = 128;

/// Frames a target draws without a range outgrowing its place before a
/// layout with gaps is repacked without them (#230).
///
/// Counted in frames drawn, not in time, because that is what the gaps cost
/// by: ten thousand labels whose churn stopped drew 152 085 quads a frame
/// across their gaps and 136 045 without, and their GPU time went from 2.86
/// to 2.72 ms. The repack that gives the gaps back rewrites 544 KB of
/// indices, which did not stand out of the frame-to-frame noise, so a few
/// frames of gaps already pay for it — and for the repack that leaves gaps
/// again should the text resume growing. A second of frames at 60 Hz is
/// several times that, so text that pauses between bursts keeps its room.
pub(super) const SETTLE_FRAMES: u32 = 60;

/// Which entry's index range starts at each offset of the order. Flat, so
/// placing a whole frame's worth of ranges after a repack costs a store per
/// range rather than a tree insert.
///
/// Kept only while the order has gaps (`clear(Some(capacity))`); otherwise
/// nothing asks, and every call is free.
#[derive(Default)]
struct OrderOwners {
    kept: bool,
    owners: Vec<u32>,
}

impl OrderOwners {
    fn clear(&mut self, capacity: Option<u32>) {
        self.kept = capacity.is_some();
        self.owners.clear();
        self.owners.resize(capacity.unwrap_or(0) as usize, u32::MAX);
    }

    fn insert(&mut self, offset: u32, entry: u32) {
        if !self.kept {
            return;
        }
        let offset = offset as usize;
        if self.owners.len() <= offset {
            self.owners.resize(offset + 1, u32::MAX);
        }
        self.owners[offset] = entry;
    }

    fn remove(&mut self, offset: u32) {
        if let Some(owner) = self.owners.get_mut(offset as usize) {
            *owner = u32::MAX;
        }
    }

    fn get(&self, offset: u32) -> Option<u32> {
        self.owners
            .get(offset as usize)
            .copied()
            .filter(|owner| *owner != u32::MAX)
    }
}

/// Queue `len` slots at `offset`, staged from `staged` on: folded into the
/// last write when it ends exactly there, so a walk that places blocks one
/// after another turns into one write.
fn stage_write(writes: &mut Vec<ArenaWrite>, staged: usize, offset: u32, len: u32) {
    let staged = staged as u32;
    if let Some(last) = writes.last_mut()
        && last.offset + (last.staged.end - last.staged.start) == offset
    {
        last.staged.end += len;
        return;
    }
    writes.push(ArenaWrite {
        offset,
        staged: staged..staged + len,
    });
}

impl RunPresentation {
    fn to_gpu(self) -> TextRunGpu {
        TextRunGpu::new(
            self.origin,
            self.presentation,
            self.flags,
            self.color,
            self.opacity,
            self.raster,
            self.fx,
        )
    }
}

/// Whether `id`'s rectangles predate the atlas's current placement epoch.
fn entry_needs_repair(entries: &EntryStore, id: u32, epoch: u64) -> bool {
    entries
        .get(id)
        .is_some_and(|entry| entry.atlas_epoch != epoch)
}

#[cfg(test)]
mod tests;
