use std::sync::Arc;

use nana_ui_core::{
    ControlSize, FlexDirection, Icon, LengthSpec, OverflowSpec, PopoverAlignment,
    PopoverPlacement, PositionSpec, SemanticColorRole, SemanticPalette, UI_BASE_TEXT_SIZE,
    UI_METRICS,
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
/// The trigger is a real button, so it matches the compact control height
/// rather than hugging its glyphs.
pub(crate) const TRIGGER_HEIGHT: f32 = UI_METRICS.compact_control_height;
const TRIGGER_PADDING_X: f32 = 10.0;
/// Icon triggers draw the standard control glyph size, centered in the chrome.
const TRIGGER_ICON_SIZE: f32 = UI_BASE_TEXT_SIZE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopoverToggled {
    pub open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopoverClosed;

/// In-flow trigger with an optional surface painted below it.
#[derive(Debug, Clone, PartialEq)]
pub struct Popover {
    /// Text trigger label, or the accessible name when [`Popover::trigger_icon`]
    /// switches the trigger to a glyph.
    pub trigger: Arc<str>,
    pub trigger_icon: Option<Icon>,
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
            trigger_icon: None,
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

    /// Icon trigger with an accessible name; the glyph is drawn centered in a
    /// square chrome instead of riding the label's text metrics.
    pub fn trigger_icon(mut self, icon: Icon, label: impl Into<Arc<str>>) -> Self {
        self.trigger = label.into();
        self.trigger_icon = Some(icon);
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

    /// Items attach and detach under the open surface, so the anchored
    /// projection must re-run with them to keep the origin math current.
    fn wants_child_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_menu_surface(
            id,
            world,
            mutations,
            MenuSurfaceKind::Popover,
            Some(Arc::clone(&self.trigger)).filter(|value| !value.is_empty()),
            self.trigger_icon,
            self.open,
            self.width,
            self.padding,
            self.gap,
            self.placement,
            self.alignment,
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

    pub fn trigger_icon(mut self, icon: Icon, label: impl Into<Arc<str>>) -> Self {
        self.popover = self.popover.trigger_icon(icon, label);
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

    /// Items attach and detach under the open surface, so the anchored
    /// projection must re-run with them to keep the origin math current.
    fn wants_child_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_menu_surface(
            id,
            world,
            mutations,
            MenuSurfaceKind::ActionMenu,
            Some(Arc::clone(&self.popover.trigger)).filter(|value| !value.is_empty()),
            self.popover.trigger_icon,
            self.popover.open,
            self.popover.width,
            self.popover.padding,
            self.popover.gap,
            self.popover.placement,
            self.popover.alignment,
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
    trigger_icon: Option<Icon>,
    open: bool,
    width: f32,
    padding: f32,
    gap: f32,
    placement: PopoverPlacement,
    alignment: PopoverAlignment,
    label: &str,
) {
    let has_trigger = trigger.is_some() || trigger_icon.is_some();
    let has_chrome = open || has_trigger;
    if has_chrome {
        let visual = StandardVisual::MenuSurface {
            kind,
            open,
            trigger: trigger.clone(),
            trigger_icon,
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
    // The trigger label is measured like any other button label, so the closed
    // surface can size itself to the text instead of a fixed box. Icon
    // triggers keep their label as the accessible name only; the square chrome
    // comes from the style instead of text measurement.
    let trigger_text = if trigger_icon.is_some() {
        ""
    } else {
        trigger.as_deref().unwrap_or("")
    };
    if world.text(id) != Some(trigger_text) {
        mutations.set_text(
            id,
            crate::TextContent {
                value: trigger_text.to_string(),
            },
        );
    }
    let mut style = triggered_menu_style(
        trigger_icon.is_some(),
        trigger.as_deref(),
        open,
        width,
        padding,
        gap,
    );
    if open && has_trigger {
        let surface_height = open_surface_height(world, id, TRIGGER_HEIGHT, padding, gap);
        anchor_open_surface(
            id,
            world,
            &mut style,
            placement,
            alignment,
            width.max(MENU_MIN_WIDTH),
            surface_height,
            gap,
        );
    }
    project_common(
        id,
        world,
        mutations,
        &style,
        InteractionState {
            pointer_events: open || has_trigger,
            focusable: has_trigger,
        },
        AccessibilityState {
            role: AccessibilityRole::Menu,
            label: Some(trigger.unwrap_or_else(|| Arc::from(label))),
            ..AccessibilityState::default()
        },
    );
}

/// An open surface must not participate in its host layout: it anchors to the
/// box of the slot it is mounted in (the parent node, which keeps the trigger's
/// neighbourhood stable) and switches to a viewport-basis `Fixed` box. Without
/// a laid-out slot the surface falls back to the previous in-flow morph.
fn anchor_open_surface(
    id: StableNodeId,
    world: &UiWorld,
    style: &mut NodeStyle,
    placement: PopoverPlacement,
    alignment: PopoverAlignment,
    surface_width: f32,
    surface_height: f32,
    gap: f32,
) {
    let slot = world
        .node(id)
        .and_then(|node| node.parent)
        .and_then(|parent| world.layout_box(parent))
        .or_else(|| world.layout_box(id));
    let Some(slot_box) = slot else {
        return;
    };
    // The viewport is not known at projection time, so edge clamping is left to
    // a later pass; the placement math itself stays in resolve_popover_origin.
    let unbounded = LayoutBox {
        x: -1.0e9,
        y: -1.0e9,
        width: 2.0e9,
        height: 2.0e9,
    };
    let (x, y) = resolve_popover_origin(
        slot_box,
        surface_width,
        surface_height,
        unbounded,
        placement,
        alignment,
        gap,
    );
    let layout = Arc::make_mut(&mut style.layout);
    layout.position = PositionSpec::Fixed;
    layout.offset_left = Some(LengthSpec::Px(x));
    layout.offset_top = Some(LengthSpec::Px(y));
}

/// Estimated open-surface height (trigger strip plus item rows) for the origin
/// math above; rows measure like compact menu items with 1px separation.
fn open_surface_height(
    world: &UiWorld,
    id: StableNodeId,
    trigger_height: f32,
    padding: f32,
    gap: f32,
) -> f32 {
    let items = world
        .node(id)
        .map(|node| node.children.len())
        .unwrap_or(0) as f32;
    let item = ControlSize::Small.height();
    trigger_height + gap + padding * 2.0 + items * item + (items - 1.0).max(0.0)
}

fn triggered_menu_style(
    icon_trigger: bool,
    trigger: Option<&str>,
    open: bool,
    width: f32,
    padding: f32,
    gap: f32,
) -> NodeStyle {
    let has_trigger = trigger.is_some() || icon_trigger;
    let trigger_h = if has_trigger { TRIGGER_HEIGHT } else { 0.0 };
    if !open {
        if icon_trigger {
            return trigger_icon_button_style();
        }
        if trigger.is_some() {
            return trigger_button_style();
        }
        // Nothing to press and nothing to show, so the node keeps the smallest
        // box that stays out of the way.
        return NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                width: Some(LengthSpec::Px(1.0)),
                height: Some(LengthSpec::Px(1.0)),
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

/// The trigger carries the same chrome contract as [`ButtonKind::Menu`]
/// (`Button`), so a standalone menu button and an in-place action-menu trigger
/// read as one control; hover and press colours resolve through the usual
/// interaction overlay.
fn trigger_button_style() -> NodeStyle {
    let mut style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            // A button hugs its label. Without this the surrounding stack
            // stretches the trigger and it reads as a field, not a control.
            align_self: Some(nana_ui_core::AlignSpec::Start),
            height: Some(LengthSpec::Px(TRIGGER_HEIGHT)),
            min_height: Some(LengthSpec::Px(TRIGGER_HEIGHT)),
            padding_left: Some(LengthSpec::Px(TRIGGER_PADDING_X)),
            padding_right: Some(LengthSpec::Px(TRIGGER_PADDING_X)),
            border_width: Some(1.0),
            border_radius: Some(UI_METRICS.radius_sm),
            ..nana_ui_core::LayoutStyle::default()
        }),
        background: Some(SemanticColorRole::Subtle),
        border: Some(SemanticColorRole::BorderSoft),
        foreground: Some(SemanticColorRole::Text),
        ..NodeStyle::default()
    };
    style.interaction.hovered.background = Some(SemanticColorRole::Hover);
    style.interaction.pressed.background = Some(SemanticColorRole::Active);
    style
}

/// Icon triggers share the text trigger's chrome but take a square min box, so
/// the glyph centers geometrically instead of riding text metrics.
fn trigger_icon_button_style() -> NodeStyle {
    let mut style = trigger_button_style();
    let layout = Arc::make_mut(&mut style.layout);
    layout.min_width = Some(LengthSpec::Px(TRIGGER_HEIGHT));
    style
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
            open,
            trigger: None,
            trigger_icon: None,
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
    trigger_icon: Option<Icon>,
    gap: f32,
    style: &crate::ComputedStyle,
    palette: &SemanticPalette,
) -> ComponentGeometry {
    let is_light = palette.background.as_rgba_array()[0] > 0.5;
    let has_trigger = trigger.is_some() || trigger_icon.is_some();
    let trigger_h = if has_trigger { TRIGGER_HEIGHT } else { 0.0 };
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
    // In icon mode the trigger text is only the accessible name, so no text
    // region is emitted and the glyph owns the chrome.
    let label = if trigger_icon.is_none() {
        trigger.filter(|value| !value.is_empty())
    } else {
        None
    };
    let trigger_bounds = LayoutBox {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: trigger_h,
    };
    let icon_extent = TRIGGER_ICON_SIZE
        .min(trigger_bounds.width)
        .min(trigger_bounds.height);
    ComponentGeometry::MenuSurface {
        trigger: label.map(|value| ComponentTextRegion {
            bounds: LayoutBox {
                x: trigger_bounds.x + TRIGGER_PADDING_X,
                width: (trigger_bounds.width - TRIGGER_PADDING_X * 2.0).max(0.0),
                ..trigger_bounds
            },
            content: Arc::clone(value),
            color: Some(style.color.unwrap_or_else(|| palette.text.as_rgba_array())),
            font_size: 13.0,
            font_weight: None,
        }),
        trigger_icon: trigger_icon.map(|icon| {
            (
                icon,
                LayoutBox {
                    x: trigger_bounds.x + (trigger_bounds.width - icon_extent) / 2.0,
                    y: trigger_bounds.y + (trigger_bounds.height - icon_extent) / 2.0,
                    width: icon_extent,
                    height: icon_extent,
                },
            )
        }),
        // Hover and press already resolved into the computed style, so the
        // trigger reads its chrome from there rather than the raw palette.
        trigger_surface: has_trigger.then_some(crate::ComponentTriggerSurface {
            bounds: trigger_bounds,
            background: style.background,
            border: style.border_color,
        }),
        surface,
        search: None,
        search_field: None,
        options: Vec::new(),
        elevation: ComponentElevation {
            color: [0.0, 0.0, 0.0, if is_light { 0.30 } else { 0.55 }],
            offset_x: 0.0,
            offset_y: 4.0,
            blur_radius: if is_light { 14.0 } else { 18.0 },
            spread_radius: 0.0,
            inset: false,
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
    use crate::LayoutViewport;
    use crate::framework::AppContext;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn choosing_an_item_closes_the_menu_that_offered_it() {
        let mut context = AppContext::new();
        let menu = context
            .create_component(document(), ActionMenu::new().trigger("Actions").open(true))
            .unwrap();
        let item = context
            .create_component(document(), crate::ActionMenuItem::new("Rename"))
            .unwrap();
        context.append_child(menu, item).unwrap();
        assert!(context.activate_action_menu_item(item).unwrap());
        assert!(!context.read(menu, |menu| menu.popover.open).unwrap());
    }

    /// The trigger is the only pressable affordance a closed menu has, so it
    /// must carry its own background rather than reading as bare text.
    #[test]
    fn a_closed_trigger_paints_pressable_button_chrome() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        let id = StableNodeId::new(1).unwrap();
        queue.create(
            id,
            document(),
            NodeKind::Element {
                tag: "action-menu".into(),
            },
        );
        queue.write_layout(
            id,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: TRIGGER_HEIGHT,
            },
        );
        queue.set_style(id, trigger_button_style());
        queue.set_standard_visual(
            id,
            Some(StandardVisual::MenuSurface {
                kind: MenuSurfaceKind::ActionMenu,
                open: false,
                trigger: Some(Arc::from("Actions")),
                trigger_icon: None,
                gap: 0.0,
                query: None,
                rows: Arc::from([]),
                highlighted: None,
            }),
        );
        world.commit(queue).unwrap();
        world.resolve_styles(&[id]).unwrap();
        let Some(ComponentGeometry::MenuSurface {
            trigger,
            trigger_surface,
            ..
        }) = world.component_geometry(id)
        else {
            panic!("expected menu surface geometry");
        };
        let chrome = trigger_surface.expect("trigger chrome");
        let idle = chrome.background.expect("trigger has a filled surface");
        assert_eq!(chrome.bounds.height, TRIGGER_HEIGHT);
        let label = trigger.expect("trigger label");
        assert!(label.bounds.x > 0.0, "label sits inside the button padding");

        world.set_pointer_hover(document(), 1, Some(id)).unwrap();
        world.resolve_styles(&[id]).unwrap();
        let Some(ComponentGeometry::MenuSurface {
            trigger_surface, ..
        }) = world.component_geometry(id)
        else {
            panic!("expected menu surface geometry");
        };
        let hovered = trigger_surface
            .expect("trigger chrome")
            .background
            .expect("hovered trigger stays filled");
        assert_ne!(idle, hovered, "the trigger answers the pointer");
    }

    /// The trigger and the surface share one box, so a closed menu whose items
    /// still took part in layout would be stretched to their width.
    #[test]
    fn a_closed_menu_keeps_its_items_out_of_the_layout() {
        let mut context = AppContext::new();
        let menu = context
            .create_component(document(), ActionMenu::new().trigger("Actions"))
            .unwrap();
        let item = context
            .create_component(document(), crate::ActionMenuItem::new("Rename"))
            .unwrap();
        context.append_child(menu, item).unwrap();
        let omits_box = |context: &AppContext| {
            context
                .world()
                .layout_style(item.stable_id())
                .unwrap()
                .omits_box()
        };
        assert!(omits_box(&context));
        context.toggle_action_menu(menu).unwrap();
        assert!(!omits_box(&context));
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
        // A closed trigger is a pressable button, not bare text.
        assert_eq!(closed.background, Some(SemanticColorRole::Subtle));
        assert_eq!(closed.border, Some(SemanticColorRole::BorderSoft));
        assert!(closed.interaction.hovered.background.is_some());
        assert!(closed.interaction.pressed.background.is_some());
        assert!(closed.layout.width.is_none(), "trigger sizes to its label");
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
    fn trigger_icon_swaps_the_label_for_an_accessible_glyph_trigger() {
        let mut context = AppContext::new();
        let menu = context
            .create_component(
                document(),
                ActionMenu::new().trigger_icon(Icon::Add, "添加"),
            )
            .unwrap();
        let id = menu.stable_id();
        assert!(matches!(
            context.world().standard_visual(id),
            Some(StandardVisual::MenuSurface {
                trigger: Some(trigger),
                trigger_icon: Some(icon),
                ..
            }) if trigger.as_ref() == "添加" && icon == Icon::Add
        ));
        // The label stays the accessible name only; the icon trigger measures
        // no text and its hit target is the square trigger box.
        assert!(context.world().text(id).is_none_or(|text| text.is_empty()));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.layout.min_width, Some(LengthSpec::Px(TRIGGER_HEIGHT)));
        assert_eq!(
            style.layout.min_height,
            Some(LengthSpec::Px(TRIGGER_HEIGHT))
        );
    }

    /// The icon trigger's glyph must center geometrically in the chrome, not
    /// ride text line metrics — bare symbols ride high inside their em box.
    #[test]
    fn an_icon_trigger_centers_its_glyph_in_the_chrome() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        let id = StableNodeId::new(1).unwrap();
        queue.create(
            id,
            document(),
            NodeKind::Element {
                tag: "action-menu".into(),
            },
        );
        queue.write_layout(
            id,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: TRIGGER_HEIGHT,
                height: TRIGGER_HEIGHT,
            },
        );
        queue.set_style(id, trigger_icon_button_style());
        queue.set_standard_visual(
            id,
            Some(StandardVisual::MenuSurface {
                kind: MenuSurfaceKind::ActionMenu,
                open: false,
                trigger: None,
                trigger_icon: Some(Icon::Add),
                gap: 0.0,
                query: None,
                rows: Arc::from([]),
                highlighted: None,
            }),
        );
        world.commit(queue).unwrap();
        world.resolve_styles(&[id]).unwrap();
        let Some(ComponentGeometry::MenuSurface {
            trigger,
            trigger_icon,
            trigger_surface,
            ..
        }) = world.component_geometry(id)
        else {
            panic!("expected menu surface geometry");
        };
        assert!(trigger.is_none(), "icon trigger carries no text region");
        let (icon, icon_bounds) = trigger_icon.expect("trigger glyph box");
        assert_eq!(icon, Icon::Add);
        let chrome = trigger_surface.expect("trigger chrome").bounds;
        assert_eq!(
            icon_bounds.x + icon_bounds.width / 2.0,
            chrome.x + chrome.width / 2.0
        );
        assert_eq!(
            icon_bounds.y + icon_bounds.height / 2.0,
            chrome.y + chrome.height / 2.0
        );
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

    /// An open menu is a viewport-basis surface: its host row keeps the exact
    /// boxes it had while the menu was closed, instead of growing by the
    /// surface height.
    #[test]
    fn an_open_menu_stops_participating_in_its_host_layout() {
        let mut context = AppContext::new();
        let row = context
            .create_component(document(), crate::Stack::row(8.0))
            .unwrap();
        let sibling = context
            .create_component(document(), crate::Button::new("侧边"))
            .unwrap();
        let slot = context
            .create_component(document(), crate::Stack::column(4.0))
            .unwrap();
        let menu = context
            .create_component(document(), ActionMenu::new().trigger("询问"))
            .unwrap();
        context.append_child(row, sibling).unwrap();
        context.append_child(row, slot).unwrap();
        context.append_child(slot, menu).unwrap();
        context.layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();
        let closed_slot = context.world().layout_box(slot.stable_id()).unwrap();
        let closed_sibling = context.world().layout_box(sibling.stable_id()).unwrap();

        context
            .update_component(menu, |menu, _| {
                menu.popover.open = true;
            })
            .unwrap();
        let item = context
            .create_component(document(), crate::ActionMenuItem::new("只读"))
            .unwrap();
        context.append_child(menu, item).unwrap();
        context.layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();

        let open_menu = context.world().layout_box(menu.stable_id()).unwrap();
        let open_slot = context.world().layout_box(slot.stable_id()).unwrap();
        let style = context.world().node_style(menu.stable_id()).unwrap();
        assert_eq!(style.layout.position, PositionSpec::Fixed);
        // The surface hangs below the slot it is mounted in (Bottom + Start).
        assert!((open_menu.y - (closed_slot.y + closed_slot.height + ACTION_MENU_GAP)).abs() < 1.0);
        assert!((open_menu.x - closed_slot.x).abs() < 1.0);
        // The slot collapses once the surface leaves the flow, but neighbours
        // keep their closed-layout boxes: nothing else is pushed around.
        assert!(open_slot.height < closed_slot.height);
        assert_eq!(
            context.world().layout_box(sibling.stable_id()).unwrap(),
            closed_sibling
        );
    }

    /// `Top` placement anchors the surface above its slot: the bottom edge
    /// sits one gap above the slot top, so bottom-of-window triggers open
    /// upwards.
    #[test]
    fn top_placement_opens_the_surface_above_its_slot() {
        let mut context = AppContext::new();
        let row = context
            .create_component(document(), crate::Stack::row(8.0))
            .unwrap();
        let slot = context
            .create_component(document(), crate::Stack::column(4.0))
            .unwrap();
        let menu = context
            .create_component(
                document(),
                ActionMenu::new()
                    .trigger("工作树")
                    .placement(PopoverPlacement::Top),
            )
            .unwrap();
        context.append_child(row, slot).unwrap();
        context.append_child(slot, menu).unwrap();
        context.layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();
        let closed_slot = context.world().layout_box(slot.stable_id()).unwrap();

        context
            .update_component(menu, |menu, _| {
                menu.popover.open = true;
            })
            .unwrap();
        let item = context
            .create_component(document(), crate::ActionMenuItem::new("当前仓库"))
            .unwrap();
        context.append_child(menu, item).unwrap();
        context.layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();

        let open_menu = context.world().layout_box(menu.stable_id()).unwrap();
        assert!(open_menu.y < closed_slot.y, "surface opens above the slot");
        assert!(
            ((open_menu.y + open_menu.height) - (closed_slot.y - ACTION_MENU_GAP)).abs() < 1.0,
            "surface bottom clears the slot top by one gap"
        );
    }

    /// The `Menu` button kind paints the trigger chrome so an app-built menu
    /// button and an in-place action-menu trigger read as one control.
    #[test]
    fn menu_button_kind_paints_menu_trigger_chrome() {
        let mut context = AppContext::new();
        let button = context
            .create_component(
                document(),
                crate::Button::new("询问").kind(nana_ui_core::ButtonKind::Menu),
            )
            .unwrap();
        let style = context.world().node_style(button.stable_id()).unwrap();
        assert_eq!(
            style.background,
            Some(nana_ui_core::SemanticColorRole::Subtle)
        );
        assert_eq!(
            style.border,
            Some(nana_ui_core::SemanticColorRole::BorderSoft)
        );
        assert_eq!(
            style.interaction.hovered.background,
            Some(nana_ui_core::SemanticColorRole::Hover)
        );
    }
}
