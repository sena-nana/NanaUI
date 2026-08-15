use std::sync::Arc;

use nana_ui_core::{
    AnchoredMenuPlacement, ControlSize, Icon, LengthSpec, PositionSpec, SemanticColorRole,
    SemanticPalette, UI_METRICS,
};

use crate::popover::{menu_surface_style, project_anchored_menu};
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentGeometry, ComponentTextRegion, ComputedStyle,
    InteractionState, InteractionStyle, LayoutBox, MenuSurfaceKind, MutationQueue, NodeKind,
    NodeStyle, SemanticPaint, StableNodeId, StandardVisual, TextContent, TextVerticalAlignment,
    UiWorld,
};

const MENU_WIDTH: f32 = 200.0;
const MENU_PADDING: f32 = 4.0;
const MENU_MIN_WIDTH: f32 = 120.0;
const MENU_MIN_HEIGHT: f32 = 32.0;
const ICON_GAP: f32 = 8.0;

/// Selectable row shared by action menus and context menus.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionMenuItem {
    pub label: Arc<str>,
    pub hint: Option<Arc<str>>,
    pub leading: Option<Icon>,
    pub size: ControlSize,
    pub active: bool,
    pub danger: bool,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl ActionMenuItem {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        let size = ControlSize::Small;
        Self {
            label: label.into(),
            hint: None,
            leading: None,
            size,
            active: false,
            danger: false,
            disabled: false,
            style: item_style(size),
        }
    }

    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        let hint = hint.into();
        self.hint = (!hint.is_empty()).then_some(hint);
        self
    }

    pub fn leading(mut self, leading: Icon) -> Self {
        self.leading = Some(leading);
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self.style = item_style(size);
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(if self.disabled {
            SemanticColorRole::Faint
        } else if self.danger {
            SemanticColorRole::Danger
        } else {
            SemanticColorRole::Text
        });
        style.background = if self.active && self.danger {
            Some(SemanticColorRole::AccentSoft)
        } else if self.active {
            Some(SemanticColorRole::Hover)
        } else {
            None
        };
        style.interaction.hovered.background = Some(if self.danger {
            SemanticColorRole::AccentSoft
        } else {
            SemanticColorRole::Hover
        });
        style.interaction.pressed.background = Some(if self.danger {
            SemanticColorRole::AccentSoftPressed
        } else {
            SemanticColorRole::Active
        });
        style.interaction.selected.background = style.background;
        style
    }
}

impl crate::ComponentView for ActionMenuItem {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "action-menu-item".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::ActionMenuItem {
            label: Arc::clone(&self.label),
            hint: self.hint.clone(),
            icon: self.leading,
            danger: self.danger,
            active: self.active,
            disabled: self.disabled,
            size: self.size,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
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
            &self.effective_style(),
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::MenuItem,
                label: Some(Arc::clone(&self.label)),
                description: self.hint.clone(),
                disabled: self.disabled,
                selected: Some(self.active),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Viewport-level menu surface pinned to a logical anchor.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchoredActionMenu {
    pub open: bool,
    pub x: f32,
    pub y: f32,
    pub placement: AnchoredMenuPlacement,
    pub width: f32,
    pub height: f32,
    pub style: NodeStyle,
}

impl AnchoredActionMenu {
    pub fn new(x: f32, y: f32) -> Self {
        let mut menu = Self {
            open: true,
            x,
            y,
            placement: AnchoredMenuPlacement::BottomStart,
            width: MENU_WIDTH,
            height: 0.0,
            style: menu_surface_style(MENU_WIDTH, MENU_PADDING),
        };
        menu.apply_anchor();
        menu
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn placement(mut self, placement: AnchoredMenuPlacement) -> Self {
        self.placement = placement;
        self.apply_anchor();
        self
    }

    pub fn menu_size(mut self, width: f32, height: f32) -> Self {
        self.width = width.max(MENU_MIN_WIDTH);
        self.height = if height >= MENU_MIN_HEIGHT {
            height
        } else {
            0.0
        };
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.width = Some(LengthSpec::Px(self.width));
        if self.height > 0.0 {
            layout.height = Some(LengthSpec::Px(self.height));
        } else {
            layout.height = None;
        }
        self.apply_anchor();
        self
    }

    pub fn place_in(&mut self, viewport: LayoutBox) {
        let origin = resolve_anchored_origin(
            self.x,
            self.y,
            self.width,
            self.height,
            viewport,
            self.placement,
        );
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.offset_left = Some(LengthSpec::Px(origin.0));
        layout.offset_top = Some(LengthSpec::Px(origin.1));
    }

    fn apply_anchor(&mut self) {
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.position = PositionSpec::Fixed;
        layout.offset_left = Some(LengthSpec::Px(self.x));
        layout.offset_top = Some(LengthSpec::Px(self.y));
        layout.height = (self.height > 0.0).then_some(LengthSpec::Px(self.height));
    }
}

impl crate::ComponentView for AnchoredActionMenu {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "anchored-action-menu".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_anchored_menu(
            id,
            world,
            mutations,
            MenuSurfaceKind::ActionMenu,
            &self.style,
            self.open,
            "menu",
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMenuItem {
    pub value: Arc<str>,
    pub label: Arc<str>,
    pub hint: Option<Arc<str>>,
    pub icon: Option<Icon>,
    pub disabled: bool,
    pub danger: bool,
}

impl ContextMenuItem {
    pub fn new(value: impl Into<Arc<str>>, label: impl Into<Arc<str>>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            hint: None,
            icon: None,
            disabled: false,
            danger: false,
        }
    }

    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        let hint = hint.into();
        self.hint = (!hint.is_empty()).then_some(hint);
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextMenuEvent {
    Select(Arc<str>),
    Dismiss,
}

/// Pointer-anchored menu. Nested search stays on the Iced host for this batch.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenu {
    pub items: Vec<ContextMenuItem>,
    pub open: bool,
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub highlighted: Option<usize>,
    pub width: f32,
    pub style: NodeStyle,
}

impl ContextMenu {
    pub fn new(anchor_x: f32, anchor_y: f32) -> Self {
        let mut menu = Self {
            items: Vec::new(),
            open: true,
            anchor_x,
            anchor_y,
            highlighted: None,
            width: MENU_WIDTH,
            style: menu_surface_style(MENU_WIDTH, MENU_PADDING),
        };
        menu.apply_anchor();
        menu
    }

    pub fn items(mut self, items: impl IntoIterator<Item = ContextMenuItem>) -> Self {
        self.items = items.into_iter().collect();
        self.apply_anchor();
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(MENU_MIN_WIDTH);
        Arc::make_mut(&mut self.style.layout).width = Some(LengthSpec::Px(self.width));
        self
    }

    pub fn place_in(&mut self, viewport: LayoutBox) {
        let height = context_menu_height(self.items.len());
        let origin = resolve_anchored_origin(
            self.anchor_x,
            self.anchor_y,
            self.width,
            height,
            viewport,
            AnchoredMenuPlacement::BottomStart,
        );
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.offset_left = Some(LengthSpec::Px(origin.0));
        layout.offset_top = Some(LengthSpec::Px(origin.1));
        layout.height = Some(LengthSpec::Px(height));
    }

    pub fn select_index(&mut self, index: usize) -> Option<ContextMenuEvent> {
        let item = self.items.get(index)?;
        if item.disabled || !self.open {
            return None;
        }
        let value = Arc::clone(&item.value);
        self.open = false;
        self.highlighted = None;
        Some(ContextMenuEvent::Select(value))
    }

    pub fn dismiss(&mut self) {
        self.open = false;
        self.highlighted = None;
    }

    fn apply_anchor(&mut self) {
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.position = PositionSpec::Fixed;
        layout.offset_left = Some(LengthSpec::Px(self.anchor_x));
        layout.offset_top = Some(LengthSpec::Px(self.anchor_y));
        layout.height = Some(LengthSpec::Px(context_menu_height(self.items.len())));
    }
}

impl crate::ComponentView for ContextMenu {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "context-menu".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_anchored_menu(
            id,
            world,
            mutations,
            MenuSurfaceKind::ContextMenu,
            &self.style,
            self.open,
            "context-menu",
        );
    }
}

pub(crate) fn action_menu_item_geometry(
    bounds: LayoutBox,
    label: &Arc<str>,
    hint: Option<&Arc<str>>,
    icon: Option<Icon>,
    danger: bool,
    disabled: bool,
    size: ControlSize,
    style: &ComputedStyle,
    palette: &SemanticPalette,
) -> ComponentGeometry {
    let pad = size.padding_x();
    let icon_size = size.icon_size();
    let mut cursor = bounds.x + pad;
    let icon = icon.map(|icon| {
        let box_ = LayoutBox {
            x: cursor,
            y: bounds.y + (bounds.height - icon_size) / 2.0,
            width: icon_size,
            height: icon_size,
        };
        cursor += icon_size + ICON_GAP;
        let color = if disabled {
            palette.faint.as_rgba_array()
        } else if danger {
            palette.danger.as_rgba_array()
        } else {
            palette.muted.as_rgba_array()
        };
        (icon, box_, color)
    });
    let hint_width = hint
        .map(|hint| (hint.len() as f32) * size.text_size() * 0.45)
        .unwrap_or(0.0);
    let hint_gap = if hint.is_some() { ICON_GAP } else { 0.0 };
    let label_right = bounds.x + bounds.width - pad - hint_width - hint_gap;
    let foreground = if disabled {
        palette.faint.as_rgba_array()
    } else if danger {
        palette.danger.as_rgba_array()
    } else {
        style.color.unwrap_or_else(|| palette.text.as_rgba_array())
    };
    ComponentGeometry::ActionMenuItem {
        icon,
        label: ComponentTextRegion {
            bounds: LayoutBox {
                x: cursor,
                y: bounds.y,
                width: (label_right - cursor).max(0.0),
                height: bounds.height,
            },
            content: Arc::clone(label),
            color: Some(foreground),
            font_size: size.text_size(),
            font_weight: Some(500),
        },
        hint: hint.map(|hint| ComponentTextRegion {
            bounds: LayoutBox {
                x: label_right + hint_gap,
                y: bounds.y,
                width: hint_width,
                height: bounds.height,
            },
            content: Arc::clone(hint),
            color: Some(palette.muted.as_rgba_array()),
            font_size: 11.0,
            font_weight: None,
        }),
        background: style.background,
    }
}

pub fn resolve_anchored_origin(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    viewport: LayoutBox,
    placement: AnchoredMenuPlacement,
) -> (f32, f32) {
    let mut origin = match placement {
        AnchoredMenuPlacement::TopStart => (x, y - height),
        AnchoredMenuPlacement::TopEnd => (x - width, y - height),
        AnchoredMenuPlacement::BottomStart => (x, y),
        AnchoredMenuPlacement::BottomEnd => (x - width, y),
    };
    let max_x = (viewport.x + viewport.width - width).max(viewport.x);
    let max_y = (viewport.y + viewport.height - height).max(viewport.y);
    origin.0 = origin.0.clamp(viewport.x, max_x);
    origin.1 = origin.1.clamp(viewport.y, max_y);
    origin
}

fn context_menu_height(item_count: usize) -> f32 {
    let count = item_count.max(1) as f32;
    let item = ControlSize::Small.height();
    MENU_PADDING * 2.0 + count * item + (count - 1.0).max(0.0)
}

fn item_style(size: ControlSize) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            width: Some(LengthSpec::Fill),
            height: Some(LengthSpec::Px(size.height())),
            padding_left: Some(LengthSpec::Px(size.padding_x())),
            padding_right: Some(LengthSpec::Px(size.padding_x())),
            border_radius: Some(UI_METRICS.radius_sm),
            ..nana_ui_core::LayoutStyle::default()
        }),
        foreground: Some(SemanticColorRole::Text),
        interaction: InteractionStyle {
            hovered: SemanticPaint {
                background: Some(SemanticColorRole::Hover),
                ..SemanticPaint::default()
            },
            pressed: SemanticPaint {
                background: Some(SemanticColorRole::Active),
                ..SemanticPaint::default()
            },
            selected: SemanticPaint {
                background: Some(SemanticColorRole::Hover),
                ..SemanticPaint::default()
            },
            disabled: SemanticPaint {
                foreground: Some(SemanticColorRole::Faint),
                ..SemanticPaint::default()
            },
            ..InteractionStyle::default()
        },
        text_vertical_alignment: TextVerticalAlignment::Center,
        ..NodeStyle::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::AppContext;
    use crate::{Activate, DocumentId};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn action_menu_item_projects_label_hint_and_danger() {
        let mut context = AppContext::new();
        let item = context
            .create_component(
                document(),
                ActionMenuItem::new("Delete")
                    .hint("⌫")
                    .danger(true)
                    .leading(Icon::Close),
            )
            .unwrap();
        let id = item.stable_id();
        assert!(matches!(
            context.world().standard_visual(id),
            Some(StandardVisual::ActionMenuItem { danger: true, .. })
        ));
        assert_eq!(context.world().text(id), Some("Delete"));
        assert_eq!(
            context.world().node_style(id).unwrap().foreground,
            Some(SemanticColorRole::Danger)
        );
        let a11y = context.world().accessibility(id).unwrap();
        assert_eq!(a11y.role, AccessibilityRole::MenuItem);
        assert_eq!(a11y.description.as_deref(), Some("⌫"));
    }

    #[test]
    fn disabled_action_menu_item_does_not_activate() {
        let mut context = AppContext::new();
        let item = context
            .create_component(document(), ActionMenuItem::new("Rename").disabled(true))
            .unwrap();
        let fired = Arc::new(std::sync::Mutex::new(false));
        let flag = Arc::clone(&fired);
        context
            .on(item, move |_item, _event: &Activate, _| {
                *flag.lock().unwrap() = true;
            })
            .unwrap();
        assert!(!context.activate_action_menu_item(item).unwrap());
        assert!(!*fired.lock().unwrap());
    }

    #[test]
    fn context_menu_selects_and_dismisses() {
        let mut menu = ContextMenu::new(24.0, 36.0).items([
            ContextMenuItem::new("rename", "Rename"),
            ContextMenuItem::new("delete", "Delete").danger(true),
        ]);
        assert!(menu.open);
        assert_eq!(
            menu.select_index(1),
            Some(ContextMenuEvent::Select(Arc::from("delete")))
        );
        assert!(!menu.open);
        menu.open = true;
        menu.dismiss();
        assert!(!menu.open);
    }

    #[test]
    fn anchored_origin_clamps_to_the_viewport() {
        let viewport = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 160.0,
        };
        assert_eq!(
            resolve_anchored_origin(
                180.0,
                140.0,
                80.0,
                60.0,
                viewport,
                AnchoredMenuPlacement::BottomStart,
            ),
            (120.0, 100.0)
        );
    }
}
