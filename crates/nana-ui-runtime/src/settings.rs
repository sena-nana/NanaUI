use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, AppearanceEvent, AppearanceSettings, CardKind, FlexDirection, LengthSpec,
    SemanticColorRole, ThemeMode,
};

use crate::view_components::{Card, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, InteractionStyle,
    MutationQueue, NodeKind, NodeStyle, SemanticPaint, StableNodeId, TextContent, UiWorld,
};

const ROW_PADDING_Y: f32 = 10.0;
const ROW_GROUP_PADDING_Y: f32 = 4.0;
const ROW_STACK_GAP: f32 = 6.0;
const ROW_STACK_GAP_LOOSE: f32 = 10.0;
const ROW_INLINE_GAP: f32 = 8.0;
const ROW_INLINE_GAP_LOOSE: f32 = 14.0;
const CARD_TRAILING_GAP: f32 = 12.0;

/// Non-interactive chrome wrapping an application-owned control child.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsRow {
    pub label: Arc<str>,
    pub hint: Option<Arc<str>>,
    pub stacked: bool,
    pub divided: bool,
    pub loose: bool,
    pub first_in_group: bool,
    pub last_in_group: bool,
    pub control: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl SettingsRow {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            hint: None,
            stacked: false,
            divided: false,
            loose: false,
            first_in_group: false,
            last_in_group: false,
            control: None,
            style: NodeStyle::default(),
        }
    }

    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
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

    pub fn control_child(mut self, control: StableNodeId) -> Self {
        self.control = Some(control);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn gap(&self) -> f32 {
        match (self.stacked, self.loose) {
            (true, true) => ROW_STACK_GAP_LOOSE,
            (true, false) => ROW_STACK_GAP,
            (false, true) => ROW_INLINE_GAP_LOOSE,
            (false, false) => ROW_INLINE_GAP,
        }
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        style.background = None;
        style.border = self.divided.then_some(SemanticColorRole::BorderSoft);
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Percent(100.0));
        layout.direction = Some(if self.stacked {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        });
        layout.align_items = if self.stacked {
            AlignSpec::Stretch
        } else {
            AlignSpec::Center
        };
        layout.gap = Some(LengthSpec::Px(self.gap()));
        layout.padding_top = Some(LengthSpec::Px(if self.first_in_group {
            ROW_GROUP_PADDING_Y
        } else {
            ROW_PADDING_Y
        }));
        layout.padding_bottom = Some(LengthSpec::Px(if self.last_in_group {
            ROW_GROUP_PADDING_Y
        } else {
            ROW_PADDING_Y
        }));
        layout.border_width = Some(if self.divided { 1.0 } else { 0.0 });
        layout.font_size = Some(13.0);
        layout.font_weight = Some(500);
        style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
        style
    }
}

impl ComponentView for SettingsRow {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-row".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        // Vue hosts already have a label child. Rust-only rows keep the label as
        // node text so Scene can still show it without a new paint primitive.
        let visible_label = if self.control.is_some()
            && world.node(id).is_some_and(|node| !node.children.is_empty())
        {
            ""
        } else {
            self.label.as_ref()
        };
        if world.text(id) != Some(visible_label) {
            mutations.set_text(
                id,
                TextContent {
                    value: visible_label.to_owned(),
                },
            );
        }
        if world.standard_visual(id).is_some() {
            mutations.set_standard_visual(id, None);
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::clone(&self.label)),
                value: self.hint.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Surface grouping for settings rows.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsCard {
    pub title: Arc<str>,
    pub style: NodeStyle,
}

impl SettingsCard {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            style: NodeStyle::default(),
        }
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn card(&self) -> Card {
        let mut card = Card::new().kind(CardKind::Surface);
        if !self.title.is_empty() {
            card = card.title(Arc::clone(&self.title));
        }
        let mut style = card.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.margin_bottom = Some(
            self.style
                .layout
                .margin_bottom
                .or(layout.margin_bottom)
                .unwrap_or(LengthSpec::Px(CARD_TRAILING_GAP)),
        );
        card.style = style;
        card
    }
}

impl ComponentView for SettingsCard {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-card".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.card().project(id, world, mutations);
    }
}

/// Controlled collapse. Header is keyboard-activatable; values stay host-owned.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsCollapsibleCard {
    pub summary: Option<StableNodeId>,
    pub details: Option<StableNodeId>,
    pub accessory: Option<StableNodeId>,
    pub expanded: bool,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl SettingsCollapsibleCard {
    pub fn new(expanded: bool) -> Self {
        Self {
            summary: None,
            details: None,
            accessory: None,
            expanded,
            disabled: false,
            style: NodeStyle::default(),
        }
    }

    pub fn summary(mut self, summary: StableNodeId) -> Self {
        self.summary = Some(summary);
        self
    }

    pub fn details(mut self, details: StableNodeId) -> Self {
        self.details = Some(details);
        self
    }

    pub fn accessory(mut self, accessory: StableNodeId) -> Self {
        self.accessory = Some(accessory);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn card(&self) -> Card {
        let mut card = Card::new().kind(CardKind::Surface);
        let mut style = card.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.direction = Some(FlexDirection::Column);
        layout.gap = Some(LengthSpec::Px(12.0));
        style.interaction = if self.disabled {
            InteractionStyle {
                disabled: SemanticPaint {
                    foreground: Some(SemanticColorRole::Faint),
                    ..SemanticPaint::default()
                },
                ..InteractionStyle::default()
            }
        } else {
            InteractionStyle {
                hovered: SemanticPaint {
                    background: Some(SemanticColorRole::Hover),
                    ..SemanticPaint::default()
                },
                pressed: SemanticPaint {
                    background: Some(SemanticColorRole::Active),
                    ..SemanticPaint::default()
                },
                focused: SemanticPaint {
                    border: Some(SemanticColorRole::Accent),
                    ..SemanticPaint::default()
                },
                ..InteractionStyle::default()
            }
        };
        card.style = style;
        card
    }
}

impl ComponentView for SettingsCollapsibleCard {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-collapsible-card".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.card().project(id, world, mutations);
        project_common(
            id,
            world,
            mutations,
            &self.card().style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                selected: Some(self.expanded),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Host-owned appearance snapshot. Events stay [`AppearanceEvent`]; values stay outside NanaUI.
#[derive(Debug, Clone, PartialEq)]
pub struct AppearanceSection {
    pub theme: ThemeMode,
    pub appearance: AppearanceSettings,
    pub material_status: Option<Arc<str>>,
    pub platform_hint: Option<Arc<str>>,
}

impl AppearanceSection {
    pub fn new(theme: ThemeMode, appearance: AppearanceSettings) -> Self {
        Self {
            theme,
            appearance,
            material_status: None,
            platform_hint: None,
        }
    }

    pub fn material_status(mut self, status: impl Into<Arc<str>>) -> Self {
        self.material_status = Some(status.into());
        self
    }

    pub fn platform_hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        self.platform_hint = Some(hint.into());
        self
    }
}

impl ComponentView for AppearanceSection {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "appearance-section".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        SettingsCard::new("外观").project(id, world, mutations);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutMetadata {
    pub product_title: Arc<str>,
    pub version: Arc<str>,
    pub description: Option<Arc<str>>,
}

impl AboutMetadata {
    pub fn new(product_title: impl Into<Arc<str>>, version: impl Into<Arc<str>>) -> Self {
        Self {
            product_title: product_title.into(),
            version: version.into(),
            description: None,
        }
    }

    pub fn description(mut self, description: impl Into<Arc<str>>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Injected metadata only. No application constants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutSection {
    pub metadata: AboutMetadata,
}

impl AboutSection {
    pub fn new(metadata: AboutMetadata) -> Self {
        Self { metadata }
    }
}

impl ComponentView for AboutSection {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "about-section".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        SettingsCard::new("关于").project(id, world, mutations);
        let accessibility = AccessibilityState {
            role: AccessibilityRole::Generic,
            label: Some(Arc::clone(&self.metadata.product_title)),
            value: Some(Arc::clone(&self.metadata.version)),
            description: self.metadata.description.clone(),
            ..AccessibilityState::default()
        };
        if world.accessibility(id) != Some(&accessibility) {
            mutations.set_accessibility(id, accessibility);
        }
    }
}

/// Apply a host-owned appearance event. Theme is not stored on [`AppearanceSettings`].
pub fn apply_appearance_event(
    theme: &mut ThemeMode,
    appearance: &mut AppearanceSettings,
    event: AppearanceEvent,
) -> bool {
    match event {
        AppearanceEvent::Theme(next) => {
            if *theme == next {
                return false;
            }
            *theme = next;
            true
        }
        AppearanceEvent::StandardRadius(radius) => {
            appearance.set_standard_radius(f32::from(radius))
        }
        AppearanceEvent::WorkspaceCorners(enabled) => {
            appearance.set_workspace_corners_enabled(enabled)
        }
        AppearanceEvent::WindowMaterial(mode) => appearance.set_window_material(mode),
        AppearanceEvent::BackdropTarget(target) => appearance.set_backdrop_target(target),
        AppearanceEvent::BackdropOpacity(opacity) => appearance.set_backdrop_opacity(opacity),
        AppearanceEvent::TitlebarFollowsSidebar(enabled) => {
            appearance.set_titlebar_follows_sidebar(enabled)
        }
        AppearanceEvent::Reset => {
            let theme_changed = *theme != AppearanceSettings::RESET_THEME;
            *theme = AppearanceSettings::RESET_THEME;
            appearance.reset() || theme_changed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, StandardVisual};
    use nana_ui_core::{BackdropTarget, WindowMaterialMode};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn settings_row_keeps_slots_and_chrome_flags() {
        let mut context = AppContext::new();
        let control = context
            .create_component(document(), crate::Switch::new("跟随", true))
            .unwrap();
        let row = context
            .create_component(
                document(),
                SettingsRow::new("标题栏跟随侧边栏透明")
                    .hint("仅在侧边栏使用透明材质时生效")
                    .stacked(true)
                    .divided(true)
                    .loose(true)
                    .first_in_group()
                    .last_in_group()
                    .control_child(control.stable_id()),
            )
            .unwrap();
        context.append_child(row, control).unwrap();
        context.update_component(row, |_, _| {}).unwrap();
        let id = row.stable_id();
        assert_eq!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element {
                tag: "settings-row".into(),
            }
        );
        assert_eq!(context.world().standard_visual(id), None);
        assert_eq!(context.world().text(id), Some(""));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.layout.direction, Some(FlexDirection::Column));
        assert_eq!(style.layout.gap, Some(LengthSpec::Px(ROW_STACK_GAP_LOOSE)));
        assert_eq!(
            style.layout.padding_top,
            Some(LengthSpec::Px(ROW_GROUP_PADDING_Y))
        );
        assert_eq!(style.layout.border_width, Some(1.0));
        assert_eq!(style.border, Some(SemanticColorRole::BorderSoft));
        assert!(!context.world().interaction(id).unwrap().pointer_events);
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.label.as_deref(), Some("标题栏跟随侧边栏透明"));
        assert_eq!(
            accessibility.value.as_deref(),
            Some("仅在侧边栏使用透明材质时生效")
        );
        context
            .read(row, |row| {
                assert_eq!(row.control, Some(control.stable_id()));
            })
            .unwrap();
    }

    #[test]
    fn settings_card_reuses_surface_card_paint() {
        let mut context = AppContext::new();
        let card = context
            .create_component(document(), SettingsCard::new("外观"))
            .unwrap();
        let id = card.stable_id();
        assert_eq!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element {
                tag: "settings-card".into(),
            }
        );
        assert!(matches!(
            context.world().standard_visual(id),
            Some(StandardVisual::Card {
                ref title,
                kind: CardKind::Surface,
                loading: false,
                ..
            }) if title.as_deref() == Some("外观")
        ));
        assert_eq!(
            context.world().node_style(id).unwrap().layout.margin_bottom,
            Some(LengthSpec::Px(CARD_TRAILING_GAP))
        );
    }

    #[test]
    fn collapsible_card_toggles_until_disabled() {
        let mut context = AppContext::new();
        let card = context
            .create_component(document(), SettingsCollapsibleCard::new(false))
            .unwrap();
        let id = card.stable_id();
        assert!(context.world().interaction(id).unwrap().focusable);
        assert_eq!(
            context.world().accessibility(id).unwrap().selected,
            Some(false)
        );
        assert!(context.activate_settings_collapsible_card(card).unwrap());
        context.read(card, |card| assert!(card.expanded)).unwrap();
        assert_eq!(
            context.world().accessibility(id).unwrap().selected,
            Some(true)
        );

        let locked = context
            .create_component(
                document(),
                SettingsCollapsibleCard::new(true).disabled(true),
            )
            .unwrap();
        assert!(
            !context
                .world()
                .interaction(locked.stable_id())
                .unwrap()
                .focusable
        );
        assert!(!context.activate_settings_collapsible_card(locked).unwrap());
        context.read(locked, |card| assert!(card.expanded)).unwrap();
    }

    #[test]
    fn appearance_events_stay_on_the_host_snapshot() {
        let mut theme = ThemeMode::Dark;
        let mut appearance = AppearanceSettings::default();
        assert!(apply_appearance_event(
            &mut theme,
            &mut appearance,
            AppearanceEvent::Theme(ThemeMode::Light),
        ));
        assert_eq!(theme, ThemeMode::Light);
        assert!(apply_appearance_event(
            &mut theme,
            &mut appearance,
            AppearanceEvent::WindowMaterial(WindowMaterialMode::Translucent),
        ));
        assert_eq!(
            appearance.window_material(),
            WindowMaterialMode::Translucent
        );
        assert!(apply_appearance_event(
            &mut theme,
            &mut appearance,
            AppearanceEvent::BackdropTarget(BackdropTarget::Main),
        ));
        assert_eq!(appearance.backdrop_target(), BackdropTarget::Main);
        assert!(apply_appearance_event(
            &mut theme,
            &mut appearance,
            AppearanceEvent::Reset,
        ));
        assert_eq!(theme, AppearanceSettings::RESET_THEME);
        assert_eq!(appearance, AppearanceSettings::default());

        let mut context = AppContext::new();
        let section = context
            .create_component(
                document(),
                AppearanceSection::new(ThemeMode::Dark, AppearanceSettings::default())
                    .platform_hint("选择窗口使用的透明材质或实色背景。"),
            )
            .unwrap();
        assert_eq!(
            context.world().node(section.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "appearance-section".into(),
            }
        );
        assert!(matches!(
            context.world().standard_visual(section.stable_id()),
            Some(StandardVisual::Card {
                ref title,
                kind: CardKind::Surface,
                ..
            }) if title.as_deref() == Some("外观")
        ));
    }
}
