use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use nana_ui_core::{
    AlignSpec, FlexDirection, LengthSpec, OverflowSpec, PaddingSpec, PositionSpec,
    RESIZE_HANDLE_SIZE, RegionId, RegionPlacement, RegionRole, RegionScope, RegionState,
    SemanticColorRole, UI_METRICS, WorkspaceLayout, WorkspaceModel, WorkspaceMutation,
};

use crate::view_components::{List, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, DesktopShell, DocumentId,
    Entity, FrameworkError, InteractionState, MutationQueue, NodeKind, NodeStyle, StableNodeId,
    TextContent, UiWorld,
};

const REGION_SEPARATOR_PX: f32 = 1.0;
const HANDLE_HIT_SLOP: f32 = 6.0;

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
    pub editor_stack: Option<StableNodeId>,
    pub model: WorkspaceModel,
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
            editor_stack: None,
            model: model.clone(),
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
        let editor_stack = self.editor_stack;
        *self = Self::from_model(model, slots);
        self.style = style;
        self.handles = handles;
        self.workspace_corners = workspace_corners;
        self.middle = middle;
        self.primary_column = primary_column;
        self.primary_row = primary_row;
        self.editor_stack = editor_stack;
    }

    pub fn apply(&mut self, mutation: WorkspaceMutation, now: Duration) -> bool {
        if !self.model.update(mutation, now) {
            return false;
        }
        let model = self.model.clone();
        self.refresh_from_model(&model);
        true
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

    pub fn editor_stack() -> List {
        track_host(FlexDirection::Column, true)
    }

    pub fn editor_stack_host(mut self, editor_stack: StableNodeId) -> Self {
        self.editor_stack = Some(editor_stack);
        self
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
        let region = region_style(
            state,
            self.region_extent(state.id()),
            overlay,
            edges,
            self.workspace_corners,
        );
        let style = overlay_region_style(world.node_style(content), region);
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
            let highlighted = self.model.resize_highlighted(state.id());
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
            let padding = world
                .node_style(region)
                .map(|style| style.layout.resolved_padding())
                .unwrap_or_default();
            project_common(
                handle,
                world,
                mutations,
                &handle_style(state.placement_value(), highlighted, padding),
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
        let chrome = match (
            self.middle,
            self.primary_column,
            self.primary_row,
            self.editor_stack,
        ) {
            (Some(middle), Some(primary_column), Some(primary_row), Some(editor_stack))
                if world.contains(middle)
                    && world.contains(primary_column)
                    && world.contains(primary_row)
                    && world.contains(editor_stack) =>
            {
                Some((middle, primary_column, primary_row, editor_stack))
            }
            _ => None,
        };
        if let Some((middle, primary_column, primary_row, editor_stack)) = chrome {
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
            self.project_track_host(
                Some(editor_stack),
                FlexDirection::Column,
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

        if let Some((middle, primary_column, primary_row, editor_stack)) = chrome {
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
                &chain_ids([contents(&groups.starts), vec![primary_column]]),
                world,
                mutations,
            );
            reconcile_children(
                primary_column,
                &chain_ids([contents(&groups.primary_top), vec![primary_row]]),
                world,
                mutations,
            );
            reconcile_children(
                primary_row,
                &chain_ids([vec![editor_stack], contents(&groups.ends)]),
                world,
                mutations,
            );
            reconcile_children(
                editor_stack,
                &chain_ids([
                    contents(&groups.primaries),
                    contents(&groups.primary_bottom),
                ]),
                world,
                mutations,
            );
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
    layout.gap = Some(LengthSpec::Px(REGION_SEPARATOR_PX));
    if fill {
        layout.flex_grow = Some(1.0);
        layout.flex_shrink = Some(1.0);
        layout.min_width = Some(LengthSpec::Px(0.0));
        layout.min_height = Some(LengthSpec::Px(0.0));
    }
    style
}

fn keep_if_unset<T: Copy>(slot: &mut Option<T>, kept: Option<T>) {
    if slot.is_none() {
        *slot = kept;
    }
}

/// Region chrome owns size, overflow, and flex. Content such as SidebarFrame
/// keeps padding, gap, and direction unless the region set those edges.
fn overlay_region_style(existing: Option<&NodeStyle>, mut region: NodeStyle) -> NodeStyle {
    let Some(existing) = existing else {
        return region;
    };
    let layout = Arc::make_mut(&mut region.layout);
    let kept = &*existing.layout;
    keep_if_unset(&mut layout.direction, kept.direction);
    keep_if_unset(&mut layout.gap, kept.gap);
    keep_if_unset(&mut layout.padding, kept.padding);
    keep_if_unset(&mut layout.padding_top, kept.padding_top);
    keep_if_unset(&mut layout.padding_right, kept.padding_right);
    keep_if_unset(&mut layout.padding_bottom, kept.padding_bottom);
    keep_if_unset(&mut layout.padding_left, kept.padding_left);
    region
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
    if state.role() == RegionRole::Inspector {
        layout.direction = Some(FlexDirection::Column);
        layout.padding_left = Some(LengthSpec::Px(REGION_SEPARATOR_PX));
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
    handle_style(RegionPlacement::Start, highlighted, PaddingSpec::default())
}

/// 8px bar centered on the painted edge. Insets undo content-box padding.
fn handle_style(placement: RegionPlacement, highlighted: bool, padding: PaddingSpec) -> NodeStyle {
    let horizontal = matches!(
        placement,
        RegionPlacement::Start | RegionPlacement::Primary | RegionPlacement::End
    );
    let half = RESIZE_HANDLE_SIZE / 2.0;
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
        layout.offset_top = Some(LengthSpec::Px(-padding.top));
        layout.offset_bottom = Some(LengthSpec::Px(-padding.bottom));
        match placement {
            RegionPlacement::End => {
                layout.offset_left = Some(LengthSpec::Px(-(padding.left + half)));
            }
            _ => layout.offset_right = Some(LengthSpec::Px(-(padding.right + half))),
        }
    } else {
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(RESIZE_HANDLE_SIZE));
        layout.min_height = Some(LengthSpec::Px(RESIZE_HANDLE_SIZE));
        layout.offset_left = Some(LengthSpec::Px(-padding.left));
        layout.offset_right = Some(LengthSpec::Px(-padding.right));
        match placement {
            RegionPlacement::Bottom => {
                layout.offset_top = Some(LengthSpec::Px(-(padding.top + half)));
            }
            _ => layout.offset_bottom = Some(LengthSpec::Px(-(padding.bottom + half))),
        }
    }
    style
}

impl AppContext {
    /// Mount start | primary-column chrome, with the inspector beside the editor stack.
    pub fn assemble_workspace(
        &mut self,
        workspace: Entity<Workspace>,
    ) -> Result<bool, FrameworkError> {
        let document = self
            .world()
            .node(workspace.stable_id())
            .map(|node| node.document)
            .ok_or(FrameworkError::MissingView(workspace.stable_id()))?;
        let snapshot = self.read(workspace, |workspace| {
            (
                workspace.middle,
                workspace.primary_column,
                workspace.primary_row,
                workspace.editor_stack,
                workspace.handles.clone(),
                workspace.layout.clone(),
                workspace.slots.clone(),
            )
        })?;
        let (
            mut middle,
            mut primary_column,
            mut primary_row,
            mut editor_stack,
            mut handles,
            layout,
            slots,
        ) = snapshot;
        let mut chrome_changed = false;
        if middle.is_none() {
            middle = Some(
                self.create_detached_component(document, Workspace::middle_track())?
                    .stable_id(),
            );
            chrome_changed = true;
        }
        if primary_column.is_none() {
            primary_column = Some(
                self.create_detached_component(document, Workspace::primary_stack())?
                    .stable_id(),
            );
            chrome_changed = true;
        }
        if primary_row.is_none() {
            primary_row = Some(
                self.create_detached_component(document, Workspace::primary_track())?
                    .stable_id(),
            );
            chrome_changed = true;
        }
        if editor_stack.is_none() {
            editor_stack = Some(
                self.create_detached_component(document, Workspace::editor_stack())?
                    .stable_id(),
            );
            chrome_changed = true;
        }
        let mut handles_changed = false;
        for slot in &slots {
            let Some(state) = layout.region(&slot.id) else {
                continue;
            };
            if !wants_resize_handle(state) {
                continue;
            }
            let existing = handles
                .get(&slot.id)
                .copied()
                .filter(|id| self.world().contains(*id));
            if existing.is_some() {
                continue;
            }
            let handle = create_workspace_handle(self, document, slot.id.clone())?;
            handles.insert(slot.id.clone(), handle);
            handles_changed = true;
        }
        if chrome_changed || handles_changed {
            self.update_component(workspace, |workspace, _| {
                workspace.middle = middle;
                workspace.primary_column = primary_column;
                workspace.primary_row = primary_row;
                workspace.editor_stack = editor_stack;
                workspace.handles = handles;
            })?;
        }
        Ok(chrome_changed || handles_changed)
    }

    pub fn is_workspace_resize_handle(&self, id: StableNodeId) -> bool {
        self.workspace_handle_id(id).is_some()
    }

    /// Handle under the pointer, including a few pixels of slop around the 8px bar.
    pub fn workspace_handle_near(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> Option<StableNodeId> {
        if let Some(target) = self.pointer_target(document, x, y)
            && let Some(handle) = self.workspace_handle_id(target)
        {
            return Some(handle);
        }
        self.world()
            .document_order(document)
            .into_iter()
            .find(|&id| {
                self.workspace_handle_id(id).is_some()
                    && self
                        .world()
                        .layout_box(id)
                        .is_some_and(|bounds| point_near_box(bounds, x, y, HANDLE_HIT_SLOP))
            })
    }

    pub fn begin_workspace_resize(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
        now: Duration,
    ) -> Result<bool, FrameworkError> {
        let Some(handle) = self.workspace_handle_id(target) else {
            return Ok(false);
        };
        let Some(workspace) = self.workspace_for_handle(handle) else {
            return Ok(false);
        };
        let Some(region) = self.handle_region(handle) else {
            return Ok(false);
        };
        let changed = self.update_component(workspace, |workspace, cx| {
            let started = workspace.apply(WorkspaceMutation::ResizeStart(region), now);
            let moved = workspace.apply(WorkspaceMutation::ResizeMove { x, y }, now);
            if started || moved {
                cx.mutations().request_focus(document, Some(handle));
                cx.mutations().capture_pointer(pointer_id, handle);
            }
            started || moved
        })?;
        if changed {
            write_back_shell_model(self, workspace)?;
        }
        Ok(changed)
    }

    pub fn update_workspace_resize(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        now: Duration,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(handle) = self.workspace_handle_id(target) else {
            return Ok(false);
        };
        let Some(workspace) = self.workspace_for_handle(handle) else {
            return Ok(false);
        };
        let changed = self.update_component(workspace, |workspace, _| {
            workspace.apply(WorkspaceMutation::ResizeMove { x, y }, now)
        })?;
        if changed {
            write_back_shell_model(self, workspace)?;
        }
        Ok(changed)
    }

    pub fn end_workspace_resize(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        now: Duration,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(handle) = self.workspace_handle_id(target) else {
            return Ok(false);
        };
        let Some(workspace) = self.workspace_for_handle(handle) else {
            return Ok(false);
        };
        let changed = self.update_component(workspace, |workspace, cx| {
            let changed = workspace.apply(WorkspaceMutation::ResizeEnd, now);
            cx.mutations().release_pointer(pointer_id, handle);
            changed
        })?;
        if changed {
            write_back_shell_model(self, workspace)?;
        }
        Ok(changed)
    }

    fn workspace_handle_id(&self, id: StableNodeId) -> Option<StableNodeId> {
        self.read(Entity::<WorkspaceResizeHandle>::from_stable_id(id), |_| ())
            .ok()
            .map(|_| id)
    }

    fn handle_region(&self, handle: StableNodeId) -> Option<RegionId> {
        self.read(
            Entity::<WorkspaceResizeHandle>::from_stable_id(handle),
            |handle| handle.region.clone(),
        )
        .ok()
    }

    fn workspace_for_handle(&self, handle: StableNodeId) -> Option<Entity<Workspace>> {
        let mut current = Some(handle);
        while let Some(id) = current {
            if self
                .read(Entity::<Workspace>::from_stable_id(id), |_| ())
                .is_ok()
            {
                let entity = Entity::<Workspace>::from_stable_id(id);
                let owns = self
                    .read(entity, |workspace| {
                        workspace.handles.values().any(|&id| id == handle)
                    })
                    .ok()
                    .unwrap_or(false);
                if owns {
                    return Some(entity);
                }
            }
            current = self.world().node(id).and_then(|node| node.parent);
        }
        None
    }
}

fn wants_resize_handle(state: &RegionState) -> bool {
    state.resizable_value() && !state.disabled_value() && state.fill_priority_value() == 0
}

fn point_near_box(bounds: crate::LayoutBox, x: f32, y: f32, slop: f32) -> bool {
    x >= bounds.x - slop
        && y >= bounds.y - slop
        && x <= bounds.x + bounds.width + slop
        && y <= bounds.y + bounds.height + slop
}

fn create_workspace_handle(
    context: &mut AppContext,
    document: DocumentId,
    region: RegionId,
) -> Result<StableNodeId, FrameworkError> {
    Ok(context
        .create_detached_component(document, WorkspaceResizeHandle::new(region))?
        .stable_id())
}

fn write_back_shell_model(
    context: &mut AppContext,
    workspace: Entity<Workspace>,
) -> Result<(), FrameworkError> {
    let Some(parent) = context
        .world()
        .node(workspace.stable_id())
        .and_then(|node| node.parent)
    else {
        return Ok(());
    };
    if context
        .read(Entity::<DesktopShell>::from_stable_id(parent), |_| ())
        .is_err()
    {
        return Ok(());
    }
    let model = context.read(workspace, |workspace| workspace.model.clone())?;
    context.update_component(
        Entity::<DesktopShell>::from_stable_id(parent),
        |shell, _| {
            shell.model = model;
        },
    )?;
    Ok(())
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
    use crate::{AppContext, DocumentId, MountState, PositionSpec, SidebarFrame};

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

    #[test]
    fn inspector_stays_separated_from_primary_toolbar() {
        let mut context = AppContext::new();
        let start = surface(&mut context);
        let primary = surface(&mut context);
        let inspector = surface(&mut context);
        let toolbar = surface(&mut context);
        let layout = WorkspaceLayout::new([
            RegionState::new(RegionId::Resources, RegionRole::Resources).size(200.0),
            RegionState::new(RegionId::PrimaryToolbar, RegionRole::Utility)
                .placement(RegionPlacement::Top)
                .scope(RegionScope::Primary)
                .size(34.0),
            RegionState::new(RegionId::Primary, RegionRole::Primary).fill_priority(1),
            RegionState::new(RegionId::Inspector, RegionRole::Inspector).size(180.0),
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
                    WorkspaceRegionSlot::new(RegionId::Inspector, inspector),
                    WorkspaceRegionSlot::new(RegionId::PrimaryToolbar, toolbar),
                ],
            ),
        );
        context.assemble_workspace(entity).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(800.0, 400.0))
            .unwrap();

        let toolbar_box = context.world().layout_box(toolbar).unwrap();
        let primary_box = context.world().layout_box(primary).unwrap();
        let inspector_box = context.world().layout_box(inspector).unwrap();
        assert!(
            inspector_box.y + 0.5 >= toolbar_box.y + toolbar_box.height,
            "inspector header must sit below the toolbar row, not on it"
        );
        assert!(
            inspector_box.x + 0.5 >= primary_box.x + primary_box.width,
            "inspector must be a right column beside the editor"
        );
        assert!(
            (toolbar_box.width - (primary_box.width + inspector_box.width)).abs() < 2.0,
            "toolbar spans editor + inspector, got toolbar {} vs columns {}",
            toolbar_box.width,
            primary_box.width + inspector_box.width
        );
        let inspector_style = &context.world().node_style(inspector).unwrap().layout;
        assert_eq!(inspector_style.direction, Some(FlexDirection::Column));
        assert_eq!(
            inspector_style.padding_left,
            Some(LengthSpec::Px(REGION_SEPARATOR_PX))
        );
    }

    #[test]
    fn assemble_creates_resize_handles_for_resizable_regions() {
        let mut context = AppContext::new();
        let resources = surface(&mut context);
        let primary = surface(&mut context);
        let inspector = surface(&mut context);
        let model = WorkspaceModel::with_layout(five_region_layout());
        let entity = mount(
            &mut context,
            Workspace::from_model(
                &model,
                [
                    WorkspaceRegionSlot::new(RegionId::Resources, resources),
                    WorkspaceRegionSlot::new(RegionId::Primary, primary),
                    WorkspaceRegionSlot::new(RegionId::Inspector, inspector),
                ],
            ),
        );
        assert!(context.assemble_workspace(entity).unwrap());
        let handles = context
            .read(entity, |workspace| workspace.handles.clone())
            .unwrap();
        let resources_handle = *handles.get(&RegionId::Resources).expect("resources handle");
        let inspector_handle = *handles.get(&RegionId::Inspector).expect("inspector handle");
        assert!(handles.get(&RegionId::Primary).is_none());
        assert!(
            context
                .world()
                .node(resources)
                .unwrap()
                .children
                .contains(&resources_handle)
        );
        assert!(
            context
                .world()
                .node(inspector)
                .unwrap()
                .children
                .contains(&inspector_handle)
        );
        assert_eq!(
            context.world().node(resources_handle).unwrap().kind,
            NodeKind::Element {
                tag: "workspace-resize-handle".into(),
            }
        );
        assert!(!context.assemble_workspace(entity).unwrap());
    }

    #[test]
    fn pointer_drag_resizes_resources_and_keeps_size_after_assemble() {
        let mut context = AppContext::new();
        let resources = surface(&mut context);
        let primary = surface(&mut context);
        let model = WorkspaceModel::with_layout(five_region_layout());
        let entity = mount(
            &mut context,
            Workspace::from_model(
                &model,
                [
                    WorkspaceRegionSlot::new(RegionId::Resources, resources),
                    WorkspaceRegionSlot::new(RegionId::Primary, primary),
                ],
            ),
        );
        context.assemble_workspace(entity).unwrap();
        let handle = context
            .read(entity, |workspace| {
                workspace.handles.get(&RegionId::Resources).copied()
            })
            .unwrap()
            .expect("resources handle");
        context
            .commit_mutations({
                let mut mutations = crate::MutationQueue::new();
                mutations.write_layout(
                    handle,
                    crate::LayoutBox {
                        x: 196.0,
                        y: 0.0,
                        width: RESIZE_HANDLE_SIZE,
                        height: 200.0,
                    },
                );
                mutations
            })
            .unwrap();
        context.rebuild_hit_test(document());

        assert_eq!(
            context.workspace_handle_near(document(), 200.0, 20.0),
            Some(handle)
        );
        assert!(
            context
                .begin_workspace_resize(document(), 1, handle, 200.0, 20.0, Duration::ZERO)
                .unwrap()
        );
        assert!(
            context
                .update_workspace_resize(document(), 1, 240.0, 20.0, Duration::ZERO)
                .unwrap()
        );
        assert!(
            context
                .end_workspace_resize(document(), 1, Duration::ZERO)
                .unwrap()
        );
        assert_eq!(
            context
                .read(entity, |workspace| workspace
                    .region_extent(&RegionId::Resources))
                .unwrap(),
            240.0
        );
        assert!(context.world().pointer_capture(document(), 1).is_none());

        context.assemble_workspace(entity).unwrap();
        assert_eq!(
            context
                .read(entity, |workspace| workspace
                    .region_extent(&RegionId::Resources))
                .unwrap(),
            240.0
        );
    }

    #[test]
    fn resize_handle_is_centered_on_the_sidebar_painted_edge() {
        let mut context = AppContext::new();
        let sidebar = context
            .create_component(document(), SidebarFrame::new())
            .expect("sidebar")
            .stable_id();
        let primary = surface(&mut context);
        let layout = WorkspaceLayout::new([
            RegionState::new(RegionId::Resources, RegionRole::Resources)
                .size(200.0)
                .resizable(true),
            RegionState::new(RegionId::Primary, RegionRole::Primary).fill_priority(1),
        ])
        .expect("layout");
        let entity = mount(
            &mut context,
            Workspace::from_model(
                &WorkspaceModel::with_layout(layout),
                [
                    WorkspaceRegionSlot::new(RegionId::Resources, sidebar),
                    WorkspaceRegionSlot::new(RegionId::Primary, primary),
                ],
            ),
        );
        context.assemble_workspace(entity).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(800.0, 400.0))
            .unwrap();

        let handle = context
            .read(entity, |workspace| {
                workspace.handles.get(&RegionId::Resources).copied()
            })
            .unwrap()
            .expect("sidebar handle");
        let region = context.world().layout_box(sidebar).unwrap();
        let bar = context.world().layout_box(handle).unwrap();
        assert!(
            (bar.x + bar.width / 2.0 - (region.x + region.width)).abs() < 0.5,
            "handle center {} vs painted edge {}",
            bar.x + bar.width / 2.0,
            region.x + region.width
        );
    }
}
