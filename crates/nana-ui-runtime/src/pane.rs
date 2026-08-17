use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ButtonKind, ControlSize, FlexDirection, Icon, JustifySpec, LengthSpec,
    SemanticColorRole, SplitAxis,
};

use crate::split_pane::split_direction;
use crate::view_components::{IconButton, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, MutationQueue,
    NodeKind, NodeStyle, StableNodeId, TextContent, UiWorld,
};

const CHROME_HEIGHT: f32 = 34.0;
const CHROME_PADDING_X: f32 = 8.0;
const ACTION_TEXT_SIZE: f32 = 10.0;
const PANE_TREE_RATIO_MIN: f32 = 0.05;
const PANE_TREE_RATIO_MAX: f32 = 0.95;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneChromeActionKind {
    Focus,
    SplitHorizontal,
    SplitVertical,
    MoveToWindow,
    MoveToNextPane,
    ClosePane,
    CloseItem,
    Custom,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaneChromeAction {
    pub kind: PaneChromeActionKind,
    pub label: Arc<str>,
    pub icon: Option<Icon>,
    pub target: Option<StableNodeId>,
}

impl PaneChromeAction {
    pub fn new(kind: PaneChromeActionKind, label: impl Into<Arc<str>>) -> Self {
        Self {
            kind,
            label: label.into(),
            icon: None,
            target: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn target(mut self, target: StableNodeId) -> Self {
        self.target = Some(target);
        self
    }
}

/// 34px title row (tabs + icon actions) over a fill body slot.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneChrome {
    pub tabs: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub actions: Vec<PaneChromeAction>,
    pub active: bool,
    pub style: NodeStyle,
    pub header: Option<StableNodeId>,
}

impl PaneChrome {
    pub fn new() -> Self {
        Self {
            tabs: None,
            body: None,
            actions: Vec::new(),
            active: true,
            style: NodeStyle::default(),
            header: None,
        }
    }

    pub fn tabs(mut self, tabs: StableNodeId) -> Self {
        self.tabs = Some(tabs);
        self
    }

    pub fn body(mut self, body: StableNodeId) -> Self {
        self.body = Some(body);
        self
    }

    pub fn header(mut self, header: StableNodeId) -> Self {
        self.header = Some(header);
        self
    }

    pub fn actions(mut self, actions: impl IntoIterator<Item = PaneChromeAction>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn header_background(&self) -> SemanticColorRole {
        if self.active {
            SemanticColorRole::Surface
        } else {
            SemanticColorRole::Faint
        }
    }

    fn root_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.background = None;
        style.border = None;
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        layout.padding = None;
        layout.padding_top = Some(LengthSpec::Px(0.0));
        layout.padding_right = Some(LengthSpec::Px(0.0));
        layout.padding_bottom = Some(LengthSpec::Px(0.0));
        layout.padding_left = Some(LengthSpec::Px(0.0));
        style
    }

    fn header_style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        style.background = Some(self.header_background());
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::Start;
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(CHROME_HEIGHT));
        layout.min_height = Some(LengthSpec::Px(CHROME_HEIGHT));
        layout.max_height = Some(LengthSpec::Px(CHROME_HEIGHT));
        layout.padding_top = Some(LengthSpec::Px(0.0));
        layout.padding_bottom = Some(LengthSpec::Px(0.0));
        layout.padding_left = Some(LengthSpec::Px(CHROME_PADDING_X));
        layout.padding_right = Some(LengthSpec::Px(CHROME_PADDING_X));
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        style
    }

    fn project_header(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        let Some(id) = self.header else {
            return;
        };
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
        if world.standard_visual(id).is_some() {
            mutations.set_standard_visual(id, None);
        }
        project_common(
            id,
            world,
            mutations,
            &self.header_style(),
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

    fn project_tabs(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        let Some(id) = self.tabs else {
            return;
        };
        if world.node(id).is_none() {
            return;
        }
        let mut style = world.node_style(id).cloned().unwrap_or_default();
        style.background = None;
        style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
        let layout = Arc::make_mut(&mut style.layout);
        layout.flex_grow = Some(1.0);
        layout.flex_shrink = Some(1.0);
        layout.min_width = Some(LengthSpec::Px(0.0));
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        layout.padding_left = Some(LengthSpec::Px(0.0));
        layout.padding_right = Some(LengthSpec::Px(0.0));
        if layout.font_size.is_none() {
            layout.font_size = Some(12.0);
        }
        if world.node_style(id) != Some(&style) {
            mutations.set_style(id, style);
        }
    }

    fn project_body(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        let Some(id) = self.body else {
            return;
        };
        if world.node(id).is_none() {
            return;
        }
        let mut style = world.node_style(id).cloned().unwrap_or_default();
        style.background = None;
        let layout = Arc::make_mut(&mut style.layout);
        layout.flex_grow = Some(1.0);
        layout.flex_shrink = Some(1.0);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        layout.min_height = Some(LengthSpec::Px(0.0));
        layout.padding_top = Some(LengthSpec::Px(0.0));
        layout.padding_right = Some(LengthSpec::Px(0.0));
        layout.padding_bottom = Some(LengthSpec::Px(0.0));
        layout.padding_left = Some(LengthSpec::Px(0.0));
        if world.node_style(id) != Some(&style) {
            mutations.set_style(id, style);
        }
    }

    fn project_actions(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        for action in &self.actions {
            let Some(id) = action.target else {
                continue;
            };
            if world.node(id).is_none() {
                continue;
            }
            project_chrome_action(action, id, world, mutations);
        }
    }
}

impl Default for PaneChrome {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for PaneChrome {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "pane-chrome".into(),
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
            &self.root_style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
        self.project_header(world, mutations);
        self.project_tabs(world, mutations);
        self.project_actions(world, mutations);
        self.project_body(world, mutations);
    }
}

fn project_chrome_action(
    action: &PaneChromeAction,
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    if let Some(icon) = action.icon {
        IconButton::new(icon, Arc::clone(&action.label))
            .kind(ButtonKind::Text)
            .size(ControlSize::Small)
            .project(id, world, mutations);
        return;
    }
    if world.text(id) != Some(action.label.as_ref()) {
        mutations.set_text(
            id,
            TextContent {
                value: action.label.to_string(),
            },
        );
    }
    if world.standard_visual(id).is_some() {
        mutations.set_standard_visual(id, None);
    }
    let mut style = NodeStyle::default();
    style.foreground = Some(SemanticColorRole::Accent);
    style.background = None;
    style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
    let layout = Arc::make_mut(&mut style.layout);
    layout.font_size = Some(ACTION_TEXT_SIZE);
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    layout.height = Some(LengthSpec::Fill);
    layout.padding_left = Some(LengthSpec::Px(4.0));
    layout.padding_right = Some(LengthSpec::Px(4.0));
    project_common(
        id,
        world,
        mutations,
        &style,
        InteractionState {
            pointer_events: true,
            focusable: true,
        },
        AccessibilityState {
            role: AccessibilityRole::Button,
            label: Some(Arc::clone(&action.label)),
            ..AccessibilityState::default()
        },
    );
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaneTreeNode {
    Leaf {
        pane_id: Arc<str>,
        content: Option<StableNodeId>,
    },
    Split {
        split_id: Arc<str>,
        axis: SplitAxis,
        ratio: f32,
        first: Box<PaneTreeNode>,
        second: Box<PaneTreeNode>,
    },
}

impl PaneTreeNode {
    pub fn leaf(pane_id: impl Into<Arc<str>>) -> Self {
        Self::Leaf {
            pane_id: pane_id.into(),
            content: None,
        }
    }

    pub fn leaf_content(pane_id: impl Into<Arc<str>>, content: StableNodeId) -> Self {
        Self::Leaf {
            pane_id: pane_id.into(),
            content: Some(content),
        }
    }

    pub fn split(
        split_id: impl Into<Arc<str>>,
        axis: SplitAxis,
        ratio: f32,
        first: Self,
        second: Self,
    ) -> Self {
        Self::Split {
            split_id: split_id.into(),
            axis,
            ratio: clamp_split_ratio(ratio),
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    pub fn visit_leaves(&self, mut visitor: impl FnMut(&Arc<str>)) {
        self.visit_leaves_with(&mut visitor);
    }

    fn visit_leaves_with(&self, visitor: &mut impl FnMut(&Arc<str>)) {
        match self {
            Self::Leaf { pane_id, .. } => visitor(pane_id),
            Self::Split { first, second, .. } => {
                first.visit_leaves_with(visitor);
                second.visit_leaves_with(visitor);
            }
        }
    }

    pub fn visit_splits(&self, mut visitor: impl FnMut(&Arc<str>, SplitAxis, f32)) {
        self.visit_splits_with(&mut visitor);
    }

    fn visit_splits_with(&self, visitor: &mut impl FnMut(&Arc<str>, SplitAxis, f32)) {
        if let Self::Split {
            split_id,
            axis,
            ratio,
            first,
            second,
        } = self
        {
            visitor(split_id, *axis, *ratio);
            first.visit_splits_with(visitor);
            second.visit_splits_with(visitor);
        }
    }

    fn project_slots(&self, grow: Option<f32>, world: &UiWorld, mutations: &mut MutationQueue) {
        match self {
            Self::Leaf { content, .. } => {
                if let Some(content) = content {
                    project_leaf_slot(*content, grow, world, mutations);
                }
            }
            Self::Split {
                ratio,
                first,
                second,
                ..
            } => {
                let ratio = clamp_split_ratio(*ratio);
                first.project_slots(Some(ratio), world, mutations);
                second.project_slots(Some(1.0 - ratio), world, mutations);
            }
        }
    }
}

/// Recursive leaf/split composition. Leaves are host content slots.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneTree {
    pub root: PaneTreeNode,
    pub style: NodeStyle,
}

impl PaneTree {
    pub fn new(root: PaneTreeNode) -> Self {
        Self {
            root,
            style: NodeStyle::default(),
        }
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn visit_leaves(&self, visitor: impl FnMut(&Arc<str>)) {
        self.root.visit_leaves(visitor);
    }

    pub fn visit_splits(&self, visitor: impl FnMut(&Arc<str>, SplitAxis, f32)) {
        self.root.visit_splits(visitor);
    }

    fn root_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        layout.align_items = AlignSpec::Stretch;
        layout.direction = Some(match &self.root {
            PaneTreeNode::Split { axis, .. } => split_direction(*axis),
            PaneTreeNode::Leaf { .. } => FlexDirection::Column,
        });
        style
    }
}

impl ComponentView for PaneTree {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "pane-tree".into(),
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
            &self.root_style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
        match &self.root {
            PaneTreeNode::Leaf { content, .. } => {
                if let Some(content) = content {
                    project_leaf_slot(*content, None, world, mutations);
                }
            }
            PaneTreeNode::Split {
                ratio,
                first,
                second,
                ..
            } => {
                let ratio = clamp_split_ratio(*ratio);
                first.project_slots(Some(ratio), world, mutations);
                second.project_slots(Some(1.0 - ratio), world, mutations);
            }
        }
    }
}

fn clamp_split_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(PANE_TREE_RATIO_MIN, PANE_TREE_RATIO_MAX)
    } else {
        0.5
    }
}

fn project_leaf_slot(
    id: StableNodeId,
    grow: Option<f32>,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    if world.node(id).is_none() {
        return;
    }
    let mut style = world.node_style(id).cloned().unwrap_or_default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    if let Some(grow) = grow {
        layout.flex_grow = Some(grow);
        layout.flex_shrink = Some(1.0);
        layout.min_width = Some(LengthSpec::Px(0.0));
        layout.min_height = Some(LengthSpec::Px(0.0));
    } else if layout.flex_grow.is_none() {
        layout.flex_grow = Some(1.0);
        layout.flex_shrink = Some(1.0);
    }
    if world.node_style(id) != Some(&style) {
        mutations.set_style(id, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, StandardVisual};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn slot(context: &mut AppContext, tag: &str) -> StableNodeId {
        context
            .create_view(document(), NodeKind::Element { tag: tag.into() }, ())
            .unwrap()
            .stable_id()
    }

    #[test]
    fn pane_chrome_active_uses_surface_inactive_uses_faint() {
        let mut context = AppContext::new();
        let tabs = slot(&mut context, "tabs");
        let body = slot(&mut context, "body");
        let header = slot(&mut context, "header");
        let chrome = context
            .create_component(
                document(),
                PaneChrome::new()
                    .tabs(tabs)
                    .body(body)
                    .header(header)
                    .active(true),
            )
            .unwrap();
        let id = chrome.stable_id();
        assert_eq!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element {
                tag: "pane-chrome".into(),
            }
        );
        assert_eq!(context.world().node_style(id).unwrap().background, None);
        assert_eq!(
            context.world().node_style(header).unwrap().background,
            Some(SemanticColorRole::Surface)
        );
        assert_eq!(
            context.world().node_style(header).unwrap().layout.height,
            Some(LengthSpec::Px(CHROME_HEIGHT))
        );
        assert_eq!(
            context
                .world()
                .node_style(header)
                .unwrap()
                .layout
                .padding_left,
            Some(LengthSpec::Px(CHROME_PADDING_X))
        );
        assert_eq!(
            context.world().node_style(tabs).unwrap().layout.flex_grow,
            Some(1.0)
        );
        assert_eq!(
            context.world().node_style(body).unwrap().layout.flex_grow,
            Some(1.0)
        );

        context
            .update_component(chrome, |chrome, _| {
                chrome.active = false;
            })
            .unwrap();
        assert_eq!(context.world().node_style(id).unwrap().background, None);
        assert_eq!(
            context.world().node_style(header).unwrap().background,
            Some(SemanticColorRole::Faint)
        );
    }

    #[test]
    fn pane_tree_preserves_leaf_and_split_identity_order() {
        let tree = PaneTreeNode::split(
            "root",
            SplitAxis::Horizontal,
            0.6,
            PaneTreeNode::leaf("left"),
            PaneTreeNode::split(
                "right-stack",
                SplitAxis::Vertical,
                0.4,
                PaneTreeNode::leaf("top"),
                PaneTreeNode::leaf("bottom"),
            ),
        );
        let mut leaves = Vec::new();
        tree.visit_leaves(|pane_id| leaves.push(pane_id.to_string()));
        assert_eq!(leaves, ["left", "top", "bottom"]);
        let mut splits = Vec::new();
        tree.visit_splits(|split_id, axis, ratio| {
            splits.push((split_id.to_string(), axis, ratio));
        });
        assert_eq!(
            splits,
            [
                ("root".to_string(), SplitAxis::Horizontal, 0.6),
                ("right-stack".to_string(), SplitAxis::Vertical, 0.4),
            ]
        );
    }

    #[test]
    fn pane_tree_clamps_ratio_at_the_public_boundary() {
        let PaneTreeNode::Split { ratio, .. } = PaneTreeNode::split(
            "root",
            SplitAxis::Horizontal,
            2.0,
            PaneTreeNode::leaf("left"),
            PaneTreeNode::leaf("right"),
        ) else {
            panic!("split")
        };
        assert_eq!(ratio, 0.95);
    }

    #[test]
    fn pane_tree_projects_leaf_slots_with_split_ratio() {
        let mut context = AppContext::new();
        let left = slot(&mut context, "left");
        let top = slot(&mut context, "top");
        let bottom = slot(&mut context, "bottom");
        let tree = context
            .create_component(
                document(),
                PaneTree::new(PaneTreeNode::split(
                    "root",
                    SplitAxis::Horizontal,
                    0.6,
                    PaneTreeNode::leaf_content("left", left),
                    PaneTreeNode::split(
                        "right-stack",
                        SplitAxis::Vertical,
                        0.4,
                        PaneTreeNode::leaf_content("top", top),
                        PaneTreeNode::leaf_content("bottom", bottom),
                    ),
                )),
            )
            .unwrap();
        assert_eq!(
            context.world().node(tree.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "pane-tree".into(),
            }
        );
        assert_eq!(
            context
                .world()
                .node_style(tree.stable_id())
                .unwrap()
                .layout
                .direction,
            Some(FlexDirection::Row)
        );
        assert_eq!(
            context.world().node_style(left).unwrap().layout.flex_grow,
            Some(0.6)
        );
        assert_eq!(
            context.world().node_style(top).unwrap().layout.flex_grow,
            Some(0.4)
        );
        assert_eq!(
            context.world().node_style(bottom).unwrap().layout.flex_grow,
            Some(0.6)
        );
    }

    #[test]
    fn pane_chrome_icon_action_is_muted_text_kind() {
        let mut context = AppContext::new();
        let action = slot(&mut context, "action");
        let _ = context
            .create_component(
                document(),
                PaneChrome::new().actions([PaneChromeAction::new(
                    PaneChromeActionKind::ClosePane,
                    "Close",
                )
                .icon(Icon::Close)
                .target(action)]),
            )
            .unwrap();
        assert_eq!(
            context.world().standard_visual(action),
            Some(StandardVisual::Icon {
                icon: Icon::Close,
                size: ControlSize::Small.icon_size(),
                tooltip: None,
            })
        );
        assert_eq!(
            context.world().node_style(action).unwrap().foreground,
            Some(SemanticColorRole::Muted)
        );
        assert!(
            context
                .world()
                .node_style(action)
                .unwrap()
                .background
                .is_none()
        );
        assert_eq!(
            context.world().interaction(action),
            Some(InteractionState {
                pointer_events: true,
                focusable: true,
            })
        );
    }

    #[test]
    fn idle_project_does_not_dirty() {
        let mut context = AppContext::new();
        let tabs = slot(&mut context, "tabs");
        let body = slot(&mut context, "body");
        let chrome = context
            .create_component(document(), PaneChrome::new().tabs(tabs).body(body))
            .unwrap();
        let tree = context
            .create_component(document(), PaneTree::new(PaneTreeNode::leaf("only")))
            .unwrap();
        let _ = context.take_system_work();
        context.update_component(chrome, |_, _| {}).unwrap();
        context.update_component(tree, |_, _| {}).unwrap();
        assert!(context.take_system_work().is_empty());
    }
}
