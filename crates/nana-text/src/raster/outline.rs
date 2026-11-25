//! [`OutlineRasterizer`]: glyph outline scaling and bitmap decoding behind
//! `crates/nana-text/src/raster` (`zeno` + `png` live only in this module;
//! font table reading lives in `crate::font::face`).

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use nana_ui_core::sort::CompactSort as _;

#[cfg(feature = "svg")]
use crate::font::read_svg_glyph;
use crate::font::{
    AxisCoord, ColorBrush, ColorCompositeMode, ColorExtend, ColorGlyphOutline, ColorOutlineVerb,
    ColorPaintOp, ColorPaintRecording, ColorStopPaint, ColorTransform, FaceOutlineSink,
    FaceScalerCache, FontInstanceKey, RawBitmapPayload, read_color_bitmap, read_raster_caps,
    record_color_v1_glyph, scale_color_outline, scale_plain_outline,
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

    #[allow(clippy::type_complexity)]
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
        let plain = key.mode.is_derived()
            || !(caps.has_color_outlines || caps.has_color_bitmaps || caps.has_svg);
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
                caps.has_svg,
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
    has_svg: bool,
    how: &Outlining,
    cache: &mut FaceScalerCache,
    outline: &mut ScaledOutline,
    scratch: &mut zeno::Scratch,
) -> Option<GlyphImage> {
    if !how.mode.is_derived() {
        if has_color_outlines {
            if let Some(recording) = record_color_v1_glyph(data, index, coords, glyph_id)
                && let Some(image) = render_color_v1(&recording, size_px, how, scratch)
            {
                return Some(image);
            }
            outline.clear();
            if scale_color_outline(
                data, index, font_key, coords, size_px, hint, glyph_id, cache, outline,
            ) && let Some(image) = color_layers(how, outline, scratch)
            {
                return Some(image);
            }
        }
        #[cfg(feature = "svg")]
        if has_svg && let Some(image) = color_svg_glyph(data, index, size_px, glyph_id) {
            return Some(image);
        }
        let _ = has_svg;
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
    let (src_w, src_h, rgba) = match raw.payload {
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
            for px in frame[..pixel_count * 3].as_chunks::<3>().0 {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            if frame.len() < pixel_count * 2 {
                return None;
            }
            let mut out = Vec::with_capacity(pixel_count * 4);
            for px in frame[..pixel_count * 2].as_chunks::<2>().0 {
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
        .as_chunks::<4>()
        .0
        .iter()
        .zip(out.as_chunks_mut::<4>().0)
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

const MAX_COLR_V1_EDGE: u32 = 4096;

#[cfg(feature = "svg")]
fn color_svg_glyph(data: &[u8], index: u32, size_px: f32, glyph_id: u16) -> Option<GlyphImage> {
    let raw = read_svg_glyph(data, index, glyph_id)?;
    let (left, top, svg) = nana_svg_raster::rasterize_opentype_svg_glyph(
        raw.document,
        glyph_id,
        raw.allow_root_fallback,
        size_px,
        raw.upem,
        MAX_COLR_V1_EDGE,
    )?;
    Some(GlyphImage {
        format: GlyphImageFormat::ColorRgba,
        width: svg.width,
        height: svg.height,
        left,
        top,
        data: svg.rgba.to_vec(),
    })
}

fn transform_mul(a: ColorTransform, b: ColorTransform) -> ColorTransform {
    ColorTransform {
        xx: a.xx * b.xx + a.xy * b.yx,
        yx: a.yx * b.xx + a.yy * b.yx,
        xy: a.xx * b.xy + a.xy * b.yy,
        yy: a.yx * b.xy + a.yy * b.yy,
        dx: a.xx * b.dx + a.xy * b.dy + a.dx,
        dy: a.yx * b.dx + a.yy * b.dy + a.dy,
    }
}

fn transform_point(t: ColorTransform, p: [f32; 2]) -> [f32; 2] {
    [
        t.xx * p[0] + t.xy * p[1] + t.dx,
        t.yx * p[0] + t.yy * p[1] + t.dy,
    ]
}

fn transform_invert(t: ColorTransform) -> Option<ColorTransform> {
    let det = t.xx * t.yy - t.xy * t.yx;
    if !det.is_finite() || det.abs() <= 1e-18 {
        return None;
    }
    let inv_det = 1.0 / det;
    let xx = t.yy * inv_det;
    let xy = -t.xy * inv_det;
    let yx = -t.yx * inv_det;
    let yy = t.xx * inv_det;
    Some(ColorTransform {
        xx,
        yx,
        xy,
        yy,
        dx: -(xx * t.dx + xy * t.dy),
        dy: -(yx * t.dx + yy * t.dy),
    })
}

fn root_colr_transform(upem: u16, size_px: f32, how: &Outlining) -> ColorTransform {
    let scale = size_px / f32::from(upem.max(1));
    let mut t = ColorTransform {
        xx: scale,
        yx: 0.0,
        xy: 0.0,
        yy: scale,
        dx: 0.0,
        dy: 0.0,
    };
    if let Some(extra) = how.transform {
        // `zeno::Transform` stores `xy`/`yx` transposed relative to `ColorTransform`.
        let m = ColorTransform {
            xx: extra.xx,
            xy: extra.yx,
            yx: extra.xy,
            yy: extra.yy,
            dx: extra.x,
            dy: extra.y,
        };
        t = transform_mul(m, t);
    }
    ColorTransform {
        dx: t.dx + how.offset.x,
        dy: t.dy + how.offset.y,
        ..t
    }
}

fn transformed_outline_into(
    outline: &ColorGlyphOutline,
    transform: ColorTransform,
    points: &mut Vec<zeno::Point>,
    verbs: &mut Vec<zeno::Verb>,
) {
    points.clear();
    verbs.clear();
    points.extend(outline.points.iter().map(|&p| {
        let q = transform_point(transform, p);
        zeno::Point::new(q[0], q[1])
    }));
    verbs.extend(outline.verbs.iter().map(|verb| match verb {
        ColorOutlineVerb::MoveTo => zeno::Verb::MoveTo,
        ColorOutlineVerb::LineTo => zeno::Verb::LineTo,
        ColorOutlineVerb::QuadTo => zeno::Verb::QuadTo,
        ColorOutlineVerb::CurveTo => zeno::Verb::CurveTo,
        ColorOutlineVerb::Close => zeno::Verb::Close,
    }));
}

fn transformed_box_into(
    rect: [f32; 4],
    transform: ColorTransform,
    points: &mut Vec<zeno::Point>,
    verbs: &mut Vec<zeno::Verb>,
) {
    points.clear();
    verbs.clear();
    let corners = [
        [rect[0], rect[1]],
        [rect[2], rect[1]],
        [rect[2], rect[3]],
        [rect[0], rect[3]],
    ];
    for c in corners {
        let q = transform_point(transform, c);
        points.push(zeno::Point::new(q[0], q[1]));
    }
    verbs.extend_from_slice(&[
        zeno::Verb::MoveTo,
        zeno::Verb::LineTo,
        zeno::Verb::LineTo,
        zeno::Verb::LineTo,
        zeno::Verb::Close,
    ]);
}

#[derive(Clone, Copy)]
struct RectBounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl RectBounds {
    const EMPTY: Self = Self {
        min_x: f32::INFINITY,
        min_y: f32::INFINITY,
        max_x: f32::NEG_INFINITY,
        max_y: f32::NEG_INFINITY,
    };

    fn is_empty(self) -> bool {
        !(self.min_x < self.max_x && self.min_y < self.max_y)
    }

    fn union(self, other: Self) -> Self {
        if other.is_empty() {
            return self;
        }
        if self.is_empty() {
            return other;
        }
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    fn intersect(self, other: Self) -> Self {
        Self {
            min_x: self.min_x.max(other.min_x),
            min_y: self.min_y.max(other.min_y),
            max_x: self.max_x.min(other.max_x),
            max_y: self.max_y.min(other.max_y),
        }
    }

    fn from_points(points: &[zeno::Point]) -> Self {
        let mut out = Self::EMPTY;
        for p in points {
            out.min_x = out.min_x.min(p.x);
            out.min_y = out.min_y.min(p.y);
            out.max_x = out.max_x.max(p.x);
            out.max_y = out.max_y.max(p.y);
        }
        out
    }
}

fn compute_colr_v1_bounds(recording: &ColorPaintRecording, root: ColorTransform) -> RectBounds {
    let mut transforms = vec![root];
    let mut clip_stack: Vec<RectBounds> = Vec::new();
    let mut total = RectBounds::EMPTY;
    let mut scratch_pts = Vec::new();
    let mut scratch_verbs = Vec::new();

    for op in &recording.ops {
        let cur = *transforms.last().unwrap_or(&root);
        match op {
            ColorPaintOp::PushTransform(t) => transforms.push(transform_mul(cur, *t)),
            ColorPaintOp::PopTransform => {
                if transforms.len() > 1 {
                    transforms.pop();
                }
            }
            ColorPaintOp::PushClipGlyph(idx) => {
                let mut bounds = recording.outlines.get(*idx).map_or(RectBounds::EMPTY, |o| {
                    transformed_outline_into(o, cur, &mut scratch_pts, &mut scratch_verbs);
                    RectBounds::from_points(&scratch_pts)
                });
                if let Some(&parent) = clip_stack.last() {
                    bounds = bounds.intersect(parent);
                }
                clip_stack.push(bounds);
            }
            ColorPaintOp::PushClipBox(rect) => {
                transformed_box_into(*rect, cur, &mut scratch_pts, &mut scratch_verbs);
                let mut bounds = RectBounds::from_points(&scratch_pts);
                if let Some(&parent) = clip_stack.last() {
                    bounds = bounds.intersect(parent);
                }
                clip_stack.push(bounds);
            }
            ColorPaintOp::PopClip => {
                clip_stack.pop();
            }
            ColorPaintOp::Fill(_) => {
                if let Some(&clip) = clip_stack.last() {
                    total = total.union(clip);
                }
            }
            ColorPaintOp::PushLayer | ColorPaintOp::PopLayer(_) => {}
        }
    }
    total
}

struct ClipMask {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    alpha: Vec<u8>,
}

impl ClipMask {
    fn empty() -> Self {
        Self {
            x0: 0,
            y0: 0,
            x1: 0,
            y1: 0,
            alpha: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    fn coverage_at(&self, x: i32, y: i32) -> u8 {
        if x < self.x0 || x >= self.x1 || y < self.y0 || y >= self.y1 {
            return 0;
        }
        let w = (self.x1 - self.x0) as usize;
        let idx = (y - self.y0) as usize * w + (x - self.x0) as usize;
        self.alpha[idx]
    }
}

fn rasterize_clip_mask(
    points: &[zeno::Point],
    verbs: &[zeno::Verb],
    surf_left: i32,
    surf_top: i32,
    surf_w: u32,
    surf_h: u32,
    parent: Option<&ClipMask>,
    scratch: &mut zeno::Scratch,
    coverage_buf: &mut Vec<u8>,
) -> ClipMask {
    if points.is_empty() || verbs.is_empty() || parent.is_some_and(ClipMask::is_empty) {
        return ClipMask::empty();
    }
    coverage_buf.clear();
    let placement = zeno::Mask::with_scratch((points, verbs), scratch)
        .origin(zeno::Origin::BottomLeft)
        .inspect(|format, width, height| coverage_buf.resize(format.buffer_size(width, height), 0))
        .render_into(&mut coverage_buf[..], None);
    if placement.width == 0 || placement.height == 0 {
        return ClipMask::empty();
    }
    let rel_x = placement.left - surf_left;
    let rel_y = surf_top - placement.top;
    let mut x0 = rel_x.max(0);
    let mut y0 = rel_y.max(0);
    let mut x1 = (rel_x + placement.width as i32).min(surf_w as i32);
    let mut y1 = (rel_y + placement.height as i32).min(surf_h as i32);
    if let Some(p) = parent {
        x0 = x0.max(p.x0);
        y0 = y0.max(p.y0);
        x1 = x1.min(p.x1);
        y1 = y1.min(p.y1);
    }
    if x0 >= x1 || y0 >= y1 {
        return ClipMask::empty();
    }
    let w = (x1 - x0) as usize;
    let h = (y1 - y0) as usize;
    let mut alpha = vec![0u8; w * h];
    let pw = placement.width as usize;
    for row in 0..h {
        let sy = (y0 + row as i32 - rel_y) as usize;
        let dst_row = &mut alpha[row * w..(row + 1) * w];
        let src_row = &coverage_buf[sy * pw..sy * pw + pw];
        for col in 0..w {
            let sx = (x0 + col as i32 - rel_x) as usize;
            let cov = src_row[sx];
            dst_row[col] = match parent {
                Some(p) => {
                    ((u32::from(cov) * u32::from(p.coverage_at(x0 + col as i32, y0 + row as i32))
                        + 127)
                        / 255) as u8
                }
                None => cov,
            };
        }
    }
    ClipMask {
        x0,
        y0,
        x1,
        y1,
        alpha,
    }
}

fn premul_rgba(color: Option<[u8; 4]>, extra_alpha: f32) -> [f32; 4] {
    let base = color.unwrap_or(LAYER_FOREGROUND);
    let a = (f32::from(base[3]) / 255.0 * extra_alpha).clamp(0.0, 1.0);
    [
        f32::from(base[0]) / 255.0 * a,
        f32::from(base[1]) / 255.0 * a,
        f32::from(base[2]) / 255.0 * a,
        a,
    ]
}

fn apply_extend(t: f32, extend: ColorExtend) -> f32 {
    match extend {
        ColorExtend::Pad => t.clamp(0.0, 1.0),
        ColorExtend::Repeat => t.rem_euclid(1.0),
        ColorExtend::Reflect => {
            let u = t.rem_euclid(2.0);
            if u > 1.0 { 2.0 - u } else { u }
        }
    }
}

fn sample_stops(stops: &[ColorStopPaint], t: f32, extend: ColorExtend) -> [f32; 4] {
    if stops.is_empty() {
        return [0.0; 4];
    }
    if stops.len() == 1 {
        return premul_rgba(stops[0].color, stops[0].alpha);
    }
    let u = apply_extend(t, extend);
    if u <= stops[0].offset {
        return premul_rgba(stops[0].color, stops[0].alpha);
    }
    let last = stops.len() - 1;
    if u >= stops[last].offset {
        return premul_rgba(stops[last].color, stops[last].alpha);
    }
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if u <= b.offset {
            let span = b.offset - a.offset;
            let frac = if span > 1e-6 {
                ((u - a.offset) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let ca = premul_rgba(a.color, a.alpha);
            let cb = premul_rgba(b.color, b.alpha);
            return [
                ca[0] + (cb[0] - ca[0]) * frac,
                ca[1] + (cb[1] - ca[1]) * frac,
                ca[2] + (cb[2] - ca[2]) * frac,
                ca[3] + (cb[3] - ca[3]) * frac,
            ];
        }
    }
    premul_rgba(stops[last].color, stops[last].alpha)
}

fn eval_brush(brush: &ColorBrush, p: [f32; 2]) -> [f32; 4] {
    match brush {
        ColorBrush::Solid { color, alpha } => premul_rgba(*color, *alpha),
        ColorBrush::LinearGradient {
            p0,
            p1,
            stops,
            extend,
        } => {
            let dx = p1[0] - p0[0];
            let dy = p1[1] - p0[1];
            let len_sq = dx * dx + dy * dy;
            if len_sq <= 1e-12 {
                return stops
                    .first()
                    .map_or([0.0; 4], |s| premul_rgba(s.color, s.alpha));
            }
            let t = ((p[0] - p0[0]) * dx + (p[1] - p0[1]) * dy) / len_sq;
            sample_stops(stops, t, *extend)
        }
        ColorBrush::RadialGradient {
            c0,
            r0,
            c1,
            r1,
            stops,
            extend,
        } => {
            let dcx = c1[0] - c0[0];
            let dcy = c1[1] - c0[1];
            let dr = r1 - r0;
            let dpx = p[0] - c0[0];
            let dpy = p[1] - c0[1];
            let a = dcx * dcx + dcy * dcy - dr * dr;
            let b = dpx * dcx + dpy * dcy + r0 * dr;
            let c = dpx * dpx + dpy * dpy - r0 * r0;
            let t = if a.abs() <= 1e-6 {
                if b.abs() <= 1e-6 {
                    return [0.0; 4];
                }
                let t = c / (2.0 * b);
                if r0 + t * dr < 0.0 {
                    return [0.0; 4];
                }
                t
            } else {
                let disc = b * b - a * c;
                if disc < 0.0 {
                    return [0.0; 4];
                }
                let sqrt_disc = disc.sqrt();
                let t1 = (b + sqrt_disc) / a;
                let t2 = (b - sqrt_disc) / a;
                let (t_hi, t_lo) = if t1 >= t2 { (t1, t2) } else { (t2, t1) };
                if r0 + t_hi * dr >= 0.0 {
                    t_hi
                } else if r0 + t_lo * dr >= 0.0 {
                    t_lo
                } else {
                    return [0.0; 4];
                }
            };
            sample_stops(stops, t, *extend)
        }
        ColorBrush::SweepGradient {
            c0,
            start_angle,
            end_angle,
            stops,
            extend,
        } => {
            let dx = p[0] - c0[0];
            let dy = p[1] - c0[1];
            let angle = (-dy).atan2(dx).to_degrees().rem_euclid(360.0);
            let span = end_angle - start_angle;
            if span.abs() <= 1e-6 {
                return match extend {
                    ColorExtend::Pad => {
                        if angle < *start_angle {
                            stops
                                .first()
                                .map_or([0.0; 4], |s| premul_rgba(s.color, s.alpha))
                        } else {
                            stops
                                .last()
                                .map_or([0.0; 4], |s| premul_rgba(s.color, s.alpha))
                        }
                    }
                    _ => [0.0; 4],
                };
            }
            let t = (angle - start_angle) / span;
            sample_stops(stops, t, *extend)
        }
    }
}

fn src_over(dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
    let inv = 1.0 - src[3];
    [
        src[0] + dst[0] * inv,
        src[1] + dst[1] * inv,
        src[2] + dst[2] * inv,
        src[3] + dst[3] * inv,
    ]
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(mut c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    if n < 0.0 {
        let d = l - n;
        if d > 1e-6 {
            for ch in &mut c {
                *ch = l + (*ch - l) * l / d;
            }
        }
    }
    if x > 1.0 {
        let d = x - l;
        if d > 1e-6 {
            for ch in &mut c {
                *ch = l + (*ch - l) * (1.0 - l) / d;
            }
        }
    }
    c
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mut idx = [0usize, 1, 2];
    idx.sort_by(|&i, &j| c[i].total_cmp(&c[j]));
    let [i_min, i_mid, i_max] = idx;
    let mut out = [0.0; 3];
    if c[i_max] > c[i_min] {
        out[i_mid] = ((c[i_mid] - c[i_min]) * s) / (c[i_max] - c[i_min]);
        out[i_max] = s;
    }
    out
}

fn blend_channel(cb: f32, cs: f32, mode: ColorCompositeMode) -> f32 {
    match mode {
        ColorCompositeMode::Multiply => cb * cs,
        ColorCompositeMode::Screen => cb + cs - cb * cs,
        ColorCompositeMode::Overlay => blend_channel(cs, cb, ColorCompositeMode::HardLight),
        ColorCompositeMode::Darken => cb.min(cs),
        ColorCompositeMode::Lighten => cb.max(cs),
        ColorCompositeMode::ColorDodge => {
            if cb <= 0.0 {
                0.0
            } else if cs >= 1.0 {
                1.0
            } else {
                (cb / (1.0 - cs)).min(1.0)
            }
        }
        ColorCompositeMode::ColorBurn => {
            if cb >= 1.0 {
                1.0
            } else if cs <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - cb) / cs).min(1.0)
            }
        }
        ColorCompositeMode::HardLight => {
            if cs <= 0.5 {
                cb * (2.0 * cs)
            } else {
                let s = 2.0 * cs - 1.0;
                cb + s - cb * s
            }
        }
        ColorCompositeMode::SoftLight => {
            if cs <= 0.5 {
                cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
            } else {
                let d = if cb <= 0.25 {
                    ((16.0 * cb - 12.0) * cb + 4.0) * cb
                } else {
                    cb.sqrt()
                };
                cb + (2.0 * cs - 1.0) * (d - cb)
            }
        }
        ColorCompositeMode::Difference => (cb - cs).abs(),
        ColorCompositeMode::Exclusion => cb + cs - 2.0 * cb * cs,
        _ => cs,
    }
}

fn composite_pixel(dst: [f32; 4], src: [f32; 4], mode: ColorCompositeMode) -> [f32; 4] {
    let sa = src[3].clamp(0.0, 1.0);
    let da = dst[3].clamp(0.0, 1.0);
    match mode {
        ColorCompositeMode::Clear => [0.0; 4],
        ColorCompositeMode::Src => src,
        ColorCompositeMode::Dest => dst,
        ColorCompositeMode::SrcOver => src_over(dst, src),
        ColorCompositeMode::DestOver => [
            dst[0] + src[0] * (1.0 - da),
            dst[1] + src[1] * (1.0 - da),
            dst[2] + src[2] * (1.0 - da),
            da + sa * (1.0 - da),
        ],
        ColorCompositeMode::SrcIn => [src[0] * da, src[1] * da, src[2] * da, sa * da],
        ColorCompositeMode::DestIn => [dst[0] * sa, dst[1] * sa, dst[2] * sa, da * sa],
        ColorCompositeMode::SrcOut => [
            src[0] * (1.0 - da),
            src[1] * (1.0 - da),
            src[2] * (1.0 - da),
            sa * (1.0 - da),
        ],
        ColorCompositeMode::DestOut => [
            dst[0] * (1.0 - sa),
            dst[1] * (1.0 - sa),
            dst[2] * (1.0 - sa),
            da * (1.0 - sa),
        ],
        ColorCompositeMode::SrcAtop => [
            src[0] * da + dst[0] * (1.0 - sa),
            src[1] * da + dst[1] * (1.0 - sa),
            src[2] * da + dst[2] * (1.0 - sa),
            da,
        ],
        ColorCompositeMode::DestAtop => [
            dst[0] * sa + src[0] * (1.0 - da),
            dst[1] * sa + src[1] * (1.0 - da),
            dst[2] * sa + src[2] * (1.0 - da),
            sa,
        ],
        ColorCompositeMode::Xor => [
            src[0] * (1.0 - da) + dst[0] * (1.0 - sa),
            src[1] * (1.0 - da) + dst[1] * (1.0 - sa),
            src[2] * (1.0 - da) + dst[2] * (1.0 - sa),
            sa + da - 2.0 * sa * da,
        ],
        ColorCompositeMode::Plus => [
            (src[0] + dst[0]).min(1.0),
            (src[1] + dst[1]).min(1.0),
            (src[2] + dst[2]).min(1.0),
            (sa + da).min(1.0),
        ],
        _ => {
            if sa <= 0.0 {
                return dst;
            }
            if da <= 0.0 {
                return src;
            }
            let cb = [
                (dst[0] / da).clamp(0.0, 1.0),
                (dst[1] / da).clamp(0.0, 1.0),
                (dst[2] / da).clamp(0.0, 1.0),
            ];
            let cs = [
                (src[0] / sa).clamp(0.0, 1.0),
                (src[1] / sa).clamp(0.0, 1.0),
                (src[2] / sa).clamp(0.0, 1.0),
            ];
            let blended = match mode {
                ColorCompositeMode::HslHue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
                ColorCompositeMode::HslSaturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
                ColorCompositeMode::HslColor => set_lum(cs, lum(cb)),
                ColorCompositeMode::HslLuminosity => set_lum(cb, lum(cs)),
                _ => [
                    blend_channel(cb[0], cs[0], mode),
                    blend_channel(cb[1], cs[1], mode),
                    blend_channel(cb[2], cs[2], mode),
                ],
            };
            let out_a = (sa + da - sa * da).clamp(0.0, 1.0);
            [
                (1.0 - sa) * dst[0] + (1.0 - da) * src[0] + sa * da * blended[0],
                (1.0 - sa) * dst[1] + (1.0 - da) * src[1] + sa * da * blended[1],
                (1.0 - sa) * dst[2] + (1.0 - da) * src[2] + sa * da * blended[2],
                out_a,
            ]
        }
    }
}

fn render_color_v1(
    recording: &ColorPaintRecording,
    size_px: f32,
    how: &Outlining,
    scratch: &mut zeno::Scratch,
) -> Option<GlyphImage> {
    if !size_px.is_finite() || size_px <= 0.0 {
        return None;
    }
    let root = root_colr_transform(recording.upem, size_px, how);
    let bounds = compute_colr_v1_bounds(recording, root);
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
    let left = bounds.min_x.floor() as i32;
    let bottom = bounds.min_y.floor() as i32;
    let right = bounds.max_x.ceil() as i32;
    let top = bounds.max_y.ceil() as i32;
    let width = (right - left).max(0) as u32;
    let height = (top - bottom).max(0) as u32;
    if width == 0 || height == 0 {
        return Some(GlyphImage {
            format: GlyphImageFormat::ColorRgba,
            width: 0,
            height: 0,
            left: 0,
            top: 0,
            data: Vec::new(),
        });
    }
    if width > MAX_COLR_V1_EDGE || height > MAX_COLR_V1_EDGE {
        return None;
    }
    let pixel_count = (width as usize).checked_mul(height as usize)?;
    let mut layers = vec![vec![[0.0f32; 4]; pixel_count]];
    let mut transforms = vec![root];
    let mut clips: Vec<ClipMask> = Vec::new();
    let mut scratch_pts = Vec::new();
    let mut scratch_verbs = Vec::new();
    let mut coverage_buf = Vec::new();

    for op in &recording.ops {
        let cur = *transforms.last().unwrap_or(&root);
        match op {
            ColorPaintOp::PushTransform(t) => transforms.push(transform_mul(cur, *t)),
            ColorPaintOp::PopTransform => {
                if transforms.len() > 1 {
                    transforms.pop();
                }
            }
            ColorPaintOp::PushClipGlyph(idx) => {
                let mask = match recording.outlines.get(*idx) {
                    Some(outline) => {
                        transformed_outline_into(
                            outline,
                            cur,
                            &mut scratch_pts,
                            &mut scratch_verbs,
                        );
                        rasterize_clip_mask(
                            &scratch_pts,
                            &scratch_verbs,
                            left,
                            top,
                            width,
                            height,
                            clips.last(),
                            scratch,
                            &mut coverage_buf,
                        )
                    }
                    None => ClipMask::empty(),
                };
                clips.push(mask);
            }
            ColorPaintOp::PushClipBox(rect) => {
                transformed_box_into(*rect, cur, &mut scratch_pts, &mut scratch_verbs);
                let mask = rasterize_clip_mask(
                    &scratch_pts,
                    &scratch_verbs,
                    left,
                    top,
                    width,
                    height,
                    clips.last(),
                    scratch,
                    &mut coverage_buf,
                );
                clips.push(mask);
            }
            ColorPaintOp::PopClip => {
                clips.pop();
            }
            ColorPaintOp::Fill(brush) => {
                let Some(surface) = layers.last_mut() else {
                    continue;
                };
                let (x0, y0, x1, y1, clip_ref) = match clips.last() {
                    Some(clip) => {
                        if clip.is_empty() {
                            continue;
                        }
                        (clip.x0, clip.y0, clip.x1, clip.y1, Some(clip))
                    }
                    None => (0, 0, width as i32, height as i32, None),
                };
                match brush {
                    ColorBrush::Solid { color, alpha } => {
                        let solid = premul_rgba(*color, *alpha);
                        if solid[3] <= 0.0 {
                            continue;
                        }
                        for py in y0..y1 {
                            let row_off = py as usize * width as usize;
                            for px in x0..x1 {
                                let cov = clip_ref
                                    .map_or(1.0, |c| f32::from(c.coverage_at(px, py)) / 255.0);
                                if cov <= 0.0 {
                                    continue;
                                }
                                let src = [
                                    solid[0] * cov,
                                    solid[1] * cov,
                                    solid[2] * cov,
                                    solid[3] * cov,
                                ];
                                let dst = &mut surface[row_off + px as usize];
                                *dst = src_over(*dst, src);
                            }
                        }
                    }
                    _ => {
                        let Some(inv) = transform_invert(cur) else {
                            continue;
                        };
                        for py in y0..y1 {
                            let row_off = py as usize * width as usize;
                            let world_y = top as f32 - py as f32 - 0.5;
                            for px in x0..x1 {
                                let cov = clip_ref
                                    .map_or(1.0, |c| f32::from(c.coverage_at(px, py)) / 255.0);
                                if cov <= 0.0 {
                                    continue;
                                }
                                let world_x = left as f32 + px as f32 + 0.5;
                                let local = transform_point(inv, [world_x, world_y]);
                                let c = eval_brush(brush, local);
                                if c[3] <= 0.0 {
                                    continue;
                                }
                                let src = [c[0] * cov, c[1] * cov, c[2] * cov, c[3] * cov];
                                let dst = &mut surface[row_off + px as usize];
                                *dst = src_over(*dst, src);
                            }
                        }
                    }
                }
            }
            ColorPaintOp::PushLayer => {
                layers.push(vec![[0.0f32; 4]; pixel_count]);
            }
            ColorPaintOp::PopLayer(mode) => {
                if layers.len() >= 2 {
                    let src = layers.pop().expect("len >= 2");
                    let dst = layers.last_mut().expect("len >= 1");
                    for (d, s) in dst.iter_mut().zip(src.iter()) {
                        *d = composite_pixel(*d, *s, *mode);
                    }
                }
            }
        }
    }

    let root_surface = layers.pop()?;
    let mut data = vec![0u8; pixel_count * 4];
    for (src, out) in root_surface.iter().zip(data.as_chunks_mut::<4>().0) {
        let a = src[3].clamp(0.0, 1.0);
        let a_u8 = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        if a_u8 == 0 {
            out.copy_from_slice(&[0, 0, 0, 0]);
        } else {
            let inv_a = 1.0 / a;
            out[0] = ((src[0] * inv_a) * 255.0).round().clamp(0.0, 255.0) as u8;
            out[1] = ((src[1] * inv_a) * 255.0).round().clamp(0.0, 255.0) as u8;
            out[2] = ((src[2] * inv_a) * 255.0).round().clamp(0.0, 255.0) as u8;
            out[3] = a_u8;
        }
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
                                    caps.has_svg,
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
                caps.has_svg,
                &color_how,
                &mut cache,
                &mut outline,
                &mut scratch,
            )
            .expect("glyph renders");
            if image.format == GlyphImageFormat::ColorRgba {
                assert!(!image.is_empty());
                assert!(image.data.as_chunks::<4>().0.iter().any(|px| px[3] > 0));
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
                    caps.has_svg,
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
        assert!(scaled.as_chunks::<4>().0.iter().any(|px| px[3] > 200));

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

    fn build_sfnt(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut sorted: Vec<(&[u8; 4], &[u8])> = tables
            .iter()
            .map(|(tag, data)| (*tag, data.as_slice()))
            .collect();
        sorted.sort_by_key(|(tag, _)| *tag);
        let num_tables = sorted.len() as u16;
        let search_range = (num_tables.next_power_of_two()
            >> (1 - usize::from(num_tables.is_power_of_two())))
            * 16;
        let entry_selector = (search_range / 16).trailing_zeros() as u16;
        let range_shift = num_tables * 16 - search_range;

        let mut out = Vec::new();
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&num_tables.to_be_bytes());
        out.extend_from_slice(&search_range.to_be_bytes());
        out.extend_from_slice(&entry_selector.to_be_bytes());
        out.extend_from_slice(&range_shift.to_be_bytes());

        let dir_len = 12 + sorted.len() * 16;
        let mut offset = dir_len;
        let mut body = Vec::new();
        for (tag, data) in &sorted {
            let mut sum = 0u32;
            for chunk in data.chunks(4) {
                let mut b = [0u8; 4];
                b[..chunk.len()].copy_from_slice(chunk);
                sum = sum.wrapping_add(u32::from_be_bytes(b));
            }
            out.extend_from_slice(*tag);
            out.extend_from_slice(&sum.to_be_bytes());
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            body.extend_from_slice(data);
            while !body.len().is_multiple_of(4) {
                body.push(0);
            }
            offset = dir_len + body.len();
        }
        out.extend_from_slice(&body);
        out
    }

    fn glyf_rect(x0: i16, y0: i16, x1: i16, y1: i16) -> Vec<u8> {
        let mut g = Vec::new();
        g.extend_from_slice(&1i16.to_be_bytes()); // numberOfContours
        g.extend_from_slice(&x0.to_be_bytes());
        g.extend_from_slice(&y0.to_be_bytes());
        g.extend_from_slice(&x1.to_be_bytes());
        g.extend_from_slice(&y1.to_be_bytes());
        g.extend_from_slice(&3u16.to_be_bytes()); // endPtsOfContours[0]
        g.extend_from_slice(&0u16.to_be_bytes()); // instructionLength
        // 4 points (short x/y deltas, flags: on-curve=1, x-short=2, y-short=4, same/pos=16/32)
        // p0=(x0,y0), p1=(x1-x0,0), p2=(0,y1-y0), p3=(-(x1-x0),0)
        // Use 16-bit signed vectors (flag=1 on-curve) so coordinates up to 1000 never overflow u8.
        g.extend_from_slice(&[1, 1, 1, 1]);
        let dx = x1 - x0;
        let dy = y1 - y0;
        for &vx in &[x0, dx, 0, -dx] {
            g.extend_from_slice(&vx.to_be_bytes());
        }
        for &vy in &[y0, 0, dy, 0] {
            g.extend_from_slice(&vy.to_be_bytes());
        }
        if !g.len().is_multiple_of(4) {
            while !g.len().is_multiple_of(4) {
                g.push(0);
            }
        }
        g
    }

    fn base_sfnt_tables(
        num_glyphs: u16,
        cmap_entries: &[(u32, u16)],
    ) -> Vec<(&'static [u8; 4], Vec<u8>)> {
        // head (54 bytes), unitsPerEm = 1000, indexToLocFormat = 1 (long loca)
        let mut head = vec![0u8; 54];
        head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[40..42].copy_from_slice(&1000i16.to_be_bytes());
        head[42..44].copy_from_slice(&1000i16.to_be_bytes());
        head[50..52].copy_from_slice(&1i16.to_be_bytes());

        // maxp v1.0 (32 bytes)
        let mut maxp = vec![0u8; 32];
        maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
        maxp[6..8].copy_from_slice(&8u16.to_be_bytes());
        maxp[8..10].copy_from_slice(&2u16.to_be_bytes());

        // hhea (36 bytes)
        let mut hhea = vec![0u8; 36];
        hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[10..12].copy_from_slice(&1000u16.to_be_bytes());
        hhea[34..36].copy_from_slice(&num_glyphs.to_be_bytes());

        // hmtx
        let mut hmtx = Vec::with_capacity(num_glyphs as usize * 4);
        for _ in 0..num_glyphs {
            hmtx.extend_from_slice(&1000u16.to_be_bytes());
            hmtx.extend_from_slice(&0i16.to_be_bytes());
        }

        // glyf + long loca: glyph 0 empty, glyphs 1..num_glyphs squares
        let mut glyf = Vec::new();
        let mut loca = Vec::with_capacity((num_glyphs as usize + 1) * 4);
        loca.extend_from_slice(&0u32.to_be_bytes());
        loca.extend_from_slice(&0u32.to_be_bytes());
        for gid in 1..num_glyphs {
            let rect = match gid {
                1 => glyf_rect(0, 0, 1000, 1000),
                2 => glyf_rect(150, 150, 850, 850),
                _ => glyf_rect(250, 250, 950, 950),
            };
            glyf.extend_from_slice(&rect);
            loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        }

        // cmap format 12
        let mut cmap = Vec::new();
        cmap.extend_from_slice(&0u16.to_be_bytes()); // version
        cmap.extend_from_slice(&1u16.to_be_bytes()); // numTables
        cmap.extend_from_slice(&3u16.to_be_bytes()); // platformID
        cmap.extend_from_slice(&10u16.to_be_bytes()); // encodingID
        cmap.extend_from_slice(&12u32.to_be_bytes()); // offset
        let sub_len = 16 + cmap_entries.len() as u32 * 12;
        cmap.extend_from_slice(&12u16.to_be_bytes()); // format 12
        cmap.extend_from_slice(&0u16.to_be_bytes());
        cmap.extend_from_slice(&sub_len.to_be_bytes());
        cmap.extend_from_slice(&0u32.to_be_bytes()); // language
        cmap.extend_from_slice(&(cmap_entries.len() as u32).to_be_bytes());
        for &(cp, gid) in cmap_entries {
            cmap.extend_from_slice(&cp.to_be_bytes());
            cmap.extend_from_slice(&cp.to_be_bytes());
            cmap.extend_from_slice(&(gid as u32).to_be_bytes());
        }

        vec![
            (b"head", head),
            (b"maxp", maxp),
            (b"hhea", hhea),
            (b"hmtx", hmtx),
            (b"glyf", glyf),
            (b"loca", loca),
            (b"cmap", cmap),
        ]
    }

    fn build_cpal(bgra: &[[u8; 4]]) -> Vec<u8> {
        let mut cpal = Vec::new();
        cpal.extend_from_slice(&0u16.to_be_bytes()); // version 0
        cpal.extend_from_slice(&(bgra.len() as u16).to_be_bytes());
        cpal.extend_from_slice(&1u16.to_be_bytes()); // numPalettes
        cpal.extend_from_slice(&(bgra.len() as u16).to_be_bytes());
        cpal.extend_from_slice(&14u32.to_be_bytes()); // colorRecordsArrayOffset
        cpal.extend_from_slice(&0u16.to_be_bytes()); // colorRecordIndices[0]
        for entry in bgra {
            cpal.extend_from_slice(entry);
        }
        cpal
    }

    fn push_u24(out: &mut Vec<u8>, v: u32) {
        out.push(((v >> 16) & 0xFF) as u8);
        out.push(((v >> 8) & 0xFF) as u8);
        out.push((v & 0xFF) as u8);
    }

    fn patch_u24(out: &mut [u8], at: usize, v: u32) {
        out[at] = ((v >> 16) & 0xFF) as u8;
        out[at + 1] = ((v >> 8) & 0xFF) as u8;
        out[at + 2] = (v & 0xFF) as u8;
    }

    fn push_color_line(out: &mut Vec<u8>, extend: u8, stops: &[(i16, u16, i16)]) {
        out.push(extend);
        out.extend_from_slice(&(stops.len() as u16).to_be_bytes());
        for &(offset, palette_idx, alpha) in stops {
            out.extend_from_slice(&offset.to_be_bytes());
            out.extend_from_slice(&palette_idx.to_be_bytes());
            out.extend_from_slice(&alpha.to_be_bytes());
        }
    }

    fn build_synthetic_colrv1_font() -> Vec<u8> {
        let mut tables =
            base_sfnt_tables(4, &[(b'A' as u32, 1), (b'B' as u32, 2), (b'C' as u32, 3)]);
        // Palette: 0=red, 1=green, 2=blue, 3=yellow (BGRA)
        let cpal = build_cpal(&[
            [0, 0, 255, 255],
            [0, 255, 0, 255],
            [255, 0, 0, 255],
            [0, 255, 255, 255],
        ]);

        // LayerList: 2 layers used by glyph 1's PaintColrLayers
        // Layer 0: PaintGlyph(gid=1) -> PaintLinearGradient(red -> blue)
        // Layer 1: PaintTransform(scale 0.5, translate 250,250) -> PaintGlyph(gid=2) -> PaintSolid(green)
        let mut layer_list = Vec::new();
        layer_list.extend_from_slice(&2u32.to_be_bytes()); // numLayers
        layer_list.extend_from_slice(&0u32.to_be_bytes()); // layer[0] offset placeholder
        layer_list.extend_from_slice(&0u32.to_be_bytes()); // layer[1] offset placeholder

        let l0 = layer_list.len() as u32;
        layer_list[4..8].copy_from_slice(&l0.to_be_bytes());
        // PaintGlyph: format=10, offset=6, gid=1
        layer_list.push(10);
        push_u24(&mut layer_list, 6);
        layer_list.extend_from_slice(&1u16.to_be_bytes());
        // PaintLinearGradient: format=4, colorLineOffset=16, x0=0,y0=0,x1=1000,y1=0,x2=0,y2=1000
        layer_list.push(4);
        push_u24(&mut layer_list, 16);
        for v in [0i16, 0, 1000, 0, 0, 1000] {
            layer_list.extend_from_slice(&v.to_be_bytes());
        }
        push_color_line(&mut layer_list, 0, &[(0, 0, 0x4000), (0x4000, 2, 0x4000)]);

        let l1 = layer_list.len() as u32;
        layer_list[8..12].copy_from_slice(&l1.to_be_bytes());
        // PaintTransform: format=12, paintOffset (placeholder), transformOffset (placeholder)
        let t_start = layer_list.len();
        layer_list.push(12);
        push_u24(&mut layer_list, 0);
        push_u24(&mut layer_list, 0);
        let paint_off = (layer_list.len() - t_start) as u32;
        patch_u24(&mut layer_list, t_start + 1, paint_off);
        // PaintGlyph(gid=2) -> PaintSolid(palette=1, alpha=1.0)
        layer_list.push(10);
        push_u24(&mut layer_list, 6);
        layer_list.extend_from_slice(&2u16.to_be_bytes());
        layer_list.push(2);
        layer_list.extend_from_slice(&1u16.to_be_bytes());
        layer_list.extend_from_slice(&0x4000i16.to_be_bytes());
        let xform_off = (layer_list.len() - t_start) as u32;
        patch_u24(&mut layer_list, t_start + 4, xform_off);
        // Affine2x3 (Fixed 16.16): xx=0.5, yx=0, xy=0, yy=0.5, dx=250, dy=250
        for v in [0x8000i32, 0, 0, 0x8000, 250 << 16, 250 << 16] {
            layer_list.extend_from_slice(&v.to_be_bytes());
        }

        // BaseGlyphList: 3 records (gid 1, 2, 3)
        let mut base_list = Vec::new();
        base_list.extend_from_slice(&3u32.to_be_bytes());
        for gid in 1u16..=3 {
            base_list.extend_from_slice(&gid.to_be_bytes());
            base_list.extend_from_slice(&0u32.to_be_bytes());
        }

        // Base gid 1: PaintColrLayers(numLayers=2, firstLayerIndex=0)
        let b1 = base_list.len() as u32;
        base_list[6..10].copy_from_slice(&b1.to_be_bytes());
        base_list.push(1);
        base_list.push(2);
        base_list.extend_from_slice(&0u32.to_be_bytes());

        // Base gid 2: PaintComposite(Xor=11, source=PaintGlyph(3,Sweep), backdrop=PaintGlyph(1,Radial))
        let b2 = base_list.len() as u32;
        base_list[12..16].copy_from_slice(&b2.to_be_bytes());
        let comp_start = base_list.len();
        base_list.push(32);
        push_u24(&mut base_list, 0); // src offset placeholder
        base_list.push(11); // Xor
        push_u24(&mut base_list, 0); // dst offset placeholder
        let src_off = (base_list.len() - comp_start) as u32;
        patch_u24(&mut base_list, comp_start + 1, src_off);
        base_list.push(10); // PaintGlyph(3)
        push_u24(&mut base_list, 6);
        base_list.extend_from_slice(&3u16.to_be_bytes());
        base_list.push(8); // PaintSweepGradient: offset=12, cx=500, cy=500, start=0, end=1.0
        push_u24(&mut base_list, 12);
        base_list.extend_from_slice(&500i16.to_be_bytes());
        base_list.extend_from_slice(&500i16.to_be_bytes());
        base_list.extend_from_slice(&0i16.to_be_bytes());
        base_list.extend_from_slice(&0x4000i16.to_be_bytes());
        push_color_line(&mut base_list, 1, &[(0, 0, 0x4000), (0x4000, 1, 0x4000)]);
        let dst_off = (base_list.len() - comp_start) as u32;
        patch_u24(&mut base_list, comp_start + 5, dst_off);
        base_list.push(10); // PaintGlyph(1)
        push_u24(&mut base_list, 6);
        base_list.extend_from_slice(&1u16.to_be_bytes());
        base_list.push(6); // PaintRadialGradient: offset=16, (500,500,50)->(500,500,500)
        push_u24(&mut base_list, 16);
        base_list.extend_from_slice(&500i16.to_be_bytes());
        base_list.extend_from_slice(&500i16.to_be_bytes());
        base_list.extend_from_slice(&50u16.to_be_bytes());
        base_list.extend_from_slice(&500i16.to_be_bytes());
        base_list.extend_from_slice(&500i16.to_be_bytes());
        base_list.extend_from_slice(&500u16.to_be_bytes());
        push_color_line(&mut base_list, 2, &[(0, 3, 0x4000), (0x4000, 2, 0x4000)]);

        // Base gid 3: PaintGlyph(1) -> PaintSolid(red) clipped by ClipList [200,200..600,600]
        let b3 = base_list.len() as u32;
        base_list[18..22].copy_from_slice(&b3.to_be_bytes());
        base_list.push(10);
        push_u24(&mut base_list, 6);
        base_list.extend_from_slice(&1u16.to_be_bytes());
        base_list.push(2);
        base_list.extend_from_slice(&0u16.to_be_bytes());
        base_list.extend_from_slice(&0x4000i16.to_be_bytes());

        // ClipList: format=1, numClips=1, (3..=3 -> offset=12), ClipBoxFormat1(200,200,600,600)
        let mut clip_list = Vec::new();
        clip_list.push(1);
        clip_list.extend_from_slice(&1u32.to_be_bytes());
        clip_list.extend_from_slice(&3u16.to_be_bytes());
        clip_list.extend_from_slice(&3u16.to_be_bytes());
        push_u24(&mut clip_list, 12);
        clip_list.push(1); // ClipBoxFormat1
        for v in [200i16, 200, 600, 600] {
            clip_list.extend_from_slice(&v.to_be_bytes());
        }

        let header_len = 34u32;
        let base_off = header_len;
        let layer_off = base_off + base_list.len() as u32;
        let clip_off = layer_off + layer_list.len() as u32;
        let mut colr = Vec::new();
        colr.extend_from_slice(&1u16.to_be_bytes()); // version 1
        colr.extend_from_slice(&0u16.to_be_bytes());
        colr.extend_from_slice(&0u32.to_be_bytes());
        colr.extend_from_slice(&0u32.to_be_bytes());
        colr.extend_from_slice(&0u16.to_be_bytes());
        colr.extend_from_slice(&base_off.to_be_bytes());
        colr.extend_from_slice(&layer_off.to_be_bytes());
        colr.extend_from_slice(&clip_off.to_be_bytes());
        colr.extend_from_slice(&0u32.to_be_bytes());
        colr.extend_from_slice(&0u32.to_be_bytes());
        colr.extend_from_slice(&base_list);
        colr.extend_from_slice(&layer_list);
        colr.extend_from_slice(&clip_list);

        tables.push((b"CPAL", cpal));
        tables.push((b"COLR", colr));
        build_sfnt(&tables)
    }

    #[test]
    fn colr_v1_layers_gradients_transforms_composite_and_clipbox_rasterize_to_color_rgba() {
        let font = build_synthetic_colrv1_font();
        let caps = read_raster_caps(&font, 0).expect("synthetic colrv1 caps");
        assert!(caps.has_color_outlines);
        assert!(caps.has_outlines);

        let mut cache = FaceScalerCache::default();
        let mut outline = ScaledOutline::default();
        let mut scratch = zeno::Scratch::new();
        let color_how = Outlining {
            format: zeno::Format::Alpha,
            offset: zeno::Vector::new(0.0, 0.0),
            transform: None,
            mode: GlyphRenderMode::Mask,
        };

        let mut unique_colors = |gid: u16, size: f32| -> (GlyphImage, usize) {
            let img = drawn(
                &font,
                0,
                9001,
                &[],
                size,
                false,
                gid,
                caps.has_outlines,
                caps.has_color_outlines,
                caps.has_color_bitmaps,
                caps.has_svg,
                &color_how,
                &mut cache,
                &mut outline,
                &mut scratch,
            )
            .expect("colrv1 glyph renders");
            assert_eq!(img.format, GlyphImageFormat::ColorRgba);
            let distinct = img
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|px| px[3] > 20)
                .map(|px| (px[0] / 32, px[1] / 32, px[2] / 32))
                .collect::<std::collections::HashSet<_>>()
                .len();
            (img, distinct)
        };

        // Glyph 1: linear gradient + transformed green overlay must yield multiple distinct colors.
        let (img1, colors1) = unique_colors(1, 32.0);
        assert!(
            colors1 >= 3,
            "expected gradient + solid green, got {colors1}"
        );
        assert!(img1.width >= 28 && img1.height >= 28);

        // Glyph 2: radial + sweep + Xor composite must produce multiple distinct colors.
        let (_img2, colors2) = unique_colors(2, 32.0);
        assert!(
            colors2 >= 3,
            "expected sweep/radial composite colors, got {colors2}"
        );

        // Glyph 3: 1000-unit square clipped to [200..600] (400 units wide) at 50px -> ~20px box, not ~50px.
        let (img3, _) = unique_colors(3, 50.0);
        assert!(
            img3.width <= 24 && img3.height <= 24,
            "clip box should bound glyph 3 to ~20x20, got {}x{}",
            img3.width,
            img3.height
        );
        // Derived stroke mode stays a coverage mask rather than color layers.
        let stroke = drawn(
            &font,
            0,
            9001,
            &[],
            32.0,
            false,
            1,
            caps.has_outlines,
            caps.has_color_outlines,
            caps.has_color_bitmaps,
            caps.has_svg,
            &Outlining {
                format: zeno::Format::Alpha,
                offset: zeno::Vector::new(0.0, 0.0),
                transform: None,
                mode: GlyphRenderMode::Stroke {
                    width_q: 4,
                    join: StrokeJoin::Round,
                },
            },
            &mut cache,
            &mut outline,
            &mut scratch,
        )
        .expect("derived stroke renders");
        assert_eq!(stroke.format, GlyphImageFormat::Mask);
    }

    #[cfg(feature = "svg")]
    #[test]
    fn opentype_svg_glyphs_rasterize_to_color_rgba_and_share_documents() {
        let mut tables = base_sfnt_tables(3, &[(b'S' as u32, 1), (b'T' as u32, 2)]);
        let svg_doc = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -1000 1000 1000"><rect id="glyph1" x="100" y="-900" width="800" height="800" fill="#ff2200"/><circle id="glyph2" cx="500" cy="-500" r="350" fill="#0088ff"/></svg>"##;
        // SVG header (10 bytes) + DocumentList (1 entry covering glyph 1..=2)
        let mut svg_table = Vec::new();
        svg_table.extend_from_slice(&0u16.to_be_bytes()); // version
        svg_table.extend_from_slice(&10u32.to_be_bytes()); // svgDocumentListOffset
        svg_table.extend_from_slice(&0u32.to_be_bytes()); // reserved
        // SVGDocumentList: numEntries=1, record(1..=2, docOffset=14 from list start, len)
        svg_table.extend_from_slice(&1u16.to_be_bytes());
        svg_table.extend_from_slice(&1u16.to_be_bytes());
        svg_table.extend_from_slice(&2u16.to_be_bytes());
        svg_table.extend_from_slice(&14u32.to_be_bytes());
        svg_table.extend_from_slice(&(svg_doc.len() as u32).to_be_bytes());
        svg_table.extend_from_slice(svg_doc);
        tables.push((b"SVG ", svg_table));
        let font = build_sfnt(&tables);

        let caps = read_raster_caps(&font, 0).expect("synthetic svg caps");
        assert!(caps.has_svg);
        assert!(!caps.has_color_outlines);

        let mut cache = FaceScalerCache::default();
        let mut outline = ScaledOutline::default();
        let mut scratch = zeno::Scratch::new();
        let how = Outlining {
            format: zeno::Format::Alpha,
            offset: zeno::Vector::new(0.0, 0.0),
            transform: None,
            mode: GlyphRenderMode::Mask,
        };

        let g1 = drawn(
            &font,
            0,
            9002,
            &[],
            40.0,
            false,
            1,
            caps.has_outlines,
            caps.has_color_outlines,
            caps.has_color_bitmaps,
            caps.has_svg,
            &how,
            &mut cache,
            &mut outline,
            &mut scratch,
        )
        .expect("svg glyph 1 renders");
        assert_eq!(g1.format, GlyphImageFormat::ColorRgba);
        assert!(
            g1.data
                .as_chunks::<4>()
                .0
                .iter()
                .any(|px| px[0] > 200 && px[2] < 40 && px[3] > 200)
        );

        let g2 = drawn(
            &font,
            0,
            9002,
            &[],
            40.0,
            false,
            2,
            caps.has_outlines,
            caps.has_color_outlines,
            caps.has_color_bitmaps,
            caps.has_svg,
            &how,
            &mut cache,
            &mut outline,
            &mut scratch,
        )
        .expect("svg glyph 2 renders from shared SVG document");
        assert_eq!(g2.format, GlyphImageFormat::ColorRgba);
        assert!(
            g2.data
                .as_chunks::<4>()
                .0
                .iter()
                .any(|px| px[2] > 200 && px[0] < 40 && px[3] > 200)
        );
    }
}
