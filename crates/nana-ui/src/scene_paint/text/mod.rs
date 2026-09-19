//! `NanaRenderer::text`: the renderer's own glyph subsystem.
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
//! Nothing below the resolver knows what shaped the paragraph. Today it is
//! this crate's cosmic-text shaper; #99 replaces that with `nana-text`'s
//! engine by rewriting [`TextPipeline::resolve_runs`] and the rasterizer's
//! face source, and every stage after it is unchanged.
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
mod entry;
mod glyph;
mod pipeline;
mod raster;
mod raster_cache;
mod upload;

use cosmic_text::{Align, Buffer, Color, Metrics, Shaping};
use nana_ui_core::LineHeightSpec;
use nana_ui_runtime::{TextHorizontalAlignment, TextShaping, TextVerticalAlignment};
use nana_ui_scene::{SceneTextOpenType, SceneTextSpan};
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::ops::Range;

use self::atlas::{AtlasPageKind, GlyphAtlasLimits, GlyphAtlasManager};
pub(in crate::scene_paint) use self::entry::EntryKey;
use self::entry::{EntrySegment, EntryStore, InstanceArena, RunSlots, SegmentBuilder};
use self::glyph::{GlyphRenderMode, NanaGlyphBuffer, PlacedGlyph, size_bits};
use self::pipeline::{
    ArenaWrite, CONTENT_COLOR, CONTENT_MASK, DrawSegment, FrameUpload, GlyphInstance, TextGpu,
    TextPresentationGpu, TextRunGpu, TextTargetGpu,
};
use self::raster::{SwashGlyphRasterizer, synthesis_from_backend};
use self::raster_cache::GlyphRasterCache;
use self::upload::GlyphUploadQueue;

use super::clip::{self, LogicalRect};
use super::color::{linear_from_srgb8, to_rgba8};
use crate::PhysicalRect;
use crate::nana_text::{
    RTL_ISOLATE_PREFIX, RTL_ISOLATE_SUFFIX, cosmic_wrap, ellipsize_end, measured_text_overflows,
    shape_attrs, wrap_for_css_direction,
};

const SHAPE_CACHE_CAP: usize = 512;
/// Hard ceiling on shaped paragraphs, whatever the view asks for.
const SHAPE_CACHE_MAX: usize = 32_768;
/// Frames a paragraph stays un-evictable after it was last drawn. Two, so a
/// painter alternating between two windows holds both of their text.
const PIN_FRAMES: u64 = 2;
/// How far along the LRU order one insert looks for a paragraph nothing wants.
/// The front is the oldest; if the oldest this many are all still on screen,
/// so is everything behind them.
const EVICT_PROBES: usize = 64;
/// Frames between retirement sweeps, and how long an entry survives without
/// being drawn. A tab switch that flips back and forth must not pay for
/// either direction.
/// What a caller passes when it cannot say whether the primitive changed.
///
/// Such a frame assembles the shape key and hashes the paragraph, which is
/// what every frame used to do. It never matches a retained entry, including
/// one built by another untracked call.
pub(super) const UNTRACKED_REVISION: u64 = u64::MAX;

/// What a presentation row is derived from: the paint transform, its
/// perspective, the fragment clip and the device scale.
type PresentationInputs = ([f32; 6], [f32; 2], clip::FragmentClip, u32);

const RETIRE_INTERVAL: u64 = 64;
const RETIRE_AFTER_FRAMES: u64 = 240;

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
    /// The run and presentation tables, which is where a move, a fade or a
    /// recolor lands instead of in the instances.
    pub text_presentation_upload_bytes: u64,
    pub text_prepare_nodes_considered: u64,
    /// Nodes that reached a draw without resolving a glyph.
    pub text_prepare_nodes_skipped: u64,
    /// Nodes that could not reach a pixel, so they cost no entry and no draw.
    pub text_prepare_nodes_culled: u64,
}

struct ShapeEntry {
    key: ShapeKey,
    buffer: Buffer,
    /// Frame this paragraph was last asked for.
    last_used: u64,
}

/// Shaped buffers keyed by [`ShapeKeyRef::hash64`].
///
/// The map is keyed by the hash rather than by an owned key so a repaint of
/// unchanged text looks up without copying the string, the family name, or the
/// rich-span list. The stored key still decides the hit, so a hash collision
/// between two different texts reshapes instead of painting the wrong glyphs.
#[derive(Default)]
struct ShapeCache {
    /// Keyed by the shape hash, hashed by [`nana_ui_runtime::IdHasher`]:
    /// running SipHash over a word that is already a hash is work for nothing.
    entries: HashMap<u64, ShapeEntry, nana_ui_runtime::BuildIdHasher>,
    order: VecDeque<u64>,
    frame: u64,
    hits: usize,
    misses: usize,
    evictions: usize,
}

impl ShapeCache {
    fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    fn get(&mut self, hash: u64, key: &ShapeKeyRef<'_>) -> Option<&Buffer> {
        let frame = self.frame;
        match self.entries.get_mut(&hash) {
            Some(entry) if entry.key.matches(key) => {
                entry.last_used = frame;
                self.hits += 1;
                Some(&entry.buffer)
            }
            _ => {
                self.misses += 1;
                None
            }
        }
    }

    /// Whether the cache still holds `hash`, counted and aged as a hit.
    ///
    /// The caller that uses this already knows the paragraph is the one it
    /// resolved from — the primitive has not been rewritten since — so the key
    /// comparison would only re-derive an answer it has.
    fn holds(&mut self, hash: u64) -> bool {
        let frame = self.frame;
        match self.entries.get_mut(&hash) {
            Some(entry) => {
                entry.last_used = frame;
                self.hits += 1;
                true
            }
            None => {
                self.misses += 1;
                false
            }
        }
    }

    fn buffer(&self, hash: u64) -> Option<&Buffer> {
        self.entries.get(&hash).map(|entry| &entry.buffer)
    }

    /// Drop every shaped paragraph. For the case where they are not merely
    /// cold but no longer meaningful, i.e. the face set changed under them.
    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    /// Store a shaped paragraph, evicting the coldest paragraph nothing on
    /// screen still wants.
    ///
    /// A paragraph a recent frame asked for is never the victim. A view with
    /// more distinct strings on it than [`SHAPE_CACHE_CAP`] would otherwise
    /// evict the row it is about to draw to make room for the next one, and
    /// reshape every one of them on every frame — the cache would turn a
    /// steady frame into the most expensive kind there is. So the nominal
    /// capacity is a floor the cache shrinks back to once the view does, not a
    /// ceiling it enforces against the view in front of it.
    ///
    /// "Recent" rather than "this frame" because two windows on one painter
    /// take turns: what window A drew last frame is still on screen while
    /// window B is drawing, and the cache has to hold both.
    fn insert(&mut self, hash: u64, key: ShapeKey, buffer: Buffer) {
        let frame = self.frame;
        let pinned = frame.saturating_sub(PIN_FRAMES);
        let mut probes = 0;
        while self.entries.len() >= SHAPE_CACHE_CAP && probes < EVICT_PROBES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            probes += 1;
            match self.entries.get(&oldest) {
                Some(entry) if entry.last_used >= pinned => {
                    // Still on a screen. Put it back and look further along.
                    self.order.push_back(oldest);
                }
                Some(_) => {
                    self.entries.remove(&oldest);
                    self.evictions += 1;
                    probes = 0;
                }
                None => probes = 0,
            }
        }
        // A hard ceiling, in case a single view really does hold this much
        // text: the alternative is a cache that remembers every paragraph a
        // session ever drew.
        while self.entries.len() >= SHAPE_CACHE_MAX {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if self.entries.remove(&oldest).is_some() {
                self.evictions += 1;
            }
        }
        if self
            .entries
            .insert(
                hash,
                ShapeEntry {
                    key,
                    buffer,
                    last_used: frame,
                },
            )
            .is_none()
        {
            self.order.push_back(hash);
        }
    }
}

/// FNV-1a over the shape key.
///
/// Not a general-purpose hasher and deliberately not the default one: this
/// runs once per text node per frame over the whole string, and it is the
/// single largest item left in the text path. What guards against a collision
/// is [`ShapeKey::matches`], which decides the hit — a collision costs a
/// reshape, never the wrong glyphs.
#[derive(Default)]
struct ShapeHasher(u64);

impl Hasher for ShapeHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut state = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for byte in bytes {
            state ^= u64::from(*byte);
            state = state.wrapping_mul(0x1000_0000_01b3);
        }
        self.0 = state;
    }
}

/// Everything that determines the shaped output. Position is applied at draw
/// time via `TextArea` and plain-text color via `default_color`, so neither
/// is part of the key; rich spans bake their colors into shaping attrs and
/// therefore belong to it.
struct ShapeKey {
    content: String,
    family: Option<String>,
    weight: Option<u16>,
    font_size_bits: u32,
    line_height_bits: u32,
    wrap: bool,
    wrap_break: nana_ui_core::TextWrapBreak,
    italic: bool,
    ellipsis: bool,
    max_lines: Option<u16>,
    shaping: u8,
    letter_spacing_bits: u32,
    word_break: u8,
    line_break: u8,
    kerning: u8,
    features: Vec<nana_ui_core::FontFeatureSetting>,
    variations: Vec<nana_ui_core::FontVariationSetting>,
    width_bits: u32,
    height_bits: u32,
    align: u8,
    direction: u8,
    writing_mode: u8,
    spans: Option<Vec<(String, [u32; 4])>>,
    font_features: Vec<nana_ui_core::FontFeatureSetting>,
}

/// Borrowed form of [`ShapeKey`] built straight from the scene primitive.
///
/// Hashing and comparison live here so a frame can answer "already shaped?"
/// without owning any of it.
struct ShapeKeyRef<'a> {
    content: &'a str,
    family: Option<&'a str>,
    weight: Option<u16>,
    font_size_bits: u32,
    line_height_bits: u32,
    wrap: bool,
    wrap_break: nana_ui_core::TextWrapBreak,
    italic: bool,
    ellipsis: bool,
    max_lines: Option<u16>,
    shaping: u8,
    letter_spacing_bits: u32,
    word_break: u8,
    line_break: u8,
    kerning: u8,
    features: &'a [nana_ui_core::FontFeatureSetting],
    variations: &'a [nana_ui_core::FontVariationSetting],
    width_bits: u32,
    height_bits: u32,
    align: u8,
    direction: u8,
    writing_mode: u8,
    /// `Some` only for rich text, whose span colors change the shaped attrs.
    spans: Option<&'a [(&'a str, [f32; 4])]>,
    font_features: &'a [nana_ui_core::FontFeatureSetting],
}

impl ShapeKeyRef<'_> {
    fn hash64(&self) -> u64 {
        let mut hasher = ShapeHasher::default();
        self.content.hash(&mut hasher);
        self.family.hash(&mut hasher);
        self.weight.hash(&mut hasher);
        self.font_size_bits.hash(&mut hasher);
        self.line_height_bits.hash(&mut hasher);
        self.wrap.hash(&mut hasher);
        self.wrap_break.hash(&mut hasher);
        self.italic.hash(&mut hasher);
        self.ellipsis.hash(&mut hasher);
        self.max_lines.hash(&mut hasher);
        self.shaping.hash(&mut hasher);
        self.letter_spacing_bits.hash(&mut hasher);
        self.word_break.hash(&mut hasher);
        self.line_break.hash(&mut hasher);
        self.kerning.hash(&mut hasher);
        self.features.hash(&mut hasher);
        self.variations.hash(&mut hasher);
        self.width_bits.hash(&mut hasher);
        self.height_bits.hash(&mut hasher);
        self.align.hash(&mut hasher);
        self.font_features.hash(&mut hasher);
        self.direction.hash(&mut hasher);
        self.writing_mode.hash(&mut hasher);
        match self.spans {
            None => 0u8.hash(&mut hasher),
            Some(spans) => {
                1u8.hash(&mut hasher);
                spans.len().hash(&mut hasher);
                for (text, color) in spans {
                    text.hash(&mut hasher);
                    color.map(f32::to_bits).hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }

    fn to_owned_key(&self) -> ShapeKey {
        ShapeKey {
            content: self.content.to_owned(),
            family: self.family.map(str::to_owned),
            weight: self.weight,
            font_size_bits: self.font_size_bits,
            line_height_bits: self.line_height_bits,
            wrap: self.wrap,
            wrap_break: self.wrap_break,
            italic: self.italic,
            ellipsis: self.ellipsis,
            max_lines: self.max_lines,
            shaping: self.shaping,
            letter_spacing_bits: self.letter_spacing_bits,
            word_break: self.word_break,
            line_break: self.line_break,
            kerning: self.kerning,
            features: self.features.to_vec(),
            variations: self.variations.to_vec(),
            width_bits: self.width_bits,
            height_bits: self.height_bits,
            align: self.align,
            font_features: self.font_features.to_vec(),
            direction: self.direction,
            writing_mode: self.writing_mode,
            spans: self.spans.map(|spans| {
                spans
                    .iter()
                    .map(|(text, color)| ((*text).to_owned(), color.map(f32::to_bits)))
                    .collect()
            }),
        }
    }
}

impl ShapeKey {
    fn matches(&self, other: &ShapeKeyRef<'_>) -> bool {
        self.content == other.content
            && self.family.as_deref() == other.family
            && self.weight == other.weight
            && self.font_size_bits == other.font_size_bits
            && self.line_height_bits == other.line_height_bits
            && self.wrap == other.wrap
            && self.wrap_break == other.wrap_break
            && self.italic == other.italic
            && self.ellipsis == other.ellipsis
            && self.max_lines == other.max_lines
            && self.shaping == other.shaping
            && self.letter_spacing_bits == other.letter_spacing_bits
            && self.word_break == other.word_break
            && self.line_break == other.line_break
            && self.kerning == other.kerning
            && self.features == other.features
            && self.variations == other.variations
            && self.width_bits == other.width_bits
            && self.height_bits == other.height_bits
            && self.align == other.align
            && self.font_features == other.font_features
            && self.direction == other.direction
            && self.writing_mode == other.writing_mode
            && match (&self.spans, other.spans) {
                (None, None) => true,
                (Some(mine), Some(theirs)) => {
                    mine.len() == theirs.len()
                        && mine
                            .iter()
                            .zip(theirs)
                            .all(|((text, color), (other, hue))| {
                                text == other && *color == hue.map(f32::to_bits)
                            })
                }
                _ => false,
            }
    }
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
    presentation_index: HashMap<[u32; 40], u32>,
    /// The presentation the label before this one asked for, by the values it
    /// was derived from. See [`TextPipeline::presentation_index`].
    last_presentation: Option<(PresentationInputs, u32)>,
    /// Where each entry's block sits in this target's instance buffer. The
    /// bytes come straight from the entry that owns the block; only the
    /// offsets live here.
    arena: InstanceArena,
    /// Blocks whose arena bytes are no longer what the GPU holds, coalesced
    /// into as few writes as the draw order allows.
    writes: Vec<ArenaWrite>,
    staging: Vec<GlyphInstance>,
    physical_size: [u32; 2],
    frame: u64,
    frame_gpu_allocations: usize,
    instance_rebuilds: u64,
    instance_patches: u64,
    instance_upload_bytes: u64,
    presentation_upload_bytes: u64,
    nodes_considered: u64,
    nodes_skipped: u64,
    nodes_culled: u64,
}

impl TextPipelineTarget {
    fn new(gpu: TextTargetGpu) -> Self {
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
            presentation_index: HashMap::new(),
            last_presentation: None,
            arena: InstanceArena::default(),
            writes: Vec::new(),
            staging: Vec::new(),
            physical_size: [0; 2],
            frame: 0,
            frame_gpu_allocations: 0,
            instance_rebuilds: 0,
            instance_patches: 0,
            instance_upload_bytes: 0,
            presentation_upload_bytes: 0,
            nodes_considered: 0,
            nodes_skipped: 0,
            nodes_culled: 0,
        }
    }
}

pub(super) struct TextPipeline {
    /// Shared with Runtime shaping; see [`crate::nana_text::nana_font_system`].
    font_system: crate::nana_text::SharedFontSystem,
    rasterizer: SwashGlyphRasterizer,
    raster: GlyphRasterCache,
    atlas: GlyphAtlasManager,
    uploads: GlyphUploadQueue,
    gpu: TextGpu,
    /// Shaped paragraphs reused across frames. Shaping is the dominant CPU
    /// cost of a text-heavy frame and identical text+style+box repeats on
    /// every repaint (scroll, hover, unrelated animations), so the shaped
    /// `Buffer` is cached and only glyph placement is redone per frame.
    shape_cache: ShapeCache,
    /// Resolver scratch, reused so a paragraph's runs cost no allocation.
    resolved: NanaGlyphBuffer,
    target: TextPipelineTarget,
    /// The font-set generation this painter's caches were filled at. A
    /// `@font-face` registration reissues faces, so both the shaped paragraphs
    /// and the glyph bitmaps stop meaning what they meant.
    font_generation: u64,
    resolve_requests: u64,
    draws: Cell<u64>,
}

impl TextPipeline {
    pub(super) fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self::with_atlas_limits(device, format, GlyphAtlasLimits::default())
    }

    fn with_atlas_limits(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        limits: GlyphAtlasLimits,
    ) -> Self {
        let raster = GlyphRasterCache::default();
        let atlas = GlyphAtlasManager::new(device, raster.generation(), limits);
        let gpu = TextGpu::new(device, format, &atlas);
        let target = TextPipelineTarget::new(gpu.new_target(device));
        Self {
            font_system: crate::nana_text::nana_font_system(),
            rasterizer: SwashGlyphRasterizer::new(crate::nana_text::nana_font_system()),
            raster,
            atlas,
            uploads: GlyphUploadQueue::default(),
            gpu,
            shape_cache: ShapeCache::default(),
            resolved: NanaGlyphBuffer::default(),
            target,
            font_generation: crate::nana_text::font_db_generation(),
            resolve_requests: 0,
            draws: Cell::new(0),
        }
    }

    pub(super) fn begin_frame(&mut self, physical_size: [u32; 2]) {
        let generation = crate::nana_text::font_db_generation();
        if generation != self.font_generation {
            // Faces were added, replaced or removed. Shaped paragraphs named
            // the old face set and glyph bitmaps were scaled from it, so both
            // are dropped rather than left to be keyed around — and with them
            // every entry, whose instances were resolved through those ids.
            self.font_generation = generation;
            self.shape_cache.clear();
            self.raster.invalidate();
            self.drop_every_entry();
        }
        self.atlas.begin_frame(self.raster.generation());
        self.shape_cache.begin_frame();
        let target = &mut self.target;
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
        target.presentation_index.clear();
        target.last_presentation = None;
        target.writes.clear();
        target.staging.clear();
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
                ..
            } = target;
            entries.retire(
                horizon,
                |handle| atlas.release(handle),
                |slot| run_slots.release(slot),
                |generation, offset, capacity| arena.release(generation, offset, capacity),
            );
        }
    }

    /// Drop every retained entry and everything it held.
    ///
    /// The blocks and run rows go back too: without that, the arena and the
    /// run table would grow past every `@font-face` registration a session
    /// sees, because the entries that owned them are gone and can no longer
    /// give them back.
    fn drop_every_entry(&mut self) {
        let atlas = &mut self.atlas;
        self.target.entries.clear(|handle| atlas.release(handle));
        self.target.arena.reset();
        self.target.run_slots.reset();
        self.target.run_table.clear();
        self.target.run_dirty = None;
    }

    /// Bumped whenever a placement moved or died. A render target that kept
    /// draw commands from an earlier frame must rebuild them when this
    /// changes: its instances name rectangles that are no longer that glyph's.
    pub(super) fn placement_epoch(&self) -> u64 {
        let counters = self.atlas.counters();
        counters.evictions.wrapping_add(counters.relocations)
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

    pub(super) fn glyph_counters(&self) -> TextGlyphCounters {
        let raster = self.raster.counters();
        let atlas = self.atlas.counters();
        let uploads = self.uploads.counters();
        let entries = self.target.entries.counters();
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
            text_gpu_entries_active: entries.active,
            text_gpu_entries_created: entries.created,
            text_gpu_entries_destroyed: entries.destroyed,
            text_gpu_entries_reused: entries.reused,
            text_gpu_entry_glyphs: entries.glyphs,
            text_instance_rebuilds: self.target.instance_rebuilds,
            text_instance_patches: self.target.instance_patches,
            text_instance_upload_bytes: self.target.instance_upload_bytes,
            text_presentation_upload_bytes: self.target.presentation_upload_bytes,
            text_prepare_nodes_considered: self.target.nodes_considered,
            text_prepare_nodes_skipped: self.target.nodes_skipped,
            text_prepare_nodes_culled: self.target.nodes_culled,
        }
    }

    /// GPU allocations this frame's text could not reuse.
    pub(super) fn take_frame_gpu_allocations(&mut self) -> usize {
        std::mem::take(&mut self.target.frame_gpu_allocations)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
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
    ) -> Option<PreparedText> {
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
        // Shape in physical px so the raster size is the shaped size.
        let physical_size = size * scale;
        let physical_line_height = line_height * scale;
        let physical_width = bounds.width.max(0.0) * scale;
        let physical_height = bounds.height.max(line_height) * scale;
        // Opacity rides on the run, not on the color the glyphs were
        // resolved with: a fade must not be a reason to reshape rich text or
        // to rebuild a single instance.
        let default_color = color.unwrap_or([0.0, 0.0, 0.0, 1.0]);
        // Match NanaTextShaper's measurement policy, including ASCII. Basic
        // shaping changes advances and can wrap/truncate text that fits the
        // Runtime content box (notably multiline chart tooltips).
        let shaping = match shaping {
            TextShaping::Auto | TextShaping::Advanced => Shaping::Advanced,
        };
        let rtl = opentype.direction.is_rtl();
        let align = match horizontal {
            TextHorizontalAlignment::Start if rtl => Some(Align::Right),
            TextHorizontalAlignment::Start => None,
            TextHorizontalAlignment::Center => Some(Align::Center),
            TextHorizontalAlignment::End if rtl => None,
            TextHorizontalAlignment::End => Some(Align::Right),
        };
        // Nothing the shape key is made of can have changed: the scene has not
        // rewritten this primitive since these glyphs were resolved, and
        // neither the device scale nor the font set has moved. Assembling the
        // key and hashing the paragraph would only prove that again, once per
        // label per frame.
        //
        // Rich text is left out: its key carries the painted spans, which are
        // built against a colour the caller may override without the scene
        // having touched the primitive.
        let retained = spans
            .is_empty()
            .then(|| self.target.entries.lookup(entry_key))
            .flatten()
            .and_then(|id| Some((id, self.target.entries.get(id)?)))
            .filter(|(_, entry)| {
                !entry.damaged
                    && revision != UNTRACKED_REVISION
                    && entry.revision == revision
                    && entry.scale_bits == scale.to_bits()
                    && entry.font_generation == self.font_generation
            })
            .map(|(id, entry)| (id, entry.layout, entry.measured))
            .filter(|(_, hash, _)| self.shape_cache.holds(*hash));
        let hash = match retained {
            Some((_, hash, _)) => hash,
            None => {
                // A label with no spans is the overwhelming majority, and splitting it
                // would allocate a one-element list per node per frame to say so.
                let painted = if spans.is_empty() {
                    Vec::new()
                } else {
                    presentation_spans(content, spans, default_color)
                };
                let rich = painted.len() > 1
                    || painted.first().is_some_and(|span| span.1 != default_color);
                // Width, height and requested ellipsis uniquely determine the result;
                // cache lookup before shaping avoids repeating the overflow probe.
                let key = ShapeKeyRef {
                    content,
                    family,
                    weight,
                    font_size_bits: physical_size.to_bits(),
                    line_height_bits: physical_line_height.to_bits(),
                    wrap,
                    wrap_break,
                    italic,
                    ellipsis,
                    max_lines,
                    shaping: match shaping {
                        Shaping::Basic => 0,
                        Shaping::Advanced => 1,
                    },
                    letter_spacing_bits: letter_spacing.to_bits(),
                    word_break: opentype_disc(opentype.word_break),
                    line_break: opentype_line_disc(opentype.line_break),
                    kerning: opentype_kern_disc(opentype.kerning),
                    features: &opentype.features,
                    variations: &opentype.variations,
                    width_bits: physical_width.to_bits(),
                    height_bits: physical_height.to_bits(),
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
                    spans: rich.then_some(painted.as_slice()),
                    font_features,
                };
                let hash = key.hash64();
                if self.shape_cache.get(hash, &key).is_none() {
                    let mut fonts = crate::nana_text::lock_font_system(&self.font_system);
                    let mut buffer = Buffer::new(
                        &mut fonts,
                        Metrics::new(physical_size, physical_line_height),
                    );
                    buffer.set_size(Some(physical_width), Some(physical_height));
                    buffer.set_wrap(cosmic_wrap(
                        wrap,
                        wrap_break,
                        opentype.word_break,
                        opentype.line_break,
                    ));
                    // Built here rather than above the cache lookup: a hit never
                    // shapes, and the family name, the feature list and the variation
                    // axes are a per-node allocation to assemble.
                    let attrs = shape_attrs(
                        family,
                        weight,
                        letter_spacing,
                        size,
                        &opentype.features,
                        &opentype.variations,
                        opentype.kerning,
                        italic,
                    );
                    buffer.set_ellipsize(cosmic_text::Ellipsize::None);
                    if rich {
                        let mut rich_text = painted
                            .iter()
                            .map(|(text, color)| (*text, attrs.clone().color(rgba8_color(*color))))
                            .collect::<Vec<_>>();
                        if opentype.direction.is_rtl() {
                            rich_text.insert(0, (RTL_ISOLATE_PREFIX, attrs.clone()));
                            rich_text.push((RTL_ISOLATE_SUFFIX, attrs.clone()));
                        }
                        buffer.set_rich_text(rich_text, &attrs, shaping, align);
                    } else {
                        let shaped = wrap_for_css_direction(content, opentype.direction);
                        buffer.set_text(&shaped, &attrs, shaping, align);
                    }
                    buffer.shape_until_scroll(&mut fonts, false);
                    if ellipsis
                        && measured_text_overflows(
                            &buffer,
                            wrap,
                            Some(physical_width),
                            Some(physical_height),
                            max_lines,
                        )
                    {
                        buffer.set_ellipsize(ellipsize_end(max_lines, Some(physical_height)));
                        buffer.shape_until_scroll(&mut fonts, false);
                    }
                    drop(fonts);
                    self.shape_cache.insert(hash, key.to_owned_key(), buffer);
                }
                hash
            }
        };
        // The widest line and the laid-out height are the shape's, and the
        // shape is the one this entry was built from, so a steady frame does
        // not walk its layout runs again to find that out.
        let (measured_width, laid_out_height) = match retained {
            Some((_, _, measured)) => (measured[0], measured[1]),
            None => {
                let buffer = self.shape_cache.buffer(hash).expect("shaped above");
                measure(buffer)
            }
        };
        let mut aligned = text_box_origin(bounds, vertical, laid_out_height / scale);
        aligned[0] += paint_offset[0];
        aligned[1] += paint_offset[1];
        if clip::is_translation_projective(affine, persp) {
            let line_logical = laid_out_height / scale;
            let [_, wy] = clip::transform_point_projective(affine, persp, aligned[0], aligned[1]);
            let (top_px, _) =
                clip::snap_centered_origin(wy + line_logical * 0.5, line_logical, scale);
            aligned[1] += top_px / scale - wy;
        }
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
        let pad_y = (size * 1.25 - line_height).max(0.0);
        let pad_x = size * 0.25;
        let ink = LogicalRect::from_xywh(
            aligned[0] - pad_x,
            aligned[1] - pad_y,
            bounds.width.max(measured_width / scale) + pad_x * 2.0,
            laid_out_height / scale + pad_y * 2.0,
        );
        // An axis-aligned run is clipped by the batch's scissor. Rotated or
        // projective text carries the same homography as Quad, applied per
        // glyph corner in the vertex stage, and a rounded or polygonal clip
        // needs the fragment test the scissor cannot express — neither of
        // which is a reason to resolve the paragraph differently.
        let translation = clip::is_translation_projective(affine, persp);
        let paint_origin = if translation {
            let [world_x, world_y] = clip::transform_point(affine, aligned[0], aligned[1]);
            [world_x * scale, world_y * scale]
        } else {
            [aligned[0] * scale, aligned[1] * scale]
        };
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
        self.target.nodes_considered += 1;
        if !reachable {
            self.target.nodes_culled += 1;
            return None;
        }
        // The whole-pixel half of the origin is presentation: it moves with
        // the paragraph and never changes a bitmap. The remainder is not —
        // it is the sub-pixel phase every glyph in this paragraph was
        // rasterized for, so it is resolved into the instances and named by
        // the entry key.
        let whole = [paint_origin[0].floor(), paint_origin[1].floor()];
        let phase = [
            (paint_origin[0] - whole[0]).to_bits(),
            (paint_origin[1] - whole[1]).to_bits(),
        ];
        let index = self.target.live_runs;
        if index == self.target.runs.len() {
            self.target.runs.push(TextRun {
                entry: 0,
                presentation: RunPresentation {
                    origin: [0.0; 2],
                    color: [0.0; 4],
                    opacity: 1.0,
                    flags: 0,
                    presentation: 0,
                },
                next: NO_RUN,
                last: index as u32,
                folded: false,
                segments: 0..0,
            });
        }
        let epoch = self.placement_epoch();
        let reusable = retained
            .map(|(id, _, _)| id)
            .or_else(|| self.target.entries.lookup(entry_key))
            .filter(|id| {
                self.target
                    .entries
                    .get(*id)
                    .is_some_and(|entry| entry.valid(hash, phase, self.font_generation))
            })
            .filter(|id| {
                // The atlas moved since this entry read its rectangles. Repair
                // them through the handles it kept — no shaping, no
                // rasterizing, no atlas traffic. Repairing here rather than at
                // flush is what lets a handle the atlas has reissued be
                // *resolved* again in the same frame instead of drawing a
                // paragraph with holes in it for one frame.
                let Self { atlas, target, .. } = self;
                let stale = target
                    .entries
                    .get(*id)
                    .is_some_and(|entry| entry.atlas_epoch != epoch);
                if !stale {
                    return true;
                }
                target.instance_patches += 1;
                target.entries.repair(*id, epoch, |handle| {
                    atlas.entry(handle).map(|entry| (entry.origin, entry.size))
                })
            });
        let entry = match reusable {
            Some(id) => {
                // The paragraph, its sub-pixel phase and the face set are all
                // the ones these instances were resolved from. Nothing below
                // this line reads a glyph.
                self.target.entries.note_reuse();
                self.target.nodes_skipped += 1;
                id
            }
            None => self.build_entry(
                device,
                entry_key,
                hash,
                phase,
                default_color,
                revision,
                scale.to_bits(),
            )?,
        };
        let frame = self.target.frame;
        if let Some(entry) = self.target.entries.get_mut(entry) {
            entry.last_used = frame;
            entry.measured = [measured_width, laid_out_height];
        }
        // After the entry, so a paragraph that resolves to nothing does not
        // leave a row in the table nobody names.
        let presentation = self.presentation_index(affine, persp, fragment_clip, scale);
        let run = &mut self.target.runs[index];
        run.entry = entry;
        run.presentation = RunPresentation {
            origin: whole,
            color: run_color(default_color),
            opacity: opacity.clamp(0.0, 1.0),
            flags,
            presentation,
        };
        run.next = NO_RUN;
        run.last = index as u32;
        run.folded = false;
        run.segments = 0..0;
        self.target.live_runs += 1;
        Some(PreparedText { index, ink })
    }

    /// The row `affine`/`persp`/`clip` present through, adding it if this
    /// frame has not seen that combination yet.
    fn presentation_index(
        &mut self,
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
        if let Some((held, index)) = self.target.last_presentation
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
        if let Some(last) = self.target.presentations.last()
            && *last == row
        {
            let index = (self.target.presentations.len() - 1) as u32;
            self.target.last_presentation = Some((inputs, index));
            return index;
        }
        if let Some(index) = self.target.presentation_index.get(&bits) {
            let index = *index;
            self.target.last_presentation = Some((inputs, index));
            return index;
        }
        let index = self.target.presentations.len() as u32;
        self.target.presentations.push(row);
        self.target.presentation_index.insert(bits, index);
        self.target.last_presentation = Some((inputs, index));
        index
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
        device: &wgpu::Device,
        key: EntryKey,
        hash: u64,
        phase: [u32; 2],
        default_color: [f32; 4],
        revision: u64,
        scale_bits: u32,
    ) -> Option<u32> {
        let origin = [f32::from_bits(phase[0]), f32::from_bits(phase[1])];
        let Self {
            shape_cache,
            resolved,
            rasterizer,
            font_generation,
            ..
        } = self;
        let buffer = shape_cache.buffer(hash).expect("shaped above");
        resolved.clear();
        let generation = *font_generation as u32;
        for run in buffer.layout_runs() {
            let line_y = run.line_y.round();
            for glyph in run.glyphs {
                let font_size = glyph.font_size;
                let x = font_size.mul_add(glyph.x_offset, glyph.x) + origin[0];
                // Y is snapped to whole pixels before the line origin is added,
                // which is what keeps a baseline from landing between texels.
                // `floor`, not `trunc`: rounding toward zero would snap text
                // above the origin the other way and shift its baseline by a
                // pixel as it scrolls past y = 0.
                let y = (font_size.mul_add(-glyph.y_offset, glyph.y) + origin[1]).floor() + line_y;
                resolved.push(
                    rasterizer.intern(glyph.font_id, glyph.font_weight),
                    generation,
                    glyph::GlyphVariationId(glyph.font_variation_hash),
                    size_bits(font_size),
                    synthesis_from_backend(glyph.cache_key_flags),
                    GlyphRenderMode::Mask,
                    glyph
                        .color_opt
                        .map(color_from_cosmic)
                        .unwrap_or(default_color),
                    PlacedGlyph {
                        glyph: u32::from(glyph.glyph_id),
                        x,
                        y,
                    },
                );
            }
        }
        if resolved.is_empty() {
            return None;
        }
        let Self {
            resolved,
            rasterizer,
            raster,
            atlas,
            uploads,
            target,
            resolve_requests,
            ..
        } = self;
        let pages_before = atlas.page_count();
        let inherited = pipeline::pack_srgb(default_color);
        let placeholders = [
            atlas.placeholder_page(AtlasPageKind::Mask),
            atlas.placeholder_page(AtlasPageKind::Color),
        ];
        let id = target
            .entries
            .begin_build(key, resolved.glyphs.len() as u32, |handle| {
                atlas.release(handle)
            });
        target.instance_rebuilds += 1;
        let mut placed = 0u32;
        let mut segments: Vec<EntrySegment> = Vec::new();
        for run in &resolved.runs {
            // The run's color is the same for every glyph under it, so it is
            // packed once rather than per glyph — and compared once against
            // the paragraph's own color, because a glyph that paints it can
            // inherit the run row instead of carrying four bytes that a
            // recolor would then have to rewrite.
            let color = pipeline::pack_srgb(run.color);
            let own = if color == inherited {
                0
            } else {
                pipeline::INSTANCE_OWN_COLOR
            };
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
                            None => continue,
                        }
                    }
                };
                atlas.retain(handle);
                let content = match placement.kind {
                    AtlasPageKind::Mask => CONTENT_MASK,
                    AtlasPageKind::Color => CONTENT_COLOR,
                };
                let (mask_page, color_page) = match placement.kind {
                    AtlasPageKind::Mask => (placement.page, placeholders[1]),
                    AtlasPageKind::Color => (placeholders[0], placement.page),
                };
                push_entry_segment(&mut segments, placeholders, mask_page, color_page, placed);
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
                    ),
                );
                placed += 1;
            }
        }
        target.frame_gpu_allocations += atlas.page_count() - pages_before;
        let counters = atlas.counters();
        let epoch = counters.evictions.wrapping_add(counters.relocations);
        target.entries.finish_build(id, placed);
        let fonts = self.font_generation;
        let entry = self.target.entries.get_mut(id).expect("just built");
        entry.layout = hash;
        entry.phase = phase;
        entry.font_generation = fonts;
        entry.revision = revision;
        entry.scale_bits = scale_bits;
        entry.atlas_epoch = epoch;
        entry.segments = segments;
        entry.run = NO_RUN;
        if placed == 0 {
            return None;
        }
        Some(id)
    }

    /// Fold the run just opened by `next` into `previous`, so both draw as one
    /// command. Returns `false` when the two cannot share a run and the caller
    /// must keep `next` as its own command.
    ///
    /// Mirrors [`super::push_icon`] / [`super::push_quad`]: only runs that are
    /// already neighbours in document order merge, and glyph order inside the
    /// merged command is placement order, so a text shadow still paints under
    /// the text it belongs to.
    ///
    /// Color, position, opacity and transform are **not** part of this
    /// decision. Each instance names its own run row, so two paragraphs that
    /// present completely differently still draw as one command as long as
    /// their glyphs come from compatible atlas pages.
    pub(super) fn can_merge_runs(&self, previous: &PreparedText, next: &PreparedText) -> bool {
        // `next` must be the run just opened, so folding it away is an append.
        next.index + 1 == self.target.live_runs
            && previous.index < next.index
            && previous.index >= self.target.flushed
            && self
                .target
                .runs
                .get(previous.index)
                .is_some_and(|run| !run.folded)
    }

    pub(super) fn merge_runs(&mut self, previous: &PreparedText, next: &PreparedText) {
        debug_assert!(self.can_merge_runs(previous, next));
        let tail = self.target.runs[previous.index].last;
        self.target.runs[tail as usize].next = next.index as u32;
        self.target.runs[previous.index].last = next.index as u32;
        self.target.runs[next.index].folded = true;
    }

    /// Give every still-open run an arena range and a run row, and turn the
    /// entries under it into draw segments. Must run before `upload` and
    /// before `draw`.
    ///
    /// This is where retention pays: an entry whose block is already at the
    /// offset this walk assigns it, under the run index it already names, is
    /// passed over without reading a single instance.
    pub(super) fn flush_runs(&mut self) {
        if self.target.flushed >= self.target.live_runs {
            return;
        }
        let Self { atlas, target, .. } = self;
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
        // What the frame needs, and what of it the arena does not already
        // hold, so a repack happens instead of running out of room.
        let mut total = 0u32;
        let mut fresh = 0u32;
        Self::walk_runs(target, |target, _, entry_id| {
            let Some(entry) = target.entries.get(entry_id) else {
                return;
            };
            let capacity = entry.capacity;
            total += capacity;
            if entry.arena_generation != Some(target.arena.generation())
                || entry.arena_capacity != capacity
            {
                fresh += capacity;
            }
        });
        if target.arena.should_repack(total, fresh) {
            target.arena.repack(total);
            target.entries.invalidate_arena();
        }
        let counters = atlas.counters();
        let epoch = counters.evictions.wrapping_add(counters.relocations);
        let placeholders = [
            atlas.placeholder_page(AtlasPageKind::Mask),
            atlas.placeholder_page(AtlasPageKind::Color),
        ];
        let generation = target.arena.generation();
        let mut breaks = 0u32;
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
                    let intact = target.entries.repair(entry_id, epoch, |handle| {
                        atlas.entry(handle).map(|entry| (entry.origin, entry.size))
                    });
                    target.instance_patches += 1;
                    if !intact && let Some(entry) = target.entries.get_mut(entry_id) {
                        entry.damaged = true;
                    }
                }
                if target.entries.bind_run(entry_id, slot) {
                    target.instance_patches += 1;
                    dirty = true;
                }
                let entry = target.entries.get(entry_id).expect("looked up above");
                let offset = entry.arena_offset;
                if dirty {
                    let staged = target.staging.len() as u32;
                    let contiguous = target.writes.last().is_some_and(|write| {
                        write.offset + (write.staged.end - write.staged.start) == offset
                    });
                    if contiguous {
                        target
                            .writes
                            .last_mut()
                            .expect("contiguous implies a write")
                            .staged
                            .end += capacity;
                    } else {
                        target.writes.push(ArenaWrite {
                            offset,
                            staged: staged..staged + capacity,
                        });
                    }
                    let entry = target.entries.get(entry_id).expect("looked up above");
                    let block = target.entries.instances(entry);
                    target.staging.extend_from_slice(block);
                }
                let entry = target.entries.get(entry_id).expect("looked up above");
                let adjacent = previous_end.is_none_or(|end| end == offset);
                if !adjacent {
                    breaks += 1;
                }
                for segment in &entry.segments {
                    builder.push(&mut target.segments, *segment, offset, adjacent);
                }
                previous_end = Some(offset + capacity);
                let next = target.runs[member as usize].next;
                if next == NO_RUN {
                    break;
                }
                member = next;
            }
            builder.finish(&mut target.segments);
            target.runs[index].segments = first_segment..target.segments.len() as u32;
        }
        target.arena.note_breaks(breaks);
        target.flushed = target.live_runs;
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

    /// Write this frame's atlas regions, arena blocks and presentation tables.
    pub(super) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        self.uploads.flush(queue, &self.atlas, work);
        // Bind groups are created here, where the atlas is still `&mut`, so
        // `draw` only has to look one up. Nearly every frame's segments name
        // the same pair, so the pair is only looked up when it changes.
        let Self { atlas, target, .. } = self;
        let mut last = None;
        for segment in target.segments.iter() {
            let pair = (segment.mask_page, segment.color_page);
            if last != Some(pair) {
                last = Some(pair);
                atlas.bind_group(device, pair.0, pair.1);
            }
        }
        let bytes = self.gpu.upload(
            device,
            queue,
            &mut target.gpu,
            target.physical_size,
            &FrameUpload {
                arena_capacity: target.arena.capacity(),
                writes: &target.writes,
                staging: &target.staging,
                runs: &target.run_table,
                run_dirty: target.run_dirty.clone(),
                presentations: &target.presentations,
                uploaded_presentations: &target.uploaded_presentations,
            },
            work,
        );
        target.instance_upload_bytes += bytes.instances as u64;
        target.presentation_upload_bytes += bytes.presentation as u64;
        target.frame_gpu_allocations += target.gpu.take_allocations();
    }

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        prepared: &PreparedText,
        scissor: PhysicalRect,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let Some(run) = self
            .target
            .runs
            .get(prepared.index)
            .filter(|_| prepared.index < self.target.live_runs)
        else {
            return;
        };
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        let start = run.segments.start as usize;
        let end = (run.segments.end as usize).min(self.target.segments.len());
        let mut drawn = 0u64;
        for segment in &self.target.segments[start.min(end)..end] {
            let Some(bind_group) = self
                .atlas
                .cached_bind_group(segment.mask_page, segment.color_page)
            else {
                continue;
            };
            self.gpu
                .draw_segment(pass, &self.target.gpu, segment, bind_group);
            drawn += 1;
        }
        self.draws.set(self.draws.get() + drawn);
        if let Some(work) = gpu_work {
            work.record_draw_batch();
            for _ in 0..drawn {
                work.record_draw_call();
            }
        }
    }

    pub(super) fn swap_target(
        &mut self,
        target: &mut Option<TextPipelineTarget>,
        device: &wgpu::Device,
    ) {
        let target =
            target.get_or_insert_with(|| TextPipelineTarget::new(self.gpu.new_target(device)));
        std::mem::swap(&mut self.target, target);
    }
}

impl RunPresentation {
    fn to_gpu(self) -> TextRunGpu {
        TextRunGpu::new(
            self.origin,
            self.presentation,
            self.flags,
            self.color,
            self.opacity,
        )
    }
}

/// Open or extend the segment an entry's glyph at `offset` belongs to.
///
/// A segment has to name a page of each kind because one bind group does, but
/// only the kinds it actually samples are constrained. A segment still holding
/// a placeholder has not sampled that kind yet, so the first glyph of it adopts
/// a page instead of splitting — which is what keeps one emoji in a line of
/// text free.
fn push_entry_segment(
    segments: &mut Vec<EntrySegment>,
    placeholders: [u32; 2],
    mask_page: u32,
    color_page: u32,
    offset: u32,
) {
    if let Some(open) = segments.last_mut() {
        let mask_ok = open.mask_page == mask_page
            || open.mask_page == placeholders[0]
            || mask_page == placeholders[0];
        let color_ok = open.color_page == color_page
            || open.color_page == placeholders[1]
            || color_page == placeholders[1];
        if mask_ok && color_ok {
            if mask_page != placeholders[0] {
                open.mask_page = mask_page;
            }
            if color_page != placeholders[1] {
                open.color_page = color_page;
            }
            open.count += 1;
            return;
        }
    }
    segments.push(EntrySegment {
        mask_page,
        color_page,
        first: offset,
        count: 1,
    });
}

/// The content split into the colors it paints in.
///
/// Node opacity is deliberately not folded in: it rides on the run row, so a
/// fade neither reshapes rich text nor rewrites a glyph.
fn presentation_spans<'a>(
    content: &'a str,
    spans: &'a [SceneTextSpan],
    default: [f32; 4],
) -> Vec<(&'a str, [f32; 4])> {
    let mut painted = Vec::new();
    let mut cursor = 0usize;
    for span in spans {
        if span.start > content.len()
            || span.end > content.len()
            || span.start >= span.end
            || !content.is_char_boundary(span.start)
            || !content.is_char_boundary(span.end)
        {
            continue;
        }
        if span.start > cursor {
            painted.push((&content[cursor..span.start], default));
        }
        painted.push((&content[span.start..span.end], span.color));
        cursor = span.end;
    }
    if cursor < content.len() {
        painted.push((&content[cursor..], default));
    }
    painted
}

fn measure(buffer: &Buffer) -> (f32, f32) {
    buffer
        .layout_runs()
        .fold((0.0, 0.0), |(width, height), run| {
            (run.line_w.max(width), height + run.line_height)
        })
}

fn text_box_origin(
    bounds: LogicalRect,
    vertical: TextVerticalAlignment,
    laid_out_height: f32,
) -> [f32; 2] {
    let top = match vertical {
        TextVerticalAlignment::Top => bounds.y,
        TextVerticalAlignment::Center => bounds.y + (bounds.height - laid_out_height) * 0.5,
        TextVerticalAlignment::Bottom => bounds.y + bounds.height - laid_out_height,
    };
    [bounds.x, top]
}

fn opentype_disc(word_break: nana_ui_core::WordBreakSpec) -> u8 {
    match word_break {
        nana_ui_core::WordBreakSpec::Normal => 0,
        nana_ui_core::WordBreakSpec::BreakAll => 1,
        nana_ui_core::WordBreakSpec::BreakWord => 2,
    }
}

fn opentype_line_disc(line_break: nana_ui_core::LineBreakSpec) -> u8 {
    match line_break {
        nana_ui_core::LineBreakSpec::Auto => 0,
        nana_ui_core::LineBreakSpec::Normal => 1,
        nana_ui_core::LineBreakSpec::Anywhere => 2,
    }
}

fn opentype_kern_disc(kerning: nana_ui_core::FontKerningSpec) -> u8 {
    match kerning {
        nana_ui_core::FontKerningSpec::Auto => 0,
        nana_ui_core::FontKerningSpec::Normal => 1,
        nana_ui_core::FontKerningSpec::None => 2,
    }
}

fn rgba8_color(color: [f32; 4]) -> Color {
    let [r, g, b, a] = to_rgba8(color);
    Color::rgba(r, g, b, a)
}

fn color_from_cosmic(color: Color) -> [f32; 4] {
    let [r, g, b, a] = color.as_rgba();
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

/// Whether `id`'s rectangles predate the atlas's current placement epoch.
fn entry_needs_repair(entries: &EntryStore, id: u32, epoch: u64) -> bool {
    entries
        .get(id)
        .is_some_and(|entry| entry.atlas_epoch != epoch)
}

/// The run row's color, quantized exactly as an instance's four bytes would
/// be.
///
/// A glyph inherits the row whenever its own packed color matches the
/// paragraph's, so the two must agree to the bit — otherwise a rich span that
/// happens to paint the default color would shift by a least significant bit
/// when it stopped carrying its own.
fn run_color(color: [f32; 4]) -> [f32; 4] {
    let [r, g, b, a] = to_rgba8(color);
    [
        linear_from_srgb8(r),
        linear_from_srgb8(g),
        linear_from_srgb8(b),
        f32::from(a) / 255.0,
    ]
}

/// The axis-aligned box `ink` covers once its node's homography is applied.
fn transformed_ink(ink: LogicalRect, affine: [f32; 6], persp: [f32; 2]) -> LogicalRect {
    clip::transformed_aabb_projective(ink, affine, persp)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn ellipsis_paint_keeps_exact_fit_and_truncates_only_narrow_boxes() {
        use nana_ui_runtime::{
            ComputedStyle, StableNodeId, TextContent, TextShapeConstraints, TextShaper,
        };

        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        pipeline.begin_frame([512, 64]);
        let mut shaper = crate::NanaTextShaper::default();
        let style = ComputedStyle {
            font_size: 12.0,
            font_weight: Some(600),
            ..ComputedStyle::default()
        };
        for (content, shaping) in [
            ("未命名 1", TextShaping::Advanced),
            ("shade", TextShaping::Advanced),
            ("Cost $0.30", TextShaping::Auto),
            ("Save memory", TextShaping::Auto),
        ] {
            let natural = shaper
                .shape(
                    StableNodeId::new(1).unwrap(),
                    &TextContent {
                        value: content.into(),
                    },
                    &style,
                    TextShapeConstraints {
                        wrap: false,
                        shaping,
                        ..TextShapeConstraints::default()
                    },
                )
                .width;
            // Revisit the exact-fit entry after inserting a truncated one.
            let mut full_glyphs = Vec::new();
            for (width, ellipsis) in [
                (natural, false),
                (natural, true),
                (natural * 0.5, true),
                (natural, true),
            ] {
                pipeline
                    .prepare(
                        &device,
                        LogicalRect::from_xywh(0.0, 0.0, width, 24.0),
                        LogicalRect::from_xywh(0.0, 0.0, 512.0, 64.0),
                        1.0,
                        content,
                        Some([1.0; 4]),
                        12.0,
                        Some(600),
                        None,
                        None,
                        false,
                        nana_ui_core::TextWrapBreak::Word,
                        false,
                        ellipsis,
                        None,
                        shaping,
                        TextHorizontalAlignment::Start,
                        TextVerticalAlignment::Top,
                        &[],
                        0.0,
                        &[],
                        &SceneTextOpenType::default(),
                        clip::IDENTITY_AFFINE,
                        [0.0; 2],
                        clip::FragmentClip::PASS,
                        1.0,
                        [0.0; 2],
                        EntryKey {
                            node: 1,
                            slot: 0,
                            pass: 0,
                        },
                        UNTRACKED_REVISION,
                    )
                    .expect("label must prepare");
                let entry = pipeline
                    .shape_cache
                    .entries
                    .values()
                    .find(|entry| {
                        entry.key.content == content
                            && entry.key.width_bits == width.to_bits()
                            && entry.key.ellipsis == ellipsis
                    })
                    .expect("prepared label must be cached at its own width");
                let painted_width = measure(&entry.buffer).0;
                let painted_glyphs: Vec<_> = entry
                    .buffer
                    .layout_runs()
                    .flat_map(|run| run.glyphs.iter())
                    .map(|glyph| (glyph.font_id, glyph.glyph_id, glyph.start, glyph.end))
                    .collect();
                if !ellipsis {
                    assert!(!painted_glyphs.is_empty());
                    full_glyphs = painted_glyphs;
                    continue;
                }
                if width == natural {
                    assert_eq!(
                        painted_glyphs, full_glyphs,
                        "exact-fit paint must retain every original glyph"
                    );
                    assert!(
                        (painted_width - natural).abs() < 0.01,
                        "exact-fit paint changed {content:?}: {painted_width} vs {natural}"
                    );
                } else {
                    assert_ne!(
                        painted_glyphs, full_glyphs,
                        "narrow paint must replace the text tail"
                    );
                    assert!(
                        painted_width <= width + 0.5 && painted_width < natural,
                        "narrow paint must truncate {content:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn rtl_latin_in_wide_box_places_first_glyph_on_the_right() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        pipeline.begin_frame([256, 64]);
        let bounds = LogicalRect::from_xywh(0.0, 0.0, 200.0, 24.0);
        let clip = LogicalRect::from_xywh(0.0, 0.0, 200.0, 24.0);
        let opentype = SceneTextOpenType {
            direction: nana_ui_core::DirSpec::Rtl,
            ..SceneTextOpenType::default()
        };
        pipeline
            .prepare(
                &device,
                bounds,
                clip,
                1.0,
                "Hello",
                Some([1.0, 1.0, 1.0, 1.0]),
                16.0,
                None,
                None,
                None,
                false,
                nana_ui_core::TextWrapBreak::Word,
                false,
                false,
                None,
                TextShaping::Advanced,
                TextHorizontalAlignment::Start,
                TextVerticalAlignment::Top,
                &[],
                0.0,
                &[],
                &opentype,
                clip::IDENTITY_AFFINE,
                [0.0, 0.0],
                clip::FragmentClip::PASS,
                1.0,
                [0.0, 0.0],
                EntryKey {
                    node: 1,
                    slot: 0,
                    pass: 0,
                },
                UNTRACKED_REVISION,
            )
            .expect("rtl latin must prepare");
        let buffer = pipeline
            .shape_cache
            .entries
            .values()
            .next()
            .map(|entry| &entry.buffer)
            .expect("paint must cache the shaped run");
        let glyph_x = crate::nana_text::first_content_glyph_x(buffer)
            .expect("rtl latin must shape a content glyph");
        assert!(
            glyph_x > 100.0,
            "first Latin glyph must sit on the right of a 200px RTL box, got {glyph_x}"
        );
    }

    #[test]
    fn text_box_origin_keeps_left_edge_and_centers_vertically() {
        let bounds = LogicalRect::from_xywh(10.0, 20.0, 100.0, 40.0);
        assert_eq!(
            text_box_origin(bounds, TextVerticalAlignment::Top, 12.0),
            [10.0, 20.0]
        );
        assert_eq!(
            text_box_origin(bounds, TextVerticalAlignment::Center, 12.0),
            [10.0, 34.0]
        );
        assert_eq!(
            text_box_origin(bounds, TextVerticalAlignment::Bottom, 12.0),
            [10.0, 48.0]
        );
    }

    #[test]
    fn perspective_text_foreshortens_the_far_edge() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut pipeline = TextPipeline::new(&device, &queue, format);
        let mat = nana_ui_core::PaintMat4::perspective(120.0)
            .expect("finite depth")
            .then(nana_ui_core::PaintMat4::rotate_y(45_f32.to_radians()))
            .around_origin(0.0, 0.0, 64.0, 64.0);
        let (affine, persp) = mat.planar_homography().expect("homography");
        let pixels = paint_block(
            &device,
            &queue,
            &mut pipeline,
            LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0),
            LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0),
            affine,
            persp,
            clip::FragmentClip::PASS,
        );
        let ink = ink_aabb(&pixels, 64, 64).expect("perspective text must paint");
        let column_height = |x: u32| {
            let mut top = None;
            let mut bottom = 0;
            for y in 0..64u32 {
                if inked(pixel(&pixels, 64, x, y)) {
                    top.get_or_insert(y);
                    bottom = y;
                }
            }
            top.map(|top| bottom - top + 1)
        };
        let near = (ink.0..=ink.2)
            .find_map(column_height)
            .expect("a near column");
        let far = (ink.0..=ink.2)
            .rev()
            .find_map(column_height)
            .expect("a far column");
        assert_ne!(
            near, far,
            "a perspective homography must foreshorten one edge of the text, \
             near={near} far={far}"
        );
    }

    #[test]
    fn rotated_text_prepare_vertices_are_not_the_unrotated_aabb() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut pipeline = TextPipeline::new(&device, &queue, format);
        let bounds = LogicalRect::from_xywh(8.0, 16.0, 48.0, 20.0);
        let clip = LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0);
        let identity = clip::IDENTITY_AFFINE;
        // x' = -y + 40, y' = x keeps a 90° rotation on-screen.
        let rot90 = [0.0, 1.0, -1.0, 0.0, 40.0, 0.0];
        let identity_pixels = paint_text(
            &device,
            &queue,
            &mut pipeline,
            bounds,
            clip,
            identity,
            [0.0, 0.0],
            clip::FragmentClip::PASS,
        );
        let rotated_pixels = paint_text(
            &device,
            &queue,
            &mut pipeline,
            bounds,
            clip,
            rot90,
            [0.0, 0.0],
            clip::FragmentClip::PASS,
        );
        let identity_ink = ink_aabb(&identity_pixels, 64, 64).expect("unrotated text must paint");
        let rotated_ink = ink_aabb(&rotated_pixels, 64, 64).expect("rotated text must paint");
        assert_ne!(
            identity_ink, rotated_ink,
            "rotated glyphs must not occupy the unrotated AABB"
        );
        let identity_w = identity_ink.2 - identity_ink.0 + 1;
        let identity_h = identity_ink.3 - identity_ink.1 + 1;
        let rotated_w = rotated_ink.2 - rotated_ink.0 + 1;
        let rotated_h = rotated_ink.3 - rotated_ink.1 + 1;
        assert!(
            identity_w > identity_h,
            "unrotated 'Hi' ink must be wide, got {identity_ink:?}"
        );
        assert!(
            rotated_h > rotated_w,
            "90° glyph quads must paint a tall AABB, not a translated wide run, got {rotated_ink:?}"
        );
    }

    #[test]
    fn rotated_clip_discards_affine_glyph_in_aabb_outside_rect() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut pipeline = TextPipeline::new(&device, &queue, format);
        let bounds = LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0);
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let clips = [nana_ui_scene::ClipRegion {
            bounds: nana_ui_scene::SceneRect {
                x: 16.0,
                y: 16.0,
                width: 32.0,
                height: 32.0,
            },
            transform: nana_ui_scene::AffineTransform::from_matrix(
                nana_ui_core::PaintTransform {
                    a: k,
                    b: k,
                    c: -k,
                    d: k,
                    ..nana_ui_core::PaintTransform::default()
                }
                .around_center(16.0, 16.0, 32.0, 32.0),
            ),
            corner_radius: 0.0,
            polygon_clip: None,
        }];
        let origin = clip::paint_origin([0.0, 0.0], [0.0, 0.0]);
        let aabb = clip::intersect_clips(
            LogicalRect::viewport([0.0, 0.0], [64.0, 64.0]),
            &clips,
            origin,
        )
        .unwrap();
        let frag = clip::fragment_clip(&clips, origin);
        let unclipped = paint_block(
            &device,
            &queue,
            &mut pipeline,
            bounds,
            LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0),
            clip::IDENTITY_AFFINE,
            [0.0, 0.0],
            clip::FragmentClip::PASS,
        );
        let clipped = paint_block(
            &device,
            &queue,
            &mut pipeline,
            bounds,
            aabb,
            clip::IDENTITY_AFFINE,
            [0.0, 0.0],
            frag,
        );
        let mut probe = None;
        for y in 0..64u32 {
            for x in 0..64u32 {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                if px >= aabb.x
                    && py >= aabb.y
                    && px < aabb.x + aabb.width
                    && py < aabb.y + aabb.height
                    && !clip::point_in_fragment_clip(px, py, frag)
                    && inked(pixel(&unclipped, 64, x, y))
                {
                    probe = Some((x, y));
                    break;
                }
            }
            if probe.is_some() {
                break;
            }
        }
        let (probe_x, probe_y) =
            probe.expect("unclipped glyphs must ink a pixel in AABB-outside-rotated-rect");
        let probe_clipped = pixel(&clipped, 64, probe_x, probe_y);
        let mut inside = false;
        for y in 0..64u32 {
            for x in 0..64u32 {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                if clip::point_in_fragment_clip(px, py, frag) && inked(pixel(&clipped, 64, x, y)) {
                    inside = true;
                    break;
                }
            }
            if inside {
                break;
            }
        }
        assert!(
            !inked(probe_clipped),
            "affine glyphs must discard AABB-outside-rotated-rect, pixel ({probe_x},{probe_y})={probe_clipped:?}"
        );
        assert!(inside, "rotated clip interior must still paint the glyph");
    }

    /// Prepare one label, flush it and upload, reporting the GPU work.
    #[allow(clippy::too_many_arguments)]
    fn prepare_label(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &mut TextPipeline,
        content: &str,
        scale: f32,
    ) -> nana_ui_core::GpuWorkObservation {
        pipeline.begin_frame([256, 64]);
        pipeline
            .prepare(
                device,
                LogicalRect::from_xywh(0.0, 0.0, 240.0, 32.0),
                LogicalRect::from_xywh(0.0, 0.0, 256.0, 64.0),
                scale,
                content,
                Some([1.0, 1.0, 1.0, 1.0]),
                16.0,
                None,
                None,
                None,
                false,
                nana_ui_core::TextWrapBreak::Word,
                false,
                false,
                None,
                TextShaping::Auto,
                TextHorizontalAlignment::Start,
                TextVerticalAlignment::Top,
                &[],
                0.0,
                &[],
                &SceneTextOpenType::default(),
                clip::IDENTITY_AFFINE,
                [0.0; 2],
                clip::FragmentClip::PASS,
                1.0,
                [0.0; 2],
                EntryKey {
                    node: 1,
                    slot: 0,
                    pass: 0,
                },
                UNTRACKED_REVISION,
            )
            .expect("label must prepare");
        pipeline.flush_runs();
        let work = crate::gpu_work::GpuWorkSink::new();
        pipeline.upload(device, queue, Some(&work));
        work.snapshot()
    }

    /// Place one hand-made glyph and return its handle, so a batching test can
    /// mix mask and color glyphs without depending on which faces this machine
    /// happens to have.
    fn place(
        pipeline: &mut TextPipeline,
        device: &wgpu::Device,
        glyph: u32,
        format: raster::GlyphImageFormat,
    ) -> atlas::GlyphAtlasEntryId {
        let key = glyph::GlyphRasterKey {
            font: glyph::GlyphFontId(0),
            font_generation: 0,
            variation: glyph::GlyphVariationId(0),
            glyph,
            size_bits: 16f32.to_bits(),
            subpixel_x: glyph::SubpixelBin::default(),
            subpixel_y: glyph::SubpixelBin::default(),
            synthesis: glyph::GlyphSynthesis::NONE,
            mode: GlyphRenderMode::Mask,
        };
        let image = std::sync::Arc::new(raster::GlyphImage {
            format,
            width: 4,
            height: 6,
            left: 0,
            top: 6,
            data: vec![255; 4 * 6 * format.bytes_per_pixel()],
        });
        let TextPipeline {
            atlas,
            raster,
            uploads,
            ..
        } = pipeline;
        atlas
            .insert(device, key, &image, raster, uploads)
            .expect("an empty atlas must place one glyph")
            .0
    }

    /// Open a run over already-placed glyphs, the way `build_entry` would.
    fn hand_built_run(pipeline: &mut TextPipeline, handles: &[atlas::GlyphAtlasEntryId]) -> usize {
        let index = pipeline.target.live_runs;
        let node = index as u64 + 1;
        let placeholders = [
            pipeline.atlas.placeholder_page(AtlasPageKind::Mask),
            pipeline.atlas.placeholder_page(AtlasPageKind::Color),
        ];
        let key = EntryKey {
            node,
            slot: 0,
            pass: 0,
        };
        let atlas = &mut pipeline.atlas;
        let id = pipeline
            .target
            .entries
            .begin_build(key, handles.len() as u32, |handle| atlas.release(handle));
        let mut segments: Vec<EntrySegment> = Vec::new();
        for (offset, handle) in handles.iter().enumerate() {
            let placement = *pipeline.atlas.entry(*handle).expect("just placed");
            let content = match placement.kind {
                AtlasPageKind::Mask => CONTENT_MASK,
                AtlasPageKind::Color => CONTENT_COLOR,
            };
            let (mask_page, color_page) = match placement.kind {
                AtlasPageKind::Mask => (placement.page, placeholders[1]),
                AtlasPageKind::Color => (placeholders[0], placement.page),
            };
            push_entry_segment(
                &mut segments,
                placeholders,
                mask_page,
                color_page,
                offset as u32,
            );
            pipeline.atlas.retain(*handle);
            pipeline.target.entries.push_glyph(
                id,
                offset as u32,
                *handle,
                GlyphInstance::new(
                    [offset as i32 * 6, 0],
                    placement.size,
                    placement.origin,
                    pipeline::pack_srgb([1.0; 4]),
                    content,
                ),
            );
        }
        let counters = pipeline.atlas.counters();
        let epoch = counters.evictions.wrapping_add(counters.relocations);
        let entry = pipeline.target.entries.get_mut(id).expect("just built");
        entry.atlas_epoch = epoch;
        entry.segments = segments;
        if index == pipeline.target.runs.len() {
            pipeline.target.runs.push(TextRun {
                entry: id,
                presentation: RunPresentation {
                    origin: [0.0; 2],
                    color: [1.0; 4],
                    opacity: 1.0,
                    flags: 0,
                    presentation: 0,
                },
                next: NO_RUN,
                last: index as u32,
                folded: false,
                segments: 0..0,
            });
        }
        pipeline.target.runs[index].entry = id;
        pipeline.target.runs[index].next = NO_RUN;
        pipeline.target.runs[index].last = index as u32;
        pipeline.target.runs[index].folded = false;
        pipeline.target.live_runs += 1;
        index
    }

    #[test]
    fn a_color_glyph_between_mask_glyphs_still_draws_as_one_batch() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        pipeline.begin_frame([64, 64]);
        let handles = [
            place(&mut pipeline, &device, 1, raster::GlyphImageFormat::Mask),
            place(
                &mut pipeline,
                &device,
                2,
                raster::GlyphImageFormat::ColorRgba,
            ),
            place(&mut pipeline, &device, 3, raster::GlyphImageFormat::Mask),
        ];
        let run = hand_built_run(&mut pipeline, &handles);
        pipeline.flush_runs();
        assert_eq!(
            pipeline.target.runs[run].segments.len(),
            1,
            "a mask page and a color page fit one bind group, so an emoji in a \
             line of text must not split the batch"
        );
        let segment = pipeline.target.segments[0];
        assert_eq!(segment.count, 3);
        assert_ne!(
            segment.color_page,
            pipeline.atlas.placeholder_page(AtlasPageKind::Color),
            "the one segment must name the real color page, not the placeholder"
        );
        assert_ne!(
            segment.mask_page,
            pipeline.atlas.placeholder_page(AtlasPageKind::Mask),
            "and the real mask page"
        );
    }

    #[test]
    fn two_paragraphs_folded_into_one_command_draw_as_one_segment() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        pipeline.begin_frame([64, 64]);
        let first = [place(
            &mut pipeline,
            &device,
            1,
            raster::GlyphImageFormat::Mask,
        )];
        let second = [place(
            &mut pipeline,
            &device,
            2,
            raster::GlyphImageFormat::Mask,
        )];
        let a = hand_built_run(&mut pipeline, &first);
        let b = hand_built_run(&mut pipeline, &second);
        // Different origin, different color, different opacity: none of that
        // is a batch key any more, because each instance names its own run.
        pipeline.target.runs[b].presentation.origin = [40.0, 12.0];
        pipeline.target.runs[b].presentation.color = [1.0, 0.0, 0.0, 1.0];
        pipeline.target.runs[b].presentation.opacity = 0.5;
        let previous = PreparedText {
            index: a,
            ink: LogicalRect::from_xywh(0.0, 0.0, 1.0, 1.0),
        };
        let next = PreparedText {
            index: b,
            ink: LogicalRect::from_xywh(0.0, 0.0, 1.0, 1.0),
        };
        assert!(pipeline.can_merge_runs(&previous, &next));
        pipeline.merge_runs(&previous, &next);
        pipeline.flush_runs();
        assert_eq!(
            pipeline.target.runs[a].segments.len(),
            1,
            "two paragraphs that only present differently must stay one draw"
        );
        let a_capacity = pipeline
            .target
            .entries
            .get(pipeline.target.runs[a].entry)
            .expect("entry")
            .capacity;
        assert_eq!(
            pipeline.target.segments[0],
            pipeline::DrawSegment {
                mask_page: pipeline.target.segments[0].mask_page,
                color_page: pipeline.target.segments[0].color_page,
                first: 0,
                count: a_capacity + 1,
            },
            "the one draw spans both blocks, slack included"
        );
        assert_eq!(
            pipeline.target.run_table.len(),
            2,
            "but each keeps its own presentation row"
        );
    }

    /// One label's presentation, so a gate can change exactly one thing.
    #[derive(Clone, Copy)]
    struct Label<'a> {
        content: &'a str,
        key: EntryKey,
        color: [f32; 4],
        opacity: f32,
        affine: [f32; 6],
        top: f32,
    }

    impl<'a> Label<'a> {
        fn new(content: &'a str, node: u64) -> Self {
            Self {
                content,
                key: EntryKey {
                    node,
                    slot: 0,
                    pass: 0,
                },
                color: [1.0, 1.0, 1.0, 1.0],
                opacity: 1.0,
                affine: clip::IDENTITY_AFFINE,
                top: 0.0,
            }
        }
    }

    /// Prepare, flush and upload one frame of `labels`.
    fn text_frame(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &mut TextPipeline,
        labels: &[Label<'_>],
    ) {
        pipeline.begin_frame([512, 256]);
        for label in labels {
            pipeline.prepare(
                device,
                LogicalRect::from_xywh(0.0, label.top, 480.0, 32.0),
                LogicalRect::from_xywh(0.0, 0.0, 512.0, 256.0),
                1.0,
                label.content,
                Some(label.color),
                16.0,
                None,
                None,
                None,
                false,
                nana_ui_core::TextWrapBreak::Word,
                false,
                false,
                None,
                TextShaping::Auto,
                TextHorizontalAlignment::Start,
                TextVerticalAlignment::Top,
                &[],
                0.0,
                &[],
                &SceneTextOpenType::default(),
                label.affine,
                [0.0; 2],
                clip::FragmentClip::PASS,
                label.opacity,
                [0.0; 2],
                label.key,
                UNTRACKED_REVISION,
            );
        }
        pipeline.flush_runs();
        pipeline.upload(device, queue, None);
    }

    #[test]
    fn more_distinct_paragraphs_than_the_cache_holds_still_shape_once_each() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        // One more label than the cache's nominal capacity, all on screen at
        // once, behind a label that changes every frame. That first insert is
        // what a fixed capacity answers by evicting the rows it is about to
        // draw — and then it reshapes every one of them, every frame.
        let contents = (0..SHAPE_CACHE_CAP + 1)
            .map(|index| format!("Distinct row {index}"))
            .collect::<Vec<_>>();
        let frame = |pipeline: &mut TextPipeline, tick: usize| {
            let ticking = format!("tick {tick}");
            let mut labels = vec![Label::new(ticking.as_str(), 1)];
            labels.extend(contents.iter().enumerate().map(|(index, content)| Label {
                top: index as f32 * 2.0,
                ..Label::new(content.as_str(), index as u64 + 2)
            }));
            text_frame(&device, &queue, pipeline, &labels);
        };
        frame(&mut pipeline, 0);
        let (_, warm_misses, _) = pipeline.shape_cache_stats();
        assert_eq!(
            warm_misses,
            contents.len() + 1,
            "the first frame shapes each paragraph once"
        );
        for tick in 1..4 {
            frame(&mut pipeline, tick);
        }
        let (_, misses, _) = pipeline.shape_cache_stats();
        assert_eq!(
            misses - warm_misses,
            3,
            "only the label that really changed is reshaped; the cache has to \
             hold one frame's worth of text, which is a property of the view"
        );
    }

    #[test]
    fn a_face_set_change_gives_back_the_blocks_and_rows_its_entries_held() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let labels = (0..12)
            .map(|index| Label {
                top: index as f32 * 18.0,
                ..Label::new("Face set row", index + 1)
            })
            .collect::<Vec<_>>();
        text_frame(&device, &queue, &mut pipeline, &labels);
        let rows = pipeline.target.run_table.len();
        let slots = pipeline.target.arena.len();
        assert_eq!(rows, labels.len());
        assert!(slots > 0);
        // What `@font-face` does: every entry stops meaning what it meant.
        pipeline.drop_every_entry();
        text_frame(&device, &queue, &mut pipeline, &labels);
        assert_eq!(
            pipeline.target.run_table.len(),
            rows,
            "the rows the dropped entries held are handed out again, not added to"
        );
        assert_eq!(
            pipeline.target.arena.len(),
            slots,
            "and so are their arena blocks"
        );
        assert_eq!(pipeline.glyph_counters().text_gpu_entries_active, 12);
    }

    #[test]
    fn a_paragraph_that_stopped_being_drawn_is_eventually_given_back() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let keep = Label::new("Still here", 1);
        let gone = Label {
            top: 40.0,
            ..Label::new("Closed panel", 2)
        };
        text_frame(&device, &queue, &mut pipeline, &[keep, gone]);
        let warm = pipeline.glyph_counters();
        assert_eq!(warm.text_gpu_entries_active, 2);
        assert_eq!(warm.text_gpu_entries_destroyed, 0);
        // The panel closes. Long enough that a tab flipped back and forth
        // would have paid for neither direction, and then long enough that
        // this one is really gone.
        for _ in 0..RETIRE_AFTER_FRAMES + RETIRE_INTERVAL {
            text_frame(&device, &queue, &mut pipeline, &[keep]);
        }
        let after = pipeline.glyph_counters();
        assert_eq!(
            after.text_gpu_entries_active, 1,
            "the entry of a paragraph nothing draws any more is given back"
        );
        assert_eq!(after.text_gpu_entries_destroyed, 1);
        assert!(
            after.text_gpu_entry_glyphs < warm.text_gpu_entry_glyphs,
            "and so are its instances"
        );
        assert_eq!(
            after.text_instance_rebuilds, warm.text_instance_rebuilds,
            "the paragraph that stayed was never resolved again"
        );
    }

    /// A shape key for `content` with everything else at its default.
    fn shape_key_ref(content: &str) -> ShapeKeyRef<'_> {
        ShapeKeyRef {
            content,
            family: None,
            weight: None,
            font_size_bits: 16f32.to_bits(),
            line_height_bits: 20f32.to_bits(),
            wrap: false,
            wrap_break: nana_ui_core::TextWrapBreak::Word,
            italic: false,
            ellipsis: false,
            max_lines: None,
            shaping: 1,
            letter_spacing_bits: 0f32.to_bits(),
            word_break: 0,
            line_break: 0,
            kerning: 0,
            features: &[],
            variations: &[],
            width_bits: 100f32.to_bits(),
            height_bits: 20f32.to_bits(),
            align: 0,
            direction: 0,
            writing_mode: 0,
            spans: None,
            font_features: &[],
        }
    }

    #[test]
    fn a_shape_hash_collision_reshapes_instead_of_painting_the_other_text() {
        // The hash finds the entry; the stored key decides the hit. A weak
        // hash may therefore cost a reshape, and may never paint the wrong
        // letters — which is the whole reason the key is kept at all.
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let mut fonts = crate::nana_text::lock_font_system(&pipeline.font_system);
        let buffer = Buffer::new(&mut fonts, Metrics::new(16.0, 20.0));
        drop(fonts);
        let collision = 0x5ca1_ab1e_u64;
        pipeline.shape_cache.insert(
            collision,
            shape_key_ref("first paragraph").to_owned_key(),
            buffer,
        );
        let (hits, misses, _) = pipeline.shape_cache_stats();
        assert!(
            pipeline
                .shape_cache
                .get(collision, &shape_key_ref("first paragraph"))
                .is_some(),
            "its own key still hits"
        );
        assert!(
            pipeline
                .shape_cache
                .get(collision, &shape_key_ref("a different paragraph"))
                .is_none(),
            "another paragraph at the same hash must miss, not read that buffer"
        );
        let (after_hits, after_misses, _) = pipeline.shape_cache_stats();
        assert_eq!(after_hits, hits + 1);
        assert_eq!(after_misses, misses + 1);
    }

    #[test]
    fn two_windows_taking_turns_keep_each_others_paragraphs_shaped() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let mut second: Option<TextPipelineTarget> = None;
        // Each window alone fits the cache; together they do not. What window
        // A drew last frame is still on A's screen while B is drawing.
        let half = SHAPE_CACHE_CAP * 2 / 3;
        let window = |offset: usize| {
            (0..half)
                .map(|index| format!("Window {offset} row {index}"))
                .collect::<Vec<_>>()
        };
        let (left, right) = (window(0), window(1));
        let paint = |pipeline: &mut TextPipeline, contents: &[String]| {
            let labels = contents
                .iter()
                .enumerate()
                .map(|(index, content)| Label {
                    top: index as f32 * 2.0,
                    ..Label::new(content.as_str(), index as u64 + 1)
                })
                .collect::<Vec<_>>();
            text_frame(&device, &queue, pipeline, &labels);
        };
        paint(&mut pipeline, &left);
        pipeline.swap_target(&mut second, &device);
        paint(&mut pipeline, &right);
        let (_, warm_misses, _) = pipeline.shape_cache_stats();
        for _ in 0..3 {
            pipeline.swap_target(&mut second, &device);
            paint(&mut pipeline, &left);
            pipeline.swap_target(&mut second, &device);
            paint(&mut pipeline, &right);
        }
        let (_, misses, _) = pipeline.shape_cache_stats();
        assert_eq!(
            misses, warm_misses,
            "neither window may evict the other's text: both are on screen"
        );
    }

    #[test]
    fn a_scrolled_viewport_paints_the_text_its_offset_brought_into_view() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        // The node sits below the viewport in its own space; the scroll offset
        // lives in the transform, exactly as `paint_transform` folds a scene
        // origin into it. Comparing the two spaces directly would drop it.
        let scrolled = [1.0, 0.0, 0.0, 1.0, 0.0, -120.0];
        let pixels = paint_text(
            &device,
            &queue,
            &mut pipeline,
            LogicalRect::from_xywh(4.0, 130.0, 48.0, 20.0),
            LogicalRect::from_xywh(0.0, 0.0, 64.0, 64.0),
            scrolled,
            [0.0, 0.0],
            clip::FragmentClip::PASS,
        );
        assert!(
            ink_aabb(&pixels, 64, 64).is_some(),
            "text the scroll offset brought into view must still paint"
        );
    }

    #[test]
    fn a_static_steady_frame_resolves_no_glyph_and_moves_no_instance() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let labels = (0..8)
            .map(|index| Label {
                top: index as f32 * 24.0,
                ..Label::new("Steady label", index + 1)
            })
            .collect::<Vec<_>>();
        text_frame(&device, &queue, &mut pipeline, &labels);
        let warm = pipeline.glyph_counters();
        for _ in 0..3 {
            text_frame(&device, &queue, &mut pipeline, &labels);
        }
        let steady = pipeline.glyph_counters();
        let (_, misses, _) = pipeline.shape_cache_stats();
        assert_eq!(
            misses, 1,
            "one paragraph is shaped once, however many nodes draw it"
        );
        assert_eq!(
            steady.glyph_resolve_requests, warm.glyph_resolve_requests,
            "a static frame must not resolve a glyph"
        );
        assert_eq!(
            steady.glyph_rasterized, warm.glyph_rasterized,
            "nor rasterize one"
        );
        assert_eq!(
            steady.glyph_upload_bytes, warm.glyph_upload_bytes,
            "nor upload an atlas region"
        );
        assert_eq!(
            steady.text_instance_rebuilds, warm.text_instance_rebuilds,
            "nor rebuild an entry"
        );
        assert_eq!(
            steady.text_instance_patches, warm.text_instance_patches,
            "nor patch one"
        );
        assert_eq!(
            steady.text_instance_upload_bytes, warm.text_instance_upload_bytes,
            "nor move an instance byte"
        );
        assert_eq!(
            steady.text_presentation_upload_bytes, warm.text_presentation_upload_bytes,
            "nor a presentation byte"
        );
        assert_eq!(steady.text_gpu_entries_active, 8);
        assert_eq!(
            steady.text_prepare_nodes_skipped - warm.text_prepare_nodes_skipped,
            24,
            "every node of every steady frame is answered by its entry"
        );
    }

    #[test]
    fn recoloring_and_fading_are_presentation_only() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let label = Label::new("Recolor me", 1);
        text_frame(&device, &queue, &mut pipeline, &[label]);
        let warm = pipeline.glyph_counters();
        let (_, warm_misses, _) = pipeline.shape_cache_stats();
        for (index, color) in [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]]
            .into_iter()
            .enumerate()
        {
            text_frame(
                &device,
                &queue,
                &mut pipeline,
                &[Label {
                    color,
                    opacity: 1.0 - index as f32 * 0.25,
                    ..label
                }],
            );
        }
        let after = pipeline.glyph_counters();
        let (_, misses, _) = pipeline.shape_cache_stats();
        assert_eq!(misses, warm_misses, "a color or a fade must not reshape");
        assert_eq!(
            after.glyph_resolve_requests, warm.glyph_resolve_requests,
            "nor resolve a glyph"
        );
        assert_eq!(
            after.glyph_rasterized, warm.glyph_rasterized,
            "nor rasterize one"
        );
        assert_eq!(
            after.text_instance_rebuilds, warm.text_instance_rebuilds,
            "nor rebuild the entry"
        );
        assert_eq!(
            after.text_instance_upload_bytes, warm.text_instance_upload_bytes,
            "nor move an instance byte"
        );
        assert!(
            after.text_presentation_upload_bytes > warm.text_presentation_upload_bytes,
            "the new color and opacity reach the GPU as run rows"
        );
    }

    #[test]
    fn a_transform_animation_does_not_rebuild_text_instances() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let label = Label::new("Spinning", 1);
        let rotated = |angle: f32| {
            let (sin, cos) = angle.sin_cos();
            [cos, sin, -sin, cos, 24.0, 24.0]
        };
        text_frame(
            &device,
            &queue,
            &mut pipeline,
            &[Label {
                affine: rotated(0.1),
                ..label
            }],
        );
        let warm = pipeline.glyph_counters();
        for step in 1..8 {
            text_frame(
                &device,
                &queue,
                &mut pipeline,
                &[Label {
                    affine: rotated(0.1 + step as f32 * 0.02),
                    ..label
                }],
            );
        }
        let after = pipeline.glyph_counters();
        assert_eq!(
            after.text_instance_rebuilds, warm.text_instance_rebuilds,
            "a rotation is presentation: the glyphs it turns are the same ones"
        );
        assert_eq!(
            after.glyph_rasterized, warm.glyph_rasterized,
            "and no angle is a new bitmap"
        );
        assert_eq!(
            after.text_instance_upload_bytes, warm.text_instance_upload_bytes,
            "nor an instance byte"
        );
        assert!(
            after.text_presentation_upload_bytes > warm.text_presentation_upload_bytes,
            "each angle is a presentation row"
        );
    }

    #[test]
    fn labels_coming_and_going_do_not_renumber_the_ones_that_stayed() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let all = (0..12)
            .map(|index| Label {
                top: index as f32 * 18.0,
                ..Label::new("Row content", index + 1)
            })
            .collect::<Vec<_>>();
        text_frame(&device, &queue, &mut pipeline, &all);
        text_frame(&device, &queue, &mut pipeline, &all);
        let warm = pipeline.glyph_counters();
        // The first label leaves. If a run row were the draw-order index, every
        // remaining label would be renumbered and every instance of every one
        // of them would have to be rewritten.
        text_frame(&device, &queue, &mut pipeline, &all[1..]);
        let after = pipeline.glyph_counters();
        assert_eq!(
            after.text_instance_rebuilds, warm.text_instance_rebuilds,
            "nothing was reshaped or re-resolved"
        );
        assert_eq!(
            after.text_instance_patches, warm.text_instance_patches,
            "and no surviving label had its run rebound"
        );
    }

    #[test]
    fn a_scene_scale_animation_does_not_open_a_new_raster_size_every_frame() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let label = Label::new("Zooming", 1);
        let scaled = |factor: f32| [factor, 0.0, 0.0, factor, 8.0, 8.0];
        text_frame(
            &device,
            &queue,
            &mut pipeline,
            &[Label {
                affine: scaled(1.0),
                ..label
            }],
        );
        let warm = pipeline.glyph_counters();
        let (_, warm_misses, _) = pipeline.shape_cache_stats();
        for step in 1..24 {
            text_frame(
                &device,
                &queue,
                &mut pipeline,
                &[Label {
                    affine: scaled(1.0 + step as f32 * 0.05),
                    ..label
                }],
            );
        }
        let after = pipeline.glyph_counters();
        let (_, misses, _) = pipeline.shape_cache_stats();
        // The DPI scale is what decides the raster size; a scene transform is
        // presentation. Letting the two share a policy is what turns a zoom
        // into a bitmap per frame and a cache that never stops growing.
        assert_eq!(
            misses, warm_misses,
            "a scene scale must not reshape the paragraph"
        );
        assert_eq!(
            after.glyph_rasterized, warm.glyph_rasterized,
            "nor open a raster size bucket per step"
        );
        assert_eq!(
            after.text_instance_rebuilds, warm.text_instance_rebuilds,
            "nor rebuild the entry"
        );
    }

    #[test]
    fn a_dpi_change_reshapes_once_and_going_back_is_free() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let paint = |pipeline: &mut TextPipeline, scale: f32| {
            pipeline.begin_frame([1024, 512]);
            pipeline.prepare(
                &device,
                LogicalRect::from_xywh(0.0, 0.0, 480.0, 32.0),
                LogicalRect::from_xywh(0.0, 0.0, 512.0, 256.0),
                scale,
                "DPI policy",
                Some([1.0; 4]),
                16.0,
                None,
                None,
                None,
                false,
                nana_ui_core::TextWrapBreak::Word,
                false,
                false,
                None,
                TextShaping::Auto,
                TextHorizontalAlignment::Start,
                TextVerticalAlignment::Top,
                &[],
                0.0,
                &[],
                &SceneTextOpenType::default(),
                clip::IDENTITY_AFFINE,
                [0.0; 2],
                clip::FragmentClip::PASS,
                1.0,
                [0.0; 2],
                EntryKey {
                    node: 1,
                    slot: 0,
                    pass: 0,
                },
                UNTRACKED_REVISION,
            );
            pipeline.flush_runs();
            pipeline.upload(&device, &queue, None);
        };
        paint(&mut pipeline, 1.0);
        paint(&mut pipeline, 2.0);
        let warm = pipeline.glyph_counters();
        paint(&mut pipeline, 1.0);
        paint(&mut pipeline, 2.0);
        let after = pipeline.glyph_counters();
        assert_eq!(
            after.glyph_rasterized, warm.glyph_rasterized,
            "a DPI round trip must answer from the caches both scales filled"
        );
        assert_eq!(
            after.text_instance_rebuilds - warm.text_instance_rebuilds,
            2,
            "but each scale is its own shaped paragraph, so its own entry \
             content: the entry is keyed by the node and the two take turns"
        );
    }

    #[test]
    fn moving_a_label_by_whole_pixels_keeps_its_glyphs() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let label = Label::new("Scrolling row", 1);
        text_frame(&device, &queue, &mut pipeline, &[label]);
        let warm = pipeline.glyph_counters();
        for step in 1..6 {
            text_frame(
                &device,
                &queue,
                &mut pipeline,
                &[Label {
                    top: step as f32 * 3.0,
                    ..label
                }],
            );
        }
        let after = pipeline.glyph_counters();
        assert_eq!(
            after.text_instance_rebuilds, warm.text_instance_rebuilds,
            "a whole-pixel move leaves the sub-pixel phase alone, so the \
             bitmaps and the instances that name them still hold"
        );
        assert_eq!(
            after.text_instance_upload_bytes, warm.text_instance_upload_bytes,
            "and the origin the instances are relative to is a run row"
        );
    }

    #[test]
    fn editing_one_label_does_not_retransmit_the_others() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let mut labels = (0..16)
            .map(|index| Label {
                top: index as f32 * 14.0,
                ..Label::new("Row content", index + 1)
            })
            .collect::<Vec<_>>();
        text_frame(&device, &queue, &mut pipeline, &labels);
        text_frame(&device, &queue, &mut pipeline, &labels);
        let block = u64::from(entry::capacity_for(11))
            * std::mem::size_of::<pipeline::GlyphInstance>() as u64;
        // A row in the middle, so every later block would move if the arena
        // could not absorb the edit where it happened.
        for (content, blocks, note) in [
            ("New content", 1, "the same glyph count"),
            (
                "New contents",
                2,
                "one more glyph, which may step its size class",
            ),
        ] {
            let warm = pipeline.glyph_counters();
            labels[7].content = content;
            text_frame(&device, &queue, &mut pipeline, &labels);
            let after = pipeline.glyph_counters();
            assert_eq!(
                after.text_instance_rebuilds - warm.text_instance_rebuilds,
                1,
                "one label changed, so one entry is resolved again ({note})"
            );
            let moved = after.text_instance_upload_bytes - warm.text_instance_upload_bytes;
            assert!(
                moved > 0 && moved <= block * blocks,
                "only the changed paragraph's block may move ({note}): \
                 {moved} bytes against a block of {block} and a list of \
                 {} blocks",
                labels.len()
            );
        }
    }

    #[test]
    fn a_block_rebuilt_shorter_does_not_keep_drawing_the_glyphs_it_dropped() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let head = EntryKey {
            node: 7,
            slot: 0,
            pass: 0,
        };
        let tail = EntryKey {
            node: 8,
            slot: 0,
            pass: 0,
        };
        // Two paragraphs folded into one draw, so the command spans the slack
        // the first one's size class left. Whatever sits there is painted.
        let long = paint_labels(
            &device,
            &queue,
            &mut pipeline,
            &[("Force majeure", head, 0.0), ("tail", tail, 24.0)],
        );
        let short = paint_labels(
            &device,
            &queue,
            &mut pipeline,
            &[("F", head, 0.0), ("tail", tail, 24.0)],
        );
        let long_top = ink_aabb(&long[..], 256, 64).expect("the long label must paint");
        let short_top = ink_aabb(&short[..], 256, 64).expect("the short label must paint");
        assert!(
            short_top.2 < long_top.2,
            "a block rebuilt shorter must not keep painting the letters it \
             dropped: long={long_top:?} short={short_top:?}"
        );
    }

    #[test]
    fn an_eviction_recovers_the_entry_it_hit_without_reshaping_anything() {
        let (device, queue) = test_device();
        // One page that either paragraph fits in with room to spare and the
        // two together do not, so each one's entry keeps losing its placements
        // to the other.
        let mut pipeline = TextPipeline::with_atlas_limits(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            GlyphAtlasLimits {
                page_edge: 80,
                byte_budget: 80 * 80 + 8,
            },
        );
        const LOWER: &str = "abcdefghijklmnopqrstuvwxy";
        const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXY";
        let first = EntryKey {
            node: 1,
            slot: 0,
            pass: 0,
        };
        let second = EntryKey {
            node: 2,
            slot: 0,
            pass: 0,
        };
        let fresh = paint_labels(&device, &queue, &mut pipeline, &[(LOWER, first, 0.0)]);
        let fresh_ink = ink_aabb(&fresh, 256, 64).expect("the first paragraph paints");
        paint_labels(&device, &queue, &mut pipeline, &[(UPPER, second, 0.0)]);
        let warm = pipeline.glyph_counters();
        let (_, warm_misses, _) = pipeline.shape_cache_stats();
        let mut recovered = Vec::new();
        for _ in 0..4 {
            recovered = paint_labels(&device, &queue, &mut pipeline, &[(LOWER, first, 0.0)]);
            paint_labels(&device, &queue, &mut pipeline, &[(UPPER, second, 0.0)]);
        }
        let after = pipeline.glyph_counters();
        let (_, misses, _) = pipeline.shape_cache_stats();
        assert!(
            after.glyph_atlas_evict > warm.glyph_atlas_evict,
            "the two paragraphs must really be evicting each other"
        );
        assert!(
            after.atlas_stale_handle_rejects > warm.atlas_stale_handle_rejects,
            "and the entry must really be meeting handles the eviction reissued"
        );
        assert_eq!(
            ink_aabb(&recovered, 256, 64),
            Some(fresh_ink),
            "the paragraph comes back whole: a rejected handle costs its glyphs, \
             never the wrong ones"
        );
        assert_eq!(
            misses, warm_misses,
            "an eviction costs an entry its glyphs, never its shaped paragraph"
        );
        assert!(
            after.text_instance_rebuilds > warm.text_instance_rebuilds,
            "the entry the eviction hit is resolved again, and only that one"
        );
    }

    #[test]
    fn evicting_a_glyph_moves_the_placement_epoch_that_gates_retained_commands() {
        let (device, queue) = test_device();
        // One page barely wider than a line of text, so a second paragraph of
        // different glyphs cannot coexist with the first.
        let mut pipeline = TextPipeline::with_atlas_limits(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            GlyphAtlasLimits {
                page_edge: 64,
                byte_budget: 64 * 64 + 8,
            },
        );
        prepare_label(&device, &queue, &mut pipeline, "abcdefghij", 1.0);
        let steady = pipeline.placement_epoch();
        prepare_label(&device, &queue, &mut pipeline, "abcdefghij", 1.0);
        assert_eq!(
            pipeline.placement_epoch(),
            steady,
            "a repaint that places nothing new must leave retained commands valid"
        );

        for label in [
            "KLMNOPQRST",
            "UVWXYZ0123",
            "456789!?@#",
            "\u{4e2d}\u{6587}\u{6e2c}\u{8a66}\u{6587}\u{5b57}",
        ] {
            prepare_label(&device, &queue, &mut pipeline, label, 1.0);
        }
        assert!(
            pipeline.glyph_counters().glyph_atlas_evict > 0,
            "the corpus must outgrow this page for the test to mean anything"
        );
        assert_ne!(
            pipeline.placement_epoch(),
            steady,
            "an eviction must invalidate commands built against the old placements"
        );
    }

    #[test]
    fn a_corpus_larger_than_the_atlas_keeps_placing_glyphs_and_never_samples_a_stale_one() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::with_atlas_limits(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            GlyphAtlasLimits {
                page_edge: 128,
                byte_budget: 128 * 128 + 8,
            },
        );
        // Far more distinct CJK glyphs than one small page holds, three times
        // over, so the atlas fills, evicts, repacks and refills.
        let corpus: Vec<String> = (0..48)
            .map(|line: u32| {
                (0..8)
                    .filter_map(|index| char::from_u32(0x4e00 + line * 8 + index))
                    .collect()
            })
            .collect();
        for _ in 0..3 {
            for label in &corpus {
                prepare_label(&device, &queue, &mut pipeline, label, 1.0);
            }
        }
        let counters = pipeline.glyph_counters();
        assert!(
            counters.glyph_atlas_evict > 0,
            "the corpus has to outgrow the page for this to test anything"
        );
        assert!(
            counters.glyph_atlas_bytes <= (128 * 128 + 8) as u64,
            "the atlas must stay inside its byte budget, got {} bytes",
            counters.glyph_atlas_bytes
        );
        assert!(
            counters.glyph_raster_cache_hit > 0,
            "a second pass over the corpus must be answered from the raster cache"
        );
        // Every glyph a frame placed is protected from that frame's own
        // eviction, so the flush that turns placements into instances must
        // never meet a handle that has gone stale.
        assert_eq!(
            counters.atlas_stale_handle_rejects, 0,
            "a frame must never build an instance from a placement it lost"
        );
        // One region per glyph faulted in, and not one more: a full page must
        // not repack — and therefore re-upload its whole live set — for every
        // glyph it cannot place.
        assert_eq!(
            counters.glyph_upload_regions, counters.glyph_atlas_miss,
            "a churning atlas must upload each placement once"
        );
    }

    #[test]
    fn a_second_window_on_one_device_reuses_the_first_windows_glyphs() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        prepare_label(&device, &queue, &mut pipeline, "Shared chrome", 1.0);
        let first = pipeline.glyph_counters();
        assert!(first.glyph_rasterized > 0);
        assert!(first.glyph_upload_regions > 0);

        // A second render target on the same device. Its own instance buffers
        // are separate; the atlas and the rasterized glyphs are not.
        let mut second = None;
        pipeline.swap_target(&mut second, &device);
        prepare_label(&device, &queue, &mut pipeline, "Shared chrome", 1.0);
        let shared = pipeline.glyph_counters();
        assert_eq!(
            shared.glyph_rasterized, first.glyph_rasterized,
            "a second window must not rasterize the first window's glyphs again"
        );
        assert_eq!(
            shared.glyph_upload_regions, first.glyph_upload_regions,
            "nor upload them to a second atlas"
        );
        assert_eq!(
            shared.glyph_atlas_hit,
            first.glyph_atlas_hit + first.glyph_atlas_miss,
            "every glyph of the second window must hit the shared atlas"
        );
        assert_eq!(shared.glyph_atlas_pages, first.glyph_atlas_pages);
    }

    #[test]
    fn closing_one_window_leaves_the_others_glyphs_placed() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let mut first = None;
        let mut second = None;

        pipeline.swap_target(&mut first, &device);
        prepare_label(&device, &queue, &mut pipeline, "Shell chrome", 1.0);
        pipeline.swap_target(&mut first, &device);

        pipeline.swap_target(&mut second, &device);
        prepare_label(&device, &queue, &mut pipeline, "Shell chrome", 1.0);
        let shared = pipeline.glyph_counters();
        pipeline.swap_target(&mut second, &device);

        // The first window closes. Its instance buffers go with it; the glyphs
        // it faulted into the shared atlas must not.
        drop(first);

        pipeline.swap_target(&mut second, &device);
        prepare_label(&device, &queue, &mut pipeline, "Shell chrome", 1.0);
        let after = pipeline.glyph_counters();
        assert_eq!(
            after.glyph_rasterized, shared.glyph_rasterized,
            "closing a window must not cost the surviving one a re-rasterization"
        );
        assert_eq!(
            after.glyph_upload_regions, shared.glyph_upload_regions,
            "nor a re-upload"
        );
        assert_eq!(
            after.glyph_atlas_evict, 0,
            "nor drop anything from the shared atlas"
        );
    }

    #[test]
    fn repainting_unchanged_text_uploads_neither_a_region_nor_an_instance() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        prepare_label(&device, &queue, &mut pipeline, "Steady label", 1.0);
        let warm = pipeline.glyph_counters();
        let work = prepare_label(&device, &queue, &mut pipeline, "Steady label", 1.0);
        let steady = pipeline.glyph_counters();
        assert_eq!(
            steady.glyph_upload_regions, warm.glyph_upload_regions,
            "an unchanged repaint must upload no atlas region"
        );
        assert_eq!(
            steady.glyph_rasterized, warm.glyph_rasterized,
            "and rasterize nothing"
        );
        assert_eq!(
            work.gpu_upload_bytes, 0,
            "the instances are byte-identical, so nothing reaches the queue"
        );
        assert_eq!(work.gpu_buffer_reallocations, 0);
    }

    #[test]
    fn a_scale_round_trip_rerasterizes_once_and_then_answers_from_cache() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        prepare_label(&device, &queue, &mut pipeline, "DPI probe", 1.0);
        let one = pipeline.glyph_counters();
        prepare_label(&device, &queue, &mut pipeline, "DPI probe", 2.0);
        let two = pipeline.glyph_counters();
        assert!(
            two.glyph_rasterized > one.glyph_rasterized,
            "a raster scale change must rasterize at the new size"
        );
        // Back again. The raster cache is keyed by size, not by "current
        // scale", so the first scale's bitmaps are still there and the atlas
        // still holds them.
        prepare_label(&device, &queue, &mut pipeline, "DPI probe", 1.0);
        let back = pipeline.glyph_counters();
        assert_eq!(
            back.glyph_rasterized, two.glyph_rasterized,
            "returning to a scale already drawn must not rasterize again"
        );
        assert_eq!(
            back.glyph_upload_regions, two.glyph_upload_regions,
            "nor re-upload"
        );
        let (_, misses, _) = pipeline.shape_cache_stats();
        assert_eq!(
            misses, 2,
            "two raster scales are two shaped paragraphs, and going back is neither"
        );
    }

    /// Paint labels folded into one draw command and read the frame back.
    fn paint_labels(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &mut TextPipeline,
        labels: &[(&str, EntryKey, f32)],
    ) -> Vec<u8> {
        pipeline.begin_frame([256, 64]);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui text label prepare"),
        });
        let mut command: Option<PreparedText> = None;
        for (content, key, top) in labels {
            let prepared = pipeline.prepare(
                device,
                LogicalRect::from_xywh(0.0, *top, 240.0, 32.0),
                LogicalRect::from_xywh(0.0, 0.0, 256.0, 64.0),
                1.0,
                content,
                Some([1.0, 1.0, 1.0, 1.0]),
                16.0,
                None,
                None,
                None,
                false,
                nana_ui_core::TextWrapBreak::Word,
                false,
                false,
                None,
                TextShaping::Auto,
                TextHorizontalAlignment::Start,
                TextVerticalAlignment::Top,
                &[],
                0.0,
                &[],
                &SceneTextOpenType::default(),
                clip::IDENTITY_AFFINE,
                [0.0; 2],
                clip::FragmentClip::PASS,
                1.0,
                [0.0; 2],
                *key,
                UNTRACKED_REVISION,
            );
            let Some(prepared) = prepared else {
                continue;
            };
            match command.as_ref() {
                Some(previous) if pipeline.can_merge_runs(previous, &prepared) => {
                    pipeline.merge_runs(previous, &prepared);
                }
                _ => command = Some(prepared),
            }
        }
        pipeline.flush_runs();
        pipeline.upload(device, queue, None);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-ui text label target"),
            size: wgpu::Extent3d {
                width: 256,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nana-ui text label pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(prepared) = command.as_ref() {
                pipeline.draw(
                    &mut pass,
                    prepared,
                    PhysicalRect {
                        x: 0,
                        y: 0,
                        width: 256,
                        height: 64,
                    },
                    None,
                );
            }
        }
        readback_rgba(device, queue, encoder, &texture, 256, 64)
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_text(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &mut TextPipeline,
        bounds: LogicalRect,
        clip: LogicalRect,
        affine: [f32; 6],
        persp: [f32; 2],
        fragment_clip: clip::FragmentClip,
    ) -> Vec<u8> {
        pipeline.begin_frame([64, 64]);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui text affine prepare"),
        });
        let opentype = SceneTextOpenType::default();
        let prepared = pipeline
            .prepare(
                device,
                bounds,
                clip,
                1.0,
                "Hi",
                Some([1.0, 1.0, 1.0, 1.0]),
                16.0,
                None,
                None,
                None,
                false,
                nana_ui_core::TextWrapBreak::Word,
                false,
                false,
                None,
                TextShaping::Auto,
                TextHorizontalAlignment::Start,
                TextVerticalAlignment::Top,
                &[],
                0.0,
                &[],
                &opentype,
                affine,
                persp,
                fragment_clip,
                1.0,
                [0.0, 0.0],
                EntryKey {
                    node: 1,
                    slot: 0,
                    pass: 0,
                },
                UNTRACKED_REVISION,
            )
            .expect("text must prepare");
        // Placements are handles until the run is flushed; nothing is on the
        // GPU until the instances and the atlas regions are uploaded.
        pipeline.flush_runs();
        pipeline.upload(device, queue, None);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-ui text affine target"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nana-ui text affine pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pipeline.draw(
                &mut pass,
                &prepared,
                PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 64,
                },
                None,
            );
        }
        readback_rgba(device, queue, encoder, &texture, 64, 64)
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_block(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &mut TextPipeline,
        bounds: LogicalRect,
        clip: LogicalRect,
        affine: [f32; 6],
        persp: [f32; 2],
        fragment_clip: clip::FragmentClip,
    ) -> Vec<u8> {
        pipeline.begin_frame([64, 64]);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui text clip prepare"),
        });
        let opentype = SceneTextOpenType::default();
        let prepared = pipeline
            .prepare(
                device,
                bounds,
                clip,
                1.0,
                "HHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHH",
                Some([1.0, 1.0, 1.0, 1.0]),
                16.0,
                None,
                None,
                None,
                true,
                nana_ui_core::TextWrapBreak::Word,
                false,
                false,
                None,
                TextShaping::Auto,
                TextHorizontalAlignment::Start,
                TextVerticalAlignment::Top,
                &[],
                0.0,
                &[],
                &opentype,
                affine,
                persp,
                fragment_clip,
                1.0,
                [0.0, 0.0],
                EntryKey {
                    node: 1,
                    slot: 0,
                    pass: 0,
                },
                UNTRACKED_REVISION,
            )
            .expect("block text must prepare");
        pipeline.flush_runs();
        pipeline.upload(device, queue, None);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-ui text clip target"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nana-ui text clip pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pipeline.draw(
                &mut pass,
                &prepared,
                PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 64,
                },
                None,
            );
        }
        readback_rgba(device, queue, encoder, &texture, 64, 64)
    }

    fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let index = ((y * width + x) * 4) as usize;
        [
            pixels[index],
            pixels[index + 1],
            pixels[index + 2],
            pixels[index + 3],
        ]
    }

    fn inked(color: [u8; 4]) -> bool {
        u16::from(color[0]) + u16::from(color[1]) + u16::from(color[2]) > 24
    }

    fn ink_aabb(pixels: &[u8], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
        let mut min_x = width;
        let mut min_y = height;
        let mut max_x = 0u32;
        let mut max_y = 0u32;
        let mut found = false;
        for y in 0..height {
            for x in 0..width {
                let index = ((y * width + x) * 4) as usize;
                if u16::from(pixels[index])
                    + u16::from(pixels[index + 1])
                    + u16::from(pixels[index + 2])
                    > 24
                {
                    found = true;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        found.then_some((min_x, min_y, max_x, max_y))
    }

    fn readback_rgba(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let unpadded = width as usize * 4;
        let padded = unpadded.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui text affine readback"),
            size: (padded * height as usize) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let submission = queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .expect("text affine readback poll");
        let mapped = slice
            .get_mapped_range()
            .expect("text affine readback must be mapped");
        let mut pixels = Vec::with_capacity(unpadded * height as usize);
        for row in mapped.chunks_exact(padded) {
            pixels.extend_from_slice(&row[..unpadded]);
        }
        pixels
    }

    fn test_device() -> (wgpu::Device, wgpu::Queue) {
        crate::test_gpu::device()
    }
}

#[cfg(test)]
mod placement_size {
    #[test]
    fn a_retained_glyph_stays_twenty_four_bytes() {
        // One per glyph, held between frames and never rewritten while the
        // paragraph, its sub-pixel phase and its atlas placements stand still.
        assert_eq!(
            std::mem::size_of::<super::pipeline::GlyphInstance>(),
            24,
            "the retained glyph is the instance; growing it grows every \
             entry's block and the arena that mirrors it"
        );
        assert_eq!(std::mem::size_of::<super::pipeline::TextRunGpu>(), 48);
        assert_eq!(
            std::mem::size_of::<super::pipeline::TextPresentationGpu>(),
            160
        );
    }
}
