use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;

use iced::widget::{column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Length, Padding, font};
use serde::{Deserialize, Deserializer, Serialize};

use crate::icons::{Icon, icon};
use crate::sidebar::{SidebarFrame, SidebarRow, SidebarRowState};
use crate::theme::{ThemeMetrics, ThemeTokens, UI_METRICS, tracked_label, ui_font};
use crate::widgets::{CardKind, canvas_style, card_style, scrollable_style, vertical_scrollbar};

const RADIUS_STEP: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct AppearanceSettings {
    standard_radius: f32,
}

impl AppearanceSettings {
    pub const MIN_STANDARD_RADIUS: u8 = 8;
    pub const MAX_STANDARD_RADIUS: u8 = 28;

    pub fn new(standard_radius: f32) -> Self {
        Self {
            standard_radius: normalize_standard_radius(standard_radius),
        }
    }

    pub fn standard_radius(&self) -> f32 {
        self.standard_radius
    }

    pub fn metrics(&self) -> ThemeMetrics {
        ThemeMetrics {
            radius_xs: self.standard_radius - RADIUS_STEP * 2.0,
            radius_sm: self.standard_radius - RADIUS_STEP,
            radius_md: self.standard_radius,
            radius_lg: self.standard_radius + RADIUS_STEP,
            ..UI_METRICS
        }
    }

    pub fn set_standard_radius(&mut self, standard_radius: f32) -> bool {
        let standard_radius = normalize_standard_radius(standard_radius);
        if (self.standard_radius - standard_radius).abs() < f32::EPSILON {
            return false;
        }
        self.standard_radius = standard_radius;
        true
    }

    pub fn reset(&mut self) -> bool {
        if self.standard_radius == UI_METRICS.radius_md {
            return false;
        }
        self.standard_radius = UI_METRICS.radius_md;
        true
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn restore_json(&mut self, value: &str) -> Result<(), serde_json::Error> {
        *self = serde_json::from_str(value)?;
        Ok(())
    }
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self::new(UI_METRICS.radius_md)
    }
}

impl<'de> Deserialize<'de> for AppearanceSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct PersistedAppearance {
            standard_radius: f32,
        }

        let persisted = PersistedAppearance::deserialize(deserializer)?;
        Ok(Self::new(persisted.standard_radius))
    }
}

fn normalize_standard_radius(radius: f32) -> f32 {
    if radius.is_finite() {
        radius.clamp(
            f32::from(AppearanceSettings::MIN_STANDARD_RADIUS),
            f32::from(AppearanceSettings::MAX_STANDARD_RADIUS),
        )
    } else {
        UI_METRICS.radius_md
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SettingsTabId(String);

impl SettingsTabId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SettingsTabId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for SettingsTabId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SettingsTabId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsTab {
    id: SettingsTabId,
    label: String,
    icon: Option<Icon>,
    full_page: bool,
}

impl SettingsTab {
    pub fn new(id: impl Into<SettingsTabId>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            full_page: false,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn full_page(mut self, full_page: bool) -> Self {
        self.full_page = full_page;
        self
    }

    pub fn id(&self) -> &SettingsTabId {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn icon_value(&self) -> Option<Icon> {
        self.icon
    }

    pub fn full_page_value(&self) -> bool {
        self.full_page
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    EmptyTabs,
    DuplicateTab(SettingsTabId),
    UnknownDefault(SettingsTabId),
    UnknownAliasTarget(SettingsTabId),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTabs => formatter.write_str("settings requires at least one tab"),
            Self::DuplicateTab(id) => write!(formatter, "settings tab `{id}` is duplicated"),
            Self::UnknownDefault(id) => write!(formatter, "default settings tab `{id}` is unknown"),
            Self::UnknownAliasTarget(id) => {
                write!(formatter, "settings alias target `{id}` is unknown")
            }
        }
    }
}

impl std::error::Error for SettingsError {}

#[derive(Debug, Clone)]
pub struct SettingsModel {
    tabs: Vec<SettingsTab>,
    default_tab: SettingsTabId,
    aliases: HashMap<SettingsTabId, SettingsTabId>,
    hide_header: bool,
}

impl SettingsModel {
    pub fn new(
        default_tab: impl Into<SettingsTabId>,
        tabs: impl IntoIterator<Item = SettingsTab>,
    ) -> Result<Self, SettingsError> {
        let tabs: Vec<_> = tabs.into_iter().collect();
        if tabs.is_empty() {
            return Err(SettingsError::EmptyTabs);
        }
        let mut ids = HashSet::new();
        for tab in &tabs {
            if !ids.insert(tab.id.clone()) {
                return Err(SettingsError::DuplicateTab(tab.id.clone()));
            }
        }
        let default_tab = default_tab.into();
        if !ids.contains(&default_tab) {
            return Err(SettingsError::UnknownDefault(default_tab));
        }
        Ok(Self {
            tabs,
            default_tab,
            aliases: HashMap::new(),
            hide_header: false,
        })
    }

    pub fn with_alias(
        mut self,
        alias: impl Into<SettingsTabId>,
        target: impl Into<SettingsTabId>,
    ) -> Result<Self, SettingsError> {
        let target = target.into();
        if self.tab(&target).is_none() {
            return Err(SettingsError::UnknownAliasTarget(target));
        }
        self.aliases.insert(alias.into(), target);
        Ok(self)
    }

    pub fn hide_header(mut self, hide_header: bool) -> Self {
        self.hide_header = hide_header;
        self
    }

    pub fn tabs(&self) -> &[SettingsTab] {
        &self.tabs
    }

    pub fn default_tab(&self) -> &SettingsTabId {
        &self.default_tab
    }

    pub fn hide_header_value(&self) -> bool {
        self.hide_header
    }

    pub fn tab(&self, id: &SettingsTabId) -> Option<&SettingsTab> {
        self.tabs.iter().find(|tab| &tab.id == id)
    }

    pub fn normalize(&self, id: &SettingsTabId) -> SettingsTabId {
        let resolved = self.aliases.get(id).unwrap_or(id);
        if self.tab(resolved).is_some() {
            resolved.clone()
        } else {
            self.default_tab.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsState {
    active_tab: SettingsTabId,
}

impl SettingsState {
    pub fn new(model: &SettingsModel) -> Self {
        Self {
            active_tab: model.default_tab.clone(),
        }
    }

    pub fn active_tab(&self) -> &SettingsTabId {
        &self.active_tab
    }

    pub fn active_view<'a>(&self, model: &'a SettingsModel) -> &'a SettingsTab {
        let normalized = model.normalize(&self.active_tab);
        model
            .tab(&normalized)
            .expect("normalized settings tab must exist")
    }

    pub fn select(&mut self, model: &SettingsModel, id: &SettingsTabId) -> bool {
        let normalized = model.normalize(id);
        let changed = self.active_tab != normalized;
        self.active_tab = normalized;
        changed
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn restore_json(
        &mut self,
        model: &SettingsModel,
        value: &str,
    ) -> Result<(), serde_json::Error> {
        let restored: Self = serde_json::from_str(value)?;
        self.active_tab = model.normalize(&restored.active_tab);
        Ok(())
    }
}

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
        ]
        .spacing(2);
        if let Some(hint) = self.hint {
            label = label.push(text(hint).size(12).color(colors.muted));
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

#[cfg(test)]
mod tests {
    use super::{
        AppearanceSettings, SettingsError, SettingsModel, SettingsState, SettingsTab, SettingsTabId,
    };
    use crate::theme::UI_METRICS;

    fn model() -> SettingsModel {
        SettingsModel::new(
            "appearance",
            [
                SettingsTab::new("appearance", "外观"),
                SettingsTab::new("workspace", "工作区"),
            ],
        )
        .expect("valid settings model")
        .with_alias("layout", "workspace")
        .expect("valid settings alias")
    }

    #[test]
    fn settings_model_rejects_invalid_navigation_contracts() {
        assert_eq!(
            SettingsModel::new("appearance", []).expect_err("empty tabs"),
            SettingsError::EmptyTabs
        );
        assert_eq!(
            SettingsModel::new(
                "appearance",
                [
                    SettingsTab::new("appearance", "外观"),
                    SettingsTab::new("appearance", "重复"),
                ],
            )
            .expect_err("duplicate tab"),
            SettingsError::DuplicateTab(SettingsTabId::from("appearance"))
        );
        assert_eq!(
            SettingsModel::new("missing", [SettingsTab::new("appearance", "外观")])
                .expect_err("unknown default"),
            SettingsError::UnknownDefault(SettingsTabId::from("missing"))
        );
        assert_eq!(
            SettingsModel::new("appearance", [SettingsTab::new("appearance", "外观")])
                .expect("base model")
                .with_alias("legacy", "removed")
                .expect_err("unknown alias target"),
            SettingsError::UnknownAliasTarget(SettingsTabId::from("removed"))
        );
    }

    #[test]
    fn settings_state_resolves_aliases_and_unknown_values() {
        let model = model();
        let mut state = SettingsState::new(&model);

        assert!(state.select(&model, &SettingsTabId::from("layout")));
        assert_eq!(state.active_tab().as_str(), "workspace");

        assert!(state.select(&model, &SettingsTabId::from("removed")));
        assert_eq!(state.active_tab().as_str(), "appearance");
    }

    #[test]
    fn settings_state_round_trip_normalizes_persisted_keys() {
        let model = model();
        let mut state = SettingsState::new(&model);
        state.select(&model, &SettingsTabId::from("workspace"));
        let encoded = state.to_json().expect("settings serialize");

        let mut restored = SettingsState::new(&model);
        restored
            .restore_json(&model, &encoded)
            .expect("settings restore");
        assert_eq!(restored, state);

        restored
            .restore_json(&model, r#"{"active_tab":"layout"}"#)
            .expect("legacy alias restores");
        assert_eq!(restored.active_tab().as_str(), "workspace");

        restored
            .restore_json(&model, r#"{"active_tab":"removed"}"#)
            .expect("removed tab restores");
        assert_eq!(restored.active_tab().as_str(), "appearance");
    }

    #[test]
    fn standard_radius_derives_the_full_scale_and_can_be_reset() {
        let mut appearance = AppearanceSettings::default();

        assert_eq!(appearance.standard_radius(), 10.0);
        assert_eq!(appearance.metrics(), UI_METRICS);

        assert!(appearance.set_standard_radius(24.0));
        assert_eq!(appearance.standard_radius(), 24.0);
        assert_eq!(appearance.metrics().radius_xs, 16.0);
        assert_eq!(appearance.metrics().radius_sm, 20.0);
        assert_eq!(appearance.metrics().radius_md, 24.0);
        assert_eq!(appearance.metrics().radius_lg, 28.0);

        assert!(appearance.set_standard_radius(80.0));
        assert_eq!(appearance.standard_radius(), 28.0);
        assert!(appearance.set_standard_radius(f32::NAN));
        assert_eq!(appearance.standard_radius(), UI_METRICS.radius_md);

        assert!(!appearance.reset());
        assert!(appearance.set_standard_radius(8.0));
        assert!(appearance.reset());
        assert_eq!(appearance.metrics(), UI_METRICS);
        assert!(!appearance.reset());
    }

    #[test]
    fn appearance_round_trip_normalizes_persisted_values() {
        let persisted = AppearanceSettings {
            standard_radius: -4.0,
        };
        let encoded = persisted.to_json().expect("appearance serializes");
        let mut restored = AppearanceSettings::default();
        restored
            .restore_json(&encoded)
            .expect("appearance restores");

        assert_eq!(restored.standard_radius(), 8.0);
        assert_eq!(restored.metrics().radius_xs, 0.0);
        assert_eq!(restored.metrics().radius_sm, 4.0);
        assert_eq!(restored.metrics().radius_md, 8.0);
        assert_eq!(restored.metrics().radius_lg, 12.0);

        let legacy = serde_json::json!({ "metrics": UI_METRICS });
        assert!(
            restored.restore_json(&legacy.to_string()).is_err(),
            "legacy metrics format must be rejected"
        );
    }
}
