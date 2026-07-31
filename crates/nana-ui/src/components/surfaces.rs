use std::borrow::Cow;

use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length, Padding, font};

use crate::components::{ValidationIntent, ValidationMessage};
use crate::icons::{Icon, icon, spinner_icon};
use crate::theme::{ThemeTokens, UI_METRICS, tracked_label, ui_font};
use crate::widgets::{CardKind, card_style, interactive_card_style, list_item_style};

/// A reusable content card with optional title and loading state.
pub struct Card<'a, Message> {
    content: Element<'a, Message>,
    title: Option<Cow<'a, str>>,
    kind: CardKind,
    loading: bool,
    loading_phase: u8,
    padding: Padding,
}

impl<'a, Message> Card<'a, Message>
where
    Message: 'a,
{
    pub fn new(content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            content: content.into(),
            title: None,
            kind: CardKind::Surface,
            loading: false,
            loading_phase: 0,
            padding: Padding::from([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x]),
        }
    }

    pub fn title(mut self, title: impl Into<Cow<'a, str>>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn kind(mut self, kind: CardKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn loading(mut self, loading: bool, phase: u8) -> Self {
        self.loading = loading;
        self.loading_phase = phase;
        self
    }

    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let mut content = column![].spacing(8);
        if let Some(title) = self.title {
            let mut heading = row![tracked_label(
                &title.to_uppercase(),
                13.0,
                font::Weight::Semibold,
                0.5,
                colors.muted,
            )]
            .width(Length::Fill)
            .align_y(Alignment::Center);
            if self.loading {
                heading = heading.push(spinner_icon(self.loading_phase, 14.0, colors.accent));
            }
            content = content.push(heading);
        }
        content = content.push(self.content);
        container(content)
            .width(Length::Fill)
            .padding(self.padding)
            .style(card_style(tokens, self.kind))
            .into()
    }
}

/// A selectable card that emits a real application message on activation.
pub struct InteractiveCard<'a, Message> {
    content: Element<'a, Message>,
    on_select: Option<Message>,
    selected: bool,
    disabled: bool,
}

impl<'a, Message> InteractiveCard<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            content: content.into(),
            on_select: None,
            selected: false,
            disabled: false,
        }
    }

    pub fn on_select(mut self, message: Message) -> Self {
        self.on_select = Some(message);
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        button(self.content)
            .width(Length::Fill)
            .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
            .on_press_maybe((!self.disabled).then_some(self.on_select).flatten())
            .style(interactive_card_style(tokens, self.selected))
            .into()
    }
}

/// A compact selectable list row with optional leading and trailing content.
pub struct ListItem<'a, Message> {
    content: Element<'a, Message>,
    leading: Option<Element<'a, Message>>,
    trailing: Option<Element<'a, Message>>,
    on_select: Option<Message>,
    selected: bool,
    disabled: bool,
}

impl<'a, Message> ListItem<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            content: content.into(),
            leading: None,
            trailing: None,
            on_select: None,
            selected: false,
            disabled: false,
        }
    }

    pub fn label(label: impl Into<Cow<'a, str>>) -> Self {
        Self::new(text(label.into()).font(ui_font(font::Weight::Medium)))
    }

    pub fn leading(mut self, leading: impl Into<Element<'a, Message>>) -> Self {
        self.leading = Some(leading.into());
        self
    }

    pub fn trailing(mut self, trailing: impl Into<Element<'a, Message>>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }

    pub fn on_select(mut self, message: Message) -> Self {
        self.on_select = Some(message);
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let mut content = row![].spacing(8).align_y(Alignment::Center);
        if let Some(leading) = self.leading {
            content = content.push(leading);
        }
        content = content.push(container(self.content).width(Length::Fill));
        if let Some(trailing) = self.trailing {
            content = content.push(trailing);
        }
        button(content)
            .width(Length::Fill)
            .height(Length::Fixed(UI_METRICS.selection_height))
            .padding([0.0, UI_METRICS.list_item_padding_x])
            .align_x(iced::alignment::Horizontal::Left)
            .on_press_maybe((!self.disabled).then_some(self.on_select).flatten())
            .style(list_item_style(tokens, self.selected))
            .into()
    }
}

/// A label/hint/error wrapper for any native control.
pub struct FormField<'a, Message> {
    label: Cow<'a, str>,
    control: Element<'a, Message>,
    hint: Option<Cow<'a, str>>,
    error: Option<Cow<'a, str>>,
}

impl<'a, Message> FormField<'a, Message>
where
    Message: 'a,
{
    pub fn new(label: impl Into<Cow<'a, str>>, control: impl Into<Element<'a, Message>>) -> Self {
        Self {
            label: label.into(),
            control: control.into(),
            hint: None,
            error: None,
        }
    }

    pub fn hint(mut self, hint: impl Into<Cow<'a, str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn error(mut self, error: impl Into<Cow<'a, str>>) -> Self {
        self.error = Some(error.into());
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let mut content = column![
            text(self.label)
                .size(12)
                .font(ui_font(font::Weight::Medium))
                .color(colors.text),
            self.control,
        ]
        .spacing(5)
        .width(Length::Fill);
        if let Some(error) = self.error {
            content =
                content.push(ValidationMessage::new(error, ValidationIntent::Danger).view(tokens));
        } else if let Some(hint) = self.hint {
            content = content.push(text(hint).size(11).color(colors.muted));
        }
        content.into()
    }
}

/// A compact empty state with optional icon, message and real action content.
pub struct EmptyState<'a, Message> {
    title: Cow<'a, str>,
    message: Option<Cow<'a, str>>,
    icon: Option<Icon>,
    action: Option<Element<'a, Message>>,
}

impl<'a, Message> EmptyState<'a, Message>
where
    Message: 'a,
{
    pub fn new(title: impl Into<Cow<'a, str>>) -> Self {
        Self {
            title: title.into(),
            message: None,
            icon: None,
            action: None,
        }
    }

    pub fn message(mut self, message: impl Into<Cow<'a, str>>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn action(mut self, action: impl Into<Element<'a, Message>>) -> Self {
        self.action = Some(action.into());
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let mut content = column![].spacing(6).align_x(Alignment::Center);
        if let Some(empty_icon) = self.icon {
            content = content.push(icon(empty_icon, 22.0, colors.faint));
        }
        content = content.push(
            text(self.title)
                .size(13)
                .font(ui_font(font::Weight::Semibold))
                .color(colors.text),
        );
        if let Some(message) = self.message {
            content = content.push(text(message).size(12).color(colors.muted));
        }
        if let Some(action) = self.action {
            content = content.push(container(action).padding(Padding {
                top: 4.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            }));
        }
        container(content)
            .width(Length::Fill)
            .padding([24, 16])
            .center_x(Length::Fill)
            .into()
    }
}
