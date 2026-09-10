//! Markdown drawing commands enter the normal Scene primitive stream.
use super::*;

pub(super) fn build(
    context: &GeometryPaintContext<'_>,
    drawing: &nana_ui_runtime::MarkdownDrawing,
    color: [f32; 4],
    emit: &mut impl FnMut(ScenePrimitive),
) {
    let node = context.node;
    let visual = VisualPrimitiveContext {
        node: node.id,
        transform: context.transform,
        clips: context.clips,
        opacity: context.opacity,
        z_index: node.z_index,
        document_order: context.node_order,
    };
    for (index, command) in drawing.commands.iter().enumerate() {
        let mut primitive = match command {
            nana_ui_runtime::MarkdownDrawingCommand::Image { bounds, source } => {
                let mut primitive = visual_quad(
                    &visual,
                    0,
                    scene_rect(*bounds),
                    VisualQuadStyle {
                        background: None,
                        border_color: None,
                        border_width: 0.0,
                        corner_radius: [0.0; 4],
                    },
                );
                if let ScenePrimitiveKind::Quad { surface, .. } = &mut primitive.kind {
                    let mut image = BackgroundImage::url_with_fit(
                        source.as_ref(),
                        nana_ui_core::BackgroundImageFit::Contain,
                    );
                    if let BackgroundImage::Url { repeat, .. } = &mut image {
                        *repeat = nana_ui_core::BackgroundRepeat::NoRepeat;
                    }
                    surface.content_image = Some(image);
                }
                primitive
            }
            nana_ui_runtime::MarkdownDrawingCommand::Svg { bounds, source } => {
                let ink = format!(
                    "rgb({},{},{})",
                    (color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                    (color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                    (color[2].clamp(0.0, 1.0) * 255.0).round() as u8
                );
                let svg = source.replace("#010203", &ink).replace("rgb(1,2,3)", &ink);
                let mut url = String::from("data:image/svg+xml,");
                use std::fmt::Write;
                for byte in svg.bytes() {
                    if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                        url.push(byte as char)
                    } else {
                        let _ = write!(url, "%{byte:02X}");
                    }
                }
                let mut primitive = visual_quad(
                    &visual,
                    0,
                    scene_rect(*bounds),
                    VisualQuadStyle {
                        background: None,
                        border_color: None,
                        border_width: 0.0,
                        corner_radius: [0.0; 4],
                    },
                );
                if let ScenePrimitiveKind::Quad { surface, .. } = &mut primitive.kind {
                    let mut image = BackgroundImage::url_with_fit(
                        url,
                        nana_ui_core::BackgroundImageFit::Contain,
                    );
                    if let BackgroundImage::Url { repeat, .. } = &mut image {
                        *repeat = nana_ui_core::BackgroundRepeat::NoRepeat;
                    }
                    surface.content_image = Some(image);
                }
                primitive
            }
            nana_ui_runtime::MarkdownDrawingCommand::Text {
                bounds,
                text,
                size,
                weight,
                italic,
                line_through,
                underline,
                code,
            } => {
                if *code {
                    let mut tint = color;
                    tint[3] *= 0.10;
                    let background = visual_quad(
                        &visual,
                        1024 + index as u64 * 4,
                        scene_rect(*bounds),
                        VisualQuadStyle {
                            background: Some(tint),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: [2.0; 4],
                        },
                    );
                    emit(background);
                }
                insert_text_decoration_strokes(
                    &visual,
                    scene_rect(*bounds),
                    color,
                    nana_ui_core::TextDecorationLine {
                        underline: *underline,
                        line_through: *line_through,
                    },
                    |mut decoration| {
                        decoration.id.slot = 1026 + index as u64 * 4 + (decoration.id.slot - 12);
                        emit(decoration);
                    },
                );
                let region = ComponentTextRegion {
                    bounds: *bounds,
                    content: text.clone(),
                    color: Some(color),
                    font_size: *size,
                    font_weight: Some(*weight),
                };
                let mut primitive = component_text_primitive(
                    node.id,
                    0,
                    &region,
                    TextHorizontalAlignment::Start,
                    false,
                    node,
                    context.transform,
                    context.clips.clone(),
                    context.opacity,
                    context.node_order,
                );
                if let ScenePrimitiveKind::Text {
                    wrap,
                    spans,
                    italic: text_italic,
                    line_through: text_strike,
                    underline: text_underline,
                    ..
                } = &mut primitive.kind
                {
                    *text_italic = *italic;
                    *text_strike = *line_through;
                    *text_underline = *underline;
                    *wrap = false;
                    spans.clear();
                }
                primitive
            }
            nana_ui_runtime::MarkdownDrawingCommand::Line { points, width } => {
                visual_stroke(&visual, 0, context.bounds, points.clone(), *width, color)
            }
            nana_ui_runtime::MarkdownDrawingCommand::Box { bounds, radius } => visual_quad(
                &visual,
                0,
                scene_rect(*bounds),
                VisualQuadStyle {
                    background: None,
                    border_color: Some(color),
                    border_width: 1.0,
                    corner_radius: corner_radii(*radius),
                },
            ),
        };
        primitive.id.slot = 1025u64.saturating_add((index as u64).saturating_mul(4));
        emit(primitive);
    }
}
