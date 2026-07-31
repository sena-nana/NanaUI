use std::borrow::Cow;

use iced::widget::{button, column, container, mouse_area, row, space, text, tooltip};
use iced::{Alignment, Element, Length, Padding, font};

use crate::components::{Button as UiButton, ControlSize};
use crate::dialog::DialogSize;
use crate::icons::{Icon, icon};
use crate::theme::{ThemeTokens, ui_font};
use crate::tooltip::TooltipConfig;
use crate::widgets::{
    ButtonKind, dialog_close_style, dialog_scrim_style, dialog_surface_style, tooltip_style,
};

/// A modal surface with explicit outside, close and inner-interaction messages.
///
/// Escape handling remains an application subscription because native Iced
/// overlays do not own the host event stream.
pub struct Dialog<'a, Message> {
    title: Cow<'a, str>,
    description: Option<Cow<'a, str>>,
    body: Element<'a, Message>,
    footer: Option<Element<'a, Message>>,
    size: DialogSize,
    on_close: Option<Message>,
    on_outside: Option<Message>,
    on_interaction: Option<Message>,
    close_hidden: bool,
}

impl<'a, Message> Dialog<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(title: impl Into<Cow<'a, str>>, body: impl Into<Element<'a, Message>>) -> Self {
        Self {
            title: title.into(),
            description: None,
            body: body.into(),
            footer: None,
            size: DialogSize::Default,
            on_close: None,
            on_outside: None,
            on_interaction: None,
            close_hidden: false,
        }
    }

    pub fn description(mut self, description: impl Into<Cow<'a, str>>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn footer(mut self, footer: impl Into<Element<'a, Message>>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    pub fn size(mut self, size: DialogSize) -> Self {
        self.size = size;
        self
    }

    pub fn on_close(mut self, message: Message) -> Self {
        self.on_close = Some(message);
        self
    }

    pub fn on_outside(mut self, message: Message) -> Self {
        self.on_outside = Some(message);
        self
    }

    pub fn on_interaction(mut self, message: Message) -> Self {
        self.on_interaction = Some(message);
        self
    }

    pub fn close_hidden(mut self, hidden: bool) -> Self {
        self.close_hidden = hidden;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let mut heading = column![
            text(self.title)
                .size(14)
                .font(ui_font(font::Weight::Semibold))
                .color(colors.text)
        ]
        .spacing(4)
        .width(Length::Fill);
        if let Some(description) = self.description {
            heading = heading.push(text(description).size(12).color(colors.muted));
        }
        let mut header = row![heading].spacing(12).align_y(Alignment::Start);
        if !self.close_hidden {
            let close = button(icon(Icon::Close, 14.0, colors.muted))
                .width(Length::Fixed(ControlSize::Small.height_in(tokens.metrics)))
                .height(Length::Fixed(ControlSize::Small.height_in(tokens.metrics)))
                .padding(0)
                .on_press_maybe(self.on_close.clone())
                .style(dialog_close_style(tokens));
            header = header.push(close);
        }

        let mut surface = column![
            container(header).padding(Padding {
                top: 14.0,
                right: 16.0,
                bottom: 8.0,
                left: 16.0,
            }),
            container(self.body).width(Length::Fill).padding(Padding {
                top: 8.0,
                right: 16.0,
                bottom: if self.footer.is_some() { 10.0 } else { 16.0 },
                left: 16.0,
            }),
        ];
        if let Some(footer) = self.footer {
            surface = surface.push(container(footer).width(Length::Fill).padding(Padding {
                top: 0.0,
                right: 16.0,
                bottom: 14.0,
                left: 16.0,
            }));
        }

        let surface = container(surface)
            .width(Length::Fixed(self.size.max_width()))
            .style(dialog_surface_style(tokens));
        let surface: Element<'a, Message> = if let Some(message) = self.on_interaction {
            mouse_area(surface).on_press(message).into()
        } else {
            surface.into()
        };
        let overlay = container(surface)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .align_top(Length::Fill)
            .padding(Padding {
                top: 90.0,
                right: 16.0,
                bottom: 16.0,
                left: 16.0,
            })
            .style(dialog_scrim_style(colors));
        if let Some(message) = self.on_outside {
            mouse_area(overlay).on_press(message).into()
        } else {
            overlay.into()
        }
    }
}

/// A standard confirm dialog built from [`Dialog`] with real cancel/confirm messages.
pub struct ConfirmDialog<'a, Message> {
    title: Cow<'a, str>,
    message: Cow<'a, str>,
    description: Option<Cow<'a, str>>,
    on_confirm: Message,
    on_cancel: Message,
    on_outside: Option<Message>,
    on_interaction: Message,
    danger: bool,
    size: DialogSize,
}

impl<'a, Message> ConfirmDialog<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        title: impl Into<Cow<'a, str>>,
        message: impl Into<Cow<'a, str>>,
        on_confirm: Message,
        on_cancel: Message,
        on_interaction: Message,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            description: None,
            on_confirm,
            on_cancel,
            on_outside: None,
            on_interaction,
            danger: false,
            size: DialogSize::Default,
        }
    }

    pub fn description(mut self, description: impl Into<Cow<'a, str>>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    pub fn on_outside(mut self, message: Message) -> Self {
        self.on_outside = Some(message);
        self
    }

    pub fn size(mut self, size: DialogSize) -> Self {
        self.size = size;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let on_outside = self.on_outside.unwrap_or_else(|| self.on_cancel.clone());
        let footer = row![
            space().width(Length::Fill),
            UiButton::label("取消")
                .kind(ButtonKind::Ghost)
                .on_press(self.on_cancel.clone())
                .view(tokens),
            UiButton::label("确认")
                .kind(if self.danger {
                    ButtonKind::Danger
                } else {
                    ButtonKind::Primary
                })
                .on_press(self.on_confirm)
                .view(tokens),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let mut dialog = Dialog::new(self.title, text(self.message).size(13).color(colors.text))
            .footer(footer)
            .size(self.size)
            .on_close(self.on_cancel.clone())
            .on_outside(on_outside)
            .on_interaction(self.on_interaction);
        if let Some(description) = self.description {
            dialog = dialog.description(description);
        }
        dialog.view(tokens)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DrawerSide {
    Left,
    #[default]
    Right,
}

/// A modal side drawer using the same dismissal and surface contracts as Dialog.
pub struct Drawer<'a, Message> {
    title: Cow<'a, str>,
    body: Element<'a, Message>,
    footer: Option<Element<'a, Message>>,
    side: DrawerSide,
    width: f32,
    on_close: Message,
    on_interaction: Message,
}

impl<'a, Message> Drawer<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        title: impl Into<Cow<'a, str>>,
        body: impl Into<Element<'a, Message>>,
        on_close: Message,
        on_interaction: Message,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            footer: None,
            side: DrawerSide::Right,
            width: 360.0,
            on_close,
            on_interaction,
        }
    }

    pub fn footer(mut self, footer: impl Into<Element<'a, Message>>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    pub fn side(mut self, side: DrawerSide) -> Self {
        self.side = side;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(240.0);
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let header = row![
            text(self.title)
                .size(14)
                .font(ui_font(font::Weight::Semibold))
                .width(Length::Fill),
            button(icon(Icon::Close, 14.0, colors.muted))
                .width(Length::Fixed(ControlSize::Small.height_in(tokens.metrics),))
                .height(Length::Fixed(ControlSize::Small.height_in(tokens.metrics),))
                .padding(0)
                .on_press(self.on_close.clone())
                .style(dialog_close_style(tokens)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let mut content = column![
            container(header).padding([14, 16]),
            container(self.body)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding([8, 16]),
        ]
        .height(Length::Fill);
        if let Some(footer) = self.footer {
            content = content.push(container(footer).padding([12, 16]));
        }
        let drawer = mouse_area(
            container(content)
                .width(Length::Fixed(self.width))
                .height(Length::Fill)
                .style(dialog_surface_style(tokens)),
        )
        .on_press(self.on_interaction);
        let aligned = match self.side {
            DrawerSide::Left => container(drawer).align_left(Length::Fill),
            DrawerSide::Right => container(drawer).align_right(Length::Fill),
        };
        mouse_area(
            aligned
                .width(Length::Fill)
                .height(Length::Fill)
                .style(dialog_scrim_style(colors)),
        )
        .on_press(self.on_close)
        .into()
    }
}

/// A generic hover/focus tooltip using NanaUI's placement and timing contract.
pub struct Tooltip<'a, Message> {
    trigger: Element<'a, Message>,
    content: Element<'a, Message>,
    config: TooltipConfig,
}

impl<'a, Message> Tooltip<'a, Message>
where
    Message: 'a,
{
    pub fn new(
        trigger: impl Into<Element<'a, Message>>,
        content: impl Into<Element<'a, Message>>,
    ) -> Self {
        Self {
            trigger: trigger.into(),
            content: content.into(),
            config: TooltipConfig::default(),
        }
    }

    pub fn config(mut self, config: TooltipConfig) -> Self {
        self.config = config;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let colors = theme.into().colors;
        tooltip(
            self.trigger,
            container(self.content)
                .width(self.config.max_width)
                .padding([4, 7]),
            self.config.placement.into(),
        )
        .gap(self.config.gap)
        .padding(self.config.viewport_padding)
        .delay(iced::time::Duration::from_millis(self.config.delay_ms))
        .snap_within_viewport(true)
        .style(tooltip_style(colors))
        .into()
    }
}
