use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, AppearanceEvent, AppearanceSettings, BackdropTarget, ButtonKind, CardKind,
    ControlSize, FlexDirection, Icon, LengthSpec, OverflowSpec, SemanticColorRole, SettingsModel,
    SettingsState, SettingsTabId, ThemeMode, UI_METRICS, WindowMaterialMode,
};

use crate::view_components::{
    Button, Card, RangeChanged, RangeField, Switch, Text, ToggleChanged, project_common,
};
use crate::{
    AccessibilityRole, AccessibilityState, Activate, AppContext, ComponentView, Entity,
    FrameworkError, InteractionState, InteractionStyle, ListItemSlots, MutationQueue, NodeKind,
    NodeStyle, ScrollAxes, ScrollView, SegmentedControl, SegmentedOption,
    SegmentedSelectionRequested, SemanticPaint, SidebarFrame, SidebarRow, SidebarRowState,
    StableNodeId, StandardVisual, TextContent, UiWorld,
};

const ROW_PADDING_Y: f32 = 10.0;
const ROW_GROUP_PADDING_Y: f32 = 4.0;
const ROW_STACK_GAP: f32 = 6.0;
const ROW_STACK_GAP_LOOSE: f32 = 10.0;
const ROW_INLINE_GAP: f32 = 8.0;
const ROW_INLINE_GAP_LOOSE: f32 = 14.0;
const ROW_COPY_GAP: f32 = 2.0;
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
    pub label_slot: Option<StableNodeId>,
    pub hint_slot: Option<StableNodeId>,
    pub(crate) copy_slot: Option<StableNodeId>,
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
            label_slot: None,
            hint_slot: None,
            copy_slot: None,
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

    pub fn label_slot(mut self, label: StableNodeId) -> Self {
        self.label_slot = Some(label);
        self
    }

    pub fn hint_slot(mut self, hint: StableNodeId) -> Self {
        self.hint_slot = Some(hint);
        self
    }

    pub fn copy_slot(mut self, copy: StableNodeId) -> Self {
        self.copy_slot = Some(copy);
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
        // Structured children/slots paint label and hint. Rust-only rows without
        // a tree keep the label as node text so Scene can still show it.
        let has_slots = self.label_slot.is_some()
            || self.hint_slot.is_some()
            || self.control.is_some()
            || self.copy_slot.is_some();
        let has_children = world.node(id).is_some_and(|node| !node.children.is_empty());
        let visible_label = if has_slots || has_children {
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
        self.project_slots(world, mutations);
    }
}

impl SettingsRow {
    fn project_slots(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        if let Some(copy) = self.copy_slot {
            SettingsRowCopy {
                stacked: self.stacked,
            }
            .project(copy, world, mutations);
        }
        if let Some(label) = self.label_slot {
            let mut text = Text::new(self.label.as_ref());
            text.style = label_slot_style();
            text.project(label, world, mutations);
        }
        if let Some(hint) = self.hint_slot {
            let mut text = Text::new(self.hint.as_deref().unwrap_or(""));
            text.style = hint_slot_style(self.hint.is_none());
            text.project(hint, world, mutations);
        }
    }
}

fn label_slot_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    style.foreground = Some(SemanticColorRole::Text);
    style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
    let layout = Arc::make_mut(&mut style.layout);
    layout.font_size = Some(13.0);
    layout.font_weight = Some(500);
    layout.width = Some(LengthSpec::Fill);
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.white_space_nowrap = true;
    layout.text_overflow_ellipsis = true;
    style
}

fn hint_slot_style(hidden: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.foreground = Some(SemanticColorRole::Muted);
    let layout = Arc::make_mut(&mut style.layout);
    layout.font_size = Some(12.0);
    layout.font_weight = Some(400);
    layout.width = Some(LengthSpec::Fill);
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.white_space_nowrap = true;
    layout.text_overflow_ellipsis = true;
    layout.hidden = hidden;
    style
}

#[derive(Debug, Clone, PartialEq)]
struct SettingsRowCopy {
    stacked: bool,
}

impl SettingsRowCopy {
    fn style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(LengthSpec::Fill);
        layout.gap = Some(LengthSpec::Px(ROW_COPY_GAP));
        layout.flex_grow = Some(if self.stacked { 0.0 } else { 1.0 });
        layout.flex_shrink = Some(1.0);
        layout.min_width = Some(LengthSpec::Px(0.0));
        style
    }
}

impl ComponentView for SettingsRowCopy {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-row-copy".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
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

const COLLAPSIBLE_HEADER_GAP: f32 = 8.0;
const COLLAPSIBLE_BODY_GAP: f32 = 12.0;
const DISCLOSURE_ICON_SIZE: f32 = 16.0;

/// Controlled collapse. Header is keyboard-activatable; values stay host-owned.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsCollapsibleCard {
    pub summary: Option<StableNodeId>,
    pub details: Option<StableNodeId>,
    pub accessory: Option<StableNodeId>,
    pub(crate) header: Option<StableNodeId>,
    pub(crate) disclosure: Option<StableNodeId>,
    pub(crate) divider: Option<StableNodeId>,
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
            header: None,
            disclosure: None,
            divider: None,
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
        layout.gap = Some(LengthSpec::Px(COLLAPSIBLE_BODY_GAP));
        layout.padding_left = Some(LengthSpec::Px(UI_METRICS.panel_padding_x));
        layout.padding_right = Some(LengthSpec::Px(UI_METRICS.panel_padding_x));
        layout.padding_top = Some(LengthSpec::Px(UI_METRICS.panel_padding_y));
        layout.padding_bottom = Some(LengthSpec::Px(UI_METRICS.panel_padding_y));
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
        self.project_slots(world, mutations);
    }
}

fn project_slots_hidden(
    id: StableNodeId,
    hidden: bool,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    let Some(current) = world.node_style(id) else {
        return;
    };
    if current.layout.hidden == hidden {
        return;
    }
    let mut style = current.clone();
    Arc::make_mut(&mut style.layout).hidden = hidden;
    mutations.set_style(id, style);
}

fn project_summary_slot(id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
    let Some(current) = world.node_style(id) else {
        return;
    };
    let layout = current.layout.as_ref();
    let needs_width = layout.width.is_none();
    let needs_grow = layout.flex_grow.is_none();
    let needs_color = current.foreground != Some(SemanticColorRole::Text);
    if !needs_width && !needs_grow && !needs_color {
        return;
    }
    let mut style = current.clone();
    if needs_color {
        style.foreground = Some(SemanticColorRole::Text);
    }
    let layout = Arc::make_mut(&mut style.layout);
    if needs_width {
        layout.width = Some(LengthSpec::Fill);
    }
    if needs_grow {
        layout.flex_grow = Some(1.0);
    }
    mutations.set_style(id, style);
}

fn disclosure_icon_kind(expanded: bool) -> Icon {
    if expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    }
}

impl SettingsCollapsibleCard {
    fn project_slots(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        if let Some(header) = self.header {
            SettingsCollapsibleHeader.project(header, world, mutations);
        }
        if let Some(summary) = self.summary {
            project_summary_slot(summary, world, mutations);
        }
        if let Some(disclosure) = self.disclosure {
            SettingsDisclosure {
                expanded: self.expanded,
            }
            .project(disclosure, world, mutations);
        }
        if let Some(divider) = self.divider {
            SettingsCollapsibleDivider {
                hidden: !self.expanded,
            }
            .project(divider, world, mutations);
        }
        if let Some(details) = self.details {
            project_slots_hidden(details, !self.expanded, world, mutations);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SettingsCollapsibleHeader;

impl SettingsCollapsibleHeader {
    fn style() -> NodeStyle {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.width = Some(LengthSpec::Fill);
        layout.gap = Some(LengthSpec::Px(COLLAPSIBLE_HEADER_GAP));
        style
    }
}

impl ComponentView for SettingsCollapsibleHeader {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-collapsible-header".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &Self::style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SettingsCollapsibleDivider {
    hidden: bool,
}

impl SettingsCollapsibleDivider {
    fn style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        style.background = Some(SemanticColorRole::BorderSoft);
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(1.0));
        layout.min_height = Some(LengthSpec::Px(1.0));
        layout.hidden = self.hidden;
        style
    }
}

impl ComponentView for SettingsCollapsibleDivider {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-collapsible-divider".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Visual chevron only. Activation stays on the card so accessory controls
/// remain independently interactive.
#[derive(Debug, Clone, PartialEq)]
struct SettingsDisclosure {
    expanded: bool,
}

impl ComponentView for SettingsDisclosure {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-disclosure".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::Icon {
            icon: disclosure_icon_kind(self.expanded),
            size: DISCLOSURE_ICON_SIZE,
            tooltip: None,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let size = ControlSize::Small.height();
        let mut style = NodeStyle::default();
        style.foreground = Some(SemanticColorRole::Muted);
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(size));
        layout.height = Some(LengthSpec::Px(size));
        layout.min_width = Some(LengthSpec::Px(size));
        layout.min_height = Some(LengthSpec::Px(size));
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                selected: Some(self.expanded),
                ..AccessibilityState::default()
            },
        );
    }
}

const DEFAULT_PLATFORM_HINT: &str = "选择窗口使用的透明材质或实色背景。";

/// Host-owned appearance snapshot. Events stay [`AppearanceEvent`]; values stay outside NanaUI.
#[derive(Debug, Clone, PartialEq)]
pub struct AppearanceSection {
    pub theme: ThemeMode,
    pub appearance: AppearanceSettings,
    pub material_status: Option<Arc<str>>,
    pub platform_hint: Option<Arc<str>>,
    pub assembly: Option<AppearanceSectionAssembly>,
}

/// Retained children created by [`AppContext::assemble_appearance_section`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppearanceSectionAssembly {
    pub theme_row: Option<StableNodeId>,
    pub theme_control: Option<StableNodeId>,
    pub theme_dark: Option<StableNodeId>,
    pub theme_light: Option<StableNodeId>,
    pub material_row: Option<StableNodeId>,
    pub material_control: Option<StableNodeId>,
    pub material_solid: Option<StableNodeId>,
    pub material_translucent: Option<StableNodeId>,
    pub material_status_row: Option<StableNodeId>,
    pub material_status_value: Option<StableNodeId>,
    pub target_row: Option<StableNodeId>,
    pub target_control: Option<StableNodeId>,
    pub target_sidebar: Option<StableNodeId>,
    pub target_main: Option<StableNodeId>,
    pub titlebar_row: Option<StableNodeId>,
    pub titlebar_switch: Option<StableNodeId>,
    pub opacity_row: Option<StableNodeId>,
    pub opacity_range: Option<StableNodeId>,
    pub opacity_text: Option<StableNodeId>,
    pub workspace_row: Option<StableNodeId>,
    pub workspace_switch: Option<StableNodeId>,
    pub radius_row: Option<StableNodeId>,
    pub radius_range: Option<StableNodeId>,
    pub reset_row: Option<StableNodeId>,
    pub reset_button: Option<StableNodeId>,
}

impl AppearanceSection {
    pub fn new(theme: ThemeMode, appearance: AppearanceSettings) -> Self {
        Self {
            theme,
            appearance,
            material_status: None,
            platform_hint: None,
            assembly: None,
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
    pub assembly: Option<AboutSectionAssembly>,
}

/// Retained children created by [`AppContext::assemble_about_section`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AboutSectionAssembly {
    pub name_row: Option<StableNodeId>,
    pub name_value: Option<StableNodeId>,
    pub version_row: Option<StableNodeId>,
    pub version_value: Option<StableNodeId>,
    pub description: Option<StableNodeId>,
}

impl AboutSection {
    pub fn new(metadata: AboutMetadata) -> Self {
        Self {
            metadata,
            assembly: None,
        }
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

const SETTINGS_SIDEBAR_GAP: f32 = 12.0;
const SETTINGS_SIDEBAR_TAB_GAP: f32 = 1.0;
const SETTINGS_SIDEBAR_ICON_SIZE: f32 = 15.0;
const SETTINGS_PAGE_GAP: f32 = 16.0;
const SETTINGS_PAGE_PADDING_TOP: f32 = 20.0;
const SETTINGS_PAGE_PADDING_RIGHT: f32 = 24.0;
const SETTINGS_PAGE_PADDING_BOTTOM: f32 = 24.0;
const SETTINGS_PAGE_PADDING_LEFT: f32 = 24.0;
const SETTINGS_PAGE_TITLE_SIZE: f32 = 18.0;
const SETTINGS_PAGE_TITLE_WEIGHT: u16 = 600;

/// Host-owned navigation snapshot. Activate emits [`SettingsBack`] / [`SettingsTabSelected`].
#[derive(Debug, Clone)]
pub struct SettingsSidebar {
    pub model: SettingsModel,
    pub state: SettingsState,
    pub assembly: Option<SettingsSidebarAssembly>,
}

/// Retained children created by [`AppContext::assemble_settings_sidebar`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SettingsSidebarAssembly {
    pub back_row: Option<StableNodeId>,
    pub back_icon: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub tab_rows: HashMap<SettingsTabId, StableNodeId>,
    pub tab_leadings: HashMap<SettingsTabId, StableNodeId>,
}

impl SettingsSidebar {
    pub fn new(model: SettingsModel, state: SettingsState) -> Self {
        Self {
            model,
            state,
            assembly: None,
        }
    }

    pub fn row_for_tab(&self, id: &SettingsTabId) -> Option<StableNodeId> {
        self.assembly.as_ref()?.tab_rows.get(id).copied()
    }
}

impl ComponentView for SettingsSidebar {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-sidebar".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut frame = SidebarFrame::new().gap(SETTINGS_SIDEBAR_GAP);
        if let Some(assembly) = &self.assembly {
            if let Some(top) = assembly.back_row {
                frame = frame.top(top);
            }
            if let Some(body) = assembly.body {
                frame = frame.body(body);
            }
        }
        frame.project(id, world, mutations);
    }
}

/// Host-owned page snapshot. Content stays application-owned.
#[derive(Debug, Clone)]
pub struct SettingsPage {
    pub model: SettingsModel,
    pub state: SettingsState,
    pub content: Option<StableNodeId>,
    pub assembly: Option<SettingsPageAssembly>,
}

/// Retained chrome created by [`AppContext::assemble_settings_page`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsPageAssembly {
    pub scroll: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub title: Option<StableNodeId>,
}

impl SettingsPage {
    pub fn new(model: SettingsModel, state: SettingsState) -> Self {
        Self {
            model,
            state,
            content: None,
            assembly: None,
        }
    }

    pub fn content(mut self, content: StableNodeId) -> Self {
        self.content = Some(content);
        self
    }
}

impl ComponentView for SettingsPage {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-page".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &settings_page_style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Activate on the back row. Host owns navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsBack;

/// Activate on a tab row. Host applies [`SettingsState::select`] then re-assembles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsTabSelected {
    pub tab: SettingsTabId,
}

#[derive(Debug, Clone, PartialEq)]
struct SettingsSidebarLeading {
    icon: Option<Icon>,
    selected: bool,
}

impl SettingsSidebarLeading {
    fn style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        style.foreground = Some(if self.selected {
            SemanticColorRole::Text
        } else {
            SemanticColorRole::Muted
        });
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE));
        layout.height = Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE));
        layout.min_width = Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE));
        layout.min_height = Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE));
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        style
    }
}

impl ComponentView for SettingsSidebarLeading {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-sidebar-leading".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new(),
                },
            );
        }
        let visual = self.icon.map(|icon| StandardVisual::Icon {
            icon,
            size: SETTINGS_SIDEBAR_ICON_SIZE,
            tooltip: None,
        });
        if world.standard_visual(id) != visual {
            mutations.set_standard_visual(id, visual);
        }
        project_common(
            id,
            world,
            mutations,
            &self.style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
struct SettingsPageBody;

impl SettingsPageBody {
    fn style() -> NodeStyle {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(LengthSpec::Fill);
        layout.gap = Some(LengthSpec::Px(SETTINGS_PAGE_GAP));
        layout.padding_top = Some(LengthSpec::Px(SETTINGS_PAGE_PADDING_TOP));
        layout.padding_right = Some(LengthSpec::Px(SETTINGS_PAGE_PADDING_RIGHT));
        layout.padding_bottom = Some(LengthSpec::Px(SETTINGS_PAGE_PADDING_BOTTOM));
        layout.padding_left = Some(LengthSpec::Px(SETTINGS_PAGE_PADDING_LEFT));
        style
    }
}

impl ComponentView for SettingsPageBody {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-page-body".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &Self::style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

fn settings_page_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    style.background = Some(SemanticColorRole::Background);
    style.foreground = Some(SemanticColorRole::Text);
    let layout = Arc::make_mut(&mut style.layout);
    layout.direction = Some(FlexDirection::Column);
    layout.align_items = AlignSpec::Stretch;
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    layout.overflow_x = OverflowSpec::Hidden;
    layout.overflow_y = OverflowSpec::Hidden;
    style
}

fn settings_page_scroll() -> ScrollView {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.min_height = Some(LengthSpec::Px(0.0));
    ScrollView::new(ScrollAxes::Vertical).style(style)
}

fn settings_sidebar_body() -> ScrollView {
    let mut scroll = SidebarFrame::vertical_body_scroll();
    let layout = Arc::make_mut(&mut scroll.style.layout);
    layout.direction = Some(FlexDirection::Column);
    layout.gap = Some(LengthSpec::Px(SETTINGS_SIDEBAR_TAB_GAP));
    scroll
}

fn page_title_text(label: &str) -> Text {
    styled_text(
        label,
        SemanticColorRole::Text,
        SETTINGS_PAGE_TITLE_SIZE,
        SETTINGS_PAGE_TITLE_WEIGHT,
    )
}

fn ensure_settings_leading(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    icon: Option<Icon>,
    selected: bool,
) -> Result<Entity<SettingsSidebarLeading>, FrameworkError> {
    if let Some(id) = *slot {
        let entity = Entity::<SettingsSidebarLeading>::from_stable_id(id);
        context.update_component(entity, |leading, _| {
            leading.icon = icon;
            leading.selected = selected;
        })?;
        Ok(entity)
    } else {
        let entity = context
            .create_detached_component(document, SettingsSidebarLeading { icon, selected })?;
        *slot = Some(entity.stable_id());
        Ok(entity)
    }
}

fn ensure_settings_nav_row(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    label: &str,
    state: SidebarRowState,
    leading: StableNodeId,
) -> Result<(Entity<SidebarRow>, bool), FrameworkError> {
    let created = slot.is_none();
    let row = if let Some(id) = *slot {
        let entity = Entity::<SidebarRow>::from_stable_id(id);
        context.update_component(entity, |row, _| {
            row.label = Arc::from(label);
            row.state = state;
            row.slots.leading = Some(leading);
        })?;
        entity
    } else {
        let entity = context.create_detached_component(
            document,
            SidebarRow::new(label).state(state).slots(ListItemSlots {
                leading: Some(leading),
                content: None,
                trailing: None,
            }),
        )?;
        *slot = Some(entity.stable_id());
        entity
    };
    reconcile_children(context, row, &[leading])?;
    Ok((row, created))
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

fn appearance_mode(appearance: &AppearanceSettings) -> (bool, bool) {
    let solid = matches!(appearance.window_material(), WindowMaterialMode::Solid);
    let titlebar_follow_disabled =
        solid || !matches!(appearance.backdrop_target(), BackdropTarget::Sidebar);
    (solid, titlebar_follow_disabled)
}

fn opacity_percent(appearance: &AppearanceSettings) -> f64 {
    f64::from((appearance.backdrop_opacity() * 100.0).round())
}

fn document_of(
    context: &AppContext,
    id: StableNodeId,
) -> Result<crate::DocumentId, FrameworkError> {
    context
        .world()
        .node(id)
        .map(|node| node.document)
        .ok_or(FrameworkError::MissingView(id))
}

fn reconcile_children<C: ComponentView>(
    context: &mut AppContext,
    parent: Entity<C>,
    ordered: &[StableNodeId],
) -> Result<bool, FrameworkError> {
    let parent_id = parent.stable_id();
    let current = context
        .world()
        .node(parent_id)
        .ok_or(FrameworkError::MissingView(parent_id))?
        .children
        .clone();
    if current.as_slice() == ordered {
        return Ok(false);
    }
    let keep = ordered.iter().copied().collect::<HashSet<_>>();
    context.update_component(parent, |_, cx| {
        for child in &current {
            if !keep.contains(child) {
                cx.mutations().park_subtree(*child);
            }
        }
        for child in ordered {
            cx.mutations().insert(parent_id, *child, None);
        }
    })?;
    Ok(true)
}

fn styled_text(value: impl Into<String>, color: SemanticColorRole, size: f32, weight: u16) -> Text {
    let mut style = NodeStyle::default();
    style.foreground = Some(color);
    let layout = Arc::make_mut(&mut style.layout);
    layout.font_size = Some(size);
    layout.font_weight = Some(weight);
    Text::new(value).style(style)
}

fn row_label_text(label: &str) -> Text {
    Text::new(label).style(label_slot_style())
}

fn row_hint_text(hint: &str, hidden: bool) -> Text {
    Text::new(hint).style(hint_slot_style(hidden))
}

fn sync_text(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    next: Text,
) -> Result<Entity<Text>, FrameworkError> {
    if let Some(id) = *slot {
        let entity = Entity::from_stable_id(id);
        context.update_component(entity, |text, _| {
            *text = next;
        })?;
        Ok(entity)
    } else {
        let entity = context.create_detached_component(document, next)?;
        *slot = Some(entity.stable_id());
        Ok(entity)
    }
}

fn mount_settings_row(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    label: &str,
    hint: Option<&str>,
    divided: bool,
    first: bool,
    last: bool,
    control: StableNodeId,
) -> Result<Entity<SettingsRow>, FrameworkError> {
    let mut label_slot = None;
    let mut hint_slot = None;
    let mut copy_slot = None;
    if let Some(row_id) = *slot {
        if let Ok((existing_label, existing_hint, existing_copy)) = context
            .read(Entity::<SettingsRow>::from_stable_id(row_id), |row| {
                (row.label_slot, row.hint_slot, row.copy_slot)
            })
        {
            label_slot = existing_label;
            hint_slot = existing_hint;
            copy_slot = existing_copy;
        }
        if label_slot.is_none() {
            if let Some(first_child) = context
                .world()
                .node(row_id)
                .and_then(|node| node.children.first().copied())
                .filter(|child| *child != control)
            {
                let nested = context
                    .world()
                    .node(first_child)
                    .map(|node| node.children.clone())
                    .unwrap_or_default();
                if nested.is_empty() {
                    label_slot = Some(first_child);
                }
            }
        }
    }
    let label_text = sync_text(context, document, &mut label_slot, row_label_text(label))?;
    let hint_text = if let Some(hint) = hint {
        Some(sync_text(
            context,
            document,
            &mut hint_slot,
            row_hint_text(hint, false),
        )?)
    } else {
        if let Some(id) = hint_slot {
            context.update_component(Entity::<Text>::from_stable_id(id), |text, _| {
                *text = row_hint_text("", true);
            })?;
        }
        None
    };
    let use_copy = hint_text.is_some() || copy_slot.is_some();
    let copy = if use_copy {
        Some(if let Some(id) = copy_slot {
            Entity::<SettingsRowCopy>::from_stable_id(id)
        } else {
            let entity =
                context.create_detached_component(document, SettingsRowCopy { stacked: false })?;
            copy_slot = Some(entity.stable_id());
            entity
        })
    } else {
        None
    };
    let hint_id = hint_text
        .as_ref()
        .map(|text| text.stable_id())
        .or(hint_slot);
    let copy_id = copy.map(|entity| entity.stable_id()).or(copy_slot);
    let apply = |row: &mut SettingsRow, _: &mut crate::ViewContext<'_, SettingsRow>| {
        row.label = Arc::from(label);
        row.hint = hint.map(Arc::from);
        row.divided = divided;
        row.first_in_group = first;
        row.last_in_group = last;
        row.control = Some(control);
        row.label_slot = Some(label_text.stable_id());
        row.hint_slot = hint_id;
        row.copy_slot = copy_id;
    };
    let row = if let Some(id) = *slot {
        let entity = Entity::<SettingsRow>::from_stable_id(id);
        context.update_component(entity, apply)?;
        entity
    } else {
        let mut row = SettingsRow::new(label)
            .divided(divided)
            .control_child(control)
            .label_slot(label_text.stable_id());
        if let Some(hint) = hint {
            row = row.hint(hint);
        }
        if let Some(hint_entity) = &hint_text {
            row = row.hint_slot(hint_entity.stable_id());
        }
        if first {
            row = row.first_in_group();
        }
        if last {
            row = row.last_in_group();
        }
        row.copy_slot = copy_slot;
        let entity = context.create_detached_component(document, row)?;
        *slot = Some(entity.stable_id());
        entity
    };
    if let Some(copy) = copy {
        let mut copy_children = vec![label_text.stable_id()];
        if let Some(hint_entity) = &hint_text {
            copy_children.push(hint_entity.stable_id());
        }
        reconcile_children(context, copy, &copy_children)?;
        reconcile_children(context, row, &[copy.stable_id(), control])?;
    } else {
        reconcile_children(context, row, &[label_text.stable_id(), control])?;
    }
    Ok(row)
}

fn ensure_segmented_pair(
    context: &mut AppContext,
    document: crate::DocumentId,
    control_slot: &mut Option<StableNodeId>,
    first_slot: &mut Option<StableNodeId>,
    second_slot: &mut Option<StableNodeId>,
    first: SegmentedOption,
    second: SegmentedOption,
    selected_first: bool,
    first_disabled: bool,
    second_disabled: bool,
) -> Result<
    (
        Entity<SegmentedControl>,
        Entity<SegmentedOption>,
        Entity<SegmentedOption>,
    ),
    FrameworkError,
> {
    let created = control_slot.is_none();
    let control = if let Some(id) = *control_slot {
        Entity::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(document, SegmentedControl::new())?;
        *control_slot = Some(entity.stable_id());
        entity
    };
    let first_entity = if let Some(id) = *first_slot {
        Entity::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(document, first)?;
        *first_slot = Some(entity.stable_id());
        entity
    };
    let second_entity = if let Some(id) = *second_slot {
        Entity::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(document, second)?;
        *second_slot = Some(entity.stable_id());
        entity
    };
    if created {
        context.set_segmented_options(
            control,
            vec![first_entity, second_entity],
            Some(if selected_first {
                first_entity
            } else {
                second_entity
            }),
        )?;
    } else {
        context.set_segmented_selection(
            control,
            Some(if selected_first {
                first_entity
            } else {
                second_entity
            }),
        )?;
    }
    context.set_segmented_option_disabled(control, first_entity, first_disabled)?;
    context.set_segmented_option_disabled(control, second_entity, second_disabled)?;
    Ok((control, first_entity, second_entity))
}

fn ensure_switch(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    label: &str,
    checked: bool,
    disabled: bool,
) -> Result<Entity<Switch>, FrameworkError> {
    if let Some(id) = *slot {
        let entity = Entity::<Switch>::from_stable_id(id);
        context.update_component(entity, |switch, _| {
            switch.checked = checked;
            switch.disabled = disabled;
        })?;
        Ok(entity)
    } else {
        let entity = context
            .create_detached_component(document, Switch::new(label, checked).disabled(disabled))?;
        *slot = Some(entity.stable_id());
        Ok(entity)
    }
}

fn ensure_range(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    value: f64,
    minimum: f64,
    maximum: f64,
    unit: &str,
    parent: StableNodeId,
) -> Result<Entity<RangeField>, FrameworkError> {
    if let Some(id) = *slot {
        let entity = Entity::<RangeField>::from_stable_id(id);
        context.update_component(entity, |range, _| {
            range.value = range.quantize(value);
        })?;
        Ok(entity)
    } else {
        let field = RangeField::new(value, minimum, maximum, 1.0)
            .map_err(|_| FrameworkError::InvalidComponentValue(parent))?
            .unit(unit);
        let entity = context.create_detached_component(document, field)?;
        *slot = Some(entity.stable_id());
        Ok(entity)
    }
}

impl AppContext {
    /// Mount a settings row with painted label / optional hint / control slots.
    pub fn mount_settings_leaf_row(
        &mut self,
        document: crate::DocumentId,
        label: &str,
        hint: Option<&str>,
        control: StableNodeId,
    ) -> Result<Entity<SettingsRow>, FrameworkError> {
        let mut slot = None;
        mount_settings_row(
            self, document, &mut slot, label, hint, false, true, true, control,
        )
    }

    /// Mount or refresh the Iced appearance row/control contract from the
    /// host snapshot stored on [`AppearanceSection`].
    ///
    /// Child activations re-emit [`AppearanceEvent`] from the section. The
    /// host binds `on(section, AppearanceEvent)` and applies the event with
    /// [`apply_appearance_event`] after the interaction returns, then writes
    /// the snapshot back and calls this method again.
    pub fn assemble_appearance_section(
        &mut self,
        section: Entity<AppearanceSection>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, section.stable_id())?;
        let snapshot = self.read(section, |section| {
            (
                section.theme,
                section.appearance,
                section.material_status.clone(),
                section.platform_hint.clone(),
                section.assembly.clone().unwrap_or_default(),
            )
        })?;
        let (theme, appearance, material_status, platform_hint, mut assembly) = snapshot;
        let (solid_mode, titlebar_follow_disabled) = appearance_mode(&appearance);
        let created_theme = assembly.theme_control.is_none();
        let created_material = assembly.material_control.is_none();
        let created_target = assembly.target_control.is_none();
        let created_titlebar = assembly.titlebar_switch.is_none();
        let created_workspace = assembly.workspace_switch.is_none();
        let created_radius = assembly.radius_range.is_none();
        let created_reset = assembly.reset_button.is_none();
        let mut created_opacity_range = false;
        let (theme_control, theme_dark, theme_light) = ensure_segmented_pair(
            self,
            document,
            &mut assembly.theme_control,
            &mut assembly.theme_dark,
            &mut assembly.theme_light,
            SegmentedOption::new("暗色").icon(Icon::Moon),
            SegmentedOption::new("浅色").icon(Icon::Appearance),
            matches!(theme, ThemeMode::Dark),
            false,
            false,
        )?;
        assembly.theme_control = Some(theme_control.stable_id());
        assembly.theme_dark = Some(theme_dark.stable_id());
        assembly.theme_light = Some(theme_light.stable_id());
        let theme_row = mount_settings_row(
            self,
            document,
            &mut assembly.theme_row,
            "主题",
            Some("选择应用配色，立即生效"),
            true,
            true,
            false,
            theme_control.stable_id(),
        )?;

        let (material_control, material_solid, material_translucent) = ensure_segmented_pair(
            self,
            document,
            &mut assembly.material_control,
            &mut assembly.material_solid,
            &mut assembly.material_translucent,
            SegmentedOption::new("实色"),
            SegmentedOption::new("透明"),
            matches!(appearance.window_material(), WindowMaterialMode::Solid),
            false,
            false,
        )?;
        assembly.material_control = Some(material_control.stable_id());
        assembly.material_solid = Some(material_solid.stable_id());
        assembly.material_translucent = Some(material_translucent.stable_id());
        let material_hint = platform_hint.as_deref().unwrap_or(DEFAULT_PLATFORM_HINT);
        let material_row = mount_settings_row(
            self,
            document,
            &mut assembly.material_row,
            "窗口材质",
            Some(material_hint),
            true,
            false,
            false,
            material_control.stable_id(),
        )?;

        let status_row = if let Some(status) = material_status.as_deref() {
            let status_value = sync_text(
                self,
                document,
                &mut assembly.material_status_value,
                styled_text(status, SemanticColorRole::Muted, 12.0, 400),
            )?;
            Some(mount_settings_row(
                self,
                document,
                &mut assembly.material_status_row,
                "材质状态",
                Some("显示窗口当前使用的外观效果。"),
                true,
                false,
                false,
                status_value.stable_id(),
            )?)
        } else {
            None
        };

        let (target_control, target_sidebar, target_main) = ensure_segmented_pair(
            self,
            document,
            &mut assembly.target_control,
            &mut assembly.target_sidebar,
            &mut assembly.target_main,
            SegmentedOption::new("侧边栏").disabled(solid_mode),
            SegmentedOption::new("主内容区").disabled(solid_mode),
            matches!(appearance.backdrop_target(), BackdropTarget::Sidebar),
            solid_mode,
            solid_mode,
        )?;
        assembly.target_control = Some(target_control.stable_id());
        assembly.target_sidebar = Some(target_sidebar.stable_id());
        assembly.target_main = Some(target_main.stable_id());
        let target_hint = if solid_mode {
            "实色模式不显示透明区域；切回透明材质后会恢复当前选择。"
        } else {
            "选择侧边栏或主内容区显示透明材质。"
        };
        let target_row = mount_settings_row(
            self,
            document,
            &mut assembly.target_row,
            "透明区域",
            Some(target_hint),
            true,
            false,
            false,
            target_control.stable_id(),
        )?;

        let titlebar_switch = ensure_switch(
            self,
            document,
            &mut assembly.titlebar_switch,
            "",
            appearance.titlebar_follows_sidebar(),
            titlebar_follow_disabled,
        )?;
        let titlebar_hint = if titlebar_follow_disabled {
            "仅在侧边栏使用透明材质时生效；当前选择会保留。"
        } else {
            "侧边栏透明时，整个标题栏同步显示透明材质。"
        };
        let titlebar_row = mount_settings_row(
            self,
            document,
            &mut assembly.titlebar_row,
            "标题栏跟随侧边栏透明",
            Some(titlebar_hint),
            true,
            false,
            false,
            titlebar_switch.stable_id(),
        )?;

        let opacity = opacity_percent(&appearance);
        let opacity_control = if solid_mode {
            sync_text(
                self,
                document,
                &mut assembly.opacity_text,
                styled_text(
                    format!("{opacity:.0}%"),
                    SemanticColorRole::Muted,
                    12.0,
                    400,
                ),
            )?
            .stable_id()
        } else {
            created_opacity_range |= assembly.opacity_range.is_none();
            ensure_range(
                self,
                document,
                &mut assembly.opacity_range,
                opacity,
                f64::from(AppearanceSettings::MIN_BACKDROP_OPACITY) * 100.0,
                f64::from(AppearanceSettings::MAX_BACKDROP_OPACITY) * 100.0,
                "%",
                section.stable_id(),
            )?
            .stable_id()
        };
        let opacity_hint = if solid_mode {
            "实色模式不使用透明度；切回透明材质后会恢复当前数值。"
        } else {
            "调节透明区域材质的前景色覆盖程度。"
        };
        let opacity_row = mount_settings_row(
            self,
            document,
            &mut assembly.opacity_row,
            "材质不透明度",
            Some(opacity_hint),
            true,
            false,
            false,
            opacity_control,
        )?;

        let workspace_switch = ensure_switch(
            self,
            document,
            &mut assembly.workspace_switch,
            "主区域圆角",
            appearance.workspace_corners_enabled(),
            false,
        )?;
        let workspace_row = mount_settings_row(
            self,
            document,
            &mut assembly.workspace_row,
            "工作区边缘",
            None,
            true,
            false,
            false,
            workspace_switch.stable_id(),
        )?;

        let radius_range = ensure_range(
            self,
            document,
            &mut assembly.radius_range,
            f64::from(appearance.standard_radius()),
            f64::from(AppearanceSettings::MIN_STANDARD_RADIUS),
            f64::from(AppearanceSettings::MAX_STANDARD_RADIUS),
            " px",
            section.stable_id(),
        )?;
        let radius_row = mount_settings_row(
            self,
            document,
            &mut assembly.radius_row,
            "组件圆角半径",
            None,
            true,
            false,
            false,
            radius_range.stable_id(),
        )?;

        let reset_button = if let Some(id) = assembly.reset_button {
            Entity::<Button>::from_stable_id(id)
        } else {
            let entity = self.create_detached_component(
                document,
                Button::new("恢复默认")
                    .kind(ButtonKind::Subtle)
                    .size(ControlSize::Small),
            )?;
            assembly.reset_button = Some(entity.stable_id());
            entity
        };
        let reset_row = mount_settings_row(
            self,
            document,
            &mut assembly.reset_row,
            "默认样式",
            Some("恢复主题、材质与圆角默认值。"),
            false,
            false,
            true,
            reset_button.stable_id(),
        )?;

        if created_theme {
            let dark = theme_dark.stable_id();
            let light = theme_light.stable_id();
            self.observe(
                theme_control,
                section,
                move |_, event: &SegmentedSelectionRequested, cx| {
                    let next = if event.option == dark {
                        ThemeMode::Dark
                    } else if event.option == light {
                        ThemeMode::Light
                    } else {
                        return;
                    };
                    cx.emit(AppearanceEvent::Theme(next));
                },
            )?;
        }
        if created_material {
            let solid = material_solid.stable_id();
            let translucent = material_translucent.stable_id();
            self.observe(
                material_control,
                section,
                move |_, event: &SegmentedSelectionRequested, cx| {
                    let next = if event.option == solid {
                        WindowMaterialMode::Solid
                    } else if event.option == translucent {
                        WindowMaterialMode::Translucent
                    } else {
                        return;
                    };
                    cx.emit(AppearanceEvent::WindowMaterial(next));
                },
            )?;
        }
        if created_target {
            let sidebar = target_sidebar.stable_id();
            let main = target_main.stable_id();
            self.observe(
                target_control,
                section,
                move |_, event: &SegmentedSelectionRequested, cx| {
                    let next = if event.option == sidebar {
                        BackdropTarget::Sidebar
                    } else if event.option == main {
                        BackdropTarget::Main
                    } else {
                        return;
                    };
                    cx.emit(AppearanceEvent::BackdropTarget(next));
                },
            )?;
        }
        if created_titlebar {
            self.observe(titlebar_switch, section, |_, event: &ToggleChanged, cx| {
                cx.emit(AppearanceEvent::TitlebarFollowsSidebar(event.checked));
            })?;
        }
        if created_opacity_range {
            if let Some(id) = assembly.opacity_range {
                self.observe(
                    Entity::<RangeField>::from_stable_id(id),
                    section,
                    |_, event: &RangeChanged, cx| {
                        cx.emit(AppearanceEvent::BackdropOpacity(event.value as f32 / 100.0));
                    },
                )?;
            }
        }
        if created_workspace {
            self.observe(workspace_switch, section, |_, event: &ToggleChanged, cx| {
                cx.emit(AppearanceEvent::WorkspaceCorners(event.checked));
            })?;
        }
        if created_radius {
            self.observe(radius_range, section, |_, event: &RangeChanged, cx| {
                cx.emit(AppearanceEvent::StandardRadius(event.value.round() as u8));
            })?;
        }
        if created_reset {
            self.observe(reset_button, section, |_, _: &Activate, cx| {
                cx.emit(AppearanceEvent::Reset);
            })?;
        }

        let mut ordered = vec![theme_row.stable_id(), material_row.stable_id()];
        if let Some(status_row) = status_row {
            ordered.push(status_row.stable_id());
        }
        ordered.extend([
            target_row.stable_id(),
            titlebar_row.stable_id(),
            opacity_row.stable_id(),
            workspace_row.stable_id(),
            radius_row.stable_id(),
            reset_row.stable_id(),
        ]);
        self.update_component(section, |section, _| {
            section.assembly = Some(assembly);
        })?;
        reconcile_children(self, section, &ordered)
    }

    /// Mount or refresh name/version/description rows from injected metadata.
    pub fn assemble_about_section(
        &mut self,
        section: Entity<AboutSection>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, section.stable_id())?;
        let (metadata, mut assembly) = self.read(section, |section| {
            (
                section.metadata.clone(),
                section.assembly.clone().unwrap_or_default(),
            )
        })?;
        let name_value = sync_text(
            self,
            document,
            &mut assembly.name_value,
            styled_text(
                metadata.product_title.as_ref(),
                SemanticColorRole::Text,
                12.0,
                500,
            ),
        )?;
        let name_row = mount_settings_row(
            self,
            document,
            &mut assembly.name_row,
            "名称",
            None,
            true,
            true,
            false,
            name_value.stable_id(),
        )?;
        let version_value = sync_text(
            self,
            document,
            &mut assembly.version_value,
            styled_text(
                metadata.version.as_ref(),
                SemanticColorRole::Muted,
                12.0,
                400,
            ),
        )?;
        let version_row = mount_settings_row(
            self,
            document,
            &mut assembly.version_row,
            "版本",
            None,
            false,
            false,
            true,
            version_value.stable_id(),
        )?;
        let mut ordered = vec![name_row.stable_id(), version_row.stable_id()];
        if let Some(description) = metadata.description.as_deref() {
            let mut style = styled_text(description, SemanticColorRole::Muted, 12.0, 400).style;
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(LengthSpec::Fill);
            layout.padding_top = Some(LengthSpec::Px(8.0));
            let description = sync_text(
                self,
                document,
                &mut assembly.description,
                styled_text(description, SemanticColorRole::Muted, 12.0, 400).style(style),
            )?;
            ordered.push(description.stable_id());
        }
        self.update_component(section, |section, _| {
            section.assembly = Some(assembly);
        })?;
        reconcile_children(self, section, &ordered)
    }

    /// Mount header chrome and host slots. Details stay hidden while collapsed.
    pub fn assemble_settings_collapsible_card(
        &mut self,
        card: Entity<SettingsCollapsibleCard>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, card.stable_id())?;
        let snapshot = self.read(card, |card| {
            (
                card.summary,
                card.details,
                card.accessory,
                card.header,
                card.disclosure,
                card.divider,
                card.expanded,
            )
        })?;
        let (summary, details, accessory, header, disclosure, divider, expanded) = snapshot;
        let header = if let Some(id) = header {
            Entity::from_stable_id(id)
        } else {
            self.create_detached_component(document, SettingsCollapsibleHeader)?
        };
        let disclosure = if let Some(id) = disclosure {
            let entity = Entity::<SettingsDisclosure>::from_stable_id(id);
            self.update_component(entity, |mark, _| {
                mark.expanded = expanded;
            })?;
            entity
        } else {
            self.create_detached_component(document, SettingsDisclosure { expanded })?
        };
        let divider = if let Some(id) = divider {
            let entity = Entity::<SettingsCollapsibleDivider>::from_stable_id(id);
            self.update_component(entity, |mark, _| {
                mark.hidden = !expanded;
            })?;
            entity
        } else {
            self.create_detached_component(
                document,
                SettingsCollapsibleDivider { hidden: !expanded },
            )?
        };
        let mut header_children = Vec::new();
        if let Some(summary) = summary {
            header_children.push(summary);
        }
        if let Some(accessory) = accessory {
            header_children.push(accessory);
        }
        header_children.push(disclosure.stable_id());
        reconcile_children(self, header, &header_children)?;
        self.update_component(card, |card, _| {
            card.header = Some(header.stable_id());
            card.disclosure = Some(disclosure.stable_id());
            card.divider = Some(divider.stable_id());
        })?;
        let mut ordered = vec![header.stable_id(), divider.stable_id()];
        if let Some(details) = details {
            ordered.push(details);
        }
        reconcile_children(self, card, &ordered)
    }

    /// Mount back + tab rows from the host snapshot on [`SettingsSidebar`].
    ///
    /// Row activation re-emits [`SettingsBack`] / [`SettingsTabSelected`] from
    /// the sidebar. The host binds those events, updates [`SettingsState`],
    /// writes the snapshot back, and calls this method again.
    pub fn assemble_settings_sidebar(
        &mut self,
        sidebar: Entity<SettingsSidebar>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, sidebar.stable_id())?;
        let (model, state, mut assembly) = self.read(sidebar, |sidebar| {
            (
                sidebar.model.clone(),
                sidebar.state.clone(),
                sidebar.assembly.clone().unwrap_or_default(),
            )
        })?;
        let mut back_icon = assembly.back_icon;
        let back_leading =
            ensure_settings_leading(self, document, &mut back_icon, Some(Icon::ArrowLeft), true)?;
        assembly.back_icon = back_icon;
        let mut back_row = assembly.back_row;
        let (back, created_back) = ensure_settings_nav_row(
            self,
            document,
            &mut back_row,
            "返回",
            SidebarRowState::Idle,
            back_leading.stable_id(),
        )?;
        assembly.back_row = back_row;
        if created_back {
            self.observe(back, sidebar, |_, _: &Activate, cx| {
                cx.emit(SettingsBack);
            })?;
        }

        let body = if let Some(id) = assembly.body {
            Entity::<ScrollView>::from_stable_id(id)
        } else {
            let entity = self.create_detached_component(document, settings_sidebar_body())?;
            assembly.body = Some(entity.stable_id());
            entity
        };

        let mut tab_children = Vec::new();
        for tab in model.tabs() {
            let selected = state.active_tab() == tab.id();
            let row_state = if selected {
                SidebarRowState::Active
            } else {
                SidebarRowState::Idle
            };
            let mut leading_slot = assembly.tab_leadings.get(tab.id()).copied();
            let leading = ensure_settings_leading(
                self,
                document,
                &mut leading_slot,
                tab.icon_value(),
                selected,
            )?;
            assembly
                .tab_leadings
                .insert(tab.id().clone(), leading.stable_id());
            let mut row_slot = assembly.tab_rows.get(tab.id()).copied();
            let (row, created_row) = ensure_settings_nav_row(
                self,
                document,
                &mut row_slot,
                tab.label(),
                row_state,
                leading.stable_id(),
            )?;
            assembly.tab_rows.insert(tab.id().clone(), row.stable_id());
            if created_row {
                let tab_id = tab.id().clone();
                self.observe(row, sidebar, move |_, _: &Activate, cx| {
                    cx.emit(SettingsTabSelected {
                        tab: tab_id.clone(),
                    });
                })?;
            }
            tab_children.push(row.stable_id());
        }

        let body_changed = reconcile_children(self, body, &tab_children)?;
        self.update_component(sidebar, |sidebar, _| {
            sidebar.assembly = Some(assembly.clone());
        })?;
        let frame_changed =
            reconcile_children(self, sidebar, &[back.stable_id(), body.stable_id()])?;
        Ok(body_changed || frame_changed)
    }

    /// Mount header + scroll chrome, or a fill child when the tab is full-page.
    pub fn assemble_settings_page(
        &mut self,
        page: Entity<SettingsPage>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, page.stable_id())?;
        let (model, state, content, mut assembly) = self.read(page, |page| {
            (
                page.model.clone(),
                page.state.clone(),
                page.content,
                page.assembly.clone().unwrap_or_default(),
            )
        })?;
        let tab = state.active_view(&model);
        let full_page = tab.full_page_value();
        let show_header = !full_page && !model.hide_header_value();

        if show_header {
            sync_text(
                self,
                document,
                &mut assembly.title,
                page_title_text(tab.label()),
            )?;
        }

        if full_page {
            self.update_component(page, |page, _| {
                page.assembly = Some(assembly);
            })?;
            let ordered = content.into_iter().collect::<Vec<_>>();
            return reconcile_children(self, page, &ordered);
        }

        let body = if let Some(id) = assembly.body {
            Entity::<SettingsPageBody>::from_stable_id(id)
        } else {
            let entity = self.create_detached_component(document, SettingsPageBody)?;
            assembly.body = Some(entity.stable_id());
            entity
        };
        let scroll = if let Some(id) = assembly.scroll {
            Entity::<ScrollView>::from_stable_id(id)
        } else {
            let entity = self.create_detached_component(document, settings_page_scroll())?;
            assembly.scroll = Some(entity.stable_id());
            entity
        };

        let mut column_children = Vec::new();
        if show_header {
            if let Some(title) = assembly.title {
                column_children.push(title);
            }
        }
        if let Some(content) = content {
            column_children.push(content);
        }
        reconcile_children(self, body, &column_children)?;
        reconcile_children(self, scroll, &[body.stable_id()])?;
        let scroll_id = scroll.stable_id();
        self.update_component(page, |page, _| {
            page.assembly = Some(assembly);
        })?;
        reconcile_children(self, page, &[scroll_id])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::{AppContext, DocumentId, Entity, StandardVisual};
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

    fn hint_slot_of(context: &AppContext, row: StableNodeId) -> Option<StableNodeId> {
        context
            .read(Entity::<SettingsRow>::from_stable_id(row), |row| {
                row.hint_slot
            })
            .ok()
            .flatten()
    }

    fn visible_hint_text(context: &AppContext, row: StableNodeId) -> Option<String> {
        let hint = hint_slot_of(context, row)?;
        let style = context.world().node_style(hint)?;
        if style.layout.hidden {
            return None;
        }
        let text = context.world().text(hint)?.to_owned();
        (!text.is_empty()).then_some(text)
    }

    #[test]
    fn settings_row_paints_muted_hint_child() {
        let mut context = AppContext::new();
        let control = context
            .create_component(document(), crate::Switch::new("跟随", true))
            .unwrap();
        let label = context
            .create_component(document(), row_label_text("主题"))
            .unwrap();
        let hint = context
            .create_component(document(), row_hint_text("选择应用配色，立即生效", false))
            .unwrap();
        let row = context
            .create_component(
                document(),
                SettingsRow::new("主题")
                    .hint("选择应用配色，立即生效")
                    .label_slot(label.stable_id())
                    .hint_slot(hint.stable_id())
                    .control_child(control.stable_id()),
            )
            .unwrap();
        context.append_child(row, label).unwrap();
        context.append_child(row, hint).unwrap();
        context.append_child(row, control).unwrap();
        context.update_component(row, |_, _| {}).unwrap();

        assert_eq!(context.world().text(row.stable_id()), Some(""));
        assert_eq!(context.world().text(label.stable_id()), Some("主题"));
        let label_style = context.world().node_style(label.stable_id()).unwrap();
        assert_eq!(label_style.layout.font_size, Some(13.0));
        assert_eq!(label_style.layout.font_weight, Some(500));
        assert_eq!(label_style.foreground, Some(SemanticColorRole::Text));
        assert!(label_style.layout.white_space_nowrap);
        assert!(label_style.layout.text_overflow_ellipsis);
        assert_eq!(label_style.layout.width, Some(LengthSpec::Fill));

        assert_eq!(
            context.world().text(hint.stable_id()),
            Some("选择应用配色，立即生效")
        );
        let hint_style = context.world().node_style(hint.stable_id()).unwrap();
        assert_eq!(hint_style.layout.font_size, Some(12.0));
        assert_eq!(hint_style.foreground, Some(SemanticColorRole::Muted));
        assert!(hint_style.layout.white_space_nowrap);
        assert!(hint_style.layout.text_overflow_ellipsis);
        assert!(!hint_style.layout.hidden);
        assert_eq!(
            visible_hint_text(&context, row.stable_id()).as_deref(),
            Some("选择应用配色，立即生效")
        );
    }

    #[test]
    fn settings_row_without_hint_has_no_visible_hint_text() {
        let mut context = AppContext::new();
        let control = context
            .create_component(document(), crate::Switch::new("圆角", true))
            .unwrap();
        let label = context
            .create_component(document(), row_label_text("工作区边缘"))
            .unwrap();
        let leftover = context
            .create_component(document(), row_hint_text("不应显示", false))
            .unwrap();
        let row = context
            .create_component(
                document(),
                SettingsRow::new("工作区边缘")
                    .label_slot(label.stable_id())
                    .hint_slot(leftover.stable_id())
                    .control_child(control.stable_id()),
            )
            .unwrap();
        context.append_child(row, label).unwrap();
        context.append_child(row, leftover).unwrap();
        context.append_child(row, control).unwrap();
        context.update_component(row, |_, _| {}).unwrap();

        assert_eq!(context.world().text(row.stable_id()), Some(""));
        assert_eq!(context.world().text(leftover.stable_id()), Some(""));
        assert!(
            context
                .world()
                .node_style(leftover.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert_eq!(visible_hint_text(&context, row.stable_id()), None);
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

    fn row_labels(context: &AppContext, section: Entity<AppearanceSection>) -> Vec<String> {
        context
            .world()
            .node(section.stable_id())
            .unwrap()
            .children
            .iter()
            .filter_map(|id| {
                context
                    .read(Entity::<SettingsRow>::from_stable_id(*id), |row| {
                        row.label.to_string()
                    })
                    .ok()
            })
            .collect()
    }

    fn assembly_of(
        context: &AppContext,
        section: Entity<AppearanceSection>,
    ) -> AppearanceSectionAssembly {
        context
            .read(section, |section| section.assembly.clone().unwrap())
            .unwrap()
    }

    fn option_disabled(context: &AppContext, id: Option<StableNodeId>) -> bool {
        context
            .read(
                Entity::<SegmentedOption>::from_stable_id(id.unwrap()),
                |option| option.disabled,
            )
            .unwrap()
    }

    fn switch_disabled(context: &AppContext, id: Option<StableNodeId>) -> bool {
        context
            .read(Entity::<Switch>::from_stable_id(id.unwrap()), |switch| {
                switch.disabled
            })
            .unwrap()
    }

    #[test]
    fn appearance_assemble_creates_rows_and_disabled_rules() {
        let mut context = AppContext::new();
        let section = context
            .create_component(
                document(),
                AppearanceSection::new(ThemeMode::Dark, AppearanceSettings::default()),
            )
            .unwrap();
        assert!(context.assemble_appearance_section(section).unwrap());
        assert_eq!(
            row_labels(&context, section),
            [
                "主题",
                "窗口材质",
                "透明区域",
                "标题栏跟随侧边栏透明",
                "材质不透明度",
                "工作区边缘",
                "组件圆角半径",
                "默认样式",
            ]
        );
        let solid = assembly_of(&context, section);
        assert_eq!(
            visible_hint_text(&context, solid.theme_row.unwrap()).as_deref(),
            Some("选择应用配色，立即生效")
        );
        let theme_hint = hint_slot_of(&context, solid.theme_row.unwrap()).unwrap();
        let theme_hint_style = context.world().node_style(theme_hint).unwrap();
        assert_eq!(theme_hint_style.layout.font_size, Some(12.0));
        assert_eq!(theme_hint_style.foreground, Some(SemanticColorRole::Muted));
        assert_eq!(
            visible_hint_text(&context, solid.workspace_row.unwrap()),
            None
        );
        assert_eq!(visible_hint_text(&context, solid.radius_row.unwrap()), None);
        assert!(option_disabled(&context, solid.target_sidebar));
        assert!(option_disabled(&context, solid.target_main));
        assert!(switch_disabled(&context, solid.titlebar_switch));
        assert!(solid.opacity_text.is_some());
        assert!(solid.opacity_range.is_none());
        assert!(
            context
                .world()
                .text(solid.opacity_text.unwrap())
                .unwrap()
                .ends_with('%')
        );
        assert!(!switch_disabled(&context, solid.workspace_switch));

        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        context
            .on(section, move |_, event: &AppearanceEvent, _| {
                sink.lock().unwrap().push(*event);
            })
            .unwrap();
        assert!(
            context
                .toggle_switch(Entity::from_stable_id(solid.workspace_switch.unwrap()))
                .unwrap()
        );
        assert_eq!(
            *events.lock().unwrap(),
            vec![AppearanceEvent::WorkspaceCorners(false)]
        );

        let mut appearance = AppearanceSettings::default();
        appearance.set_window_material(WindowMaterialMode::Translucent);
        context
            .update_component(section, |section, _| {
                section.appearance = appearance;
                section.material_status = Some(Arc::from("vibrancy"));
            })
            .unwrap();
        context.assemble_appearance_section(section).unwrap();
        assert_eq!(
            row_labels(&context, section),
            [
                "主题",
                "窗口材质",
                "材质状态",
                "透明区域",
                "标题栏跟随侧边栏透明",
                "材质不透明度",
                "工作区边缘",
                "组件圆角半径",
                "默认样式",
            ]
        );
        let translucent_sidebar = assembly_of(&context, section);
        assert!(!option_disabled(
            &context,
            translucent_sidebar.target_sidebar
        ));
        assert!(!option_disabled(&context, translucent_sidebar.target_main));
        assert!(!switch_disabled(
            &context,
            translucent_sidebar.titlebar_switch
        ));
        assert!(translucent_sidebar.opacity_range.is_some());
        assert!(
            !context
                .world()
                .node_style(translucent_sidebar.opacity_range.unwrap())
                .unwrap()
                .layout
                .hidden
        );
        let range_parent = context
            .world()
            .node(translucent_sidebar.opacity_row.unwrap())
            .unwrap()
            .children
            .last()
            .copied();
        assert_eq!(range_parent, translucent_sidebar.opacity_range);

        appearance.set_backdrop_target(BackdropTarget::Main);
        context
            .update_component(section, |section, _| {
                section.appearance = appearance;
            })
            .unwrap();
        context.assemble_appearance_section(section).unwrap();
        let translucent_main = assembly_of(&context, section);
        assert!(!option_disabled(&context, translucent_main.target_sidebar));
        assert!(switch_disabled(&context, translucent_main.titlebar_switch));
    }

    #[test]
    fn about_section_shows_injected_metadata() {
        let mut context = AppContext::new();
        let metadata =
            AboutMetadata::new("Fixture Product", "9.9.9").description("Host-owned blurb");
        let section = context
            .create_component(document(), AboutSection::new(metadata.clone()))
            .unwrap();
        assert!(context.assemble_about_section(section).unwrap());
        let assembly = context
            .read(section, |section| section.assembly.clone().unwrap())
            .unwrap();
        assert_eq!(
            context.world().text(assembly.name_value.unwrap()),
            Some("Fixture Product")
        );
        assert_eq!(
            context.world().text(assembly.version_value.unwrap()),
            Some("9.9.9")
        );
        assert_eq!(
            context.world().text(assembly.description.unwrap()),
            Some("Host-owned blurb")
        );
        let labels = context
            .world()
            .node(section.stable_id())
            .unwrap()
            .children
            .iter()
            .filter_map(|id| {
                context
                    .read(Entity::<SettingsRow>::from_stable_id(*id), |row| {
                        row.label.to_string()
                    })
                    .ok()
            })
            .collect::<Vec<_>>();
        assert_eq!(labels, ["名称", "版本"]);
        assert_eq!(
            visible_hint_text(&context, assembly.name_row.unwrap()),
            None
        );
        assert_eq!(
            visible_hint_text(&context, assembly.version_row.unwrap()),
            None
        );
        let mut texts = Vec::new();
        let mut stack = context
            .world()
            .node(section.stable_id())
            .unwrap()
            .children
            .clone();
        while let Some(id) = stack.pop() {
            if let Some(text) = context.world().text(id) {
                texts.push(text.to_owned());
            }
            if let Some(node) = context.world().node(id) {
                stack.extend(node.children.iter().copied());
            }
        }
        assert!(texts.iter().any(|text| text == "Fixture Product"));
        assert!(texts.iter().all(|text| !text.contains("NanaUI")));
    }

    #[test]
    fn collapsible_card_hides_details_until_expanded() {
        let mut context = AppContext::new();
        let summary = context
            .create_component(document(), Text::new("高级"))
            .unwrap();
        let details = context
            .create_component(document(), Text::new("明细"))
            .unwrap();
        let card = context
            .create_component(
                document(),
                SettingsCollapsibleCard::new(false)
                    .summary(summary.stable_id())
                    .details(details.stable_id()),
            )
            .unwrap();
        assert!(context.assemble_settings_collapsible_card(card).unwrap());
        let (header, disclosure, divider) = context
            .read(card, |card| (card.header, card.disclosure, card.divider))
            .unwrap();
        assert_eq!(
            context.world().node(card.stable_id()).unwrap().children,
            vec![header.unwrap(), divider.unwrap(), details.stable_id()]
        );
        assert_eq!(
            context.world().node(header.unwrap()).unwrap().children,
            vec![summary.stable_id(), disclosure.unwrap()]
        );
        assert_eq!(
            context
                .world()
                .node_style(summary.stable_id())
                .unwrap()
                .foreground,
            Some(SemanticColorRole::Text)
        );
        assert_eq!(
            context.world().standard_visual(disclosure.unwrap()),
            Some(StandardVisual::Icon {
                icon: Icon::ChevronRight,
                size: DISCLOSURE_ICON_SIZE,
                tooltip: None,
            })
        );
        assert!(
            context
                .world()
                .node_style(disclosure.unwrap())
                .unwrap()
                .layout
                .transform
                .is_none()
        );
        assert!(
            context
                .world()
                .node_style(details.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert!(
            context
                .world()
                .node_style(divider.unwrap())
                .unwrap()
                .layout
                .hidden
        );
        assert!(context.activate_settings_collapsible_card(card).unwrap());
        assert_eq!(
            context.world().standard_visual(disclosure.unwrap()),
            Some(StandardVisual::Icon {
                icon: Icon::ChevronDown,
                size: DISCLOSURE_ICON_SIZE,
                tooltip: None,
            })
        );
        assert!(
            context
                .world()
                .node_style(disclosure.unwrap())
                .unwrap()
                .layout
                .transform
                .is_none()
        );
        assert!(
            !context
                .world()
                .node_style(details.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert!(
            !context
                .world()
                .node_style(divider.unwrap())
                .unwrap()
                .layout
                .hidden
        );

        let locked_details = context
            .create_component(document(), Text::new("锁定明细"))
            .unwrap();
        let locked = context
            .create_component(
                document(),
                SettingsCollapsibleCard::new(true)
                    .disabled(true)
                    .details(locked_details.stable_id()),
            )
            .unwrap();
        context.assemble_settings_collapsible_card(locked).unwrap();
        assert!(
            !context
                .world()
                .node_style(locked_details.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert!(!context.activate_settings_collapsible_card(locked).unwrap());
        context.read(locked, |card| assert!(card.expanded)).unwrap();
        assert!(
            !context
                .world()
                .node_style(locked_details.stable_id())
                .unwrap()
                .layout
                .hidden
        );
    }

    fn settings_model(tabs: impl IntoIterator<Item = nana_ui_core::SettingsTab>) -> SettingsModel {
        let tabs: Vec<_> = tabs.into_iter().collect();
        let default = tabs[0].id().clone();
        SettingsModel::new(default, tabs).expect("valid settings model")
    }

    fn is_descendant(context: &AppContext, root: StableNodeId, target: StableNodeId) -> bool {
        let mut current = Some(target);
        while let Some(id) = current {
            if id == root {
                return true;
            }
            current = context.world().node(id).and_then(|node| node.parent);
        }
        false
    }

    fn sidebar_row_state(context: &AppContext, id: StableNodeId) -> SidebarRowState {
        context
            .read(Entity::<SidebarRow>::from_stable_id(id), |row| row.state)
            .unwrap()
    }

    #[test]
    fn settings_sidebar_mounts_back_and_tab_rows() {
        let mut context = AppContext::new();
        let model = settings_model([
            nana_ui_core::SettingsTab::new("appearance", "外观").icon(Icon::Appearance),
            nana_ui_core::SettingsTab::new("workspace", "工作区").icon(Icon::Workspace),
        ]);
        let state = SettingsState::new(&model);
        let sidebar = context
            .create_component(document(), SettingsSidebar::new(model, state))
            .unwrap();
        assert!(context.assemble_settings_sidebar(sidebar).unwrap());
        let assembly = context
            .read(sidebar, |sidebar| sidebar.assembly.clone().unwrap())
            .unwrap();
        let back = assembly.back_row.unwrap();
        let body = assembly.body.unwrap();
        let appearance = assembly
            .tab_rows
            .get(&SettingsTabId::from("appearance"))
            .copied()
            .unwrap();
        let workspace = assembly
            .tab_rows
            .get(&SettingsTabId::from("workspace"))
            .copied()
            .unwrap();
        assert_eq!(
            context.world().node(sidebar.stable_id()).unwrap().children,
            vec![back, body]
        );
        assert_eq!(
            context.world().node(body).unwrap().kind,
            NodeKind::Element {
                tag: "scroll".into(),
            }
        );
        assert_eq!(
            context.world().node(body).unwrap().children,
            vec![appearance, workspace]
        );
        assert_eq!(context.world().text(back), Some("返回"));
        assert_eq!(
            sidebar_row_state(&context, appearance),
            SidebarRowState::Active
        );
        assert_eq!(
            sidebar_row_state(&context, workspace),
            SidebarRowState::Idle
        );
        assert_eq!(
            context
                .world()
                .node_style(sidebar.stable_id())
                .unwrap()
                .layout
                .gap,
            Some(LengthSpec::Px(SETTINGS_SIDEBAR_GAP))
        );
        assert!(
            context
                .world()
                .node_style(body)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
        assert!(
            !context
                .world()
                .node_style(back)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
    }

    #[test]
    fn settings_sidebar_activate_selects_tab_and_reassemble_marks_active() {
        let mut context = AppContext::new();
        let model = settings_model([
            nana_ui_core::SettingsTab::new("appearance", "外观"),
            nana_ui_core::SettingsTab::new("workspace", "工作区"),
        ]);
        let state = SettingsState::new(&model);
        let sidebar = context
            .create_component(document(), SettingsSidebar::new(model, state))
            .unwrap();
        context.assemble_settings_sidebar(sidebar).unwrap();
        let selected = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&selected);
        context
            .on(sidebar, move |_, event: &SettingsTabSelected, _| {
                sink.lock().unwrap().push(event.tab.clone());
            })
            .unwrap();
        let workspace = context
            .read(sidebar, |sidebar| {
                sidebar.row_for_tab(&SettingsTabId::from("workspace"))
            })
            .unwrap()
            .unwrap();
        assert!(
            context
                .activate_sidebar_row(Entity::from_stable_id(workspace))
                .unwrap()
        );
        assert_eq!(
            *selected.lock().unwrap(),
            vec![SettingsTabId::from("workspace")]
        );
        context
            .update_component(sidebar, |sidebar, _| {
                let model = sidebar.model.clone();
                sidebar
                    .state
                    .select(&model, &SettingsTabId::from("workspace"));
            })
            .unwrap();
        context.assemble_settings_sidebar(sidebar).unwrap();
        let appearance = context
            .read(sidebar, |sidebar| {
                sidebar.row_for_tab(&SettingsTabId::from("appearance"))
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            sidebar_row_state(&context, appearance),
            SidebarRowState::Idle
        );
        assert_eq!(
            sidebar_row_state(&context, workspace),
            SidebarRowState::Active
        );
    }

    #[test]
    fn settings_sidebar_missing_icon_is_spacer_without_glyph() {
        let mut context = AppContext::new();
        let model = settings_model([
            nana_ui_core::SettingsTab::new("appearance", "外观").icon(Icon::Appearance),
            nana_ui_core::SettingsTab::new("about", "关于"),
        ]);
        let state = SettingsState::new(&model);
        let sidebar = context
            .create_component(document(), SettingsSidebar::new(model, state))
            .unwrap();
        context.assemble_settings_sidebar(sidebar).unwrap();
        let assembly = context
            .read(sidebar, |sidebar| sidebar.assembly.clone().unwrap())
            .unwrap();
        let icon = assembly
            .tab_leadings
            .get(&SettingsTabId::from("appearance"))
            .copied()
            .unwrap();
        let spacer = assembly
            .tab_leadings
            .get(&SettingsTabId::from("about"))
            .copied()
            .unwrap();
        assert_eq!(
            context.world().standard_visual(icon),
            Some(StandardVisual::Icon {
                icon: Icon::Appearance,
                size: SETTINGS_SIDEBAR_ICON_SIZE,
                tooltip: None,
            })
        );
        assert_eq!(context.world().standard_visual(spacer), None);
        let spacer_layout = &context.world().node_style(spacer).unwrap().layout;
        assert_eq!(
            spacer_layout.width,
            Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE))
        );
        assert_eq!(
            spacer_layout.height,
            Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE))
        );
    }

    #[test]
    fn settings_page_header_title_and_scroll_wraps_content() {
        let mut context = AppContext::new();
        let model = settings_model([nana_ui_core::SettingsTab::new("appearance", "外观")]);
        let state = SettingsState::new(&model);
        let content = context
            .create_component(document(), Text::new("内容"))
            .unwrap();
        let page = context
            .create_component(
                document(),
                SettingsPage::new(model, state).content(content.stable_id()),
            )
            .unwrap();
        assert!(context.assemble_settings_page(page).unwrap());
        let assembly = context
            .read(page, |page| page.assembly.clone().unwrap())
            .unwrap();
        let scroll = assembly.scroll.unwrap();
        let body = assembly.body.unwrap();
        let title = assembly.title.unwrap();
        assert_eq!(
            context.world().node(page.stable_id()).unwrap().children,
            vec![scroll]
        );
        assert_eq!(
            context.world().node(scroll).unwrap().kind,
            NodeKind::Element {
                tag: "scroll".into(),
            }
        );
        assert!(
            context
                .world()
                .node_style(scroll)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
        assert_eq!(context.world().node(scroll).unwrap().children, vec![body]);
        assert_eq!(
            context.world().node(body).unwrap().children,
            vec![title, content.stable_id()]
        );
        assert_eq!(context.world().text(title), Some("外观"));
        let title_style = context.world().node_style(title).unwrap();
        assert_eq!(title_style.layout.font_size, Some(SETTINGS_PAGE_TITLE_SIZE));
        assert_eq!(
            title_style.layout.font_weight,
            Some(SETTINGS_PAGE_TITLE_WEIGHT)
        );
        assert_eq!(title_style.foreground, Some(SemanticColorRole::Text));
        assert_eq!(
            context
                .world()
                .node_style(page.stable_id())
                .unwrap()
                .background,
            Some(SemanticColorRole::Background)
        );
    }

    #[test]
    fn settings_page_full_page_fills_with_content_only() {
        let mut context = AppContext::new();
        let model =
            settings_model([nana_ui_core::SettingsTab::new("workspace", "工作区").full_page(true)]);
        let state = SettingsState::new(&model);
        let content = context
            .create_component(document(), Text::new("整页"))
            .unwrap();
        let page = context
            .create_component(
                document(),
                SettingsPage::new(model, state).content(content.stable_id()),
            )
            .unwrap();
        assert!(context.assemble_settings_page(page).unwrap());
        let assembly = context
            .read(page, |page| page.assembly.clone().unwrap())
            .unwrap();
        assert_eq!(
            context.world().node(page.stable_id()).unwrap().children,
            vec![content.stable_id()]
        );
        assert!(
            assembly.title.is_none()
                || !is_descendant(&context, page.stable_id(), assembly.title.unwrap())
        );
        if let Some(scroll) = assembly.scroll {
            assert!(!is_descendant(&context, page.stable_id(), scroll));
        }
        assert!(!is_descendant(
            &context,
            content.stable_id(),
            page.stable_id()
        ));
    }

    #[test]
    fn settings_page_hide_header_omits_title() {
        let mut context = AppContext::new();
        let model = settings_model([nana_ui_core::SettingsTab::new("appearance", "外观")])
            .hide_header(true);
        let state = SettingsState::new(&model);
        let content = context
            .create_component(document(), Text::new("内容"))
            .unwrap();
        let page = context
            .create_component(
                document(),
                SettingsPage::new(model, state).content(content.stable_id()),
            )
            .unwrap();
        assert!(context.assemble_settings_page(page).unwrap());
        let assembly = context
            .read(page, |page| page.assembly.clone().unwrap())
            .unwrap();
        let body = assembly.body.unwrap();
        assert_eq!(
            context.world().node(body).unwrap().children,
            vec![content.stable_id()]
        );
        assert!(
            assembly.title.is_none()
                || !is_descendant(&context, page.stable_id(), assembly.title.unwrap())
        );
        assert_eq!(
            context.world().node(page.stable_id()).unwrap().children,
            vec![assembly.scroll.unwrap()]
        );
    }
}
