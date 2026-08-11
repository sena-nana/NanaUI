use iced::widget::{button, column, container, mouse_area, row, space, text};
use iced::{Alignment, Element, Length, Padding, font};
use std::borrow::Cow;
use std::rc::Rc;

use crate::components::ControlSize;
use crate::geometry::TITLE_BAR_HEIGHT;
use crate::icons::{Icon, icon};
use crate::layout::RegionId;
use crate::sidebar::SidebarFrame;
use crate::theme::{Colors, ThemeMode, ThemeTokens, tracked_label};
use crate::widgets::{ButtonKind, button_style};
use crate::window_chrome::{
    WindowChrome, WindowChromeAction, WindowChromeEvent, WindowChromeState,
};
use crate::workspace::{WorkspaceAction, WorkspaceController, WorkspaceRegions, workspace_view};

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
            .width(Length::Fixed(ControlSize::Small.height()))
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
            .width(Length::Fixed(ControlSize::Small.height()))
            .height(Length::Fixed(ControlSize::Small.height()))
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
///
/// `title` is a [`Cow`] so Hosted `Element<'static>` callers can pass an owned
/// [`String`] without `Box::leak`.
pub struct AppTitleBar<'a, Message> {
    title: Cow<'a, str>,
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
    pub fn new(title: impl Into<Cow<'a, str>>, theme: impl Into<ThemeTokens>) -> Self {
        Self {
            title: title.into(),
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
        let titlebar = self.tokens.titlebar;
        let leading = self
            .leading
            .unwrap_or_else(|| space().width(Length::Shrink).into());
        let center = self.center.unwrap_or_else(|| {
            tracked_label(
                self.title.as_ref(),
                13.0,
                font::Weight::Semibold,
                0.2,
                colors.text,
            )
            .into()
        });
        let mut trailing = row![].spacing(2).align_y(Alignment::Center);
        if let Some(content) = self.trailing {
            trailing = trailing.push(content);
        }
        if let Some(on_event) = self.on_window_event.as_ref() {
            trailing = trailing.push(window_chrome_controls(
                self.chrome,
                self.maximized,
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
                .background(titlebar)
                .color(colors.text)
        });

        let Some(on_event) = self.on_window_event else {
            return bar.into();
        };
        let bar = window_chrome_drag_start_area(bar, &on_event);
        window_chrome_drag_tracker(bar, on_event)
    }
}

pub(crate) fn window_chrome_controls<'a, Message>(
    chrome: WindowChrome,
    maximized: bool,
    tokens: ThemeTokens,
    on_event: &Rc<dyn Fn(WindowChromeEvent) -> Message + 'a>,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let controls = row![].spacing(2).align_y(Alignment::Center);
    if !chrome.uses_custom_controls() {
        return controls.into();
    }
    controls
        .push(window_control_button(
            Icon::Minimize,
            WindowChromeAction::Minimize,
            false,
            tokens,
            on_event,
        ))
        .push(window_control_button(
            if maximized {
                Icon::Restore
            } else {
                Icon::Maximize
            },
            WindowChromeAction::ToggleMaximize,
            false,
            tokens,
            on_event,
        ))
        .push(window_control_button(
            Icon::Close,
            WindowChromeAction::Close,
            true,
            tokens,
            on_event,
        ))
        .into()
}

pub(crate) fn window_chrome_drag_start_area<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    on_event: &Rc<dyn Fn(WindowChromeEvent) -> Message + 'a>,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    mouse_area(content)
        .on_press(on_event(WindowChromeEvent::PointerPressed))
        .into()
}

pub(crate) fn window_chrome_drag_tracker<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    on_event: Rc<dyn Fn(WindowChromeEvent) -> Message + 'a>,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let on_move = on_event.clone();
    mouse_area(content)
        .on_move(move |position| on_move(WindowChromeEvent::PointerMoved(position)))
        .on_release(on_event(WindowChromeEvent::PointerReleased))
        .on_exit(on_event(WindowChromeEvent::PointerCancelled))
        .into()
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
        .width(Length::Fixed(ControlSize::Small.height_in(tokens.metrics)))
        .height(Length::Fixed(ControlSize::Small.height_in(tokens.metrics)))
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

/// A convenience composition for the common title bar + navigation +
/// workspace shape.
///
/// Takes an **owned** [`WorkspaceController`] snapshot so the resulting
/// `Element` does not borrow the live controller. That keeps Hosted
/// `Element<'static>` / retained `UserInterface` viable without
/// self-referential UI storage. Callers typically pass
/// `controller.clone()`; region mutations still go through
/// [`WorkspaceAction`] on the live controller.
pub struct DesktopShell<'a, Message, OnAction> {
    title_bar: Element<'a, Message>,
    controller: WorkspaceController,
    primary: Element<'a, Message>,
    navigation: Option<Element<'a, Message>>,
    navigation_footer: Option<Element<'a, Message>>,
    inspector: Option<Element<'a, Message>>,
    bottom: Option<Element<'a, Message>>,
    extra_regions: Vec<(RegionId, Element<'a, Message>)>,
    overlays: Vec<Element<'a, Message>>,
    on_action: OnAction,
    tokens: ThemeTokens,
}

impl<'a, Message, OnAction> DesktopShell<'a, Message, OnAction>
where
    Message: Clone + 'a,
    OnAction: Fn(WorkspaceAction) -> Message + Copy + 'a,
{
    pub fn new(
        title_bar: impl Into<Element<'a, Message>>,
        controller: WorkspaceController,
        primary: impl Into<Element<'a, Message>>,
        on_action: OnAction,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            title_bar: title_bar.into(),
            controller,
            primary: primary.into(),
            navigation: None,
            navigation_footer: None,
            inspector: None,
            bottom: None,
            extra_regions: Vec::new(),
            overlays: Vec::new(),
            on_action,
            tokens: theme.into(),
        }
    }

    pub fn navigation(mut self, content: impl Into<Element<'a, Message>>) -> Self {
        self.navigation = Some(content.into());
        self
    }

    pub fn navigation_footer(mut self, footer: impl Into<Element<'a, Message>>) -> Self {
        self.navigation_footer = Some(footer.into());
        self
    }

    pub fn inspector(mut self, content: impl Into<Element<'a, Message>>) -> Self {
        self.inspector = Some(content.into());
        self
    }

    pub fn bottom(mut self, content: impl Into<Element<'a, Message>>) -> Self {
        self.bottom = Some(content.into());
        self
    }

    pub fn region(mut self, id: RegionId, content: impl Into<Element<'a, Message>>) -> Self {
        self.extra_regions.push((id, content.into()));
        self
    }

    pub fn overlay(mut self, overlay: impl Into<Element<'a, Message>>) -> Self {
        self.overlays.push(overlay.into());
        self
    }

    pub fn view(self) -> Element<'a, Message> {
        let mut regions = WorkspaceRegions::new().with_region(RegionId::Primary, self.primary);
        if let Some(navigation) = self.navigation {
            let mut frame = SidebarFrame::new(navigation);
            if let Some(footer) = self.navigation_footer {
                frame = frame.footer(footer);
            }
            regions = regions.with_region(RegionId::Resources, frame.view(self.tokens.colors));
        }
        if let Some(inspector) = self.inspector {
            regions = regions.with_region(RegionId::Inspector, inspector);
        }
        if let Some(bottom) = self.bottom {
            regions = regions.with_region(RegionId::Diagnostics, bottom);
        }
        for (id, content) in self.extra_regions {
            regions = regions.with_region(id, content);
        }
        let workspace = workspace_view(&self.controller, regions, self.tokens, self.on_action);
        let base = app_shell(self.title_bar, workspace, self.tokens.colors);
        let mut host = crate::components::OverlayHost::new(base);
        for overlay in self.overlays {
            host = host.push(overlay);
        }
        host.view()
    }
}

/// Full-window shell used by compact popup and status windows.
pub struct PopupShell<'a, Message> {
    body: Element<'a, Message>,
    title_bar: Option<Element<'a, Message>>,
    status: bool,
}

impl<'a, Message> PopupShell<'a, Message>
where
    Message: 'a,
{
    pub fn new(body: impl Into<Element<'a, Message>>) -> Self {
        Self {
            body: body.into(),
            title_bar: None,
            status: false,
        }
    }

    pub fn title_bar(mut self, title_bar: impl Into<Element<'a, Message>>) -> Self {
        self.title_bar = Some(title_bar.into());
        self
    }

    pub fn status(mut self, status: bool) -> Self {
        self.status = status;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let colors = theme.into().colors;
        let body = container(self.body)
            .width(Length::Fill)
            .height(Length::Fill);
        let content: Element<'a, Message> = if self.status {
            body.into()
        } else {
            column![
                self.title_bar.unwrap_or_else(|| {
                    space()
                        .width(Length::Fill)
                        .height(Length::Fixed(TITLE_BAR_HEIGHT))
                        .into()
                }),
                body,
            ]
            .into()
        };
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| {
                iced::widget::container::Style::default()
                    .background(if self.status {
                        colors.background.scale_alpha(0.0)
                    } else {
                        colors.background
                    })
                    .color(colors.text)
            })
            .into()
    }
}

/// Popup-specific title bar with focus-main/new actions and native
/// minimize/close commands.
pub struct PopupTitleBarFrame<'a, Message> {
    center: Element<'a, Message>,
    on_focus_main: Message,
    on_new_item: Message,
    on_window_event: Rc<dyn Fn(WindowChromeEvent) -> Message + 'a>,
    tokens: ThemeTokens,
}

impl<'a, Message> PopupTitleBarFrame<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        center: impl Into<Element<'a, Message>>,
        on_focus_main: Message,
        on_new_item: Message,
        on_window_event: impl Fn(WindowChromeEvent) -> Message + 'a,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            center: center.into(),
            on_focus_main,
            on_new_item,
            on_window_event: Rc::new(on_window_event),
            tokens: theme.into(),
        }
    }

    pub fn view(self) -> Element<'a, Message> {
        let colors = self.tokens.colors;
        let titlebar = self.tokens.titlebar;
        let focus = popup_title_bar_button(Icon::ArrowLeft, self.on_focus_main, false, self.tokens);
        let new_item = popup_title_bar_button(Icon::Add, self.on_new_item, false, self.tokens);
        let minimize = popup_title_bar_button(
            Icon::Minimize,
            (self.on_window_event)(WindowChromeEvent::Action(WindowChromeAction::Minimize)),
            false,
            self.tokens,
        );
        let close = popup_title_bar_button(
            Icon::Close,
            (self.on_window_event)(WindowChromeEvent::Action(WindowChromeAction::Close)),
            true,
            self.tokens,
        );
        let bar = container(
            row![
                container(row![focus, new_item].spacing(2))
                    .width(Length::Fixed(72.0))
                    .padding([0.0, 6.0]),
                container(self.center)
                    .width(Length::Fill)
                    .center(Length::Fill)
                    .padding([0.0, 10.0]),
                container(row![minimize, close].spacing(2))
                    .width(Length::Fixed(72.0))
                    .align_right(Length::Fill)
                    .padding([0.0, 6.0]),
            ]
            .height(Length::Fill)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(TITLE_BAR_HEIGHT))
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(titlebar)
                .color(colors.text)
        });
        let on_move = self.on_window_event.clone();
        let on_press = self.on_window_event.clone();
        let on_release = self.on_window_event.clone();
        mouse_area(bar)
            .on_move(move |position| on_move(WindowChromeEvent::PointerMoved(position)))
            .on_press(on_press(WindowChromeEvent::PointerPressed))
            .on_release(on_release(WindowChromeEvent::PointerReleased))
            .on_exit((self.on_window_event)(WindowChromeEvent::PointerCancelled))
            .into()
    }
}

fn popup_title_bar_button<'a, Message>(
    glyph: Icon,
    message: Message,
    danger: bool,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    button(icon(glyph, 14.0, tokens.colors.muted))
        .width(Length::Fixed(ControlSize::Small.height_in(tokens.metrics)))
        .height(Length::Fixed(ControlSize::Small.height_in(tokens.metrics)))
        .padding(0)
        .on_press(message)
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
