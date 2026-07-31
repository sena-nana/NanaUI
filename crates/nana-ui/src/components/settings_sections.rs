use std::borrow::Cow;
use std::rc::Rc;

use iced::widget::{button, column, container, row, space, text};
use iced::{Alignment, Element, Length, Padding};

use super::{ControlSize, RangeField, SegmentedControl, SelectionOption, Switch};
use crate::icons::{Icon, disclosure_icon};
use crate::settings::{AppearanceSettings, SettingsCard, SettingsRow};
use crate::theme::{ThemeMode, ThemeTokens, UI_METRICS, ui_font};
use crate::widgets::{ButtonKind, CardKind, button_style, card_style};

/// A controlled settings card whose details can be expanded without owning
/// application state.
pub struct SettingsCollapsibleCard<'a, Message> {
    summary: Element<'a, Message>,
    details: Element<'a, Message>,
    accessory: Option<Element<'a, Message>>,
    expanded: bool,
    disabled: bool,
    on_toggle: Message,
}

impl<'a, Message> SettingsCollapsibleCard<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        summary: impl Into<Element<'a, Message>>,
        details: impl Into<Element<'a, Message>>,
        expanded: bool,
        on_toggle: Message,
    ) -> Self {
        Self {
            summary: summary.into(),
            details: details.into(),
            accessory: None,
            expanded,
            disabled: false,
            on_toggle,
        }
    }

    /// Adds an independently interactive control, such as a switch, beside
    /// the disclosure button.
    pub fn accessory(mut self, accessory: impl Into<Element<'a, Message>>) -> Self {
        self.accessory = Some(accessory.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let summary = button(self.summary)
            .width(Length::Fill)
            .padding(0)
            .on_press_maybe((!self.disabled).then_some(self.on_toggle.clone()))
            .style(button_style(tokens, ButtonKind::Text));
        let disclosure = button(disclosure_icon(
            f32::from(self.expanded),
            16.0,
            colors.muted,
        ))
        .width(Length::Fixed(ControlSize::Small.height_in(tokens.metrics)))
        .height(Length::Fixed(ControlSize::Small.height_in(tokens.metrics)))
        .padding(0)
        .on_press_maybe((!self.disabled).then_some(self.on_toggle))
        .style(button_style(tokens, ButtonKind::Ghost));
        let mut header = row![summary]
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill);
        if let Some(accessory) = self.accessory {
            header = header.push(accessory);
        }
        header = header.push(disclosure);

        let mut content = column![header].spacing(12);
        if self.expanded {
            content = content
                .push(
                    container(space())
                        .width(Length::Fill)
                        .height(Length::Fixed(1.0))
                        .style(move |_theme| {
                            iced::widget::container::Style::default().background(colors.border_soft)
                        }),
                )
                .push(self.details);
        }
        container(content)
            .width(Length::Fill)
            .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
            .style(card_style(tokens, CardKind::Surface))
            .into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppearanceEvent {
    Theme(ThemeMode),
    StandardRadius(u8),
    WorkspaceCorners(bool),
    Reset,
}

/// A reusable appearance settings section driven entirely by host-owned state.
pub struct AppearanceSection<'a, Message> {
    theme: ThemeMode,
    appearance: &'a AppearanceSettings,
    on_event: Rc<dyn Fn(AppearanceEvent) -> Message + 'a>,
}

impl<'a, Message> AppearanceSection<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        theme: ThemeMode,
        appearance: &'a AppearanceSettings,
        on_event: impl Fn(AppearanceEvent) -> Message + 'a,
    ) -> Self {
        Self {
            theme,
            appearance,
            on_event: Rc::new(on_event),
        }
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let theme_event = self.on_event.clone();
        let theme_control = SegmentedControl::new(
            self.theme,
            [
                SelectionOption::new(ThemeMode::Dark, "暗色").icon(Icon::Moon),
                SelectionOption::new(ThemeMode::Light, "浅色").icon(Icon::Appearance),
            ],
            move |value| theme_event(AppearanceEvent::Theme(value)),
        )
        .view(tokens);
        let corner_event = self.on_event.clone();
        let corner_switch = Switch::new(self.appearance.workspace_corners_enabled(), "主区域圆角")
            .on_toggle(move |enabled| corner_event(AppearanceEvent::WorkspaceCorners(enabled)))
            .view(tokens);
        let radius_event = self.on_event.clone();
        let radius = RangeField::new(
            f32::from(AppearanceSettings::MIN_STANDARD_RADIUS)
                ..=f32::from(AppearanceSettings::MAX_STANDARD_RADIUS),
            self.appearance.standard_radius(),
            move |value| radius_event(AppearanceEvent::StandardRadius(value.round() as u8)),
        )
        .unit(" px")
        .view(tokens);
        let reset = button(text("恢复默认").size(12))
            .height(Length::Fixed(ControlSize::Small.height_in(tokens.metrics)))
            .padding([0.0, UI_METRICS.control_padding_x])
            .on_press((self.on_event)(AppearanceEvent::Reset))
            .style(button_style(tokens, ButtonKind::Subtle));
        SettingsCard::new(
            "外观",
            column![
                SettingsRow::new("主题", theme_control)
                    .hint("选择应用配色，立即生效")
                    .first_in_group()
                    .divided(true)
                    .view(tokens),
                SettingsRow::new("工作区边缘", corner_switch)
                    .divided(true)
                    .view(tokens),
                SettingsRow::new("组件圆角半径", radius)
                    .divided(true)
                    .view(tokens),
                SettingsRow::new("默认样式", reset)
                    .last_in_group()
                    .view(tokens),
            ],
        )
        .view(tokens)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutMetadata<'a> {
    pub product_title: Cow<'a, str>,
    pub version: Cow<'a, str>,
    pub description: Option<Cow<'a, str>>,
}

impl<'a> AboutMetadata<'a> {
    pub fn new(product_title: impl Into<Cow<'a, str>>, version: impl Into<Cow<'a, str>>) -> Self {
        Self {
            product_title: product_title.into(),
            version: version.into(),
            description: None,
        }
    }

    pub fn description(mut self, description: impl Into<Cow<'a, str>>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// A compact metadata section with no application-specific constants.
pub struct AboutSection<'a> {
    metadata: AboutMetadata<'a>,
}

impl<'a> AboutSection<'a> {
    pub fn new(metadata: AboutMetadata<'a>) -> Self {
        Self { metadata }
    }

    pub fn view<Message: 'a>(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let colors = tokens.colors;
        let mut rows = column![
            SettingsRow::new(
                "名称",
                text(self.metadata.product_title)
                    .size(12)
                    .font(ui_font(iced::font::Weight::Medium))
                    .color(colors.text),
            )
            .first_in_group()
            .divided(true)
            .view(tokens),
            SettingsRow::new(
                "版本",
                text(self.metadata.version).size(12).color(colors.muted),
            )
            .last_in_group()
            .view(tokens),
        ];
        if let Some(description) = self.metadata.description {
            rows = rows.push(
                container(text(description).size(12).color(colors.muted))
                    .width(Length::Fill)
                    .padding(Padding {
                        top: 8.0,
                        right: 0.0,
                        bottom: 0.0,
                        left: 0.0,
                    }),
            );
        }
        SettingsCard::new("关于", rows).view(tokens)
    }
}
