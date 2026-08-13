use std::sync::{Arc, LazyLock};

use bevy_ecs::component::Component;
use nana_ui_core::{LayoutStyle, LineHeightSpec, SemanticColorRole, UI_BASE_TEXT_SIZE};

use crate::{NodeKind, StableNodeId};

static DEFAULT_LAYOUT_STYLE: LazyLock<Arc<LayoutStyle>> =
    LazyLock::new(|| Arc::new(LayoutStyle::default()));

#[derive(Component, Debug, Clone, PartialEq)]
pub struct NodeStyle {
    pub layout: Arc<LayoutStyle>,
    pub foreground: Option<SemanticColorRole>,
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
        }
    }
}

#[derive(Component, Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub foreground: SemanticColorRole,
    pub color: Option<[f32; 4]>,
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

pub trait TextShaper {
    fn shape(&mut self, id: StableNodeId, text: &TextContent, style: &ComputedStyle)
    -> TextMetrics;
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
    TabList,
    Tab,
    Dialog,
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
    pub focused: bool,
    pub bounds: LayoutBox,
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
    pub source_style: NodeStyle,
    pub style: ComputedStyle,
    pub text: Option<TextContent>,
    pub text_metrics: Option<TextMetrics>,
    pub z_index: i32,
    pub focused: bool,
    pub ime: Option<ImeComposition>,
    pub text_input: Option<TextInputState>,
    pub custom_render: Option<CustomRenderNode>,
}
