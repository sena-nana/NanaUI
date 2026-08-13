use iced::advanced::text::{
    Alignment, Ellipsis, LineHeight, Paragraph, Renderer as TextRenderer, Shaping, Text, Wrapping,
};
use iced::{Pixels, Size, alignment};
use nana_ui_runtime::{ComputedStyle, StableNodeId, TextContent, TextMetrics, TextShaper};

use crate::iced_app::{css_font_weight_to_iced, line_height_from_spec, resolve_iced_font};

type RendererParagraph = <iced::Renderer as TextRenderer>::Paragraph;

/// Compatibility adapter from the backend-neutral runtime to NanaUI's current
/// Iced text backend. It uses the same advanced shaping and fallback path as
/// visible widgets; the runtime itself remains independent from Iced.
#[derive(Debug, Default, Clone, Copy)]
pub struct IcedTextShaper;

impl TextShaper for IcedTextShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
    ) -> TextMetrics {
        let weight = css_font_weight_to_iced(style.font_weight);
        let font = resolve_iced_font(style.font_family.as_deref(), weight);
        let paragraph = RendererParagraph::with_text(Text {
            content: text.value.as_str(),
            bounds: Size::INFINITE,
            size: Pixels(style.font_size),
            line_height: style
                .line_height
                .map(line_height_from_spec)
                .unwrap_or_else(LineHeight::default),
            font,
            align_x: Alignment::Default,
            align_y: alignment::Vertical::Top,
            shaping: Shaping::Advanced,
            wrapping: Wrapping::None,
            ellipsis: Ellipsis::None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_cjk_through_the_visible_renderer_backend() {
        let mut shaper = IcedTextShaper;
        let metrics = shaper.shape(
            StableNodeId::new(1).unwrap(),
            &TextContent {
                value: "输入法".into(),
            },
            &ComputedStyle::default(),
        );
        assert!(metrics.width > 0.0);
        assert!(metrics.height > 0.0);
    }
}
