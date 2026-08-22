use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::icon::Icon;
use crate::theme::{ThemeMetrics, ThemeMode, UI_METRICS};

const RADIUS_STEP: f32 = 4.0;

/// Window backdrop selected by the app or Appearance settings.
///
/// `Translucent` is window alpha only. Vibrancy / Mica / Acrylic must be requested
/// explicitly; hosts must not substitute another blur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowMaterialMode {
    #[default]
    Solid,
    Translucent,
    Vibrancy,
    Mica,
    Acrylic,
}

impl WindowMaterialMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Solid => "实色",
            Self::Translucent => "透明",
            Self::Vibrancy => "Vibrancy",
            Self::Mica => "Mica",
            Self::Acrylic => "Acrylic",
        }
    }

    pub const fn wants_native(self) -> bool {
        matches!(self, Self::Vibrancy | Self::Mica | Self::Acrylic)
    }

    pub const fn wants_transparent_surface(self) -> bool {
        !matches!(self, Self::Solid)
    }
}

/// Which shell region should reveal the translucent material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackdropTarget {
    #[default]
    Sidebar,
    Main,
}

impl BackdropTarget {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sidebar => "侧边栏",
            Self::Main => "主内容区",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct AppearanceSettings {
    standard_radius: f32,
    workspace_corners_enabled: bool,
    window_material: WindowMaterialMode,
    backdrop_target: BackdropTarget,
    backdrop_opacity: f32,
    titlebar_follows_sidebar: bool,
}

impl AppearanceSettings {
    pub const MIN_STANDARD_RADIUS: u8 = 8;
    pub const MAX_STANDARD_RADIUS: u8 = 28;
    pub const MIN_BACKDROP_OPACITY: f32 = 0.28;
    pub const MAX_BACKDROP_OPACITY: f32 = 0.92;
    /// Default cover opacity when native material is active.
    ///
    /// Matches Lilia / `nanavue-components` `BACKDROP_OPACITY_DEFAULT` (0.64) and
    /// `--lilia-backdrop-opacity`. Do not silent-regress to older UI mix factors
    /// such as 0.78 (hover blending in widgets, not Appearance).
    pub const DEFAULT_BACKDROP_OPACITY: f32 = 0.64;
    /// Theme restored by [`AppearanceEvent::Reset`], matching Lilia
    /// `resetAppearanceDefaults` (`setTheme("light")`).
    ///
    /// Not [`ThemeMode::default`] (Dark): reset always returns to Light.
    pub const RESET_THEME: ThemeMode = ThemeMode::Light;

    pub fn new(standard_radius: f32) -> Self {
        Self {
            standard_radius: normalize_standard_radius(standard_radius),
            workspace_corners_enabled: true,
            window_material: WindowMaterialMode::Solid,
            backdrop_target: BackdropTarget::Sidebar,
            backdrop_opacity: Self::DEFAULT_BACKDROP_OPACITY,
            titlebar_follows_sidebar: true,
        }
    }

    pub fn standard_radius(&self) -> f32 {
        self.standard_radius
    }

    pub fn workspace_corners_enabled(&self) -> bool {
        self.workspace_corners_enabled
    }

    pub fn window_material(&self) -> WindowMaterialMode {
        self.window_material
    }

    pub fn backdrop_target(&self) -> BackdropTarget {
        self.backdrop_target
    }

    pub fn backdrop_opacity(&self) -> f32 {
        self.backdrop_opacity
    }

    pub fn titlebar_follows_sidebar(&self) -> bool {
        self.titlebar_follows_sidebar
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

    pub fn set_workspace_corners_enabled(&mut self, enabled: bool) -> bool {
        if self.workspace_corners_enabled == enabled {
            return false;
        }
        self.workspace_corners_enabled = enabled;
        true
    }

    pub fn set_window_material(&mut self, mode: WindowMaterialMode) -> bool {
        if self.window_material == mode {
            return false;
        }
        self.window_material = mode;
        true
    }

    pub fn set_backdrop_target(&mut self, target: BackdropTarget) -> bool {
        if self.backdrop_target == target {
            return false;
        }
        self.backdrop_target = target;
        true
    }

    pub fn set_backdrop_opacity(&mut self, opacity: f32) -> bool {
        let opacity = normalize_backdrop_opacity(opacity);
        if (self.backdrop_opacity - opacity).abs() < f32::EPSILON {
            return false;
        }
        self.backdrop_opacity = opacity;
        true
    }

    pub fn set_titlebar_follows_sidebar(&mut self, enabled: bool) -> bool {
        if self.titlebar_follows_sidebar == enabled {
            return false;
        }
        self.titlebar_follows_sidebar = enabled;
        true
    }

    pub fn reset(&mut self) -> bool {
        let defaults = Self::default();
        if self == &defaults {
            return false;
        }
        *self = defaults;
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
            workspace_corners_enabled: bool,
            #[serde(default)]
            window_material: WindowMaterialMode,
            #[serde(default)]
            backdrop_target: BackdropTarget,
            #[serde(default = "default_backdrop_opacity")]
            backdrop_opacity: f32,
            #[serde(default = "default_titlebar_follows_sidebar")]
            titlebar_follows_sidebar: bool,
        }

        let persisted = PersistedAppearance::deserialize(deserializer)?;
        Ok(Self {
            standard_radius: normalize_standard_radius(persisted.standard_radius),
            workspace_corners_enabled: persisted.workspace_corners_enabled,
            window_material: persisted.window_material,
            backdrop_target: persisted.backdrop_target,
            backdrop_opacity: normalize_backdrop_opacity(persisted.backdrop_opacity),
            titlebar_follows_sidebar: persisted.titlebar_follows_sidebar,
        })
    }
}

fn default_backdrop_opacity() -> f32 {
    AppearanceSettings::DEFAULT_BACKDROP_OPACITY
}

fn default_titlebar_follows_sidebar() -> bool {
    true
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

fn normalize_backdrop_opacity(opacity: f32) -> f32 {
    if opacity.is_finite() {
        opacity.clamp(
            AppearanceSettings::MIN_BACKDROP_OPACITY,
            AppearanceSettings::MAX_BACKDROP_OPACITY,
        )
    } else {
        AppearanceSettings::DEFAULT_BACKDROP_OPACITY
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

/// Appearance settings events shared by backend-neutral hosts.
///
/// Kept next to settings data so Vue and Runtime can share the same contract
/// without depending on settings-section widgets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppearanceEvent {
    Theme(ThemeMode),
    StandardRadius(u8),
    WorkspaceCorners(bool),
    WindowMaterial(WindowMaterialMode),
    BackdropTarget(BackdropTarget),
    BackdropOpacity(f32),
    TitlebarFollowsSidebar(bool),
    /// Restore appearance defaults **and** theme.
    ///
    /// Matches Lilia / `nanavue-components` `resetAppearanceDefaults`: hosts must
    /// call [`AppearanceSettings::reset`] and set theme to
    /// [`AppearanceSettings::RESET_THEME`] (`ThemeMode::Light`).
    /// `AppearanceSettings` itself does not store theme; theme lives on the host.
    Reset,
}

#[cfg(test)]
mod tests {
    use super::{
        AppearanceSettings, BackdropTarget, SettingsError, SettingsModel, SettingsState,
        SettingsTab, SettingsTabId, WindowMaterialMode,
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
        assert!(appearance.workspace_corners_enabled());
        assert_eq!(appearance.window_material(), WindowMaterialMode::Solid);
        assert!(!WindowMaterialMode::Translucent.wants_native());
        assert!(WindowMaterialMode::Mica.wants_native());
        assert!(WindowMaterialMode::Translucent.wants_transparent_surface());
        assert_eq!(appearance.backdrop_target(), BackdropTarget::Sidebar);
        assert!(
            (appearance.backdrop_opacity() - AppearanceSettings::DEFAULT_BACKDROP_OPACITY).abs()
                < f32::EPSILON
        );
        assert!(appearance.titlebar_follows_sidebar());
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

        assert!(appearance.set_workspace_corners_enabled(false));
        assert!(appearance.set_window_material(WindowMaterialMode::Translucent));
        assert!(appearance.set_backdrop_target(BackdropTarget::Main));
        assert!(appearance.set_backdrop_opacity(0.5));
        assert!(appearance.set_titlebar_follows_sidebar(false));
        assert!(appearance.set_standard_radius(8.0));
        assert!(appearance.reset());
        assert_eq!(appearance.metrics(), UI_METRICS);
        assert!(appearance.workspace_corners_enabled());
        assert_eq!(appearance.window_material(), WindowMaterialMode::Solid);
        assert_eq!(appearance.backdrop_target(), BackdropTarget::Sidebar);
        assert!(appearance.titlebar_follows_sidebar());
        assert!(!appearance.reset());
        assert_eq!(
            AppearanceSettings::RESET_THEME,
            crate::theme::ThemeMode::Light,
            "Reset must restore Light to match Lilia resetAppearanceDefaults"
        );
    }

    #[test]
    fn appearance_round_trip_normalizes_persisted_values() {
        let persisted = AppearanceSettings {
            standard_radius: -4.0,
            workspace_corners_enabled: false,
            window_material: WindowMaterialMode::Translucent,
            backdrop_target: BackdropTarget::Main,
            backdrop_opacity: 2.0,
            titlebar_follows_sidebar: false,
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
        assert!(!restored.workspace_corners_enabled());
        assert_eq!(restored.window_material(), WindowMaterialMode::Translucent);
        assert_eq!(restored.backdrop_target(), BackdropTarget::Main);
        assert!(
            (restored.backdrop_opacity() - AppearanceSettings::MAX_BACKDROP_OPACITY).abs()
                < f32::EPSILON
        );
        assert!(!restored.titlebar_follows_sidebar());

        let legacy = serde_json::json!({ "metrics": UI_METRICS });
        assert!(
            restored.restore_json(&legacy.to_string()).is_err(),
            "legacy metrics format must be rejected"
        );
        assert!(
            restored
                .restore_json(r#"{"standard_radius":10.0}"#)
                .is_err(),
            "appearance settings without the corner switch must be rejected"
        );

        restored
            .restore_json(r#"{"standard_radius":12.0,"workspace_corners_enabled":true}"#)
            .expect("legacy appearance without material fields restores with defaults");
        assert_eq!(restored.window_material(), WindowMaterialMode::Solid);
        assert_eq!(restored.backdrop_target(), BackdropTarget::Sidebar);
        assert!(
            (restored.backdrop_opacity() - AppearanceSettings::DEFAULT_BACKDROP_OPACITY).abs()
                < f32::EPSILON
        );
        assert!(restored.titlebar_follows_sidebar());
    }
}
