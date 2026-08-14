use iced::advanced::text::{
    Alignment, Ellipsis, LineHeight, Paragraph, Renderer as TextRenderer, Shaping, Text, Wrapping,
};
use iced::{Pixels, Size, alignment, font};
use nana_ui_runtime::{
    ComputedStyle, StableNodeId, TextContent, TextMetrics, TextShapeConstraints, TextShaper,
    TextShaping,
};

use crate::ui_font;

type RendererParagraph = <iced::Renderer as TextRenderer>::Paragraph;

/// Compatibility text backend used by canonical Runtime frames.
///
/// Shaping uses the same Iced/Cryoglyph path as visible NanaUI text while the
/// retained metrics remain backend-neutral Runtime data.
#[derive(Debug, Default, Clone, Copy)]
pub struct IcedTextShaper;

impl TextShaper for IcedTextShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics {
        let font = resolve_font(style.font_family.as_deref(), font_weight(style.font_weight));
        let paragraph = RendererParagraph::with_text(Text {
            content: text.value.as_str(),
            bounds: Size::new(
                constraints.max_width.unwrap_or(f32::INFINITY),
                constraints.max_height.unwrap_or(f32::INFINITY),
            ),
            size: Pixels(style.font_size),
            line_height: style.line_height.map(line_height).unwrap_or_default(),
            font,
            align_x: Alignment::Default,
            align_y: alignment::Vertical::Top,
            shaping: match constraints.shaping {
                TextShaping::Auto => Shaping::Auto,
                TextShaping::Advanced => Shaping::Advanced,
            },
            wrapping: if constraints.wrap {
                Wrapping::Word
            } else {
                Wrapping::None
            },
            ellipsis: if constraints.ellipsis {
                Ellipsis::End
            } else {
                Ellipsis::None
            },
            hint_factor: None,
        });
        let bounds = paragraph.min_bounds();
        let tracking = style.letter_spacing * text.value.chars().count().saturating_sub(1) as f32;
        TextMetrics {
            width: (bounds.width + tracking).max(0.0),
            height: bounds.height,
        }
    }
}

fn line_height(spec: nana_ui_core::LineHeightSpec) -> LineHeight {
    match spec {
        nana_ui_core::LineHeightSpec::Relative(value) => LineHeight::Relative(value.max(0.0)),
        nana_ui_core::LineHeightSpec::Absolute(value) => {
            LineHeight::Absolute(Pixels(value.max(0.0)))
        }
    }
}

fn font_weight(weight: Option<u16>) -> font::Weight {
    match weight.unwrap_or(400) {
        0..=199 => font::Weight::Thin,
        200..=299 => font::Weight::ExtraLight,
        300..=349 => font::Weight::Light,
        350..=449 => font::Weight::Normal,
        450..=549 => font::Weight::Medium,
        550..=649 => font::Weight::Semibold,
        650..=749 => font::Weight::Bold,
        750..=849 => font::Weight::ExtraBold,
        _ => font::Weight::Black,
    }
}

fn resolve_font(family: Option<&str>, weight: font::Weight) -> iced::Font {
    match family
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("monospace") | Some("ui-monospace") => iced::Font {
            weight,
            ..iced::Font::MONOSPACE
        },
        _ => ui_font(weight),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_cjk_with_the_visible_runtime_backend() {
        let metrics = IcedTextShaper.shape(
            StableNodeId::new(1).unwrap(),
            &TextContent {
                value: "输入法".into(),
            },
            &ComputedStyle::default(),
            TextShapeConstraints::default(),
        );
        assert!(metrics.width > 0.0);
        assert!(metrics.height > 0.0);
    }
}
