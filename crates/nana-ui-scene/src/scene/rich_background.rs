//! 富文本背景使用文字本身的 retained 行布局，复用换行、双向文本与竖排几何。

use super::*;

/// 背景改变时只覆盖默认文字绘制色，作者文字色与选区/语法色保持优先。
pub(super) fn fill_spans(
    node: &ExtractedNode,
    nodes: &SceneNodes,
    existing: Vec<SceneTextSpan>,
) -> Vec<SceneTextSpan> {
    let Some(text) = node.text.as_ref() else {
        return existing;
    };
    if !depends_on_backdrop(node) {
        return rich_fill_spans(text.value.as_str(), node.rich_text.as_ref(), existing);
    }
    let default = slot_color(node.style.paint_colors.color, node.style.color);
    let backdrop = backdrop(node, nodes);
    rich_fill_spans_with(
        text.value.as_str(),
        node.rich_text.as_ref(),
        existing,
        |span| {
            span.paint
                .color
                .or_else(|| contrast_fill(span.paint.background_color?, default?, backdrop))
        },
    )
}

/// 仅背景存在且使用默认文字色的节点依赖祖先底色。
pub(super) fn depends_on_backdrop(node: &ExtractedNode) -> bool {
    node.rich_text.as_ref().is_some_and(|rich| {
        rich.spans()
            .iter()
            .any(|(_, span)| span.paint.background_color.is_some() && span.paint.color.is_none())
    })
}

/// 从最近表面向外查找，在线性颜色空间还原半透明表面的实际底色。
fn backdrop(node: &ExtractedNode, nodes: &SceneNodes) -> nana_ui_core::PaintColor {
    let mut surface = nana_ui_core::PaintColor::srgb([0.0; 4]);
    let mut current = Some(node);
    for _ in 0..MAX_ANCESTOR_DEPTH {
        let Some(node) = current else {
            break;
        };
        if let Some(background) =
            slot_color(node.style.paint_colors.background, node.style.background)
        {
            surface = composite(surface, background);
            if surface.to_linear_sc_rgb().1 >= 1.0 {
                break;
            }
        }
        current = node.parent.and_then(|id| nodes.get(&id).map(Arc::as_ref));
    }
    surface
}

/// 只有默认文字与合成背景不足 4.5:1 时才使用对比更高的黑/白色。
fn contrast_fill(
    background: nana_ui_core::PaintColor,
    default: nana_ui_core::PaintColor,
    backdrop: nana_ui_core::PaintColor,
) -> Option<nana_ui_core::PaintColor> {
    let background = composite(background, backdrop);
    if background.to_linear_sc_rgb().1 < 1.0 {
        // 场景外宿主底色未知时保持默认，避免猜测透明窗口后面的颜色。
        return None;
    }
    let background_luminance = luminance(background);
    let fill_luminance = luminance(composite(default, background));
    let contrast = (background_luminance.max(fill_luminance) + 0.05)
        / (background_luminance.min(fill_luminance) + 0.05);
    if contrast >= 4.5 {
        return None;
    }
    let black_contrast = (background_luminance + 0.05) / 0.05;
    let white_contrast = 1.05 / (background_luminance + 0.05);
    let channel = if black_contrast >= white_contrast {
        0.0
    } else {
        1.0
    };
    Some(nana_ui_core::PaintColor::srgb([
        channel, channel, channel, 1.0,
    ]))
}

/// 与绘制器同样使用线性 RGB 进行 alpha 合成，避免半透明颜色亮度失真。
fn composite(
    front: nana_ui_core::PaintColor,
    back: nana_ui_core::PaintColor,
) -> nana_ui_core::PaintColor {
    let (front, front_alpha) = front.to_linear_sc_rgb();
    let (back, back_alpha) = back.to_linear_sc_rgb();
    let front_alpha = front_alpha.clamp(0.0, 1.0);
    let back_weight = back_alpha.clamp(0.0, 1.0) * (1.0 - front_alpha);
    let alpha = front_alpha + back_weight;
    let channels = if alpha > 0.0 {
        std::array::from_fn(|i| (front[i] * front_alpha + back[i] * back_weight) / alpha)
    } else {
        [0.0; 3]
    };
    nana_ui_core::PaintColor::LinearScRgb { channels, alpha }
}

/// WCAG 相对明度，使用线性颜色的加权亮度。
fn luminance(color: nana_ui_core::PaintColor) -> f32 {
    let (channels, _) = color.to_linear_sc_rgb();
    0.2126 * channels[0].clamp(0.0, 1.0)
        + 0.7152 * channels[1].clamp(0.0, 1.0)
        + 0.0722 * channels[2].clamp(0.0, 1.0)
}

/// 按已排版的文字范围生成背景，颜色变更无需重新排版。
pub(super) fn primitives(
    context: &VisualPrimitiveContext<'_>,
    bounds: SceneRect,
    vertical: TextVerticalAlignment,
    retained: &nana_ui_runtime::RetainedTextLayout,
    rich: &nana_ui_core::RichText,
) -> Vec<ScenePrimitive> {
    let (page_width, top) = text_frame(bounds, vertical, retained);
    let mut out = Vec::new();
    for (range, style) in rich.spans().iter() {
        let Some(color) = style.paint.background_color else {
            continue;
        };
        for rect in retained.layout.selection_rects(range) {
            let rect = retained.layout.page_rect(rect, page_width);
            let mut primitive = visual_quad(
                context,
                collection_slot(TEXT_SPAN_BACKGROUND, out.len()),
                SceneRect {
                    x: bounds.x + rect.x,
                    y: top + rect.y,
                    width: rect.width,
                    height: rect.height,
                },
                VisualQuadStyle::solid(color.to_srgb()),
            );
            // 保留作者颜色合同，由 painter 按主题解析语义色与混合色。
            if let ScenePrimitiveKind::Quad { background, .. } = &mut primitive.kind {
                *background = Some(color);
            }
            out.push(primitive);
        }
    }
    out
}
