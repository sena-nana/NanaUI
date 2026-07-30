use iced::widget::{button, column, container, mouse_area, row, space, text};
use iced::{Alignment, Element, Length, Padding, font};
use std::rc::Rc;

use crate::geometry::TITLE_BAR_HEIGHT;
use crate::icons::{Icon, icon};
use crate::theme::{Colors, ThemeMode, ThemeTokens, UI_METRICS, tracked_label, ui_font};
use crate::widgets::{ButtonKind, button_style};
use crate::window_chrome::{
    WindowChrome, WindowChromeAction, WindowChromeEvent, WindowChromeState,
};

pub fn app_title_bar<'a, Message>(
    title: &'a str,
    context: &'a str,
    theme: ThemeMode,
    toggle_theme: Message,
    leading_action: Option<Element<'a, Message>>,
    colors: Colors,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let theme_icon = match theme {
        ThemeMode::Dark => Icon::Appearance,
        ThemeMode::Light => Icon::Moon,
    };
    let leading = container(leading_action.unwrap_or_else(|| {
        space()
            .width(Length::Fixed(UI_METRICS.icon_button_size))
            .into()
    }))
    .width(Length::Fill)
    .align_left(Length::Fill);

    let title_view = container(tracked_label(
        title,
        13.0,
        font::Weight::Semibold,
        0.2,
        colors.text,
    ))
    .width(Length::Fixed(140.0))
    .align_x(iced::alignment::Horizontal::Center);

    let controls = row![
        text(context).size(11).color(colors.muted),
        button(icon(theme_icon, 14.0, colors.accent))
            .on_press(toggle_theme)
            .width(Length::Fixed(UI_METRICS.icon_button_size))
            .height(Length::Fixed(UI_METRICS.icon_button_size))
            .padding(0)
            .style(button_style(colors, ButtonKind::Text)),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    AppTitleBar::new(title, colors)
        .leading(leading)
        .center(title_view)
        .trailing(controls)
        .view()
}

/// Builder for NanaUI's Lilia-style application title bar.
pub struct AppTitleBar<'a, Message> {
    title: &'a str,
    tokens: ThemeTokens,
    leading: Option<Element<'a, Message>>,
    center: Option<Element<'a, Message>>,
    trailing: Option<Element<'a, Message>>,
    chrome: WindowChrome,
    maximized: bool,
    on_window_event: Option<Rc<dyn Fn(WindowChromeEvent) -> Message + 'a>>,
}

impl<'a, Message> AppTitleBar<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(title: &'a str, theme: impl Into<ThemeTokens>) -> Self {
        Self {
            title,
            tokens: theme.into(),
            leading: None,
            center: None,
            trailing: None,
            chrome: WindowChrome::custom(),
            maximized: false,
            on_window_event: None,
        }
    }

    pub fn leading(mut self, leading: impl Into<Element<'a, Message>>) -> Self {
        self.leading = Some(leading.into());
        self
    }

    pub fn center(mut self, center: impl Into<Element<'a, Message>>) -> Self {
        self.center = Some(center.into());
        self
    }

    pub fn trailing(mut self, trailing: impl Into<Element<'a, Message>>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }

    pub fn window_chrome(
        mut self,
        state: &WindowChromeState,
        on_event: impl Fn(WindowChromeEvent) -> Message + 'a,
    ) -> Self {
        self.chrome = state.chrome();
        self.maximized = state.is_maximized();
        self.on_window_event = Some(Rc::new(on_event));
        self
    }

    pub fn view(self) -> Element<'a, Message> {
        let colors = self.tokens.colors;
        let leading = self
            .leading
            .unwrap_or_else(|| space().width(Length::Shrink).into());
        let center = self.center.unwrap_or_else(|| {
            tracked_label(self.title, 13.0, font::Weight::Semibold, 0.2, colors.text).into()
        });
        let mut trailing = row![].spacing(2).align_y(Alignment::Center);
        if let Some(content) = self.trailing {
            trailing = trailing.push(content);
        }
        if self.chrome.uses_custom_controls()
            && let Some(on_event) = self.on_window_event.as_ref()
        {
            trailing = trailing
                .push(window_control_button(
                    Icon::Minimize,
                    WindowChromeAction::Minimize,
                    false,
                    self.tokens,
                    on_event,
                ))
                .push(window_control_button(
                    if self.maximized {
                        Icon::Restore
                    } else {
                        Icon::Maximize
                    },
                    WindowChromeAction::ToggleMaximize,
                    false,
                    self.tokens,
                    on_event,
                ))
                .push(window_control_button(
                    Icon::Close,
                    WindowChromeAction::Close,
                    true,
                    self.tokens,
                    on_event,
                ));
        }
        let leading = container(leading)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_left(Length::Fill)
            .align_y(iced::alignment::Vertical::Center)
            .padding(Padding {
                top: 0.0,
                right: 6.0,
                bottom: 0.0,
                left: 6.0 + self.chrome.leading_inset,
            });
        let center = container(center)
            .width(Length::Fixed(168.0))
            .height(Length::Fill)
            .padding([0.0, 14.0])
            .clip(true)
            .center(Length::Fill);
        let trailing = container(trailing)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_right(Length::Fill)
            .align_y(iced::alignment::Vertical::Center)
            .padding(Padding {
                top: 0.0,
                right: 6.0 + self.chrome.trailing_inset,
                bottom: 0.0,
                left: 6.0,
            });
        let bar = container(
            row![leading, center, trailing]
                .align_y(Alignment::Center)
                .spacing(0),
        )
        .width(Length::Fill)
        .height(Length::Fixed(TITLE_BAR_HEIGHT))
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(colors.surface)
                .color(colors.text)
        });

        let Some(on_event) = self.on_window_event else {
            return bar.into();
        };
        let on_move = on_event.clone();
        mouse_area(bar)
            .on_move(move |position| on_move(WindowChromeEvent::PointerMoved(position)))
            .on_press(on_event(WindowChromeEvent::PointerPressed))
            .on_release(on_event(WindowChromeEvent::PointerReleased))
            .on_exit(on_event(WindowChromeEvent::PointerCancelled))
            .into()
    }
}

fn window_control_button<'a, Message>(
    glyph: Icon,
    action: WindowChromeAction,
    danger: bool,
    tokens: ThemeTokens,
    on_event: &Rc<dyn Fn(WindowChromeEvent) -> Message + 'a>,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    button(icon(glyph, 14.0, tokens.colors.muted))
        .width(Length::Fixed(UI_METRICS.icon_button_size))
        .height(Length::Fixed(UI_METRICS.icon_button_size))
        .padding(0)
        .on_press(on_event(WindowChromeEvent::Action(action)))
        .style(button_style(
            tokens,
            if danger {
                ButtonKind::Danger
            } else {
                ButtonKind::Ghost
            },
        ))
        .into()
}

pub fn app_shell<'a, Message>(
    title_bar: impl Into<Element<'a, Message>>,
    workspace: impl Into<Element<'a, Message>>,
    colors: Colors,
) -> Element<'a, Message>
where
    Message: 'a,
{
    container(column![title_bar.into(), workspace.into()])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(colors.background)
                .color(colors.text)
        })
        .into()
}

pub(crate) fn section_heading<'a, Message>(
    title: &'a str,
    trailing: Option<Element<'a, Message>>,
    colors: Colors,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let mut content = row![
        text(title)
            .size(12)
            .font(ui_font(font::Weight::Bold))
            .color(colors.muted)
    ]
    .align_y(Alignment::Center)
    .spacing(8);
    if let Some(trailing) = trailing {
        content = content.push(space().width(Length::Fill)).push(trailing);
    }
    container(content)
        .height(Length::Fixed(UI_METRICS.selection_height))
        .padding([0.0, UI_METRICS.selection_padding_x])
        .align_y(iced::alignment::Vertical::Center)
        .into()
}
