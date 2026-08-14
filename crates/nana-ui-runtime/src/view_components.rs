use std::fmt;
use std::sync::Arc;

use crate::{
    AccessibilityRole, AccessibilityState, InteractionState, MutationQueue, NodeKind, NodeStyle,
    OverlayHostState, SemanticPaint, StableNodeId, StandardVisual, TextContent,
    TextHorizontalAlignment, TextInputState, TextVerticalAlignment, UiWorld,
};

fn control_layout(horizontal_padding: f32) -> Arc<nana_ui_core::LayoutStyle> {
    Arc::new(nana_ui_core::LayoutStyle {
        padding_left: Some(nana_ui_core::LengthSpec::Px(horizontal_padding)),
        padding_right: Some(nana_ui_core::LengthSpec::Px(horizontal_padding)),
        min_height: Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.control_height,
        )),
        border_width: Some(1.0),
        border_radius: Some(6.0),
        ..nana_ui_core::LayoutStyle::default()
    })
}

fn text_field_style(multiline: bool) -> NodeStyle {
    let mut layout = (*control_layout(nana_ui_core::UI_METRICS.field_padding_x)).clone();
    if multiline {
        layout.min_height = Some(nana_ui_core::LengthSpec::Px(96.0));
    }
    NodeStyle {
        layout: Arc::new(layout),
        background: Some(nana_ui_core::SemanticColorRole::Surface),
        border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
        interaction: crate::InteractionStyle {
            hovered: SemanticPaint {
                border: Some(nana_ui_core::SemanticColorRole::Accent),
                ..SemanticPaint::default()
            },
            focused: SemanticPaint {
                border: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                ..SemanticPaint::default()
            },
            disabled: SemanticPaint {
                foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                background: Some(nana_ui_core::SemanticColorRole::Subtle),
                border: Some(nana_ui_core::SemanticColorRole::Border),
            },
            ..crate::InteractionStyle::default()
        },
        text_vertical_alignment: if multiline {
            TextVerticalAlignment::Top
        } else {
            TextVerticalAlignment::Center
        },
        ..NodeStyle::default()
    }
}

struct TextFieldProjection<'a> {
    state: &'a TextInputState,
    label: &'a Option<Arc<str>>,
    disabled: bool,
    multiline: bool,
    style: &'a NodeStyle,
}

fn project_text_field(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    field: TextFieldProjection<'_>,
) {
    if world.text_input(id) != Some(field.state) {
        mutations.set_text_input(id, Some(field.state.clone()));
    }
    project_common(
        id,
        world,
        mutations,
        field.style,
        InteractionState {
            pointer_events: !field.disabled,
            focusable: !field.disabled,
        },
        AccessibilityState {
            role: AccessibilityRole::TextInput,
            label: field.label.clone(),
            disabled: field.disabled,
            multiline: field.multiline,
            editable: true,
            ..AccessibilityState::default()
        },
    );
}

/// A Nana-native component projects its state into the retained runtime. The
/// backend consumes the resulting UiWorld/UiScene data; no renderer type is
/// part of this contract.
pub trait ComponentView: Clone + Send + 'static {
    fn node_kind(&self) -> NodeKind;
    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue);
}

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub value: String,
    pub style: NodeStyle,
}

impl Text {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            style: NodeStyle::default(),
        }
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Text {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Text
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let text = TextContent {
            value: self.value.clone(),
        };
        if world.text(id) != Some(text.value.as_str()) {
            mutations.set_text(id, text);
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub label: String,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl Button {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            disabled: false,
            style: NodeStyle {
                layout: control_layout(nana_ui_core::UI_METRICS.control_padding_x),
                foreground: Some(nana_ui_core::SemanticColorRole::AccentText),
                background: Some(nana_ui_core::SemanticColorRole::Accent),
                border: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                interaction: crate::InteractionStyle {
                    hovered: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                        ..SemanticPaint::default()
                    },
                    pressed: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Active),
                        ..SemanticPaint::default()
                    },
                    focused: SemanticPaint {
                        border: Some(nana_ui_core::SemanticColorRole::AccentText),
                        ..SemanticPaint::default()
                    },
                    disabled: SemanticPaint {
                        foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                        background: Some(nana_ui_core::SemanticColorRole::Subtle),
                        border: Some(nana_ui_core::SemanticColorRole::Border),
                    },
                    ..crate::InteractionStyle::default()
                },
                text_horizontal_alignment: TextHorizontalAlignment::Center,
                text_vertical_alignment: TextVerticalAlignment::Center,
            },
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Button {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "button".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let text = TextContent {
            value: self.label.clone(),
        };
        if world.text(id) != Some(text.value.as_str()) {
            mutations.set_text(id, text);
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Compact action whose visible glyph is independent from its accessible name.
#[derive(Debug, Clone, PartialEq)]
pub struct IconButton {
    pub glyph: String,
    pub label: Arc<str>,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl IconButton {
    pub fn new(glyph: impl Into<String>, label: impl Into<Arc<str>>) -> Self {
        let mut style = Button::new("").style;
        let layout = Arc::make_mut(&mut style.layout);
        layout.padding_left = Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.compact_control_padding_x,
        ));
        layout.padding_right = layout.padding_left;
        layout.min_width = Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.icon_button_size,
        ));
        layout.min_height = layout.min_width;
        Self {
            glyph: glyph.into(),
            label: label.into(),
            disabled: false,
            style,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for IconButton {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "icon-button".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.glyph.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.glyph.clone(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: Some(Arc::clone(&self.label)),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Non-interactive content surface. Actions belong to explicit child controls.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

impl Card {
    pub fn new() -> Self {
        Self {
            label: None,
            style: NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    padding_left: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.panel_padding_x,
                    )),
                    padding_right: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.panel_padding_x,
                    )),
                    padding_top: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.panel_padding_y,
                    )),
                    padding_bottom: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.panel_padding_y,
                    )),
                    border_width: Some(1.0),
                    border_radius: Some(8.0),
                    ..nana_ui_core::LayoutStyle::default()
                }),
                background: Some(nana_ui_core::SemanticColorRole::Surface),
                border: Some(nana_ui_core::SemanticColorRole::Border),
                ..NodeStyle::default()
            },
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for Card {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "card".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState::default(),
            AccessibilityState {
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub label: String,
    pub selected: bool,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl ListItem {
    pub fn new(label: impl Into<String>) -> Self {
        let mut layout = (*control_layout(nana_ui_core::UI_METRICS.list_item_padding_x)).clone();
        layout.min_height = Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::UI_METRICS.selection_height,
        ));
        Self {
            label: label.into(),
            selected: false,
            disabled: false,
            style: NodeStyle {
                layout: Arc::new(layout),
                background: Some(nana_ui_core::SemanticColorRole::Surface),
                interaction: crate::InteractionStyle {
                    selected: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Selected),
                        ..SemanticPaint::default()
                    },
                    hovered: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Hover),
                        ..SemanticPaint::default()
                    },
                    pressed: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Active),
                        ..SemanticPaint::default()
                    },
                    focused: SemanticPaint {
                        border: Some(nana_ui_core::SemanticColorRole::Accent),
                        ..SemanticPaint::default()
                    },
                    disabled: SemanticPaint {
                        foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                        background: Some(nana_ui_core::SemanticColorRole::Subtle),
                        ..SemanticPaint::default()
                    },
                    ..crate::InteractionStyle::default()
                },
                text_vertical_alignment: TextVerticalAlignment::Center,
                ..NodeStyle::default()
            },
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for ListItem {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "list-item".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.clone(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::ListItem,
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled,
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChanged {
    pub value: String,
    pub selection: crate::TextSelection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCellFocused {
    pub row: usize,
    pub column: usize,
    pub cell: StableNodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabSelected {
    pub tab: StableNodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleChanged {
    pub checked: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SliderChanged {
    pub value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxes {
    Horizontal,
    Vertical,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollChanged {
    pub offset: crate::ScrollOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayChanged {
    pub active: Option<StableNodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderError {
    NonFinite,
    InvalidRange,
    OutOfRange,
}

impl fmt::Display for SliderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "slider values must be finite",
            Self::InvalidRange => "slider minimum must be less than maximum",
            Self::OutOfRange => "slider value must be within its range",
        })
    }
}

impl std::error::Error for SliderError {}

#[derive(Debug, Clone, PartialEq)]
pub struct TextInput {
    pub state: TextInputState,
    pub label: Option<Arc<str>>,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            state: TextInputState::new(value),
            label: None,
            disabled: false,
            style: text_field_style(false),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn replace_selection(&mut self, text: &str) -> bool {
        self.state.replace_selection(text)
    }
}

impl ComponentView for TextInput {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "input".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_text_field(
            id,
            world,
            mutations,
            TextFieldProjection {
                state: &self.state,
                label: &self.label,
                disabled: self.disabled,
                multiline: false,
                style: &self.style,
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextArea {
    pub state: TextInputState,
    pub label: Option<Arc<str>>,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl TextArea {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            state: TextInputState::new(value),
            label: None,
            disabled: false,
            style: text_field_style(true),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn replace_selection(&mut self, text: &str) -> bool {
        self.state.replace_selection(text)
    }
}

impl ComponentView for TextArea {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "textarea".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_text_field(
            id,
            world,
            mutations,
            TextFieldProjection {
                state: &self.state,
                label: &self.label,
                disabled: self.disabled,
                multiline: true,
                style: &self.style,
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OverlayHost {
    pub style: NodeStyle,
}

impl OverlayHost {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ComponentView for OverlayHost {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "overlay-host".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.overlay_host(id).is_none() {
            mutations.set_overlay_host(id, OverlayHostState::default());
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState::default(),
            AccessibilityState::default(),
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    pub title: Arc<str>,
    pub size: nana_ui_core::DialogSize,
    pub close_policy: nana_ui_core::DialogClosePolicy,
    pub style: NodeStyle,
}

impl Dialog {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        let size = nana_ui_core::DialogSize::default();
        Self {
            title: title.into(),
            size,
            close_policy: nana_ui_core::DialogClosePolicy::default(),
            style: overlay_surface_style(size.max_width()),
        }
    }

    pub fn size(mut self, size: nana_ui_core::DialogSize) -> Self {
        self.size = size;
        self.style = overlay_surface_style(size.max_width());
        self
    }

    pub fn close_policy(mut self, policy: nana_ui_core::DialogClosePolicy) -> Self {
        self.close_policy = policy;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Dialog {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "dialog".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: AccessibilityRole::Dialog,
                label: Some(Arc::clone(&self.title)),
                modal: true,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

impl Menu {
    pub fn new() -> Self {
        Self {
            label: None,
            style: overlay_surface_style(320.0),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl Default for Menu {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for Menu {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "menu".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Menu,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl MenuItem {
    pub fn new(label: impl Into<String>) -> Self {
        let label = label.into();
        let button = Button::new(label.clone());
        Self {
            label,
            disabled: false,
            style: button.style,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for MenuItem {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "menuitem".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.clone(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::MenuItem,
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tooltip {
    pub label: Arc<str>,
    pub style: NodeStyle,
}

impl Tooltip {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            style: overlay_surface_style(320.0),
        }
    }
}

impl ComponentView for Tooltip {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "tooltip".into(),
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
            &self.style,
            InteractionState::default(),
            AccessibilityState {
                role: AccessibilityRole::Tooltip,
                label: Some(Arc::clone(&self.label)),
                ..AccessibilityState::default()
            },
        );
    }
}

fn overlay_surface_style(max_width: f32) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            position: nana_ui_core::PositionSpec::Fixed,
            max_width: Some(nana_ui_core::LengthSpec::Px(max_width)),
            padding_left: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::UI_METRICS.panel_padding_x,
            )),
            padding_right: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::UI_METRICS.panel_padding_x,
            )),
            padding_top: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::UI_METRICS.panel_padding_y,
            )),
            padding_bottom: Some(nana_ui_core::LengthSpec::Px(
                nana_ui_core::UI_METRICS.panel_padding_y,
            )),
            border_width: Some(1.0),
            border_radius: Some(nana_ui_core::UI_METRICS.radius_md),
            z_index: Some(1_000),
            ..nana_ui_core::LayoutStyle::default()
        }),
        background: Some(nana_ui_core::SemanticColorRole::Surface),
        border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
        ..NodeStyle::default()
    }
}

fn toggle_style() -> NodeStyle {
    NodeStyle {
        foreground: Some(nana_ui_core::SemanticColorRole::Text),
        background: Some(nana_ui_core::SemanticColorRole::Surface),
        border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
        interaction: crate::InteractionStyle {
            selected: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Accent),
                border: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                ..SemanticPaint::default()
            },
            selected_hovered: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                ..SemanticPaint::default()
            },
            selected_pressed: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Active),
                ..SemanticPaint::default()
            },
            hovered: SemanticPaint {
                border: Some(nana_ui_core::SemanticColorRole::Accent),
                ..SemanticPaint::default()
            },
            pressed: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Active),
                ..SemanticPaint::default()
            },
            focused: SemanticPaint {
                border: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                ..SemanticPaint::default()
            },
            disabled: SemanticPaint {
                foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                background: Some(nana_ui_core::SemanticColorRole::Subtle),
                border: Some(nana_ui_core::SemanticColorRole::Border),
            },
        },
        text_vertical_alignment: TextVerticalAlignment::Center,
        ..NodeStyle::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Checkbox {
    pub label: String,
    pub checked: bool,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl Checkbox {
    pub fn new(label: impl Into<String>, checked: bool) -> Self {
        Self {
            label: label.into(),
            checked,
            disabled: false,
            style: toggle_style(),
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Checkbox {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "checkbox".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.clone(),
                },
            );
        }
        let visual = StandardVisual::Checkbox {
            checked: self.checked,
        };
        if world.standard_visual(id) != Some(visual) {
            mutations.set_standard_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Checkbox,
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled,
                checked: Some(self.checked),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    pub label: String,
    pub checked: bool,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl Switch {
    pub fn new(label: impl Into<String>, checked: bool) -> Self {
        Self {
            label: label.into(),
            checked,
            disabled: false,
            style: toggle_style(),
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Switch {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "switch".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.clone(),
                },
            );
        }
        let visual = StandardVisual::Switch {
            checked: self.checked,
        };
        if world.standard_visual(id) != Some(visual) {
            mutations.set_standard_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Switch,
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled,
                checked: Some(self.checked),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Slider {
    pub value: f32,
    pub minimum: f32,
    pub maximum: f32,
    pub label: Option<Arc<str>>,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl Slider {
    pub fn new(value: f32, minimum: f32, maximum: f32) -> Result<Self, SliderError> {
        if !value.is_finite() || !minimum.is_finite() || !maximum.is_finite() {
            return Err(SliderError::NonFinite);
        }
        if minimum >= maximum {
            return Err(SliderError::InvalidRange);
        }
        if !(minimum..=maximum).contains(&value) {
            return Err(SliderError::OutOfRange);
        }
        Ok(Self {
            value,
            minimum,
            maximum,
            label: None,
            disabled: false,
            style: NodeStyle {
                background: Some(nana_ui_core::SemanticColorRole::Accent),
                border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
                interaction: crate::InteractionStyle {
                    hovered: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                        ..SemanticPaint::default()
                    },
                    focused: SemanticPaint {
                        border: Some(nana_ui_core::SemanticColorRole::Accent),
                        ..SemanticPaint::default()
                    },
                    disabled: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Faint),
                        border: Some(nana_ui_core::SemanticColorRole::Border),
                        ..SemanticPaint::default()
                    },
                    ..crate::InteractionStyle::default()
                },
                ..NodeStyle::default()
            },
        })
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn ratio(&self) -> f32 {
        (self.value - self.minimum) / (self.maximum - self.minimum)
    }
}

impl ComponentView for Slider {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "slider".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::Slider {
            ratio: self.ratio(),
        };
        if world.standard_visual(id) != Some(visual) {
            mutations.set_standard_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Slider,
                label: self.label.clone(),
                value: Some(Arc::from(self.value.to_string())),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TabList {
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

impl TabList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for TabList {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "tablist".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::TabList,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub label: String,
    pub selected: bool,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl Tab {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            selected: false,
            disabled: false,
            style: NodeStyle {
                layout: control_layout(nana_ui_core::UI_METRICS.selection_padding_x),
                foreground: Some(nana_ui_core::SemanticColorRole::Muted),
                background: Some(nana_ui_core::SemanticColorRole::Surface),
                border: Some(nana_ui_core::SemanticColorRole::Border),
                interaction: crate::InteractionStyle {
                    selected: SemanticPaint {
                        foreground: Some(nana_ui_core::SemanticColorRole::Text),
                        background: Some(nana_ui_core::SemanticColorRole::Selected),
                        border: Some(nana_ui_core::SemanticColorRole::Accent),
                    },
                    selected_hovered: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::SelectedHover),
                        ..SemanticPaint::default()
                    },
                    selected_pressed: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::SelectedPressed),
                        ..SemanticPaint::default()
                    },
                    hovered: SemanticPaint {
                        foreground: Some(nana_ui_core::SemanticColorRole::Text),
                        background: Some(nana_ui_core::SemanticColorRole::Hover),
                        ..SemanticPaint::default()
                    },
                    pressed: SemanticPaint {
                        background: Some(nana_ui_core::SemanticColorRole::Active),
                        ..SemanticPaint::default()
                    },
                    focused: SemanticPaint {
                        border: Some(nana_ui_core::SemanticColorRole::AccentStrong),
                        ..SemanticPaint::default()
                    },
                    disabled: SemanticPaint {
                        foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                        background: Some(nana_ui_core::SemanticColorRole::Subtle),
                        border: Some(nana_ui_core::SemanticColorRole::BorderSoft),
                    },
                },
                text_horizontal_alignment: TextHorizontalAlignment::Center,
                text_vertical_alignment: TextVerticalAlignment::Center,
            },
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Tab {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "tab".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.clone(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Tab,
                label: Some(Arc::from(self.label.as_str())),
                disabled: self.disabled,
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScrollView {
    pub axes: ScrollAxes,
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

impl ScrollView {
    pub fn new(axes: ScrollAxes) -> Self {
        Self {
            axes,
            label: None,
            style: NodeStyle::default(),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn projected_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        if matches!(self.axes, ScrollAxes::Horizontal | ScrollAxes::Both) {
            layout.overflow_x = nana_ui_core::OverflowSpec::Scroll;
        }
        if matches!(self.axes, ScrollAxes::Vertical | ScrollAxes::Both) {
            layout.overflow_y = nana_ui_core::OverflowSpec::Scroll;
        }
        style
    }
}

impl ComponentView for ScrollView {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "scroll".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.projected_style(),
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct List {
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

impl List {
    pub fn new() -> Self {
        Self {
            label: None,
            style: NodeStyle::default(),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl Default for List {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for List {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "list".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::List,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

impl Table {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for Table {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "table".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Table,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableRow {
    pub selected: bool,
    pub style: NodeStyle,
}

impl TableRow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for TableRow {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "tr".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Row,
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    pub value: String,
    pub column_header: bool,
    pub selected: bool,
    pub style: NodeStyle,
}

impl TableCell {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            column_header: false,
            selected: false,
            style: NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    padding_left: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.list_item_padding_x,
                    )),
                    padding_right: Some(nana_ui_core::LengthSpec::Px(
                        nana_ui_core::UI_METRICS.list_item_padding_x,
                    )),
                    ..nana_ui_core::LayoutStyle::default()
                }),
                text_vertical_alignment: TextVerticalAlignment::Center,
                ..NodeStyle::default()
            },
        }
    }

    pub fn column_header(mut self, column_header: bool) -> Self {
        self.column_header = column_header;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for TableCell {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: if self.column_header { "th" } else { "td" }.into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.value.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.value.clone(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: if self.column_header {
                    AccessibilityRole::ColumnHeader
                } else {
                    AccessibilityRole::Cell
                },
                label: Some(Arc::from(self.value.as_str())),
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

fn project_common(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    style: &NodeStyle,
    interaction: InteractionState,
    accessibility: AccessibilityState,
) {
    if world.node_style(id) != Some(style) {
        mutations.set_style(id, style.clone());
    }
    if world.interaction(id) != Some(interaction) {
        mutations.set_interaction(id, interaction);
    }
    if world.accessibility(id) != Some(&accessibility) {
        mutations.set_accessibility(id, accessibility);
    }
}
