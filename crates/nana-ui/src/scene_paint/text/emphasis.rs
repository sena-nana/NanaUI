//! 着重号直接读取保留排版的字素光标几何，复用文字实例和裁剪路径。

use super::{SceneRichPaint, SolidQuad, SpanColors, TextLayout};
use nana_text::{Affinity, CaretPosition};
use unicode_segmentation::UnicodeSegmentation;

/// 每个可见字素绘制一个下方圆点；连字与组合字符仍按字素计数。
pub(super) fn quads(
    layout: &TextLayout,
    content: &str,
    effects: &SceneRichPaint,
    colors: &SpanColors,
    scale: f32,
    origin: [f32; 2],
    out: &mut Vec<SolidQuad>,
) {
    for (at, grapheme) in content.grapheme_indices(true) {
        if !effects.effects_at(at).emphasis || !visible(grapheme) {
            continue;
        }
        let Some((line_index, line)) = layout
            .lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.source.contains(&at))
        else {
            continue;
        };
        let Some(run) = layout
            .line_runs(line)
            .iter()
            .find(|run| run.source.contains(&at))
        else {
            continue;
        };
        if run.is_object() {
            continue;
        }
        let Some(start) = layout.caret_geometry(CaretPosition::new(
            at,
            Affinity::Downstream,
            line_index as u32,
        )) else {
            continue;
        };
        let Some(end) = layout.caret_geometry(CaretPosition::new(
            at + grapheme.len(),
            Affinity::Upstream,
            line_index as u32,
        )) else {
            continue;
        };
        let radius = (run.font_size_px * scale * 0.075).max(1.0);
        // 留出下划线及字体 descent 的空间，注音位于文字上方。
        let center = [
            (start.x_px + end.x_px) * 0.5 * scale + origin[0],
            (line.metrics.baseline_y_px
                + run.metrics.descent_px.max(run.font_size_px * 0.15)
                + run.font_size_px * 0.1)
                * scale
                + origin[1]
                + radius,
        ];
        circle(center, radius, colors.color_at(at).srgb, at as u32, out);
    }
}

fn visible(grapheme: &str) -> bool {
    grapheme
        .chars()
        .any(|ch| !ch.is_whitespace() && !ch.is_control() && ch != '\u{fffc}')
}

/// 在文字栅格尺度生成圆的覆盖率，复用无需图集采样的实色实例。
fn circle(center: [f32; 2], radius: f32, color: [f32; 4], cluster: u32, out: &mut Vec<SolidQuad>) {
    let left = (center[0] - radius).floor() as i32;
    let top = (center[1] - radius).floor() as i32;
    let right = (center[0] + radius).ceil() as i32;
    let bottom = (center[1] + radius).ceil() as i32;
    for y in top..bottom {
        for x in left..right {
            let mut covered = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let dx = x as f32 + (sx as f32 + 0.5) * 0.25 - center[0];
                    let dy = y as f32 + (sy as f32 + 0.5) * 0.25 - center[1];
                    covered += u32::from(dx * dx + dy * dy <= radius * radius);
                }
            }
            if covered > 0 {
                let mut color = color;
                color[3] *= covered as f32 / 16.0;
                out.push(SolidQuad {
                    origin: [x, y],
                    size: [1, 1],
                    color,
                    paint_color: None,
                    line_through: false,
                    cluster,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emphasis_excludes_whitespace_control_and_objects() {
        assert!(!visible(" \t\n"));
        assert!(!visible("\u{fffc}"));
        assert!(visible("中"));
        assert!(visible("e\u{301}"));
    }

    #[test]
    fn circle_has_rounded_edges_and_partial_coverage() {
        let mut out = Vec::new();
        circle([3.0, 3.0], 3.0, [0.0, 0.0, 0.0, 1.0], 8, &mut out);
        assert!(!out.iter().any(|quad| quad.origin == [0, 0]));
        assert!(out.iter().any(|quad| quad.color[3] < 1.0));
        assert!(out.iter().all(|quad| quad.cluster == 8));
    }

    #[test]
    fn retained_layout_places_one_dot_per_visible_grapheme_across_lines() {
        use super::super::SceneTextEffects;
        use nana_text::{
            TextConstraints, TextEngine, TextKind, TextSource, TextStyle, TextWorkCounters,
        };
        let content = "office e\u{301} 中\n文";
        let engine = crate::text_engine::nana_text_engine();
        let layout = crate::text_engine::lock_engine(&engine).layout(
            TextKind::Paragraph,
            &TextSource::new(content),
            &TextStyle {
                font_size_px: 16.0,
                line_height: Some(nana_ui_core::LineHeightSpec::Relative(2.0)),
                ..TextStyle::default()
            },
            &TextConstraints {
                max_width_px: Some(65.0),
                preserve_lines: true,
                ..TextConstraints::default()
            },
            &mut TextWorkCounters::default(),
        );
        let effects = SceneRichPaint::new(
            SceneTextEffects {
                emphasis: true,
                ..SceneTextEffects::default()
            },
            vec![],
        )
        .unwrap();
        let colors = SpanColors::new(content, &[], [1.0; 4], None);
        let mut out = Vec::new();
        quads(&layout, content, &effects, &colors, 1.0, [0.0; 2], &mut out);
        let clusters: std::collections::BTreeSet<_> =
            out.iter().map(|quad| quad.cluster as usize).collect();
        let expected = content
            .grapheme_indices(true)
            .filter(|(_, g)| visible(g))
            .map(|(at, _)| at)
            .collect();
        assert_eq!(
            clusters,
            expected,
            "lines {:?}, runs {:?}",
            layout.lines,
            layout
                .runs
                .iter()
                .map(|run| (
                    run.source.clone(),
                    run.glyphs
                        .iter()
                        .map(|glyph| glyph.cluster)
                        .collect::<Vec<_>>()
                ))
                .collect::<Vec<_>>()
        );
        assert!(layout.lines.len() > 1);
        for quad in &out {
            let line = layout
                .lines
                .iter()
                .find(|line| line.source.contains(&(quad.cluster as usize)))
                .unwrap();
            assert!(
                quad.origin[1] as f32 + quad.size[1] as f32
                    <= line.metrics.baseline_y_px
                        + line.metrics.descent_px
                        + (line.metrics.height_px
                            - line.metrics.ascent_px
                            - line.metrics.descent_px)
                            * 0.5
                        + 1.0,
                "dot must fit line bottom: {quad:?}, {:?}",
                line.metrics
            );
        }
    }
}
