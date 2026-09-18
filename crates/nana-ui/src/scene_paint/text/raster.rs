//! The renderer's own rasterizer boundary.
//!
//! [`GlyphRasterizer`] is the abstraction `NanaRenderer::text` owns; what sits
//! under it is an implementation detail it may replace. The first
//! implementation scales outlines with `swash`, reached through the shaping
//! backend's scaler so there is exactly one font instance resolution in the
//! process — #99 swaps that for `nana-text`'s font layer by writing another
//! `impl GlyphRasterizer`, with no change above this file.
//!
//! The boundary is deliberately wider than today's needs: a request names a
//! [`GlyphRenderMode`] and an answer names its own [`GlyphImageFormat`], so an
//! LCD, SDF or vector backend is a new arm rather than a new pipeline.

use std::collections::HashMap;

use cosmic_text::{CacheKey, CacheKeyFlags, SwashCache, SwashContent, fontdb};

use super::glyph::{GlyphFontId, GlyphRasterKey, GlyphRenderMode, GlyphSynthesis, size_from_bits};

/// How a rasterized glyph's bytes are laid out.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum GlyphImageFormat {
    /// One byte of coverage per pixel.
    Mask,
    /// Four bytes per pixel, straight (non-premultiplied) sRGB.
    ColorRgba,
}

impl GlyphImageFormat {
    pub(super) const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Mask => 1,
            Self::ColorRgba => 4,
        }
    }
}

/// A rasterized glyph, in the rasterizer's own terms.
///
/// `left` / `top` are the bitmap's offset from the pen position, the sign
/// convention every outline rasterizer uses: the quad sits at
/// `(pen.x + left, pen.y - top)`.
#[derive(Debug)]
pub(super) struct GlyphImage {
    pub format: GlyphImageFormat,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub data: Vec<u8>,
}

impl GlyphImage {
    pub(super) fn byte_len(&self) -> usize {
        self.data.len()
    }

    /// Whether this glyph covers no pixels — a space, or an outline that
    /// scaled away. Cached like any other answer so it is asked for once.
    pub(super) fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

pub(super) struct GlyphRasterRequest {
    pub key: GlyphRasterKey,
}

/// What `NanaRenderer::text` requires of a glyph raster backend.
pub(super) trait GlyphRasterizer {
    /// Rasterize one glyph, or `None` when the backend cannot produce it at
    /// all (an unknown face, a mode it does not implement). A glyph that is
    /// simply blank answers `Some` with an empty image, so the cache can tell
    /// "nothing to draw" from "ask again".
    fn rasterize(&mut self, request: &GlyphRasterRequest) -> Option<GlyphImage>;
}

/// The face identity the cosmic-text-backed source interns behind a
/// [`GlyphFontId`]. Variation coordinates are *not* here: they vary per run
/// over the same face and travel as [`GlyphVariationId`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct CosmicFace {
    id: fontdb::ID,
    weight: fontdb::Weight,
}

/// `swash` behind the renderer's rasterizer boundary.
///
/// Holds the face table that issues [`GlyphFontId`]s, so the ids the glyph IR
/// carries and the faces this rasterizer can scale cannot drift apart.
pub(super) struct SwashGlyphRasterizer {
    /// Shared with Runtime shaping; see [`crate::nana_text::nana_font_system`].
    fonts: crate::nana_text::SharedFontSystem,
    /// Used only for its scaler: every cached answer lives in
    /// [`super::raster_cache::GlyphRasterCache`], which is the renderer's.
    scaler: SwashCache,
    faces: Vec<CosmicFace>,
    index: HashMap<CosmicFace, GlyphFontId>,
    /// The face asked for last. Runs are contiguous by face, so a paragraph
    /// asks for the same one for every glyph and this keeps the hash lookup
    /// off the per-glyph path.
    recent: Option<(CosmicFace, GlyphFontId)>,
}

impl SwashGlyphRasterizer {
    pub(super) fn new(fonts: crate::nana_text::SharedFontSystem) -> Self {
        Self {
            fonts,
            scaler: SwashCache::new(),
            faces: Vec::new(),
            index: HashMap::new(),
            recent: None,
        }
    }

    /// The renderer's id for one backend face, minting it on first sight.
    ///
    /// Face ids are never reused by the backend, so an id stays valid across
    /// font-set generations; the generation rides in the raster key instead,
    /// where it invalidates bitmaps without invalidating identity.
    pub(super) fn intern(&mut self, id: fontdb::ID, weight: fontdb::Weight) -> GlyphFontId {
        let face = CosmicFace { id, weight };
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

    #[cfg(test)]
    pub(super) fn face_count(&self) -> usize {
        self.faces.len()
    }
}

impl GlyphRasterizer for SwashGlyphRasterizer {
    fn rasterize(&mut self, request: &GlyphRasterRequest) -> Option<GlyphImage> {
        let key = &request.key;
        let face = *self.faces.get(key.font.0 as usize)?;
        let glyph_id = u16::try_from(key.glyph).ok()?;
        let cache_key = CacheKey {
            font_id: face.id,
            glyph_id,
            font_size_bits: size_from_bits(key.size_bits).to_bits(),
            x_bin: subpixel(key.subpixel_x.quarters()),
            y_bin: subpixel(key.subpixel_y.quarters()),
            font_weight: face.weight,
            font_variation_hash: key.variation.0,
            flags: flags(key.synthesis),
        };
        let GlyphRenderMode::Mask = key.mode;
        let image = {
            let mut fonts = crate::nana_text::lock_font_system(&self.fonts);
            self.scaler.get_image_uncached(&mut fonts, cache_key)?
        };
        let width = image.placement.width;
        let height = image.placement.height;
        let pixels = (width as usize).saturating_mul(height as usize);
        let format = match image.content {
            SwashContent::Mask => GlyphImageFormat::Mask,
            // A subpixel mask is three coverages plus padding, i.e. the same
            // four bytes per pixel a color bitmap has. Treating it as a mask
            // would read a quarter of it; the backend does not emit one under
            // the alpha format requested above, and this keeps the byte count
            // honest if it ever does.
            SwashContent::Color | SwashContent::SubpixelMask => GlyphImageFormat::ColorRgba,
        };
        let bytes = pixels * format.bytes_per_pixel();
        if image.data.len() < bytes {
            return None;
        }
        Some(GlyphImage {
            format,
            width,
            height,
            left: image.placement.left,
            top: image.placement.top,
            data: image.data[..bytes].to_vec(),
        })
    }
}

fn subpixel(quarters: u8) -> cosmic_text::SubpixelBin {
    match quarters {
        1 => cosmic_text::SubpixelBin::One,
        2 => cosmic_text::SubpixelBin::Two,
        3 => cosmic_text::SubpixelBin::Three,
        _ => cosmic_text::SubpixelBin::Zero,
    }
}

/// Mapped arm by arm rather than by bit value: the two flag sets happen to
/// agree today, and a silent `from_bits` would turn a future divergence into
/// wrongly hinted glyphs instead of a compile error.
fn flags(synthesis: GlyphSynthesis) -> CacheKeyFlags {
    let mut flags = CacheKeyFlags::empty();
    if synthesis.contains(GlyphSynthesis::FAKE_ITALIC) {
        flags |= CacheKeyFlags::FAKE_ITALIC;
    }
    if synthesis.contains(GlyphSynthesis::DISABLE_HINTING) {
        flags |= CacheKeyFlags::DISABLE_HINTING;
    }
    if synthesis.contains(GlyphSynthesis::PIXEL_FONT) {
        flags |= CacheKeyFlags::PIXEL_FONT;
    }
    flags
}

/// The renderer's synthesis flags for one shaped glyph's backend flags.
pub(super) fn synthesis_from_backend(backend: CacheKeyFlags) -> GlyphSynthesis {
    let mut synthesis = GlyphSynthesis::NONE;
    if backend.contains(CacheKeyFlags::FAKE_ITALIC) {
        synthesis = synthesis.with(GlyphSynthesis::FAKE_ITALIC);
    }
    if backend.contains(CacheKeyFlags::DISABLE_HINTING) {
        synthesis = synthesis.with(GlyphSynthesis::DISABLE_HINTING);
    }
    if backend.contains(CacheKeyFlags::PIXEL_FONT) {
        synthesis = synthesis.with(GlyphSynthesis::PIXEL_FONT);
    }
    synthesis
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesis_round_trips_through_the_backend_flag_set() {
        let all = GlyphSynthesis::NONE
            .with(GlyphSynthesis::FAKE_ITALIC)
            .with(GlyphSynthesis::DISABLE_HINTING)
            .with(GlyphSynthesis::PIXEL_FONT);
        assert_eq!(synthesis_from_backend(flags(all)), all);
        assert_eq!(
            synthesis_from_backend(flags(GlyphSynthesis::NONE)),
            GlyphSynthesis::NONE
        );
    }

    #[test]
    fn interning_the_same_face_twice_issues_one_id() {
        let mut rasterizer = SwashGlyphRasterizer::new(crate::nana_text::nana_font_system());
        let db = fontdb::Database::new();
        let _ = db;
        let weight = fontdb::Weight::NORMAL;
        let ids: Vec<_> = {
            let fonts = crate::nana_text::lock_font_system(&rasterizer.fonts);
            fonts.db().faces().take(2).map(|face| face.id).collect()
        };
        if ids.is_empty() {
            return;
        }
        let first = rasterizer.intern(ids[0], weight);
        assert_eq!(rasterizer.intern(ids[0], weight), first);
        assert_eq!(rasterizer.face_count(), 1);
        assert_ne!(
            rasterizer.intern(ids[0], fontdb::Weight::BOLD),
            first,
            "a different synthetic weight is a different face instance"
        );
    }
}
