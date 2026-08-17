use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, Icon, JustifySpec, LengthSpec, OverflowSpec,
    PositionSpec, RegionId, SemanticColorRole, TITLE_BAR_HEIGHT, UI_METRICS, WorkspaceModel,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, DocumentId, Entity,
    FrameworkError, InteractionState, InteractionStyle, MutationQueue, NodeKind, NodeStyle,
    OverlayHost, SemanticPaint, SidebarFrame, StableNodeId, StandardVisual, TextContent, UiWorld,
    Workspace, WorkspaceRegionSlot,
};

const SLOT_PADDING: f32 = 6.0;
const CENTER_PADDING_X: f32 = 14.0;
const DEFAULT_CENTER_WIDTH: f32 = 168.0;
const CONTROL_GAP: f32 = 2.0;
const TITLE_FONT_SIZE: f32 = 13.0;
const TITLE_FONT_WEIGHT: u16 = 600;
const OVERLAY_Z_INDEX: i32 = 1;

/// Typed window command. Runtime never touches a platform window handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowChromeAction {
    Minimize,
    ToggleMaximize,
    Close,
}

impl WindowChromeAction {
    pub const ALL: [Self; 3] = [Self::Minimize, Self::ToggleMaximize, Self::Close];

    pub fn icon(self, maximized: bool) -> Icon {
        match self {
            Self::Minimize => Icon::Minimize,
            Self::ToggleMaximize if maximized => Icon::Restore,
            Self::ToggleMaximize => Icon::Maximize,
            Self::Close => Icon::Close,
        }
    }

    pub fn label(self, maximized: bool) -> &'static str {
        match self {
            Self::Minimize => "Minimize",
            Self::ToggleMaximize if maximized => "Restore",
            Self::ToggleMaximize => "Maximize",
            Self::Close => "Close",
        }
    }
}

/// 36px application title bar. Leading / center / trailing / controls are host-mounted.
#[derive(Debug, Clone, PartialEq)]
pub struct AppTitleBar {
    pub title: Arc<str>,
    pub leading: Option<StableNodeId>,
    pub center: Option<StableNodeId>,
    pub trailing: Option<StableNodeId>,
    pub controls: Option<StableNodeId>,
    pub center_width: f32,
    pub leading_inset: f32,
    pub trailing_inset: f32,
    pub show_window_controls: bool,
    pub maximized: bool,
    pub style: NodeStyle,
}

impl AppTitleBar {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            leading: None,
            center: None,
            trailing: None,
            controls: None,
            center_width: DEFAULT_CENTER_WIDTH,
            leading_inset: 0.0,
            trailing_inset: 0.0,
            show_window_controls: false,
            maximized: false,
            style: NodeStyle::default(),
        }
    }

    pub fn leading(mut self, leading: StableNodeId) -> Self {
        self.leading = Some(leading);
        self
    }

    pub fn center(mut self, center: StableNodeId) -> Self {
        self.center = Some(center);
        self
    }

    pub fn trailing(mut self, trailing: StableNodeId) -> Self {
        self.trailing = Some(trailing);
        self
    }

    pub fn controls(mut self, controls: StableNodeId) -> Self {
        self.controls = Some(controls);
        self
    }

    pub fn center_width(mut self, width: f32) -> Self {
        self.center_width = finite_positive(width, DEFAULT_CENTER_WIDTH).max(1.0);
        self
    }

    pub fn leading_inset(mut self, inset: f32) -> Self {
        self.leading_inset = valid_inset(inset);
        self
    }

    pub fn trailing_inset(mut self, inset: f32) -> Self {
        self.trailing_inset = valid_inset(inset);
        self
    }

    pub fn show_window_controls(mut self, show: bool) -> Self {
        self.show_window_controls = show;
        self
    }

    pub fn maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn resolved_center_width(&self) -> f32 {
        finite_positive(self.center_width, DEFAULT_CENTER_WIDTH).max(1.0)
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        style.background = Some(SemanticColorRole::Surface);
        style.text_horizontal_alignment = crate::TextHorizontalAlignment::Center;
        style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::Start;
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(TITLE_BAR_HEIGHT));
        layout.min_height = Some(LengthSpec::Px(TITLE_BAR_HEIGHT));
        layout.max_height = Some(LengthSpec::Px(TITLE_BAR_HEIGHT));
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        layout.font_size = Some(TITLE_FONT_SIZE);
        layout.font_weight = Some(TITLE_FONT_WEIGHT);
        layout.overflow_x = OverflowSpec::Hidden;
        style
    }

    fn project_slots(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        if let Some(leading) = self.leading {
            patch_layout(world, mutations, leading, |layout| {
                apply_fill_slot(layout, AlignSpec::Center, JustifySpec::Start);
                layout.padding_left = Some(LengthSpec::Px(
                    SLOT_PADDING + valid_inset(self.leading_inset),
                ));
                layout.padding_right = Some(LengthSpec::Px(SLOT_PADDING));
                layout.padding_top = Some(LengthSpec::Px(0.0));
                layout.padding_bottom = Some(LengthSpec::Px(0.0));
            });
        }
        if let Some(center) = self.center {
            let width = self.resolved_center_width();
            patch_layout(world, mutations, center, |layout| {
                layout.direction = Some(FlexDirection::Row);
                layout.align_items = AlignSpec::Center;
                layout.justify_content = JustifySpec::Center;
                layout.width = Some(LengthSpec::Px(width));
                layout.min_width = Some(LengthSpec::Px(width));
                layout.max_width = Some(LengthSpec::Px(width));
                layout.height = Some(LengthSpec::Fill);
                layout.flex_grow = Some(0.0);
                layout.flex_shrink = Some(0.0);
                layout.padding_left = Some(LengthSpec::Px(CENTER_PADDING_X));
                layout.padding_right = Some(LengthSpec::Px(CENTER_PADDING_X));
                layout.padding_top = Some(LengthSpec::Px(0.0));
                layout.padding_bottom = Some(LengthSpec::Px(0.0));
                layout.overflow_x = OverflowSpec::Hidden;
                layout.overflow_y = OverflowSpec::Hidden;
                layout.white_space_nowrap = true;
                layout.text_overflow_ellipsis = true;
            });
        }
        if let Some(trailing) = self.trailing {
            patch_layout(world, mutations, trailing, |layout| {
                apply_fill_slot(layout, AlignSpec::Center, JustifySpec::End);
                layout.padding_left = Some(LengthSpec::Px(SLOT_PADDING));
                layout.padding_right = Some(LengthSpec::Px(
                    SLOT_PADDING + valid_inset(self.trailing_inset),
                ));
                layout.padding_top = Some(LengthSpec::Px(0.0));
                layout.padding_bottom = Some(LengthSpec::Px(0.0));
            });
        }
        self.project_controls(world, mutations);
    }

    fn project_controls(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        let Some(controls) = self.controls else {
            return;
        };
        if !self.show_window_controls {
            patch_layout(world, mutations, controls, |layout| {
                layout.hidden = true;
            });
            return;
        }
        AppTitleBarControls::new(self.maximized).project(controls, world, mutations);
    }
}

impl ComponentView for AppTitleBar {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "app-title-bar".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let root_text = if self.center.is_some() {
            ""
        } else {
            self.title.as_ref()
        };
        if world.text(id) != Some(root_text) {
            mutations.set_text(
                id,
                TextContent {
                    value: root_text.to_owned(),
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
                label: Some(Arc::clone(&self.title)),
                ..AccessibilityState::default()
            },
        );
        self.project_slots(world, mutations);
    }
}

/// Host-mounted Minimize / Maximize-or-Restore / Close icons.
#[derive(Debug, Clone, PartialEq)]
pub struct AppTitleBarControls {
    pub maximized: bool,
    pub minimize: Option<StableNodeId>,
    pub maximize: Option<StableNodeId>,
    pub close: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl AppTitleBarControls {
    pub fn new(maximized: bool) -> Self {
        Self {
            maximized,
            minimize: None,
            maximize: None,
            close: None,
            style: NodeStyle::default(),
        }
    }

    pub fn minimize(mut self, minimize: StableNodeId) -> Self {
        self.minimize = Some(minimize);
        self
    }

    pub fn maximize(mut self, maximize: StableNodeId) -> Self {
        self.maximize = Some(maximize);
        self
    }

    pub fn close(mut self, close: StableNodeId) -> Self {
        self.close = Some(close);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::End;
        layout.gap = Some(layout.gap.unwrap_or(LengthSpec::Px(CONTROL_GAP)));
        layout.height = Some(LengthSpec::Fill);
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        layout.hidden = false;
        style
    }

    fn control_ids(&self, world: &UiWorld, id: StableNodeId) -> [Option<StableNodeId>; 3] {
        let explicit = [self.minimize, self.maximize, self.close];
        if explicit.iter().any(Option::is_some) {
            return explicit;
        }
        let children = world.node(id).map(|node| node.children).unwrap_or_default();
        [
            children.first().copied(),
            children.get(1).copied(),
            children.get(2).copied(),
        ]
    }
}

impl Default for AppTitleBarControls {
    fn default() -> Self {
        Self::new(false)
    }
}

impl ComponentView for AppTitleBarControls {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "app-title-bar-controls".into(),
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
                ..AccessibilityState::default()
            },
        );
        for (action, child) in WindowChromeAction::ALL
            .into_iter()
            .zip(self.control_ids(world, id))
        {
            let Some(child) = child else {
                continue;
            };
            project_window_control(child, action, self.maximized, world, mutations);
        }
    }
}

/// Title bar + fill body + optional overlay stack sibling.
#[derive(Debug, Clone, PartialEq)]
pub struct AppShell {
    pub title_bar: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub overlay: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl AppShell {
    pub fn new() -> Self {
        Self {
            title_bar: None,
            body: None,
            overlay: None,
            style: NodeStyle::default(),
        }
    }

    pub fn title_bar(mut self, title_bar: StableNodeId) -> Self {
        self.title_bar = Some(title_bar);
        self
    }

    pub fn body(mut self, body: StableNodeId) -> Self {
        self.body = Some(body);
        self
    }

    pub fn overlay(mut self, overlay: StableNodeId) -> Self {
        self.overlay = Some(overlay);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        style.background = Some(SemanticColorRole::Background);
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.justify_content = JustifySpec::Start;
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        layout.position = PositionSpec::Relative;
        style
    }

    fn project_slots(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        if let Some(title_bar) = self.title_bar {
            patch_layout(world, mutations, title_bar, |layout| {
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(LengthSpec::Px(TITLE_BAR_HEIGHT));
                layout.min_height = Some(LengthSpec::Px(TITLE_BAR_HEIGHT));
                layout.flex_grow = Some(0.0);
                layout.flex_shrink = Some(0.0);
            });
        }
        if let Some(body) = self.body {
            patch_layout(world, mutations, body, |layout| {
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(LengthSpec::Fill);
                layout.min_height = Some(LengthSpec::Px(0.0));
                layout.flex_grow = Some(1.0);
                layout.flex_shrink = Some(1.0);
                layout.position = PositionSpec::Static;
            });
        }
        if let Some(overlay) = self.overlay {
            patch_layout(world, mutations, overlay, |layout| {
                layout.position = PositionSpec::Absolute;
                layout.offset_top = Some(LengthSpec::Px(0.0));
                layout.offset_right = Some(LengthSpec::Px(0.0));
                layout.offset_bottom = Some(LengthSpec::Px(0.0));
                layout.offset_left = Some(LengthSpec::Px(0.0));
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(LengthSpec::Fill);
                layout.flex_grow = Some(0.0);
                layout.flex_shrink = Some(0.0);
                layout.z_index = Some(layout.z_index.unwrap_or(OVERLAY_Z_INDEX));
            });
            let has_content = world
                .node(overlay)
                .is_some_and(|node| !node.children.is_empty());
            let interaction = InteractionState {
                pointer_events: has_content,
                focusable: false,
            };
            if world.interaction(overlay) != Some(interaction) {
                mutations.set_interaction(overlay, interaction);
            }
        }
    }
}

impl Default for AppShell {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for AppShell {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "app-shell".into(),
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
                ..AccessibilityState::default()
            },
        );
        self.project_slots(world, mutations);
    }
}

/// Page-level composer equivalent to Iced `DesktopShell`.
///
/// Wires host-mounted [`AppTitleBar`], [`Workspace`], [`AppShell`] layout, and
/// [`OverlayHost`]. Application content stays in the supplied region slots.
#[derive(Debug, Clone)]
pub struct DesktopShell {
    pub title_bar: Option<StableNodeId>,
    pub workspace: Option<StableNodeId>,
    pub overlay: Option<StableNodeId>,
    pub primary: Option<StableNodeId>,
    pub navigation: Option<StableNodeId>,
    pub navigation_footer: Option<StableNodeId>,
    pub inspector: Option<StableNodeId>,
    pub bottom: Option<StableNodeId>,
    pub extra_regions: Vec<(RegionId, StableNodeId)>,
    pub overlays: Vec<StableNodeId>,
    pub navigation_frame: Option<StableNodeId>,
    pub title: Option<Arc<str>>,
    pub title_leading: Option<StableNodeId>,
    pub title_center: Option<StableNodeId>,
    pub title_trailing: Option<StableNodeId>,
    pub model: WorkspaceModel,
    pub style: NodeStyle,
}

impl DesktopShell {
    pub fn new() -> Self {
        Self {
            title_bar: None,
            workspace: None,
            overlay: None,
            primary: None,
            navigation: None,
            navigation_footer: None,
            inspector: None,
            bottom: None,
            extra_regions: Vec::new(),
            overlays: Vec::new(),
            navigation_frame: None,
            title: None,
            title_leading: None,
            title_center: None,
            title_trailing: None,
            model: WorkspaceModel::new(),
            style: NodeStyle::default(),
        }
    }

    pub fn from_model(model: WorkspaceModel) -> Self {
        Self {
            model,
            ..Self::new()
        }
    }

    pub fn title_bar(mut self, title_bar: StableNodeId) -> Self {
        self.title_bar = Some(title_bar);
        self
    }

    /// Create an [`AppTitleBar`] on assemble when no host-mounted bar is set.
    pub fn title(mut self, title: impl Into<Arc<str>>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn title_leading(mut self, leading: StableNodeId) -> Self {
        self.title_leading = Some(leading);
        self
    }

    pub fn title_center(mut self, center: StableNodeId) -> Self {
        self.title_center = Some(center);
        self
    }

    pub fn title_trailing(mut self, trailing: StableNodeId) -> Self {
        self.title_trailing = Some(trailing);
        self
    }

    pub fn primary(mut self, primary: StableNodeId) -> Self {
        self.primary = Some(primary);
        self
    }

    pub fn navigation(mut self, navigation: StableNodeId) -> Self {
        self.navigation = Some(navigation);
        self
    }

    pub fn navigation_footer(mut self, footer: StableNodeId) -> Self {
        self.navigation_footer = Some(footer);
        self
    }

    pub fn inspector(mut self, inspector: StableNodeId) -> Self {
        self.inspector = Some(inspector);
        self
    }

    pub fn bottom(mut self, bottom: StableNodeId) -> Self {
        self.bottom = Some(bottom);
        self
    }

    pub fn region(mut self, id: RegionId, content: StableNodeId) -> Self {
        if let Some((_, existing)) = self
            .extra_regions
            .iter_mut()
            .find(|(region, _)| *region == id)
        {
            *existing = content;
        } else {
            self.extra_regions.push((id, content));
        }
        self
    }

    /// Push an overlay child onto the assembled [`OverlayHost`].
    pub fn overlay(mut self, overlay: StableNodeId) -> Self {
        self.overlays.push(overlay);
        self
    }

    pub fn workspace_model(mut self, model: WorkspaceModel) -> Self {
        self.model = model;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl Default for DesktopShell {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for DesktopShell {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "desktop-shell".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        AppShell {
            title_bar: self.title_bar,
            body: self.workspace,
            overlay: self.overlay,
            style: self.style.clone(),
        }
        .project(id, world, mutations);
    }
}

impl AppContext {
    /// Reconcile title bar, body, and optional overlay, then re-project slots.
    ///
    /// Host-mounted slots are kept. A title bar or overlay node is created
    /// only when that slot was already assigned or already present as a child.
    pub fn assemble_app_shell(&mut self, shell: Entity<AppShell>) -> Result<bool, FrameworkError> {
        let parent = shell.stable_id();
        let document = document_of(self, parent)?;
        let snapshot = self.read(shell, Clone::clone)?;
        let title_bar = resolve_app_title_bar(self, document, parent, &snapshot)?;
        let body = snapshot.body.filter(|id| self.world().contains(*id));
        let overlay = resolve_app_overlay(self, document, parent, &snapshot)?;
        let fields_changed =
            title_bar != snapshot.title_bar || body != snapshot.body || overlay != snapshot.overlay;
        if fields_changed {
            self.update_component(shell, |shell, _| {
                shell.title_bar = title_bar;
                shell.body = body;
                shell.overlay = overlay;
            })?;
        }
        let mut children = Vec::new();
        if let Some(title_bar) = title_bar {
            children.push(title_bar);
        }
        if let Some(body) = body {
            children.push(body);
        }
        if let Some(overlay) = overlay {
            children.push(overlay);
        }
        let changed = reconcile_ids(self, parent, &children)?;
        self.update_component(shell, |_, _| {})?;
        Ok(changed || fields_changed)
    }

    /// Mount title bar, workspace regions, and overlay host on `shell`.
    ///
    /// Created chrome is reused on the next call. Host content nodes are
    /// reparented, not recreated. Floating dock windows are out of scope.
    pub fn assemble_desktop_shell(
        &mut self,
        shell: Entity<DesktopShell>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, shell.stable_id())?;
        let snapshot = self.read(shell, Clone::clone)?;
        let previous_slots = snapshot
            .workspace
            .filter(|id| view_is::<Workspace>(self, *id))
            .map(|id| {
                self.read(Entity::<Workspace>::from_stable_id(id), |workspace| {
                    workspace.slots.clone()
                })
            })
            .transpose()?
            .unwrap_or_default();
        let previous_used = used_ids(&snapshot, &previous_slots);

        let workspace_id = ensure_workspace(self, document, snapshot.workspace)?;
        let overlay_id = ensure_overlay_host(self, document, snapshot.overlay)?;
        let title_bar = ensure_title_bar(
            self,
            document,
            snapshot.title_bar,
            snapshot.title.as_ref(),
            snapshot.title_leading,
            snapshot.title_center,
            snapshot.title_trailing,
        )?;
        let (resources, navigation_frame) = resolve_navigation(
            self,
            document,
            snapshot.navigation,
            snapshot.navigation_footer,
            snapshot.navigation_frame,
        )?;

        let mut assembled = snapshot;
        assembled.title_bar = title_bar;
        assembled.workspace = Some(workspace_id);
        assembled.overlay = Some(overlay_id);
        assembled.navigation_frame = navigation_frame;
        let slots = region_slots(self, &assembled, resources);
        let next_used = used_ids(&assembled, &slots);
        park_unused(self, shell.stable_id(), &previous_used, &next_used)?;

        self.update_component(shell, |desktop, _| {
            desktop.title_bar = title_bar;
            desktop.workspace = Some(workspace_id);
            desktop.overlay = Some(overlay_id);
            desktop.navigation_frame = navigation_frame;
        })?;
        let mut shell_children = Vec::new();
        if let Some(title_bar) = title_bar {
            shell_children.push(title_bar);
        }
        shell_children.push(workspace_id);
        shell_children.push(overlay_id);
        let mut changed = reconcile_ids(self, shell.stable_id(), &shell_children)?;

        let workspace = Entity::<Workspace>::from_stable_id(workspace_id);
        self.update_component(workspace, |workspace, _| {
            workspace.refresh_from_model(&assembled.model);
            workspace.slots = slots;
        })?;
        changed |= self.assemble_workspace(workspace)?;
        changed |= reconcile_ids(self, overlay_id, &assembled.overlays)?;
        self.update_component(shell, |_, _| {})?;
        Ok(changed)
    }
}

fn document_of(context: &AppContext, id: StableNodeId) -> Result<DocumentId, FrameworkError> {
    context
        .world()
        .node(id)
        .map(|node| node.document)
        .ok_or(FrameworkError::MissingView(id))
}

fn view_is<C: ComponentView>(context: &AppContext, id: StableNodeId) -> bool {
    context
        .read(Entity::<C>::from_stable_id(id), |_| ())
        .is_ok()
}

fn resolve_app_title_bar(
    context: &mut AppContext,
    document: DocumentId,
    parent: StableNodeId,
    snapshot: &AppShell,
) -> Result<Option<StableNodeId>, FrameworkError> {
    if let Some(id) = snapshot
        .title_bar
        .filter(|id| context.world().contains(*id))
    {
        return Ok(Some(id));
    }
    if let Some(id) = find_title_bar_child(context, parent) {
        return Ok(Some(id));
    }
    if snapshot.title_bar.is_none() {
        return Ok(None);
    }
    let title = recovered_title(context, parent).unwrap_or_else(|| Arc::from(""));
    Ok(Some(
        context
            .create_detached_component(document, AppTitleBar::new(title))?
            .stable_id(),
    ))
}

fn resolve_app_overlay(
    context: &mut AppContext,
    document: DocumentId,
    parent: StableNodeId,
    snapshot: &AppShell,
) -> Result<Option<StableNodeId>, FrameworkError> {
    if let Some(id) = snapshot.overlay.filter(|id| context.world().contains(*id)) {
        return Ok(Some(id));
    }
    if let Some(id) = find_overlay_child(context, parent) {
        return Ok(Some(id));
    }
    if snapshot.overlay.is_none() {
        return Ok(None);
    }
    Ok(Some(
        context
            .create_detached_component(document, OverlayHost::new())?
            .stable_id(),
    ))
}

fn find_title_bar_child(context: &AppContext, parent: StableNodeId) -> Option<StableNodeId> {
    context
        .world()
        .node(parent)
        .into_iter()
        .flat_map(|node| node.children)
        .find(|&id| {
            view_is::<AppTitleBar>(context, id)
                || matches!(
                    context.world().node(id).map(|node| node.kind),
                    Some(NodeKind::Element { tag })
                        if tag.contains("title-bar") || tag.contains("titlebar")
                )
        })
}

fn find_overlay_child(context: &AppContext, parent: StableNodeId) -> Option<StableNodeId> {
    context
        .world()
        .node(parent)
        .into_iter()
        .flat_map(|node| node.children)
        .find(|&id| {
            view_is::<OverlayHost>(context, id)
                || matches!(
                    context.world().node(id).map(|node| node.kind),
                    Some(NodeKind::Element { tag }) if tag.contains("overlay")
                )
        })
}

fn recovered_title(context: &AppContext, parent: StableNodeId) -> Option<Arc<str>> {
    let id = find_title_bar_child(context, parent)?;
    if let Ok(title) = context.read(Entity::<AppTitleBar>::from_stable_id(id), |bar| {
        Arc::clone(&bar.title)
    }) {
        if !title.is_empty() {
            return Some(title);
        }
    }
    if let Some(text) = context.world().text(id).filter(|text| !text.is_empty()) {
        return Some(Arc::from(text));
    }
    context
        .world()
        .accessibility(id)
        .and_then(|state| state.label.clone())
        .filter(|label| !label.is_empty())
}

fn ensure_workspace(
    context: &mut AppContext,
    document: DocumentId,
    existing: Option<StableNodeId>,
) -> Result<StableNodeId, FrameworkError> {
    if let Some(id) = existing.filter(|id| view_is::<Workspace>(context, *id)) {
        return Ok(id);
    }
    Ok(context
        .create_detached_component(document, Workspace::new())?
        .stable_id())
}

fn ensure_overlay_host(
    context: &mut AppContext,
    document: DocumentId,
    existing: Option<StableNodeId>,
) -> Result<StableNodeId, FrameworkError> {
    if let Some(id) = existing.filter(|id| context.world().contains(*id)) {
        return Ok(id);
    }
    Ok(context
        .create_detached_component(document, OverlayHost::new())?
        .stable_id())
}

fn ensure_title_bar(
    context: &mut AppContext,
    document: DocumentId,
    existing: Option<StableNodeId>,
    title: Option<&Arc<str>>,
    leading: Option<StableNodeId>,
    center: Option<StableNodeId>,
    trailing: Option<StableNodeId>,
) -> Result<Option<StableNodeId>, FrameworkError> {
    if let Some(id) = existing.filter(|id| context.world().contains(*id)) {
        return Ok(Some(id));
    }
    let Some(title) = title else {
        return Ok(None);
    };
    let leading = leading.filter(|id| context.world().contains(*id));
    let center = center.filter(|id| context.world().contains(*id));
    let trailing = trailing.filter(|id| context.world().contains(*id));
    let mut bar = AppTitleBar::new(Arc::clone(title));
    if let Some(leading) = leading {
        bar = bar.leading(leading);
    }
    if let Some(center) = center {
        bar = bar.center(center);
    }
    if let Some(trailing) = trailing {
        bar = bar.trailing(trailing);
    }
    let title_bar = context
        .create_detached_component(document, bar)?
        .stable_id();
    let mut children = Vec::new();
    if let Some(leading) = leading {
        children.push(leading);
    }
    if let Some(center) = center {
        children.push(center);
    }
    if let Some(trailing) = trailing {
        children.push(trailing);
    }
    reconcile_ids(context, title_bar, &children)?;
    Ok(Some(title_bar))
}

fn resolve_navigation(
    context: &mut AppContext,
    document: DocumentId,
    navigation: Option<StableNodeId>,
    footer: Option<StableNodeId>,
    existing_frame: Option<StableNodeId>,
) -> Result<(Option<StableNodeId>, Option<StableNodeId>), FrameworkError> {
    let Some(navigation) = navigation.filter(|id| context.world().contains(*id)) else {
        return Ok((None, None));
    };
    let footer = footer.filter(|id| context.world().contains(*id));
    if view_is::<SidebarFrame>(context, navigation) {
        if let Some(footer) = footer {
            context.update_component(
                Entity::<SidebarFrame>::from_stable_id(navigation),
                |frame, _| {
                    frame.footer = Some(footer);
                },
            )?;
            let children = context
                .world()
                .node(navigation)
                .map(|node| node.children)
                .unwrap_or_default();
            if !children.contains(&footer) {
                let mut mutations = MutationQueue::new();
                mutations.insert(navigation, footer, None);
                context.commit_mutations(mutations)?;
            }
        }
        return Ok((Some(navigation), None));
    }

    let frame = if let Some(id) = existing_frame.filter(|id| view_is::<SidebarFrame>(context, *id))
    {
        id
    } else {
        context
            .create_detached_component(document, SidebarFrame::new())?
            .stable_id()
    };
    let scroll = match context.read(Entity::<SidebarFrame>::from_stable_id(frame), |frame| {
        frame.body
    })? {
        Some(id) if is_scroll_node(context, id) => id,
        _ => context
            .create_detached_component(document, SidebarFrame::vertical_body_scroll())?
            .stable_id(),
    };
    context.update_component(Entity::<SidebarFrame>::from_stable_id(frame), |frame, _| {
        frame.body = Some(scroll);
        frame.footer = footer;
    })?;
    reconcile_ids(context, scroll, &[navigation])?;
    let mut frame_children = vec![scroll];
    if let Some(footer) = footer {
        frame_children.push(footer);
    }
    reconcile_ids(context, frame, &frame_children)?;
    Ok((Some(frame), Some(frame)))
}

fn is_scroll_node(context: &AppContext, id: StableNodeId) -> bool {
    matches!(
        context.world().node(id).map(|node| node.kind),
        Some(NodeKind::Element { tag }) if tag == "scroll"
    )
}

fn region_slots(
    context: &AppContext,
    shell: &DesktopShell,
    resources: Option<StableNodeId>,
) -> Vec<WorkspaceRegionSlot> {
    let present = |id: Option<StableNodeId>| id.filter(|id| context.world().contains(*id));
    let mut slots = Vec::new();
    if let Some(content) = present(resources) {
        slots.push(WorkspaceRegionSlot::new(RegionId::Resources, content));
    }
    if let Some(content) = present(shell.primary) {
        slots.push(WorkspaceRegionSlot::new(RegionId::Primary, content));
    }
    if let Some(content) = present(shell.inspector) {
        slots.push(WorkspaceRegionSlot::new(RegionId::Inspector, content));
    }
    if let Some(content) = present(shell.bottom) {
        slots.push(WorkspaceRegionSlot::new(RegionId::Diagnostics, content));
    }
    for (id, content) in &shell.extra_regions {
        let Some(content) = present(Some(*content)) else {
            continue;
        };
        if let Some(existing) = slots.iter_mut().find(|slot| slot.id == *id) {
            existing.content = Some(content);
        } else {
            slots.push(WorkspaceRegionSlot::new(id.clone(), content));
        }
    }
    slots
}

fn used_ids(shell: &DesktopShell, slots: &[WorkspaceRegionSlot]) -> HashSet<StableNodeId> {
    let mut ids = HashSet::new();
    for id in [
        shell.title_bar,
        shell.workspace,
        shell.overlay,
        shell.primary,
        shell.navigation,
        shell.navigation_footer,
        shell.inspector,
        shell.bottom,
        shell.navigation_frame,
        shell.title_leading,
        shell.title_center,
        shell.title_trailing,
    ]
    .into_iter()
    .flatten()
    {
        ids.insert(id);
    }
    for (_, content) in &shell.extra_regions {
        ids.insert(*content);
    }
    ids.extend(shell.overlays.iter().copied());
    for slot in slots {
        if let Some(content) = slot.content {
            ids.insert(content);
        }
    }
    ids
}

fn park_unused(
    context: &mut AppContext,
    shell: StableNodeId,
    previous: &HashSet<StableNodeId>,
    next: &HashSet<StableNodeId>,
) -> Result<(), FrameworkError> {
    let mut mutations = MutationQueue::new();
    for id in previous {
        if *id == shell || next.contains(id) || !context.world().contains(*id) {
            continue;
        }
        mutations.park_subtree(*id);
    }
    if mutations.as_slice().is_empty() {
        return Ok(());
    }
    context.commit_mutations(mutations)?;
    Ok(())
}

fn reconcile_ids(
    context: &mut AppContext,
    parent: StableNodeId,
    ordered: &[StableNodeId],
) -> Result<bool, FrameworkError> {
    let ordered = ordered
        .iter()
        .copied()
        .filter(|id| *id != parent && context.world().contains(*id))
        .collect::<Vec<_>>();
    let current = context
        .world()
        .node(parent)
        .ok_or(FrameworkError::MissingView(parent))?
        .children
        .clone();
    if current.as_slice() == ordered.as_slice() {
        return Ok(false);
    }
    let keep = ordered.iter().copied().collect::<HashSet<_>>();
    let mut mutations = MutationQueue::new();
    for child in &current {
        if !keep.contains(child) {
            mutations.park_subtree(*child);
        }
    }
    for child in ordered {
        mutations.insert(parent, child, None);
    }
    context.commit_mutations(mutations)?;
    Ok(true)
}

fn project_window_control(
    id: StableNodeId,
    action: WindowChromeAction,
    maximized: bool,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    if world.node(id).is_none() {
        return;
    }
    if world.text(id) != Some("") {
        mutations.set_text(
            id,
            TextContent {
                value: String::new(),
            },
        );
    }
    let visual = StandardVisual::Icon {
        icon: action.icon(maximized),
        size: ControlSize::Small.icon_size(),
        tooltip: None,
    };
    if world.standard_visual(id) != Some(visual.clone()) {
        mutations.set_standard_visual(id, Some(visual));
    }
    project_common(
        id,
        world,
        mutations,
        &window_control_style(action == WindowChromeAction::Close),
        InteractionState {
            pointer_events: true,
            focusable: true,
        },
        AccessibilityState {
            role: AccessibilityRole::Button,
            label: Some(Arc::from(action.label(maximized))),
            ..AccessibilityState::default()
        },
    );
}

fn window_control_style(danger: bool) -> NodeStyle {
    let extent = ControlSize::Small.height();
    let mut style = NodeStyle::default();
    style.foreground = Some(SemanticColorRole::Muted);
    style.background = None;
    style.text_horizontal_alignment = crate::TextHorizontalAlignment::Center;
    style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
    style.interaction = InteractionStyle {
        hovered: SemanticPaint {
            foreground: Some(if danger {
                SemanticColorRole::Danger
            } else {
                SemanticColorRole::Text
            }),
            background: Some(if danger {
                SemanticColorRole::DangerSoftHover
            } else {
                SemanticColorRole::Hover
            }),
            ..SemanticPaint::default()
        },
        pressed: SemanticPaint {
            foreground: Some(if danger {
                SemanticColorRole::Danger
            } else {
                SemanticColorRole::Text
            }),
            background: Some(if danger {
                SemanticColorRole::DangerSoftPressed
            } else {
                SemanticColorRole::Active
            }),
            ..SemanticPaint::default()
        },
        focused: SemanticPaint {
            border: Some(SemanticColorRole::Accent),
            ..SemanticPaint::default()
        },
        ..InteractionStyle::default()
    };
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Px(extent));
    layout.height = Some(LengthSpec::Px(extent));
    layout.min_width = Some(LengthSpec::Px(extent));
    layout.min_height = Some(LengthSpec::Px(extent));
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    layout.padding_left = Some(LengthSpec::Px(0.0));
    layout.padding_right = Some(LengthSpec::Px(0.0));
    layout.padding_top = Some(LengthSpec::Px(0.0));
    layout.padding_bottom = Some(LengthSpec::Px(0.0));
    layout.border_radius = Some(UI_METRICS.radius_sm);
    style
}

fn apply_fill_slot(layout: &mut nana_ui_core::LayoutStyle, align: AlignSpec, justify: JustifySpec) {
    layout.direction = Some(FlexDirection::Row);
    layout.align_items = align;
    layout.justify_content = justify;
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
}

fn patch_layout(
    world: &UiWorld,
    mutations: &mut MutationQueue,
    id: StableNodeId,
    patch: impl FnOnce(&mut nana_ui_core::LayoutStyle),
) {
    if world.node(id).is_none() {
        return;
    }
    let mut style = world.node_style(id).cloned().unwrap_or_default();
    patch(Arc::make_mut(&mut style.layout));
    if world.node_style(id) != Some(&style) {
        mutations.set_style(id, style);
    }
}

fn valid_inset(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn finite_positive(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, Entity, IconButton, LayoutViewport, SidebarFrame, Text};
    use nana_ui_core::RegionId;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn title_bar_is_36px_and_projects_title_without_center() {
        let mut context = AppContext::new();
        let bar = context
            .create_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let id = bar.stable_id();
        assert_eq!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element {
                tag: "app-title-bar".into(),
            }
        );
        assert_eq!(context.world().text(id), Some("Nana"));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.layout.height, Some(LengthSpec::Px(TITLE_BAR_HEIGHT)));
        assert_eq!(style.layout.font_size, Some(TITLE_FONT_SIZE));
        assert_eq!(style.layout.font_weight, Some(TITLE_FONT_WEIGHT));
        assert_eq!(style.foreground, Some(SemanticColorRole::Text));
        assert_eq!(style.background, Some(SemanticColorRole::Surface));
        assert_eq!(style.layout.direction, Some(FlexDirection::Row));

        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let bounds = context.world().layout_box(id).unwrap();
        assert_eq!(bounds.height, TITLE_BAR_HEIGHT);
        assert_eq!(bounds.width, 800.0);
    }

    #[test]
    fn center_slot_suppresses_default_title_text() {
        let mut context = AppContext::new();
        let center = context
            .create_component(document(), Text::new("Workspace / File"))
            .unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana").center(center.stable_id()),
            )
            .unwrap();
        assert_eq!(context.world().text(bar.stable_id()), Some(""));
        assert_eq!(
            context.world().text(center.stable_id()),
            Some("Workspace / File")
        );
        let center_layout = &context
            .world()
            .node_style(center.stable_id())
            .unwrap()
            .layout;
        assert_eq!(
            center_layout.width,
            Some(LengthSpec::Px(DEFAULT_CENTER_WIDTH))
        );
        assert_eq!(center_layout.overflow_x, OverflowSpec::Hidden);
        assert!(center_layout.text_overflow_ellipsis);

        context
            .update_component(bar, |bar, _| {
                bar.center = None;
            })
            .unwrap();
        assert_eq!(context.world().text(bar.stable_id()), Some("Nana"));
    }

    #[test]
    fn controls_helper_projects_three_icons_and_restore() {
        let mut context = AppContext::new();
        let minimize = context
            .create_component(
                document(),
                IconButton::new(Icon::Search, "x").size(ControlSize::Small),
            )
            .unwrap();
        let maximize = context
            .create_component(
                document(),
                IconButton::new(Icon::Search, "x").size(ControlSize::Small),
            )
            .unwrap();
        let close = context
            .create_component(
                document(),
                IconButton::new(Icon::Search, "x").size(ControlSize::Small),
            )
            .unwrap();
        let controls = context
            .create_component(
                document(),
                AppTitleBarControls::new(false)
                    .minimize(minimize.stable_id())
                    .maximize(maximize.stable_id())
                    .close(close.stable_id()),
            )
            .unwrap();

        assert_eq!(
            context.world().node(controls.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "app-title-bar-controls".into(),
            }
        );
        assert!(matches!(
            context.world().standard_visual(minimize.stable_id()),
            Some(StandardVisual::Icon {
                icon: Icon::Minimize,
                ..
            })
        ));
        assert!(matches!(
            context.world().standard_visual(maximize.stable_id()),
            Some(StandardVisual::Icon {
                icon: Icon::Maximize,
                ..
            })
        ));
        assert!(matches!(
            context.world().standard_visual(close.stable_id()),
            Some(StandardVisual::Icon {
                icon: Icon::Close,
                ..
            })
        ));
        assert_eq!(
            context
                .world()
                .node_style(minimize.stable_id())
                .unwrap()
                .layout
                .width,
            Some(LengthSpec::Px(ControlSize::Small.height()))
        );
        assert_eq!(
            context
                .world()
                .accessibility(maximize.stable_id())
                .unwrap()
                .label
                .as_deref(),
            Some("Maximize")
        );

        context
            .update_component(controls, |controls, _| {
                controls.maximized = true;
            })
            .unwrap();
        assert!(matches!(
            context.world().standard_visual(maximize.stable_id()),
            Some(StandardVisual::Icon {
                icon: Icon::Restore,
                ..
            })
        ));
        assert_eq!(
            context
                .world()
                .accessibility(maximize.stable_id())
                .unwrap()
                .label
                .as_deref(),
            Some("Restore")
        );
        assert!(matches!(
            context.world().standard_visual(minimize.stable_id()),
            Some(StandardVisual::Icon {
                icon: Icon::Minimize,
                ..
            })
        ));
    }

    #[test]
    fn window_controls_are_omitted_when_disabled() {
        let mut context = AppContext::new();
        let minimize = context
            .create_component(
                document(),
                IconButton::new(Icon::Minimize, "Minimize").size(ControlSize::Small),
            )
            .unwrap();
        let maximize = context
            .create_component(
                document(),
                IconButton::new(Icon::Maximize, "Maximize").size(ControlSize::Small),
            )
            .unwrap();
        let close = context
            .create_component(
                document(),
                IconButton::new(Icon::Close, "Close").size(ControlSize::Small),
            )
            .unwrap();
        let controls = context
            .create_component(
                document(),
                AppTitleBarControls::new(false)
                    .minimize(minimize.stable_id())
                    .maximize(maximize.stable_id())
                    .close(close.stable_id()),
            )
            .unwrap();
        context.append_child(controls, minimize).unwrap();
        context.append_child(controls, maximize).unwrap();
        context.append_child(controls, close).unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana")
                    .controls(controls.stable_id())
                    .show_window_controls(false),
            )
            .unwrap();
        assert!(
            context
                .world()
                .node_style(controls.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert_eq!(context.world().standard_visual(bar.stable_id()), None);
        assert!(
            context
                .world()
                .node(bar.stable_id())
                .unwrap()
                .children
                .is_empty()
        );

        context
            .update_component(bar, |bar, _| {
                bar.show_window_controls = true;
                bar.maximized = true;
            })
            .unwrap();
        assert!(
            !context
                .world()
                .node_style(controls.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert!(matches!(
            context.world().standard_visual(maximize.stable_id()),
            Some(StandardVisual::Icon {
                icon: Icon::Restore,
                ..
            })
        ));
    }

    #[test]
    fn app_shell_stacks_title_then_fill_body_with_overlay_out_of_flow() {
        let mut context = AppContext::new();
        let title = context
            .create_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let body = context
            .create_component(document(), Text::new("workspace"))
            .unwrap();
        let overlay = context
            .create_component(document(), Text::new("overlay"))
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(title.stable_id())
                    .body(body.stable_id())
                    .overlay(overlay.stable_id()),
            )
            .unwrap();
        context.append_child(shell, title).unwrap();
        context.append_child(shell, body).unwrap();
        context.append_child(shell, overlay).unwrap();

        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "app-shell".into(),
            }
        );
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children,
            vec![title.stable_id(), body.stable_id(), overlay.stable_id()]
        );
        let title_layout = &context
            .world()
            .node_style(title.stable_id())
            .unwrap()
            .layout;
        assert_eq!(title_layout.height, Some(LengthSpec::Px(TITLE_BAR_HEIGHT)));
        assert_eq!(title_layout.flex_grow, Some(0.0));
        let shell_layout = &context
            .world()
            .node_style(shell.stable_id())
            .unwrap()
            .layout;
        assert!(
            shell_layout.padding.is_none()
                && shell_layout.padding_left.is_none()
                && shell_layout.padding_right.is_none()
                && shell_layout.padding_top.is_none()
                && shell_layout.padding_bottom.is_none()
        );
        let body_layout = &context.world().node_style(body.stable_id()).unwrap().layout;
        assert_eq!(body_layout.flex_grow, Some(1.0));
        assert_eq!(body_layout.height, Some(LengthSpec::Fill));
        assert_eq!(body_layout.min_height, Some(LengthSpec::Px(0.0)));
        assert_eq!(body_layout.position, PositionSpec::Static);
        let overlay_layout = &context
            .world()
            .node_style(overlay.stable_id())
            .unwrap()
            .layout;
        assert_eq!(overlay_layout.position, PositionSpec::Absolute);
        assert_eq!(overlay_layout.width, Some(LengthSpec::Fill));
        assert_eq!(overlay_layout.height, Some(LengthSpec::Fill));
        assert_eq!(overlay_layout.flex_grow, Some(0.0));

        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let title_box = context.world().layout_box(title.stable_id()).unwrap();
        let body_box = context.world().layout_box(body.stable_id()).unwrap();
        let overlay_box = context.world().layout_box(overlay.stable_id()).unwrap();
        let shell_box = context.world().layout_box(shell.stable_id()).unwrap();
        assert_eq!(title_box.height, TITLE_BAR_HEIGHT);
        assert_eq!(title_box.y, shell_box.y);
        assert_eq!(body_box.y, title_box.y + title_box.height);
        assert_eq!(body_box.height, 400.0 - TITLE_BAR_HEIGHT);
        assert_eq!(overlay_box.x, shell_box.x);
        assert_eq!(overlay_box.y, shell_box.y);
        assert_eq!(overlay_box.width, shell_box.width);
        assert_eq!(overlay_box.height, shell_box.height);
        assert_eq!(shell_box.height, 400.0);
    }

    #[test]
    fn idle_project_does_not_dirty() {
        let mut context = AppContext::new();
        let center = context
            .create_component(document(), Text::new("crumbs"))
            .unwrap();
        let body = context
            .create_component(document(), Text::new("workspace"))
            .unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana").center(center.stable_id()),
            )
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(bar.stable_id())
                    .body(body.stable_id()),
            )
            .unwrap();
        let _ = context.take_system_work();
        context.update_component(bar, |_, _| {}).unwrap();
        context.update_component(shell, |_, _| {}).unwrap();
        assert!(context.take_system_work().is_empty());
    }

    #[test]
    fn assemble_app_shell_reconciles_title_bar_and_body() {
        let mut context = AppContext::new();
        let title = context
            .create_detached_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let body = context
            .create_detached_component(document(), Text::new("workspace"))
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(title.stable_id())
                    .body(body.stable_id()),
            )
            .unwrap();

        assert!(context.assemble_app_shell(shell).unwrap());
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children,
            vec![title.stable_id(), body.stable_id()]
        );
        let title_layout = &context
            .world()
            .node_style(title.stable_id())
            .unwrap()
            .layout;
        assert_eq!(title_layout.height, Some(LengthSpec::Px(TITLE_BAR_HEIGHT)));
        assert_eq!(
            title_layout.min_height,
            Some(LengthSpec::Px(TITLE_BAR_HEIGHT))
        );
        assert_eq!(title_layout.flex_grow, Some(0.0));
        let body_layout = &context.world().node_style(body.stable_id()).unwrap().layout;
        assert_eq!(body_layout.flex_grow, Some(1.0));
        assert_eq!(body_layout.height, Some(LengthSpec::Fill));
        assert_eq!(body_layout.min_height, Some(LengthSpec::Px(0.0)));
        assert!(
            context
                .read(shell, |shell| shell.overlay)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn assemble_app_shell_is_idempotent() {
        let mut context = AppContext::new();
        let title = context
            .create_detached_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let body = context
            .create_detached_component(document(), Text::new("workspace"))
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(title.stable_id())
                    .body(body.stable_id()),
            )
            .unwrap();
        context.assemble_app_shell(shell).unwrap();
        let children = context.world().node(shell.stable_id()).unwrap().children;

        assert!(!context.assemble_app_shell(shell).unwrap());
        assert_eq!(
            context
                .read(shell, |shell| (shell.title_bar, shell.body, shell.overlay))
                .unwrap(),
            (Some(title.stable_id()), Some(body.stable_id()), None)
        );
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children,
            children
        );
    }

    #[test]
    fn assemble_app_shell_keeps_overlay_absolute_fill() {
        let mut context = AppContext::new();
        let title = context
            .create_detached_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let body = context
            .create_detached_component(document(), Text::new("workspace"))
            .unwrap();
        let overlay = context
            .create_detached_component(document(), Text::new("overlay"))
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(title.stable_id())
                    .body(body.stable_id())
                    .overlay(overlay.stable_id()),
            )
            .unwrap();

        context.assemble_app_shell(shell).unwrap();
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children,
            vec![title.stable_id(), body.stable_id(), overlay.stable_id()]
        );
        let overlay_layout = &context
            .world()
            .node_style(overlay.stable_id())
            .unwrap()
            .layout;
        assert_eq!(overlay_layout.position, PositionSpec::Absolute);
        assert_eq!(overlay_layout.offset_top, Some(LengthSpec::Px(0.0)));
        assert_eq!(overlay_layout.offset_right, Some(LengthSpec::Px(0.0)));
        assert_eq!(overlay_layout.offset_bottom, Some(LengthSpec::Px(0.0)));
        assert_eq!(overlay_layout.offset_left, Some(LengthSpec::Px(0.0)));
        assert_eq!(overlay_layout.width, Some(LengthSpec::Fill));
        assert_eq!(overlay_layout.height, Some(LengthSpec::Fill));
        assert_eq!(overlay_layout.flex_grow, Some(0.0));
        assert_eq!(overlay_layout.z_index, Some(OVERLAY_Z_INDEX));
        assert_eq!(
            context.read(shell, |shell| shell.overlay).unwrap(),
            Some(overlay.stable_id())
        );
    }

    fn mounted_roots(context: &AppContext, document: DocumentId) -> Vec<StableNodeId> {
        context
            .world()
            .document_order(document)
            .into_iter()
            .filter(|id| {
                context.world().is_mounted(*id)
                    && context
                        .world()
                        .node(*id)
                        .is_some_and(|node| node.parent.is_none())
            })
            .collect()
    }

    fn assemble_shell(
        context: &mut AppContext,
        shell: DesktopShell,
    ) -> crate::Entity<DesktopShell> {
        let entity = context
            .create_component(document(), shell)
            .expect("desktop shell");
        context
            .assemble_desktop_shell(entity)
            .expect("assemble desktop shell");
        entity
    }

    #[test]
    fn desktop_shell_stacks_title_workspace_and_overlay() {
        let mut context = AppContext::new();
        let title = context
            .create_detached_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let primary = context
            .create_detached_component(document(), Text::new("workspace"))
            .unwrap();
        let overlay = context
            .create_detached_component(document(), Text::new("overlay"))
            .unwrap();
        let shell = assemble_shell(
            &mut context,
            DesktopShell::new()
                .title_bar(title.stable_id())
                .primary(primary.stable_id())
                .overlay(overlay.stable_id()),
        );
        let (workspace, overlay_host) = context
            .read(shell, |shell| (shell.workspace, shell.overlay))
            .unwrap();
        let workspace = workspace.expect("workspace");
        let overlay_host = overlay_host.expect("overlay host");

        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "desktop-shell".into(),
            }
        );
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children,
            vec![title.stable_id(), workspace, overlay_host]
        );
        let title_layout = &context
            .world()
            .node_style(title.stable_id())
            .unwrap()
            .layout;
        assert_eq!(title_layout.height, Some(LengthSpec::Px(TITLE_BAR_HEIGHT)));
        assert_eq!(title_layout.flex_grow, Some(0.0));
        let shell_layout = &context
            .world()
            .node_style(shell.stable_id())
            .unwrap()
            .layout;
        assert!(
            shell_layout.padding.is_none()
                && shell_layout.padding_left.is_none()
                && shell_layout.padding_right.is_none()
                && shell_layout.padding_top.is_none()
                && shell_layout.padding_bottom.is_none()
        );
        let body_layout = &context.world().node_style(workspace).unwrap().layout;
        assert_eq!(body_layout.flex_grow, Some(1.0));
        assert_eq!(body_layout.height, Some(LengthSpec::Fill));
        assert_eq!(body_layout.min_height, Some(LengthSpec::Px(0.0)));
        assert_eq!(body_layout.position, PositionSpec::Static);
        let overlay_layout = &context.world().node_style(overlay_host).unwrap().layout;
        assert_eq!(overlay_layout.position, PositionSpec::Absolute);
        assert_eq!(overlay_layout.width, Some(LengthSpec::Fill));
        assert_eq!(overlay_layout.height, Some(LengthSpec::Fill));
        assert_eq!(overlay_layout.flex_grow, Some(0.0));
        assert_eq!(
            context.world().node(overlay_host).unwrap().children,
            vec![overlay.stable_id()]
        );

        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let title_box = context.world().layout_box(title.stable_id()).unwrap();
        let body_box = context.world().layout_box(workspace).unwrap();
        let overlay_box = context.world().layout_box(overlay_host).unwrap();
        let shell_box = context.world().layout_box(shell.stable_id()).unwrap();
        assert_eq!(title_box.height, TITLE_BAR_HEIGHT);
        assert_eq!(title_box.y, shell_box.y);
        assert_eq!(body_box.y, title_box.y + title_box.height);
        assert_eq!(body_box.height, 400.0 - TITLE_BAR_HEIGHT);
        assert_eq!(overlay_box.x, shell_box.x);
        assert_eq!(overlay_box.y, shell_box.y);
        assert_eq!(overlay_box.width, shell_box.width);
        assert_eq!(overlay_box.height, shell_box.height);
        assert_eq!(shell_box.height, 400.0);
    }

    #[test]
    fn desktop_shell_maps_navigation_and_primary_to_workspace_slots() {
        let mut context = AppContext::new();
        let navigation = context
            .create_detached_component(document(), SidebarFrame::new())
            .unwrap();
        let primary = context
            .create_detached_component(document(), Text::new("primary"))
            .unwrap();
        let shell = assemble_shell(
            &mut context,
            DesktopShell::new()
                .navigation(navigation.stable_id())
                .primary(primary.stable_id()),
        );
        let workspace_id = context
            .read(shell, |shell| shell.workspace)
            .unwrap()
            .expect("workspace");
        let workspace = Entity::<Workspace>::from_stable_id(workspace_id);
        let (slots, middle, primary_column) = context
            .read(workspace, |workspace| {
                (
                    workspace.slots.clone(),
                    workspace.middle,
                    workspace.primary_column,
                )
            })
            .unwrap();
        assert_eq!(
            slots,
            vec![
                WorkspaceRegionSlot::new(RegionId::Resources, navigation.stable_id()),
                WorkspaceRegionSlot::new(RegionId::Primary, primary.stable_id()),
            ]
        );
        let middle = middle.expect("assembled start track");
        let primary_column = primary_column.expect("assembled primary column");
        assert_eq!(
            context.world().node(middle).unwrap().children,
            vec![navigation.stable_id(), primary_column]
        );
        assert!(
            context
                .world()
                .node(workspace_id)
                .unwrap()
                .children
                .contains(&middle)
        );
    }

    #[test]
    fn desktop_shell_omits_missing_inspector_and_bottom() {
        let mut context = AppContext::new();
        let navigation = context
            .create_detached_component(document(), SidebarFrame::new())
            .unwrap();
        let primary = context
            .create_detached_component(document(), Text::new("primary"))
            .unwrap();
        let shell = assemble_shell(
            &mut context,
            DesktopShell::new()
                .navigation(navigation.stable_id())
                .primary(primary.stable_id()),
        );
        let workspace_id = context
            .read(shell, |shell| shell.workspace)
            .unwrap()
            .expect("workspace");
        let slots = context
            .read(
                Entity::<Workspace>::from_stable_id(workspace_id),
                |workspace| workspace.slots.clone(),
            )
            .unwrap();
        assert!(
            !slots
                .iter()
                .any(|slot| slot.id == RegionId::Inspector || slot.id == RegionId::Diagnostics)
        );
        assert_eq!(slots.len(), 2);
        assert!(context.world().node(navigation.stable_id()).is_some());
        assert!(context.world().node(primary.stable_id()).is_some());
    }

    #[test]
    fn desktop_shell_reassemble_after_primary_change_keeps_single_root() {
        let mut context = AppContext::new();
        let title = context
            .create_detached_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let first = context
            .create_detached_component(document(), Text::new("first"))
            .unwrap();
        let second = context
            .create_detached_component(document(), Text::new("second"))
            .unwrap();
        let shell = assemble_shell(
            &mut context,
            DesktopShell::new()
                .title_bar(title.stable_id())
                .primary(first.stable_id()),
        );
        let first_workspace = context
            .read(shell, |shell| shell.workspace)
            .unwrap()
            .expect("workspace");
        assert_eq!(mounted_roots(&context, document()), vec![shell.stable_id()]);

        context
            .update_component(shell, |shell, _| {
                shell.primary = Some(second.stable_id());
            })
            .unwrap();
        context.assemble_desktop_shell(shell).unwrap();
        let workspace_id = context
            .read(shell, |shell| shell.workspace)
            .unwrap()
            .expect("workspace");
        let slots = context
            .read(
                Entity::<Workspace>::from_stable_id(workspace_id),
                |workspace| workspace.slots.clone(),
            )
            .unwrap();

        assert_eq!(workspace_id, first_workspace);
        assert_eq!(mounted_roots(&context, document()), vec![shell.stable_id()]);
        assert_eq!(
            slots,
            vec![WorkspaceRegionSlot::new(
                RegionId::Primary,
                second.stable_id()
            )]
        );
        assert!(context.world().is_mounted(second.stable_id()));
        assert!(!context.world().is_mounted(first.stable_id()));
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children[0],
            title.stable_id()
        );
    }
}
