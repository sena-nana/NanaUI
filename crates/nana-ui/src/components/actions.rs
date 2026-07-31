use std::borrow::Cow;

use iced::widget::text::LineHeight;
use iced::widget::{button, container, row, text, tooltip};
use iced::{Alignment, Element, Length, Padding, Pixels, font};

use crate::icons::{Icon, icon, spinner_icon};
use crate::theme::{ThemeTokens, UI_BASE_TEXT_SIZE, UI_METRICS, ui_font};
use crate::widgets::{ButtonKind, button_style, tooltip_style};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ControlSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl ControlSize {
    pub const fn height(self) -> f32 {
        self.height_in(UI_METRICS)
    }

    pub const fn height_in(self, metrics: crate::theme::ThemeMetrics) -> f32 {
        match self {
            Self::Small => metrics.small_control_height(),
            Self::Medium => metrics.medium_control_height(),
            Self::Large => metrics.large_control_height(),
        }
    }

    pub const fn line_height(self) -> f32 {
        match self {
            Self::Small | Self::Medium => 16.0,
            Self::Large => 18.0,
        }
    }

    pub const fn vertical_padding(self, metrics: crate::theme::ThemeMetrics) -> f32 {
        let remaining = self.height_in(metrics) - self.line_height();
        if remaining > 0.0 {
            remaining / 2.0
        } else {
            0.0
        }
    }

    pub fn nearest(height: f32) -> Self {
        if !height.is_finite() {
            return Self::Medium;
        }
        if height <= (Self::Small.height() + Self::Medium.height()) / 2.0 {
            Self::Small
        } else if height <= (Self::Medium.height() + Self::Large.height()) / 2.0 {
            Self::Medium
        } else {
            Self::Large
        }
    }

    pub const fn padding_x(self) -> f32 {
        match self {
            Self::Small => 8.0,
            Self::Medium => UI_METRICS.control_padding_x,
            Self::Large => 14.0,
        }
    }

    pub const fn text_size(self) -> f32 {
        match self {
            Self::Small => UI_BASE_TEXT_SIZE - 1.0,
            Self::Medium => UI_BASE_TEXT_SIZE,
            Self::Large => UI_BASE_TEXT_SIZE + 1.0,
        }
    }

    pub const fn icon_size(self) -> f32 {
        match self {
            Self::Small => 13.0,
            Self::Medium => 14.0,
            Self::Large => 16.0,
        }
    }
}

/// A Lilia-style action button with shared sizing, loading and disabled behavior.
pub struct Button<'a, Message> {
    content: ButtonContent<'a, Message>,
    on_press: Option<Message>,
    kind: ButtonKind,
    size: ControlSize,
    width: Length,
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
        let content: Element<'a, Message> = match self.content {
            ButtonContent::Custom(content) => content,
            ButtonContent::Label(label) => text(label)
                .size(self.size.text_size())
                .line_height(LineHeight::Absolute(Pixels(self.size.line_height())))
                .font(ui_font(font::Weight::Medium))
                .into(),
        };
        let content: Element<'a, Message> = if self.loading {
            row![
                spinner_icon(
                    self.loading_phase,
                    self.size.icon_size(),
                    button_foreground(colors, self.kind),
                ),
                content,
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        } else {
            content
        };

        button(content)
            .width(self.width)
            .height(Length::Fixed(self.size.height_in(tokens.metrics)))
            .padding(Padding {
                top: 0.0,
                right: self.size.padding_x(),
                bottom: 0.0,
                left: self.size.padding_x(),
            })
            .on_press_maybe(
                (!self.disabled && !self.loading)
                    .then_some(self.on_press)
                    .flatten(),
            )
            .style(button_style(tokens, self.kind))
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
        let size = self.size.height_in(tokens.metrics);
        let action = button(icon(
            self.icon,
            self.size.icon_size(),
            button_foreground(colors, kind),
        ))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .padding(0)
        .on_press_maybe((!self.disabled).then_some(self.on_press).flatten())
        .style(button_style(tokens, kind));
        tooltip(
            action,
            container(text(self.label).size(11))
                .padding([4, 7])
                .style(tooltip_style(colors)),
            tooltip::Position::Bottom,
        )
        .gap(6)
        .into()
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
