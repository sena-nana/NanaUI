//! Glyph rasterization off the faces this crate's font layer issues.
//!
//! The renderer that draws `nana-text` layouts and any host that draws them
//! itself share this layer, so there is exactly one font instance resolution
//! in the process: the coordinates a run was *shaped* at are the ones it is
//! *scaled* at. [`TextLayout::glyph_placements`](crate::TextLayout::glyph_placements)
//! says where each glyph goes; [`OutlineRasterizer`] turns a
//! [`GlyphRasterKey`] into coverage or a colour bitmap on the CPU. Nothing
//! here names a GPU: uploading, atlasing and compositing belong to the caller.
//!
//! The split that matters is which facts are in the raster key and which are
//! not. Text color, opacity and the transform a glyph is drawn under change
//! where it lands and how it is tinted, never the bitmap, so they stay out of
//! the key — otherwise one label in two colors would rasterize twice.

mod outline;
mod paragraph;

pub use outline::OutlineRasterizer;
pub use paragraph::{RasterizedGlyph, rasterize_layout};

/// How a stroked glyph's outline turns its corners.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum StrokeJoin {
    #[default]
    Round,
    Miter,
    Bevel,
}

/// An OpenType glyph index. `u32` rather than `u16`: this is a cross-layer id
/// and widening it later would be an ABI break, the same call
/// [`crate::ShapedGlyph`] made.
pub type GlyphId = u32;

/// A face as the renderer names it.
///
/// Issued by the glyph resolver's font source (see
/// [`OutlineRasterizer::intern_instance`]), dense and `Copy`. It is
/// deliberately **not** a shaping backend's face id: #99 replaces the source
/// that issues it without touching the raster cache or the atlas.
///
/// Only meaningful together with the font generation it was issued under.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphFontId(pub u32);

/// The axis coordinates a face was instantiated at, as one id.
///
/// Zero is the face's own default instance. Two runs that differ only here are
/// different outlines, so this is part of the raster key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphVariationId(pub u64);

/// Synthesis, hinting and orientation the rasterizer applies, as opposed to
/// what the face already carries. Part of the raster key: each flag changes
/// the bitmap.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct GlyphSynthesis(pub u8);

impl GlyphSynthesis {
    pub const NONE: Self = Self(0);
    /// Skew an upright face to stand in for a missing italic.
    pub const FAKE_ITALIC: Self = Self(1 << 0);
    pub const DISABLE_HINTING: Self = Self(1 << 1);
    /// Snap the subpixel offset: a bitmap face must land on whole pixels.
    pub const PIXEL_FONT: Self = Self(1 << 2);
    /// Embolden an upright face to stand in for a weight it cannot reach.
    /// Reported by the font layer when the requested weight is out of the
    /// face's range and no other face covers it.
    pub const FAKE_BOLD: Self = Self(1 << 3);
    /// Turn the outline 90° clockwise: a sideways run of a vertical line
    /// (#59), whose glyphs lie on their side with their tops facing the line's
    /// block end. Not synthesis in the font sense, but the same kind of fact —
    /// a transform the rasterizer applies that changes the bitmap — so it
    /// rides in the same key field.
    pub const ROTATE_CW: Self = Self(1 << 4);
    /// Rasterize from the outline with the renderer's own scaler, never a
    /// platform rasterizer. A glyph that is also stroked or blurred is: its
    /// outline and its shadow come from the same scaler, and a fill drawn by
    /// another one would not sit exactly inside them.
    pub const OUTLINE_RASTER: Self = Self(1 << 5);

    pub const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }

    #[must_use]
    pub const fn with(self, flag: Self) -> Self {
        Self(self.0 | flag.0)
    }
}

/// What a run asks the rasterizer to produce.
///
/// The *request*, not the result: a color face answers a [`Mask`](Self::Mask)
/// request with a color bitmap, and [`GlyphImage::format`] is
/// what actually came back.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GlyphRenderMode {
    /// Grayscale coverage, with color faces passed through as color bitmaps.
    Mask,
    /// Per-subpixel (LCD / ClearType) coverage for a panel whose subpixels
    /// run red, green, blue left to right. Color faces still answer with
    /// color bitmaps.
    SubpixelRgb,
    /// The same for a panel whose subpixels run blue, green, red.
    SubpixelBgr,
    /// Grayscale coverage of the glyph's outline stroked `width_q / 4` raster
    /// px wide, centred on it. Outline glyphs only.
    Stroke { width_q: u16, join: StrokeJoin },
    /// Grayscale coverage grown by `spread_q / 4` raster px, then blurred by
    /// a Gaussian of CSS blur radius `radius_q / 4` raster px (twice its
    /// standard deviation). What a text shadow layer samples.
    Blur { radius_q: u16, spread_q: u16 },
}

impl GlyphRenderMode {
    /// Quarter raster px, the bucket the stroke and blur keys are cut in.
    pub fn quarters(px: f32) -> u16 {
        (px.max(0.0) * 4.0).round().min(f32::from(u16::MAX)) as u16
    }

    /// Whether this mode is coverage the renderer derives from the outline
    /// itself (a stroke, a blur), rather than the glyph as the face draws it.
    pub fn is_derived(self) -> bool {
        matches!(self, Self::Stroke { .. } | Self::Blur { .. })
    }
}

/// Fractional pen placement, quantized to quarter pixels.
///
/// Unquantized placement would make the raster key unbounded: a label sliding
/// under an animation would rasterize a new bitmap every frame. Quarters are
/// the same bucketing the reference path used, so glyph positioning does not
/// shift under this rewrite.
///
/// [`Self::split`] is cosmic-text's `SubpixelBin::new` — the thresholds, the
/// branch order and the carry into the whole pixel are all kept, because what
/// makes the migration invisible is that a glyph lands in the same bin it
/// used to. Derived from cosmic-text, Copyright (c) 2022 System76, MIT OR
/// Apache-2.0 (<https://github.com/pop-os/cosmic-text>), the same licence pair
/// NanaUI is under. See `docs/reference/third-party.md`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct SubpixelBin(u8);

impl SubpixelBin {
    /// Split a physical-pixel coordinate into its whole pixel and its bin.
    pub fn split(position: f32) -> (i32, Self) {
        let trunc = position as i32;
        let fract = position - trunc as f32;
        if position.is_sign_negative() {
            if fract > -0.125 {
                (trunc, Self(0))
            } else if fract > -0.375 {
                (trunc - 1, Self(3))
            } else if fract > -0.625 {
                (trunc - 1, Self(2))
            } else if fract > -0.875 {
                (trunc - 1, Self(1))
            } else {
                (trunc - 1, Self(0))
            }
        } else if fract < 0.125 {
            (trunc, Self(0))
        } else if fract < 0.375 {
            (trunc, Self(1))
        } else if fract < 0.625 {
            (trunc, Self(2))
        } else if fract < 0.875 {
            (trunc, Self(3))
        } else {
            (trunc + 1, Self(0))
        }
    }

    /// The pen offset this bin rasterizes at, in pixels.
    pub fn as_float(self) -> f32 {
        f32::from(self.0) * 0.25
    }
}

/// The raster size is the shaped size, to the bit.
///
/// Bucketing it was tried and rejected: at 1/64 px a 15.6px heading shifted to
/// 15.59375 and its antialiased edges moved by up to 11/255, which is a
/// rendering change this phase has no reason to make. What bounds the key
/// space is the cache's byte budget and its LRU, not a coarser key — and the
/// shaped paragraphs above already key on the same exact size, so a scale
/// animation was never going to reuse a bitmap anyway.
pub fn size_bits(size_px: f32) -> u32 {
    size_px.max(0.0).to_bits()
}

pub fn size_from_bits(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// Everything that decides a glyph's bitmap, and nothing that does not.
///
/// The font generation is part of the key rather than a reason to flush: a
/// `@font-face` registration reissues face ids, and an entry keyed by the old
/// generation can never be handed to the new one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphRasterKey {
    pub font: GlyphFontId,
    pub font_generation: u32,
    pub variation: GlyphVariationId,
    pub glyph: GlyphId,
    /// Raster size, see [`size_bits`].
    pub size_bits: u32,
    pub subpixel_x: SubpixelBin,
    pub subpixel_y: SubpixelBin,
    pub synthesis: GlyphSynthesis,
    pub mode: GlyphRenderMode,
}

/// How a rasterized glyph's bytes are laid out.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GlyphImageFormat {
    /// One byte of coverage per pixel.
    Mask,
    /// Four bytes per pixel, straight (non-premultiplied) sRGB.
    ColorRgba,
    /// Four bytes per pixel: the coverage of the pixel's red, green and blue
    /// subpixels in screen order, then an unused byte.
    ///
    /// Stored sRGB-*encoded*. These share the color pages, whose format
    /// decodes on sampling, so encoding them here is what makes the shader
    /// read back the coverage the rasterizer produced.
    SubpixelRgb,
}

impl GlyphImageFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Mask => 1,
            Self::ColorRgba | Self::SubpixelRgb => 4,
        }
    }
}

/// A rasterized glyph, in the rasterizer's own terms.
///
/// `left` / `top` are the bitmap's offset from the pen position, the sign
/// convention every outline rasterizer uses: the quad sits at
/// `(pen.x + left, pen.y - top)`.
#[derive(Debug)]
pub struct GlyphImage {
    pub format: GlyphImageFormat,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub data: Vec<u8>,
}

impl GlyphImage {
    pub fn byte_len(&self) -> usize {
        self.data.len()
    }

    /// Whether this glyph covers no pixels — a space, or an outline that
    /// scaled away. Cached like any other answer so it is asked for once.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Above this CSS blur radius (raster px) a shadow is blurred at half
/// resolution and scaled back up: the kernel's cost grows with the radius,
/// and a wide blur has no detail a half-resolution pass would lose.
const HALF_RES_BLUR_RADIUS: f32 = 8.0;

/// The widest CSS blur radius a shadow is rasterized with, raster px. The
/// painter caps the logical radius before it reaches the key; this bounds a
/// bitmap whatever the scale.
pub const MAX_BLUR_RADIUS: f32 = 96.0;

/// Distinct axis-coordinate sets kept before the least recently used go. An
/// animated font axis (Issue #85) is a new set on every sampled frame.
pub const VARIATION_CAP: usize = 4096;
/// `image`'s coverage under a Gaussian of CSS blur radius `radius` (twice its
/// standard deviation), grown by the kernel's reach on every side so no
/// coverage is cut off.
pub fn blur_image(image: GlyphImage, radius: f32) -> GlyphImage {
    let radius = radius.clamp(0.0, MAX_BLUR_RADIUS);
    if image.is_empty() || radius < 0.5 {
        return image;
    }
    let (data, width, height, pad) = blur_mask(&image.data, image.width, image.height, radius);
    GlyphImage {
        format: GlyphImageFormat::Mask,
        width,
        height,
        left: image.left - pad as i32,
        top: image.top + pad as i32,
        data,
    }
}

/// The separable Gaussian blur behind [`blur_image`], over one byte of
/// coverage a pixel. Returns the blurred mask, its size and how many pixels
/// it grew by on each side.
///
/// Above [`HALF_RES_BLUR_RADIUS`] the work is done at half resolution: the
/// padded mask is box-filtered down, blurred with half the deviation, and
/// scaled back up bilinearly. The kernel is normalized and nothing is cropped,
/// so the total coverage — how dark the shadow is overall — is what the glyph
/// had, up to rounding.
pub fn blur_mask(src: &[u8], width: u32, height: u32, radius: f32) -> (Vec<u8>, u32, u32, u32) {
    let sigma = radius * 0.5;
    let pad = (sigma * 3.0).ceil() as u32 + 1;
    let (w, h) = ((width + pad * 2) as usize, (height + pad * 2) as usize);
    let mut plane = vec![0f32; w * h];
    for y in 0..height as usize {
        for x in 0..width as usize {
            plane[(y + pad as usize) * w + x + pad as usize] =
                f32::from(src[y * width as usize + x]);
        }
    }
    let blurred = if radius > HALF_RES_BLUR_RADIUS {
        let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
        let mut half = vec![0f32; hw * hh];
        for y in 0..h {
            for x in 0..w {
                half[(y / 2) * hw + x / 2] += plane[y * w + x] * 0.25;
            }
        }
        let half = gaussian(&half, hw, hh, sigma * 0.5);
        // Bilinear back up, sampling the half-resolution grid at each full
        // pixel's centre.
        let mut full = vec![0f32; w * h];
        for y in 0..h {
            let fy = ((y as f32 + 0.5) * 0.5 - 0.5).max(0.0);
            let y0 = (fy as usize).min(hh - 1);
            let y1 = (y0 + 1).min(hh - 1);
            let ty = fy - y0 as f32;
            for x in 0..w {
                let fx = ((x as f32 + 0.5) * 0.5 - 0.5).max(0.0);
                let x0 = (fx as usize).min(hw - 1);
                let x1 = (x0 + 1).min(hw - 1);
                let tx = fx - x0 as f32;
                let top = half[y0 * hw + x0] * (1.0 - tx) + half[y0 * hw + x1] * tx;
                let bottom = half[y1 * hw + x0] * (1.0 - tx) + half[y1 * hw + x1] * tx;
                full[y * w + x] = top * (1.0 - ty) + bottom * ty;
            }
        }
        full
    } else {
        gaussian(&plane, w, h, sigma)
    };
    let data = blurred
        .iter()
        .map(|value| value.round().clamp(0.0, 255.0) as u8)
        .collect();
    (data, w as u32, h as u32, pad)
}

/// A normalized separable Gaussian of deviation `sigma` over a `w` × `h`
/// plane, zero outside it.
fn gaussian(plane: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma <= 0.0 {
        return plane.to_vec();
    }
    let reach = (sigma * 3.0).ceil() as isize;
    let mut kernel: Vec<f32> = (-reach..=reach)
        .map(|offset| (-(offset * offset) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = kernel.iter().sum();
    kernel.iter_mut().for_each(|weight| *weight /= sum);
    let mut across = vec![0f32; w * h];
    for y in 0..h {
        let row = &plane[y * w..(y + 1) * w];
        for x in 0..w {
            let mut value = 0.0;
            for (index, weight) in kernel.iter().enumerate() {
                let at = x as isize + index as isize - reach;
                if at >= 0 && (at as usize) < w {
                    value += row[at as usize] * weight;
                }
            }
            across[y * w + x] = value;
        }
    }
    let mut out = vec![0f32; w * h];
    for x in 0..w {
        for y in 0..h {
            let mut value = 0.0;
            for (index, weight) in kernel.iter().enumerate() {
                let at = y as isize + index as isize - reach;
                if at >= 0 && (at as usize) < h {
                    value += across[at as usize * w + x] * weight;
                }
            }
            out[y * w + x] = value;
        }
    }
    out
}

/// Write one pixel of [`GlyphImageFormat::SubpixelRgb`] from its subpixel
/// coverages in red, green, blue order: reordered for a BGR panel and
/// sRGB-encoded.
pub fn encode_subpixel(pixel: &mut [u8; 4], rgb: [u8; 3], mode: GlyphRenderMode) {
    let [r, g, b] = match mode {
        GlyphRenderMode::SubpixelBgr => [rgb[2], rgb[1], rgb[0]],
        _ => rgb,
    };
    *pixel = [r, g, b, 0].map(|value| SRGB_ENCODE[usize::from(value)]);
}

/// Linear 8-bit coverage to the sRGB-encoded byte that decodes back to it.
static SRGB_ENCODE: std::sync::LazyLock<[u8; 256]> = std::sync::LazyLock::new(|| {
    std::array::from_fn(|value| {
        let c = value as f32 / 255.0;
        let encoded = if c <= 0.003_130_8 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round() as u8
    })
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subpixel_bins_round_to_quarters_and_carry_into_the_whole_pixel() {
        assert_eq!(SubpixelBin::split(4.0), (4, SubpixelBin(0)));
        assert_eq!(SubpixelBin::split(4.3), (4, SubpixelBin(1)));
        assert_eq!(SubpixelBin::split(4.5), (4, SubpixelBin(2)));
        assert_eq!(SubpixelBin::split(4.95), (5, SubpixelBin(0)));
        assert_eq!(SubpixelBin::split(-0.5), (-1, SubpixelBin(2)));
        assert_eq!(SubpixelBin(3).as_float(), 0.75);
    }

    #[test]
    fn a_fractional_dpi_size_survives_the_raster_size_bucket_exactly() {
        for size in [13.0f32, 16.0, 20.0] {
            for scale in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
                let physical = size * scale;
                assert_eq!(
                    size_from_bits(size_bits(physical)),
                    physical,
                    "{size} at {scale} must not drift through the raster bucket"
                );
            }
        }
    }
}
