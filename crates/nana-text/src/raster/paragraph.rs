//! A whole paragraph's glyphs as bitmaps, for a host that composites text
//! itself rather than through the renderer's atlas.

use crate::TextLayout;
use crate::layout::GlyphSource;

use super::{
    GlyphImage, GlyphRasterKey, GlyphRenderMode, GlyphSynthesis, OutlineRasterizer, SubpixelBin,
    size_bits,
};

/// One glyph of a paragraph, rasterized and placed.
#[derive(Debug)]
pub struct RasterizedGlyph {
    /// Where the bitmap's top-left pixel goes, in raster px.
    pub at: [i32; 2],
    pub image: GlyphImage,
    /// See [`GlyphPlacement::cluster`](crate::GlyphPlacement::cluster).
    pub cluster: u32,
    pub source: GlyphSource,
}

/// Every glyph of `layout` that covers a pixel, in paint order, rasterized
/// in `mode` at `scale` raster px per layout px with the paragraph's origin
/// at `origin`. The rasterizer must belong to the engine `layout` was laid
/// out with, or its faces are not the ones the runs name.
///
/// Each glyph is rasterized afresh: a caller that redraws often keeps its
/// own cache over [`OutlineRasterizer::rasterize`] instead.
#[must_use]
pub fn rasterize_layout(
    rasterizer: &mut OutlineRasterizer,
    layout: &TextLayout,
    scale: f32,
    origin: [f32; 2],
    mode: GlyphRenderMode,
) -> Vec<RasterizedGlyph> {
    let generation = layout.font_generation.get() as u32;
    layout
        .glyph_placements(scale, origin)
        .into_iter()
        .filter_map(|placed| {
            let (font, variation, mut synthesis) = rasterizer.intern_instance(placed.instance);
            if placed.sideways {
                synthesis = synthesis.with(GlyphSynthesis::ROTATE_CW);
            }
            let (x, subpixel_x) = SubpixelBin::split(placed.position[0]);
            let (y, subpixel_y) = SubpixelBin::split(placed.position[1]);
            let image = rasterizer.rasterize(&GlyphRasterKey {
                font,
                font_generation: generation,
                variation,
                glyph: placed.glyph_id,
                size_bits: size_bits(placed.font_size_px),
                subpixel_x,
                subpixel_y,
                synthesis,
                mode,
            })?;
            (!image.is_empty()).then(|| RasterizedGlyph {
                at: [x + image.left, y - image.top],
                image,
                cluster: placed.cluster,
                source: placed.source,
            })
        })
        .collect()
}
