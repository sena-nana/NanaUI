use std::borrow::Cow;

use iced::widget::{button, container, row, text, tooltip};
use iced::{Alignment, Element, Length, Padding, font};

use crate::icons::{Icon, icon, spinner_icon};
use crate::theme::{ThemeTokens, UI_METRICS, ui_font};
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
        match self {
            Self::Small => 26.0,
            Self::Medium => UI_METRICS.control_height,
            Self::Large => 38.0,
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
            Self::Small => 12.0,
            Self::Medium => 13.0,
            Self::Large => 14.0,
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
    content: Element<'a, Message>,
    on_press: Option<Message>,
    kind: ButtonKind,
    size: ControlSize,
    width: Length,
    disabled: bool,
    loading: bool,
    loading_phase: u8,
}

impl<'a, Message> Button<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            content: content.into(),
            on_press: None,
            kind: ButtonKind::Ghost,
            size: ControlSize::Medium,
            width: Length::Shrink,
            disabled: false,
            loading: false,
            loading_phase: 0,
        }
    }

    pub fn label(label: impl Into<Cow<'a, str>>) -> Self {
        Self::new(text(label.into()).font(ui_font(font::Weight::Medium)))
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
        let content: Element<'a, Message> = if self.loading {
            row![
                spinner_icon(
                    self.loading_phase,
                    self.size.icon_size(),
                    button_foreground(colors, self.kind),
                ),
                self.content,
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        } else {
            self.content
        };

        button(content)
            .width(self.width)
            .height(Length::Fixed(self.size.height()))
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
        let size = self.size.height();
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

    #[test]
    fn control_sizes_preserve_lilia_geometry_order() {
        assert!(ControlSize::Small.height() < ControlSize::Medium.height());
        assert!(ControlSize::Medium.height() < ControlSize::Large.height());
        assert!(ControlSize::Small.padding_x() < ControlSize::Large.padding_x());
    }
}
