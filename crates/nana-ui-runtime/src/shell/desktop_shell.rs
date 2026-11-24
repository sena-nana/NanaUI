use super::app_shell::AppShell;
use super::title_bar::AppTitleBar;
use super::{
    STATUS_OVERLAY_Z_INDEX, document_of, is_scroll_node, patch_layout, reconcile_ids, view_is,
};
use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::{LengthSpec, PositionSpec, RegionId, WorkspaceModel};

use crate::{
    AppContext, ComponentView, DocumentId, Entity, FrameworkError, MutationQueue, NodeKind,
    NodeStyle, OverlayHost, SidebarFrame, StableNodeId, UiWorld, Workspace, WorkspaceRegionSlot,
};

/// Page-level desktop composer.
///
/// Wires host-mounted [`AppTitleBar`], [`Workspace`], [`AppShell`] layout, and
/// [`OverlayHost`]. Application content stays in the supplied region slots.
#[derive(Debug, Clone, PartialEq)]
pub struct DesktopShell {
    pub title_bar: Option<StableNodeId>,
    pub workspace: Option<StableNodeId>,
    pub overlay: Option<StableNodeId>,
    pub status: Option<StableNodeId>,
    pub primary: Option<StableNodeId>,
    pub navigation: Option<StableNodeId>,
    pub navigation_footer: Option<StableNodeId>,
    pub inspector: Option<StableNodeId>,
    pub bottom: Option<StableNodeId>,
    pub extra_regions: Vec<(RegionId, StableNodeId)>,
    pub overlays: Vec<StableNodeId>,
    pub status_overlays: Vec<StableNodeId>,
    pub navigation_frame: Option<StableNodeId>,
    pub title: Option<Arc<str>>,
    pub title_leading: Option<StableNodeId>,
    pub title_center: Option<StableNodeId>,
    pub title_trailing: Option<StableNodeId>,
    pub title_center_width: Option<f32>,
    pub title_window_controls: Option<bool>,
    /// Whether the workspace rounds the corners of its main region
    /// ([`crate::Workspace::workspace_corners`]); the shell hands it to the
    /// workspace it assembles. On by default.
    pub workspace_corners: bool,
    pub model: WorkspaceModel,
    pub style: NodeStyle,
}

impl DesktopShell {
    pub fn new() -> Self {
        Self {
            title_bar: None,
            workspace: None,
            overlay: None,
            status: None,
            primary: None,
            navigation: None,
            navigation_footer: None,
            inspector: None,
            bottom: None,
            extra_regions: Vec::new(),
            overlays: Vec::new(),
            status_overlays: Vec::new(),
            navigation_frame: None,
            title: None,
            title_leading: None,
            title_center: None,
            title_trailing: None,
            title_center_width: None,
            title_window_controls: None,
            workspace_corners: true,
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

    /// Width of the centred title column when the shell creates the title bar.
    ///
    /// The default suits a short product title; a breadcrumb needs more room or
    /// it wraps and clips inside the fixed column.
    pub fn title_center_width(mut self, width: f32) -> Self {
        self.title_center_width = Some(width);
        self
    }

    /// Whether the shell-created title bar mounts its own window controls.
    ///
    /// The mounted controls are presentation only. Hosts that bind their own
    /// minimize / maximize / close buttons pass `false` to avoid a second,
    /// inert set of buttons.
    pub fn title_window_controls(mut self, show: bool) -> Self {
        self.title_window_controls = Some(show);
        self
    }

    /// Round the corners of the workspace's main region, or square them
    /// (the Appearance "workspace corners" setting).
    pub fn workspace_corners(mut self, rounded: bool) -> Self {
        self.workspace_corners = rounded;
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

    /// Right column below a Primary-scoped toolbar. Inspector tabs belong here,
    /// not on [`RegionId::PrimaryToolbar`].
    pub fn inspector(mut self, inspector: StableNodeId) -> Self {
        self.inspector = Some(inspector);
        self
    }

    pub fn bottom(mut self, bottom: StableNodeId) -> Self {
        self.bottom = Some(bottom);
        self
    }

    /// Extra region content. For [`RegionId::PrimaryToolbar`], pass only
    /// document / edit / kind actions — inspector tabs belong in [`Self::inspector`].
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

    /// Push a modal overlay child onto the assembled dialog [`OverlayHost`].
    pub fn overlay(mut self, overlay: StableNodeId) -> Self {
        self.overlays.push(overlay);
        self
    }

    /// Push a status overlay (toast). It uses a second host so a confirm
    /// dialog can stay open.
    pub fn status(mut self, overlay: StableNodeId) -> Self {
        self.status_overlays.push(overlay);
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
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::DESKTOP_SHELL.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(AppContext::assemble_desktop_shell),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// Patches the body and status nodes, which other components own and project.
    const ALWAYS_REPROJECT: bool = true;

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
        if let Some(status) = self.status {
            patch_layout(world, mutations, status, |layout| {
                layout.position = PositionSpec::Absolute;
                layout.offset_top = Some(LengthSpec::Px(0.0));
                layout.offset_right = Some(LengthSpec::Px(0.0));
                layout.offset_bottom = Some(LengthSpec::Px(0.0));
                layout.offset_left = Some(LengthSpec::Px(0.0));
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(LengthSpec::Fill);
                layout.flex_grow = Some(0.0);
                layout.flex_shrink = Some(0.0);
                layout.z_index = Some(layout.z_index.unwrap_or(STATUS_OVERLAY_Z_INDEX));
            });
        }
    }
}

impl AppContext {
    /// Replace the primary and inspector region content on an assembled shell.
    ///
    /// Chrome is reused. Previous region content is parked, not destroyed.
    /// Unchanged slots return without assembling.
    pub fn set_desktop_slots(
        &mut self,
        shell: Entity<DesktopShell>,
        primary: Option<StableNodeId>,
        inspector: Option<StableNodeId>,
    ) -> Result<bool, FrameworkError> {
        let changed = self.read(shell, |desktop| {
            desktop.primary != primary || desktop.inspector != inspector
        })?;
        if !changed {
            return Ok(false);
        }
        self.update_component(shell, |desktop, _| {
            desktop.primary = primary;
            desktop.inspector = inspector;
        })?;
        self.assemble_desktop_shell(shell)
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
        let status_id = ensure_overlay_host(
            self,
            document,
            snapshot.status.filter(|id| *id != overlay_id),
        )?;
        let title_bar = ensure_title_bar(
            self,
            document,
            snapshot.title_bar,
            snapshot.title.as_ref(),
            snapshot.title_leading,
            snapshot.title_center,
            snapshot.title_trailing,
            snapshot.title_center_width,
            snapshot.title_window_controls,
        )?;
        if let Some(title_bar) = title_bar {
            let _ = self.assemble_app_title_bar(Entity::<AppTitleBar>::from_stable_id(title_bar));
        }
        let (resources, navigation_frame) = resolve_navigation(
            self,
            document,
            snapshot.navigation,
            snapshot.navigation_footer,
            snapshot.navigation_frame,
        )?;

        let chrome_changed = snapshot.title_bar != title_bar
            || snapshot.workspace != Some(workspace_id)
            || snapshot.overlay != Some(overlay_id)
            || snapshot.status != Some(status_id)
            || snapshot.navigation_frame != navigation_frame;
        let mut assembled = snapshot;
        assembled.title_bar = title_bar;
        assembled.workspace = Some(workspace_id);
        assembled.overlay = Some(overlay_id);
        assembled.status = Some(status_id);
        assembled.navigation_frame = navigation_frame;
        let slots = region_slots(self, &assembled, resources);
        let next_used = used_ids(&assembled, &slots);
        park_unused(self, shell.stable_id(), &previous_used, &next_used)?;
        if chrome_changed {
            self.update_component(shell, |desktop, _| {
                desktop.title_bar = title_bar;
                desktop.workspace = Some(workspace_id);
                desktop.overlay = Some(overlay_id);
                desktop.status = Some(status_id);
                desktop.navigation_frame = navigation_frame;
            })?;
        }
        let mut shell_children = Vec::new();
        if let Some(title_bar) = title_bar {
            shell_children.push(title_bar);
        }
        shell_children.push(workspace_id);
        shell_children.push(overlay_id);
        shell_children.push(status_id);
        let mut changed = reconcile_ids(self, shell.stable_id(), &shell_children)?;

        let workspace = Entity::<Workspace>::from_stable_id(workspace_id);
        let slots_changed = self.read(workspace, |workspace| workspace.slots != slots)?;
        let corners = assembled.workspace_corners;
        self.update_component(workspace, |workspace, _| {
            workspace.refresh_from_model(&assembled.model);
            workspace.slots = slots;
            workspace.workspace_corners = corners;
        })?;
        changed |= self.assemble_workspace(workspace)?;
        changed |= reconcile_ids(self, overlay_id, &assembled.overlays)?;
        changed |= reconcile_ids(self, status_id, &assembled.status_overlays)?;
        if changed || chrome_changed {
            self.reproject_component(shell)?;
        }
        Ok(changed || chrome_changed || slots_changed)
    }
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

#[allow(clippy::too_many_arguments)]
fn ensure_title_bar(
    context: &mut AppContext,
    document: DocumentId,
    existing: Option<StableNodeId>,
    title: Option<&Arc<str>>,
    leading: Option<StableNodeId>,
    center: Option<StableNodeId>,
    trailing: Option<StableNodeId>,
    center_width: Option<f32>,
    window_controls: Option<bool>,
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
    if let Some(width) = center_width {
        bar = bar.center_width(width);
    }
    if let Some(show) = window_controls {
        bar = bar.show_window_controls(show);
    }
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

fn region_slots(
    context: &AppContext,
    shell: &DesktopShell,
    resources: Option<StableNodeId>,
) -> Vec<WorkspaceRegionSlot> {
    let present = |id: Option<StableNodeId>| id.filter(|id| context.world().contains(*id));
    let region_slot = |id, content| {
        if is_scroll_node(context, content) {
            WorkspaceRegionSlot::borrowed(id, content)
        } else {
            WorkspaceRegionSlot::new(id, content)
        }
    };
    let mut slots = Vec::new();
    if let Some(content) = present(resources) {
        slots.push(region_slot(RegionId::Resources, content));
    }
    if let Some(content) = present(shell.primary) {
        slots.push(region_slot(RegionId::Primary, content));
    }
    if let Some(content) = present(shell.inspector) {
        slots.push(region_slot(RegionId::Inspector, content));
    }
    if let Some(content) = present(shell.bottom) {
        slots.push(region_slot(RegionId::Diagnostics, content));
    }
    for (id, content) in &shell.extra_regions {
        let Some(content) = present(Some(*content)) else {
            continue;
        };
        if let Some(existing) = slots.iter_mut().find(|slot| slot.id == *id) {
            *existing = region_slot(id.clone(), content);
        } else {
            slots.push(region_slot(id.clone(), content));
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
        shell.status,
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
    ids.extend(shell.status_overlays.iter().copied());
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

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::title_bar::{CENTER_COLUMN_TAG, NATIVE_WINDOW_CONTROLS_WIDTH, node_tag};
    use crate::{
        AppContext, DocumentId, IconButton, LayoutViewport, MountState, SidebarFrame, StableNodeId,
        Text, TextHorizontalAlignment,
    };
    use nana_ui_core::{
        ControlSize, Icon, RegionId, RegionPlacement, RegionRole, RegionScope, RegionState,
        TITLE_BAR_HEIGHT, WindowChrome, WorkspaceLayout, WorkspaceModel,
    };

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
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
    fn desktop_shell_forwards_center_width_and_suppresses_mounted_controls() {
        let mut context = AppContext::new();
        let center = context
            .create_detached_component(document(), Text::new("Nana › 很长的面包屑上下文标题"))
            .unwrap();
        let shell = assemble_shell(
            &mut context,
            DesktopShell::new()
                .title("Nana")
                .title_center(center.stable_id())
                .title_center_width(420.0)
                .title_window_controls(false),
        );
        let bar = context
            .read(shell, |shell| shell.title_bar)
            .unwrap()
            .expect("shell creates a title bar");
        let snapshot = context
            .read(Entity::<AppTitleBar>::from_stable_id(bar), Clone::clone)
            .unwrap();
        assert_eq!(snapshot.center_width, 420.0);
        assert!(!snapshot.show_window_controls);
        // Only the hidden, empty native placeholder may remain mounted.
        assert!(
            snapshot
                .controls
                .is_none_or(|controls| snapshot.native_controls
                    && context.world().node(controls).unwrap().children.is_empty()
                    && context.world().node_style(controls).unwrap().layout.hidden),
            "suppressed chrome must not mount a second control strip"
        );

        context
            .layout_document(document(), LayoutViewport::new(1180.0, 760.0))
            .unwrap();
        let center_slot = context
            .world()
            .node(bar)
            .unwrap()
            .children
            .into_iter()
            .find(|&id| node_tag(context.world(), id).as_deref() == Some(CENTER_COLUMN_TAG))
            .expect("center column");
        assert_eq!(
            context
                .world()
                .canonical_layout_box(center_slot)
                .unwrap()
                .width,
            420.0
        );
    }

    /// The shell carries the workspace-corners setting and hands it to the
    /// workspace it builds, on the first assembly and on later ones.
    #[test]
    fn desktop_shell_hands_its_corner_setting_to_its_workspace() {
        let mut context = AppContext::new();
        let primary = context
            .create_detached_component(document(), Text::new("workspace"))
            .unwrap();
        let shell = assemble_shell(
            &mut context,
            DesktopShell::new()
                .primary(primary.stable_id())
                .workspace_corners(false),
        );
        let workspace = Entity::<Workspace>::from_stable_id(
            context
                .read(shell, |shell| shell.workspace)
                .unwrap()
                .unwrap(),
        );
        let rounded = |context: &AppContext| {
            context
                .read(workspace, |workspace| workspace.workspace_corners)
                .unwrap()
        };
        assert!(!rounded(&context));
        context
            .update_component(shell, |shell, _| shell.workspace_corners = true)
            .unwrap();
        context.assemble_desktop_shell(shell).unwrap();
        assert!(rounded(&context));
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
        let (workspace, overlay_host, status_host) = context
            .read(shell, |shell| {
                (shell.workspace, shell.overlay, shell.status)
            })
            .unwrap();
        let workspace = workspace.expect("workspace");
        let overlay_host = overlay_host.expect("overlay host");
        let status_host = status_host.expect("status host");

        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "desktop-shell".into(),
            }
        );
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children,
            vec![title.stable_id(), workspace, overlay_host, status_host]
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
        let title_box = context
            .world()
            .canonical_layout_box(title.stable_id())
            .unwrap();
        let body_box = context.world().canonical_layout_box(workspace).unwrap();
        let overlay_box = context.world().canonical_layout_box(overlay_host).unwrap();
        let shell_box = context
            .world()
            .canonical_layout_box(shell.stable_id())
            .unwrap();
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
        let nav_layout = &context
            .world()
            .node_style(navigation.stable_id())
            .unwrap()
            .layout;
        assert_eq!(nav_layout.padding_left, Some(LengthSpec::Px(12.0)));
        assert_eq!(nav_layout.padding_right, Some(LengthSpec::Px(8.0)));
        assert_eq!(nav_layout.padding_top, Some(LengthSpec::Px(10.0)));
        assert_eq!(nav_layout.padding_bottom, Some(LengthSpec::Px(10.0)));
        assert_eq!(nav_layout.gap, Some(LengthSpec::Px(14.0)));
        assert_eq!(nav_layout.width, Some(LengthSpec::Px(260.0)));
    }

    #[test]
    fn desktop_shell_keeps_dragged_sidebar_size_after_assemble() {
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
        let handle = context
            .read(workspace, |workspace| {
                workspace.handles.get(&RegionId::Resources).copied()
            })
            .unwrap()
            .expect("resources handle");
        assert!(
            context
                .begin_workspace_resize(
                    document(),
                    1,
                    handle,
                    260.0,
                    20.0,
                    std::time::Duration::ZERO
                )
                .unwrap()
        );
        assert!(
            context
                .update_workspace_resize(document(), 1, 320.0, 20.0, std::time::Duration::ZERO)
                .unwrap()
        );
        assert!(
            context
                .end_workspace_resize(document(), 1, std::time::Duration::ZERO)
                .unwrap()
        );
        assert_eq!(
            context
                .read(shell, |shell| shell
                    .model
                    .region_extent(&RegionId::Resources))
                .unwrap(),
            320.0
        );
        context.assemble_desktop_shell(shell).unwrap();
        assert_eq!(
            context
                .read(workspace, |workspace| workspace
                    .region_extent(&RegionId::Resources))
                .unwrap(),
            320.0
        );
        assert_eq!(
            context
                .world()
                .node_style(navigation.stable_id())
                .unwrap()
                .layout
                .width,
            Some(LengthSpec::Px(320.0))
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

    #[test]
    fn set_desktop_slots_parks_previous_inspector_and_remounts_it() {
        let mut context = AppContext::new();
        let navigation = context
            .create_detached_component(document(), SidebarFrame::new())
            .unwrap();
        let primary = context
            .create_detached_component(document(), Text::new("primary"))
            .unwrap();
        let first = context
            .create_detached_component(document(), Text::new("inspector-a"))
            .unwrap();
        let second = context
            .create_detached_component(document(), Text::new("inspector-b"))
            .unwrap();
        let shell = assemble_shell(
            &mut context,
            DesktopShell::new()
                .navigation(navigation.stable_id())
                .primary(primary.stable_id())
                .inspector(first.stable_id()),
        );
        assert!(context.world().is_mounted(first.stable_id()));

        assert!(
            context
                .set_desktop_slots(shell, Some(primary.stable_id()), Some(second.stable_id()),)
                .unwrap()
        );
        assert!(context.world().is_mounted(second.stable_id()));
        assert!(!context.world().is_mounted(first.stable_id()));
        assert_eq!(
            context.world().mount_state(first.stable_id()),
            Some(MountState::Parked)
        );
        assert!(
            !context
                .world()
                .document_order(document())
                .contains(&first.stable_id())
        );

        assert!(
            context
                .set_desktop_slots(shell, Some(primary.stable_id()), Some(first.stable_id()),)
                .unwrap()
        );
        assert!(context.world().is_mounted(first.stable_id()));
        assert_eq!(
            context.world().mount_state(second.stable_id()),
            Some(MountState::Parked)
        );
        assert!(
            context
                .world()
                .document_order(document())
                .contains(&first.stable_id())
        );
        assert!(
            !context
                .set_desktop_slots(shell, Some(primary.stable_id()), Some(first.stable_id()),)
                .unwrap()
        );
    }

    #[test]
    fn set_desktop_slots_repeated_inspector_swap_reuses_parked_trees() {
        let mut context = AppContext::new();
        let navigation = context
            .create_detached_component(document(), SidebarFrame::new())
            .unwrap();
        let primary = context
            .create_detached_component(document(), Text::new("primary"))
            .unwrap();
        let first = context
            .create_detached_component(document(), Text::new("inspector-a"))
            .unwrap();
        let second = context
            .create_detached_component(document(), Text::new("inspector-b"))
            .unwrap();
        let shell = assemble_shell(
            &mut context,
            DesktopShell::new()
                .navigation(navigation.stable_id())
                .primary(primary.stable_id())
                .inspector(first.stable_id()),
        );
        for round in 0..32 {
            let inspector = if round % 2 == 0 {
                second.stable_id()
            } else {
                first.stable_id()
            };
            assert!(
                context
                    .set_desktop_slots(shell, Some(primary.stable_id()), Some(inspector))
                    .unwrap()
            );
        }
        assert!(context.world().is_mounted(first.stable_id()));
        assert_eq!(
            context.world().mount_state(second.stable_id()),
            Some(MountState::Parked)
        );
        assert!(
            !context
                .set_desktop_slots(shell, Some(primary.stable_id()), Some(first.stable_id()),)
                .unwrap()
        );
    }

    #[test]
    fn desktop_shell_keeps_title_off_trailing_edge_and_sidebar_below_chrome() {
        let mut context = AppContext::new();
        let leading = context
            .create_detached_component(
                document(),
                IconButton::new(Icon::Sidebar, "切换侧栏").size(ControlSize::Small),
            )
            .unwrap();
        let files = context
            .create_detached_component(document(), Text::new("文件"))
            .unwrap();
        let navigation = context
            .create_detached_component(document(), SidebarFrame::new().body(files.stable_id()))
            .unwrap();
        context.append_child(navigation, files).unwrap();
        let toolbar = context
            .create_detached_component(document(), Text::new("toolbar"))
            .unwrap();
        let primary = context
            .create_detached_component(document(), Text::new("primary"))
            .unwrap();
        let inspector = context
            .create_detached_component(document(), Text::new("通道"))
            .unwrap();
        let layout = WorkspaceLayout::new([
            RegionState::new(RegionId::Resources, RegionRole::Resources).size(220.0),
            RegionState::new(RegionId::PrimaryToolbar, RegionRole::Utility)
                .placement(RegionPlacement::Top)
                .scope(RegionScope::Primary)
                .size(34.0),
            RegionState::new(RegionId::Primary, RegionRole::Primary).fill_priority(1),
            RegionState::new(RegionId::Inspector, RegionRole::Inspector).size(286.0),
        ])
        .expect("editor regions");
        let shell = assemble_shell(
            &mut context,
            DesktopShell::from_model(WorkspaceModel::with_layout(layout))
                .title("NanaShader")
                .title_leading(leading.stable_id())
                .navigation(navigation.stable_id())
                .primary(primary.stable_id())
                .inspector(inspector.stable_id())
                .region(RegionId::PrimaryToolbar, toolbar.stable_id()),
        );
        context
            .layout_document(document(), LayoutViewport::new(1200.0, 800.0))
            .unwrap();

        let title_bar = context
            .read(shell, |shell| shell.title_bar)
            .unwrap()
            .expect("assembled title bar");
        assert_eq!(context.world().text(title_bar), Some(""));
        let columns = context.world().node(title_bar).unwrap().children;
        assert_eq!(columns.len(), 3);
        let title_label = context
            .world()
            .node(columns[1])
            .unwrap()
            .children
            .first()
            .copied()
            .expect("center column title");
        assert_eq!(context.world().text(title_label), Some("NanaShader"));
        let title_style = context.world().node_style(title_bar).unwrap();
        assert_eq!(
            title_style.text_horizontal_alignment,
            TextHorizontalAlignment::Center
        );

        let bar_box = context.world().canonical_layout_box(title_bar).unwrap();
        let center_box = context.world().canonical_layout_box(columns[1]).unwrap();
        let title_mid = center_box.x + center_box.width / 2.0;
        assert!(
            title_mid < bar_box.x + bar_box.width * 0.65,
            "title sat on the trailing edge at {title_mid}"
        );
        let leading_box = context
            .world()
            .canonical_layout_box(leading.stable_id())
            .unwrap();
        assert!(
            leading_box.width < 80.0,
            "leading chrome must hug, got {}",
            leading_box.width
        );
        if !WindowChrome::platform_default().uses_custom_controls() {
            assert!(
                leading_box.x + 0.5 >= bar_box.x + NATIVE_WINDOW_CONTROLS_WIDTH,
                "toggle overlapped the traffic lights"
            );
        }

        let nav_box = context
            .world()
            .canonical_layout_box(navigation.stable_id())
            .unwrap();
        let files_box = context
            .world()
            .canonical_layout_box(files.stable_id())
            .unwrap();
        assert!(
            nav_box.y + 0.5 >= bar_box.y + bar_box.height,
            "sidebar entered the title-bar / traffic-light band"
        );
        assert!(
            files_box.y + 0.5 >= bar_box.y + bar_box.height,
            "文件 header overlapped native window chrome"
        );

        let toolbar_box = context
            .world()
            .canonical_layout_box(toolbar.stable_id())
            .unwrap();
        let primary_box = context
            .world()
            .canonical_layout_box(primary.stable_id())
            .unwrap();
        let inspector_box = context
            .world()
            .canonical_layout_box(inspector.stable_id())
            .unwrap();
        assert!(
            inspector_box.y + 0.5 >= toolbar_box.y + toolbar_box.height,
            "inspector tabs must not sit on the window toolbar row"
        );
        assert!(
            inspector_box.x + 0.5 >= primary_box.x + primary_box.width,
            "inspector must be a right column beside the workspace"
        );
    }
}
