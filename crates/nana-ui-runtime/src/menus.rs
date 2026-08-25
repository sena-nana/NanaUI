use std::sync::Arc;

use nana_ui_core::{
    AnchoredMenuPlacement, ControlSize, Icon, LengthSpec, PositionSpec, SemanticColorRole,
    SemanticPalette, UI_METRICS,
};

use crate::popover::{menu_surface_style, project_anchored_menu};
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentElevation, ComponentGeometry,
    ComponentTextRegion, ComputedStyle, InteractionState, InteractionStyle, LayoutBox,
    MenuSurfaceKind, MutationQueue, NodeKind, NodeStyle, SelectOptionData, SemanticPaint,
    StableNodeId, StandardVisual, TextContent, TextInputState, TextVerticalAlignment, UiWorld,
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
    Search(Arc<str>),
    Dismiss,
}

/// Pointer-anchored menu. Slash-separated values (`parent/child`) are a tree;
/// [`Self::query`] filters the current level or matching leaves. Searchable
/// menus own a committed [`TextInputState`] for the filter field.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenu {
    pub items: Vec<ContextMenuItem>,
    pub open: bool,
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub highlighted: Option<usize>,
    pub width: f32,
    pub style: NodeStyle,
    pub active_path: Vec<Arc<str>>,
    pub query: Arc<str>,
    pub searchable: bool,
    pub state: TextInputState,
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
            active_path: Vec::new(),
            query: Arc::from(""),
            searchable: false,
            state: TextInputState::new(""),
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

    pub fn active_path(mut self, path: impl IntoIterator<Item = impl Into<Arc<str>>>) -> Self {
        self.active_path = path.into_iter().map(Into::into).collect();
        self.apply_anchor();
        self
    }

    pub fn query(mut self, query: impl Into<Arc<str>>) -> Self {
        let query = query.into();
        self.query = Arc::clone(&query);
        self.state = TextInputState::new(query.as_ref());
        self.apply_anchor();
        self
    }

    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self.apply_anchor();
        self
    }

    pub fn set_query(&mut self, query: impl Into<Arc<str>>) {
        let query = query.into();
        self.query = Arc::clone(&query);
        self.state = TextInputState::new(query.as_ref());
        self.apply_anchor();
    }

    pub(crate) fn sync_query_from_state(&mut self) {
        self.query = Arc::from(self.state.value.as_str());
        self.apply_anchor();
    }

    /// Rows at the current nested level, or matching leaves when `query` is set.
    pub fn visible_items(&self) -> Vec<ContextMenuItem> {
        if self.query.trim().is_empty() {
            self.current_level_items()
        } else {
            self.matching_leaves()
        }
    }

    pub fn place_in(&mut self, viewport: LayoutBox) {
        let height = context_menu_height(self.visible_items().len(), self.searchable);
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
        let item = self.visible_items().get(index)?.clone();
        if item.disabled || !self.open {
            return None;
        }
        if self.has_descendants(&item.value) {
            self.active_path = value_segments(&item.value)
                .into_iter()
                .map(Arc::from)
                .collect();
            self.highlighted = None;
            self.apply_anchor();
            return None;
        }
        self.open = false;
        self.highlighted = None;
        self.active_path.clear();
        Some(ContextMenuEvent::Select(item.value))
    }

    /// Pop the current nested level. Returns `true` when the path changed.
    pub fn back(&mut self) -> bool {
        if self.active_path.pop().is_none() {
            return false;
        }
        self.highlighted = None;
        self.apply_anchor();
        true
    }

    pub fn dismiss(&mut self) {
        self.open = false;
        self.highlighted = None;
        self.active_path.clear();
    }

    fn apply_anchor(&mut self) {
        let height = context_menu_height(self.visible_items().len(), self.searchable);
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.position = PositionSpec::Fixed;
        layout.offset_left = Some(LengthSpec::Px(self.anchor_x));
        layout.offset_top = Some(LengthSpec::Px(self.anchor_y));
        layout.height = Some(LengthSpec::Px(height));
    }

    fn current_level_items(&self) -> Vec<ContextMenuItem> {
        let prefix: Vec<&str> = self.active_path.iter().map(|part| part.as_ref()).collect();
        let mut level: Vec<(String, ContextMenuItem)> = Vec::new();
        for item in &self.items {
            let segments = value_segments(&item.value);
            if segments.len() <= prefix.len() || segments[..prefix.len()] != prefix {
                continue;
            }
            let next = segments[prefix.len()];
            let next_value = if prefix.is_empty() {
                next.to_string()
            } else {
                format!("{}/{next}", prefix.join("/"))
            };
            let is_exact = segments.len() == prefix.len() + 1;
            if let Some((_, existing)) = level.iter_mut().find(|(key, _)| key == &next_value) {
                if is_exact {
                    *existing = item.clone();
                }
            } else if is_exact {
                level.push((next_value, item.clone()));
            } else {
                level.push((
                    next_value.clone(),
                    ContextMenuItem::new(Arc::<str>::from(next_value), Arc::<str>::from(next)),
                ));
            }
        }
        level.into_iter().map(|(_, item)| item).collect()
    }

    fn matching_leaves(&self) -> Vec<ContextMenuItem> {
        self.items
            .iter()
            .filter(|item| {
                !self.has_descendants(&item.value) && item_matches_query(item, &self.query)
            })
            .cloned()
            .collect()
    }

    fn has_descendants(&self, value: &str) -> bool {
        let prefix = value_segments(value);
        self.items.iter().any(|item| {
            let segments = value_segments(&item.value);
            segments.len() > prefix.len() && segments[..prefix.len()] == prefix
        })
    }
}

impl crate::ComponentView for ContextMenu {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "context-menu".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let rows: Arc<[SelectOptionData]> = self
            .visible_items()
            .into_iter()
            .map(|item| SelectOptionData {
                label: item.label,
                hint: item.hint,
                disabled: item.disabled,
                checked: false,
                icon: item.icon,
            })
            .collect();
        if self.open {
            let visual = StandardVisual::MenuSurface {
                kind: MenuSurfaceKind::ContextMenu,
                open: true,
                trigger: None,
                gap: 0.0,
                query: self.searchable.then(|| Arc::clone(&self.query)),
                rows,
                highlighted: self.highlighted,
            };
            if world.standard_visual(id) != Some(visual.clone()) {
                mutations.set_standard_visual(id, Some(visual));
            }
        } else if world.standard_visual(id).is_some() {
            mutations.set_standard_visual(id, None);
        }
        if self.searchable && self.open {
            if world.text_input(id) != Some(&self.state) {
                mutations.set_text_input(id, Some(self.state.clone()));
            }
        } else if world.text_input(id).is_some() {
            mutations.set_text_input(id, None);
        }
        let mut style = self.style.clone();
        Arc::make_mut(&mut style.layout).hidden = !self.open;
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: self.open,
                focusable: self.open && self.searchable,
            },
            AccessibilityState {
                role: AccessibilityRole::Menu,
                label: Some(Arc::from("context-menu")),
                value: (self.searchable && !self.query.is_empty()).then(|| Arc::clone(&self.query)),
                editable: self.searchable && self.open,
                ..AccessibilityState::default()
            },
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
    let icon_color = if disabled {
        palette.faint.as_rgba_array()
    } else if danger {
        palette.danger.as_rgba_array()
    } else {
        palette.muted.as_rgba_array()
    };
    let (cursor, icon) = menu_option_icon(bounds, icon, size, icon_color);
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

const SEARCH_FIELD_HEIGHT: f32 = 28.0;
const SEARCH_FIELD_GAP: f32 = 4.0;

fn context_menu_height(item_count: usize, searchable: bool) -> f32 {
    let count = item_count.max(1) as f32;
    let item = ControlSize::Small.height();
    let list = MENU_PADDING * 2.0 + count * item + (count - 1.0).max(0.0);
    if searchable {
        list + SEARCH_FIELD_HEIGHT + SEARCH_FIELD_GAP
    } else {
        list
    }
}

pub(crate) fn context_menu_geometry(
    bounds: LayoutBox,
    query: Option<&Arc<str>>,
    rows: &[SelectOptionData],
    highlighted: Option<usize>,
    palette: &SemanticPalette,
) -> ComponentGeometry {
    let is_light = palette.background.as_rgba_array()[0] > 0.5;
    let searchable = query.is_some();
    let search_field = searchable.then(|| LayoutBox {
        x: bounds.x + MENU_PADDING,
        y: bounds.y + MENU_PADDING,
        width: (bounds.width - MENU_PADDING * 2.0).max(0.0),
        height: SEARCH_FIELD_HEIGHT,
    });
    let search = search_field.map(|field| {
        let empty = query.is_none_or(|query| query.trim().is_empty());
        ComponentTextRegion {
            bounds: field,
            content: if empty {
                Arc::from("搜索操作")
            } else {
                Arc::clone(query.expect("searchable query"))
            },
            color: Some(if empty {
                palette.faint.as_rgba_array()
            } else {
                palette.text.as_rgba_array()
            }),
            font_size: ControlSize::Small.text_size(),
            font_weight: None,
        }
    });
    let list_top = if searchable {
        bounds.y + MENU_PADDING + SEARCH_FIELD_HEIGHT + SEARCH_FIELD_GAP
    } else {
        bounds.y + MENU_PADDING
    };
    let item_height = ControlSize::Small.height();
    let size = ControlSize::Small;
    let options = rows
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let y = list_top + index as f32 * (item_height + 1.0);
            let selected = highlighted == Some(index);
            let row = LayoutBox {
                x: bounds.x + MENU_PADDING,
                y,
                width: (bounds.width - MENU_PADDING * 2.0).max(0.0),
                height: item_height,
            };
            let icon_color = if option.disabled {
                palette.faint.as_rgba_array()
            } else {
                palette.muted.as_rgba_array()
            };
            let (label_x, icon) = menu_option_icon(row, option.icon, size, icon_color);
            let label_right = row.x + row.width - size.padding_x();
            crate::SelectOptionGeometry {
                bounds: row,
                label: ComponentTextRegion {
                    bounds: LayoutBox {
                        x: label_x,
                        y: row.y,
                        width: (label_right - label_x).max(0.0),
                        height: row.height,
                    },
                    content: crate::select::menu_option_label(option),
                    color: Some(if option.disabled {
                        palette.faint.as_rgba_array()
                    } else {
                        palette.text.as_rgba_array()
                    }),
                    font_size: size.text_size(),
                    font_weight: Some(500),
                },
                selected,
                checked: option.checked,
                disabled: option.disabled,
                background: selected.then_some(palette.hover.as_rgba_array()),
                icon,
            }
        })
        .collect();
    ComponentGeometry::MenuSurface {
        trigger_surface: None,
        trigger: None,
        surface: bounds,
        search,
        search_field,
        options,
        elevation: ComponentElevation {
            color: [0.0, 0.0, 0.0, if is_light { 0.30 } else { 0.55 }],
            offset_y: 4.0,
            blur_radius: if is_light { 14.0 } else { 18.0 },
            spread_radius: 0.0,
        },
        background: palette.surface.as_rgba_array(),
        border: palette.border_soft.as_rgba_array(),
    }
}

pub(crate) fn context_menu_option_at(
    geometry: &ComponentGeometry,
    x: f32,
    y: f32,
) -> Option<usize> {
    let ComponentGeometry::MenuSurface { options, .. } = geometry else {
        return None;
    };
    options
        .iter()
        .position(|option| !option.disabled && option.bounds.contains(x, y))
}

fn menu_option_icon(
    row: LayoutBox,
    icon: Option<Icon>,
    size: ControlSize,
    color: [f32; 4],
) -> (f32, Option<(Icon, LayoutBox, [f32; 4])>) {
    let icon_size = size.icon_size();
    let mut cursor = row.x + size.padding_x();
    let icon = icon.map(|icon| {
        let bounds = LayoutBox {
            x: cursor,
            y: row.y + (row.height - icon_size) / 2.0,
            width: icon_size,
            height: icon_size,
        };
        cursor += icon_size + ICON_GAP;
        (icon, bounds, color)
    });
    (cursor, icon)
}

fn value_segments(value: &str) -> Vec<&str> {
    value
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn item_matches_query(item: &ContextMenuItem, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    item.label.to_lowercase().contains(&query)
        || item.value.to_lowercase().contains(&query)
        || item
            .hint
            .as_ref()
            .is_some_and(|hint| hint.to_lowercase().contains(&query))
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
    use crate::{Activate, DocumentId, StandardVisual};

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
        assert!(menu.active_path.is_empty());
    }

    #[test]
    fn context_menu_opens_nested_child_and_selects_leaf() {
        let mut menu = ContextMenu::new(24.0, 36.0).items([
            ContextMenuItem::new("file", "File"),
            ContextMenuItem::new("file/rename", "Rename"),
            ContextMenuItem::new("file/delete", "Delete"),
            ContextMenuItem::new("edit", "Edit"),
        ]);
        let visible: Vec<_> = menu
            .visible_items()
            .into_iter()
            .map(|item| item.value)
            .collect();
        assert_eq!(
            visible,
            [Arc::<str>::from("file"), Arc::<str>::from("edit")]
        );
        assert_eq!(menu.select_index(0), None);
        assert!(menu.open);
        assert_eq!(menu.active_path.as_slice(), &[Arc::<str>::from("file")]);
        let visible: Vec<_> = menu
            .visible_items()
            .into_iter()
            .map(|item| item.value)
            .collect();
        assert_eq!(
            visible,
            [
                Arc::<str>::from("file/rename"),
                Arc::<str>::from("file/delete")
            ]
        );
        assert_eq!(
            menu.select_index(0),
            Some(ContextMenuEvent::Select(Arc::from("file/rename")))
        );
        assert!(!menu.open);
        assert!(menu.active_path.is_empty());

        menu.open = true;
        assert_eq!(menu.select_index(0), None);
        assert!(menu.back());
        assert!(menu.active_path.is_empty());
        assert!(!menu.back());
        assert_eq!(
            menu.select_index(1),
            Some(ContextMenuEvent::Select(Arc::from("edit")))
        );
    }

    #[test]
    fn context_menu_query_hides_non_matching_items() {
        let menu = ContextMenu::new(24.0, 36.0)
            .items([
                ContextMenuItem::new("file/rename", "Rename").hint("Ctrl+R"),
                ContextMenuItem::new("file/delete", "Delete"),
                ContextMenuItem::new("edit", "Edit"),
            ])
            .searchable(true)
            .query("REN");
        let visible: Vec<_> = menu
            .visible_items()
            .into_iter()
            .map(|item| item.value)
            .collect();
        assert_eq!(visible, [Arc::<str>::from("file/rename")]);

        let by_hint = menu.clone().query("ctrl");
        assert_eq!(
            by_hint
                .visible_items()
                .into_iter()
                .map(|item| item.value)
                .collect::<Vec<_>>(),
            [Arc::<str>::from("file/rename")]
        );

        let empty = menu.clone().query("");
        let root: Vec<_> = empty
            .visible_items()
            .into_iter()
            .map(|item| item.value)
            .collect();
        assert_eq!(root, [Arc::<str>::from("file"), Arc::<str>::from("edit")]);
        assert!(matches!(
            empty.style.layout.height,
            Some(LengthSpec::Px(full)) if matches!(
                menu.style.layout.height,
                Some(LengthSpec::Px(filtered)) if filtered < full
            )
        ));
        let Some(LengthSpec::Px(searchable_height)) = menu.style.layout.height else {
            panic!("searchable menu height");
        };
        assert!(searchable_height > context_menu_height(1, false));
    }

    #[test]
    fn context_menu_projects_item_icons_into_menu_surface_rows() {
        let mut context = AppContext::new();
        let menu = context
            .create_component(
                document(),
                ContextMenu::new(8.0, 12.0).items([
                    ContextMenuItem::new("add", "Add").icon(Icon::Add),
                    ContextMenuItem::new("rename", "Rename"),
                ]),
            )
            .unwrap();
        match context.world().standard_visual(menu.stable_id()) {
            Some(StandardVisual::MenuSurface { rows, .. }) => {
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].icon, Some(Icon::Add));
                assert_eq!(rows[1].icon, None);
            }
            other => panic!("context menu visual: {other:?}"),
        }
    }

    #[test]
    fn context_menu_geometry_reserves_icon_box_and_keeps_iconless_rows() {
        let palette = SemanticPalette::for_mode(nana_ui_core::ThemeMode::Dark);
        let rows = [
            SelectOptionData {
                label: Arc::from("Add"),
                hint: None,
                disabled: false,
                checked: false,
                icon: Some(Icon::Add),
            },
            SelectOptionData {
                label: Arc::from("Rename"),
                hint: None,
                disabled: false,
                checked: false,
                icon: None,
            },
        ];
        let ComponentGeometry::MenuSurface { options, .. } = context_menu_geometry(
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 80.0,
            },
            None,
            &rows,
            None,
            &palette,
        ) else {
            panic!("context menu geometry");
        };
        assert_eq!(options.len(), 2);
        let Some((Icon::Add, icon_box, _)) = options[0].icon else {
            panic!("expected Add icon geometry");
        };
        assert!(icon_box.width > 0.0 && icon_box.height > 0.0);
        assert!(options[0].label.bounds.x >= icon_box.x + icon_box.width);
        assert!(options[0].label.bounds.width > 0.0);
        assert_eq!(options[1].icon, None);
        assert!(options[1].label.bounds.width > 0.0);
        assert!(options[1].label.bounds.height > 0.0);
        assert!(options[1].label.bounds.x < options[0].label.bounds.x);
    }

    #[test]
    fn searchable_context_menu_projects_query_state_and_rows() {
        let mut context = AppContext::new();
        let menu = context
            .create_component(
                document(),
                ContextMenu::new(8.0, 12.0)
                    .items([
                        ContextMenuItem::new("rename", "Rename"),
                        ContextMenuItem::new("delete", "Delete"),
                    ])
                    .searchable(true)
                    .query("del"),
            )
            .unwrap();
        let id = menu.stable_id();
        assert_eq!(
            context
                .world()
                .text_input(id)
                .map(|state| state.value.as_str()),
            Some("del")
        );
        assert!(context.world().interaction(id).unwrap().focusable);
        match context.world().standard_visual(id) {
            Some(StandardVisual::MenuSurface {
                query: Some(query),
                rows,
                ..
            }) => {
                assert_eq!(query.as_ref(), "del");
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].label.as_ref(), "Delete");
            }
            other => panic!("searchable menu visual: {other:?}"),
        }
    }

    #[test]
    fn context_menu_disabled_items_stay_unselectable() {
        let mut menu = ContextMenu::new(24.0, 36.0).items([
            ContextMenuItem::new("file", "File"),
            ContextMenuItem::new("file/rename", "Rename").disabled(true),
            ContextMenuItem::new("paste", "Paste").disabled(true),
            ContextMenuItem::new("delete", "Delete").danger(true),
        ]);
        assert_eq!(menu.select_index(1), None);
        assert!(menu.open);
        assert!(menu.active_path.is_empty());

        assert_eq!(menu.select_index(0), None);
        assert_eq!(menu.active_path.as_slice(), &[Arc::<str>::from("file")]);
        assert_eq!(menu.select_index(0), None);
        assert!(menu.open);
        assert_eq!(menu.active_path.as_slice(), &[Arc::<str>::from("file")]);
        assert!(menu.back());

        assert_eq!(
            menu.select_index(2),
            Some(ContextMenuEvent::Select(Arc::from("delete")))
        );
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
