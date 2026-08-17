//! Backend-neutral dock chrome. Application pane bodies stay host-mounted slots.

use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, JustifySpec, LengthSpec, OverflowSpec, PositionSpec,
    SemanticColorRole,
};

use crate::tabs::{TabOption, Tabs};
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, DocumentId, Entity,
    FrameworkError, InteractionState, InteractionStyle, MutationQueue, NodeKind, NodeStyle,
    SemanticPaint, StableNodeId, TextContent, TextVerticalAlignment, UiWorld,
};

pub(crate) const DOCK_TITLE_BAR_HEIGHT: f32 = 28.0;
pub(crate) const DOCK_DIVIDER_HIT_SIZE: f32 = 8.0;
pub(crate) const MIN_SPLIT_RATIO: f32 = 0.05;
pub(crate) const MAX_SPLIT_RATIO: f32 = 0.95;

const HANDLE_INDICATOR: f32 = 2.0;
const TITLE_PADDING_X: f32 = 6.0;
const TITLE_SIZE: f32 = 11.0;
const TITLE_WEIGHT: u16 = 600;
const TAB_OVERLAY_THICKNESS: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockDropZone {
    Left,
    Right,
    Top,
    Bottom,
    Tab,
}

/// Recursive dock tree. `ratio` is the first child's share after clamp.
#[derive(Debug, Clone, PartialEq)]
pub enum DockNode {
    Item {
        id: Arc<str>,
        content: Option<StableNodeId>,
    },
    Tabs {
        tabs: Vec<Arc<str>>,
        active: Arc<str>,
        contents: Vec<(Arc<str>, Option<StableNodeId>)>,
    },
    Split {
        axis: DockAxis,
        ratio: f32,
        first: Box<DockNode>,
        second: Box<DockNode>,
    },
}

impl DockNode {
    pub fn item(id: impl Into<Arc<str>>, content: Option<StableNodeId>) -> Self {
        Self::Item {
            id: id.into(),
            content,
        }
    }

    pub fn tabs(
        tabs: impl IntoIterator<Item = impl Into<Arc<str>>>,
        active: impl Into<Arc<str>>,
        contents: impl IntoIterator<Item = (impl Into<Arc<str>>, Option<StableNodeId>)>,
    ) -> Self {
        let tabs = tabs.into_iter().map(Into::into).collect::<Vec<_>>();
        let active = active.into();
        Self::Tabs {
            active: effective_active(&tabs, &active),
            tabs,
            contents: contents
                .into_iter()
                .map(|(id, content)| (id.into(), content))
                .collect(),
        }
    }

    pub fn split(axis: DockAxis, ratio: f32, first: DockNode, second: DockNode) -> Self {
        Self::Split {
            axis,
            ratio: clamp_ratio(ratio),
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    pub fn flatten(&self) -> Vec<Arc<str>> {
        let mut ids = Vec::new();
        collect_ids(self, &mut ids);
        ids
    }

    pub fn clamp_ratios(&mut self) {
        match self {
            Self::Item { .. } => {}
            Self::Tabs { tabs, active, .. } => {
                *active = effective_active(tabs, active);
            }
            Self::Split {
                ratio,
                first,
                second,
                ..
            } => {
                *ratio = clamp_ratio(*ratio);
                first.clamp_ratios();
                second.clamp_ratios();
            }
        }
    }

    #[cfg(test)]
    fn split_ratio(&self) -> Option<f32> {
        match self {
            Self::Split { ratio, .. } => Some(*ratio),
            _ => None,
        }
    }
}

fn collect_ids(node: &DockNode, output: &mut Vec<Arc<str>>) {
    match node {
        DockNode::Item { id, .. } => output.push(Arc::clone(id)),
        DockNode::Tabs { tabs, .. } => output.extend(tabs.iter().cloned()),
        DockNode::Split { first, second, .. } => {
            collect_ids(first, output);
            collect_ids(second, output);
        }
    }
}

fn collect_contents(node: &DockNode, output: &mut Vec<(Arc<str>, Option<StableNodeId>)>) {
    match node {
        DockNode::Item { id, content } => output.push((Arc::clone(id), *content)),
        DockNode::Tabs { contents, .. } => output.extend(contents.iter().cloned()),
        DockNode::Split { first, second, .. } => {
            collect_contents(first, output);
            collect_contents(second, output);
        }
    }
}

fn host_content_ids(node: &DockNode) -> HashSet<StableNodeId> {
    let mut ids = HashSet::new();
    let mut slots = Vec::new();
    collect_contents(node, &mut slots);
    for (_, content) in slots {
        if let Some(id) = content {
            ids.insert(id);
        }
    }
    ids
}

fn effective_active(tabs: &[Arc<str>], active: &Arc<str>) -> Arc<str> {
    if tabs.iter().any(|tab| tab == active) {
        Arc::clone(active)
    } else {
        tabs.first().cloned().unwrap_or_else(|| Arc::clone(active))
    }
}

pub fn clamp_ratio(ratio: f32) -> f32 {
    finite(ratio, 0.5).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

#[derive(Debug, Clone, PartialEq)]
enum DockChrome {
    Item {
        frame: StableNodeId,
        title: StableNodeId,
        overlay: Option<StableNodeId>,
    },
    Tabs {
        frame: StableNodeId,
        strip: StableNodeId,
        overlay: Option<StableNodeId>,
    },
    Split {
        frame: StableNodeId,
        handle: StableNodeId,
        indicator: StableNodeId,
        first: Box<DockChrome>,
        second: Box<DockChrome>,
    },
}

impl DockChrome {
    fn frame(&self) -> StableNodeId {
        match self {
            Self::Item { frame, .. } | Self::Tabs { frame, .. } | Self::Split { frame, .. } => {
                *frame
            }
        }
    }

    #[cfg(test)]
    fn split_children(&self) -> Option<(StableNodeId, StableNodeId, StableNodeId)> {
        match self {
            Self::Split {
                handle,
                first,
                second,
                ..
            } => Some((first.frame(), *handle, second.frame())),
            _ => None,
        }
    }
}

/// Recursive split / tabs / item surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Dock {
    pub root: DockNode,
    pub drop_target: Option<(Arc<str>, DockDropZone)>,
    pub locked: bool,
    pub titles: Vec<(Arc<str>, Arc<str>)>,
    pub style: NodeStyle,
    chrome: Option<DockChrome>,
}

impl Dock {
    pub fn new(mut root: DockNode) -> Self {
        root.clamp_ratios();
        Self {
            root,
            drop_target: None,
            locked: false,
            titles: Vec::new(),
            style: NodeStyle::default(),
            chrome: None,
        }
    }

    pub fn drop_target(mut self, id: impl Into<Arc<str>>, zone: DockDropZone) -> Self {
        self.drop_target = Some((id.into(), zone));
        self
    }

    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    pub fn title(mut self, id: impl Into<Arc<str>>, title: impl Into<Arc<str>>) -> Self {
        let id = id.into();
        let title = title.into();
        if let Some(existing) = self.titles.iter_mut().find(|(key, _)| key == &id) {
            existing.1 = title;
        } else {
            self.titles.push((id, title));
        }
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn flatten(&self) -> Vec<Arc<str>> {
        self.root.flatten()
    }

    fn title_for(&self, id: &str) -> Arc<str> {
        self.titles
            .iter()
            .find(|(key, _)| key.as_ref() == id)
            .map(|(_, title)| Arc::clone(title))
            .unwrap_or_else(|| Arc::from(id))
    }

    fn drop_for(&self, id: &str) -> Option<DockDropZone> {
        self.drop_target
            .as_ref()
            .filter(|(target, _)| target.as_ref() == id)
            .map(|(_, zone)| *zone)
    }

    fn node_drop(&self, node: &DockNode) -> Option<DockDropZone> {
        match node {
            DockNode::Item { id, .. } => self.drop_for(id),
            DockNode::Tabs { tabs, .. } => tabs.iter().find_map(|id| self.drop_for(id)),
            DockNode::Split { .. } => None,
        }
    }
}

impl ComponentView for Dock {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "dock".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut root = self.root.clone();
        root.clamp_ratios();
        if let Some(chrome) = &self.chrome {
            project_chrome(self, chrome, &root, world, mutations);
        } else {
            project_common(
                id,
                world,
                mutations,
                &dock_root_style(&root, &self.style),
                InteractionState {
                    pointer_events: false,
                    focusable: false,
                },
                dock_root_accessibility(&root),
            );
            project_content_slots(&root, None, id, world, mutations);
        }
    }
}

fn dock_root_style(root: &DockNode, base: &NodeStyle) -> NodeStyle {
    let mut style = fill_surface(base.clone());
    let layout = Arc::make_mut(&mut style.layout);
    match root {
        DockNode::Split { axis, .. } => {
            layout.direction = Some(split_direction(*axis));
            layout.align_items = AlignSpec::Stretch;
        }
        DockNode::Item { .. } | DockNode::Tabs { .. } => {
            layout.direction = Some(FlexDirection::Column);
            layout.align_items = AlignSpec::Stretch;
            layout.position = PositionSpec::Relative;
        }
    }
    style
}

fn dock_root_accessibility(root: &DockNode) -> AccessibilityState {
    let label = match root {
        DockNode::Item { id, .. } => Some(Arc::clone(id)),
        DockNode::Tabs { active, .. } => Some(Arc::clone(active)),
        DockNode::Split { .. } => None,
    };
    AccessibilityState {
        role: AccessibilityRole::Generic,
        label,
        ..AccessibilityState::default()
    }
}

fn fill_surface(mut style: NodeStyle) -> NodeStyle {
    style.background = Some(SemanticColorRole::Surface);
    style.foreground = Some(SemanticColorRole::Text);
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(layout.width.unwrap_or(LengthSpec::Fill));
    layout.height = Some(layout.height.unwrap_or(LengthSpec::Fill));
    layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
    layout.min_height = Some(layout.min_height.unwrap_or(LengthSpec::Px(0.0)));
    layout.overflow_x = OverflowSpec::Hidden;
    layout.overflow_y = OverflowSpec::Hidden;
    style
}

fn split_direction(axis: DockAxis) -> FlexDirection {
    match axis {
        DockAxis::Horizontal => FlexDirection::Row,
        DockAxis::Vertical => FlexDirection::Column,
    }
}

fn project_chrome(
    dock: &Dock,
    chrome: &DockChrome,
    node: &DockNode,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    match (chrome, node) {
        (
            DockChrome::Item {
                frame,
                title,
                overlay,
            },
            DockNode::Item { id, content },
        ) => {
            let label = dock.title_for(id);
            DockTitle {
                label: Arc::clone(&label),
            }
            .project(*title, world, mutations);
            DockItemFrame { id: Arc::clone(id) }.project(*frame, world, mutations);
            project_content_slots(node, *content, *frame, world, mutations);
            project_overlay(*overlay, dock.node_drop(node), world, mutations);
        }
        (
            DockChrome::Tabs {
                frame,
                strip,
                overlay,
            },
            DockNode::Tabs {
                tabs,
                active,
                contents,
            },
        ) => {
            DockTabsFrame.project(*frame, world, mutations);
            if let Some(style) = world.node_style(*strip).cloned() {
                let mut tabs_view = Tabs::new(effective_active(tabs, active)).fill(true);
                tabs_view.style = style;
                tabs_view.project(*strip, world, mutations);
            }
            project_tab_bodies(tabs, active, contents, *frame, world, mutations);
            project_overlay(*overlay, dock.node_drop(node), world, mutations);
        }
        (
            DockChrome::Split {
                frame,
                handle,
                indicator,
                first,
                second,
            },
            DockNode::Split {
                axis,
                ratio,
                first: first_node,
                second: second_node,
            },
        ) => {
            let ratio = clamp_ratio(*ratio);
            DockSplitFrame { axis: *axis }.project(*frame, world, mutations);
            DockHandle {
                axis: *axis,
                locked: dock.locked,
            }
            .project(*handle, world, mutations);
            DockHandleMark { axis: *axis }.project(*indicator, world, mutations);
            project_chrome(dock, first, first_node, world, mutations);
            project_chrome(dock, second, second_node, world, mutations);
            apply_split_grow(first.frame(), ratio, world, mutations);
            apply_split_grow(second.frame(), 1.0 - ratio, world, mutations);
        }
        _ => project_content_slots(node, None, chrome.frame(), world, mutations),
    }
}

fn project_content_slots(
    node: &DockNode,
    single: Option<StableNodeId>,
    parent: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    match node {
        DockNode::Item { content, .. } => {
            if let Some(content) = single.or(*content) {
                apply_body_slot(content, false, parent, world, mutations);
            }
        }
        DockNode::Tabs {
            tabs,
            active,
            contents,
        } => project_tab_bodies(tabs, active, contents, parent, world, mutations),
        DockNode::Split { first, second, .. } => {
            let mut slots = Vec::new();
            collect_contents(first, &mut slots);
            collect_contents(second, &mut slots);
            for (_, content) in slots {
                if let Some(content) = content {
                    apply_body_slot(content, false, parent, world, mutations);
                }
            }
        }
    }
}

fn project_tab_bodies(
    tabs: &[Arc<str>],
    active: &Arc<str>,
    contents: &[(Arc<str>, Option<StableNodeId>)],
    parent: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    let active = effective_active(tabs, active);
    for (id, content) in contents {
        if let Some(content) = *content {
            apply_body_slot(
                content,
                id.as_ref() != active.as_ref(),
                parent,
                world,
                mutations,
            );
        }
    }
}

fn apply_body_slot(
    id: StableNodeId,
    hidden: bool,
    parent: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    if !world.contains(id) {
        return;
    }
    if world
        .node(id)
        .is_some_and(|node| node.parent != Some(parent))
    {
        mutations.insert(parent, id, None);
    }
    let mut style = world.node_style(id).cloned().unwrap_or_default();
    let layout = Arc::make_mut(&mut style.layout);
    let mut changed = layout.hidden != hidden;
    layout.hidden = hidden;
    if layout.flex_grow.is_none() {
        layout.flex_grow = Some(1.0);
        changed = true;
    }
    if layout.flex_shrink.is_none() {
        layout.flex_shrink = Some(1.0);
        changed = true;
    }
    if layout.width.is_none() {
        layout.width = Some(LengthSpec::Fill);
        changed = true;
    }
    if layout.height.is_none() {
        layout.height = Some(LengthSpec::Fill);
        changed = true;
    }
    if layout.min_width.is_none() {
        layout.min_width = Some(LengthSpec::Px(0.0));
        changed = true;
    }
    if layout.min_height.is_none() {
        layout.min_height = Some(LengthSpec::Px(0.0));
        changed = true;
    }
    if changed {
        mutations.set_style(id, style);
    }
}

fn apply_split_grow(id: StableNodeId, grow: f32, world: &UiWorld, mutations: &mut MutationQueue) {
    let Some(current) = world.node_style(id) else {
        return;
    };
    if current.layout.flex_grow == Some(grow) {
        return;
    }
    let mut style = current.clone();
    Arc::make_mut(&mut style.layout).flex_grow = Some(grow);
    mutations.set_style(id, style);
}

fn project_overlay(
    overlay: Option<StableNodeId>,
    zone: Option<DockDropZone>,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    let Some(id) = overlay else {
        return;
    };
    if !world.contains(id) {
        return;
    }
    if let Some(zone) = zone {
        DockDropOverlay {
            zone,
            visible: true,
        }
        .project(id, world, mutations);
    } else if let Some(current) = world.node_style(id) {
        if !current.layout.hidden {
            let mut style = current.clone();
            Arc::make_mut(&mut style.layout).hidden = true;
            mutations.set_style(id, style);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DockItemFrame {
    id: Arc<str>,
}

impl ComponentView for DockItemFrame {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "dock-item".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &item_frame_style(
                world
                    .node_style(id)
                    .and_then(|style| style.layout.flex_grow),
            ),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::clone(&self.id)),
                ..AccessibilityState::default()
            },
        );
    }
}

fn item_frame_style(grow: Option<f32>) -> NodeStyle {
    let mut style = fill_surface(NodeStyle::default());
    let layout = Arc::make_mut(&mut style.layout);
    layout.direction = Some(FlexDirection::Column);
    layout.align_items = AlignSpec::Stretch;
    layout.position = PositionSpec::Relative;
    layout.flex_grow = grow;
    layout.flex_shrink = Some(1.0);
    style
}

#[derive(Debug, Clone, PartialEq)]
struct DockTitle {
    label: Arc<str>,
}

impl ComponentView for DockTitle {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "dock-title".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_ref()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.to_string(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &title_style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::clone(&self.label)),
                ..AccessibilityState::default()
            },
        );
    }
}

fn title_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    style.background = Some(SemanticColorRole::Surface);
    style.foreground = Some(SemanticColorRole::Muted);
    style.text_vertical_alignment = TextVerticalAlignment::Center;
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Px(DOCK_TITLE_BAR_HEIGHT));
    layout.min_height = Some(LengthSpec::Px(DOCK_TITLE_BAR_HEIGHT));
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    layout.padding_left = Some(LengthSpec::Px(TITLE_PADDING_X));
    layout.padding_right = Some(LengthSpec::Px(TITLE_PADDING_X));
    layout.font_size = Some(TITLE_SIZE);
    layout.font_weight = Some(TITLE_WEIGHT);
    layout.white_space_nowrap = true;
    layout.text_overflow_ellipsis = true;
    style
}

#[derive(Debug, Clone, PartialEq)]
struct DockTabsFrame;

impl ComponentView for DockTabsFrame {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "dock-tabs".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut style = fill_surface(NodeStyle::default());
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.position = PositionSpec::Relative;
        layout.flex_grow = world
            .node_style(id)
            .and_then(|style| style.layout.flex_grow);
        layout.flex_shrink = Some(1.0);
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
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DockSplitFrame {
    axis: DockAxis,
}

impl ComponentView for DockSplitFrame {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "dock-split".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &split_frame_style(
                self.axis,
                world
                    .node_style(id)
                    .and_then(|style| style.layout.flex_grow),
            ),
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

fn split_frame_style(axis: DockAxis, grow: Option<f32>) -> NodeStyle {
    let mut style = fill_surface(NodeStyle::default());
    let layout = Arc::make_mut(&mut style.layout);
    layout.direction = Some(split_direction(axis));
    layout.align_items = AlignSpec::Stretch;
    layout.flex_grow = grow;
    layout.flex_shrink = Some(1.0);
    style
}

#[derive(Debug, Clone, PartialEq)]
struct DockHandle {
    axis: DockAxis,
    locked: bool,
}

impl ComponentView for DockHandle {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "dock-handle".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &handle_style(self.axis),
            handle_interaction(self.locked),
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::from("Resize")),
                ..AccessibilityState::default()
            },
        );
    }
}

fn handle_style(axis: DockAxis) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.interaction = InteractionStyle {
        focused: SemanticPaint {
            border: Some(SemanticColorRole::Accent),
            ..SemanticPaint::default()
        },
        hovered: SemanticPaint {
            background: Some(SemanticColorRole::Hover),
            ..SemanticPaint::default()
        },
        ..InteractionStyle::default()
    };
    let layout = Arc::make_mut(&mut style.layout);
    layout.direction = Some(split_direction(axis));
    layout.align_items = AlignSpec::Center;
    layout.justify_content = JustifySpec::Center;
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    match axis {
        DockAxis::Horizontal => {
            layout.width = Some(LengthSpec::Px(DOCK_DIVIDER_HIT_SIZE));
            layout.min_width = Some(LengthSpec::Px(DOCK_DIVIDER_HIT_SIZE));
            layout.height = Some(LengthSpec::Fill);
        }
        DockAxis::Vertical => {
            layout.height = Some(LengthSpec::Px(DOCK_DIVIDER_HIT_SIZE));
            layout.min_height = Some(LengthSpec::Px(DOCK_DIVIDER_HIT_SIZE));
            layout.width = Some(LengthSpec::Fill);
        }
    }
    style
}

fn handle_interaction(locked: bool) -> InteractionState {
    InteractionState {
        pointer_events: !locked,
        focusable: !locked,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DockHandleMark {
    axis: DockAxis,
}

impl ComponentView for DockHandleMark {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "dock-handle-mark".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &handle_mark_style(self.axis),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState::default(),
        );
    }
}

fn handle_mark_style(axis: DockAxis) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.background = Some(SemanticColorRole::BorderStrong);
    let layout = Arc::make_mut(&mut style.layout);
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    match axis {
        DockAxis::Horizontal => {
            layout.width = Some(LengthSpec::Px(HANDLE_INDICATOR));
            layout.height = Some(LengthSpec::Fill);
        }
        DockAxis::Vertical => {
            layout.height = Some(LengthSpec::Px(HANDLE_INDICATOR));
            layout.width = Some(LengthSpec::Fill);
        }
    }
    style
}

#[derive(Debug, Clone, PartialEq)]
struct DockDropOverlay {
    zone: DockDropZone,
    visible: bool,
}

impl ComponentView for DockDropOverlay {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "dock-drop-overlay".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &overlay_style(self.zone, self.visible),
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

fn overlay_style(zone: DockDropZone, visible: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.background = Some(SemanticColorRole::AccentSoft);
    let layout = Arc::make_mut(&mut style.layout);
    layout.position = PositionSpec::Absolute;
    layout.z_index = Some(1);
    layout.hidden = !visible;
    match zone {
        DockDropZone::Left => {
            layout.offset_left = Some(LengthSpec::Px(0.0));
            layout.offset_top = Some(LengthSpec::Px(0.0));
            layout.offset_bottom = Some(LengthSpec::Px(0.0));
            layout.width = Some(LengthSpec::Percent(50.0));
            layout.height = Some(LengthSpec::Fill);
        }
        DockDropZone::Right => {
            layout.offset_right = Some(LengthSpec::Px(0.0));
            layout.offset_top = Some(LengthSpec::Px(0.0));
            layout.offset_bottom = Some(LengthSpec::Px(0.0));
            layout.width = Some(LengthSpec::Percent(50.0));
            layout.height = Some(LengthSpec::Fill);
        }
        DockDropZone::Top => {
            layout.offset_left = Some(LengthSpec::Px(0.0));
            layout.offset_right = Some(LengthSpec::Px(0.0));
            layout.offset_top = Some(LengthSpec::Px(0.0));
            layout.width = Some(LengthSpec::Fill);
            layout.height = Some(LengthSpec::Percent(50.0));
        }
        DockDropZone::Bottom => {
            layout.offset_left = Some(LengthSpec::Px(0.0));
            layout.offset_right = Some(LengthSpec::Px(0.0));
            layout.offset_bottom = Some(LengthSpec::Px(0.0));
            layout.width = Some(LengthSpec::Fill);
            layout.height = Some(LengthSpec::Percent(50.0));
        }
        DockDropZone::Tab => {
            layout.offset_left = Some(LengthSpec::Px(0.0));
            layout.offset_right = Some(LengthSpec::Px(0.0));
            layout.offset_top = Some(LengthSpec::Px(0.0));
            layout.width = Some(LengthSpec::Fill);
            layout.height = Some(LengthSpec::Px(TAB_OVERLAY_THICKNESS));
        }
    }
    style
}

/// Flat bordered inspector / dock content surface.
#[derive(Debug, Clone, PartialEq)]
pub struct DockPanel {
    pub content: Option<StableNodeId>,
    pub padding: f32,
    pub style: NodeStyle,
}

impl DockPanel {
    pub fn new() -> Self {
        Self {
            content: None,
            padding: 0.0,
            style: NodeStyle::default(),
        }
    }

    pub fn content(mut self, content: StableNodeId) -> Self {
        self.content = Some(content);
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding.max(0.0);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.background = Some(SemanticColorRole::Surface);
        style.foreground = Some(SemanticColorRole::Text);
        style.border = Some(SemanticColorRole::BorderSoft);
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(layout.height.unwrap_or(LengthSpec::Shrink));
        layout.border_width = Some(1.0);
        layout.border_radius = Some(0.0);
        let pad = LengthSpec::Px(self.padding);
        layout.padding_left = Some(pad);
        layout.padding_right = Some(pad);
        layout.padding_top = Some(pad);
        layout.padding_bottom = Some(pad);
        style
    }
}

impl Default for DockPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for DockPanel {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "dock-panel".into(),
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
        if let Some(content) = self.content {
            apply_body_slot(content, false, id, world, mutations);
        }
    }
}

impl AppContext {
    /// Mount retained chrome for `dock`. Host content slots are reparented, not recreated.
    pub fn assemble_dock(&mut self, dock: Entity<Dock>) -> Result<bool, FrameworkError> {
        let document = document_of(self, dock.stable_id())?;
        let snapshot = self.read(dock, |dock| {
            (
                dock.root.clone(),
                dock.drop_target.clone(),
                dock.locked,
                dock.titles.clone(),
                dock.chrome.clone(),
            )
        })?;
        let (mut root, drop_target, locked, titles, old) = snapshot;
        root.clamp_ratios();
        let hosts = host_content_ids(&root);
        if let Some(old) = old.as_ref() {
            park_hosts(self, old, &hosts)?;
        }
        let chrome = assemble_branch(
            self,
            document,
            dock.stable_id(),
            &root,
            old,
            &drop_target,
            locked,
            &titles,
            &hosts,
            true,
        )?;
        let children = branch_children(&chrome, &root);
        self.update_component(dock, |dock, _| {
            dock.root = root.clone();
            dock.chrome = Some(chrome.clone());
        })?;
        reconcile_children(self, dock, &children)
    }
}

fn document_of(context: &AppContext, id: StableNodeId) -> Result<DocumentId, FrameworkError> {
    context
        .world()
        .node(id)
        .map(|node| node.document)
        .ok_or(FrameworkError::MissingView(id))
}

fn assemble_branch(
    context: &mut AppContext,
    document: DocumentId,
    root_id: StableNodeId,
    node: &DockNode,
    old: Option<DockChrome>,
    drop_target: &Option<(Arc<str>, DockDropZone)>,
    locked: bool,
    titles: &[(Arc<str>, Arc<str>)],
    hosts: &HashSet<StableNodeId>,
    at_root: bool,
) -> Result<DockChrome, FrameworkError> {
    match node {
        DockNode::Item { id, content } => assemble_item(
            context,
            document,
            root_id,
            id,
            *content,
            old,
            drop_zone_for(drop_target, std::slice::from_ref(id)),
            titles,
            hosts,
            at_root,
        ),
        DockNode::Tabs {
            tabs,
            active,
            contents,
        } => assemble_tabs(
            context,
            document,
            root_id,
            tabs,
            active,
            contents,
            old,
            drop_zone_for(drop_target, tabs),
            titles,
            hosts,
            at_root,
        ),
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => assemble_split(
            context,
            document,
            root_id,
            *axis,
            *ratio,
            first,
            second,
            old,
            drop_target,
            locked,
            titles,
            hosts,
            at_root,
        ),
    }
}

fn drop_zone_for(
    drop_target: &Option<(Arc<str>, DockDropZone)>,
    ids: &[Arc<str>],
) -> Option<DockDropZone> {
    drop_target
        .as_ref()
        .filter(|(id, _)| ids.iter().any(|item| item == id))
        .map(|(_, zone)| *zone)
}

fn title_lookup(titles: &[(Arc<str>, Arc<str>)], id: &str) -> Arc<str> {
    titles
        .iter()
        .find(|(key, _)| key.as_ref() == id)
        .map(|(_, title)| Arc::clone(title))
        .unwrap_or_else(|| Arc::from(id))
}

fn assemble_item(
    context: &mut AppContext,
    document: DocumentId,
    root_id: StableNodeId,
    id: &Arc<str>,
    content: Option<StableNodeId>,
    old: Option<DockChrome>,
    zone: Option<DockDropZone>,
    titles: &[(Arc<str>, Arc<str>)],
    hosts: &HashSet<StableNodeId>,
    at_root: bool,
) -> Result<DockChrome, FrameworkError> {
    let reused = match old {
        Some(DockChrome::Item {
            frame,
            title,
            overlay,
        }) => Some((frame, title, overlay)),
        Some(other) => {
            dispose_chrome(context, other, root_id, hosts)?;
            None
        }
        None => None,
    };
    let frame = if let Some((frame, _, _)) = reused {
        if frame != root_id {
            context.update_component(
                Entity::<DockItemFrame>::from_stable_id(frame),
                |frame, _| {
                    frame.id = Arc::clone(id);
                },
            )?;
        }
        frame
    } else if at_root {
        root_id
    } else {
        context
            .create_detached_component(document, DockItemFrame { id: Arc::clone(id) })?
            .stable_id()
    };
    let label = title_lookup(titles, id);
    let title = if let Some((_, title, _)) = reused {
        let entity = Entity::<DockTitle>::from_stable_id(title);
        context.update_component(entity, |title, _| {
            title.label = Arc::clone(&label);
        })?;
        title
    } else {
        context
            .create_detached_component(document, DockTitle { label })?
            .stable_id()
    };
    let overlay = ensure_overlay(
        context,
        document,
        reused.and_then(|(_, _, overlay)| overlay),
        zone,
    )?;
    let mut ordered = vec![title];
    if let Some(content) = content {
        ordered.push(content);
    }
    if let Some(overlay) = overlay {
        ordered.push(overlay);
    }
    reconcile_ids(context, frame, &ordered)?;
    Ok(DockChrome::Item {
        frame,
        title,
        overlay,
    })
}

fn assemble_tabs(
    context: &mut AppContext,
    document: DocumentId,
    root_id: StableNodeId,
    tabs: &[Arc<str>],
    active: &Arc<str>,
    contents: &[(Arc<str>, Option<StableNodeId>)],
    old: Option<DockChrome>,
    zone: Option<DockDropZone>,
    titles: &[(Arc<str>, Arc<str>)],
    hosts: &HashSet<StableNodeId>,
    at_root: bool,
) -> Result<DockChrome, FrameworkError> {
    let reused = match old {
        Some(DockChrome::Tabs {
            frame,
            strip,
            overlay,
        }) => Some((frame, strip, overlay)),
        Some(other) => {
            dispose_chrome(context, other, root_id, hosts)?;
            None
        }
        None => None,
    };
    let frame = if let Some((frame, _, _)) = reused {
        frame
    } else if at_root {
        root_id
    } else {
        context
            .create_detached_component(document, DockTabsFrame)?
            .stable_id()
    };
    if frame != root_id && reused.is_some() {
        context.update_component(Entity::<DockTabsFrame>::from_stable_id(frame), |_, _| {})?;
    }
    let active = effective_active(tabs, active);
    let options = tabs
        .iter()
        .map(|id| TabOption::new(Arc::clone(id), title_lookup(titles, id)))
        .collect::<Vec<_>>();
    let strip = if let Some((_, strip, _)) = reused {
        context.update_component(Entity::<Tabs>::from_stable_id(strip), |strip, _| {
            strip.options = options.clone();
            strip.selected = Some(Arc::clone(&active));
            strip.focus = Some(Arc::clone(&active));
            strip.fill = true;
            strip.size = ControlSize::Small;
        })?;
        strip
    } else {
        context
            .create_detached_component(
                document,
                Tabs::new(Arc::clone(&active))
                    .options(options)
                    .fill(true)
                    .size(ControlSize::Small),
            )?
            .stable_id()
    };
    let overlay = ensure_overlay(
        context,
        document,
        reused.and_then(|(_, _, overlay)| overlay),
        zone,
    )?;
    let mut ordered = vec![strip];
    for (_, content) in contents {
        if let Some(content) = *content {
            ordered.push(content);
        }
    }
    if let Some(overlay) = overlay {
        ordered.push(overlay);
    }
    reconcile_ids(context, frame, &ordered)?;
    Ok(DockChrome::Tabs {
        frame,
        strip,
        overlay,
    })
}

fn assemble_split(
    context: &mut AppContext,
    document: DocumentId,
    root_id: StableNodeId,
    axis: DockAxis,
    ratio: f32,
    first: &DockNode,
    second: &DockNode,
    old: Option<DockChrome>,
    drop_target: &Option<(Arc<str>, DockDropZone)>,
    locked: bool,
    titles: &[(Arc<str>, Arc<str>)],
    hosts: &HashSet<StableNodeId>,
    at_root: bool,
) -> Result<DockChrome, FrameworkError> {
    let reused = match old {
        Some(DockChrome::Split {
            frame,
            handle,
            indicator,
            first,
            second,
        }) => Some((frame, handle, indicator, first, second)),
        Some(other) => {
            dispose_chrome(context, other, root_id, hosts)?;
            None
        }
        None => None,
    };
    let frame = if let Some((frame, _, _, _, _)) = reused {
        if frame != root_id {
            context.update_component(
                Entity::<DockSplitFrame>::from_stable_id(frame),
                |split, _| {
                    split.axis = axis;
                },
            )?;
        }
        frame
    } else if at_root {
        root_id
    } else {
        context
            .create_detached_component(document, DockSplitFrame { axis })?
            .stable_id()
    };
    let handle = if let Some((_, handle, _, _, _)) = reused {
        context.update_component(Entity::<DockHandle>::from_stable_id(handle), |handle, _| {
            handle.axis = axis;
            handle.locked = locked;
        })?;
        handle
    } else {
        context
            .create_detached_component(document, DockHandle { axis, locked })?
            .stable_id()
    };
    let indicator = if let Some((_, _, indicator, _, _)) = reused {
        context.update_component(
            Entity::<DockHandleMark>::from_stable_id(indicator),
            |mark, _| {
                mark.axis = axis;
            },
        )?;
        indicator
    } else {
        context
            .create_detached_component(document, DockHandleMark { axis })?
            .stable_id()
    };
    reconcile_ids(context, handle, &[indicator])?;
    let first_chrome = assemble_branch(
        context,
        document,
        root_id,
        first,
        reused.as_ref().map(|chrome| (*chrome.3).clone()),
        drop_target,
        locked,
        titles,
        hosts,
        false,
    )?;
    let second_chrome = assemble_branch(
        context,
        document,
        root_id,
        second,
        reused.as_ref().map(|chrome| (*chrome.4).clone()),
        drop_target,
        locked,
        titles,
        hosts,
        false,
    )?;
    apply_grow_now(context, first_chrome.frame(), clamp_ratio(ratio))?;
    apply_grow_now(context, second_chrome.frame(), 1.0 - clamp_ratio(ratio))?;
    reconcile_ids(
        context,
        frame,
        &[first_chrome.frame(), handle, second_chrome.frame()],
    )?;
    Ok(DockChrome::Split {
        frame,
        handle,
        indicator,
        first: Box::new(first_chrome),
        second: Box::new(second_chrome),
    })
}

fn apply_grow_now(
    context: &mut AppContext,
    id: StableNodeId,
    grow: f32,
) -> Result<(), FrameworkError> {
    let Some(current) = context.world().node_style(id).cloned() else {
        return Ok(());
    };
    if current.layout.flex_grow == Some(grow) {
        return Ok(());
    }
    let mut style = current;
    Arc::make_mut(&mut style.layout).flex_grow = Some(grow);
    let mut mutations = MutationQueue::new();
    mutations.set_style(id, style);
    context.commit_mutations(mutations)?;
    Ok(())
}

fn ensure_overlay(
    context: &mut AppContext,
    document: DocumentId,
    existing: Option<StableNodeId>,
    zone: Option<DockDropZone>,
) -> Result<Option<StableNodeId>, FrameworkError> {
    match (existing, zone) {
        (Some(id), Some(zone)) => {
            context.update_component(
                Entity::<DockDropOverlay>::from_stable_id(id),
                |overlay, _| {
                    overlay.zone = zone;
                    overlay.visible = true;
                },
            )?;
            Ok(Some(id))
        }
        (Some(id), None) => {
            context.update_component(
                Entity::<DockDropOverlay>::from_stable_id(id),
                |overlay, _| {
                    overlay.visible = false;
                },
            )?;
            Ok(Some(id))
        }
        (None, Some(zone)) => Ok(Some(
            context
                .create_detached_component(
                    document,
                    DockDropOverlay {
                        zone,
                        visible: true,
                    },
                )?
                .stable_id(),
        )),
        (None, None) => Ok(None),
    }
}

fn branch_children(chrome: &DockChrome, node: &DockNode) -> Vec<StableNodeId> {
    match (chrome, node) {
        (DockChrome::Item { title, overlay, .. }, DockNode::Item { content, .. }) => {
            let mut children = vec![*title];
            if let Some(content) = content {
                children.push(*content);
            }
            if let Some(overlay) = overlay {
                children.push(*overlay);
            }
            children
        }
        (DockChrome::Tabs { strip, overlay, .. }, DockNode::Tabs { contents, .. }) => {
            let mut children = vec![*strip];
            for (_, content) in contents {
                if let Some(content) = content {
                    children.push(*content);
                }
            }
            if let Some(overlay) = overlay {
                children.push(*overlay);
            }
            children
        }
        (
            DockChrome::Split {
                handle,
                first,
                second,
                ..
            },
            _,
        ) => {
            vec![first.frame(), *handle, second.frame()]
        }
        _ => Vec::new(),
    }
}

fn park_hosts(
    context: &mut AppContext,
    chrome: &DockChrome,
    hosts: &HashSet<StableNodeId>,
) -> Result<(), FrameworkError> {
    let mut mutations = MutationQueue::new();
    collect_host_parks(context, chrome.frame(), hosts, &mut mutations);
    if !mutations.is_empty() {
        context.commit_mutations(mutations)?;
    }
    Ok(())
}

fn collect_host_parks(
    context: &AppContext,
    id: StableNodeId,
    hosts: &HashSet<StableNodeId>,
    mutations: &mut MutationQueue,
) {
    let Some(node) = context.world().node(id) else {
        return;
    };
    for child in node.children {
        if hosts.contains(&child) {
            mutations.park_subtree(child);
        } else {
            collect_host_parks(context, child, hosts, mutations);
        }
    }
}

fn dispose_chrome(
    context: &mut AppContext,
    chrome: DockChrome,
    root_id: StableNodeId,
    hosts: &HashSet<StableNodeId>,
) -> Result<(), FrameworkError> {
    park_hosts(context, &chrome, hosts)?;
    match chrome {
        DockChrome::Item {
            frame,
            title,
            overlay,
        } => {
            remove_if_present::<DockTitle>(context, title)?;
            if let Some(overlay) = overlay {
                remove_if_present::<DockDropOverlay>(context, overlay)?;
            }
            if frame != root_id {
                remove_if_present::<DockItemFrame>(context, frame)?;
            }
        }
        DockChrome::Tabs {
            frame,
            strip,
            overlay,
        } => {
            remove_if_present::<Tabs>(context, strip)?;
            if let Some(overlay) = overlay {
                remove_if_present::<DockDropOverlay>(context, overlay)?;
            }
            if frame != root_id {
                remove_if_present::<DockTabsFrame>(context, frame)?;
            }
        }
        DockChrome::Split {
            frame,
            handle,
            indicator,
            first,
            second,
        } => {
            dispose_chrome(context, *first, root_id, hosts)?;
            dispose_chrome(context, *second, root_id, hosts)?;
            remove_if_present::<DockHandleMark>(context, indicator)?;
            remove_if_present::<DockHandle>(context, handle)?;
            if frame != root_id {
                remove_if_present::<DockSplitFrame>(context, frame)?;
            }
        }
    }
    Ok(())
}

fn remove_if_present<C: ComponentView>(
    context: &mut AppContext,
    id: StableNodeId,
) -> Result<(), FrameworkError> {
    if context.world().contains(id) {
        let _ = context.remove_view(Entity::<C>::from_stable_id(id))?;
    }
    Ok(())
}

fn reconcile_children<C: ComponentView>(
    context: &mut AppContext,
    parent: Entity<C>,
    ordered: &[StableNodeId],
) -> Result<bool, FrameworkError> {
    reconcile_ids(context, parent.stable_id(), ordered)
}

fn reconcile_ids(
    context: &mut AppContext,
    parent: StableNodeId,
    ordered: &[StableNodeId],
) -> Result<bool, FrameworkError> {
    let current = context
        .world()
        .node(parent)
        .ok_or(FrameworkError::MissingView(parent))?
        .children
        .clone();
    if current.as_slice() == ordered {
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
        mutations.insert(parent, *child, None);
    }
    context.commit_mutations(mutations)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentId, Text};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn body(context: &mut AppContext, label: &str) -> StableNodeId {
        context
            .create_component(document(), Text::new(label))
            .unwrap()
            .stable_id()
    }

    fn descendants(context: &AppContext, root: StableNodeId) -> Vec<StableNodeId> {
        let mut ids = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            ids.push(id);
            if let Some(node) = context.world().node(id) {
                stack.extend(node.children.iter().rev().copied());
            }
        }
        ids
    }

    fn find_tag(context: &AppContext, root: StableNodeId, tag: &str) -> Vec<StableNodeId> {
        descendants(context, root)
            .into_iter()
            .filter(|id| {
                context.world().node(*id).is_some_and(|node| {
                    matches!(
                        &node.kind,
                        NodeKind::Element { tag: current } if current == tag
                    )
                })
            })
            .collect()
    }

    fn visible_overlay(context: &AppContext, root: StableNodeId) -> Option<StableNodeId> {
        find_tag(context, root, "dock-drop-overlay")
            .into_iter()
            .find(|id| {
                context
                    .world()
                    .node_style(*id)
                    .is_some_and(|style| !style.layout.hidden)
            })
    }

    #[test]
    fn dock_split_ratio_clamps_and_preserves_child_order() {
        let low = DockNode::split(
            DockAxis::Horizontal,
            -2.0,
            DockNode::item("left", None),
            DockNode::item("right", None),
        );
        assert_eq!(low.split_ratio(), Some(MIN_SPLIT_RATIO));
        assert_eq!(
            low.flatten()
                .iter()
                .map(|id| id.as_ref())
                .collect::<Vec<_>>(),
            ["left", "right"]
        );

        let high = DockNode::split(
            DockAxis::Vertical,
            4.0,
            DockNode::item("top", None),
            DockNode::item("bottom", None),
        );
        assert_eq!(high.split_ratio(), Some(MAX_SPLIT_RATIO));

        let nan = DockNode::split(
            DockAxis::Horizontal,
            f32::NAN,
            DockNode::item("a", None),
            DockNode::item("b", None),
        );
        assert_eq!(nan.split_ratio(), Some(0.5));

        let mut context = AppContext::new();
        let first = body(&mut context, "first");
        let second = body(&mut context, "second");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    1.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let chrome = context
            .read(dock, |dock| dock.chrome.clone())
            .unwrap()
            .unwrap();
        let (first_frame, handle, second_frame) = chrome.split_children().unwrap();
        assert_eq!(
            context.world().node(dock.stable_id()).unwrap().children,
            [first_frame, handle, second_frame]
        );
        assert_eq!(
            context
                .world()
                .node_style(first_frame)
                .unwrap()
                .layout
                .flex_grow,
            Some(MAX_SPLIT_RATIO)
        );
        assert_eq!(
            context
                .world()
                .node_style(second_frame)
                .unwrap()
                .layout
                .flex_grow,
            Some(1.0 - MAX_SPLIT_RATIO)
        );
        assert_eq!(
            context
                .world()
                .node_style(dock.stable_id())
                .unwrap()
                .layout
                .direction,
            Some(FlexDirection::Row)
        );
    }

    #[test]
    fn dock_tabs_hide_inactive_body() {
        let mut context = AppContext::new();
        let code = body(&mut context, "code");
        let preview = body(&mut context, "preview");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::tabs(
                    ["code", "preview"],
                    "code",
                    [("code", Some(code)), ("preview", Some(preview))],
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        assert!(!context.world().node_style(code).unwrap().layout.hidden);
        assert!(context.world().node_style(preview).unwrap().layout.hidden);
        assert_eq!(
            context.world().node(code).unwrap().parent,
            Some(dock.stable_id())
        );
        assert_eq!(
            context.world().node(preview).unwrap().parent,
            Some(dock.stable_id())
        );

        context
            .update_component(dock, |dock, _| {
                if let DockNode::Tabs { active, .. } = &mut dock.root {
                    *active = Arc::from("preview");
                }
            })
            .unwrap();
        assert!(context.world().node_style(code).unwrap().layout.hidden);
        assert!(!context.world().node_style(preview).unwrap().layout.hidden);
        assert!(context.world().contains(code));
        assert!(context.world().contains(preview));
    }

    #[test]
    fn dock_item_projects_content_slot() {
        let mut context = AppContext::new();
        let content = body(&mut context, "inspector-body");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::item("inspector", Some(content)))
                    .title("inspector", "Inspector"),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let title = find_tag(&context, dock.stable_id(), "dock-title")[0];
        assert_eq!(context.world().text(title), Some("Inspector"));
        assert_eq!(
            context.world().node(content).unwrap().parent,
            Some(dock.stable_id())
        );
        assert_eq!(
            context.world().node(dock.stable_id()).unwrap().children[0],
            title
        );
        assert!(
            context
                .world()
                .node(dock.stable_id())
                .unwrap()
                .children
                .contains(&content)
        );
        let style = context.world().node_style(dock.stable_id()).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Surface));
        assert_eq!(style.layout.height, Some(LengthSpec::Fill));
        assert_eq!(style.layout.overflow_y, OverflowSpec::Hidden);
        assert_eq!(
            context
                .world()
                .accessibility(dock.stable_id())
                .unwrap()
                .label
                .as_deref(),
            Some("inspector")
        );
    }

    #[test]
    fn dock_drop_overlay_present_only_when_drop_target_set() {
        let mut context = AppContext::new();
        let content = body(&mut context, "pane");
        let dock = context
            .create_component(document(), Dock::new(DockNode::item("pane", Some(content))))
            .unwrap();
        context.assemble_dock(dock).unwrap();
        assert!(visible_overlay(&context, dock.stable_id()).is_none());

        context
            .update_component(dock, |dock, _| {
                dock.drop_target = Some((Arc::from("pane"), DockDropZone::Left));
            })
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let overlay = visible_overlay(&context, dock.stable_id()).expect("overlay");
        let style = context.world().node_style(overlay).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::AccentSoft));
        assert_eq!(style.layout.width, Some(LengthSpec::Percent(50.0)));
        assert_eq!(style.layout.position, PositionSpec::Absolute);
        assert_eq!(
            context.world().node(overlay).unwrap().parent,
            Some(dock.stable_id())
        );
        assert!(
            context
                .world()
                .node(dock.stable_id())
                .unwrap()
                .children
                .contains(&content)
        );

        context
            .update_component(dock, |dock, _| {
                dock.drop_target = None;
            })
            .unwrap();
        assert!(visible_overlay(&context, dock.stable_id()).is_none());
    }

    #[test]
    fn dock_locked_omits_handle_pointer() {
        let mut context = AppContext::new();
        let first = body(&mut context, "a");
        let second = body(&mut context, "b");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::split(
                    DockAxis::Vertical,
                    0.4,
                    DockNode::item("a", Some(first)),
                    DockNode::item("b", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let handle = find_tag(&context, dock.stable_id(), "dock-handle")[0];
        assert_eq!(
            context.world().interaction(handle),
            Some(InteractionState {
                pointer_events: true,
                focusable: true,
            })
        );

        context
            .update_component(dock, |dock, _| {
                dock.locked = true;
            })
            .unwrap();
        assert_eq!(
            context.world().interaction(handle),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        let mark = find_tag(&context, dock.stable_id(), "dock-handle-mark")[0];
        assert_eq!(
            context.world().node_style(mark).unwrap().background,
            Some(SemanticColorRole::BorderStrong)
        );
    }

    #[test]
    fn dock_panel_border_and_child_slot() {
        let mut context = AppContext::new();
        let child = body(&mut context, "fields");
        let panel = context
            .create_component(document(), DockPanel::new().padding(8.0).content(child))
            .unwrap();
        let style = context.world().node_style(panel.stable_id()).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Surface));
        assert_eq!(style.foreground, Some(SemanticColorRole::Text));
        assert_eq!(style.border, Some(SemanticColorRole::BorderSoft));
        assert_eq!(style.layout.border_width, Some(1.0));
        assert_eq!(style.layout.border_radius, Some(0.0));
        assert_eq!(style.layout.width, Some(LengthSpec::Fill));
        assert_eq!(style.layout.height, Some(LengthSpec::Shrink));
        assert_eq!(style.layout.padding_left, Some(LengthSpec::Px(8.0)));
        assert_eq!(
            context.world().node(panel.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "dock-panel".into(),
            }
        );
        assert_eq!(
            context.world().node(child).unwrap().parent,
            Some(panel.stable_id())
        );
    }

    #[test]
    fn dock_idle_project_does_not_dirty() {
        let mut context = AppContext::new();
        let content = body(&mut context, "body");
        let dock = context
            .create_component(
                document(),
                Dock::new(DockNode::item("pane", Some(content)))
                    .drop_target("pane", DockDropZone::Tab),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let panel_child = body(&mut context, "panel-body");
        let panel = context
            .create_component(document(), DockPanel::new().content(panel_child))
            .unwrap();
        let _ = context.take_system_work();
        context.update_component(dock, |_, _| {}).unwrap();
        context.update_component(panel, |_, _| {}).unwrap();
        assert!(context.take_system_work().is_empty());
    }
}
