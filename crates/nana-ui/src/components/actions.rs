use std::borrow::Cow;

use iced::widget::text::LineHeight;
use iced::widget::{button, row, text};
use iced::{Alignment, Element, Length, Padding, Pixels, font};

use crate::icons::{Icon, icon, spinner_icon};
use crate::theme::{ThemeTokens, ui_font};
use crate::tooltip::{TooltipConfig, TooltipPlacement, tooltip_view};
use crate::widgets::{ButtonKind, ButtonPaintOverride, button_style_overridden};

pub use nana_ui_core::ControlSize;

/// A Lilia-style action button with shared sizing, loading and disabled behavior.
pub struct Button<'a, Message> {
    content: ButtonContent<'a, Message>,
    on_press: Option<Message>,
    kind: ButtonKind,
    size: ControlSize,
    width: Length,
    height: Option<f32>,
    padding: Option<Padding>,
    paint: ButtonPaintOverride,
    disabled: bool,
    loading: bool,
    loading_phase: u8,
}

enum ButtonContent<'a, Message> {
    Custom(Element<'a, Message>),
    Label(Cow<'a, str>),
}

impl<'a, Message> Button<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(content: impl Into<Element<'a, Message>>) -> Self {
        Self::with_content(ButtonContent::Custom(content.into()))
    }

    pub fn label(label: impl Into<Cow<'a, str>>) -> Self {
        Self::with_content(ButtonContent::Label(label.into()))
    }

    fn with_content(content: ButtonContent<'a, Message>) -> Self {
        Self {
            content,
            on_press: None,
            kind: ButtonKind::Ghost,
            size: ControlSize::Medium,
            width: Length::Shrink,
            height: None,
            padding: None,
            paint: ButtonPaintOverride::default(),
            disabled: false,
            loading: false,
            loading_phase: 0,
        }
    }

    pub fn on_press(mut self, message: Message) -> Self {
        self.on_press = Some(message);
        self
    }

    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Override control height (CSS `height` / fixed chrome). `None` keeps [`ControlSize`].
    pub fn height(mut self, height: impl Into<Option<f32>>) -> Self {
        self.height = height.into();
        self
    }

    /// Override padding (CSS `padding`). `None` keeps [`ControlSize::padding_x`].
    pub fn padding(mut self, padding: impl Into<Option<Padding>>) -> Self {
        self.padding = padding.into();
        self
    }

    /// Overlay Layout/CSS surface paint on top of [`ButtonKind`] defaults.
    pub fn paint(mut self, paint: ButtonPaintOverride) -> Self {
        self.paint = paint;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn loading(mut self, loading: bool, phase: u8) -> Self {
        self.loading = loading;
        self.loading_phase = phase;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let fg = self
            .paint
            .text_color
            .unwrap_or_else(|| button_foreground(colors, self.kind));
        let content: Element<'a, Message> = match self.content {
            ButtonContent::Custom(content) => content,
            ButtonContent::Label(label) => text(label)
                .size(self.size.text_size())
                .line_height(LineHeight::Absolute(Pixels(self.size.line_height())))
                .font(ui_font(font::Weight::Medium))
                .color(fg)
                .into(),
        };
        let content: Element<'a, Message> = if self.loading {
            row![
                spinner_icon(self.loading_phase, self.size.icon_size(), fg),
                content,
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        } else {
            content
        };

        let height = self
            .height
            .filter(|h| h.is_finite() && *h > 0.0)
            .unwrap_or_else(|| self.size.height_in(tokens.metrics));
        let padding = self.padding.unwrap_or(Padding {
            top: 0.0,
            right: self.size.padding_x(),
            bottom: 0.0,
            left: self.size.padding_x(),
        });

        button(content)
            .width(self.width)
            .height(Length::Fixed(height))
            .padding(padding)
            .on_press_maybe(
                (!self.disabled && !self.loading)
                    .then_some(self.on_press)
                    .flatten(),
            )
            .style(button_style_overridden(tokens, self.kind, self.paint))
            .into()
    }
}

/// A square icon action that always exposes its label through a native tooltip.
pub struct IconButton<'a, Message> {
    label: Cow<'a, str>,
    icon: Icon,
    on_press: Option<Message>,
    kind: ButtonKind,
    size: ControlSize,
    width: Option<f32>,
    height: Option<f32>,
    padding: Option<Padding>,
    paint: ButtonPaintOverride,
    disabled: bool,
    selected: bool,
}

impl<'a, Message> IconButton<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(label: impl Into<Cow<'a, str>>, icon: Icon) -> Self {
        Self {
            label: label.into(),
            icon,
            on_press: None,
            kind: ButtonKind::Ghost,
            size: ControlSize::Medium,
            width: None,
            height: None,
            padding: None,
            paint: ButtonPaintOverride::default(),
            disabled: false,
            selected: false,
        }
    }

    pub fn on_press(mut self, message: Message) -> Self {
        self.on_press = Some(message);
        self
    }

    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn width(mut self, width: impl Into<Option<f32>>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Option<f32>>) -> Self {
        self.height = height.into();
        self
    }

    pub fn padding(mut self, padding: impl Into<Option<Padding>>) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn paint(mut self, paint: ButtonPaintOverride) -> Self {
        self.paint = paint;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let kind = if self.selected {
            ButtonKind::Selected
        } else {
            self.kind
        };
        let fallback = self.size.height_in(tokens.metrics);
        let width = self
            .width
            .filter(|w| w.is_finite() && *w > 0.0)
            .unwrap_or(fallback);
        let height = self
            .height
            .filter(|h| h.is_finite() && *h > 0.0)
            .unwrap_or(fallback);
        let fg = self
            .paint
            .text_color
            .unwrap_or_else(|| button_foreground(colors, kind));
        let action = button(icon(self.icon, self.size.icon_size(), fg))
            .width(Length::Fixed(width))
            .height(Length::Fixed(height))
            .padding(self.padding.unwrap_or(Padding::ZERO))
            .on_press_maybe((!self.disabled).then_some(self.on_press).flatten())
            .style(button_style_overridden(tokens, kind, self.paint));
        tooltip_view(
            action,
            text(self.label).size(11),
            TooltipConfig {
                placement: TooltipPlacement::Bottom,
                ..TooltipConfig::default()
            },
            colors,
        )
    }
}

fn button_foreground(colors: crate::theme::Colors, kind: ButtonKind) -> iced::Color {
    match kind {
        ButtonKind::Primary => colors.accent_on_soft,
        ButtonKind::Warning => colors.warning,
        ButtonKind::Danger => colors.danger,
        ButtonKind::Text => colors.accent,
        ButtonKind::Ghost | ButtonKind::Subtle | ButtonKind::Selected => colors.text,
    }
}

#[cfg(test)]
mod tests {
    use super::ControlSize;
    use crate::theme::UI_METRICS;

    #[test]
    fn control_sizes_preserve_lilia_geometry_order() {
        assert_eq!(ControlSize::Small.height(), 28.0);
        assert_eq!(ControlSize::Medium.height(), 32.0);
        assert_eq!(ControlSize::Large.height(), 36.0);
        assert!(ControlSize::Small.padding_x() < ControlSize::Large.padding_x());
    }

    #[test]
    fn legacy_heights_snap_to_the_nearest_control_size() {
        assert_eq!(ControlSize::nearest(27.0), ControlSize::Small);
        assert_eq!(ControlSize::nearest(31.0), ControlSize::Medium);
        assert_eq!(ControlSize::nearest(35.0), ControlSize::Large);
        assert_eq!(ControlSize::nearest(f32::NAN), ControlSize::Medium);
    }

    #[test]
    fn control_sizes_resolve_host_metrics_without_changing_the_public_shape() {
        let metrics = crate::theme::ThemeMetrics {
            compact_control_height: 30.0,
            control_height: 34.0,
            selection_height: 38.0,
            ..UI_METRICS
        };

        assert_eq!(ControlSize::Small.height_in(metrics), 30.0);
        assert_eq!(ControlSize::Medium.height_in(metrics), 34.0);
        assert_eq!(ControlSize::Large.height_in(metrics), 38.0);
        for size in [ControlSize::Small, ControlSize::Medium, ControlSize::Large] {
            assert_eq!(
                size.line_height() + size.vertical_padding(metrics) * 2.0,
                size.height_in(metrics)
            );
        }
    }
}
