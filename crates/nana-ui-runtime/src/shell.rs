use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, Icon, JustifySpec, LengthSpec, OverflowSpec,
    PositionSpec, SemanticColorRole, TITLE_BAR_HEIGHT, UI_METRICS,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, InteractionStyle,
    MutationQueue, NodeKind, NodeStyle, SemanticPaint, StableNodeId, StandardVisual, TextContent,
    UiWorld,
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
    use crate::{AppContext, DocumentId, IconButton, LayoutViewport, Text};

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
}
