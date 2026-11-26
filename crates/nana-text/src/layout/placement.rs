//! Where each glyph of a laid-out paragraph goes on the page.
//!
//! A layout holds runs in line-relative coordinates: an origin and an advance
//! per glyph. Every renderer needs the same walk over them — the line's
//! baseline, the pen moving along the run, the shaper's offsets turned to
//! page y — and the same snapping, or two of them would put a glyph a pixel
//! apart. [`TextLayout::glyph_placements`] is that walk, once.

use crate::font::{FontInstanceKey, FontSystem};
use crate::metrics::VisualVerticalMetrics;
use crate::shape::RunOrientation;

use super::{TextLayout, TextRect};

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

/// Receiver for vector glyph outlines in layout coordinates (logical px, Y-down).
pub trait OutlineSink {
    fn move_to(&mut self, x: f32, y: f32);
    fn line_to(&mut self, x: f32, y: f32);
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32);
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32);
    fn close(&mut self);
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

    /// Union of the measured glyph ink boxes on `line_index`, in layout
    /// coordinates (logical px, Y-down, unscaled). Returns `None` if the line
    /// does not exist, is vertical, or contains no inked glyphs.
    pub fn line_ink_bounds(&self, line_index: usize, fonts: &FontSystem) -> Option<TextRect> {
        if self.is_vertical() {
            return None;
        }
        let line = self.lines.get(line_index)?;
        let baseline = line.metrics.baseline_y_px;
        let mut x_min = f32::INFINITY;
        let mut y_min = f32::INFINITY;
        let mut x_max = f32::NEG_INFINITY;
        let mut y_max = f32::NEG_INFINITY;
        let mut any = false;
        for run in self.line_runs(line) {
            let Some(instance) = run.instance.as_ref() else {
                continue;
            };
            let mut pen = run.origin_x_px;
            for glyph in &run.glyphs {
                if !glyph.flags.contains(crate::shape::GlyphFlags::IGNORED)
                    && let Some(box_rect) =
                        fonts.glyph_ink_bounds(instance, run.font_size_px, glyph.glyph_id)
                {
                    let gx = pen + glyph.offset_x_px + box_rect.x;
                    let gy = baseline - glyph.offset_y_px + box_rect.y;
                    x_min = x_min.min(gx);
                    y_min = y_min.min(gy);
                    x_max = x_max.max(gx + box_rect.width);
                    y_max = y_max.max(gy + box_rect.height);
                    any = true;
                }
                pen += glyph.advance_px;
            }
        }
        any.then(|| TextRect::new(x_min, y_min, x_max - x_min, y_max - y_min))
    }

    /// Union of [`Self::line_ink_bounds`] across every line in this layout.
    pub fn ink_bounds(&self, fonts: &FontSystem) -> Option<TextRect> {
        let mut x_min = f32::INFINITY;
        let mut y_min = f32::INFINITY;
        let mut x_max = f32::NEG_INFINITY;
        let mut y_max = f32::NEG_INFINITY;
        let mut any = false;
        for idx in 0..self.lines.len() {
            if let Some(rect) = self.line_ink_bounds(idx, fonts) {
                x_min = x_min.min(rect.x);
                y_min = y_min.min(rect.y);
                x_max = x_max.max(rect.x + rect.width);
                y_max = y_max.max(rect.y + rect.height);
                any = true;
            }
        }
        any.then(|| TextRect::new(x_min, y_min, x_max - x_min, y_max - y_min))
    }

    /// Measured vertical ink bounds and optical centre offset of `line_index`
    /// relative to its line box centre.
    pub fn line_visual_vertical_metrics(
        &self,
        line_index: usize,
        fonts: &FontSystem,
    ) -> Option<VisualVerticalMetrics> {
        let line = self.lines.get(line_index)?;
        let ink = self.line_ink_bounds(line_index, fonts)?;
        let ink_top_px = ink.y;
        let ink_bottom_px = ink.y + ink.height;
        let ink_center_y_px = ink_top_px + ink.height * 0.5;
        let line_center_y_px = line.metrics.top_y_px + line.metrics.height_px * 0.5;
        Some(VisualVerticalMetrics {
            ink_top_px,
            ink_bottom_px,
            ink_center_y_px,
            line_center_y_px,
            visual_center_offset_y_px: ink_center_y_px - line_center_y_px,
        })
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
                    if glyph.flags.contains(crate::shape::GlyphFlags::IGNORED) {
                        pen += glyph.advance_px;
                        continue;
                    }
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
        if glyph.flags.contains(crate::shape::GlyphFlags::IGNORED) {
            pen += glyph.advance_px;
            continue;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{FaceDescriptor, font_blob};
    use crate::{
        NativeTextEngine, TextConstraints, TextEngine, TextKind, TextSource, TextStyle,
        TextWorkCounters,
    };

    const NOTO_KR: &[u8] = include_bytes!("../../fonts/noto-sans-kr.ttf");
    const COLOR_TTF: &[u8] = include_bytes!("../../fonts/nana-test-color.ttf");

    #[test]
    fn family_names_are_sorted_deduplicated_and_exclude_dot_families() {
        let mut fonts = FontSystem::hermetic();
        // Register in reverse alphabetical order with duplicates to verify sorting & dedup.
        fonts
            .register_bytes(font_blob(NOTO_KR), &FaceDescriptor::default())
            .expect("register noto kr");
        fonts
            .register_bytes(font_blob(NOTO_KR), &FaceDescriptor::default())
            .expect("register duplicate noto kr");
        fonts
            .register_bytes(font_blob(COLOR_TTF), &FaceDescriptor::default())
            .expect("register color ttf");

        let names = fonts.family_names();
        assert_eq!(
            names,
            vec!["NanaTestColor".to_string(), "Noto Sans KR".to_string()]
        );

        let system = FontSystem::with_system_fonts();
        let sys_names = system.family_names();
        assert!(
            sys_names
                .iter()
                .all(|name| !name.is_empty() && !name.starts_with('.')),
            "system family_names must exclude empty and dot-prefixed UI families"
        );
    }

    #[test]
    fn layout_ink_bounds_and_visual_vertical_metrics_measure_glyphs() {
        let mut fonts = FontSystem::hermetic();
        fonts
            .register_bytes(font_blob(NOTO_KR), &FaceDescriptor::default())
            .expect("register noto kr");
        let family = fonts.family_names().into_iter().next().expect("family");
        let mut engine = NativeTextEngine::new(fonts);
        let style = TextStyle {
            font_family: Some(family.into()),
            font_size_px: 15.0,
            ..TextStyle::default()
        };
        let mut counters = TextWorkCounters::default();
        let layout = engine.layout(
            TextKind::Label,
            &TextSource::new("AB가"),
            &style,
            &TextConstraints::default(),
            &mut counters,
        );
        assert_eq!(layout.lines.len(), 1);
        assert!(
            layout
                .runs
                .iter()
                .all(|run| run.metrics.cap_height_px > 0.0),
            "cap_height_px should be populated"
        );

        let line_ink = layout
            .line_ink_bounds(0, engine.fonts())
            .expect("line 0 ink bounds");
        assert!(line_ink.width > 0.0 && line_ink.height > 0.0);
        let total_ink = layout.ink_bounds(engine.fonts()).expect("total ink bounds");
        assert_eq!(total_ink, line_ink);

        let vm = layout
            .line_visual_vertical_metrics(0, engine.fonts())
            .expect("visual vertical metrics");
        assert!((vm.ink_top_px - line_ink.y).abs() < 1e-4);
        assert!((vm.ink_bottom_px - (line_ink.y + line_ink.height)).abs() < 1e-4);
        assert!(
            (vm.visual_center_offset_y_px - (vm.ink_center_y_px - vm.line_center_y_px)).abs()
                < 1e-4
        );
        assert!(
            layout
                .line_visual_vertical_metrics(1, engine.fonts())
                .is_none()
        );
    }
}
