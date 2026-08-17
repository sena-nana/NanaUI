use std::sync::Arc;

use nana_ui_core::{
    FlexDirection, LengthSpec, OverflowSpec, PopoverAlignment, PopoverPlacement, PositionSpec,
    SemanticColorRole, SemanticPalette, UI_METRICS,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentElevation, ComponentGeometry,
    ComponentTextRegion, InteractionState, LayoutBox, MenuSurfaceKind, MutationQueue, NodeKind,
    NodeStyle, StableNodeId, StandardVisual, UiWorld,
};

const POPOVER_WIDTH: f32 = 240.0;
const POPOVER_PADDING: f32 = 10.0;
const POPOVER_GAP: f32 = 6.0;
const ACTION_MENU_WIDTH: f32 = 200.0;
const ACTION_MENU_PADDING: f32 = 4.0;
const ACTION_MENU_GAP: f32 = 4.0;
const MENU_MIN_WIDTH: f32 = 120.0;
pub(crate) const TRIGGER_HEIGHT: f32 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopoverToggled {
    pub open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopoverClosed;

/// In-flow trigger with an optional surface painted below it.
#[derive(Debug, Clone, PartialEq)]
pub struct Popover {
    pub trigger: Arc<str>,
    pub open: bool,
    pub placement: PopoverPlacement,
    pub alignment: PopoverAlignment,
    pub gap: f32,
    pub width: f32,
    pub padding: f32,
    pub close_on_escape: bool,
    pub close_on_outside: bool,
}

impl Popover {
    pub fn new() -> Self {
        Self {
            trigger: Arc::from(""),
            open: false,
            placement: PopoverPlacement::Bottom,
            alignment: PopoverAlignment::Center,
            gap: POPOVER_GAP,
            width: POPOVER_WIDTH,
            padding: POPOVER_PADDING,
            close_on_escape: true,
            close_on_outside: true,
        }
    }

    pub fn trigger(mut self, trigger: impl Into<Arc<str>>) -> Self {
        self.trigger = trigger.into();
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn placement(mut self, placement: PopoverPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn alignment(mut self, alignment: PopoverAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(MENU_MIN_WIDTH);
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding.max(0.0);
        self
    }

    pub fn close_on_escape(mut self, enabled: bool) -> Self {
        self.close_on_escape = enabled;
        self
    }

    pub fn close_on_outside(mut self, enabled: bool) -> Self {
        self.close_on_outside = enabled;
        self
    }
}

impl Default for Popover {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::ComponentView for Popover {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "popover".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_menu_surface(
            id,
            world,
            mutations,
            MenuSurfaceKind::Popover,
            Some(Arc::clone(&self.trigger)).filter(|value| !value.is_empty()),
            self.open,
            self.width,
            self.padding,
            self.gap,
            "popover",
        );
    }
}

/// Trigger-bound action menu. Same surface as Popover with start alignment.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionMenu {
    pub popover: Popover,
}

impl ActionMenu {
    pub fn new() -> Self {
        Self {
            popover: Popover::new()
                .alignment(PopoverAlignment::Start)
                .gap(ACTION_MENU_GAP)
                .width(ACTION_MENU_WIDTH)
                .padding(ACTION_MENU_PADDING),
        }
    }

    pub fn trigger(mut self, trigger: impl Into<Arc<str>>) -> Self {
        self.popover = self.popover.trigger(trigger);
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.popover.open = open;
        self
    }

    pub fn placement(mut self, placement: PopoverPlacement) -> Self {
        self.popover = self.popover.placement(placement);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.popover = self.popover.width(width);
        self
    }
}

impl Default for ActionMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::ComponentView for ActionMenu {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "action-menu".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_menu_surface(
            id,
            world,
            mutations,
            MenuSurfaceKind::ActionMenu,
            Some(Arc::clone(&self.popover.trigger)).filter(|value| !value.is_empty()),
            self.popover.open,
            self.popover.width,
            self.popover.padding,
            self.popover.gap,
            "action-menu",
        );
    }
}

pub(crate) fn project_menu_surface(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    kind: MenuSurfaceKind,
    trigger: Option<Arc<str>>,
    open: bool,
    width: f32,
    padding: f32,
    gap: f32,
    label: &str,
) {
    let has_chrome = open || trigger.is_some();
    if has_chrome {
        let visual = StandardVisual::MenuSurface {
            kind,
            trigger: trigger.clone(),
            gap,
            query: None,
            rows: Arc::from([]),
            highlighted: None,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
    } else if world.standard_visual(id).is_some() {
        mutations.set_standard_visual(id, None);
    }
    project_common(
        id,
        world,
        mutations,
        &triggered_menu_style(trigger.as_deref(), open, width, padding, gap),
        InteractionState {
            pointer_events: open || trigger.is_some(),
            focusable: trigger.is_some(),
        },
        AccessibilityState {
            role: AccessibilityRole::Menu,
            label: Some(trigger.clone().unwrap_or_else(|| Arc::from(label))),
            ..AccessibilityState::default()
        },
    );
}

fn triggered_menu_style(
    trigger: Option<&str>,
    open: bool,
    width: f32,
    padding: f32,
    gap: f32,
) -> NodeStyle {
    let trigger_h = if trigger.is_some() {
        TRIGGER_HEIGHT
    } else {
        0.0
    };
    if !open {
        return NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                height: Some(LengthSpec::Px(trigger_h.max(1.0))),
                overflow_x: OverflowSpec::Hidden,
                overflow_y: OverflowSpec::Hidden,
                ..nana_ui_core::LayoutStyle::default()
            }),
            foreground: Some(SemanticColorRole::Text),
            ..NodeStyle::default()
        };
    }
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            width: Some(LengthSpec::Px(width.max(MENU_MIN_WIDTH))),
            min_width: Some(LengthSpec::Px(MENU_MIN_WIDTH)),
            direction: Some(FlexDirection::Column),
            gap: Some(LengthSpec::Px(1.0)),
            padding_left: Some(LengthSpec::Px(padding)),
            padding_right: Some(LengthSpec::Px(padding)),
            padding_top: Some(LengthSpec::Px(trigger_h + gap + padding)),
            padding_bottom: Some(LengthSpec::Px(padding)),
            border_radius: Some(UI_METRICS.radius_md),
            ..nana_ui_core::LayoutStyle::default()
        }),
        foreground: Some(SemanticColorRole::Text),
        ..NodeStyle::default()
    }
}

pub(crate) fn menu_surface_style(width: f32, padding: f32) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            position: PositionSpec::Fixed,
            width: Some(LengthSpec::Px(width)),
            min_width: Some(LengthSpec::Px(MENU_MIN_WIDTH)),
            direction: Some(FlexDirection::Column),
            gap: Some(LengthSpec::Px(1.0)),
            padding_left: Some(LengthSpec::Px(padding)),
            padding_right: Some(LengthSpec::Px(padding)),
            padding_top: Some(LengthSpec::Px(padding)),
            padding_bottom: Some(LengthSpec::Px(padding)),
            border_width: Some(1.0),
            border_radius: Some(UI_METRICS.radius_md),
            z_index: Some(1_000),
            ..nana_ui_core::LayoutStyle::default()
        }),
        background: Some(SemanticColorRole::Surface),
        border: Some(SemanticColorRole::BorderSoft),
        foreground: Some(SemanticColorRole::Text),
        ..NodeStyle::default()
    }
}

pub(crate) fn project_anchored_menu(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    kind: MenuSurfaceKind,
    style: &NodeStyle,
    open: bool,
    label: &str,
) {
    if open {
        let visual = StandardVisual::MenuSurface {
            kind,
            trigger: None,
            gap: 0.0,
            query: None,
            rows: Arc::from([]),
            highlighted: None,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
    } else if world.standard_visual(id).is_some() {
        mutations.set_standard_visual(id, None);
    }
    let mut style = style.clone();
    Arc::make_mut(&mut style.layout).hidden = !open;
    project_common(
        id,
        world,
        mutations,
        &style,
        InteractionState {
            pointer_events: open,
            focusable: false,
        },
        AccessibilityState {
            role: AccessibilityRole::Menu,
            label: Some(Arc::from(label)),
            ..AccessibilityState::default()
        },
    );
}

pub(crate) fn menu_surface_geometry(
    bounds: LayoutBox,
    trigger: Option<&Arc<str>>,
    gap: f32,
    palette: &SemanticPalette,
) -> ComponentGeometry {
    let is_light = palette.background.as_rgba_array()[0] > 0.5;
    let trigger_h = if trigger.is_some() {
        TRIGGER_HEIGHT
    } else {
        0.0
    };
    let surface = if trigger_h > 0.0 {
        LayoutBox {
            x: bounds.x,
            y: bounds.y + trigger_h + gap,
            width: bounds.width,
            height: (bounds.height - trigger_h - gap).max(0.0),
        }
    } else {
        bounds
    };
    ComponentGeometry::MenuSurface {
        trigger: trigger
            .filter(|value| !value.is_empty())
            .map(|value| ComponentTextRegion {
                bounds: LayoutBox {
                    x: bounds.x,
                    y: bounds.y,
                    width: bounds.width,
                    height: trigger_h,
                },
                content: Arc::clone(value),
                color: Some(palette.text.as_rgba_array()),
                font_size: 13.0,
                font_weight: None,
            }),
        surface,
        search: None,
        search_field: None,
        options: Vec::new(),
        elevation: ComponentElevation {
            color: [0.0, 0.0, 0.0, if is_light { 0.30 } else { 0.55 }],
            offset_y: 4.0,
            blur_radius: if is_light { 14.0 } else { 18.0 },
        },
        background: palette.surface.as_rgba_array(),
        border: palette.border_soft.as_rgba_array(),
    }
}

pub fn resolve_popover_origin(
    trigger: LayoutBox,
    surface_width: f32,
    surface_height: f32,
    viewport: LayoutBox,
    placement: PopoverPlacement,
    alignment: PopoverAlignment,
    gap: f32,
) -> (f32, f32) {
    let mut x = match placement {
        PopoverPlacement::Top | PopoverPlacement::Bottom => match alignment {
            PopoverAlignment::Start => trigger.x,
            PopoverAlignment::Center => trigger.x + trigger.width / 2.0 - surface_width / 2.0,
            PopoverAlignment::End => trigger.x + trigger.width - surface_width,
        },
        PopoverPlacement::Left => trigger.x - surface_width - gap,
        PopoverPlacement::Right => trigger.x + trigger.width + gap,
    };
    let mut y = match placement {
        PopoverPlacement::Top => trigger.y - surface_height - gap,
        PopoverPlacement::Bottom => trigger.y + trigger.height + gap,
        PopoverPlacement::Left | PopoverPlacement::Right => match alignment {
            PopoverAlignment::Start => trigger.y,
            PopoverAlignment::Center => trigger.y + trigger.height / 2.0 - surface_height / 2.0,
            PopoverAlignment::End => trigger.y + trigger.height - surface_height,
        },
    };
    let max_x = (viewport.x + viewport.width - surface_width).max(viewport.x);
    let max_y = (viewport.y + viewport.height - surface_height).max(viewport.y);
    x = x.clamp(viewport.x, max_x);
    y = y.clamp(viewport.y, max_y);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn popover_closed_keeps_the_trigger_and_open_reserves_surface_padding() {
        let mut context = AppContext::new();
        let popover = context
            .create_component(document(), Popover::new().trigger("Details"))
            .unwrap();
        let id = popover.stable_id();
        assert!(matches!(
            context.world().standard_visual(id),
            Some(StandardVisual::MenuSurface {
                kind: MenuSurfaceKind::Popover,
                trigger: Some(label),
                ..
            }) if label.as_ref() == "Details"
        ));
        let closed = context.world().node_style(id).unwrap();
        assert!(!closed.layout.hidden);
        assert_eq!(closed.layout.height, Some(LengthSpec::Px(TRIGGER_HEIGHT)));
        assert!(closed.background.is_none());
        context
            .update_component(popover, |popover, _| {
                popover.open = true;
            })
            .unwrap();
        let open = context.world().node_style(id).unwrap();
        assert_eq!(
            open.layout.padding_top,
            Some(LengthSpec::Px(
                TRIGGER_HEIGHT + POPOVER_GAP + POPOVER_PADDING
            ))
        );
        assert_eq!(
            open.layout.padding_left,
            Some(LengthSpec::Px(POPOVER_PADDING))
        );
        assert!(open.background.is_none());
    }

    #[test]
    fn action_menu_defaults_start_alignment_and_compact_padding() {
        let menu = ActionMenu::new();
        assert_eq!(menu.popover.alignment, PopoverAlignment::Start);
        assert_eq!(menu.popover.gap, ACTION_MENU_GAP);
        assert_eq!(menu.popover.width, ACTION_MENU_WIDTH);
        assert_eq!(menu.popover.padding, ACTION_MENU_PADDING);
    }

    #[test]
    fn popover_origin_clamps_to_the_viewport() {
        let trigger = LayoutBox {
            x: 90.0,
            y: 80.0,
            width: 20.0,
            height: 20.0,
        };
        let viewport = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 120.0,
        };
        assert_eq!(
            resolve_popover_origin(
                trigger,
                80.0,
                60.0,
                viewport,
                PopoverPlacement::Bottom,
                PopoverAlignment::Center,
                6.0,
            ),
            (40.0, 60.0)
        );
        let tight = LayoutBox {
            x: 4.0,
            y: 4.0,
            width: 20.0,
            height: 20.0,
        };
        assert_eq!(
            resolve_popover_origin(
                tight,
                80.0,
                60.0,
                viewport,
                PopoverPlacement::Left,
                PopoverAlignment::Center,
                6.0,
            ),
            (0.0, 0.0)
        );
    }
}
