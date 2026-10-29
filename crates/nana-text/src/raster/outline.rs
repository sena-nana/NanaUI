//! [`OutlineRasterizer`]: `swash` behind the raster boundary. The only file
//! of this crate that names it (`scripts/check-engine-boundary.py`).

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
    faces: Vec<crate::FontId>,
    index: HashMap<crate::FontId, GlyphFontId>,
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
            faces: Vec::new(),
            index: HashMap::new(),
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
        cold.sort_unstable_by_key(|(last_used, id)| (*last_used, id.0));
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
                self.faces.push(face);
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
        let face = *self.faces.get(font.0 as usize)?;
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
        self.faces.len()
    }
}

impl OutlineRasterizer {
    /// Rasterize one glyph, or `None` when the face is unknown or cannot
    /// draw it. A glyph that is simply blank answers `Some` with an empty
    /// image, so a cache can tell "nothing to draw" from "ask again".
    pub fn rasterize(&mut self, key: &GlyphRasterKey) -> Option<GlyphImage> {
        let face = *self.faces.get(key.font.0 as usize)?;
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
        // A color outline with the first palette, then a color strike, then
        // the plain outline. Order matters: an emoji face can have all three,
        // and the first is the one with the palette applied.
        const FACE_SOURCES: &[swash::scale::Source] = &[
            swash::scale::Source::ColorOutline(0),
            swash::scale::Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
            swash::scale::Source::Outline,
        ];
        // A stroke or a shadow is coverage of the outline itself, whatever
        // colours the face would paint it in.
        const OUTLINE_ONLY: &[swash::scale::Source] = &[swash::scale::Source::Outline];
        let mut render = swash::scale::Render::new(if key.mode.is_derived() {
            OUTLINE_ONLY
        } else {
            FACE_SOURCES
        });
        let format = match key.mode {
            GlyphRenderMode::SubpixelRgb | GlyphRenderMode::SubpixelBgr => {
                swash::zeno::Format::Subpixel
            }
            GlyphRenderMode::Mask
            | GlyphRenderMode::Stroke { .. }
            | GlyphRenderMode::Blur { .. } => swash::zeno::Format::Alpha,
        };
        let offset = subpixel_offset(key, snap_to_pixel);
        render.format(format).offset(offset);
        if let GlyphRenderMode::Stroke { width_q, join } = key.mode {
            let mut stroke = swash::zeno::Stroke::new(f32::from(width_q) * 0.25);
            stroke.join(zeno_join(join));
            render.style(stroke);
        }
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
        render.transform(transform);
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
        let image = if embolden > 0.0 && plain && scaler.has_outlines() {
            emboldened(
                &mut scaler,
                glyph_id,
                embolden,
                &Outlining {
                    format,
                    offset,
                    transform,
                    mode: key.mode,
                },
            )?
        } else {
            render.render(&mut scaler, glyph_id)?
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

/// How [`emboldened`] draws the outline: what `swash::scale::Render` was
/// configured with.
struct Outlining {
    format: swash::zeno::Format,
    offset: swash::zeno::Vector,
    transform: Option<swash::zeno::Transform>,
    mode: GlyphRenderMode,
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
fn subpixel_offset(key: &GlyphRasterKey, snap: bool) -> swash::zeno::Vector {
    let x = key.subpixel_x.as_float();
    let y = key.subpixel_y.as_float();
    if snap {
        swash::zeno::Vector::new(x.round(), y.round())
    } else {
        swash::zeno::Vector::new(x, y)
    }
}
