use bytemuck::{Pod, Zeroable};
use cosmic_text::{Align, Attrs, Buffer, Color, Metrics, Shaping, SwashCache, SwashContent, Wrap};
use nana_ui_core::LineHeightSpec;
use nana_ui_runtime::{TextHorizontalAlignment, TextShaping, TextVerticalAlignment};
use nana_ui_scene::SceneTextSpan;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

use super::clip::{self, LogicalRect};
use super::color::{orthographic, pack_linear, to_rgba8, with_opacity};
use crate::PhysicalRect;
use crate::nana_text::{letter_spacing_em, resolve_family};

const SHAPE_CACHE_CAP: usize = 512;
const AFFINE_CACHE_CAP: usize = 128;
const ATLAS_ROW_ALIGN: u32 = 64;

const AFFINE_GLYPH_SHADER: &str = concat!(
    include_str!("shader/color.wgsl"),
    r#"
struct Globals {
    transform: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> globals: Globals;

@group(1) @binding(0)
var atlas: texture_2d<f32>;

@group(1) @binding(1)
var atlas_sampler: sampler;

struct VsIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) clip_rect: vec4<f32>,
    @location(4) clip_inv_abcd: vec4<f32>,
    @location(5) clip_inv_ef: vec2<f32>,
}

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world_pos: vec2<f32>,
    @location(3) clip_rect: vec4<f32>,
    @location(4) clip_inv_abcd: vec4<f32>,
    @location(5) clip_inv_ef: vec2<f32>,
}

@vertex
fn vs_main(input: VsIn) -> VsOut {
    var out: VsOut;
    out.position = globals.transform * vec4<f32>(input.position, 0.0, 1.0);
    out.uv = input.uv;
    out.color = input.color;
    out.world_pos = input.position;
    out.clip_rect = input.clip_rect;
    out.clip_inv_abcd = input.clip_inv_abcd;
    out.clip_inv_ef = input.clip_inv_ef;
    return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
    if !inside_transformed_rect(
        input.world_pos,
        input.clip_rect,
        input.clip_inv_abcd,
        input.clip_inv_ef
    ) {
        discard;
    }
    let sampled = textureSample(atlas, atlas_sampler, input.uv);
    return vec4<f32>(sampled.rgb * input.color.rgb, sampled.a * input.color.a);
}
"#
);

pub(super) struct TextPipeline {
    /// Shared with Runtime shaping; see [`crate::nana_text::nana_font_system`].
    font_system: crate::nana_text::SharedFontSystem,
    #[allow(dead_code)]
    cache: cryoglyph::Cache,
    atlas: cryoglyph::TextAtlas,
    viewport: cryoglyph::Viewport,
    swash: SwashCache,
    renderers: Vec<cryoglyph::TextRenderer>,
    affine: AffineGlyphPipeline,
    affine_cache: AffineCache,
    frame: u64,
    /// Shaped paragraphs reused across frames. Shaping is the dominant CPU
    /// cost of a text-heavy frame and identical text+style+box repeats on
    /// every repaint (scroll, hover, unrelated animations), so the shaped
    /// `Buffer` is cached and only glyph vertices are regenerated per frame.
    shape_cache: ShapeCache,
    frame_texts: usize,
    prev_frame_texts: usize,
    frame_affines: usize,
    prev_frame_affines: usize,
    /// GPU allocations the affine cache could not avoid this frame. Drained into
    /// the host's observed GPU work so a cache regression shows up as per-frame
    /// resource creation instead of only as a slower frame.
    frame_gpu_allocations: usize,
}

struct AffineGlyphPipeline {
    pipeline: wgpu::RenderPipeline,
    atlas_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
}

struct AffineSlot {
    _atlas: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    vertex_count: u32,
}

/// Everything that determines an affine text node's rasterized atlas and its
/// vertex buffer. When all of it repeats, the GPU resources are byte-identical
/// and are reused instead of rebuilt.
///
/// Rotated or scaled text takes the affine path on every repaint, so without
/// this key a spinning label allocated a texture, a bind group and a vertex
/// buffer every frame.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct AffineKey {
    /// Shaped-output identity, as [`ShapeKeyRef::hash64`].
    shape: u64,
    origin_bits: [u32; 2],
    scale_bits: u32,
    affine_bits: [u32; 6],
    clip_bits: [u32; 12],
    color_bits: [u32; 4],
}

struct AffineEntry {
    key: AffineKey,
    slot: AffineSlot,
    /// Frame that last used this entry. Entries the frame in flight already
    /// handed out as a `PreparedText` index are never evicted.
    frame: u64,
}

/// Affine text GPU resources reused across frames, evicted least-recently-used.
#[derive(Default)]
struct AffineCache {
    /// Stable slot storage. `PreparedText::index` addresses this directly, so
    /// an evicted entry clears its slot rather than shifting the vector.
    slots: Vec<Option<AffineEntry>>,
    index: HashMap<AffineKey, usize>,
    hits: usize,
    misses: usize,
    evictions: usize,
}

impl AffineCache {
    fn get(&mut self, key: &AffineKey, frame: u64) -> Option<usize> {
        let Some(&slot) = self.index.get(key) else {
            self.misses += 1;
            return None;
        };
        self.hits += 1;
        if let Some(entry) = self.slots.get_mut(slot).and_then(Option::as_mut) {
            entry.frame = frame;
        }
        Some(slot)
    }

    fn insert(&mut self, key: AffineKey, slot: AffineSlot, frame: u64) -> usize {
        let entry = AffineEntry { key, slot, frame };
        let index = match self.reclaim(frame) {
            Some(index) => {
                self.slots[index] = Some(entry);
                index
            }
            None => {
                self.slots.push(Some(entry));
                self.slots.len() - 1
            }
        };
        self.index.insert(key, index);
        index
    }

    /// Slot to overwrite once the cache is full: the least recently used entry
    /// the frame in flight is not holding. Eviction only runs at the cap, so
    /// the scan is off the per-glyph path, and a frame with more affine text
    /// than the cap keeps every live entry and grows instead.
    fn reclaim(&mut self, frame: u64) -> Option<usize> {
        if self.index.len() < AFFINE_CACHE_CAP {
            return None;
        }
        let (index, key) = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.as_ref().map(|entry| (index, entry)))
            .filter(|(_, entry)| entry.frame != frame)
            .min_by_key(|(_, entry)| entry.frame)
            .map(|(index, entry)| (index, entry.key))?;
        self.index.remove(&key);
        self.slots[index] = None;
        self.evictions += 1;
        Some(index)
    }

    fn slot(&self, index: usize) -> Option<&AffineSlot> {
        self.slots
            .get(index)
            .and_then(Option::as_ref)
            .map(|entry| &entry.slot)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct AffineUniforms {
    transform: [f32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GlyphVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    clip_rect: [f32; 4],
    clip_inv_abcd: [f32; 4],
    clip_inv_ef: [f32; 2],
}

struct PackedGlyph {
    logical: LogicalRect,
    color: [f32; 4],
    atlas_x: u32,
    atlas_y: u32,
    pixel_w: u32,
    pixel_h: u32,
    rgba: Vec<u8>,
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
    ellipsis: bool,
    shaping: u8,
    letter_spacing_bits: u32,
    width_bits: u32,
    height_bits: u32,
    align: u8,
    spans: Option<Vec<(String, [u32; 4])>>,
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
    ellipsis: bool,
    shaping: u8,
    letter_spacing_bits: u32,
    width_bits: u32,
    height_bits: u32,
    align: u8,
    /// `Some` only for rich text, whose span colors change the shaped attrs.
    spans: Option<&'a [(&'a str, [f32; 4])]>,
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
        self.ellipsis.hash(&mut hasher);
        self.shaping.hash(&mut hasher);
        self.letter_spacing_bits.hash(&mut hasher);
        self.width_bits.hash(&mut hasher);
        self.height_bits.hash(&mut hasher);
        self.align.hash(&mut hasher);
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
            ellipsis: self.ellipsis,
            shaping: self.shaping,
            letter_spacing_bits: self.letter_spacing_bits,
            width_bits: self.width_bits,
            height_bits: self.height_bits,
            align: self.align,
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
            && self.ellipsis == other.ellipsis
            && self.shaping == other.shaping
            && self.letter_spacing_bits == other.letter_spacing_bits
            && self.width_bits == other.width_bits
            && self.height_bits == other.height_bits
            && self.align == other.align
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

pub(super) struct PreparedText {
    pub index: usize,
    kind: PreparedKind,
}

enum PreparedKind {
    Cryoglyph,
    Affine,
}

impl TextPipeline {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let cache = cryoglyph::Cache::new(device);
        let atlas = cryoglyph::TextAtlas::with_color_mode(
            device,
            queue,
            &cache,
            format,
            cryoglyph::ColorMode::Accurate,
        );
        let viewport = cryoglyph::Viewport::new(device, &cache);
        Self {
            font_system: crate::nana_text::nana_font_system(),
            cache,
            atlas,
            viewport,
            swash: SwashCache::new(),
            renderers: Vec::new(),
            affine: AffineGlyphPipeline::new(device, format),
            affine_cache: AffineCache::default(),
            frame: 0,
            shape_cache: ShapeCache::default(),
            frame_texts: 0,
            prev_frame_texts: 0,
            frame_affines: 0,
            prev_frame_affines: 0,
            frame_gpu_allocations: 0,
        }
    }

    pub(super) fn begin_frame(&mut self, queue: &wgpu::Queue, physical_size: [u32; 2]) {
        self.prev_frame_texts = self.frame_texts;
        self.frame_texts = 0;
        self.prev_frame_affines = self.frame_affines;
        self.frame_affines = 0;
        self.frame_gpu_allocations = 0;
        self.frame = self.frame.wrapping_add(1);
        // Renderer high-water decay: keep the GPU-side working set near the
        // last frame's text count instead of retaining a peak forever.
        let keep = self.prev_frame_texts + 8;
        if self.renderers.len() > keep {
            self.renderers.truncate(keep);
        }
        self.viewport.update(
            queue,
            cryoglyph::Resolution {
                width: physical_size[0].max(1),
                height: physical_size[1].max(1),
            },
        );
        let uniforms = AffineUniforms {
            transform: orthographic(physical_size[0], physical_size[1]),
        };
        queue.write_buffer(&self.affine.uniforms, 0, bytemuck::bytes_of(&uniforms));
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

    /// Affine glyph resource cache counters: (hits, misses, evictions). A miss
    /// is one atlas texture, bind group, and vertex buffer creation.
    pub(super) fn affine_cache_stats(&self) -> (usize, usize, usize) {
        (
            self.affine_cache.hits,
            self.affine_cache.misses,
            self.affine_cache.evictions,
        )
    }

    /// GPU allocations this frame's affine text could not reuse.
    pub(super) fn take_frame_gpu_allocations(&mut self) -> usize {
        std::mem::take(&mut self.frame_gpu_allocations)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
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
        ellipsis: bool,
        shaping: TextShaping,
        horizontal: TextHorizontalAlignment,
        vertical: TextVerticalAlignment,
        spans: &[SceneTextSpan],
        letter_spacing: f32,
        affine: [f32; 6],
        fragment_clip: clip::FragmentClip,
        opacity: f32,
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
        // Shape in physical px and keep TextArea.scale = 1 so glyphs are not double-scaled.
        let physical_size = size * scale;
        let physical_line_height = line_height * scale;
        let physical_width = bounds.width.max(0.0) * scale;
        let physical_height = bounds.height.max(line_height) * scale;
        let default_color = with_opacity(color.unwrap_or([0.0, 0.0, 0.0, 1.0]), opacity);
        let attrs = text_attrs(family, weight, letter_spacing, size);
        let shaping = match shaping {
            TextShaping::Auto if content.is_ascii() => Shaping::Basic,
            TextShaping::Auto | TextShaping::Advanced => Shaping::Advanced,
        };
        let align = match horizontal {
            TextHorizontalAlignment::Start => None,
            TextHorizontalAlignment::Center => Some(Align::Center),
            TextHorizontalAlignment::End => Some(Align::Right),
        };
        let painted = presentation_spans(content, spans, default_color, opacity);
        let rich = painted.len() > 1 || painted.first().is_some_and(|span| span.1 != default_color);
        let key = ShapeKeyRef {
            content,
            family,
            weight,
            font_size_bits: physical_size.to_bits(),
            line_height_bits: physical_line_height.to_bits(),
            wrap,
            ellipsis,
            shaping: match shaping {
                Shaping::Basic => 0,
                Shaping::Advanced => 1,
            },
            letter_spacing_bits: letter_spacing.to_bits(),
            width_bits: physical_width.to_bits(),
            height_bits: physical_height.to_bits(),
            align: match horizontal {
                TextHorizontalAlignment::Start => 0,
                TextHorizontalAlignment::Center => 1,
                TextHorizontalAlignment::End => 2,
            },
            spans: rich.then_some(painted.as_slice()),
        };
        let hash = key.hash64();
        if self.shape_cache.get(hash, &key).is_none() {
            let mut fonts = crate::nana_text::lock_font_system(&self.font_system);
            let mut buffer = Buffer::new(
                &mut fonts,
                Metrics::new(physical_size, physical_line_height),
            );
            buffer.set_size(Some(physical_width), Some(physical_height));
            buffer.set_wrap(if wrap { Wrap::Word } else { Wrap::None });
            buffer.set_ellipsize(if ellipsis {
                cosmic_text::Ellipsize::End(cosmic_text::EllipsizeHeightLimit::Height(
                    physical_height,
                ))
            } else {
                cosmic_text::Ellipsize::None
            });
            if rich {
                let rich_text = painted
                    .iter()
                    .map(|(text, color)| (*text, attrs.clone().color(rgba8_color(*color))))
                    .collect::<Vec<_>>();
                buffer.set_rich_text(rich_text, &attrs, shaping, align);
            } else {
                buffer.set_text(content, &attrs, shaping, align);
            }
            buffer.shape_until_scroll(&mut fonts, false);
            drop(fonts);
            self.shape_cache.insert(hash, key.to_owned_key(), buffer);
        }
        let laid_out_height = {
            let buffer = self.shape_cache.buffer(hash).expect("shaped above");
            measure(buffer).1
        };
        let aligned = text_box_origin(bounds, vertical, laid_out_height / scale);
        if fragment_clip == clip::FragmentClip::REJECT {
            return None;
        }
        // Cryoglyph TextBounds is an AABB. Rotated overflow must go through
        // the affine atlas path so fragment_clip can discard parallelogram
        // exteriors; translation-only axis-aligned clips keep the AABB path.
        if clip::is_translation(affine) && fragment_clip == clip::FragmentClip::PASS {
            self.prepare_cryoglyph(
                device,
                queue,
                encoder,
                hash,
                aligned,
                clip,
                scale,
                affine,
                default_color,
            )
        } else {
            self.prepare_affine_glyphs(
                device,
                queue,
                hash,
                aligned,
                scale,
                affine,
                fragment_clip,
                default_color,
            )
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_cryoglyph(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        shape: u64,
        aligned: [f32; 2],
        clip: LogicalRect,
        scale: f32,
        affine: [f32; 6],
        default_color: [f32; 4],
    ) -> Option<PreparedText> {
        let buffer = self.shape_cache.buffer(shape).expect("shaped above");
        let [world_x, world_y] = clip::transform_point(affine, aligned[0], aligned[1]);
        let left = world_x * scale;
        let top = world_y * scale;
        let text_bounds = cryoglyph::TextBounds {
            left: (clip.x * scale).round() as i32,
            top: (clip.y * scale).round() as i32,
            right: ((clip.x + clip.width) * scale).round() as i32,
            bottom: ((clip.y + clip.height) * scale).round() as i32,
        };
        let index = self.frame_texts;
        self.frame_texts += 1;
        if self.renderers.len() <= index {
            self.renderers.push(cryoglyph::TextRenderer::new(
                &mut self.atlas,
                device,
                wgpu::MultisampleState::default(),
                None,
            ));
        }
        let area = cryoglyph::TextArea {
            text: buffer.layout_runs(),
            left,
            top,
            scale: 1.0,
            bounds: text_bounds,
            default_color: rgba8_color(default_color),
        };
        let mut fonts = crate::nana_text::lock_font_system(&self.font_system);
        let result = self.renderers[index].prepare(
            device,
            queue,
            encoder,
            &mut fonts,
            &mut self.atlas,
            &self.viewport,
            [area],
            &mut self.swash,
        );
        if matches!(result, Err(cryoglyph::PrepareError::AtlasFull)) {
            self.atlas.trim();
            let area = cryoglyph::TextArea {
                text: buffer.layout_runs(),
                left,
                top,
                scale: 1.0,
                bounds: text_bounds,
                default_color: rgba8_color(default_color),
            };
            let _ = self.renderers[index].prepare(
                device,
                queue,
                encoder,
                &mut fonts,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash,
            );
        }
        drop(fonts);
        Some(PreparedText {
            index,
            kind: PreparedKind::Cryoglyph,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_affine_glyphs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shape: u64,
        aligned: [f32; 2],
        scale: f32,
        affine: [f32; 6],
        fragment_clip: clip::FragmentClip,
        default_color: [f32; 4],
    ) -> Option<PreparedText> {
        let cache_key = AffineKey {
            shape,
            origin_bits: [aligned[0].to_bits(), aligned[1].to_bits()],
            scale_bits: scale.to_bits(),
            affine_bits: affine.map(f32::to_bits),
            clip_bits: fragment_clip.to_bits(),
            color_bits: default_color.map(f32::to_bits),
        };
        self.frame_affines += 1;
        if let Some(index) = self.affine_cache.get(&cache_key, self.frame) {
            return Some(PreparedText {
                index,
                kind: PreparedKind::Affine,
            });
        }

        let buffer = self.shape_cache.buffer(shape).expect("shaped above");
        let origin_physical = [aligned[0] * scale, aligned[1] * scale];
        let mut fonts = crate::nana_text::lock_font_system(&self.font_system);
        let mut packed = Vec::new();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((origin_physical[0], origin_physical[1]), 1.0);
                let Some(image) = self.swash.get_image(&mut fonts, physical.cache_key).clone()
                else {
                    continue;
                };
                let Some((pixel_w, pixel_h, rgba)) = glyph_rgba(&image) else {
                    continue;
                };
                let x = physical.x + image.placement.left;
                let y = run.line_y.round() as i32 + physical.y - image.placement.top;
                let scene_color = glyph
                    .color_opt
                    .map(color_from_cosmic)
                    .unwrap_or(default_color);
                let color = if matches!(image.content, SwashContent::Color) {
                    [1.0, 1.0, 1.0, scene_color[3]]
                } else {
                    pack_linear(scene_color)
                };
                packed.push(PackedGlyph {
                    logical: LogicalRect::from_xywh(
                        x as f32 / scale,
                        y as f32 / scale,
                        pixel_w as f32 / scale,
                        pixel_h as f32 / scale,
                    ),
                    color,
                    atlas_x: 0,
                    atlas_y: 0,
                    pixel_w,
                    pixel_h,
                    rgba,
                });
            }
        }
        if packed.is_empty() {
            return None;
        }
        let (atlas_w, atlas_h, atlas) = pack_glyph_atlas(&mut packed);
        let vertices =
            affine_glyph_vertices(&packed, affine, scale, atlas_w, atlas_h, fragment_clip);
        if vertices.is_empty() {
            return None;
        }
        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-ui.scene.text.affine.atlas"),
            size: wgpu::Extent3d {
                width: atlas_w,
                height: atlas_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas_w * 4),
                rows_per_image: Some(atlas_h),
            },
            wgpu::Extent3d {
                width: atlas_w,
                height: atlas_h,
                depth_or_array_layers: 1,
            },
        );
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.text.affine.atlas.bind"),
            layout: &self.affine.atlas_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.affine.sampler),
                },
            ],
        });
        let vertex_bytes = bytemuck::cast_slice(&vertices);
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.text.affine.vertices"),
            size: vertex_bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, vertex_bytes);
        // The atlas texture and the vertex buffer are the two memory allocations
        // a cache hit avoids.
        self.frame_gpu_allocations = self.frame_gpu_allocations.saturating_add(2);
        let slot = AffineSlot {
            _atlas: atlas_texture,
            bind_group,
            vertices: vertex_buffer,
            vertex_count: vertices.len() as u32,
        };
        let index = self.affine_cache.insert(cache_key, slot, self.frame);
        Some(PreparedText {
            index,
            kind: PreparedKind::Affine,
        })
    }

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        prepared: &PreparedText,
        scissor: PhysicalRect,
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        match prepared.kind {
            PreparedKind::Cryoglyph => {
                let Some(renderer) = self.renderers.get(prepared.index) else {
                    return;
                };
                let _ = renderer.render(&self.atlas, &self.viewport, pass);
            }
            PreparedKind::Affine => {
                let Some(slot) = self.affine_cache.slot(prepared.index) else {
                    return;
                };
                if slot.vertex_count == 0 {
                    return;
                }
                pass.set_pipeline(&self.affine.pipeline);
                pass.set_bind_group(0, &self.affine.uniform_bind_group, &[]);
                pass.set_bind_group(1, &slot.bind_group, &[]);
                pass.set_vertex_buffer(0, slot.vertices.slice(..));
                pass.draw(0..slot.vertex_count, 0..1);
            }
        }
        if let Some(work) = gpu_work {
            work.record_draw_batch();
            work.record_draw_call();
        }
    }
}

impl AffineGlyphPipeline {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui.scene.text.affine.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(AFFINE_GLYPH_SHADER)),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.text.affine.uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<AffineUniforms>() as u64
                    ),
                },
                count: None,
            }],
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.text.affine.atlas"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui.scene.text.affine.uniforms"),
            size: std::mem::size_of::<AffineUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-ui.scene.text.affine.uniforms.bind"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nana-ui.scene.text.affine.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui.scene.text.affine.pipeline"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&atlas_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui.scene.text.affine.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GlyphVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array!(
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4,
                        3 => Float32x4,
                        4 => Float32x4,
                        5 => Float32x2,
                    ),
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            atlas_layout,
            sampler,
            uniform_bind_group,
            uniforms,
        }
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

fn text_attrs<'a>(
    family: Option<&'a str>,
    weight: Option<u16>,
    letter_spacing: f32,
    font_size: f32,
) -> Attrs<'a> {
    let mut attrs =
        Attrs::new()
            .family(resolve_family(family))
            .weight(match weight.unwrap_or(400) {
                0..=199 => cosmic_text::Weight::THIN,
                200..=299 => cosmic_text::Weight::EXTRA_LIGHT,
                300..=349 => cosmic_text::Weight::LIGHT,
                350..=449 => cosmic_text::Weight::NORMAL,
                450..=549 => cosmic_text::Weight::MEDIUM,
                550..=649 => cosmic_text::Weight::SEMIBOLD,
                650..=749 => cosmic_text::Weight::BOLD,
                750..=849 => cosmic_text::Weight::EXTRA_BOLD,
                _ => cosmic_text::Weight::BLACK,
            });
    let tracking = letter_spacing_em(letter_spacing, font_size);
    if tracking != 0.0 {
        attrs = attrs.letter_spacing(tracking);
    }
    attrs
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

/// Four corners of a glyph quad after the same CSS/Canvas affine used for Quad/Mesh.
fn transform_glyph_quad(affine: [f32; 6], x: f32, y: f32, w: f32, h: f32) -> [[f32; 2]; 4] {
    [
        clip::transform_point(affine, x, y),
        clip::transform_point(affine, x + w, y),
        clip::transform_point(affine, x, y + h),
        clip::transform_point(affine, x + w, y + h),
    ]
}

fn glyph_rgba(image: &cosmic_text::SwashImage) -> Option<(u32, u32, Vec<u8>)> {
    let width = image.placement.width;
    let height = image.placement.height;
    if width == 0 || height == 0 {
        return None;
    }
    let pixels = (width as usize).saturating_mul(height as usize);
    match image.content {
        SwashContent::Mask => {
            if image.data.len() < pixels {
                return None;
            }
            let mut rgba = vec![0u8; pixels * 4];
            for (index, coverage) in image.data.iter().take(pixels).enumerate() {
                let offset = index * 4;
                rgba[offset] = 255;
                rgba[offset + 1] = 255;
                rgba[offset + 2] = 255;
                rgba[offset + 3] = *coverage;
            }
            Some((width, height, rgba))
        }
        SwashContent::Color | SwashContent::SubpixelMask => {
            let bytes = pixels * 4;
            if image.data.len() < bytes {
                return None;
            }
            Some((width, height, image.data[..bytes].to_vec()))
        }
    }
}

fn pack_glyph_atlas(glyphs: &mut [PackedGlyph]) -> (u32, u32, Vec<u8>) {
    let max_glyph_w = glyphs.iter().map(|glyph| glyph.pixel_w).max().unwrap_or(1);
    let width = (max_glyph_w + 2)
        .max(ATLAS_ROW_ALIGN)
        .next_multiple_of(ATLAS_ROW_ALIGN);
    let mut x = 1u32;
    let mut y = 1u32;
    let mut row_h = 0u32;
    for glyph in glyphs.iter_mut() {
        let packed_w = glyph.pixel_w + 1;
        let packed_h = glyph.pixel_h + 1;
        if x + packed_w + 1 > width {
            x = 1;
            y += row_h;
            row_h = 0;
        }
        glyph.atlas_x = x;
        glyph.atlas_y = y;
        x += packed_w;
        row_h = row_h.max(packed_h);
    }
    let height = (y + row_h + 1).max(1);
    let mut atlas = vec![0u8; (width as usize) * (height as usize) * 4];
    for glyph in glyphs.iter() {
        for row in 0..glyph.pixel_h {
            let src = (row as usize) * (glyph.pixel_w as usize) * 4;
            let dest =
                ((glyph.atlas_y + row) as usize * width as usize + glyph.atlas_x as usize) * 4;
            let span = (glyph.pixel_w as usize) * 4;
            atlas[dest..dest + span].copy_from_slice(&glyph.rgba[src..src + span]);
        }
    }
    (width, height, atlas)
}

fn affine_glyph_vertices(
    glyphs: &[PackedGlyph],
    affine: [f32; 6],
    scale: f32,
    atlas_w: u32,
    atlas_h: u32,
    fragment_clip: clip::FragmentClip,
) -> Vec<GlyphVertex> {
    let atlas_w = atlas_w.max(1) as f32;
    let atlas_h = atlas_h.max(1) as f32;
    let clip = fragment_clip.for_physical_pixels(scale);
    let mut vertices = Vec::with_capacity(glyphs.len() * 6);
    for glyph in glyphs {
        let [tl, tr, bl, br] = transform_glyph_quad(
            affine,
            glyph.logical.x,
            glyph.logical.y,
            glyph.logical.width,
            glyph.logical.height,
        );
        let u0 = glyph.atlas_x as f32 / atlas_w;
        let v0 = glyph.atlas_y as f32 / atlas_h;
        let u1 = (glyph.atlas_x + glyph.pixel_w) as f32 / atlas_w;
        let v1 = (glyph.atlas_y + glyph.pixel_h) as f32 / atlas_h;
        let color = glyph.color;
        let corners = [
            (tl, [u0, v0]),
            (tr, [u1, v0]),
            (bl, [u0, v1]),
            (tr, [u1, v0]),
            (br, [u1, v1]),
            (bl, [u0, v1]),
        ];
        for ([x, y], uv) in corners {
            vertices.push(GlyphVertex {
                position: [x * scale, y * scale],
                uv,
                color,
                clip_rect: clip.rect,
                clip_inv_abcd: clip.inv_abcd,
                clip_inv_ef: clip.inv_ef,
            });
        }
    }
    vertices
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let unrotated = transform_glyph_quad(identity, 10.0, 20.0, 30.0, 8.0);
        let rotated = transform_glyph_quad(rot90, 10.0, 20.0, 30.0, 8.0);
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
            transform: nana_ui_scene::AffineTransform(
                nana_ui_core::PaintTransform {
                    a: k,
                    b: k,
                    c: -k,
                    d: k,
                    ..nana_ui_core::PaintTransform::default()
                }
                .around_center(16.0, 16.0, 32.0, 32.0),
            ),
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

    fn paint_text(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &mut TextPipeline,
        bounds: LogicalRect,
        clip: LogicalRect,
        affine: [f32; 6],
        fragment_clip: clip::FragmentClip,
    ) -> Vec<u8> {
        pipeline.begin_frame(queue, [64, 64]);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui text affine prepare"),
        });
        let prepared = pipeline
            .prepare(
                device,
                queue,
                &mut encoder,
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
                false,
                TextShaping::Auto,
                TextHorizontalAlignment::Start,
                TextVerticalAlignment::Top,
                &[],
                0.0,
                affine,
                fragment_clip,
                1.0,
            )
            .expect("text must prepare");
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
        pipeline.begin_frame(queue, [64, 64]);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui text clip prepare"),
        });
        let prepared = pipeline
            .prepare(
                device,
                queue,
                &mut encoder,
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
                false,
                TextShaping::Auto,
                TextHorizontalAlignment::Start,
                TextVerticalAlignment::Top,
                &[],
                0.0,
                affine,
                fragment_clip,
                1.0,
            )
            .expect("block text must prepare");
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
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .expect("text affine test requires a WGPU adapter");
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("nana-ui text affine test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .expect("text affine test requires a WGPU device")
    }
}
