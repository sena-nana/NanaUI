use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, FlexDirection, LengthSpec, OverflowSpec, PositionSpec, RESIZE_HANDLE_SIZE, RegionId,
    RegionPlacement, RegionRole, RegionScope, RegionState, SemanticColorRole, UI_METRICS,
    WorkspaceLayout, WorkspaceModel,
};

use crate::view_components::{List, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, Entity, FrameworkError,
    InteractionState, MutationQueue, NodeKind, NodeStyle, StableNodeId, TextContent, UiWorld,
};

const BOTTOM_SEPARATOR_PX: f32 = 1.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RegionEdges {
    start: bool,
    end: bool,
}

fn primary_edges(expanded: bool, has_track_before: bool, has_track_after: bool) -> RegionEdges {
    RegionEdges {
        start: expanded && !has_track_before,
        end: expanded && !has_track_after,
    }
}

/// Host-mounted region surface. Application content stays a child of `content`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRegionSlot {
    pub id: RegionId,
    pub content: Option<StableNodeId>,
}

impl WorkspaceRegionSlot {
    pub fn new(id: RegionId, content: StableNodeId) -> Self {
        Self {
            id,
            content: Some(content),
        }
    }
}

/// 8px inner-edge hit target. Painted by [`Workspace`]; this type only owns identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceResizeHandle {
    pub region: RegionId,
}

impl WorkspaceResizeHandle {
    pub fn new(region: RegionId) -> Self {
        Self { region }
    }
}

impl ComponentView for WorkspaceResizeHandle {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "workspace-resize-handle".into(),
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
            &default_handle_style(false),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            handle_accessibility(&self.region),
        );
    }
}

/// Backend-neutral workspace chrome. Snapshot the model; `project` never owns a clock.
#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub slots: Vec<WorkspaceRegionSlot>,
    pub style: NodeStyle,
    pub layout: WorkspaceLayout,
    pub extents: HashMap<RegionId, f32>,
    pub hovered_resize: Option<RegionId>,
    pub transitioning: HashSet<RegionId>,
    pub overlays: HashSet<RegionId>,
    pub inline_size: f32,
    pub workspace_corners: bool,
    pub handles: HashMap<RegionId, StableNodeId>,
    pub middle: Option<StableNodeId>,
    pub primary_column: Option<StableNodeId>,
    pub primary_row: Option<StableNodeId>,
}

impl Workspace {
    pub fn new() -> Self {
        Self::from_model(&WorkspaceModel::new(), [])
    }

    pub fn from_model(
        model: &WorkspaceModel,
        slots: impl IntoIterator<Item = WorkspaceRegionSlot>,
    ) -> Self {
        let layout = model.layout().clone();
        let mut extents = HashMap::with_capacity(layout.regions().len());
        let mut transitioning = HashSet::new();
        let mut overlays = HashSet::new();
        let mut hovered_resize = None;
        for state in layout.regions() {
            let id = state.id().clone();
            extents.insert(id.clone(), model.region_extent(&id));
            if model.region_transitioning(&id) {
                transitioning.insert(id.clone());
            }
            if model.region_overlay(state) {
                overlays.insert(id.clone());
            }
            if hovered_resize.is_none() && model.resize_highlighted(&id) {
                hovered_resize = Some(id);
            }
        }
        Self {
            slots: slots.into_iter().collect(),
            style: NodeStyle::default(),
            layout,
            extents,
            hovered_resize,
            transitioning,
            overlays,
            inline_size: model.inline_size(),
            workspace_corners: true,
            handles: HashMap::new(),
            middle: None,
            primary_column: None,
            primary_row: None,
        }
    }

    /// Refresh model-derived fields without replacing host slots or chrome.
    pub fn refresh_from_model(&mut self, model: &WorkspaceModel) {
        let slots = std::mem::take(&mut self.slots);
        let style = self.style.clone();
        let handles = std::mem::take(&mut self.handles);
        let workspace_corners = self.workspace_corners;
        let middle = self.middle;
        let primary_column = self.primary_column;
        let primary_row = self.primary_row;
        *self = Self::from_model(model, slots);
        self.style = style;
        self.handles = handles;
        self.workspace_corners = workspace_corners;
        self.middle = middle;
        self.primary_column = primary_column;
        self.primary_row = primary_row;
    }

    pub fn slot(mut self, id: RegionId, content: StableNodeId) -> Self {
        if let Some(existing) = self.slots.iter_mut().find(|slot| slot.id == id) {
            existing.content = Some(content);
        } else {
            self.slots.push(WorkspaceRegionSlot::new(id, content));
        }
        self
    }

    pub fn handle(mut self, id: RegionId, handle: StableNodeId) -> Self {
        self.handles.insert(id, handle);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn middle(mut self, middle: StableNodeId) -> Self {
        self.middle = Some(middle);
        self
    }

    pub fn primary_column(mut self, primary_column: StableNodeId) -> Self {
        self.primary_column = Some(primary_column);
        self
    }

    pub fn primary_row(mut self, primary_row: StableNodeId) -> Self {
        self.primary_row = Some(primary_row);
        self
    }

    pub fn middle_track() -> List {
        track_host(FlexDirection::Row, true)
    }

    pub fn primary_stack() -> List {
        track_host(FlexDirection::Column, true)
    }

    pub fn primary_track() -> List {
        track_host(FlexDirection::Row, true)
    }

    pub fn region_extent(&self, id: &RegionId) -> f32 {
        self.extents.get(id).copied().unwrap_or_else(|| {
            self.layout
                .region(id)
                .map(|state| {
                    if state.collapsed_value() {
                        0.0
                    } else {
                        state.extent()
                    }
                })
                .unwrap_or(0.0)
        })
    }

    pub fn shows_resize_handle(&self, id: &RegionId) -> bool {
        let Some(state) = self.layout.region(id) else {
            return false;
        };
        state.resizable_value()
            && !state.disabled_value()
            && state.fill_priority_value() == 0
            && !self.transitioning.contains(id)
            && self.region_visible(state)
    }

    fn region_visible(&self, state: &RegionState) -> bool {
        if self.transitioning.contains(state.id()) {
            !state.hidden_value() && !state.responsive_collapsed(self.inline_size)
        } else {
            state.visible_at(self.inline_size)
        }
    }

    fn region_overlay(&self, state: &RegionState) -> bool {
        self.overlays.contains(state.id())
    }

    fn slot_content(&self, id: &RegionId) -> Option<StableNodeId> {
        self.slots
            .iter()
            .find(|slot| &slot.id == id)
            .and_then(|slot| slot.content)
    }

    fn effective_root_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.background = Some(style.background.unwrap_or(SemanticColorRole::Surface));
        style.foreground = Some(style.foreground.unwrap_or(SemanticColorRole::Text));
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(layout.width.unwrap_or(LengthSpec::Fill));
        layout.height = Some(layout.height.unwrap_or(LengthSpec::Fill));
        layout.overflow_x = OverflowSpec::Hidden;
        layout.overflow_y = OverflowSpec::Hidden;
        layout.position = PositionSpec::Relative;
        style
    }

    fn project_root(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
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
            &self.effective_root_style(),
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

    fn project_track_host(
        &self,
        id: Option<StableNodeId>,
        direction: FlexDirection,
        fill: bool,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        let Some(id) = id else {
            return;
        };
        if !world.contains(id) {
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
        if world.standard_visual(id).is_some() {
            mutations.set_standard_visual(id, None);
        }
        project_common(
            id,
            world,
            mutations,
            &track_host_style(direction, fill),
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

    fn project_region(
        &self,
        state: &RegionState,
        content: StableNodeId,
        edges: RegionEdges,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        let overlay = self.region_overlay(state);
        let style = region_style(
            state,
            self.region_extent(state.id()),
            overlay,
            edges,
            self.workspace_corners,
        );
        project_common(
            content,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::from(state.id().as_str())),
                ..AccessibilityState::default()
            },
        );
        self.project_handle(state, content, world, mutations);
    }

    fn project_handle(
        &self,
        state: &RegionState,
        region: StableNodeId,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        let Some(&handle) = self.handles.get(state.id()) else {
            return;
        };
        if !world.contains(handle) {
            return;
        }
        let show = self.shows_resize_handle(state.id());
        if show {
            if world
                .node(region)
                .is_none_or(|node| !node.children.contains(&handle))
            {
                mutations.insert(region, handle, None);
            }
            let highlighted = self.hovered_resize.as_ref() == Some(state.id());
            if world.text(handle) != Some("") {
                mutations.set_text(
                    handle,
                    TextContent {
                        value: String::new(),
                    },
                );
            }
            if world.standard_visual(handle).is_some() {
                mutations.set_standard_visual(handle, None);
            }
            project_common(
                handle,
                world,
                mutations,
                &handle_style(state.placement_value(), highlighted),
                InteractionState {
                    pointer_events: true,
                    focusable: true,
                },
                handle_accessibility(state.id()),
            );
        } else if world
            .node(handle)
            .is_some_and(|node| node.parent == Some(region))
        {
            mutations.park_subtree(handle);
        }
    }

    fn visible_groups(&self) -> VisibleGroups<'_> {
        let mut groups = VisibleGroups::default();
        let mut starts_expanded = false;
        let mut ends_expanded = false;
        let mut primary_expanded = 0usize;
        for state in self.layout.regions() {
            if self.slot_content(state.id()).is_none() || !self.region_visible(state) {
                continue;
            }
            if self.region_overlay(state) {
                continue;
            }
            match state.placement_value() {
                RegionPlacement::Start => starts_expanded |= !state.collapsed_value(),
                RegionPlacement::End => ends_expanded |= !state.collapsed_value(),
                RegionPlacement::Primary => {
                    primary_expanded += usize::from(!state.collapsed_value());
                }
                RegionPlacement::Top | RegionPlacement::Bottom => {}
            }
        }

        let mut has_track_before = starts_expanded;
        let mut expanded_after = primary_expanded;
        for state in self.layout.regions() {
            let Some(content) = self.slot_content(state.id()) else {
                continue;
            };
            if !self.region_visible(state) {
                continue;
            }
            let overlay = self.region_overlay(state);
            let mut edges = RegionEdges::default();
            if !overlay && state.role() == RegionRole::Primary {
                let expanded = !state.collapsed_value();
                expanded_after = expanded_after.saturating_sub(usize::from(expanded));
                edges = primary_edges(
                    expanded,
                    has_track_before,
                    expanded_after > 0 || ends_expanded,
                );
                has_track_before |= expanded;
            }
            let item = VisibleRegion {
                state,
                content,
                edges,
            };
            if overlay {
                groups.overlays.push(item);
                continue;
            }
            match (state.placement_value(), state.scope_value()) {
                (RegionPlacement::Start, _) => groups.starts.push(item),
                (RegionPlacement::Primary, _) => groups.primaries.push(item),
                (RegionPlacement::End, _) => groups.ends.push(item),
                (RegionPlacement::Top, RegionScope::Workspace) => groups.workspace_top.push(item),
                (RegionPlacement::Top, RegionScope::Primary) => groups.primary_top.push(item),
                (RegionPlacement::Bottom, RegionScope::Workspace) => {
                    groups.workspace_bottom.push(item)
                }
                (RegionPlacement::Bottom, RegionScope::Primary) => groups.primary_bottom.push(item),
            }
        }
        groups
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for Workspace {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "workspace".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.project_root(id, world, mutations);
        let groups = self.visible_groups();
        let chrome = match (self.middle, self.primary_column, self.primary_row) {
            (Some(middle), Some(primary_column), Some(primary_row))
                if world.contains(middle)
                    && world.contains(primary_column)
                    && world.contains(primary_row) =>
            {
                Some((middle, primary_column, primary_row))
            }
            _ => None,
        };
        if let Some((middle, primary_column, primary_row)) = chrome {
            self.project_track_host(Some(middle), FlexDirection::Row, true, world, mutations);
            self.project_track_host(
                Some(primary_column),
                FlexDirection::Column,
                true,
                world,
                mutations,
            );
            self.project_track_host(
                Some(primary_row),
                FlexDirection::Row,
                true,
                world,
                mutations,
            );
        }

        for item in groups.iter() {
            if world.contains(item.content) {
                self.project_region(item.state, item.content, item.edges, world, mutations);
            }
        }

        if let Some((middle, primary_column, primary_row)) = chrome {
            reconcile_children(
                id,
                &chain_ids([
                    contents(&groups.workspace_top),
                    vec![middle],
                    contents(&groups.workspace_bottom),
                    contents(&groups.overlays),
                ]),
                world,
                mutations,
            );
            reconcile_children(
                middle,
                &chain_ids([
                    contents(&groups.starts),
                    vec![primary_column],
                    contents(&groups.ends),
                ]),
                world,
                mutations,
            );
            reconcile_children(
                primary_column,
                &chain_ids([
                    contents(&groups.primary_top),
                    vec![primary_row],
                    contents(&groups.primary_bottom),
                ]),
                world,
                mutations,
            );
            reconcile_children(primary_row, &contents(&groups.primaries), world, mutations);
        } else {
            // Placement order so unique slots stay Start / Primary / End / Top / Bottom.
            reconcile_children(
                id,
                &chain_ids([
                    contents(&groups.starts),
                    contents(&groups.primaries),
                    contents(&groups.ends),
                    contents(&groups.workspace_top),
                    contents(&groups.primary_top),
                    contents(&groups.workspace_bottom),
                    contents(&groups.primary_bottom),
                    contents(&groups.overlays),
                ]),
                world,
                mutations,
            );
        }
    }
}

#[derive(Clone, Copy)]
struct VisibleRegion<'a> {
    state: &'a RegionState,
    content: StableNodeId,
    edges: RegionEdges,
}

#[derive(Default)]
struct VisibleGroups<'a> {
    starts: Vec<VisibleRegion<'a>>,
    primaries: Vec<VisibleRegion<'a>>,
    ends: Vec<VisibleRegion<'a>>,
    workspace_top: Vec<VisibleRegion<'a>>,
    primary_top: Vec<VisibleRegion<'a>>,
    workspace_bottom: Vec<VisibleRegion<'a>>,
    primary_bottom: Vec<VisibleRegion<'a>>,
    overlays: Vec<VisibleRegion<'a>>,
}

impl<'a> VisibleGroups<'a> {
    fn iter(&self) -> impl Iterator<Item = VisibleRegion<'a>> {
        self.starts
            .iter()
            .copied()
            .chain(self.primaries.iter().copied())
            .chain(self.ends.iter().copied())
            .chain(self.workspace_top.iter().copied())
            .chain(self.primary_top.iter().copied())
            .chain(self.workspace_bottom.iter().copied())
            .chain(self.primary_bottom.iter().copied())
            .chain(self.overlays.iter().copied())
    }
}

fn contents(regions: &[VisibleRegion<'_>]) -> Vec<StableNodeId> {
    regions.iter().map(|region| region.content).collect()
}

fn chain_ids(parts: impl IntoIterator<Item = Vec<StableNodeId>>) -> Vec<StableNodeId> {
    parts.into_iter().flatten().collect()
}

fn track_host(direction: FlexDirection, fill: bool) -> List {
    let mut host = List::new();
    host.style = track_host_style(direction, fill);
    host
}

fn track_host_style(direction: FlexDirection, fill: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.direction = Some(direction);
    layout.align_items = AlignSpec::Stretch;
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    layout.overflow_x = OverflowSpec::Hidden;
    layout.overflow_y = OverflowSpec::Hidden;
    if fill {
        layout.flex_grow = Some(1.0);
        layout.flex_shrink = Some(1.0);
        layout.min_width = Some(LengthSpec::Px(0.0));
        layout.min_height = Some(LengthSpec::Px(0.0));
    }
    style
}

fn region_style(
    state: &RegionState,
    extent: f32,
    overlay: bool,
    edges: RegionEdges,
    workspace_corners: bool,
) -> NodeStyle {
    let horizontal = matches!(
        state.placement_value(),
        RegionPlacement::Start | RegionPlacement::Primary | RegionPlacement::End
    );
    let fill = state.fill_priority_value() > 0;
    let track = if fill {
        LengthSpec::Fill
    } else if state.size_value().is_some() {
        LengthSpec::Px(extent)
    } else {
        LengthSpec::Shrink
    };
    let mut style = NodeStyle::default();
    style.foreground = Some(SemanticColorRole::Text);
    style.background = Some(if state.role() == RegionRole::Primary {
        SemanticColorRole::Background
    } else {
        SemanticColorRole::Surface
    });
    let layout = Arc::make_mut(&mut style.layout);
    layout.overflow_x = OverflowSpec::Hidden;
    layout.overflow_y = OverflowSpec::Hidden;
    layout.align_items = AlignSpec::Stretch;
    if overlay {
        layout.position = PositionSpec::Absolute;
        layout.z_index = Some(1);
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        match state.placement_value() {
            RegionPlacement::Start | RegionPlacement::Primary => {
                layout.offset_left = Some(LengthSpec::Px(0.0));
                layout.offset_top = Some(LengthSpec::Px(0.0));
                layout.offset_bottom = Some(LengthSpec::Px(0.0));
                layout.width = Some(track);
                layout.height = Some(LengthSpec::Fill);
            }
            RegionPlacement::End => {
                layout.offset_right = Some(LengthSpec::Px(0.0));
                layout.offset_top = Some(LengthSpec::Px(0.0));
                layout.offset_bottom = Some(LengthSpec::Px(0.0));
                layout.width = Some(track);
                layout.height = Some(LengthSpec::Fill);
            }
            RegionPlacement::Top => {
                layout.offset_top = Some(LengthSpec::Px(0.0));
                layout.offset_left = Some(LengthSpec::Px(0.0));
                layout.offset_right = Some(LengthSpec::Px(0.0));
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(track);
            }
            RegionPlacement::Bottom => {
                layout.offset_bottom = Some(LengthSpec::Px(0.0));
                layout.offset_left = Some(LengthSpec::Px(0.0));
                layout.offset_right = Some(LengthSpec::Px(0.0));
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(track);
            }
        }
    } else {
        layout.position = PositionSpec::Relative;
        if fill {
            layout.flex_grow = Some(f32::from(state.fill_priority_value()));
            layout.flex_shrink = Some(1.0);
            layout.allow_shrink = true;
        } else {
            layout.flex_grow = Some(0.0);
            layout.flex_shrink = Some(0.0);
        }
        if horizontal {
            layout.width = Some(track);
            layout.height = Some(LengthSpec::Fill);
            layout.min_width = Some(LengthSpec::Px(state.min_size_value()));
            layout.max_width = Some(LengthSpec::Px(state.max_size_value()));
        } else {
            layout.width = Some(LengthSpec::Fill);
            layout.height = Some(track);
            layout.min_height = Some(LengthSpec::Px(state.min_size_value()));
            layout.max_height = Some(LengthSpec::Px(state.max_size_value()));
        }
    }
    if state.placement_value() == RegionPlacement::Bottom {
        // 1px reserved top gap. LayoutStyle has no border-top token.
        layout.padding_top = Some(LengthSpec::Px(BOTTOM_SEPARATOR_PX));
    }
    if state.role() == RegionRole::Primary {
        layout.border_radius = Some(primary_radius(workspace_corners, edges));
    }
    style
}

fn primary_radius(workspace_corners: bool, edges: RegionEdges) -> f32 {
    if !workspace_corners {
        return 0.0;
    }
    let rounded_start = !edges.start;
    let rounded_end = !edges.end;
    if rounded_start || rounded_end {
        UI_METRICS.radius_lg
    } else {
        0.0
    }
}

fn default_handle_style(highlighted: bool) -> NodeStyle {
    handle_style(RegionPlacement::Start, highlighted)
}

fn handle_style(placement: RegionPlacement, highlighted: bool) -> NodeStyle {
    let horizontal = matches!(
        placement,
        RegionPlacement::Start | RegionPlacement::Primary | RegionPlacement::End
    );
    let mut style = NodeStyle::default();
    style.background = highlighted.then_some(SemanticColorRole::BorderStrong);
    let layout = Arc::make_mut(&mut style.layout);
    layout.position = PositionSpec::Absolute;
    layout.z_index = Some(2);
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    if horizontal {
        layout.width = Some(LengthSpec::Px(RESIZE_HANDLE_SIZE));
        layout.height = Some(LengthSpec::Fill);
        layout.min_width = Some(LengthSpec::Px(RESIZE_HANDLE_SIZE));
        layout.offset_top = Some(LengthSpec::Px(0.0));
        layout.offset_bottom = Some(LengthSpec::Px(0.0));
        match placement {
            RegionPlacement::End => layout.offset_left = Some(LengthSpec::Px(0.0)),
            _ => layout.offset_right = Some(LengthSpec::Px(0.0)),
        }
    } else {
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(RESIZE_HANDLE_SIZE));
        layout.min_height = Some(LengthSpec::Px(RESIZE_HANDLE_SIZE));
        layout.offset_left = Some(LengthSpec::Px(0.0));
        layout.offset_right = Some(LengthSpec::Px(0.0));
        match placement {
            RegionPlacement::Bottom => layout.offset_top = Some(LengthSpec::Px(0.0)),
            _ => layout.offset_bottom = Some(LengthSpec::Px(0.0)),
        }
    }
    style
}

impl AppContext {
    /// Mount the start | primary-column | end chrome hosts, then re-project.
    pub fn assemble_workspace(
        &mut self,
        workspace: Entity<Workspace>,
    ) -> Result<bool, FrameworkError> {
        let document = self
            .world()
            .node(workspace.stable_id())
            .map(|node| node.document)
            .ok_or(FrameworkError::MissingView(workspace.stable_id()))?;
        let (mut middle, mut primary_column, mut primary_row) =
            self.read(workspace, |workspace| {
                (
                    workspace.middle,
                    workspace.primary_column,
                    workspace.primary_row,
                )
            })?;
        if middle.is_none() {
            middle = Some(
                self.create_detached_component(document, Workspace::middle_track())?
                    .stable_id(),
            );
        }
        if primary_column.is_none() {
            primary_column = Some(
                self.create_detached_component(document, Workspace::primary_stack())?
                    .stable_id(),
            );
        }
        if primary_row.is_none() {
            primary_row = Some(
                self.create_detached_component(document, Workspace::primary_track())?
                    .stable_id(),
            );
        }
        self.update_component(workspace, |workspace, _| {
            workspace.middle = middle;
            workspace.primary_column = primary_column;
            workspace.primary_row = primary_row;
        })?;
        Ok(true)
    }
}

fn handle_accessibility(region: &RegionId) -> AccessibilityState {
    AccessibilityState {
        role: AccessibilityRole::Generic,
        label: Some(Arc::from(format!("resize {region}"))),
        ..AccessibilityState::default()
    }
}

fn reconcile_children(
    parent: StableNodeId,
    desired: &[StableNodeId],
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    let desired = desired
        .iter()
        .copied()
        .filter(|id| *id != parent && world.contains(*id))
        .collect::<Vec<_>>();
    let current = world
        .node(parent)
        .map(|node| node.children)
        .unwrap_or_default();
    if current.as_slice() == desired.as_slice() {
        return;
    }
    for child in &current {
        if !desired.contains(child) {
            mutations.park_subtree(*child);
        }
    }
    for child in desired {
        mutations.insert(parent, child, None);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nana_ui_core::{NarrowBehavior, RegionRole, RegionScope, WorkspaceMutation};

    use super::*;
    use crate::{AppContext, DocumentId, MountState, PositionSpec};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn five_region_layout() -> WorkspaceLayout {
        WorkspaceLayout::new([
            RegionState::new(RegionId::Resources, RegionRole::Resources)
                .size(200.0)
                .resizable(true),
            RegionState::new(RegionId::Primary, RegionRole::Primary)
                .min_size(120.0)
                .fill_priority(1),
            RegionState::new(RegionId::Inspector, RegionRole::Inspector)
                .size(180.0)
                .resizable(true),
            RegionState::new(RegionId::PrimaryToolbar, RegionRole::Utility)
                .placement(RegionPlacement::Top)
                .scope(RegionScope::Workspace)
                .size(34.0),
            RegionState::new(RegionId::Diagnostics, RegionRole::Console)
                .placement(RegionPlacement::Bottom)
                .scope(RegionScope::Workspace)
                .size(80.0)
                .resizable(true),
        ])
        .expect("unique placements")
    }

    fn surface(context: &mut AppContext) -> StableNodeId {
        context
            .create_component(document(), List::new())
            .expect("region surface")
            .stable_id()
    }

    fn mount(context: &mut AppContext, workspace: Workspace) -> crate::Entity<Workspace> {
        context
            .create_component(document(), workspace)
            .expect("workspace")
    }

    #[test]
    fn unique_region_slots_project_visible_children_in_start_primary_end_top_bottom_order() {
        let mut context = AppContext::new();
        let start = surface(&mut context);
        let primary = surface(&mut context);
        let end = surface(&mut context);
        let top = surface(&mut context);
        let bottom = surface(&mut context);
        let model = WorkspaceModel::with_layout(five_region_layout());
        let workspace = Workspace::from_model(
            &model,
            [
                WorkspaceRegionSlot::new(RegionId::Resources, start),
                WorkspaceRegionSlot::new(RegionId::Primary, primary),
                WorkspaceRegionSlot::new(RegionId::Inspector, end),
                WorkspaceRegionSlot::new(RegionId::PrimaryToolbar, top),
                WorkspaceRegionSlot::new(RegionId::Diagnostics, bottom),
            ],
        );
        let entity = mount(&mut context, workspace);
        assert_eq!(
            context.world().node(entity.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "workspace".into(),
            }
        );
        assert_eq!(
            context.world().node(entity.stable_id()).unwrap().children,
            vec![start, primary, end, top, bottom]
        );
        assert_eq!(
            context
                .world()
                .node_style(entity.stable_id())
                .unwrap()
                .background,
            Some(SemanticColorRole::Surface)
        );
        let root_layout = &context
            .world()
            .node_style(entity.stable_id())
            .unwrap()
            .layout;
        assert!(root_layout.padding.is_none());
        assert!(
            root_layout.padding_left.is_none()
                && root_layout.padding_right.is_none()
                && root_layout.padding_top.is_none()
                && root_layout.padding_bottom.is_none()
        );
        assert_eq!(
            context
                .world()
                .accessibility(entity.stable_id())
                .unwrap()
                .role,
            AccessibilityRole::Generic
        );
        assert_eq!(
            context
                .world()
                .accessibility(start)
                .unwrap()
                .label
                .as_deref(),
            Some("resources")
        );
        assert_eq!(
            context.world().node_style(start).unwrap().layout.width,
            Some(LengthSpec::Px(200.0))
        );
        assert_eq!(
            context.world().node_style(primary).unwrap().layout.width,
            Some(LengthSpec::Fill)
        );
        assert_eq!(
            context.world().node_style(top).unwrap().layout.height,
            Some(LengthSpec::Px(34.0))
        );
        assert_eq!(
            context.world().node_style(bottom).unwrap().layout.height,
            Some(LengthSpec::Px(80.0))
        );
    }

    #[test]
    fn switching_primary_slot_parks_previous_content_and_remounts_it() {
        let mut context = AppContext::new();
        let first = surface(&mut context);
        let second = surface(&mut context);
        let layout =
            WorkspaceLayout::new([
                RegionState::new(RegionId::Primary, RegionRole::Primary).fill_priority(1)
            ])
            .expect("layout");
        let model = WorkspaceModel::with_layout(layout);
        let entity = mount(
            &mut context,
            Workspace::from_model(&model, [WorkspaceRegionSlot::new(RegionId::Primary, first)]),
        );
        assert_eq!(
            context.world().node(entity.stable_id()).unwrap().children,
            vec![first]
        );

        context
            .update_component(entity, |workspace, _| {
                workspace.slots = vec![WorkspaceRegionSlot::new(RegionId::Primary, second)];
            })
            .unwrap();
        assert_eq!(
            context.world().node(entity.stable_id()).unwrap().children,
            vec![second]
        );
        assert_eq!(context.world().mount_state(first), Some(MountState::Parked));
        assert!(context.world().is_mounted(second));
        assert!(!context.world().document_order(document()).contains(&first));

        context
            .update_component(entity, |workspace, _| {
                workspace.slots = vec![WorkspaceRegionSlot::new(RegionId::Primary, first)];
            })
            .unwrap();
        assert_eq!(
            context.world().node(entity.stable_id()).unwrap().children,
            vec![first]
        );
        assert!(context.world().is_mounted(first));
        assert_eq!(
            context.world().mount_state(second),
            Some(MountState::Parked)
        );
        assert!(context.world().document_order(document()).contains(&first));
    }

    #[test]
    fn collapsed_and_hidden_regions_omit_content_from_the_track() {
        let mut context = AppContext::new();
        let start = surface(&mut context);
        let primary = surface(&mut context);
        let hidden = surface(&mut context);
        let layout = WorkspaceLayout::new([
            RegionState::new(RegionId::Resources, RegionRole::Resources)
                .size(200.0)
                .collapsible(true)
                .collapsed(true),
            RegionState::new(RegionId::Primary, RegionRole::Primary).fill_priority(1),
            RegionState::new(RegionId::Inspector, RegionRole::Inspector)
                .size(180.0)
                .hidden(true),
        ])
        .expect("layout");
        let model = WorkspaceModel::with_layout(layout);
        let entity = mount(
            &mut context,
            Workspace::from_model(
                &model,
                [
                    WorkspaceRegionSlot::new(RegionId::Resources, start),
                    WorkspaceRegionSlot::new(RegionId::Primary, primary),
                    WorkspaceRegionSlot::new(RegionId::Inspector, hidden),
                ],
            ),
        );
        assert_eq!(
            context.world().node(entity.stable_id()).unwrap().children,
            vec![primary]
        );
        assert!(context.world().node(start).unwrap().parent.is_none());
        assert!(context.world().node(hidden).unwrap().parent.is_none());
    }

    #[test]
    fn overlay_regions_do_not_steal_flex_space() {
        let mut context = AppContext::new();
        let start = surface(&mut context);
        let primary = surface(&mut context);
        let overlay = surface(&mut context);
        let layout = WorkspaceLayout::new([
            RegionState::new(RegionId::Resources, RegionRole::Resources).size(200.0),
            RegionState::new(RegionId::Primary, RegionRole::Primary).fill_priority(1),
            RegionState::new(RegionId::Inspector, RegionRole::Inspector)
                .size(180.0)
                .narrow_behavior(NarrowBehavior::Overlay)
                .collapse_below(2000.0),
        ])
        .expect("layout");
        let model = WorkspaceModel::with_layout(layout);
        assert!(
            model
                .layout()
                .region(&RegionId::Inspector)
                .unwrap()
                .responsive_overlay(model.inline_size())
        );
        let entity = mount(
            &mut context,
            Workspace::from_model(
                &model,
                [
                    WorkspaceRegionSlot::new(RegionId::Resources, start),
                    WorkspaceRegionSlot::new(RegionId::Primary, primary),
                    WorkspaceRegionSlot::new(RegionId::Inspector, overlay),
                ],
            ),
        );
        assert_eq!(
            context.world().node(entity.stable_id()).unwrap().children,
            vec![start, primary, overlay]
        );
        let overlay_layout = &context.world().node_style(overlay).unwrap().layout;
        assert_eq!(overlay_layout.position, PositionSpec::Absolute);
        assert_eq!(overlay_layout.flex_grow, Some(0.0));
        assert_eq!(overlay_layout.width, Some(LengthSpec::Px(180.0)));
        let start_layout = &context.world().node_style(start).unwrap().layout;
        assert_eq!(start_layout.position, PositionSpec::Relative);
        assert_eq!(start_layout.flex_grow, Some(0.0));
        let primary_layout = &context.world().node_style(primary).unwrap().layout;
        assert_eq!(primary_layout.width, Some(LengthSpec::Fill));
        assert!(primary_layout.flex_grow.unwrap_or(0.0) > 0.0);
    }

    #[test]
    fn from_model_uses_region_extent_after_collapse_and_resize() {
        let mut model = WorkspaceModel::new();
        assert!(model.update(
            WorkspaceMutation::SetRegionSize(RegionId::Resources, 300.0),
            Duration::ZERO,
        ));
        let resized = Workspace::from_model(&model, []);
        assert_eq!(resized.region_extent(&RegionId::Resources), 300.0);
        assert_eq!(
            resized.region_extent(&RegionId::Resources),
            model.region_extent(&RegionId::Resources)
        );

        assert!(model.update(
            WorkspaceMutation::SetRegionCollapsed(RegionId::Resources, true),
            Duration::from_millis(40),
        ));
        let collapsing = Workspace::from_model(&model, []);
        assert_eq!(
            collapsing.region_extent(&RegionId::Resources),
            model.region_extent(&RegionId::Resources)
        );
        assert!(collapsing.transitioning.contains(&RegionId::Resources));
        assert!(collapsing.region_extent(&RegionId::Resources) > 0.0);

        assert!(model.update(
            WorkspaceMutation::AdvanceAnimations,
            Duration::from_millis(40) + nana_ui_core::WORKSPACE_REGION_TRANSITION_DURATION,
        ));
        let collapsed = Workspace::from_model(&model, []);
        assert_eq!(collapsed.region_extent(&RegionId::Resources), 0.0);
        assert!(!collapsed.transitioning.contains(&RegionId::Resources));
        assert!(
            !collapsed.region_visible(
                collapsed
                    .layout
                    .region(&RegionId::Resources)
                    .expect("resources")
            )
        );
    }

    #[test]
    fn layout_json_round_trips_unchanged_through_from_model() {
        let mut model = WorkspaceModel::new();
        assert!(model.update(
            WorkspaceMutation::SetRegionSize(RegionId::Inspector, 312.0),
            Duration::ZERO,
        ));
        assert!(model.update(
            WorkspaceMutation::SetRegionCollapsed(RegionId::Diagnostics, true),
            Duration::from_millis(500),
        ));
        assert!(model.update(
            WorkspaceMutation::AdvanceAnimations,
            Duration::from_millis(500) + nana_ui_core::WORKSPACE_REGION_TRANSITION_DURATION,
        ));
        let json = model.layout_json().expect("layout json");
        let view = Workspace::from_model(&model, []);
        assert_eq!(model.layout_json().expect("layout json after view"), json);
        assert_eq!(view.layout.to_json().expect("cloned layout"), json);
    }

    #[test]
    fn resize_handle_is_omitted_when_fill_disabled_or_transitioning() {
        let mut context = AppContext::new();
        let disabled_id = RegionId::custom("tools");
        let layout = WorkspaceLayout::new([
            RegionState::new(RegionId::Resources, RegionRole::Resources)
                .size(200.0)
                .collapsible(true)
                .resizable(true),
            RegionState::new(RegionId::Primary, RegionRole::Primary)
                .fill_priority(1)
                .resizable(true),
            RegionState::new(disabled_id.clone(), RegionRole::Utility)
                .size(120.0)
                .resizable(true)
                .disabled(true),
        ])
        .expect("layout");
        let mut model = WorkspaceModel::with_layout(layout);
        let resources = surface(&mut context);
        let primary = surface(&mut context);
        let disabled = surface(&mut context);
        let resources_handle = context
            .create_component(document(), WorkspaceResizeHandle::new(RegionId::Resources))
            .unwrap()
            .stable_id();
        let primary_handle = context
            .create_component(document(), WorkspaceResizeHandle::new(RegionId::Primary))
            .unwrap()
            .stable_id();
        let disabled_handle = context
            .create_component(document(), WorkspaceResizeHandle::new(disabled_id.clone()))
            .unwrap()
            .stable_id();

        let idle = Workspace::from_model(
            &model,
            [
                WorkspaceRegionSlot::new(RegionId::Resources, resources),
                WorkspaceRegionSlot::new(RegionId::Primary, primary),
                WorkspaceRegionSlot::new(disabled_id.clone(), disabled),
            ],
        )
        .handle(RegionId::Resources, resources_handle)
        .handle(RegionId::Primary, primary_handle)
        .handle(disabled_id.clone(), disabled_handle);
        assert!(idle.shows_resize_handle(&RegionId::Resources));
        assert!(!idle.shows_resize_handle(&RegionId::Primary));
        assert!(!idle.shows_resize_handle(&disabled_id));

        let entity = mount(&mut context, idle);
        assert_eq!(
            context.world().node(resources).unwrap().children,
            vec![resources_handle]
        );
        assert!(
            context
                .world()
                .interaction(resources_handle)
                .unwrap()
                .focusable
        );
        assert!(
            context
                .world()
                .interaction(resources_handle)
                .unwrap()
                .pointer_events
        );
        assert!(context.world().node(primary).unwrap().children.is_empty());
        assert!(context.world().node(disabled).unwrap().children.is_empty());

        assert!(model.update(
            WorkspaceMutation::SetRegionCollapsed(RegionId::Resources, true),
            Duration::ZERO,
        ));
        context
            .update_component(entity, |workspace, _| {
                workspace.refresh_from_model(&model);
            })
            .unwrap();
        assert!(
            context
                .read(entity, |workspace| {
                    workspace.transitioning.contains(&RegionId::Resources)
                        && !workspace.shows_resize_handle(&RegionId::Resources)
                })
                .unwrap()
        );
        assert!(context.world().node(resources).unwrap().children.is_empty());
    }
}
