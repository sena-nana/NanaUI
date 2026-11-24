use std::collections::HashMap;
use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, Icon, LengthSpec, OverflowSpec, PaddingSpec,
    SemanticColorRole, SettingsModel, SettingsState, SettingsTabId,
};

use super::row::{styled_text, sync_text};
use super::{document_of, reconcile_children};
use crate::view_components::{Text, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, Activate, AppContext, ComponentView, Entity,
    FrameworkError, InteractionState, ListItemSlots, MutationQueue, NodeKind, NodeStyle,
    ScrollAxes, ScrollView, SidebarFrame, SidebarRow, SidebarRowState, StableNodeId, TextContent,
    UiWorld,
};

pub(super) const SETTINGS_SIDEBAR_GAP: f32 = nana_ui_core::space::XL;
const SETTINGS_SIDEBAR_TAB_GAP: f32 = nana_ui_core::space::XXS;
pub(super) const SETTINGS_SIDEBAR_ICON_SIZE: f32 = ControlSize::Small.icon_size();
const SETTINGS_PAGE_GAP: f32 = nana_ui_core::space::XXXL;
/// Tighter than the other three: the page sits under a title row that already
/// contributes its own leading.
const SETTINGS_PAGE_PADDING_TOP: f32 = nana_ui_core::space::PAGE_TIGHT;
const SETTINGS_PAGE_PADDING_RIGHT: f32 = nana_ui_core::space::PAGE;
const SETTINGS_PAGE_PADDING_BOTTOM: f32 = nana_ui_core::space::PAGE;
const SETTINGS_PAGE_PADDING_LEFT: f32 = nana_ui_core::space::PAGE;

/// Host-owned navigation snapshot. Activate emits [`SettingsBack`] / [`SettingsTabSelected`].
#[derive(Debug, Clone, PartialEq)]
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

/// SettingsSidebar is always hosted inside the shell's wrapping SidebarFrame,
/// which supplies the standard frame padding; this frame must stay padding-free.
fn settings_sidebar_frame_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.padding_top = Some(LengthSpec::Px(0.0));
    layout.padding_right = Some(LengthSpec::Px(0.0));
    layout.padding_bottom = Some(LengthSpec::Px(0.0));
    layout.padding_left = Some(LengthSpec::Px(0.0));
    style
}

impl ComponentView for SettingsSidebar {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(AppContext::assemble_settings_sidebar),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-sidebar".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut frame = SidebarFrame::new()
            .gap(SETTINGS_SIDEBAR_GAP)
            .style(settings_sidebar_frame_style());
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
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsPage {
    pub model: SettingsModel,
    pub state: SettingsState,
    pub content: Option<StableNodeId>,
    /// Insets of the internal scrolling body; None uses the standard page frame.
    pub content_padding: Option<PaddingSpec>,
    pub content_gap: Option<f32>,
    /// Font size of the page title; `None` is the settings title size.
    pub title_size: Option<f32>,
    /// Font weight of the page title; `None` is semibold.
    pub title_weight: Option<u16>,
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
            content_padding: None,
            content_gap: None,
            title_size: None,
            title_weight: None,
            assembly: None,
        }
    }

    /// The page title's font size, kept through every assembly (a tab
    /// switch re-assembles the page).
    pub fn title_size(mut self, size: f32) -> Self {
        self.title_size = Some(size.max(1.0));
        self
    }

    /// The page title's font weight, kept through every assembly.
    pub fn title_weight(mut self, weight: u16) -> Self {
        self.title_weight = Some(weight.clamp(1, 1000));
        self
    }

    pub fn content_padding(mut self, padding: PaddingSpec) -> Self {
        self.content_padding = Some(PaddingSpec {
            top: padding.top.max(0.0),
            right: padding.right.max(0.0),
            bottom: padding.bottom.max(0.0),
            left: padding.left.max(0.0),
        });
        self
    }

    pub fn content_gap(mut self, gap: f32) -> Self {
        self.content_gap = Some(gap.max(0.0));
        self
    }

    pub fn content(mut self, content: StableNodeId) -> Self {
        self.content = Some(content);
        self
    }
}

impl ComponentView for SettingsPage {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::SETTINGS_PAGE.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(AppContext::assemble_settings_page),
        ..crate::TypeBehavior::NONE
    };

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
        let extent = nana_ui_core::ThemeToken::Type(nana_ui_core::TypeRole::Meta);
        layout.theme.width = Some(extent.into());
        layout.theme.height = Some(extent.into());
        layout.theme.min_width = Some(extent.into());
        layout.theme.min_height = Some(extent.into());
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        style
    }
}

impl ComponentView for SettingsSidebarLeading {
    fn wants_metrics_reproject() -> bool {
        true
    }

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
                    value: String::new().into(),
                },
            );
        }
        let visual = self.icon.map(|icon| crate::IconVisual {
            icon,
            size: world.node_theme(id).typography().meta,
            tooltip: None,
        });
        if world.widget_visual_as::<crate::IconVisual>(id) != visual.as_ref() {
            mutations.set_widget_visual(id, visual);
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
struct SettingsPageBody {
    padding: Option<PaddingSpec>,
    gap: Option<f32>,
}

impl SettingsPageBody {
    fn style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(LengthSpec::Fill);
        layout.gap = Some(LengthSpec::Px(self.gap.unwrap_or(SETTINGS_PAGE_GAP)));
        if self.gap.is_none() {
            layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxxl));
        }
        if self.padding.is_none() {
            layout.theme.padding_top = Some(crate::theme_intent::space(
                nana_ui_core::SpacingStep::PageTight,
            ));
            layout.theme.padding_right =
                Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Page));
            layout.theme.padding_bottom =
                Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Page));
            layout.theme.padding_left =
                Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Page));
        }
        let padding = self.padding.unwrap_or(PaddingSpec {
            top: SETTINGS_PAGE_PADDING_TOP,
            right: SETTINGS_PAGE_PADDING_RIGHT,
            bottom: SETTINGS_PAGE_PADDING_BOTTOM,
            left: SETTINGS_PAGE_PADDING_LEFT,
        });
        layout.padding_top = Some(LengthSpec::Px(padding.top));
        layout.padding_right = Some(LengthSpec::Px(padding.right));
        layout.padding_bottom = Some(LengthSpec::Px(padding.bottom));
        layout.padding_left = Some(LengthSpec::Px(padding.left));
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
    layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxs));
    scroll
}

fn page_title_text(label: &str, size: Option<f32>, weight: Option<u16>) -> Text {
    let mut text = styled_text(
        label,
        SemanticColorRole::Text,
        nana_ui_core::TypeRole::Title,
        nana_ui_core::TextWeight::Semibold,
    );
    if let Some(size) = size {
        text = text.font_size(size);
    }
    if let Some(weight) = weight {
        text = text.font_weight(weight);
    }
    text
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

impl AppContext {
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
        let strings = self.world().framework_strings().clone();
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
            strings.settings_back.as_ref(),
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
        let (model, state, content, padding, gap, title_font, mut assembly) =
            self.read(page, |page| {
                (
                    page.model.clone(),
                    page.state.clone(),
                    page.content,
                    page.content_padding,
                    page.content_gap,
                    (page.title_size, page.title_weight),
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
                page_title_text(tab.label(), title_font.0, title_font.1),
            )?;
        }

        if full_page {
            self.update_component(page, |page, _| {
                page.assembly = Some(assembly);
            })?;
            let ordered = content.into_iter().collect::<Vec<_>>();
            return reconcile_children(self, page, &ordered);
        }

        let body_style = SettingsPageBody { padding, gap };
        let body = if let Some(id) = assembly.body {
            let entity = Entity::<SettingsPageBody>::from_stable_id(id);
            self.update_component(entity, |body, _| *body = body_style)?;
            entity
        } else {
            let entity = self.create_detached_component(document, body_style)?;
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
        if show_header && let Some(title) = assembly.title {
            column_children.push(title);
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
