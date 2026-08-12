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

pub use nana_ui_core::AppearanceEvent;

/// A reusable appearance settings section driven entirely by host-owned state.
pub struct AppearanceSection<'a, Message> {
    theme: ThemeMode,
    appearance: &'a AppearanceSettings,
    on_event: Rc<dyn Fn(AppearanceEvent) -> Message + 'a>,
    material_status: Option<Cow<'a, str>>,
    platform_hint: Option<Cow<'a, str>>,
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
            material_status: None,
            platform_hint: None,
        }
    }

    /// Shows the latest [`nana_window::MaterialOutcome`] status from the host.
    pub fn material_status(mut self, status: impl Into<Cow<'a, str>>) -> Self {
        self.material_status = Some(status.into());
        self
    }

    /// Optional platform capability hint (no window handle required).
    pub fn platform_hint(mut self, hint: impl Into<Cow<'a, str>>) -> Self {
        self.platform_hint = Some(hint.into());
        self
    }

    pub fn view(self, theme: impl Into<ThemeTokens>) -> Element<'a, Message> {
        let tokens = theme.into();
        let solid_mode = matches!(
            self.appearance.window_material(),
            nana_ui_core::WindowMaterialMode::Solid
        );
        let titlebar_follow_disabled = solid_mode
            || !matches!(
                self.appearance.backdrop_target(),
                nana_ui_core::BackdropTarget::Sidebar
            );

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

        let material_event = self.on_event.clone();
        let material_control = SegmentedControl::new(
            self.appearance.window_material(),
            [
                SelectionOption::new(nana_ui_core::WindowMaterialMode::Solid, "实色"),
                SelectionOption::new(nana_ui_core::WindowMaterialMode::Translucent, "透明"),
            ],
            move |value| material_event(AppearanceEvent::WindowMaterial(value)),
        )
        .view(tokens);

        let target_event = self.on_event.clone();
        let target_control = SegmentedControl::new(
            self.appearance.backdrop_target(),
            [
                SelectionOption::new(nana_ui_core::BackdropTarget::Sidebar, "侧边栏")
                    .disabled(solid_mode),
                SelectionOption::new(nana_ui_core::BackdropTarget::Main, "主内容区")
                    .disabled(solid_mode),
            ],
            move |value| target_event(AppearanceEvent::BackdropTarget(value)),
        )
        .view(tokens);

        let titlebar_event = self.on_event.clone();
        let titlebar_switch = Switch::new(self.appearance.titlebar_follows_sidebar(), "")
            .disabled(titlebar_follow_disabled)
            .on_toggle(move |enabled| {
                titlebar_event(AppearanceEvent::TitlebarFollowsSidebar(enabled))
            })
            .view(tokens);

        let opacity_event = self.on_event.clone();
        let opacity_percent = (self.appearance.backdrop_opacity() * 100.0).round();
        let opacity = if solid_mode {
            text(format!("{opacity_percent:.0}%"))
                .size(12)
                .color(tokens.colors.muted)
                .into()
        } else {
            RangeField::new(
                (AppearanceSettings::MIN_BACKDROP_OPACITY * 100.0)
                    ..=(AppearanceSettings::MAX_BACKDROP_OPACITY * 100.0),
                opacity_percent,
                move |value| opacity_event(AppearanceEvent::BackdropOpacity(value / 100.0)),
            )
            .unit("%")
            .view(tokens)
        };

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

        let mut rows = column![
            SettingsRow::new("主题", theme_control)
                .hint("选择应用配色，立即生效")
                .first_in_group()
                .divided(true)
                .view(tokens),
            SettingsRow::new("窗口材质", material_control)
                .hint(
                    self.platform_hint
                        .clone()
                        .unwrap_or(Cow::Borrowed("选择窗口使用的透明材质或实色背景。")),
                )
                .divided(true)
                .view(tokens),
        ];

        if let Some(status) = self.material_status {
            rows = rows.push(
                SettingsRow::new("材质状态", text(status).size(12).color(tokens.colors.muted))
                    .hint("由宿主经 nana-window 应用后回报；失败时保持可读实色。")
                    .divided(true)
                    .view(tokens),
            );
        }

        rows = rows.push(
            SettingsRow::new("透明区域", target_control)
                .hint(if solid_mode {
                    "实色模式不显示透明区域；切回透明材质后会恢复当前选择。"
                } else {
                    "选择侧边栏或主内容区显示透明材质。"
                })
                .divided(true)
                .view(tokens),
        );
        rows = rows.push(
            SettingsRow::new("标题栏跟随侧边栏透明", titlebar_switch)
                .hint(if titlebar_follow_disabled {
                    "仅在侧边栏使用透明材质时生效；当前选择会保留。"
                } else {
                    "侧边栏透明时，整个标题栏同步显示透明材质。"
                })
                .divided(true)
                .view(tokens),
        );
        rows = rows.push(
            SettingsRow::new("材质不透明度", opacity)
                .hint(if solid_mode {
                    "实色模式不使用透明度；切回透明材质后会恢复当前数值。"
                } else {
                    "调节透明区域材质的前景色覆盖程度。"
                })
                .divided(true)
                .view(tokens),
        );
        rows = rows.push(
            SettingsRow::new("工作区边缘", corner_switch)
                .divided(true)
                .view(tokens),
        );
        rows = rows.push(
            SettingsRow::new("组件圆角半径", radius)
                .divided(true)
                .view(tokens),
        );
        rows = rows.push(
            SettingsRow::new("默认样式", reset)
                .hint("恢复主题、材质与圆角默认值。")
                .last_in_group()
                .view(tokens),
        );

        SettingsCard::new("外观", rows).view(tokens)
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
