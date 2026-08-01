use std::borrow::Cow;

use iced::widget::{button, column, container, progress_bar, row, space, text};
use iced::{Alignment, Border, Color, Element, Length, Padding, font};

use crate::components::ControlSize;
use crate::icons::{spinner_icon, status_indicator};
use crate::theme::{Colors, ThemeTokens, ui_font};
use crate::widgets::{ButtonKind, CardKind, button_style, card_style, progress_style};

/// A determinate progress display with an optional real cancel action.
pub struct Progress<'a, Message> {
    value: f32,
    max: f32,
    label: Option<Cow<'a, str>>,
    on_cancel: Option<Message>,
}

impl<'a, Message> Progress<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(value: f32, max: f32) -> Self {
        Self {
            value,
            max: max.max(f32::EPSILON),
            label: None,
            on_cancel: None,
        }
    }

    pub fn label(mut self, label: impl Into<Cow<'a, str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn on_cancel(mut self, message: Message) -> Self {
        self.on_cancel = Some(message);
        self
    }

    pub fn value(&self) -> f32 {
        self.value.clamp(0.0, self.max)
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let value = self.value();
        let mut content = column![].spacing(6);
        if self.label.is_some() || self.on_cancel.is_some() {
            let mut heading = row![].spacing(8).align_y(Alignment::Center);
            if let Some(label) = self.label {
                heading = heading.push(
                    text(label)
                        .size(12)
                        .font(ui_font(font::Weight::Medium))
                        .color(colors.text)
                        .width(Length::Fill),
                );
            } else {
                heading = heading.push(space().width(Length::Fill));
            }
            if let Some(message) = self.on_cancel {
                let size = ControlSize::Small.height_in(tokens.metrics);
                heading = heading.push(
                    button(text("×").size(15))
                        .width(Length::Fixed(size))
                        .height(Length::Fixed(size))
                        .padding(0)
                        .on_press(message)
                        .style(button_style(tokens, ButtonKind::Ghost)),
                );
            }
            content = content.push(heading);
        }
        content = content.push(
            progress_bar(0.0..=self.max, value)
                .girth(6)
                .style(progress_style(colors)),
        );
        content.into()
    }
}

/// A lightweight native loading indicator.
pub struct Spinner<'a> {
    label: Cow<'a, str>,
    phase: u8,
    size: f32,
}

impl<'a> Spinner<'a> {
    pub fn new(label: impl Into<Cow<'a, str>>, phase: u8) -> Self {
        Self {
            label: label.into(),
            phase,
            size: 14.0,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size.max(8.0);
        self
    }

    pub fn view<Message: 'a>(self, colors: Colors) -> Element<'a, Message> {
        row![
            spinner_icon(self.phase, self.size, colors.accent),
            text(self.label).size(12).color(colors.muted),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    }
}

/// A non-interactive placeholder that uses the same surface hierarchy as content.
pub struct Skeleton {
    width: Length,
    height: f32,
}

impl Skeleton {
    pub fn new(width: impl Into<Length>, height: f32) -> Self {
        Self {
            width: width.into(),
            height: height.max(1.0),
        }
    }

    pub fn view<'a, Message: 'a>(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let radius = tokens.metrics.radius_sm;
        container(space())
            .width(self.width)
            .height(Length::Fixed(self.height))
            .style(move |_theme| {
                container::Style::default()
                    .background(colors.subtle)
                    .border(Border::default().rounded(radius))
            })
            .into()
    }
}

/// A compact determinate meter for continuously sampled levels.
pub struct LevelMeter {
    value: f32,
    height: f32,
    tone: StatusTone,
}

impl LevelMeter {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            height: 4.0,
            tone: StatusTone::Success,
        }
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(1.0);
        self
    }

    pub fn tone(mut self, tone: StatusTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn view<'a, Message: 'a>(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let colors = theme.into().colors;
        let bar = self.tone.color(colors);
        progress_bar(0.0..=1.0, self.value.clamp(0.0, 1.0))
            .girth(self.height)
            .style(move |_theme| iced::widget::progress_bar::Style {
                background: colors.background.into(),
                bar: bar.into(),
                border: Border::default().rounded(self.height / 2.0),
            })
            .into()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusTone {
    #[default]
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

impl StatusTone {
    fn color(self, colors: Colors) -> Color {
        match self {
            Self::Neutral => colors.muted,
            Self::Info => colors.accent,
            Self::Success => colors.success,
            Self::Warning => colors.warning,
            Self::Danger => colors.danger,
        }
    }
}

/// A compact semantic state label.
pub struct StatusBadge<'a> {
    label: Cow<'a, str>,
    tone: StatusTone,
}

impl<'a> StatusBadge<'a> {
    pub fn new(label: impl Into<Cow<'a, str>>, tone: StatusTone) -> Self {
        Self {
            label: label.into(),
            tone,
        }
    }

    pub fn view<Message: 'a>(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let tone = self.tone.color(colors);
        container(
            row![
                status_indicator(true, 6.0, tone),
                text(self.label)
                    .size(11)
                    .font(ui_font(font::Weight::Medium))
                    .color(tone),
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .padding([3, 7])
        .style(move |_theme| {
            container::Style::default()
                .background(tone.scale_alpha(0.12))
                .border(Border::default().rounded(999.0))
        })
        .into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationIntent {
    Warning,
    Danger,
}

/// Inline validation feedback for form controls.
pub struct ValidationMessage<'a> {
    message: Cow<'a, str>,
    intent: ValidationIntent,
}

impl<'a> ValidationMessage<'a> {
    pub fn new(message: impl Into<Cow<'a, str>>, intent: ValidationIntent) -> Self {
        Self {
            message: message.into(),
            intent,
        }
    }

    pub fn view<Message: 'a>(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let colors = theme.into().colors;
        let tone = match self.intent {
            ValidationIntent::Warning => colors.warning,
            ValidationIntent::Danger => colors.danger,
        };
        row![
            status_indicator(false, 12.0, tone),
            text(self.message).size(11).color(tone),
        ]
        .spacing(5)
        .align_y(Alignment::Center)
        .into()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToastTone {
    #[default]
    Info,
    Success,
    Warning,
    Danger,
}

impl ToastTone {
    fn status(self) -> StatusTone {
        match self {
            Self::Info => StatusTone::Info,
            Self::Success => StatusTone::Success,
            Self::Warning => StatusTone::Warning,
            Self::Danger => StatusTone::Danger,
        }
    }
}

/// An inline or overlay-ready notification with an optional dismiss action.
pub struct Toast<'a, Message> {
    title: Cow<'a, str>,
    description: Option<Cow<'a, str>>,
    tone: ToastTone,
    on_dismiss: Option<Message>,
}

impl<'a, Message> Toast<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(title: impl Into<Cow<'a, str>>, tone: ToastTone) -> Self {
        Self {
            title: title.into(),
            description: None,
            tone,
            on_dismiss: None,
        }
    }

    pub fn description(mut self, description: impl Into<Cow<'a, str>>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn on_dismiss(mut self, message: Message) -> Self {
        self.on_dismiss = Some(message);
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let tone = self.tone.status().color(colors);
        let mut copy = column![
            text(self.title)
                .size(12)
                .font(ui_font(font::Weight::Semibold))
                .color(colors.text),
        ]
        .spacing(2)
        .width(Length::Fill);
        if let Some(description) = self.description {
            copy = copy.push(text(description).size(11).color(colors.muted));
        }
        let mut content = row![status_indicator(true, 7.0, tone), copy]
            .spacing(8)
            .align_y(Alignment::Center);
        if let Some(message) = self.on_dismiss {
            let size = ControlSize::Small.height_in(tokens.metrics);
            content = content.push(
                button(text("×").size(15))
                    .width(Length::Fixed(size))
                    .height(Length::Fixed(size))
                    .padding(0)
                    .on_press(message)
                    .style(button_style(tokens, ButtonKind::Ghost)),
            );
        }
        container(content)
            .width(Length::Fill)
            .padding(Padding::from([10, 12]))
            .style(card_style(tokens, CardKind::Outlined))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::Progress;

    #[test]
    fn progress_clamps_invalid_external_values() {
        assert_eq!(Progress::<()>::new(-5.0, 100.0).value(), 0.0);
        assert_eq!(Progress::<()>::new(125.0, 100.0).value(), 100.0);
        assert_eq!(Progress::<()>::new(0.5, 0.0).value(), f32::EPSILON);
    }
}
