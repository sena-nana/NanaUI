//! [`OutlineRasterizer`]: glyph outline scaling and bitmap decoding behind
//! `crates/nana-text/src/raster` (`zeno` + `png` live only in this module;
//! font table reading lives in `crate::font::face`).

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use nana_ui_core::sort::CompactSort as _;

use crate::font::{
    AxisCoord, FaceOutlineSink, FaceScalerCache, FontInstanceKey, RawBitmapPayload,
    read_color_bitmap, read_raster_caps, scale_color_outline, scale_plain_outline,
};

use super::{
    GlyphFontId, GlyphImage, GlyphImageFormat, GlyphRasterKey, GlyphRenderMode, GlyphSynthesis,
    GlyphVariationId, StrokeJoin, VARIATION_CAP, blur_image, encode_subpixel, size_from_bits,
};

/// Synthetic-oblique slant, in degrees: 14°, matched across platforms.
const OBLIQUE_DEGREES: f32 = 14.0;

/// Synthetic-bold stroke as a fraction of the raster size. Stroke weight is
/// proportional to size in every real face, so faking it with a constant
/// number of pixels would over-embolden captions and under-embolden headings.
const EMBOLDEN_RATIO: f32 = 0.02;

/// What a colour layer is painted in when the face's palette has no entry
/// for it.
const LAYER_FOREGROUND: [u8; 4] = [128, 128, 128, 255];

/// Frames a variation set stays after its last use, whatever the count: the
/// frame that interned it and the one after may still rasterize with it.
const VARIATION_PIN_FRAMES: u64 = 2;

fn zeno_join(join: StrokeJoin) -> zeno::Join {
    match join {
        StrokeJoin::Miter => zeno::Join::Miter,
        StrokeJoin::Bevel => zeno::Join::Bevel,
        StrokeJoin::Round => zeno::Join::Round,
    }
}

struct InternedVariation {
    coords: Arc<[AxisCoord]>,
    last_used: u64,
}

#[derive(Clone, Copy)]
struct OutlineLayer {
    points: (usize, usize),
    verbs: (usize, usize),
    color: Option<[u8; 4]>,
}

/// Reusable scaled outline buffer shared across glyphs on an [`OutlineRasterizer`].
#[derive(Default)]
struct ScaledOutline {
    layers: Vec<OutlineLayer>,
    points: Vec<zeno::Point>,
    verbs: Vec<zeno::Verb>,
}

impl ScaledOutline {
    fn clear(&mut self) {
        self.layers.clear();
        self.points.clear();
        self.verbs.clear();
    }

    fn path(&self) -> (&[zeno::Point], &[zeno::Verb]) {
        (&self.points[..], &self.verbs[..])
    }

    fn bounds(&self) -> zeno::Bounds {
        zeno::Bounds::from_points(&self.points)
    }

    fn transform(&mut self, transform: &zeno::Transform) {
        for point in &mut self.points {
            *point = transform.transform_point(*point);
        }
    }

    fn layer_count(&self) -> usize {
        self.layers.len()
    }

    fn layer(&self, index: usize) -> Option<((&[zeno::Point], &[zeno::Verb]), Option<[u8; 4]>)> {
        let layer = self.layers.get(index)?;
        let points = self.points.get(layer.points.0..layer.points.1)?;
        let verbs = self.verbs.get(layer.verbs.0..layer.verbs.1)?;
        Some(((points, verbs), layer.color))
    }

    fn close_open_subpath_in_layer(&mut self, verbs_start: usize) {
        if self.verbs.len() > verbs_start && self.verbs.last() != Some(&zeno::Verb::Close) {
            self.verbs.push(zeno::Verb::Close);
        }
    }
}

impl FaceOutlineSink for ScaledOutline {
    fn begin_layer(&mut self, color: Option<[u8; 4]>) {
        let points_end = self.points.len();
        let verbs_end = self.verbs.len();
        self.layers.push(OutlineLayer {
            points: (points_end, points_end),
            verbs: (verbs_end, verbs_end),
            color,
        });
    }

    fn move_to(&mut self, x: f32, y: f32) {
        let verbs_start = self.layers.last().map_or(0, |layer| layer.verbs.0);
        self.close_open_subpath_in_layer(verbs_start);
        self.points.push(zeno::Point::new(x, y));
        self.verbs.push(zeno::Verb::MoveTo);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.points.push(zeno::Point::new(x, y));
        self.verbs.push(zeno::Verb::LineTo);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.points.push(zeno::Point::new(cx0, cy0));
        self.points.push(zeno::Point::new(x, y));
        self.verbs.push(zeno::Verb::QuadTo);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.points.push(zeno::Point::new(cx0, cy0));
        self.points.push(zeno::Point::new(cx1, cy1));
        self.points.push(zeno::Point::new(x, y));
        self.verbs.push(zeno::Verb::CurveTo);
    }

    fn close(&mut self) {
        self.verbs.push(zeno::Verb::Close);
    }

    fn finish_layer(&mut self) {
        let verbs_start = self.layers.last().map_or(0, |layer| layer.verbs.0);
        self.close_open_subpath_in_layer(verbs_start);
        let points_end = self.points.len();
        let verbs_end = self.verbs.len();
        if let Some(last) = self.layers.last_mut() {
            last.points.1 = points_end;
            last.verbs.1 = verbs_end;
        }
    }
}

/// Glyph outlines and colour tables scaled on the CPU.
///
/// Holds the face table that issues [`GlyphFontId`]s, so the ids the glyph IR
/// carries and the faces this rasterizer can scale cannot drift apart.
pub struct OutlineRasterizer {
    /// The engine whose font set issues these faces. The same one the layouts
    /// being resolved were measured against, so a face id in a run always
    /// names a face this can read bytes for.
    engine: crate::SharedTextEngine,
    /// Cached hinting instances for outline scaling. Every *cached rasterized
    /// answer* lives in the caller's cache (the renderer keeps one keyed by
    /// [`GlyphRasterKey`]); this is only the machinery that produces them.
    scaler_cache: FaceScalerCache,
    /// The outline being drawn and the rasterizer's working memory, kept
    /// between glyphs so a glyph that misses allocates only its image.
    outline: ScaledOutline,
    scratch: zeno::Scratch,
    faces: Vec<Option<crate::FontId>>,
    index: HashMap<crate::FontId, GlyphFontId>,
    font_generation: u64,
    retired_faces: Vec<GlyphFontId>,
    /// Axis coordinates behind each [`GlyphVariationId`] a run interned (zero
    /// is the face's default instance). The key carries the id because a
    /// bitmap differs by coordinates; the scaler needs the coordinates
    /// themselves.
    ///
    /// Issued in order and looked up by the coordinates themselves, like the
    /// face ids: a hash of the coordinates would let two sets that collide
    /// draw with the first one's axes and share its bitmaps. An id is never
    /// issued twice, so one evicted here ([`Self::begin_frame`]) cannot come
    /// back naming other coordinates; bitmaps keyed by it just go cold.
    variations: HashMap<GlyphVariationId, InternedVariation>,
    variation_index: HashMap<Arc<[AxisCoord]>, GlyphVariationId>,
    issued_variations: u64,
    /// Frames begun, for the variations' last use.
    frame: u64,
    /// The face asked for last. Runs are contiguous by face, so a paragraph
    /// asks for the same one for every glyph and this keeps the hash lookup
    /// off the per-glyph path.
    recent: Option<(crate::FontId, GlyphFontId)>,
}

impl OutlineRasterizer {
    pub fn new(engine: crate::SharedTextEngine) -> Self {
        Self {
            engine,
            scaler_cache: FaceScalerCache::default(),
            outline: ScaledOutline::default(),
            scratch: zeno::Scratch::new(),
            faces: Vec::new(),
            index: HashMap::new(),
            font_generation: 0,
            retired_faces: Vec::new(),
            variations: HashMap::new(),
            variation_index: HashMap::new(),
            issued_variations: 0,
            frame: 0,
            recent: None,
        }
    }

    /// The renderer's ids for one face instance.
    ///
    /// Interning the whole instance rather than the face alone is what keeps
    /// the coordinates reachable at raster time: the key carries a
    /// [`GlyphVariationId`], and this is the table that turns it back into the
    /// axis values the scaler needs.
    ///
    /// Face ids are generational and never alias, so an id stays valid across
    /// font-set generations; the generation rides in the raster key instead,
    /// where it invalidates bitmaps without invalidating identity.
    pub fn intern_instance(
        &mut self,
        instance: &FontInstanceKey,
    ) -> (GlyphFontId, GlyphVariationId, GlyphSynthesis) {
        let font = self.intern_face(instance.font);
        let variation = self.intern_variation(&instance.coords);
        let mut synthesis = GlyphSynthesis::NONE;
        if instance.synthesis.bold {
            synthesis = synthesis.with(GlyphSynthesis::FAKE_BOLD);
        }
        if instance.synthesis.oblique {
            synthesis = synthesis.with(GlyphSynthesis::FAKE_ITALIC);
        }
        (font, variation, synthesis)
    }

    fn intern_variation(&mut self, coords: &Arc<[AxisCoord]>) -> GlyphVariationId {
        if coords.is_empty() {
            return GlyphVariationId(0);
        }
        let frame = self.frame;
        if let Some(variation) = self.variation_index.get(coords) {
            if let Some(interned) = self.variations.get_mut(variation) {
                interned.last_used = frame;
            }
            return *variation;
        }
        self.issued_variations += 1;
        let variation = GlyphVariationId(self.issued_variations);
        self.variations.insert(
            variation,
            InternedVariation {
                coords: Arc::clone(coords),
                last_used: frame,
            },
        );
        self.variation_index.insert(Arc::clone(coords), variation);
        variation
    }

    /// Coordinate sets holding an id.
    #[doc(hidden)]
    pub fn variation_count(&self) -> usize {
        self.variations.len()
    }

    /// Starts a frame. Past [`VARIATION_CAP`] the least recently used sets
    /// not used in the last [`VARIATION_PIN_FRAMES`] frames go, and their ids
    /// are returned. Nothing rasterizes with one again: a run that shows those
    /// coordinates later interns them afresh, and an entry already built
    /// holds its atlas cells, not the id's coordinates.
    pub fn begin_frame(&mut self) -> Vec<GlyphVariationId> {
        self.frame += 1;
        if !self.faces.is_empty() {
            let mut reset_scaler = false;
            {
                let engine = crate::lock_text_engine(&self.engine);
                let generation = engine.fonts().generation().get();
                if generation != self.font_generation {
                    self.font_generation = generation;
                    for (slot_idx, entry) in self.faces.iter_mut().enumerate() {
                        if let Some(face) = *entry
                            && engine.fonts().face_data(face).is_none()
                        {
                            *entry = None;
                            self.index.remove(&face);
                            if self.recent.is_some_and(|(recent, _)| recent == face) {
                                self.recent = None;
                            }
                            self.retired_faces.push(GlyphFontId(slot_idx as u32));
                            reset_scaler = true;
                        }
                    }
                }
            }
            if reset_scaler {
                self.scaler_cache = FaceScalerCache::default();
            }
        }
        if self.variations.len() <= VARIATION_CAP {
            return Vec::new();
        }
        let pinned_since = self.frame.saturating_sub(VARIATION_PIN_FRAMES);
        let mut cold: Vec<(u64, GlyphVariationId)> = self
            .variations
            .iter()
            .filter(|(_, interned)| interned.last_used < pinned_since)
            .map(|(id, interned)| (interned.last_used, *id))
            .collect();
        // Down to three quarters, so the scan runs once per many frames
        // rather than on every new set.
        let excess = self.variations.len() - VARIATION_CAP * 3 / 4;
        cold.compact_sort_by_key(|(last_used, id)| (*last_used, id.0));
        cold.truncate(excess);
        cold.into_iter()
            .map(|(_, id)| {
                if let Some(interned) = self.variations.remove(&id) {
                    self.variation_index.remove(&interned.coords);
                }
                id
            })
            .collect()
    }

    /// Face ids retired by font unregistration since the last call.
    pub fn take_retired_faces(&mut self) -> Vec<GlyphFontId> {
        std::mem::take(&mut self.retired_faces)
    }

    fn intern_face(&mut self, face: crate::FontId) -> GlyphFontId {
        if let Some((recent, font)) = self.recent
            && recent == face
        {
            return font;
        }
        let font = match self.index.get(&face) {
            Some(font) => *font,
            None => {
                let font = GlyphFontId(self.faces.len() as u32);
                self.faces.push(Some(face));
                self.index.insert(face, font);
                font
            }
        };
        self.recent = Some((face, font));
        font
    }

    /// The bytes behind a face id this rasterizer issued, for a backend that
    /// scales the same face by other means.
    pub fn face_data(&self, font: GlyphFontId) -> Option<(crate::FontId, crate::font::FontData)> {
        let face = (*self.faces.get(font.0 as usize)?)?;
        let engine = crate::lock_text_engine(&self.engine);
        Some((face, engine.fonts().face_data(face)?))
    }

    /// The axis coordinates behind a variation id, empty for the default
    /// instance.
    pub fn coords(&self, variation: GlyphVariationId) -> &[AxisCoord] {
        self.variation_coords(variation)
            .map_or(&[], |coords| coords)
    }

    fn variation_coords(&self, variation: GlyphVariationId) -> Option<&Arc<[AxisCoord]>> {
        self.variations
            .get(&variation)
            .map(|interned| &interned.coords)
    }

    #[doc(hidden)]
    pub fn face_count(&self) -> usize {
        self.faces.iter().flatten().count()
    }

    /// Rasterize one glyph, or `None` when the face is unknown or cannot
    /// draw it. A glyph that is simply blank answers `Some` with an empty
    /// image, so a cache can tell "nothing to draw" from "ask again".
    pub fn rasterize(&mut self, key: &GlyphRasterKey) -> Option<GlyphImage> {
        let face = (*self.faces.get(key.font.0 as usize)?)?;
        let glyph_id = u16::try_from(key.glyph).ok()?;
        // The blob is cloned out of the engine rather than scaled under its
        // lock: a glyph that misses must not hold the lock every other
        // window's layout needs. `FontData` shares the bytes, so this is a
        // refcount, not a copy of the face.
        let data = {
            let engine = crate::lock_text_engine(&self.engine);
            engine.fonts().face_data(face)?
        };
        let caps = read_raster_caps(data.bytes(), data.index())?;
        let size = size_from_bits(key.size_bits);
        let hint = !key.synthesis.contains(GlyphSynthesis::DISABLE_HINTING);
        let coords = self
            .variations
            .get(&key.variation)
            .map_or(&[][..], |interned| &interned.coords[..]);
        let font_key = u64::from(key.font.0) | (u64::from(key.font_generation) << 32);

        // A face with no outlines has only strikes, and a strike cannot be
        // shifted by a fraction of a pixel — asking for one blurs it.
        let snap_to_pixel =
            !caps.has_outlines || key.synthesis.contains(GlyphSynthesis::PIXEL_FONT);
        let format = match key.mode {
            GlyphRenderMode::SubpixelRgb | GlyphRenderMode::SubpixelBgr => zeno::Format::Subpixel,
            GlyphRenderMode::Mask
            | GlyphRenderMode::Stroke { .. }
            | GlyphRenderMode::Blur { .. } => zeno::Format::Alpha,
        };
        let offset = subpixel_offset(key, snap_to_pixel);
        let mut transform = None;
        if key.synthesis.contains(GlyphSynthesis::FAKE_ITALIC) {
            transform = Some(zeno::Transform::skew(
                zeno::Angle::from_degrees(OBLIQUE_DEGREES),
                zeno::Angle::from_degrees(0.0),
            ));
        }
        if key.synthesis.contains(GlyphSynthesis::ROTATE_CW) {
            // Font space is y-up, so a quarter turn clockwise on the page
            // sends the outline's x to page-down (font −y) and its y to
            // page-right (font +x): (x, y) → (y, −x). Written out rather than
            // as `rotation(-90°)`, whose `cos` is not exactly zero and would
            // lean every sideways glyph by a hair. After the oblique skew, so
            // a fake italic slants along its own baseline.
            let quarter_turn = zeno::Transform::new(0.0, -1.0, 1.0, 0.0, 0.0, 0.0);
            transform = Some(match transform {
                Some(skew) => skew.then(&quarter_turn),
                None => quarter_turn,
            });
        }
        let mut embolden = 0.0;
        if key.synthesis.contains(GlyphSynthesis::FAKE_BOLD) {
            embolden += size * EMBOLDEN_RATIO;
        }
        if let GlyphRenderMode::Blur { spread_q, .. } = key.mode {
            // Spread grows the coverage before it is blurred, the way CSS
            // spreads a shadow's shape: an embolden by twice the spread moves
            // each edge out by about the spread.
            embolden += f32::from(spread_q) * 0.5;
        }
        // Colour sources take no embolden.
        let plain = key.mode.is_derived() || !(caps.has_color_outlines || caps.has_color_bitmaps);
        let how = Outlining {
            format,
            offset,
            transform,
            mode: key.mode,
        };
        let mut image = if embolden > 0.0 && plain && caps.has_outlines {
            emboldened(
                data.bytes(),
                data.index(),
                font_key,
                coords,
                size,
                hint,
                glyph_id,
                embolden,
                &how,
                &mut self.scaler_cache,
                &mut self.outline,
                &mut self.scratch,
            )?
        } else {
            drawn(
                data.bytes(),
                data.index(),
                font_key,
                coords,
                size,
                hint,
                glyph_id,
                caps.has_outlines,
                caps.has_color_outlines,
                caps.has_color_bitmaps,
                &how,
                &mut self.scaler_cache,
                &mut self.outline,
                &mut self.scratch,
            )?
        };
        let bytes = (image.width as usize)
            .saturating_mul(image.height as usize)
            .saturating_mul(image.format.bytes_per_pixel());
        if image.data.len() < bytes {
            return None;
        }
        image.data.truncate(bytes);
        if image.format == GlyphImageFormat::SubpixelRgb {
            for pixel in image.data.as_chunks_mut::<4>().0 {
                let rgb = [pixel[0], pixel[1], pixel[2]];
                encode_subpixel(pixel, rgb, key.mode);
            }
        }
        match key.mode {
            GlyphRenderMode::Blur { radius_q, .. } if image.format == GlyphImageFormat::Mask => {
                Some(blur_image(image, f32::from(radius_q) * 0.25))
            }
            _ => Some(image),
        }
    }
}

/// How [`drawn`] and [`emboldened`] draw the outline.
struct Outlining {
    format: zeno::Format,
    offset: zeno::Vector,
    transform: Option<zeno::Transform>,
    mode: GlyphRenderMode,
}

/// The glyph from the first source that has it: a colour outline in the
/// face's first palette, then a colour strike, then the plain outline. Order
/// matters: an emoji face can have all three, and the first is the one with
/// the palette applied. A stroke or a shadow is coverage of the outline
/// itself, whatever colours the face would paint it in.
#[allow(clippy::too_many_arguments)]
fn drawn(
    data: &[u8],
    index: u32,
    font_key: u64,
    coords: &[AxisCoord],
    size_px: f32,
    hint: bool,
    glyph_id: u16,
    has_outlines: bool,
    has_color_outlines: bool,
    has_color_bitmaps: bool,
    how: &Outlining,
    cache: &mut FaceScalerCache,
    outline: &mut ScaledOutline,
    scratch: &mut zeno::Scratch,
) -> Option<GlyphImage> {
    if !how.mode.is_derived() {
        if has_color_outlines {
            outline.clear();
            if scale_color_outline(
                data, index, font_key, coords, size_px, hint, glyph_id, cache, outline,
            ) && let Some(image) = color_layers(how, outline, scratch)
            {
                return Some(image);
            }
        }
        if has_color_bitmaps && let Some(image) = color_bitmap(data, index, size_px, glyph_id) {
            return Some(image);
        }
    }
    if !has_outlines {
        return None;
    }
    outline.clear();
    if !scale_plain_outline(
        data, index, font_key, coords, size_px, hint, glyph_id, cache, outline,
    ) {
        return None;
    }
    if let Some(transform) = &how.transform {
        outline.transform(transform);
    }
    let style = match how.mode {
        GlyphRenderMode::Stroke { width_q, join } => {
            let mut stroke = zeno::Stroke::new(f32::from(width_q) * 0.25);
            stroke.join(zeno_join(join));
            zeno::Style::Stroke(stroke)
        }
        _ => zeno::Style::default(),
    };
    let mut pixels = Vec::new();
    let placement = zeno::Mask::with_scratch(outline.path(), scratch)
        .format(how.format)
        .origin(zeno::Origin::BottomLeft)
        .style(style)
        .offset(how.offset)
        .render_offset(how.offset)
        .inspect(|format, width, height| pixels.resize(format.buffer_size(width, height), 0))
        .render_into(&mut pixels[..], None);
    let format = match how.format {
        zeno::Format::Subpixel => GlyphImageFormat::SubpixelRgb,
        _ => GlyphImageFormat::Mask,
    };
    Some(GlyphImage {
        format,
        width: placement.width,
        height: placement.height,
        left: placement.left,
        top: placement.top,
        data: pixels,
    })
}

/// Decodes and scales a best-fit color bitmap strike glyph into straight RGBA.
fn color_bitmap(data: &[u8], index: u32, size_px: f32, glyph_id: u16) -> Option<GlyphImage> {
    let raw = read_color_bitmap(data, index, size_px, glyph_id)?;
    let (mut src_w, mut src_h, rgba) = match raw.payload {
        RawBitmapPayload::Png(bytes) => decode_png_rgba(bytes)?,
        RawBitmapPayload::Bgra(bytes) => (
            raw.width,
            raw.height,
            decode_premul_bgra(bytes, raw.width, raw.height)?,
        ),
    };
    let scale = size_px / raw.ppem;
    if (scale - 1.0).abs() > 1e-4 {
        let dst_w = (src_w as f32 * scale) as u32;
        let dst_h = (src_h as f32 * scale) as u32;
        let left = (raw.left as f32 * scale) as i32;
        let top = (raw.top as f32 * scale) as i32;
        let scaled = resize_rgba(&rgba, src_w, src_h, dst_w, dst_h)?;
        return Some(GlyphImage {
            format: GlyphImageFormat::ColorRgba,
            width: dst_w,
            height: dst_h,
            left,
            top,
            data: scaled,
        });
    }
    src_w = src_w.max(0);
    src_h = src_h.max(0);
    Some(GlyphImage {
        format: GlyphImageFormat::ColorRgba,
        width: src_w,
        height: src_h,
        left: raw.left,
        top: raw.top,
        data: rgba,
    })
}

/// Decodes PNG bytes into straight 8-bit RGBA pixels `(width, height, data)`.
fn decode_png_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().ok()?;
    let buf_len = reader.output_buffer_size()?;
    let mut raw = vec![0u8; buf_len];
    let info = reader.next_frame(&mut raw).ok()?;
    let frame = &raw[..info.buffer_size()];
    let pixel_count = usize::try_from(info.width)
        .ok()?
        .checked_mul(usize::try_from(info.height).ok()?)?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => {
            if frame.len() < pixel_count * 4 {
                return None;
            }
            frame[..pixel_count * 4].to_vec()
        }
        png::ColorType::Rgb => {
            if frame.len() < pixel_count * 3 {
                return None;
            }
            let mut out = Vec::with_capacity(pixel_count * 4);
            for px in frame[..pixel_count * 3].chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            if frame.len() < pixel_count * 2 {
                return None;
            }
            let mut out = Vec::with_capacity(pixel_count * 4);
            for px in frame[..pixel_count * 2].chunks_exact(2) {
                out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
            out
        }
        png::ColorType::Grayscale => {
            if frame.len() < pixel_count {
                return None;
            }
            let mut out = Vec::with_capacity(pixel_count * 4);
            for &gray in &frame[..pixel_count] {
                out.extend_from_slice(&[gray, gray, gray, 255]);
            }
            out
        }
        png::ColorType::Indexed => return None,
    };
    Some((info.width, info.height, rgba))
}

/// Converts OpenType CBDT premultiplied BGRA bytes into straight RGBA.
fn decode_premul_bgra(bytes: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let pixel_count = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    let byte_len = pixel_count.checked_mul(4)?;
    if bytes.len() < byte_len {
        return None;
    }
    let mut out = vec![0u8; byte_len];
    for (src, dst) in bytes[..byte_len]
        .chunks_exact(4)
        .zip(out.chunks_exact_mut(4))
    {
        let a = src[3];
        if a == 0 {
            dst.copy_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
        } else {
            let unpremul = |c: u8| -> u8 {
                ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8
            };
            dst.copy_from_slice(&[unpremul(src[2]), unpremul(src[1]), unpremul(src[0]), a]);
        }
    }
    Some(out)
}

fn mitchell_weight(x: f32) -> f32 {
    let ax = x.abs();
    if ax < 1.0 {
        ((16.0 + ax * ax * (21.0 * ax - 36.0)) / 18.0).abs()
    } else if ax < 2.0 {
        ((32.0 + ax * (-60.0 + ax * (36.0 - 7.0 * ax))) / 18.0).abs()
    } else {
        0.0
    }
}

/// Separable Mitchell-Netravali resample for straight 8-bit RGBA images.
fn resize_rgba(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Option<Vec<u8>> {
    let src_len = (src_w as usize)
        .checked_mul(src_h as usize)?
        .checked_mul(4)?;
    if src.len() < src_len {
        return None;
    }
    let dst_len = (dst_w as usize)
        .checked_mul(dst_h as usize)?
        .checked_mul(4)?;
    if dst_w == 0 || dst_h == 0 {
        return Some(Vec::new());
    }
    if src_w == 0 || src_h == 0 {
        return Some(vec![0u8; dst_len]);
    }
    let scratch_len = (dst_w as usize)
        .checked_mul(src_h as usize)?
        .checked_mul(4)?;
    let mut scratch = vec![0u8; scratch_len];
    let mut dst = vec![0u8; dst_len];

    // Horizontal pass: `(src_w, src_h)` -> `(dst_w, src_h)`.
    resample_axis_rgba(
        src_w,
        src_h,
        dst_w,
        |x, y| {
            let offset = (y as usize * src_w as usize + x as usize) * 4;
            [
                src[offset] as f32 / 255.0,
                src[offset + 1] as f32 / 255.0,
                src[offset + 2] as f32 / 255.0,
                src[offset + 3] as f32 / 255.0,
            ]
        },
        |x, y, px| {
            let offset = (y as usize * dst_w as usize + x as usize) * 4;
            scratch[offset] = (px[0] * 255.0).round().clamp(0.0, 255.0) as u8;
            scratch[offset + 1] = (px[1] * 255.0).round().clamp(0.0, 255.0) as u8;
            scratch[offset + 2] = (px[2] * 255.0).round().clamp(0.0, 255.0) as u8;
            scratch[offset + 3] = (px[3] * 255.0).round().clamp(0.0, 255.0) as u8;
        },
    );

    // Vertical pass (transposed coordinates): `(src_h, dst_w)` -> `(dst_h, dst_w)`.
    resample_axis_rgba(
        src_h,
        dst_w,
        dst_h,
        |y, x| {
            let offset = (y as usize * dst_w as usize + x as usize) * 4;
            [
                scratch[offset] as f32 / 255.0,
                scratch[offset + 1] as f32 / 255.0,
                scratch[offset + 2] as f32 / 255.0,
                scratch[offset + 3] as f32 / 255.0,
            ]
        },
        |y, x, px| {
            let offset = (y as usize * dst_w as usize + x as usize) * 4;
            dst[offset] = (px[0] * 255.0).round().clamp(0.0, 255.0) as u8;
            dst[offset + 1] = (px[1] * 255.0).round().clamp(0.0, 255.0) as u8;
            dst[offset + 2] = (px[2] * 255.0).round().clamp(0.0, 255.0) as u8;
            dst[offset + 3] = (px[3] * 255.0).round().clamp(0.0, 255.0) as u8;
        },
    );

    Some(dst)
}

fn resample_axis_rgba<In, Out>(
    src_extent: u32,
    span_extent: u32,
    dst_extent: u32,
    input: In,
    mut output: Out,
) where
    In: Fn(u32, u32) -> [f32; 4],
    Out: FnMut(u32, u32, &[f32; 4]),
{
    const MAX_WEIGHTS: usize = 64;
    const SUPPORT: f32 = 2.0;
    let mut weights = [0.0f32; MAX_WEIGHTS];
    let ratio = src_extent as f32 / dst_extent as f32;
    let scale = ratio.max(1.0);
    let src_support = SUPPORT * scale;
    let inv_scale = 1.0 / scale;
    for out_i in 0..dst_extent {
        let center = (out_i as f32 + 0.5) * ratio;
        let mut left = ((center - src_support).floor() as i32)
            .max(0)
            .min(src_extent as i32 - 1) as usize;
        let mut right = ((center + src_support).ceil() as i32)
            .max(left as i32 + 1)
            .min(src_extent as i32) as usize;
        let sample_center = center - 0.5;
        while right - left > MAX_WEIGHTS {
            right -= 1;
            left += 1;
        }
        let mut count = 0usize;
        let mut sum = 0.0f32;
        for i in left..right {
            let w = mitchell_weight((i as f32 - sample_center) * inv_scale);
            weights[count] = w;
            count += 1;
            sum += w;
        }
        let inv_sum = if sum != 0.0 { 1.0 / sum } else { 0.0 };
        let active_weights = &weights[..count];
        for span in 0..span_extent {
            let mut accum = [0.0f32; 4];
            for (k, &w) in active_weights.iter().enumerate() {
                let p = input((left + k) as u32, span);
                let a = p[3];
                accum[0] += p[0] * w * a;
                accum[1] += p[1] * w * a;
                accum[2] += p[2] * w * a;
                accum[3] += a * w;
            }
            if accum[3] != 0.0 {
                let inv_a = 1.0 / accum[3];
                accum[0] *= inv_a;
                accum[1] *= inv_a;
                accum[2] *= inv_a;
                accum[3] *= inv_sum;
            }
            output(out_i, span, &accum);
        }
    }
}

/// A scaled colour outline's layers, each filled in its palette colour and
/// laid over the ones before it. `None` when a layer cannot be read.
fn color_layers(
    how: &Outlining,
    outline: &mut ScaledOutline,
    scratch: &mut zeno::Scratch,
) -> Option<GlyphImage> {
    if let Some(transform) = &how.transform {
        outline.transform(transform);
    }
    let bounds = outline.bounds();
    if bounds.is_empty() {
        return Some(GlyphImage {
            format: GlyphImageFormat::ColorRgba,
            width: 0,
            height: 0,
            left: 0,
            top: 0,
            data: Vec::new(),
        });
    }
    // The image is the whole glyph's box; a layer's mask is placed in it by
    // where the two boxes sit on the same pixel grid.
    let left = (bounds.min.x + how.offset.x).floor() as i32;
    let bottom = (bounds.min.y + how.offset.y).ceil() as i32;
    let width = bounds.width().ceil() as u32;
    let height = bounds.height().ceil() as u32;
    let top = height as i32 + bottom;
    let mut data = vec![0; width as usize * height as usize * 4];
    let mut coverage = Vec::new();
    for index in 0..outline.layer_count() {
        let (path, layer_color) = outline.layer(index)?;
        coverage.clear();
        let placement = zeno::Mask::with_scratch(path, scratch)
            .origin(zeno::Origin::BottomLeft)
            .offset(how.offset)
            .render_offset(how.offset)
            .inspect(|format, width, height| coverage.resize(format.buffer_size(width, height), 0))
            .render_into(&mut coverage[..], None);
        let color = layer_color.unwrap_or(LAYER_FOREGROUND);
        tint_over(
            &coverage,
            [placement.width, placement.height],
            [
                placement.left.wrapping_sub(left),
                top.wrapping_sub(placement.top),
            ],
            color,
            &mut data,
            [width, height],
        );
    }
    Some(GlyphImage {
        format: GlyphImageFormat::ColorRgba,
        width,
        height,
        left,
        top,
        data,
    })
}

/// Lays `color` over an RGBA `target` through a coverage `mask` whose
/// top-left pixel goes at `at`; what falls outside the target is dropped.
///
/// The arithmetic shifts by 8 (`>> 8`), matching straight-coverage compositing:
/// coverage times the colour's alpha, divided by 256 rather than 255.
fn tint_over(
    mask: &[u8],
    mask_size: [u32; 2],
    at: [i32; 2],
    color: [u8; 4],
    target: &mut [u8],
    target_size: [u32; 2],
) {
    let [mask_width, mask_height] = mask_size.map(|side| side as usize);
    let [target_width, target_height] = target_size.map(i64::from);
    for (row, line) in mask
        .chunks_exact(mask_width.max(1))
        .take(mask_height)
        .enumerate()
    {
        let y = i64::from(at[1]) + row as i64;
        if y < 0 || y >= target_height {
            continue;
        }
        for (column, coverage) in line.iter().enumerate() {
            let x = i64::from(at[0]) + column as i64;
            if x < 0 || x >= target_width {
                continue;
            }
            let alpha = (u32::from(*coverage) * u32::from(color[3])) >> 8;
            if alpha == 0 {
                continue;
            }
            let start = (y * target_width + x) as usize * 4;
            let Some(pixel) = target.get_mut(start..start + 4) else {
                continue;
            };
            let under = 255 - alpha;
            for (channel, tint) in pixel.iter_mut().zip([color[0], color[1], color[2], 255]) {
                *channel = ((under * u32::from(*channel) + alpha * u32::from(tint)) >> 8) as u8;
            }
        }
    }
}

/// `glyph_id`'s plain outline with every edge moved out by `amount` raster
/// px.
///
/// The fill with a round-joined stroke under it moves every edge out
/// whatever the contours' order or direction. A stroked glyph is stroked
/// that much wider instead: the fill drawn over it hides the inner half.
#[allow(clippy::too_many_arguments)]
fn emboldened(
    data: &[u8],
    index: u32,
    font_key: u64,
    coords: &[AxisCoord],
    size_px: f32,
    hint: bool,
    glyph_id: u16,
    amount: f32,
    how: &Outlining,
    cache: &mut FaceScalerCache,
    outline: &mut ScaledOutline,
    scratch: &mut zeno::Scratch,
) -> Option<GlyphImage> {
    outline.clear();
    if !scale_plain_outline(
        data, index, font_key, coords, size_px, hint, glyph_id, cache, outline,
    ) {
        return None;
    }
    if let Some(transform) = &how.transform {
        outline.transform(transform);
    }
    let path = outline.path();
    let render_mask = |scratch: &mut zeno::Scratch, style: zeno::Style<'_>| {
        let mut buf = Vec::new();
        let placement = zeno::Mask::with_scratch(path, scratch)
            .format(how.format)
            .origin(zeno::Origin::BottomLeft)
            .style(style)
            .offset(how.offset)
            .render_offset(how.offset)
            .inspect(|format, width, height| buf.resize(format.buffer_size(width, height), 0))
            .render_into(&mut buf[..], None);
        (buf, placement)
    };
    let (pixels, placement) = if let GlyphRenderMode::Stroke { width_q, join } = how.mode {
        let mut stroke = zeno::Stroke::new(f32::from(width_q) * 0.25 + 2.0 * amount);
        stroke.join(zeno_join(join));
        render_mask(scratch, stroke.into())
    } else {
        let mut stroke = zeno::Stroke::new(2.0 * amount);
        stroke.join(zeno::Join::Round);
        let (mut outer_buf, outer) = render_mask(scratch, stroke.into());
        let (inner_buf, inner) = render_mask(scratch, zeno::Style::Fill(zeno::Fill::NonZero));
        let bpp = if how.format == zeno::Format::Alpha {
            1
        } else {
            4
        };
        // Both masks are in the same space; the stroke's reaches past the
        // fill's on every side, so the fill is copied in at an offset.
        let (dx, dy) = (inner.left - outer.left, outer.top - inner.top);
        let pitch = inner.width as usize * bpp;
        for (row, line) in inner_buf.chunks_exact(pitch.max(1)).enumerate() {
            let y = dy + row as i32;
            if y < 0 || y >= outer.height as i32 || dx < 0 {
                continue;
            }
            let start = (y as usize * outer.width as usize + dx as usize) * bpp;
            let Some(target) = outer_buf.get_mut(start..start + line.len()) else {
                continue;
            };
            for (out, value) in target.iter_mut().zip(line) {
                *out = (*out).max(*value);
            }
        }
        (outer_buf, outer)
    };
    let format = match how.format {
        zeno::Format::Subpixel => GlyphImageFormat::SubpixelRgb,
        _ => GlyphImageFormat::Mask,
    };
    Some(GlyphImage {
        format,
        width: placement.width,
        height: placement.height,
        left: placement.left,
        top: placement.top,
        data: pixels,
    })
}

/// One step of a path handed to [`path_coverage`]. `MoveTo` and `LineTo`
/// take one point, `CurveTo` (a cubic) three, `Close` none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathStep {
    MoveTo,
    LineTo,
    CurveTo,
    Close,
}

/// How [`path_coverage`] paints a path.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PathPaint {
    /// The area the path encloses, by the non-zero rule.
    Fill,
    /// An outline `width` wide in the path's own units, with round caps and
    /// joins.
    Stroke { width: f32 },
}

/// The coverage of a path over a `size` × `size` square of pixels, one byte
/// each, row by row from the top. The path is in its own units, `scale`
/// pixels to one, with its origin at the square's top-left corner.
///
/// Glyphs are outlines scaled to coverage by this crate's rasterizer, and a
/// renderer that has other small outlines to draw (icons) draws them with
/// the same one rather than link a second. The path goes in as the points
/// and verbs a glyph outline is, so it is also the same compiled code.
pub fn path_coverage(
    points: &[[f32; 2]],
    steps: &[PathStep],
    paint: PathPaint,
    scale: f32,
    size: u32,
) -> Vec<u8> {
    let mut coverage = vec![0; size as usize * size as usize];
    if size == 0 || steps.is_empty() {
        return coverage;
    }
    let points: Vec<zeno::Vector> = points
        .iter()
        .map(|point| zeno::Vector::new(point[0], point[1]))
        .collect();
    let verbs: Vec<zeno::Verb> = steps
        .iter()
        .map(|step| match step {
            PathStep::MoveTo => zeno::Verb::MoveTo,
            PathStep::LineTo => zeno::Verb::LineTo,
            PathStep::CurveTo => zeno::Verb::CurveTo,
            PathStep::Close => zeno::Verb::Close,
        })
        .collect();
    let mut stroke;
    let style = match paint {
        PathPaint::Fill => zeno::Style::Fill(zeno::Fill::NonZero),
        PathPaint::Stroke { width } => {
            stroke = zeno::Stroke::new(width);
            stroke.cap(zeno::Cap::Round).join(zeno::Join::Round);
            zeno::Style::from(&stroke)
        }
    };
    zeno::Mask::new((&points[..], &verbs[..]))
        .style(style)
        .transform(Some(zeno::Transform::scale(scale, scale)))
        .format(zeno::Format::Alpha)
        .size(size, size)
        .render_into(&mut coverage, None);
    coverage
}

/// The fractional pen offset a glyph is rendered at, snapped whole for a face
/// that has nothing to shift.
fn subpixel_offset(key: &GlyphRasterKey, snap: bool) -> zeno::Vector {
    let x = key.subpixel_x.as_float();
    let y = key.subpixel_y.as_float();
    if snap {
        zeno::Vector::new(x.round(), y.round())
    } else {
        zeno::Vector::new(x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::map_char_for_test;

    const COLOR: &[u8] = include_bytes!("../../fonts/nana-test-color.ttf");
    const EMOJI: &[u8] = include_bytes!("../../fonts/noto-emoji.ttf");
    const AXES: &[u8] = include_bytes!("../../fonts/nana-test-axes.ttf");
    const KOREAN: &[u8] = include_bytes!("../../fonts/noto-sans-kr.ttf");

    #[test]
    fn outlines_render_across_sizes_offsets_transforms_and_modes() {
        let mut cache = FaceScalerCache::default();
        let mut outline = ScaledOutline::default();
        let mut scratch = zeno::Scratch::new();
        let skew = zeno::Transform::skew(
            zeno::Angle::from_degrees(OBLIQUE_DEGREES),
            zeno::Angle::from_degrees(0.0),
        );
        let quarter_turn = zeno::Transform::new(0.0, -1.0, 1.0, 0.0, 0.0, 0.0);
        let modes = [
            (
                GlyphRenderMode::Mask,
                zeno::Format::Alpha,
                GlyphImageFormat::Mask,
            ),
            (
                GlyphRenderMode::SubpixelRgb,
                zeno::Format::Subpixel,
                GlyphImageFormat::SubpixelRgb,
            ),
            (
                GlyphRenderMode::Stroke {
                    width_q: 6,
                    join: StrokeJoin::Round,
                },
                zeno::Format::Alpha,
                GlyphImageFormat::Mask,
            ),
            (
                GlyphRenderMode::Stroke {
                    width_q: 9,
                    join: StrokeJoin::Miter,
                },
                zeno::Format::Alpha,
                GlyphImageFormat::Mask,
            ),
        ];

        for (font_key, bytes, text) in [
            (1u64, AXES, "AbgQ@&"),
            (2u64, KOREAN, "\u{d55c}\u{ae00}A8"),
            (3u64, EMOJI, "\u{1f600}\u{2764}"),
        ] {
            let caps = read_raster_caps(bytes, 0).expect("raster caps");
            assert!(caps.has_outlines);
            let mut rendered = 0usize;
            for ch in text.chars() {
                let glyph_id = map_char_for_test(bytes, ch);
                if glyph_id == 0 {
                    continue;
                }
                for size in [9.0f32, 14.0, 31.5, 96.0] {
                    for offset in [zeno::Vector::new(0.0, 0.0), zeno::Vector::new(0.25, 0.75)] {
                        for transform in [None, Some(skew), Some(skew.then(&quarter_turn))] {
                            for (mode, format, expected_format) in modes {
                                let how = Outlining {
                                    format,
                                    offset,
                                    transform,
                                    mode,
                                };
                                let image = drawn(
                                    bytes,
                                    0,
                                    font_key,
                                    &[],
                                    size,
                                    true,
                                    glyph_id,
                                    caps.has_outlines,
                                    caps.has_color_outlines,
                                    caps.has_color_bitmaps,
                                    &how,
                                    &mut cache,
                                    &mut outline,
                                    &mut scratch,
                                )
                                .expect("outline draws");
                                assert_eq!(image.format, expected_format);
                                assert!(!image.is_empty(), "{ch:?} at {size}");
                                assert_eq!(
                                    image.data.len(),
                                    image.width as usize
                                        * image.height as usize
                                        * expected_format.bytes_per_pixel()
                                );
                                assert!(image.data.iter().any(|&v| v > 0), "{ch:?} at {size}");
                                rendered += 1;
                            }
                        }
                    }
                }
            }
            assert!(rendered > 0);
        }
    }

    #[test]
    fn colour_layers_draw_as_rgba_and_derived_strokes_stay_masks() {
        let caps = read_raster_caps(COLOR, 0).expect("color fixture caps");
        assert!(caps.has_color_outlines, "the fixture is a COLR face");
        let glyphs: Vec<u16> = (0x20u32..0x250)
            .chain(0x1f300..0x1f700)
            .filter_map(char::from_u32)
            .map(|ch| map_char_for_test(COLOR, ch))
            .filter(|&gid| gid != 0)
            .collect();
        assert!(!glyphs.is_empty());

        let mut cache = FaceScalerCache::default();
        let mut outline = ScaledOutline::default();
        let mut scratch = zeno::Scratch::new();
        let color_how = Outlining {
            format: zeno::Format::Alpha,
            offset: zeno::Vector::new(0.0, 0.0),
            transform: None,
            mode: GlyphRenderMode::Mask,
        };
        let stroke_how = Outlining {
            format: zeno::Format::Alpha,
            offset: zeno::Vector::new(0.0, 0.0),
            transform: None,
            mode: GlyphRenderMode::Stroke {
                width_q: 8,
                join: StrokeJoin::Round,
            },
        };

        let mut color_count = 0usize;
        for glyph_id in glyphs {
            let image = drawn(
                COLOR,
                0,
                10,
                &[],
                32.0,
                false,
                glyph_id,
                caps.has_outlines,
                caps.has_color_outlines,
                caps.has_color_bitmaps,
                &color_how,
                &mut cache,
                &mut outline,
                &mut scratch,
            )
            .expect("glyph renders");
            if image.format == GlyphImageFormat::ColorRgba {
                assert!(!image.is_empty());
                assert!(image.data.chunks_exact(4).any(|px| px[3] > 0));
                let stroke_image = drawn(
                    COLOR,
                    0,
                    10,
                    &[],
                    32.0,
                    false,
                    glyph_id,
                    caps.has_outlines,
                    caps.has_color_outlines,
                    caps.has_color_bitmaps,
                    &stroke_how,
                    &mut cache,
                    &mut outline,
                    &mut scratch,
                )
                .expect("derived stroke renders as mask");
                assert_eq!(stroke_image.format, GlyphImageFormat::Mask);
                assert!(!stroke_image.is_empty());
                color_count += 1;
            }
        }
        assert!(
            color_count > 0,
            "fixture should draw at least one COLR glyph"
        );
    }

    #[test]
    fn png_rgba_decode_and_mitchell_resize_produce_expected_pixels() {
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("png header");
            writer
                .write_image_data(&[
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
                ])
                .expect("png pixels");
        }
        let (w, h, rgba) = decode_png_rgba(&encoded).expect("decode png rgba");
        assert_eq!((w, h), (2, 2));
        assert_eq!(
            rgba,
            [
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128
            ]
        );
        let scaled = resize_rgba(&rgba, w, h, 4, 4).expect("resize rgba");
        assert_eq!(scaled.len(), 4 * 4 * 4);
        assert!(scaled.chunks_exact(4).any(|px| px[3] > 200));

        // 2x1 CBDT premultiplied BGRA -> straight RGBA check.
        let unpremul = decode_premul_bgra(&[64, 0, 128, 128, 0, 255, 0, 255], 2, 1)
            .expect("decode premul bgra");
        assert_eq!(unpremul, [255, 0, 128, 128, 0, 255, 0, 255]);
    }

    #[test]
    fn a_tint_is_clipped_to_the_target_on_every_side() {
        let mask = [255u8; 9];
        for at in [[-2, -2], [-1, 0], [0, 0], [1, 1], [2, -1], [5, 5]] {
            let mut target = vec![0u8; 3 * 3 * 4];
            tint_over(&mask, [3, 3], at, [10, 20, 30, 255], &mut target, [3, 3]);
            for y in 0..3i32 {
                for x in 0..3i32 {
                    let inside = (0..3).contains(&(x - at[0])) && (0..3).contains(&(y - at[1]));
                    let pixel = &target[(y * 3 + x) as usize * 4..][..4];
                    assert_eq!(pixel[3] != 0, inside, "({x}, {y}) with the mask at {at:?}");
                }
            }
        }
    }
}
