//! Where each glyph of a laid-out paragraph goes on the page.
//!
//! A layout holds runs in line-relative coordinates: an origin and an advance
//! per glyph. Every renderer needs the same walk over them — the line's
//! baseline, the pen moving along the run, the shaper's offsets turned to
//! page y — and the same snapping, or two of them would put a glyph a pixel
//! apart. [`TextLayout::glyph_placements`] is that walk, once.

use crate::font::FontInstanceKey;
use crate::shape::RunOrientation;

use super::TextLayout;

/// What part of the paragraph a glyph draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphSource {
    /// The text itself.
    Line,
    /// A ruby annotation above its base.
    Ruby,
    /// An emphasis mark beside its character.
    Emphasis,
    /// An object label, by its index in [`TextLayout::labels`].
    Label(usize),
}

/// One glyph, placed.
#[derive(Clone, Copy, Debug)]
pub struct GlyphPlacement<'a> {
    /// The face instance its run was shaped with.
    pub instance: &'a FontInstanceKey,
    /// The raster size, layout px times the scale.
    pub font_size_px: f32,
    pub glyph_id: u32,
    /// The pen position in raster px, origin added: `x` keeps its fraction
    /// for sub-pixel placement, `y` sits on a whole-pixel baseline.
    pub position: [f32; 2],
    /// Byte offset of the text it stands for: its own cluster on a line, the
    /// base's first byte for a ruby, the marked or labelled character.
    pub cluster: u32,
    pub source: GlyphSource,
    /// A sideways run of a vertical line: its glyphs lie on their side and
    /// rasterize turned a quarter clockwise.
    pub sideways: bool,
}

impl TextLayout {
    /// Every glyph that draws, in paint order: the lines, then ruby, emphasis
    /// marks and object labels. `scale` turns layout px into raster px and
    /// `origin` is where the paragraph's origin lands, in raster px.
    ///
    /// A run the font layer could not instantiate has no face to draw with
    /// and is skipped; its advances are still in the line, so nothing after
    /// it moves.
    #[must_use]
    pub fn glyph_placements(&self, scale: f32, origin: [f32; 2]) -> Vec<GlyphPlacement<'_>> {
        let mut out = Vec::new();
        if self.is_vertical() {
            self.vertical_lines(scale, origin, &mut out);
        } else {
            for line in &self.lines {
                // The baseline is snapped whole so it cannot land between
                // texels; the origin's sub-pixel phase stays fractional.
                let baseline = (line.metrics.baseline_y_px * scale).round();
                for run in self.line_runs(line) {
                    push_run(
                        &mut out,
                        run,
                        scale,
                        origin,
                        [run.origin_x_px, baseline],
                        None,
                        GlyphSource::Line,
                    );
                }
            }
        }
        for ruby in &self.rubies {
            let baseline = (ruby.baseline_y_px * scale).round();
            for run in &ruby.runs {
                push_run(
                    &mut out,
                    run,
                    scale,
                    origin,
                    [run.origin_x_px, baseline],
                    Some(ruby.range.start as u32),
                    GlyphSource::Ruby,
                );
            }
        }
        for mark in &self.emphasis {
            let Some(runs) = self.emphasis_marks.get(mark.mark as usize) else {
                continue;
            };
            let baseline = (mark.baseline_y_px * scale).round();
            for run in runs {
                push_run(
                    &mut out,
                    run,
                    scale,
                    origin,
                    [mark.x_px + run.origin_x_px, baseline],
                    Some(mark.offset as u32),
                    GlyphSource::Emphasis,
                );
            }
        }
        for (index, label) in self.labels.iter().enumerate() {
            let baseline = (label.baseline_y_px * scale).round();
            for run in &label.runs {
                push_run(
                    &mut out,
                    run,
                    scale,
                    origin,
                    [run.origin_x_px, baseline],
                    Some(label.offset as u32),
                    GlyphSource::Label(index),
                );
            }
        }
        out
    }

    fn vertical_lines<'a>(
        &'a self,
        scale: f32,
        origin: [f32; 2],
        out: &mut Vec<GlyphPlacement<'a>>,
    ) {
        let stack = self.physical_size().0;
        for line in &self.lines {
            let centre =
                (self.physical_x_of_block(line.metrics.baseline_y_px, stack) * scale).round();
            for run in self.line_runs(line) {
                let Some(instance) = run.instance.as_ref() else {
                    continue;
                };
                let sideways = run.orientation == RunOrientation::Sideways;
                // The sideways run's own baseline, placed so that its em box
                // — ascent to the right of it, descent to the left — is
                // centred on the column's centre line.
                let baseline =
                    centre - (run.metrics.ascent_px - run.metrics.descent_px) * 0.5 * scale;
                let mut pen = run.origin_x_px;
                for glyph in &run.glyphs {
                    let position = if sideways {
                        // Font y (up) is page x; font x is page y.
                        [
                            baseline + glyph.offset_y_px * scale + origin[0],
                            (pen + glyph.offset_x_px) * scale + origin[1],
                        ]
                    } else {
                        [
                            centre + glyph.offset_x_px * scale + origin[0],
                            (pen - glyph.offset_y_px) * scale + origin[1],
                        ]
                    };
                    pen += glyph.advance_px;
                    out.push(GlyphPlacement {
                        instance,
                        font_size_px: run.font_size_px * scale,
                        glyph_id: glyph.glyph_id,
                        position,
                        cluster: glyph.cluster,
                        source: GlyphSource::Line,
                        sideways,
                    });
                }
            }
        }
    }
}

/// One horizontal run's glyphs from `pen` (`[x layout px, baseline raster
/// px]`). `cluster` stands every glyph for one character; `None` keeps each
/// glyph's own.
fn push_run<'a>(
    out: &mut Vec<GlyphPlacement<'a>>,
    run: &'a crate::shape::ShapedRun,
    scale: f32,
    origin: [f32; 2],
    [mut pen, baseline]: [f32; 2],
    cluster: Option<u32>,
    source: GlyphSource,
) {
    let Some(instance) = run.instance.as_ref() else {
        return;
    };
    for glyph in &run.glyphs {
        let x = (pen + glyph.offset_x_px) * scale + origin[0];
        // `offset_y_px` is the shaper's, positive up; screen y grows down.
        // `floor`, not `trunc`: rounding toward zero would snap text above
        // the origin the other way and shift its baseline by a pixel as it
        // scrolls past y = 0.
        let y = (origin[1] - glyph.offset_y_px * scale).floor() + baseline;
        pen += glyph.advance_px;
        out.push(GlyphPlacement {
            instance,
            font_size_px: run.font_size_px * scale,
            glyph_id: glyph.glyph_id,
            position: [x, y],
            cluster: cluster.unwrap_or(glyph.cluster),
            source,
            sideways: false,
        });
    }
}
