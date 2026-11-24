use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, JustifySpec, LengthSpec, OverflowSpec, PositionSpec,
    SemanticColorRole,
};

use super::DOCK_CAPABILITIES;
use super::model::{
    DEFAULT_FLOATING_HEIGHT, DEFAULT_FLOATING_WIDTH, DEFAULT_FLOATING_X, DEFAULT_FLOATING_Y,
    DOCK_DIVIDER_HIT_SIZE, DockAxis, DockDropZone, DockFloatingSurface, DockNode, can_hide_id,
    clamp_ratio, collect_contents, effective_active, extract_item, filter_hidden, finite, hide_id,
    insert_dock_item, is_hidden, reorder_dock_tab, show_id, visible_branch,
};
use crate::tabs::Tabs;
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, InteractionStyle,
    LayoutBox, MutationQueue, NodeKind, NodeStyle, SemanticPaint, StableNodeId, TextContent,
    TextVerticalAlignment, UiWorld,
};

pub(super) const DOCK_TITLE_BAR_HEIGHT: f32 =
    ControlSize::Small.height_in(nana_ui_core::UI_METRICS);
const HANDLE_INDICATOR: f32 = nana_ui_core::space::XXS;
const TITLE_PADDING_X: f32 = nana_ui_core::space::SM;
const TAB_OVERLAY_THICKNESS: f32 = nana_ui_core::space::XS;

pub(super) fn point_near_box(bounds: LayoutBox, x: f32, y: f32, slop: f32) -> bool {
    x >= bounds.x - slop
        && y >= bounds.y - slop
        && x <= bounds.x + bounds.width + slop
        && y <= bounds.y + bounds.height + slop
}

pub(super) fn chrome_has_handle(chrome: &DockChrome, handle: StableNodeId) -> bool {
    match chrome {
        DockChrome::Split {
            handle: current,
            first,
            second,
            ..
        } => {
            *current == handle
                || chrome_has_handle(first, handle)
                || chrome_has_handle(second, handle)
        }
        DockChrome::Item { .. } | DockChrome::Tabs { .. } => false,
    }
}

/// The first node `pick` answers for, depth first, splits before their panes.
pub(super) fn chrome_find(
    chrome: &DockChrome,
    pick: &mut impl FnMut(&DockChrome) -> Option<StableNodeId>,
) -> Option<StableNodeId> {
    if let Some(found) = pick(chrome) {
        return Some(found);
    }
    match chrome {
        DockChrome::Split { first, second, .. } => {
            chrome_find(first, pick).or_else(|| chrome_find(second, pick))
        }
        DockChrome::Item { .. } | DockChrome::Tabs { .. } => None,
    }
}

pub(super) fn chrome_has_strip(chrome: &DockChrome, strip: StableNodeId) -> bool {
    match chrome {
        DockChrome::Tabs { strip: current, .. } => *current == strip,
        DockChrome::Split { first, second, .. } => {
            chrome_has_strip(first, strip) || chrome_has_strip(second, strip)
        }
        DockChrome::Item { .. } => false,
    }
}

pub(super) fn chrome_has_title(chrome: &DockChrome, title: StableNodeId) -> bool {
    match chrome {
        DockChrome::Item { title: current, .. } => *current == title,
        DockChrome::Split { first, second, .. } => {
            chrome_has_title(first, title) || chrome_has_title(second, title)
        }
        DockChrome::Tabs { .. } => false,
    }
}

pub(super) fn split_info_for_handle(
    chrome: &DockChrome,
    node: &DockNode,
    handle: StableNodeId,
) -> Option<(StableNodeId, DockAxis, f32, Vec<Arc<str>>)> {
    match (chrome, node) {
        (
            DockChrome::Split {
                handle: current,
                frame,
                first,
                second,
                ..
            },
            DockNode::Split {
                axis,
                ratio,
                first: first_node,
                second: second_node,
            },
        ) => {
            if *current == handle {
                return Some((*frame, *axis, *ratio, first_node.flatten()));
            }
            split_info_for_handle(first, first_node, handle)
                .or_else(|| split_info_for_handle(second, second_node, handle))
        }
        _ => None,
    }
}

pub(super) fn title_item_id(
    chrome: &DockChrome,
    node: &DockNode,
    title: StableNodeId,
) -> Option<Arc<str>> {
    match (chrome, node) {
        (DockChrome::Item { title: current, .. }, DockNode::Item { id, .. })
            if *current == title =>
        {
            Some(Arc::clone(id))
        }
        (
            DockChrome::Split { first, second, .. },
            DockNode::Split {
                first: first_node,
                second: second_node,
                ..
            },
        ) => title_item_id(first, first_node, title)
            .or_else(|| title_item_id(second, second_node, title)),
        _ => None,
    }
}

pub(super) fn strip_containing_item(
    chrome: &DockChrome,
    node: &DockNode,
    id: &str,
) -> Option<StableNodeId> {
    match (chrome, node) {
        (DockChrome::Tabs { strip, .. }, DockNode::Tabs { tabs, .. })
            if tabs.iter().any(|tab| tab.as_ref() == id) =>
        {
            Some(*strip)
        }
        (
            DockChrome::Split { first, second, .. },
            DockNode::Split {
                first: first_node,
                second: second_node,
                ..
            },
        ) => strip_containing_item(first, first_node, id)
            .or_else(|| strip_containing_item(second, second_node, id)),
        _ => None,
    }
}

pub(super) fn strip_for_tabs(
    chrome: &DockChrome,
    node: &DockNode,
    strip: StableNodeId,
) -> Option<Arc<str>> {
    match (chrome, node) {
        (DockChrome::Tabs { strip: current, .. }, DockNode::Tabs { active, tabs, .. })
            if *current == strip =>
        {
            Some(effective_active(tabs, active))
        }
        (
            DockChrome::Split { first, second, .. },
            DockNode::Split {
                first: first_node,
                second: second_node,
                ..
            },
        ) => strip_for_tabs(first, first_node, strip)
            .or_else(|| strip_for_tabs(second, second_node, strip)),
        _ => None,
    }
}

pub(super) fn collect_drop_leaves(
    chrome: &DockChrome,
    node: &DockNode,
    output: &mut Vec<(Arc<str>, StableNodeId)>,
) {
    match (chrome, node) {
        (DockChrome::Item { frame, .. }, DockNode::Item { id, .. }) => {
            output.push((Arc::clone(id), *frame));
        }
        (DockChrome::Tabs { frame, .. }, DockNode::Tabs { tabs, .. }) => {
            if let Some(id) = tabs.first() {
                output.push((Arc::clone(id), *frame));
            }
        }
        (
            DockChrome::Split { first, second, .. },
            DockNode::Split {
                first: first_node,
                second: second_node,
                ..
            },
        ) => {
            collect_drop_leaves(first, first_node, output);
            collect_drop_leaves(second, second_node, output);
        }
        _ => {}
    }
}

pub(super) fn drop_zone_at(bounds: LayoutBox, x: f32, y: f32) -> Option<DockDropZone> {
    if bounds.width <= 0.0 || bounds.height <= 0.0 || !bounds.contains(x, y) {
        return None;
    }
    let local_x = (x - bounds.x) / bounds.width;
    let local_y = (y - bounds.y) / bounds.height;
    Some(if local_x <= 0.25 {
        DockDropZone::Left
    } else if local_x >= 0.75 {
        DockDropZone::Right
    } else if local_y <= 0.25 {
        DockDropZone::Top
    } else if local_y >= 0.75 {
        DockDropZone::Bottom
    } else {
        DockDropZone::Tab
    })
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum DockChrome {
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
    pub(super) fn frame(&self) -> StableNodeId {
        match self {
            Self::Item { frame, .. } | Self::Tabs { frame, .. } | Self::Split { frame, .. } => {
                *frame
            }
        }
    }

    #[cfg(test)]
    pub(super) fn split_children(&self) -> Option<(StableNodeId, StableNodeId, StableNodeId)> {
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

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DockSplitResize {
    pub(super) handle: StableNodeId,
    pub(super) frame: StableNodeId,
    pub(super) axis: DockAxis,
    pub(super) start: Option<f32>,
    pub(super) start_ratio: f32,
    pub(super) first_ids: Vec<Arc<str>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DockItemDrag {
    pub(super) id: Arc<str>,
    pub(super) reorder: Option<(Arc<str>, bool)>,
}

/// Recursive split / tabs / item surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Dock {
    pub root: DockNode,
    pub drop_target: Option<(Arc<str>, DockDropZone)>,
    pub locked: bool,
    pub titles: Vec<(Arc<str>, Arc<str>)>,
    pub style: NodeStyle,
    /// Items omitted from assemble / flatten until [`Self::show`].
    pub hidden: Vec<Arc<str>>,
    /// Center item that cannot hide, matching Gallery `gallery.primary`.
    pub primary: Option<Arc<str>>,
    pub(super) split_resize: Option<DockSplitResize>,
    pub(super) item_drag: Option<DockItemDrag>,
}

impl Dock {
    pub fn new(mut root: DockNode) -> Self {
        root.fold_past_split_depth();
        root.clamp_ratios();
        Self {
            root,
            drop_target: None,
            locked: false,
            titles: Vec::new(),
            style: NodeStyle::default(),
            hidden: Vec::new(),
            primary: None,
            split_resize: None,
            item_drag: None,
        }
    }

    pub fn primary(mut self, id: impl Into<Arc<str>>) -> Self {
        self.primary = Some(id.into());
        self
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
        visible_branch(&self.root, &self.hidden)
            .map(|node| node.flatten())
            .unwrap_or_default()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.root.contains(id)
    }

    pub fn visible_root(&self) -> Option<DockNode> {
        filter_hidden(&self.root, &self.hidden)
    }

    pub fn hide(&mut self, id: impl AsRef<str>) -> bool {
        let changed = hide_id(
            &self.root.flatten(),
            &mut self.hidden,
            self.primary.as_deref(),
            id.as_ref(),
        );
        if changed {
            self.clear_drop_if_missing();
        }
        changed
    }

    pub fn show(&mut self, id: impl AsRef<str>) -> bool {
        show_id(&mut self.hidden, id.as_ref())
    }

    pub fn is_visible(&self, id: &str) -> bool {
        self.contains(id) && !is_hidden(&self.hidden, id)
    }

    /// Move a tab before or after a sibling in the same strip. Locked docks refuse.
    pub fn reorder_tab(
        &mut self,
        dragged_id: impl AsRef<str>,
        target_id: impl AsRef<str>,
        before: bool,
    ) -> bool {
        if self.locked {
            return false;
        }
        reorder_dock_tab(
            &mut self.root,
            dragged_id.as_ref(),
            target_id.as_ref(),
            before,
        )
    }

    /// Move `id` onto `target`'s `zone`. Fails when extract or insert cannot apply.
    pub fn retarget(
        &mut self,
        id: impl AsRef<str>,
        target: impl AsRef<str>,
        zone: DockDropZone,
    ) -> bool {
        let id = id.as_ref();
        let target = target.as_ref();
        if id == target || self.primary.as_deref() == Some(id) || !self.can_take(id) {
            return false;
        }
        let before = self.root.clone();
        let Some(taken) = extract_item(&mut self.root, id) else {
            return false;
        };
        self.hidden.retain(|hidden| hidden.as_ref() != id);
        if !insert_dock_item(&mut self.root, target, taken, zone) {
            self.root = before;
            return false;
        }
        self.clear_drop_if_missing();
        true
    }

    fn can_take(&self, id: &str) -> bool {
        can_hide_id(
            &self.root.flatten(),
            &self.hidden,
            self.primary.as_deref(),
            id,
        )
    }

    pub(super) fn clear_drop_if_missing(&mut self) {
        if let Some((target, _)) = &self.drop_target
            && !self.is_visible(target.as_ref())
        {
            self.drop_target = None;
        }
    }

    /// Remove `id` from this surface. Fails when the id is missing or last.
    pub fn float_item(&mut self, id: impl AsRef<str>) -> Option<DockFloatingSurface> {
        self.float_item_at(
            id,
            DEFAULT_FLOATING_X,
            DEFAULT_FLOATING_Y,
            DEFAULT_FLOATING_WIDTH,
            DEFAULT_FLOATING_HEIGHT,
        )
    }

    pub fn float_item_at(
        &mut self,
        id: impl AsRef<str>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<DockFloatingSurface> {
        let id = id.as_ref();
        if !self.can_take(id) {
            return None;
        }
        let taken = extract_item(&mut self.root, id)?;
        self.hidden.retain(|hidden| hidden.as_ref() != id);
        self.clear_drop_if_missing();
        Some(DockFloatingSurface {
            id: Arc::from(id),
            root: taken,
            x: finite(x, DEFAULT_FLOATING_X),
            y: finite(y, DEFAULT_FLOATING_Y),
            width: finite(width, DEFAULT_FLOATING_WIDTH),
            height: finite(height, DEFAULT_FLOATING_HEIGHT),
        })
    }

    pub(super) fn title_for(&self, id: &str) -> Arc<str> {
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

    pub(super) fn node_drop(&self, node: &DockNode) -> Option<DockDropZone> {
        match node {
            DockNode::Item { id, .. } => self.drop_for(id),
            DockNode::Tabs { tabs, .. } => tabs.iter().find_map(|id| self.drop_for(id)),
            DockNode::Split { .. } => None,
        }
    }
}

impl ComponentView for Dock {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::DOCK.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(|caps| caps.dock = Some(&DOCK_CAPABILITIES)),
        slot_assembler: Some(crate::AppContext::assemble_dock_panels),
        slot_projector: Some(Self::project_with_slots),
        ..crate::TypeBehavior::NONE
    };

    fn fit_framework_limits(&mut self) {
        self.root.fold_past_split_depth();
    }

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "dock".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.project_with_chrome(id, world, mutations, None);
    }
}

impl Dock {
    fn project_with_slots(
        cx: &crate::AppContext,
        id: StableNodeId,
        view: &Self,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        view.project_with_chrome(id, world, mutations, cx.assembled_slot::<DockChrome>(id));
    }

    fn project_with_chrome(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        mutations: &mut MutationQueue,
        chrome: Option<&DockChrome>,
    ) {
        let visible = visible_branch(&self.root, &self.hidden);
        let root = visible.as_deref().unwrap_or(&self.root);
        if let Some(chrome) = chrome {
            project_chrome(self, chrome, root, world, mutations);
        } else {
            project_common(
                id,
                world,
                mutations,
                &dock_root_style(root, &self.style),
                InteractionState {
                    pointer_events: false,
                    focusable: false,
                },
                dock_root_accessibility(root),
            );
            project_content_slots(root, None, id, world, mutations);
        }
    }
}

pub(super) fn dock_root_style(root: &DockNode, base: &NodeStyle) -> NodeStyle {
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
                locked: dock.locked,
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

pub(super) fn apply_body_slot(
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
    } else if let Some(current) = world.node_style(id)
        && !current.layout.hidden
    {
        let mut style = current.clone();
        Arc::make_mut(&mut style.layout).hidden = true;
        mutations.set_style(id, style);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DockItemFrame {
    pub(super) id: Arc<str>,
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
pub(super) struct DockTitle {
    pub(super) label: Arc<str>,
    pub(super) locked: bool,
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
                    value: self.label.to_string().into(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &title_style(),
            InteractionState {
                pointer_events: !self.locked,
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
    crate::theme_intent::font(layout, nana_ui_core::TypeRole::Hint);
    crate::theme_intent::weight(layout, nana_ui_core::TextWeight::Semibold);
    layout.white_space = nana_ui_core::WhiteSpaceSpec::Nowrap;
    layout.text_overflow_ellipsis = true;
    style
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DockTabsFrame;

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
pub(super) struct DockSplitFrame {
    pub(super) axis: DockAxis,
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
pub(super) struct DockHandle {
    pub(super) axis: DockAxis,
    pub(super) locked: bool,
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
        focused: SemanticPaint::FOCUS_SURFACE,
        hovered: SemanticPaint {
            background: Some(SemanticColorRole::Hover),
            ..SemanticPaint::default()
        },
        ..InteractionStyle::default()
    }
    .into();
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
pub(super) struct DockHandleMark {
    pub(super) axis: DockAxis,
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
pub(super) struct DockDropOverlay {
    pub(super) zone: DockDropZone,
    pub(super) visible: bool,
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
        layout.border_width = Some(nana_ui_core::HAIRLINE);
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
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

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
                    value: String::new().into(),
                },
            );
        }
        if world.widget_visual_ref(id).is_some() {
            mutations.set_widget_visual_handle(id, None);
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
