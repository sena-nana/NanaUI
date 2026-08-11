use std::borrow::Cow;

use iced::widget::{column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Length, Padding, font};

pub use nana_ui_core::AppearanceEvent;
pub use nana_ui_core::settings::{
    AppearanceSettings, BackdropTarget, SettingsError, SettingsModel, SettingsState, SettingsTab,
    SettingsTabId, WindowMaterialMode,
};

use crate::icons::{Icon, icon};
use crate::sidebar::{SidebarFrame, SidebarRow, SidebarRowState};
use crate::theme::{ThemeTokens, UI_METRICS, tracked_label, ui_font};
use crate::widgets::{CardKind, canvas_style, card_style, scrollable_style, vertical_scrollbar};

pub fn settings_sidebar<'a, Message>(
    model: &'a SettingsModel,
    state: &'a SettingsState,
    on_back: Message,
    on_select: impl Fn(SettingsTabId) -> Message + Copy,
    theme: impl Into<ThemeTokens>,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let tokens = theme.into();
    let colors = tokens.colors;
    let back = SidebarRow::new("返回")
        .leading(icon(Icon::ArrowLeft, 15.0, colors.text))
        .on_select(on_back)
        .view(tokens);

    let mut tabs = column![].spacing(1);
    for tab in model.tabs() {
        let selected = state.active_tab() == tab.id();
        let leading: Element<'a, Message> = match tab.icon_value() {
            Some(tab_icon) => icon(
                tab_icon,
                15.0,
                if selected { colors.text } else { colors.muted },
            ),
            None => space()
                .width(Length::Fixed(15.0))
                .height(Length::Fixed(15.0))
                .into(),
        };
        tabs = tabs.push(
            SidebarRow::new(tab.label())
                .leading(leading)
                .state(if selected {
                    SidebarRowState::Active
                } else {
                    SidebarRowState::Idle
                })
                .on_select(on_select(tab.id().clone()))
                .view(tokens),
        );
    }

    SidebarFrame::new(tabs).top(back).gap(12.0).view(colors)
}

pub fn settings_page<'a, Message>(
    model: &'a SettingsModel,
    state: &'a SettingsState,
    content: impl Into<Element<'a, Message>>,
    theme: impl Into<ThemeTokens>,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let tokens = theme.into();
    let colors = tokens.colors;
    let tab = state.active_view(model);
    let content = content.into();
    if tab.full_page_value() {
        return container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(canvas_style(tokens))
            .into();
    }

    let mut page = column![].spacing(16);
    if !model.hide_header_value() {
        page = page.push(
            text(tab.label())
                .size(18)
                .font(ui_font(font::Weight::Semibold))
                .color(colors.text),
        );
    }
    page = page.push(content);

    container(
        scrollable(container(page).width(Length::Fill).padding(Padding {
            top: 20.0,
            right: 24.0,
            bottom: 24.0,
            left: 24.0,
        }))
        .direction(vertical_scrollbar())
        .style(scrollable_style(colors))
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(canvas_style(tokens))
    .into()
}

pub struct SettingsRow<'a, Message> {
    label: Cow<'a, str>,
    hint: Option<Cow<'a, str>>,
    control: Element<'a, Message>,
    stacked: bool,
    divided: bool,
    loose: bool,
    first_in_group: bool,
    last_in_group: bool,
}

impl<'a, Message> SettingsRow<'a, Message>
where
    Message: 'a,
{
    pub fn new(label: impl Into<Cow<'a, str>>, control: impl Into<Element<'a, Message>>) -> Self {
        Self {
            label: label.into(),
            hint: None,
            control: control.into(),
            stacked: false,
            divided: false,
            loose: false,
            first_in_group: false,
            last_in_group: false,
        }
    }

    pub fn hint(mut self, hint: impl Into<Cow<'a, str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn stacked(mut self, stacked: bool) -> Self {
        self.stacked = stacked;
        self
    }

    pub fn divided(mut self, divided: bool) -> Self {
        self.divided = divided;
        self
    }

    pub fn loose(mut self, loose: bool) -> Self {
        self.loose = loose;
        self
    }

    pub fn first_in_group(mut self) -> Self {
        self.first_in_group = true;
        self
    }

    pub fn last_in_group(mut self) -> Self {
        self.last_in_group = true;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let mut label = column![
            text(self.label)
                .size(13)
                .font(ui_font(font::Weight::Medium))
                .color(colors.text)
                .width(Length::Fill)
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::End)
        ]
        .spacing(2)
        .width(Length::Fill);
        if let Some(hint) = self.hint {
            label = label.push(
                text(hint)
                    .size(12)
                    .color(colors.muted)
                    .width(Length::Fill)
                    .wrapping(text::Wrapping::None)
                    .ellipsis(text::Ellipsis::End),
            );
        }

        let content: Element<'a, Message> = if self.stacked {
            column![label, self.control]
                .spacing(if self.loose { 10 } else { 6 })
                .width(Length::Fill)
                .into()
        } else {
            row![
                label.width(Length::Fill),
                container(self.control)
                    .align_right(Length::Shrink)
                    .center_y(Length::Shrink)
            ]
            .spacing(if self.loose { 14 } else { 8 })
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into()
        };

        let row = container(content).width(Length::Fill).padding(Padding {
            top: if self.first_in_group { 4.0 } else { 10.0 },
            right: 0.0,
            bottom: if self.last_in_group { 4.0 } else { 10.0 },
            left: 0.0,
        });
        if self.divided {
            let divider = container(space())
                .width(Length::Fill)
                .height(Length::Fixed(1.0))
                .style(move |_theme| container::Style::default().background(colors.border_soft));
            column![row, divider].width(Length::Fill).into()
        } else {
            row.into()
        }
    }
}

pub struct SettingsCard<'a, Message> {
    title: Cow<'a, str>,
    content: Element<'a, Message>,
}

impl<'a, Message> SettingsCard<'a, Message>
where
    Message: 'a,
{
    pub fn new(title: impl Into<Cow<'a, str>>, content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            title: title.into(),
            content: content.into(),
        }
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let title = tracked_label(
            &self.title.to_uppercase(),
            13.0,
            font::Weight::Semibold,
            0.5,
            colors.muted,
        );
        let card = container(column![title, self.content].spacing(8))
            .width(Length::Fill)
            .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
            .style(card_style(tokens, CardKind::Surface));
        container(card)
            .width(Length::Fill)
            .padding(Padding {
                top: 0.0,
                right: 0.0,
                bottom: 12.0,
                left: 0.0,
            })
            .into()
    }
}
