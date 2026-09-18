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

use self::atlas::{AtlasPageKind, GlyphAtlasEntryId, GlyphAtlasLimits, GlyphAtlasManager};
use self::glyph::{GlyphRenderMode, NanaGlyphBuffer, PlacedGlyph, size_bits};
use self::pipeline::{
    AffineVertex, CONTENT_COLOR, CONTENT_MASK, DrawSegment, GlyphInstance, SegmentKind, TextGpu,
    TextTargetGpu,
};
use self::raster::{SwashGlyphRasterizer, synthesis_from_backend};
use self::raster_cache::GlyphRasterCache;
use self::upload::GlyphUploadQueue;

use super::clip::{self, LogicalRect};
use super::color::{pack_linear, to_rgba8, with_opacity};
use crate::PhysicalRect;
use crate::nana_text::{
    RTL_ISOLATE_PREFIX, RTL_ISOLATE_SUFFIX, cosmic_wrap, ellipsize_end, measured_text_overflows,
    shape_attrs, wrap_for_css_direction,
};

const SHAPE_CACHE_CAP: usize = 512;

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
}

struct ShapeEntry {
    key: ShapeKey,
    buffer: Buffer,
}

/// Shaped buffers keyed by [`ShapeKeyRef::hash64`].
///
/// The map is keyed by the hash rather than by an owned key so a repaint of
/// unchanged text looks up without copying the string, the family name, or the
/// rich-span list. The stored key still decides the hit, so a hash collision
/// between two different texts reshapes instead of painting the wrong glyphs.
#[derive(Default)]
struct ShapeCache {
    entries: HashMap<u64, ShapeEntry>,
    order: VecDeque<u64>,
    hits: usize,
    misses: usize,
    evictions: usize,
}

impl ShapeCache {
    fn get(&mut self, hash: u64, key: &ShapeKeyRef<'_>) -> Option<&Buffer> {
        match self.entries.get(&hash) {
            Some(entry) if entry.key.matches(key) => {
                self.hits += 1;
                Some(&entry.buffer)
            }
            _ => {
                self.misses += 1;
                None
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

    fn insert(&mut self, hash: u64, key: ShapeKey, buffer: Buffer) {
        while self.entries.len() >= SHAPE_CACHE_CAP {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if self.entries.remove(&oldest).is_some() {
                self.evictions += 1;
            }
        }
        if self
            .entries
            .insert(hash, ShapeEntry { key, buffer })
            .is_none()
        {
            self.order.push_back(hash);
        }
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
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
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

/// One glyph as the frame retains it: a handle, not coordinates.
///
/// The atlas may relocate or evict between this being recorded and the draw
/// that uses it, so the rectangle is read back through the handle at flush
/// time. A frame that baked UVs here would sample a neighbour's glyph after a
/// compaction — the exact ABA the handle exists to rule out.
#[derive(Clone, Copy, Debug)]
struct RunPlacement {
    entry: GlyphAtlasEntryId,
    /// Quad top-left in physical pixels. World space for an axis run, the
    /// pre-transform space of its node for an affine one.
    origin: [f32; 2],
    color: [f32; 4],
}

/// How a run reaches the screen.
#[derive(Clone, Copy, Debug, Default)]
enum RunTransform {
    /// Translation only: the batch's scissor is the whole clip.
    #[default]
    Axis,
    /// The same homography and fragment clip as `Quad`, per glyph corner.
    Affine {
        affine: [f32; 6],
        persp: [f32; 2],
        clip: clip::FragmentClip,
        scale: f32,
    },
}

/// One text draw command's glyphs, before they become instances.
///
/// Pooled across frames: `runs` keeps its entries and `live_runs` says how
/// many this frame uses, so a shell with two hundred labels reuses two hundred
/// placement buffers instead of allocating them every repaint.
#[derive(Default)]
struct TextRun {
    transform: RunTransform,
    placements: Vec<RunPlacement>,
    segments: Range<u32>,
}

pub(super) struct PreparedText {
    pub index: usize,
    /// Local-space rectangle the glyphs can cover, `bounds` overflow included.
    pub ink: LogicalRect,
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
    /// Pooled run storage; only the first `live_runs` are this frame's.
    runs: Vec<TextRun>,
    live_runs: usize,
    /// How many runs have already been turned into instances. Runs below this
    /// are closed and can no longer be extended.
    flushed: usize,
    segments: Vec<DrawSegment>,
    instances: Vec<GlyphInstance>,
    uploaded_instances: Vec<GlyphInstance>,
    vertices: Vec<AffineVertex>,
    uploaded_vertices: Vec<AffineVertex>,
    physical_size: [u32; 2],
    /// The font-set generation this painter's caches were filled at. A
    /// `@font-face` registration reissues faces, so both the shaped paragraphs
    /// and the glyph bitmaps stop meaning what they meant.
    font_generation: u64,
    resolve_requests: u64,
    draws: Cell<u64>,
    /// GPU allocations this frame could not avoid. Drained into the host's
    /// observed GPU work so a regression shows up as per-frame resource
    /// creation instead of only as a slower frame.
    frame_gpu_allocations: usize,
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
        Self {
            font_system: crate::nana_text::nana_font_system(),
            rasterizer: SwashGlyphRasterizer::new(crate::nana_text::nana_font_system()),
            raster,
            atlas,
            uploads: GlyphUploadQueue::default(),
            gpu,
            shape_cache: ShapeCache::default(),
            resolved: NanaGlyphBuffer::default(),
            runs: Vec::new(),
            live_runs: 0,
            flushed: 0,
            segments: Vec::new(),
            instances: Vec::new(),
            uploaded_instances: Vec::new(),
            vertices: Vec::new(),
            uploaded_vertices: Vec::new(),
            physical_size: [0; 2],
            font_generation: crate::nana_text::font_db_generation(),
            resolve_requests: 0,
            draws: Cell::new(0),
            frame_gpu_allocations: 0,
        }
    }

    pub(super) fn begin_frame(&mut self, physical_size: [u32; 2]) {
        let generation = crate::nana_text::font_db_generation();
        if generation != self.font_generation {
            // Faces were added, replaced or removed. Shaped paragraphs named
            // the old face set and glyph bitmaps were scaled from it, so both
            // are dropped rather than left to be keyed around.
            self.font_generation = generation;
            self.shape_cache.clear();
            self.raster.invalidate();
        }
        self.atlas.begin_frame(self.raster.generation());
        // Truncate logically: the placement buffers are the pool.
        for run in &mut self.runs[..self.live_runs] {
            run.placements.clear();
        }
        self.live_runs = 0;
        self.flushed = 0;
        self.segments.clear();
        self.instances.clear();
        self.vertices.clear();
        self.frame_gpu_allocations = 0;
        self.physical_size = physical_size;
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
        }
    }

    /// GPU allocations this frame's text could not reuse.
    pub(super) fn take_frame_gpu_allocations(&mut self) -> usize {
        let pipeline = self.gpu.take_allocations();
        std::mem::take(&mut self.frame_gpu_allocations) + pipeline
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
        let default_color = with_opacity(color.unwrap_or([0.0, 0.0, 0.0, 1.0]), opacity);
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
        let painted = presentation_spans(content, spans, default_color, opacity);
        let rich = painted.len() > 1 || painted.first().is_some_and(|span| span.1 != default_color);
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
        let (measured_width, laid_out_height) = {
            let buffer = self.shape_cache.buffer(hash).expect("shaped above");
            measure(buffer)
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
        // An axis-aligned run is clipped by the batch's scissor, which is this
        // same rect. Rotated or projective text must go through the glyph-quad
        // path so the same homography as Quad is applied to each glyph
        // (4 corners, no triangulation), and a rounded clip needs the fragment
        // test the scissor cannot express.
        let (origin, transform, visible) = if clip::is_translation_projective(affine, persp)
            && fragment_clip == clip::FragmentClip::PASS
        {
            let [world_x, world_y] = clip::transform_point(affine, aligned[0], aligned[1]);
            // The same rectangle [`super::physical_scissor`] will set, so a
            // glyph dropped here is exactly one the scissor would have
            // discarded — text does not clip tighter than its siblings.
            let visible = [
                (clip.x * scale).floor() as i32,
                (clip.y * scale).floor() as i32,
                ((clip.x + clip.width) * scale).ceil() as i32,
                ((clip.y + clip.height) * scale).ceil() as i32,
            ];
            (
                [world_x * scale, world_y * scale],
                RunTransform::Axis,
                Some(visible),
            )
        } else {
            (
                [aligned[0] * scale, aligned[1] * scale],
                RunTransform::Affine {
                    affine,
                    persp,
                    clip: fragment_clip,
                    scale,
                },
                None,
            )
        };
        let index = self.live_runs;
        if index == self.runs.len() {
            self.runs.push(TextRun::default());
        }
        if !self.resolve_runs(device, hash, origin, default_color, visible, index) {
            return None;
        }
        self.runs[index].transform = transform;
        self.runs[index].segments = 0..0;
        self.live_runs += 1;
        Some(PreparedText { index, ink })
    }

    /// Turn one shaped paragraph into placed, atlas-resident glyphs.
    ///
    /// This is the only function that knows how the paragraph was laid out.
    /// Everything it returns is in the renderer's own terms: a handle per
    /// glyph, its quad origin in physical pixels, and its color.
    fn resolve_runs(
        &mut self,
        device: &wgpu::Device,
        hash: u64,
        origin: [f32; 2],
        default_color: [f32; 4],
        visible: Option<[i32; 4]>,
        run_index: usize,
    ) -> bool {
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
            // A layout run wholly outside the clip band emits no glyph, so
            // resolving it would cost a raster and an instance that never
            // reach a pixel. Runs are ordered in y, so this is the same
            // predicate the reference renderer applied.
            if let Some([_, top, _, bottom]) = visible {
                let start = (origin[1] + run.line_top) as i32;
                let end = start + run.line_height as i32;
                if start > bottom || end < top {
                    continue;
                }
            }
            let line_y = run.line_y.round();
            for glyph in run.glyphs {
                let font_size = glyph.font_size;
                let x = font_size.mul_add(glyph.x_offset, glyph.x) + origin[0];
                // Y is hinted to whole pixels before the line origin is added,
                // which is what keeps a baseline from landing between texels.
                let y = (font_size.mul_add(-glyph.y_offset, glyph.y) + origin[1]).trunc() + line_y;
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
            return false;
        }
        let Self {
            resolved,
            rasterizer,
            raster,
            atlas,
            uploads,
            runs,
            resolve_requests,
            frame_gpu_allocations,
            ..
        } = self;
        let pages_before = atlas.page_count();
        let placements = &mut runs[run_index].placements;
        placements.reserve(resolved.glyphs.len());
        for run in &resolved.runs {
            for placed in resolved.glyphs_of(run) {
                *resolve_requests += 1;
                let (key, pen) = run.raster_key(placed);
                let (entry, placement) = match atlas.lookup(&key) {
                    Some(placed) => placed,
                    None => {
                        let Some(image) = raster.get_or_rasterize(rasterizer, key) else {
                            continue;
                        };
                        match atlas.insert(device, key, &image, raster, uploads) {
                            Some(placed) => placed,
                            None => continue,
                        }
                    }
                };
                let origin = [pen[0] + placement.left, pen[1] - placement.top];
                // A glyph the scissor would discard costs an instance and a
                // rasterizer thread's worth of vertex work for nothing. One
                // unwrapped line in a narrow box is hundreds of them.
                let size = [placement.size[0] as i32, placement.size[1] as i32];
                if let Some([left, top, right, bottom]) = visible
                    && (origin[0] > right
                        || origin[0] + size[0] < left
                        || origin[1] > bottom
                        || origin[1] + size[1] < top)
                {
                    continue;
                }
                placements.push(RunPlacement {
                    entry,
                    origin: [origin[0] as f32, origin[1] as f32],
                    color: run.color,
                });
            }
        }
        *frame_gpu_allocations += atlas.page_count() - pages_before;
        !placements.is_empty()
    }

    /// Fold the run just opened by `next` into `previous`, so both draw as one
    /// command. Returns `false` when the two cannot share a run and the caller
    /// must keep `next` as its own command.
    ///
    /// Mirrors [`super::push_icon`] / [`super::push_quad`]: only runs that are
    /// already neighbours in document order merge, and glyph order inside the
    /// merged run is placement order, so a text shadow still paints under the
    /// text it belongs to.
    pub(super) fn can_merge_runs(&self, previous: &PreparedText, next: &PreparedText) -> bool {
        // Only axis runs merge: an affine run carries its node's homography,
        // and two nodes' transforms cannot be one draw.
        matches!(
            self.runs.get(previous.index).map(|run| &run.transform),
            Some(RunTransform::Axis)
        ) && matches!(
            self.runs.get(next.index).map(|run| &run.transform),
            Some(RunTransform::Axis)
        )
            // `next` must be the run just opened, so folding it away is a pop.
            && next.index + 1 == self.live_runs
            && previous.index < next.index
            && previous.index >= self.flushed
    }

    pub(super) fn merge_runs(&mut self, previous: &PreparedText, next: &PreparedText) {
        debug_assert!(self.can_merge_runs(previous, next));
        let (kept, folded) = self.runs.split_at_mut(next.index);
        kept[previous.index]
            .placements
            .append(&mut folded[0].placements);
        self.live_runs -= 1;
    }

    /// Turn every still-open run's placements into draw segments. Must run
    /// before `upload` and before `draw`.
    pub(super) fn flush_runs(&mut self) {
        if self.flushed >= self.live_runs {
            return;
        }
        let Self {
            atlas,
            runs,
            live_runs,
            flushed,
            segments,
            instances,
            vertices,
            ..
        } = self;
        for run in &mut runs[*flushed..*live_runs] {
            let first_segment = segments.len() as u32;
            let kind = match run.transform {
                RunTransform::Axis => SegmentKind::Axis,
                RunTransform::Affine { .. } => SegmentKind::Affine,
            };
            // A segment has to name a page of each kind because one bind group
            // does, but only the kinds it actually samples are constrained. A
            // segment still holding a placeholder has not sampled that kind
            // yet, so the first glyph of it adopts a page instead of splitting
            // — which is what keeps one emoji in a line of text free.
            let placeholders = [
                atlas.placeholder_page(AtlasPageKind::Mask),
                atlas.placeholder_page(AtlasPageKind::Color),
            ];
            let mut open: Option<DrawSegment> = None;
            for placement in &run.placements {
                // Read the rectangle now, not when it was placed: the atlas
                // may have relocated this glyph while a later paragraph in the
                // same frame was faulting glyphs in.
                let Some(entry) = atlas.entry(placement.entry) else {
                    continue;
                };
                let (content, mask_page, color_page) = match entry.kind {
                    AtlasPageKind::Mask => (CONTENT_MASK, Some(entry.page), None),
                    AtlasPageKind::Color => (CONTENT_COLOR, None, Some(entry.page)),
                };
                // Two pages of one kind in one run is the only thing that
                // splits a segment, and it splits rather than regroups, so
                // glyph order inside a run stays document order.
                let compatible = open.as_ref().is_some_and(|segment| {
                    mask_page.is_none_or(|page| {
                        segment.mask_page == page || segment.mask_page == placeholders[0]
                    }) && color_page.is_none_or(|page| {
                        segment.color_page == page || segment.color_page == placeholders[1]
                    })
                });
                if !compatible && let Some(segment) = open.take() {
                    segments.push(segment);
                }
                let cursor = match kind {
                    SegmentKind::Axis => instances.len() as u32,
                    SegmentKind::Affine => vertices.len() as u32,
                };
                let segment = open.get_or_insert(DrawSegment {
                    kind,
                    mask_page: placeholders[0],
                    color_page: placeholders[1],
                    first: cursor,
                    count: 0,
                });
                if let Some(page) = mask_page {
                    segment.mask_page = page;
                }
                if let Some(page) = color_page {
                    segment.color_page = page;
                }
                match run.transform {
                    RunTransform::Axis => {
                        instances.push(GlyphInstance::new(
                            [placement.origin[0] as i32, placement.origin[1] as i32],
                            entry.size,
                            entry.origin,
                            placement.color,
                            content,
                        ));
                        segment.count += 1;
                    }
                    RunTransform::Affine {
                        affine,
                        persp,
                        clip: fragment_clip,
                        scale,
                    } => {
                        push_affine_glyph(
                            vertices,
                            placement,
                            entry,
                            affine,
                            persp,
                            fragment_clip,
                            scale,
                            content,
                        );
                        segment.count += 6;
                    }
                }
            }
            if let Some(segment) = open.take() {
                segments.push(segment);
            }
            run.segments = first_segment..segments.len() as u32;
        }
        *flushed = *live_runs;
    }

    /// Write this frame's atlas regions, instances and vertices.
    pub(super) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        self.uploads.flush(queue, &self.atlas, work);
        // Bind groups are created here, where the atlas is still `&mut`, so
        // `draw` only has to look one up.
        let Self {
            atlas, segments, ..
        } = self;
        for segment in segments.iter() {
            atlas.bind_group(device, segment.mask_page, segment.color_page);
        }
        self.gpu.upload(
            device,
            queue,
            self.physical_size,
            &self.instances,
            &self.vertices,
            &self.uploaded_instances,
            &self.uploaded_vertices,
            work,
        );
        self.uploaded_instances.clone_from(&self.instances);
        self.uploaded_vertices.clone_from(&self.vertices);
    }

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        prepared: &PreparedText,
        scissor: PhysicalRect,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let Some(run) = self
            .runs
            .get(prepared.index)
            .filter(|_| prepared.index < self.live_runs)
        else {
            return;
        };
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        let start = run.segments.start as usize;
        let end = (run.segments.end as usize).min(self.segments.len());
        let mut drawn = 0u64;
        for segment in &self.segments[start.min(end)..end] {
            let Some(bind_group) = self
                .atlas
                .cached_bind_group(segment.mask_page, segment.color_page)
            else {
                continue;
            };
            self.gpu.draw_segment(pass, segment, bind_group);
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
}

/// Six vertices of one glyph quad under the run's homography.
#[allow(clippy::too_many_arguments)]
fn push_affine_glyph(
    vertices: &mut Vec<AffineVertex>,
    placement: &RunPlacement,
    entry: &atlas::AtlasEntry,
    affine: [f32; 6],
    persp: [f32; 2],
    fragment_clip: clip::FragmentClip,
    scale: f32,
    content: u32,
) {
    let clip = fragment_clip.for_physical_pixels(scale);
    // The quad is transformed in logical space, like every other Scene
    // primitive, and scaled back to physical afterwards.
    let x = placement.origin[0] / scale;
    let y = placement.origin[1] / scale;
    let w = entry.size[0] as f32 / scale;
    let h = entry.size[1] as f32 / scale;
    let [tl, tr, bl, br] = transform_glyph_quad(affine, persp, x, y, w, h);
    let u0 = entry.origin[0] as f32;
    let v0 = entry.origin[1] as f32;
    let u1 = u0 + entry.size[0] as f32;
    let v1 = v0 + entry.size[1] as f32;
    let color = if content == CONTENT_COLOR {
        // A color bitmap carries its own color; only the run's alpha applies.
        [1.0, 1.0, 1.0, placement.color[3]]
    } else {
        pack_linear(placement.color)
    };
    for (position, uv) in [
        (tl, [u0, v0]),
        (tr, [u1, v0]),
        (bl, [u0, v1]),
        (tr, [u1, v0]),
        (br, [u1, v1]),
        (bl, [u0, v1]),
    ] {
        vertices.push(AffineVertex::new(
            [position[0] * scale, position[1] * scale],
            uv,
            color,
            &clip,
            content,
        ));
    }
}
fn presentation_spans<'a>(
    content: &'a str,
    spans: &'a [SceneTextSpan],
    default: [f32; 4],
    opacity: f32,
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
        painted.push((
            &content[span.start..span.end],
            with_opacity(span.color, opacity),
        ));
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

/// Four corners of a glyph quad after the same Scene homography as Quad.
fn transform_glyph_quad(
    affine: [f32; 6],
    persp: [f32; 2],
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> [[f32; 2]; 4] {
    [
        clip::transform_point_projective(affine, persp, x, y),
        clip::transform_point_projective(affine, persp, x + w, y),
        clip::transform_point_projective(affine, persp, x, y + h),
        clip::transform_point_projective(affine, persp, x + w, y + h),
    ]
}

#[cfg(test)]
fn quad_aabb(corners: &[[f32; 2]]) -> LogicalRect {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for [x, y] in corners {
        min_x = min_x.min(*x);
        min_y = min_y.min(*y);
        max_x = max_x.max(*x);
        max_y = max_y.max(*y);
    }
    LogicalRect::from_xywh(
        min_x,
        min_y,
        (max_x - min_x).max(0.0),
        (max_y - min_y).max(0.0),
    )
}

/// Per-target text state: the buffers this frame's glyphs land in and the
/// draw commands that name them.
///
/// The atlas, the raster cache and the shaped paragraphs are **not** here.
/// They belong to the device context, so a second window on the same device
/// reuses every glyph the first one faulted in rather than filling a second
/// atlas with the same shell chrome.
pub(super) struct TextPipelineTarget {
    gpu: TextTargetGpu,
    runs: Vec<TextRun>,
    live_runs: usize,
    flushed: usize,
    segments: Vec<DrawSegment>,
    instances: Vec<GlyphInstance>,
    uploaded_instances: Vec<GlyphInstance>,
    vertices: Vec<AffineVertex>,
    uploaded_vertices: Vec<AffineVertex>,
    physical_size: [u32; 2],
    frame_gpu_allocations: usize,
}

impl TextPipeline {
    pub(super) fn swap_target(
        &mut self,
        target: &mut Option<TextPipelineTarget>,
        device: &wgpu::Device,
    ) {
        let target = target.get_or_insert_with(|| TextPipelineTarget {
            gpu: self.gpu.new_target(device),
            runs: Vec::new(),
            live_runs: 0,
            flushed: 0,
            segments: Vec::new(),
            instances: Vec::new(),
            uploaded_instances: Vec::new(),
            vertices: Vec::new(),
            uploaded_vertices: Vec::new(),
            physical_size: [0; 2],
            frame_gpu_allocations: 0,
        });
        std::mem::swap(&mut self.gpu.target, &mut target.gpu);
        std::mem::swap(&mut self.runs, &mut target.runs);
        std::mem::swap(&mut self.live_runs, &mut target.live_runs);
        std::mem::swap(&mut self.flushed, &mut target.flushed);
        std::mem::swap(&mut self.segments, &mut target.segments);
        std::mem::swap(&mut self.instances, &mut target.instances);
        std::mem::swap(&mut self.uploaded_instances, &mut target.uploaded_instances);
        std::mem::swap(&mut self.vertices, &mut target.vertices);
        std::mem::swap(&mut self.uploaded_vertices, &mut target.uploaded_vertices);
        std::mem::swap(&mut self.physical_size, &mut target.physical_size);
        std::mem::swap(
            &mut self.frame_gpu_allocations,
            &mut target.frame_gpu_allocations,
        );
    }
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
    fn glyph_quads_follow_90_degree_affine() {
        let identity = clip::IDENTITY_AFFINE;
        let rot90 = [0.0, 1.0, -1.0, 0.0, 0.0, 0.0];
        let unrotated = transform_glyph_quad(identity, [0.0, 0.0], 10.0, 20.0, 30.0, 8.0);
        let rotated = transform_glyph_quad(rot90, [0.0, 0.0], 10.0, 20.0, 30.0, 8.0);
        let unrotated_bounds = quad_aabb(&unrotated);
        let rotated_bounds = quad_aabb(&rotated);
        assert_ne!(
            (
                unrotated_bounds.x,
                unrotated_bounds.y,
                unrotated_bounds.width,
                unrotated_bounds.height
            ),
            (
                rotated_bounds.x,
                rotated_bounds.y,
                rotated_bounds.width,
                rotated_bounds.height
            ),
            "90° affine must not leave the unrotated AABB"
        );
        assert!(
            unrotated_bounds.width > unrotated_bounds.height,
            "unrotated glyph run is wide, got {unrotated_bounds:?}"
        );
        assert!(
            rotated_bounds.height > rotated_bounds.width,
            "90° glyph quads must swap into a tall AABB, got {rotated_bounds:?}"
        );
        assert_eq!(rotated[0], [-20.0, 10.0]);
        assert_eq!(rotated[1], [-20.0, 40.0]);
        assert_eq!(rotated[2], [-28.0, 10.0]);
        assert_eq!(rotated[3], [-28.0, 40.0]);
    }

    #[test]
    fn glyph_quads_follow_perspective_rotate_y_homography() {
        let mat = nana_ui_core::PaintMat4::perspective(800.0)
            .expect("d")
            .then(nana_ui_core::PaintMat4::rotate_y(30_f32.to_radians()))
            .around_origin(0.0, 0.0, 100.0, 40.0);
        let (affine, persp) = mat.planar_homography().expect("homography");
        let corners = transform_glyph_quad(affine, persp, 0.0, 0.0, 200.0, 80.0);
        let left = {
            let dx = corners[0][0] - corners[2][0];
            let dy = corners[0][1] - corners[2][1];
            (dx * dx + dy * dy).sqrt()
        };
        let right = {
            let dx = corners[1][0] - corners[3][0];
            let dy = corners[1][1] - corners[3][1];
            (dx * dx + dy * dy).sqrt()
        };
        assert!(
            (left - right).abs() > 4.0,
            "text glyph quads must share the box homography, left={left} right={right}"
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
            clip::FragmentClip::PASS,
        );
        let rotated_pixels = paint_text(
            &device,
            &queue,
            &mut pipeline,
            bounds,
            clip,
            rot90,
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
            clip::FragmentClip::PASS,
        );
        let clipped = paint_block(
            &device,
            &queue,
            &mut pipeline,
            bounds,
            aabb,
            clip::IDENTITY_AFFINE,
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
    ) -> GlyphAtlasEntryId {
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

    #[test]
    fn a_color_glyph_between_mask_glyphs_still_draws_as_one_batch() {
        let (device, queue) = test_device();
        let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        pipeline.begin_frame([64, 64]);
        let entries = [
            place(&mut pipeline, &device, 1, raster::GlyphImageFormat::Mask),
            place(
                &mut pipeline,
                &device,
                2,
                raster::GlyphImageFormat::ColorRgba,
            ),
            place(&mut pipeline, &device, 3, raster::GlyphImageFormat::Mask),
        ];
        pipeline.runs.push(TextRun {
            transform: RunTransform::Axis,
            placements: entries
                .iter()
                .enumerate()
                .map(|(index, entry)| RunPlacement {
                    entry: *entry,
                    origin: [index as f32 * 6.0, 0.0],
                    color: [1.0; 4],
                })
                .collect(),
            segments: 0..0,
        });
        pipeline.live_runs = 1;
        pipeline.flush_runs();
        assert_eq!(
            pipeline.runs[0].segments.len(),
            1,
            "a mask page and a color page fit one bind group, so an emoji in a \
             line of text must not split the batch"
        );
        assert_eq!(pipeline.instances.len(), 3);
        let segment = pipeline.segments[0];
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

    fn paint_text(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &mut TextPipeline,
        bounds: LogicalRect,
        clip: LogicalRect,
        affine: [f32; 6],
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
                [0.0, 0.0],
                fragment_clip,
                1.0,
                [0.0, 0.0],
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

    fn paint_block(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &mut TextPipeline,
        bounds: LogicalRect,
        clip: LogicalRect,
        affine: [f32; 6],
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
                [0.0, 0.0],
                fragment_clip,
                1.0,
                [0.0, 0.0],
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
