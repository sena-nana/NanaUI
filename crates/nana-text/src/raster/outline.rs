//! [`OutlineRasterizer`]: `swash` behind the raster boundary. The only file
//! of this crate that names it (`scripts/check-engine-boundary.py`).

use nana_ui_core::sort::CompactSort as _;
use std::collections::HashMap;
use std::sync::Arc;

use crate::font::{AxisCoord, FontInstanceKey};

use super::{
    GlyphFontId, GlyphImage, GlyphImageFormat, GlyphRasterKey, GlyphRenderMode, GlyphSynthesis,
    GlyphVariationId, StrokeJoin, VARIATION_CAP, blur_image, encode_subpixel, size_from_bits,
};

/// Synthetic-oblique slant, in degrees: 14°, the angle the previous text
/// backend used, so a face without an italic kept leaning the same way through
/// the #99 cutover.
const OBLIQUE_DEGREES: f32 = 14.0;

/// Synthetic-bold stroke as a fraction of the raster size. Stroke weight is
/// proportional to size in every real face, so faking it with a constant
/// number of pixels would over-embolden captions and under-embolden headings.
const EMBOLDEN_RATIO: f32 = 0.02;

fn zeno_join(join: StrokeJoin) -> swash::zeno::Join {
    match join {
        StrokeJoin::Miter => swash::zeno::Join::Miter,
        StrokeJoin::Bevel => swash::zeno::Join::Bevel,
        StrokeJoin::Round => swash::zeno::Join::Round,
    }
}

/// Frames a set stays after its last use, whatever the count: the frame that
/// interned it and the one after may still rasterize with it.
const VARIATION_PIN_FRAMES: u64 = 2;

struct InternedVariation {
    coords: Arc<[AxisCoord]>,
    last_used: u64,
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
    /// Swash's own scaler, which carries its outline cache. Every *cached
    /// answer* lives in the caller's cache (the renderer keeps
    /// one keyed by [`GlyphRasterKey`]); this is only the machinery that produces them.
    context: swash::scale::ScaleContext,
    /// The outline being drawn and the rasterizer's working memory, kept
    /// between glyphs so a glyph that misses allocates only its image.
    outline: swash::scale::outline::Outline,
    scratch: swash::zeno::Scratch,
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
            context: swash::scale::ScaleContext::new(),
            outline: swash::scale::outline::Outline::new(),
            scratch: swash::zeno::Scratch::new(),
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
                self.context = swash::scale::ScaleContext::new();
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
}

impl OutlineRasterizer {
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
        let font = swash::FontRef::from_index(data.bytes(), data.index() as usize)?;
        let size = size_from_bits(key.size_bits);
        let coords = self.variation_coords(key.variation).cloned();
        let mut builder = self
            .context
            .builder(font)
            .size(size)
            .hint(!key.synthesis.contains(GlyphSynthesis::DISABLE_HINTING))
            // swash keeps the coordinates in the shared context and neither
            // `builder` nor `variations` resets them: without this a
            // default-instance glyph is drawn at whatever axis position the
            // previous glyph of any variable face asked for.
            .normalized_coords(std::iter::empty::<swash::NormalizedCoord>());
        if let Some(coords) = coords {
            builder = builder.variations(
                coords
                    .iter()
                    .map(|coord| (swash::Tag::from_be_bytes(coord.tag), coord.value)),
            );
        }
        let mut scaler = builder.build();
        // A face with no outlines has only strikes, and a strike cannot be
        // shifted by a fraction of a pixel — asking for one blurs it. Asked of
        // the scaler rather than carried as a synthesis flag, because it is a
        // property of the face, not of what the caller wanted.
        let snap_to_pixel =
            !scaler.has_outlines() || key.synthesis.contains(GlyphSynthesis::PIXEL_FONT);
        let format = match key.mode {
            GlyphRenderMode::SubpixelRgb | GlyphRenderMode::SubpixelBgr => {
                swash::zeno::Format::Subpixel
            }
            GlyphRenderMode::Mask
            | GlyphRenderMode::Stroke { .. }
            | GlyphRenderMode::Blur { .. } => swash::zeno::Format::Alpha,
        };
        let offset = subpixel_offset(key, snap_to_pixel);
        let mut transform = None;
        if key.synthesis.contains(GlyphSynthesis::FAKE_ITALIC) {
            transform = Some(swash::zeno::Transform::skew(
                swash::zeno::Angle::from_degrees(OBLIQUE_DEGREES),
                swash::zeno::Angle::from_degrees(0.0),
            ));
        }
        if key.synthesis.contains(GlyphSynthesis::ROTATE_CW) {
            // Font space is y-up, so a quarter turn clockwise on the page
            // sends the outline's x to page-down (font −y) and its y to
            // page-right (font +x): (x, y) → (y, −x). Written out rather than
            // as `rotation(-90°)`, whose `cos` is not exactly zero and would
            // lean every sideways glyph by a hair. After the oblique skew, so
            // a fake italic slants along its own baseline.
            let quarter_turn = swash::zeno::Transform::new(0.0, -1.0, 1.0, 0.0, 0.0, 0.0);
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
        // Colour sources take no embolden (swash skips it for them too).
        let plain =
            key.mode.is_derived() || !(scaler.has_color_outlines() || scaler.has_color_bitmaps());
        let how = Outlining {
            format,
            offset,
            transform,
            mode: key.mode,
        };
        let image = if embolden > 0.0 && plain && scaler.has_outlines() {
            emboldened(&mut scaler, glyph_id, embolden, &how)?
        } else {
            drawn(
                &mut scaler,
                &font,
                glyph_id,
                &how,
                &mut self.outline,
                &mut self.scratch,
            )?
        };
        let width = image.placement.width;
        let height = image.placement.height;
        let pixels = (width as usize).saturating_mul(height as usize);
        let format = match image.content {
            swash::scale::image::Content::Mask => GlyphImageFormat::Mask,
            swash::scale::image::Content::Color => GlyphImageFormat::ColorRgba,
            swash::scale::image::Content::SubpixelMask => GlyphImageFormat::SubpixelRgb,
        };
        let bytes = pixels * format.bytes_per_pixel();
        if image.data.len() < bytes {
            return None;
        }
        let mut data = image.data[..bytes].to_vec();
        if format == GlyphImageFormat::SubpixelRgb {
            for pixel in data.as_chunks_mut::<4>().0 {
                let rgb = [pixel[0], pixel[1], pixel[2]];
                encode_subpixel(pixel, rgb, key.mode);
            }
        }
        let image = GlyphImage {
            format,
            width,
            height,
            left: image.placement.left,
            top: image.placement.top,
            data,
        };
        match key.mode {
            GlyphRenderMode::Blur { radius_q, .. } if format == GlyphImageFormat::Mask => {
                Some(blur_image(image, f32::from(radius_q) * 0.25))
            }
            _ => Some(image),
        }
    }
}

/// How [`drawn`] and [`emboldened`] draw the outline.
struct Outlining {
    format: swash::zeno::Format,
    offset: swash::zeno::Vector,
    transform: Option<swash::zeno::Transform>,
    mode: GlyphRenderMode,
}

/// The glyph from the first source that has it: a colour outline in the
/// face's first palette, then a colour strike, then the plain outline. Order
/// matters: an emoji face can have all three, and the first is the one with
/// the palette applied. A stroke or a shadow is coverage of the outline
/// itself, whatever colours the face would paint it in.
///
/// This is `swash::scale::Render::render`, written here because that one
/// compiles the mask rasterizer a second time inside swash: [`emboldened`]
/// and [`path_coverage`] already compile it in this crate, the two copies
/// are not merged, and each is about 130 KiB.
fn drawn(
    scaler: &mut swash::scale::Scaler,
    font: &swash::FontRef,
    glyph_id: u16,
    how: &Outlining,
    outline: &mut swash::scale::outline::Outline,
    scratch: &mut swash::zeno::Scratch,
) -> Option<swash::scale::image::Image> {
    use swash::scale::image::{Content, Image};
    use swash::zeno::{Format, Mask, Origin, Stroke, Style};
    if !how.mode.is_derived() {
        if scaler.has_color_outlines()
            && scaler.scale_color_outline_into(glyph_id, outline)
            && let Some(image) = color_layers(font, how, outline, scratch)
        {
            return Some(image);
        }
        if scaler.has_color_bitmaps() {
            let mut image = Image::new();
            let strike = swash::scale::StrikeWith::BestFit;
            if scaler.scale_color_bitmap_into(glyph_id, strike, &mut image) {
                return Some(image);
            }
        }
    }
    if !scaler.has_outlines() || !scaler.scale_outline_into(glyph_id, outline) {
        return None;
    }
    if let Some(transform) = &how.transform {
        outline.transform(transform);
    }
    let style = match how.mode {
        GlyphRenderMode::Stroke { width_q, join } => {
            let mut stroke = Stroke::new(f32::from(width_q) * 0.25);
            stroke.join(zeno_join(join));
            Style::Stroke(stroke)
        }
        _ => Style::default(),
    };
    let mut image = Image::new();
    let mut data = Vec::new();
    image.placement = Mask::with_scratch(outline.path(), scratch)
        .format(how.format)
        .origin(Origin::BottomLeft)
        .style(style)
        .offset(how.offset)
        .render_offset(how.offset)
        .inspect(|format, width, height| data.resize(format.buffer_size(width, height), 0))
        .render_into(&mut data[..], None);
    image.data = data;
    image.source = swash::scale::Source::Outline;
    image.content = if how.format == Format::Alpha {
        Content::Mask
    } else {
        Content::SubpixelMask
    };
    Some(image)
}

/// What a colour layer is painted in when the face's palette has no entry
/// for it.
const LAYER_FOREGROUND: [u8; 4] = [128, 128, 128, 255];

/// A scaled colour outline's layers, each filled in its palette colour and
/// laid over the ones before it. `None` when a layer cannot be read.
fn color_layers(
    font: &swash::FontRef,
    how: &Outlining,
    outline: &mut swash::scale::outline::Outline,
    scratch: &mut swash::zeno::Scratch,
) -> Option<swash::scale::image::Image> {
    use swash::zeno::{Mask, Origin};
    if let Some(transform) = &how.transform {
        outline.transform(transform);
    }
    let palette = font.color_palettes().next();
    let bounds = outline.bounds();
    // The image is the whole glyph's box; a layer's mask is placed in it by
    // where the two boxes sit on the same pixel grid.
    let left = (bounds.min.x + how.offset.x).floor() as i32;
    let bottom = (bounds.min.y + how.offset.y).ceil() as i32;
    let width = bounds.width().ceil() as u32;
    let height = bounds.height().ceil() as u32;
    let top = height as i32 + bottom;
    let mut image = swash::scale::image::Image::new();
    image.data = vec![0; width as usize * height as usize * 4];
    image.placement.left = left;
    image.placement.top = top;
    image.placement.width = width;
    image.placement.height = height;
    let mut coverage = Vec::new();
    for index in 0..outline.len() {
        let layer = outline.get(index)?;
        coverage.clear();
        let placement = Mask::with_scratch(layer.path(), scratch)
            .origin(Origin::BottomLeft)
            .offset(how.offset)
            .render_offset(how.offset)
            .inspect(|format, width, height| coverage.resize(format.buffer_size(width, height), 0))
            .render_into(&mut coverage[..], None);
        let color = layer
            .color_index()
            .and_then(|entry| palette.map(|palette| palette.get(entry)))
            .unwrap_or(LAYER_FOREGROUND);
        tint_over(
            &coverage,
            [placement.width, placement.height],
            [
                placement.left.wrapping_sub(left),
                top.wrapping_sub(placement.top),
            ],
            color,
            &mut image.data,
            [width, height],
        );
    }
    image.source = swash::scale::Source::ColorOutline(0);
    image.content = swash::scale::image::Content::Color;
    Some(image)
}

/// Lays `color` over an RGBA `target` through a coverage `mask` whose
/// top-left pixel goes at `at`; what falls outside the target is dropped.
///
/// The arithmetic is swash's, shift for shift, so a colour glyph keeps the
/// pixels it had: coverage times the colour's alpha, divided by 256 rather
/// than 255, which leaves a fully covered opaque pixel at 254.
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
/// px, which thickens a stem as much as `Render::embolden(amount)` does a
/// correctly wound one (that grows it up and right by twice `amount`).
///
/// swash's embolden takes the outline's winding from all of its contours
/// joined into one polygon (swash 0.2.10), and where that sign comes out
/// wrong it pulls every edge *in*: in a fake-bold run Noto Sans SC's 传 and
/// 仍, LXGW WenKai's た or Fusion Pixel's ほ came out with less ink than their
/// regular weight.
/// The fill with a round-joined stroke under it moves every edge out
/// whatever the contours' order or direction. A stroked glyph is stroked
/// that much wider instead: the fill drawn over it hides the inner half.
fn emboldened(
    scaler: &mut swash::scale::Scaler,
    glyph_id: u16,
    amount: f32,
    how: &Outlining,
) -> Option<swash::scale::image::Image> {
    use swash::zeno::{Fill, Format, Join, Mask, Origin, Stroke, Style};
    let mut outline = scaler.scale_outline(glyph_id)?;
    if let Some(transform) = &how.transform {
        outline.transform(transform);
    }
    // Sized before rendering, as swash does: `Mask::render` places a
    // bottom-left origin mask before it knows the mask's height.
    let mask = |style: Style| {
        let mut data = Vec::new();
        let placement = Mask::new(outline.path())
            .format(how.format)
            .origin(Origin::BottomLeft)
            .style(style)
            .offset(how.offset)
            .render_offset(how.offset)
            .inspect(|format, width, height| data.resize(format.buffer_size(width, height), 0))
            .render_into(&mut data[..], None);
        (data, placement)
    };
    let (data, placement) = if let GlyphRenderMode::Stroke { width_q, join } = how.mode {
        let mut stroke = Stroke::new(f32::from(width_q) * 0.25 + 2.0 * amount);
        stroke.join(zeno_join(join));
        mask(stroke.into())
    } else {
        let mut stroke = Stroke::new(2.0 * amount);
        stroke.join(Join::Round);
        let (mut data, outer) = mask(stroke.into());
        let (fill, inner) = mask(Style::Fill(Fill::NonZero));
        let bpp = if how.format == Format::Alpha { 1 } else { 4 };
        // Both masks are in the same space; the stroke's reaches past the
        // fill's on every side, so the fill is copied in at an offset.
        let (dx, dy) = (inner.left - outer.left, outer.top - inner.top);
        let pitch = inner.width as usize * bpp;
        for (row, line) in fill.chunks_exact(pitch.max(1)).enumerate() {
            let y = dy + row as i32;
            if y < 0 || y >= outer.height as i32 || dx < 0 {
                continue;
            }
            let start = (y as usize * outer.width as usize + dx as usize) * bpp;
            let Some(target) = data.get_mut(start..start + line.len()) else {
                continue;
            };
            for (out, value) in target.iter_mut().zip(line) {
                *out = (*out).max(*value);
            }
        }
        (data, outer)
    };
    let mut image = swash::scale::image::Image::new();
    image.data = data;
    image.placement = placement;
    image.source = swash::scale::Source::Outline;
    image.content = if how.format == Format::Alpha {
        swash::scale::image::Content::Mask
    } else {
        swash::scale::image::Content::SubpixelMask
    };
    Some(image)
}

/// The fractional pen offset a glyph is rendered at, snapped whole for a face
/// that has nothing to shift.
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
    use swash::zeno::{Cap, Fill, Format, Join, Mask, Stroke, Style, Transform, Vector, Verb};
    let mut coverage = vec![0; size as usize * size as usize];
    if size == 0 || steps.is_empty() {
        return coverage;
    }
    let points: Vec<Vector> = points
        .iter()
        .map(|point| Vector::new(point[0], point[1]))
        .collect();
    let verbs: Vec<Verb> = steps
        .iter()
        .map(|step| match step {
            PathStep::MoveTo => Verb::MoveTo,
            PathStep::LineTo => Verb::LineTo,
            PathStep::CurveTo => Verb::CurveTo,
            PathStep::Close => Verb::Close,
        })
        .collect();
    let mut stroke;
    let style = match paint {
        PathPaint::Fill => Style::Fill(Fill::NonZero),
        PathPaint::Stroke { width } => {
            stroke = Stroke::new(width);
            stroke.cap(Cap::Round).join(Join::Round);
            Style::from(&stroke)
        }
    };
    Mask::new((&points[..], &verbs[..]))
        .style(style)
        .transform(Some(Transform::scale(scale, scale)))
        .format(Format::Alpha)
        .size(size, size)
        .render_into(&mut coverage, None);
    coverage
}

fn subpixel_offset(key: &GlyphRasterKey, snap: bool) -> swash::zeno::Vector {
    let x = key.subpixel_x.as_float();
    let y = key.subpixel_y.as_float();
    if snap {
        swash::zeno::Vector::new(x.round(), y.round())
    } else {
        swash::zeno::Vector::new(x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swash::zeno::{Angle, Format, Transform, Vector};

    const COLOR: &[u8] = include_bytes!("../../fonts/nana-test-color.ttf");
    const EMOJI: &[u8] = include_bytes!("../../fonts/noto-emoji.ttf");
    const AXES: &[u8] = include_bytes!("../../fonts/nana-test-axes.ttf");
    const KOREAN: &[u8] = include_bytes!("../../fonts/noto-sans-kr.ttf");

    /// What this file drew glyphs with before [`drawn`]: swash's own
    /// renderer, configured the way `rasterize` configured it.
    fn reference(
        scaler: &mut swash::scale::Scaler,
        glyph_id: u16,
        how: &Outlining,
    ) -> Option<swash::scale::image::Image> {
        use swash::scale::{Render, Source, StrikeWith};
        const FACE_SOURCES: &[Source] = &[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ];
        const OUTLINE_ONLY: &[Source] = &[Source::Outline];
        let mut render = Render::new(if how.mode.is_derived() {
            OUTLINE_ONLY
        } else {
            FACE_SOURCES
        });
        render.format(how.format).offset(how.offset);
        if let GlyphRenderMode::Stroke { width_q, join } = how.mode {
            let mut stroke = swash::zeno::Stroke::new(f32::from(width_q) * 0.25);
            stroke.join(zeno_join(join));
            render.style(stroke);
        }
        render.transform(how.transform);
        render.render(scaler, glyph_id)
    }

    /// Every glyph of `bytes` the charmap reaches from `text`, at each size,
    /// offset, transform and mode `rasterize` can ask for. Answers how many
    /// images were compared.
    fn compare_with_swash(bytes: &[u8], text: &str) -> usize {
        let font = swash::FontRef::from_index(bytes, 0).expect("a font");
        let charmap = font.charmap();
        let mut context = swash::scale::ScaleContext::new();
        let mut outline = swash::scale::outline::Outline::new();
        let mut scratch = swash::zeno::Scratch::new();
        let skew = Transform::skew(
            Angle::from_degrees(OBLIQUE_DEGREES),
            Angle::from_degrees(0.0),
        );
        let quarter_turn = Transform::new(0.0, -1.0, 1.0, 0.0, 0.0, 0.0);
        let modes = [
            (GlyphRenderMode::Mask, Format::Alpha),
            (GlyphRenderMode::SubpixelRgb, Format::Subpixel),
            (
                GlyphRenderMode::Stroke {
                    width_q: 6,
                    join: StrokeJoin::Round,
                },
                Format::Alpha,
            ),
            (
                GlyphRenderMode::Stroke {
                    width_q: 9,
                    join: StrokeJoin::Miter,
                },
                Format::Alpha,
            ),
        ];
        let mut compared = 0;
        for character in text.chars() {
            let glyph_id = charmap.map(character);
            for size in [9.0, 14.0, 31.5, 96.0] {
                for offset in [Vector::new(0.0, 0.0), Vector::new(0.25, 0.75)] {
                    for transform in [None, Some(skew), Some(skew.then(&quarter_turn))] {
                        for (mode, format) in modes {
                            let how = Outlining {
                                format,
                                offset,
                                transform,
                                mode,
                            };
                            let mut scaler = context.builder(font).size(size).hint(true).build();
                            let expected = reference(&mut scaler, glyph_id, &how);
                            let actual = drawn(
                                &mut scaler,
                                &font,
                                glyph_id,
                                &how,
                                &mut outline,
                                &mut scratch,
                            );
                            let label =
                                format!("{character:?} at {size} px, {mode:?}, {transform:?}");
                            match (expected, actual) {
                                (None, None) => {}
                                (Some(expected), Some(actual)) => {
                                    let placed = |image: &swash::scale::image::Image| {
                                        let at = image.placement;
                                        (at.left, at.top, at.width, at.height)
                                    };
                                    assert_eq!(placed(&actual), placed(&expected), "{label}");
                                    assert!(actual.content == expected.content, "{label}");
                                    assert!(actual.data == expected.data, "pixels of {label}");
                                    compared += 1;
                                }
                                (expected, actual) => panic!(
                                    "{label}: swash drew {}, this drew {}",
                                    expected.is_some(),
                                    actual.is_some()
                                ),
                            }
                        }
                    }
                }
            }
        }
        compared
    }

    #[test]
    fn outlines_are_drawn_as_swash_renders_them() {
        assert!(compare_with_swash(AXES, "AbgQ@&") > 0);
        assert!(compare_with_swash(KOREAN, "\u{d55c}\u{ae00}A8") > 0);
        assert!(compare_with_swash(EMOJI, "\u{1f600}\u{2764}") > 0);
    }

    #[test]
    fn colour_layers_are_drawn_as_swash_renders_them() {
        let font = swash::FontRef::from_index(COLOR, 0).expect("a font");
        let mut context = swash::scale::ScaleContext::new();
        let scaler = context.builder(font).size(32.0).build();
        assert!(scaler.has_color_outlines(), "the fixture is a COLR face");
        let covered: String = (0x20u32..0x250)
            .chain(0x1f300..0x1f700)
            .filter_map(char::from_u32)
            .filter(|character| font.charmap().map(*character) != 0)
            .collect();
        assert!(!covered.is_empty());
        assert!(compare_with_swash(COLOR, &covered) > 0);
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
