//! Renderer-facing glyph IR: what `NanaRenderer::text` draws.
//!
//! Nothing here names a shaping backend. A resolver turns a
//! `nana_text::TextLayout` into [`NanaGlyphRun`]s over a flat [`PlacedGlyph`]
//! arena, and every stage below this module only ever sees that IR plus
//! [`GlyphRasterKey`]. The key and its parts are `nana_text::raster`'s, so the
//! renderer and a host that rasterizes text itself agree on what a bitmap is.
//!
//! The split that matters is which facts are in the raster key and which are
//! not. Text color, node opacity and the scene transform change where a glyph
//! lands and how it is tinted, never the bitmap, so they stay on the run and
//! out of the key — otherwise one label in two colors would rasterize twice.

use std::ops::Range;

#[cfg(windows)]
pub(super) use nana_text::raster::size_from_bits;
pub(super) use nana_text::raster::{
    GlyphFontId, GlyphId, GlyphRasterKey, GlyphRenderMode, GlyphSynthesis, GlyphVariationId,
    SubpixelBin, size_bits,
};

/// What an instance is to the paragraph it belongs to. Kept on the run and
/// in the instance, so the presentation layer can tell a shadow from the
/// glyph it shadows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(super) enum GlyphRole {
    #[default]
    Fill,
    Shadow,
    Stroke,
}

/// One glyph's pen position in physical pixels, fraction included.
///
/// The fraction is not folded away here: [`GlyphRasterKey`] bins it and the
/// whole-pixel remainder is what the instance is placed at, so the two cannot
/// disagree about where the bitmap was rendered for.
#[derive(Clone, Copy, Debug)]
pub(super) struct PlacedGlyph {
    pub glyph: GlyphId,
    pub x: f32,
    pub y: f32,
    /// Byte offset of the glyph's cluster in the paragraph's text: what a
    /// per-range paint (a rich span's stroke, a shadow) is looked up by.
    pub cluster: u32,
}

/// A maximal span of glyphs sharing a face instance, a size and a color.
///
/// `glyphs` indexes the [`NanaGlyphBuffer`] arena rather than owning a `Vec`:
/// a paragraph is many short runs and a per-run allocation is the cost this IR
/// exists to avoid.
#[derive(Clone, Debug)]
pub(super) struct NanaGlyphRun {
    pub font: GlyphFontId,
    pub font_generation: u32,
    pub variation: GlyphVariationId,
    pub size_bits: u32,
    pub synthesis: GlyphSynthesis,
    pub render_mode: GlyphRenderMode,
    /// sRGB, opacity already folded in. Not in the raster key.
    pub color: [f32; 4],
    /// Authoring-space value, when this run came from an explicit CSS paint.
    pub paint_color: Option<nana_ui_core::PaintColor>,
    pub role: GlyphRole,
    pub glyphs: Range<u32>,
}

impl NanaGlyphRun {
    /// The raster key for one of this run's glyphs at `placed`.
    ///
    /// Returns the whole-pixel pen position alongside, because the two are
    /// derived from the same split and separating them invites drift.
    pub(super) fn raster_key(&self, placed: &PlacedGlyph) -> (GlyphRasterKey, [i32; 2]) {
        let (x, subpixel_x) = SubpixelBin::split(placed.x);
        let (y, subpixel_y) = SubpixelBin::split(placed.y);
        (
            GlyphRasterKey {
                font: self.font,
                font_generation: self.font_generation,
                variation: self.variation,
                glyph: placed.glyph,
                size_bits: self.size_bits,
                subpixel_x,
                subpixel_y,
                synthesis: self.synthesis,
                mode: self.render_mode,
            },
            [x, y],
        )
    }
}

/// One paragraph's runs and the arena their ranges index.
///
/// Reused across frames by [`clear`](Self::clear) rather than reallocated: a
/// text-heavy frame resolves tens of thousands of glyphs and this is the
/// buffer they land in.
#[derive(Default, Debug)]
pub(super) struct NanaGlyphBuffer {
    pub runs: Vec<NanaGlyphRun>,
    pub glyphs: Vec<PlacedGlyph>,
}

impl NanaGlyphBuffer {
    pub(super) fn clear(&mut self) {
        self.runs.clear();
        self.glyphs.clear();
    }

    pub(super) fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    pub(super) fn glyphs_of(&self, run: &NanaGlyphRun) -> &[PlacedGlyph] {
        let start = run.glyphs.start as usize;
        let end = (run.glyphs.end as usize).min(self.glyphs.len());
        if start >= end {
            return &[];
        }
        &self.glyphs[start..end]
    }

    /// Append `glyph` to the run matching `descriptor`, opening one when the
    /// last run is a different instance, size or color.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push(
        &mut self,
        font: GlyphFontId,
        font_generation: u32,
        variation: GlyphVariationId,
        size_bits: u32,
        synthesis: GlyphSynthesis,
        render_mode: GlyphRenderMode,
        color: [f32; 4],
        paint_color: Option<nana_ui_core::PaintColor>,
        role: GlyphRole,
        glyph: PlacedGlyph,
    ) {
        let index = self.glyphs.len() as u32;
        self.glyphs.push(glyph);
        if let Some(run) = self.runs.last_mut()
            && run.font == font
            && run.font_generation == font_generation
            && run.variation == variation
            && run.size_bits == size_bits
            && run.synthesis == synthesis
            && run.render_mode == render_mode
            && run.color == color
            && run.paint_color == paint_color
            && run.role == role
            && run.glyphs.end == index
        {
            run.glyphs.end = index + 1;
            return;
        }
        self.runs.push(NanaGlyphRun {
            font,
            font_generation,
            variation,
            size_bits,
            synthesis,
            render_mode,
            color,
            paint_color,
            role,
            glyphs: index..index + 1,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_leaves_the_raster_key_alone_but_opens_a_new_run() {
        let mut buffer = NanaGlyphBuffer::default();
        let glyph = PlacedGlyph {
            glyph: 7,
            x: 1.0,
            y: 2.0,
            cluster: 0,
        };
        for color in [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]] {
            buffer.push(
                GlyphFontId(1),
                3,
                GlyphVariationId(0),
                size_bits(16.0),
                GlyphSynthesis::NONE,
                GlyphRenderMode::Mask,
                color,
                None,
                GlyphRole::Fill,
                glyph,
            );
        }
        assert_eq!(buffer.runs.len(), 2, "a color change starts a run");
        let (first, _) = buffer.runs[0].raster_key(&glyph);
        let (second, _) = buffer.runs[1].raster_key(&glyph);
        assert_eq!(
            first, second,
            "the same glyph in two colors must share one bitmap"
        );
    }

    #[test]
    fn glyphs_of_a_run_are_the_ones_pushed_under_it() {
        let mut buffer = NanaGlyphBuffer::default();
        for index in 0..3u32 {
            buffer.push(
                GlyphFontId(1),
                0,
                GlyphVariationId(0),
                size_bits(16.0),
                GlyphSynthesis::NONE,
                GlyphRenderMode::Mask,
                [1.0; 4],
                None,
                GlyphRole::Fill,
                PlacedGlyph {
                    glyph: index,
                    x: index as f32,
                    y: 0.0,
                    cluster: index,
                },
            );
        }
        assert_eq!(buffer.runs.len(), 1);
        let glyphs = buffer.glyphs_of(&buffer.runs[0]);
        assert_eq!(glyphs.len(), 3);
        assert_eq!(glyphs[2].glyph, 2);
    }
}
