use std::sync::{Arc, LazyLock};

use bevy_ecs::component::Component;
use nana_ui_core::{LayoutStyle, LineHeightSpec, SemanticColorRole, UI_BASE_TEXT_SIZE};

use crate::{NodeKind, StableNodeId};

static DEFAULT_LAYOUT_STYLE: LazyLock<Arc<LayoutStyle>> =
    LazyLock::new(|| Arc::new(LayoutStyle::default()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SemanticPaint {
    pub foreground: Option<SemanticColorRole>,
    pub background: Option<SemanticColorRole>,
    pub border: Option<SemanticColorRole>,
}

impl SemanticPaint {
    pub fn overlay(self, overlay: Self) -> Self {
        Self {
            foreground: overlay.foreground.or(self.foreground),
            background: overlay.background.or(self.background),
            border: overlay.border.or(self.border),
        }
    }

    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InteractionStyle {
    pub selected: SemanticPaint,
    pub selected_hovered: SemanticPaint,
    pub selected_pressed: SemanticPaint,
    pub hovered: SemanticPaint,
    pub pressed: SemanticPaint,
    pub focused: SemanticPaint,
    pub disabled: SemanticPaint,
}

#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub enum StandardVisual {
    Checkbox { checked: bool },
    Switch { checked: bool },
    Slider { ratio: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextHorizontalAlignment {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextVerticalAlignment {
    #[default]
    Top,
    Center,
    Bottom,
}

impl InteractionStyle {
    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

#[derive(Component, Debug, Clone, PartialEq)]
pub struct NodeStyle {
    pub layout: Arc<LayoutStyle>,
    pub foreground: Option<SemanticColorRole>,
    pub background: Option<SemanticColorRole>,
    pub border: Option<SemanticColorRole>,
    pub interaction: InteractionStyle,
    pub text_horizontal_alignment: TextHorizontalAlignment,
    pub text_vertical_alignment: TextVerticalAlignment,
}

impl Default for NodeStyle {
    fn default() -> Self {
        Self::visible()
    }
}

impl NodeStyle {
    pub fn visible() -> Self {
        Self {
            layout: Arc::clone(&DEFAULT_LAYOUT_STYLE),
            foreground: None,
            background: None,
            border: None,
            interaction: InteractionStyle::default(),
            text_horizontal_alignment: TextHorizontalAlignment::Start,
            text_vertical_alignment: TextVerticalAlignment::Top,
        }
    }
}

#[derive(Component, Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub foreground: SemanticColorRole,
    pub color: Option<[f32; 4]>,
    pub background: Option<[f32; 4]>,
    pub border_color: Option<[f32; 4]>,
    pub opacity: f32,
    pub visible: bool,
    pub font_size: f32,
    pub font_weight: Option<u16>,
    pub font_family: Option<Arc<str>>,
    pub line_height: Option<LineHeightSpec>,
    pub letter_spacing: f32,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            foreground: SemanticColorRole::Text,
            color: None,
            background: None,
            border_color: None,
            opacity: 1.0,
            visible: true,
            font_size: UI_BASE_TEXT_SIZE,
            font_weight: None,
            font_family: None,
            line_height: None,
            letter_spacing: 0.0,
        }
    }
}

#[derive(Component, Debug, Clone, PartialEq, Default)]
pub struct TextContent {
    pub value: String,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextShapeConstraints {
    pub max_width: Option<f32>,
    pub max_height: Option<f32>,
    pub wrap: bool,
    pub ellipsis: bool,
}

pub trait TextShaper {
    fn shape(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics;
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutInput {
    pub id: StableNodeId,
    pub parent: Option<StableNodeId>,
    pub children: Vec<StableNodeId>,
    pub style: Arc<LayoutStyle>,
    pub text_metrics: Option<TextMetrics>,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollOffset {
    pub x: f32,
    pub y: f32,
}

/// Derived scrollport and content extents in logical pixels. Absence means
/// the layout backend has not measured this scroll container yet.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct ScrollMetrics {
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub content_width: f32,
    pub content_height: f32,
}

/// Exclusive overlay state attached to an overlay host. `active` must be a
/// direct child of the host; `restore_focus` remains in the same document.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OverlayHostState {
    pub active: Option<StableNodeId>,
    pub restore_focus: Option<StableNodeId>,
}

impl ScrollMetrics {
    pub fn max_offset(self) -> ScrollOffset {
        ScrollOffset {
            x: (self.content_width - self.viewport_width).max(0.0),
            y: (self.content_height - self.viewport_height).max(0.0),
        }
    }

    pub fn clamp(self, offset: ScrollOffset) -> ScrollOffset {
        let max = self.max_offset();
        ScrollOffset {
            x: offset.x.clamp(0.0, max.x),
            y: offset.y.clamp(0.0, max.y),
        }
    }
}

impl LayoutBox {
    pub fn contains(self, x: f32, y: f32) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && x >= self.x
            && y >= self.y
            && x < self.x + self.width
            && y < self.y + self.height
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionState {
    pub pointer_events: bool,
    pub focusable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccessibilityRole {
    Document,
    Text,
    Button,
    TextInput,
    Checkbox,
    Switch,
    Slider,
    ComboBox,
    ProgressIndicator,
    List,
    ListItem,
    Table,
    Row,
    Cell,
    ColumnHeader,
    TabList,
    Tab,
    Dialog,
    Menu,
    MenuItem,
    Tooltip,
    Image,
    #[default]
    Generic,
}

#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessibilityState {
    pub role: AccessibilityRole,
    pub label: Option<Arc<str>>,
    pub value: Option<Arc<str>>,
    pub disabled: bool,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub multiline: bool,
    pub editable: bool,
    pub modal: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccessibilityNode {
    pub id: StableNodeId,
    pub parent: Option<StableNodeId>,
    pub children: Vec<StableNodeId>,
    pub role: AccessibilityRole,
    pub label: Option<Arc<str>>,
    pub value: Option<Arc<str>>,
    pub disabled: bool,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub multiline: bool,
    pub editable: bool,
    pub selection: Option<TextSelection>,
    pub modal: bool,
    pub focused: bool,
    pub bounds: LayoutBox,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccessibilityDelta {
    pub generation: u64,
    pub updated: Vec<AccessibilityNode>,
    pub removed: Vec<StableNodeId>,
}

/// One platform-facing accessibility transaction.
///
/// Retained hosts normally emit [`Self::Delta`]. [`Self::Full`] is the bounded
/// fallback when a consumer has not drained changes quickly enough.
#[derive(Debug, Clone, PartialEq)]
pub enum AccessibilityUpdate {
    Full {
        /// `None` is reserved for snapshot-only hosts without retained revisions.
        generation: Option<u64>,
        nodes: Vec<AccessibilityNode>,
    },
    Delta(AccessibilityDelta),
}

/// Backend-neutral action requested by a platform accessibility service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessibilityAction {
    Click,
    Focus,
    SetValue(String),
    SetSelection(TextSelection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityActionRequest {
    pub target: StableNodeId,
    pub action: AccessibilityAction,
}

/// Stable capture/target/bubble route derived from the retained hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRoute {
    /// Ancestors from the document root down to the target's parent.
    pub capture: Vec<StableNodeId>,
    pub target: StableNodeId,
    /// Ancestors from the target's parent back to the document root.
    pub bubble: Vec<StableNodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerCaptureChange {
    pub pointer_id: u64,
    pub target: StableNodeId,
    pub captured: bool,
}

impl Default for InteractionState {
    fn default() -> Self {
        Self {
            pointer_events: true,
            focusable: false,
        }
    }
}

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct ImeComposition {
    pub text: String,
    pub selection: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextSelection {
    pub anchor: usize,
    pub focus: usize,
}

impl TextSelection {
    pub const fn caret(offset: usize) -> Self {
        Self {
            anchor: offset,
            focus: offset,
        }
    }

    pub fn ordered(self) -> std::ops::Range<usize> {
        self.anchor.min(self.focus)..self.anchor.max(self.focus)
    }

    pub fn is_valid_for(self, value: &str) -> bool {
        self.anchor <= value.len()
            && self.focus <= value.len()
            && value.is_char_boundary(self.anchor)
            && value.is_char_boundary(self.focus)
    }
}

/// Committed editable text and its selection. IME preedit remains separate in
/// [`ImeComposition`], so cancelling composition never corrupts committed text.
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct TextInputState {
    pub value: String,
    pub selection: TextSelection,
}

impl TextInputState {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let selection = TextSelection::caret(value.len());
        Self { value, selection }
    }

    pub fn replace_selection(&mut self, text: &str) -> bool {
        if !self.selection.is_valid_for(&self.value) {
            return false;
        }
        let range = self.selection.ordered();
        let caret = range.start + text.len();
        self.value.replace_range(range, text);
        self.selection = TextSelection::caret(caret);
        true
    }

    /// Replace a controlled value while keeping a valid selection when
    /// possible. If the old offsets no longer land on UTF-8 boundaries, move
    /// the caret to the new end.
    pub fn replace_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        if !self.selection.is_valid_for(&self.value) {
            self.selection = TextSelection::caret(self.value.len());
        }
    }

    /// Accept a complete value snapshot from a native editor.
    ///
    /// Native controlled editors commonly report the resulting value without
    /// exposing their internal selection. Infer the caret from the minimal
    /// changed span so retained text, accessibility, and application events
    /// commit one value without forcing every edit to the end.
    pub fn synchronize_editor_value(&mut self, value: impl Into<String>) {
        let value = value.into();
        if self.value == value {
            return;
        }
        let prefix = self
            .value
            .chars()
            .zip(value.chars())
            .take_while(|(current, next)| current == next)
            .map(|(character, _)| character.len_utf8())
            .sum::<usize>();
        let suffix = self.value[prefix..]
            .chars()
            .rev()
            .zip(value[prefix..].chars().rev())
            .take_while(|(current, next)| current == next)
            .map(|(_, character)| character.len_utf8())
            .sum::<usize>();
        let caret = value.len() - suffix;
        self.value = value;
        self.selection = TextSelection::caret(caret);
    }
}

/// Backend-neutral custom render content attached to one retained UI node.
///
/// `renderer` selects an installed renderer extension and `resource` is an
/// opaque application-owned lookup key. Neither field exposes a GPU backend
/// object, so the same extraction can be consumed by WGPU or a future RHI.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct CustomRenderNode {
    pub renderer: Arc<str>,
    pub resource: Arc<str>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedNode {
    pub id: StableNodeId,
    pub kind: NodeKind,
    pub parent: Option<StableNodeId>,
    pub children: Vec<StableNodeId>,
    pub layout: LayoutBox,
    pub scroll_offset: ScrollOffset,
    pub source_style: NodeStyle,
    pub style: ComputedStyle,
    pub text: Option<TextContent>,
    pub text_metrics: Option<TextMetrics>,
    pub z_index: i32,
    pub focused: bool,
    pub ime: Option<ImeComposition>,
    pub text_input: Option<TextInputState>,
    pub standard_visual: Option<StandardVisual>,
    pub standard_visual_foreground: Option<[f32; 4]>,
    pub custom_render: Option<CustomRenderNode>,
}
