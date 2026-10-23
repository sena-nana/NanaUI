//! 主题着色器字形快照的保留几何、范围颜色与外部整形器回归。
use nana_text::{
    FontId, GlyphFlags, LineBox, LineBreakCause, LineMetrics, RunDirection, RunMetrics,
    RunOrientation, ShapedGlyph, ShapedRun, TextConstraints, TextKind, TextLayout, TextRect,
};
use nana_ui_core::{PaintColor, TextWrapBreak};
use nana_ui_runtime::{
    RetainedTextLayout, TextHorizontalAlignment, TextShaping, TextValue, TextVerticalAlignment,
};
use nana_ui_scene::{
    AffineTransform, PrimitiveId, ScenePrimitive, ScenePrimitiveKind, SceneRect, SceneTextSpan,
};
use std::sync::Arc;

fn text_primitive() -> ScenePrimitive {
    let glyph = ShapedGlyph {
        glyph_id: 7,
        cluster: 0,
        cluster_end: 1,
        advance_px: 12.0,
        advance_y_px: 0.0,
        offset_x_px: 1.0,
        offset_y_px: 2.0,
        flags: GlyphFlags::NONE,
    };
    let run = ShapedRun {
        id: Default::default(),
        source: 0..1,
        direction: RunDirection::Ltr,
        bidi_level: 0,
        script: Default::default(),
        orientation: RunOrientation::Horizontal,
        font: FontId::default(),
        font_size_px: 14.0,
        glyphs: vec![glyph],
        advance_px: 12.0,
        origin_x_px: 3.0,
        metrics: RunMetrics::default(),
        instance: None,
        ignored_axes: Vec::new(),
    };
    let layout = TextLayout {
        id: nana_text::TextLayoutId::from_parts(4, 1),
        kind: TextKind::Paragraph,
        revision: Default::default(),
        font_generation: Default::default(),
        constraints: TextConstraints::default(),
        runs: vec![run],
        lines: vec![LineBox {
            index: 0,
            source: 0..1,
            runs: 0..1,
            break_cause: LineBreakCause::EndOfText,
            metrics: LineMetrics {
                baseline_y_px: 10.0,
                ..LineMetrics::default()
            },
            bounds: TextRect::new(0.0, 0.0, 20.0, 20.0),
            base_direction: RunDirection::Ltr,
        }],
        bounds: TextRect::new(0.0, 0.0, 20.0, 20.0),
        overflow: Default::default(),
        unsupported_writing_mode: false,
        objects: Vec::new(),
        rubies: Vec::new(),
        rubies_dropped: false,
        labels: Vec::new(),
    };
    ScenePrimitive {
        id: PrimitiveId {
            node: nana_ui_runtime::StableNodeId::new(1).unwrap(),
            slot: 0,
        },
        node: nana_ui_runtime::StableNodeId::new(1).unwrap(),
        bounds: SceneRect {
            x: 5.0,
            y: 6.0,
            width: 20.0,
            height: 20.0,
        },
        transform: AffineTransform::IDENTITY,
        clips: Arc::from([]),
        opacity: 0.75,
        z_index: 0,
        document_order: 0,
        kind: ScenePrimitiveKind::Text {
            content: TextValue::from("a"),
            color: Some(PaintColor::srgb([0.1, 0.2, 0.3, 1.0])),
            size: 14.0,
            weight: None,
            family: None,
            line_height: None,
            letter_spacing: 0.0,
            wrap: false,
            ellipsis: false,
            max_lines: None,
            shaping: TextShaping::Auto,
            horizontal_alignment: TextHorizontalAlignment::Start,
            vertical_alignment: TextVerticalAlignment::Top,
            spans: vec![SceneTextSpan {
                start: 0,
                end: 1,
                color: PaintColor::srgb([0.8, 0.7, 0.6, 1.0]),
            }],
            text_shadow: None,
            underline: false,
            line_through: false,
            font_features: Vec::new(),
            italic: false,
            wrap_break: TextWrapBreak::default(),
            opentype: Default::default(),
            layout: Some(RetainedTextLayout {
                id: layout.id,
                layout: Arc::new(layout),
            }),
            rich: None,
            presentation: None,
        },
    }
}

#[test]
fn shader_input_exposes_retained_glyph_geometry_and_span_color() {
    let input = text_primitive()
        .text_shader_input()
        .expect("retained layout");
    assert_eq!(input.layout_id, nana_text::TextLayoutId::from_parts(4, 1));
    assert_eq!(input.glyphs.len(), 1);
    assert_eq!(input.glyphs[0].glyph_id, 7);
    assert_eq!(input.glyphs[0].position, [4.0, 8.0]);
    assert_eq!(
        input.glyphs[0].color,
        input.spans.first().map(|span| span.color.clone())
    );
    assert_eq!(input.opacity, 0.75);
}

#[test]
fn shader_input_is_absent_for_host_shaper_layouts() {
    let mut primitive = text_primitive();
    if let ScenePrimitiveKind::Text { layout, .. } = &mut primitive.kind {
        *layout = None;
    }
    assert!(primitive.text_shader_input().is_none());
}
