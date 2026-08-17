use std::sync::{Arc, LazyLock};

use bevy_ecs::component::Component;
use nana_ui_core::{
    CardKind, ControlSize, Icon, LayoutStyle, LineHeightSpec, SemanticColorRole,
    SwitchControlPosition, UI_BASE_TEXT_SIZE,
};

pub(crate) fn status_tone_role(tone: nana_ui_core::StatusTone) -> SemanticColorRole {
    match tone {
        nana_ui_core::StatusTone::Neutral => SemanticColorRole::Muted,
        nana_ui_core::StatusTone::Info => SemanticColorRole::Accent,
        nana_ui_core::StatusTone::Success => SemanticColorRole::Success,
        nana_ui_core::StatusTone::Warning => SemanticColorRole::Warning,
        nana_ui_core::StatusTone::Danger => SemanticColorRole::Danger,
    }
}

use crate::{NodeKind, StableNodeId};

static DEFAULT_LAYOUT_STYLE: LazyLock<Arc<LayoutStyle>> =
    LazyLock::new(|| Arc::new(LayoutStyle::default()));

/// Effective retained-document presence for a node.
///
/// Parked nodes keep their stable identity and application-owned view state,
/// but are excluded from layout, rendering, input, focus and accessibility
/// until the same subtree is inserted again.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MountState {
    #[default]
    Mounted,
    Parked,
}

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

#[derive(Debug, Clone, PartialEq)]
pub struct TooltipVisual {
    pub label: Arc<str>,
    pub config: nana_ui_core::TooltipConfig,
    pub open: bool,
}

#[derive(Component, Debug, Clone, PartialEq)]
pub enum StandardVisual {
    ModalFrame {
        title: Arc<str>,
        description: Option<Arc<str>>,
        body_text: Option<Arc<str>>,
        kind: crate::ModalSurfaceKind,
        busy: bool,
        danger: bool,
        slots: crate::ModalSlots,
    },
    Button {
        label: Arc<str>,
        kind: nana_ui_core::ButtonKind,
        size: ControlSize,
        loading: bool,
        loading_phase: f32,
        invalid: bool,
    },
    TextInput {
        placeholder: Arc<str>,
        size: ControlSize,
        secure: bool,
        invalid: bool,
    },
    Checkbox {
        checked: bool,
    },
    Icon {
        icon: Icon,
        size: f32,
        tooltip: Option<TooltipVisual>,
    },
    Switch {
        label: Arc<str>,
        hint: Option<Arc<str>>,
        checked: bool,
        control_position: SwitchControlPosition,
        size: ControlSize,
        loading: bool,
        loading_phase: f32,
        invalid: bool,
    },
    Slider {
        ratio: f32,
    },
    Range {
        label: Option<Arc<str>>,
        value: Arc<str>,
        unit: Option<Arc<str>>,
        size: ControlSize,
        ratio: f32,
        invalid: bool,
    },
    Card {
        title: Option<Arc<str>>,
        kind: CardKind,
        loading: bool,
        loading_phase: f32,
    },
    ListItem {
        leading: Option<StableNodeId>,
        content: Option<StableNodeId>,
        trailing: Option<StableNodeId>,
    },
    StatusBadge {
        label: Arc<str>,
        tone: nana_ui_core::StatusTone,
        compact: bool,
    },
    ValidationMessage {
        message: Arc<str>,
        intent: nana_ui_core::ValidationIntent,
        compact: bool,
    },
    EmptyState {
        title: Arc<str>,
        message: Option<Arc<str>>,
        icon: Option<Icon>,
        compact: bool,
        action: Option<StableNodeId>,
    },
    LabeledValue {
        label: Arc<str>,
        value: Arc<str>,
        value_role: SemanticColorRole,
        value_weight: u16,
        compact: bool,
        action: Option<StableNodeId>,
    },
    SelectionOption {
        label: Arc<str>,
        icon: Option<Icon>,
        selected: bool,
        disabled: bool,
        size: ControlSize,
        show_focus_ring: bool,
    },
    Progress {
        value_ratio: f32,
        label: Option<Arc<str>>,
        cancellable: bool,
    },
    Spinner {
        label: Arc<str>,
        size: f32,
        phase: f32,
    },
    LevelMeter {
        value_ratio: f32,
        girth: f32,
        tone: nana_ui_core::StatusTone,
    },
    FormField {
        label: Arc<str>,
        hint: Option<Arc<str>>,
        error: Option<Arc<str>>,
        size: ControlSize,
        control: Option<StableNodeId>,
    },
    QrCode {
        modules: Arc<[bool]>,
        width: usize,
    },
    Toast {
        title: Arc<str>,
        description: Option<Arc<str>>,
        tone: nana_ui_core::ToastTone,
        dismissible: bool,
    },
    XYPad {
        value: nana_ui_core::XYPadValue,
        nx: f32,
        ny: f32,
        size: ControlSize,
        invalid: bool,
        disabled: bool,
    },
    Select {
        label: Arc<str>,
        placeholder: bool,
        size: ControlSize,
        opened: bool,
        invalid: bool,
        loading: bool,
        options: Arc<[SelectOptionData]>,
        highlighted: Option<usize>,
    },
    MenuSurface {
        kind: MenuSurfaceKind,
        trigger: Option<Arc<str>>,
        gap: f32,
        query: Option<Arc<str>>,
        rows: Arc<[SelectOptionData]>,
        highlighted: Option<usize>,
    },
    ActionMenuItem {
        label: Arc<str>,
        hint: Option<Arc<str>>,
        icon: Option<Icon>,
        danger: bool,
        active: bool,
        disabled: bool,
        size: ControlSize,
    },
    TreeView {
        rows: Arc<[crate::tree_view::TreeRowData]>,
        size: ControlSize,
    },
    CommandPalette {
        title: Arc<str>,
        query: Arc<str>,
        placeholder: Arc<str>,
        empty: Option<Arc<str>>,
        rows: Arc<[crate::command_palette::PaletteRowData]>,
    },
    CalendarHeatmap {
        cells: Arc<[crate::calendar::CalendarHeatmapCellPaint]>,
        month_labels: Arc<[crate::calendar::CalendarHeatmapLabelPaint]>,
        day_labels: Arc<[crate::calendar::CalendarHeatmapLabelPaint]>,
        cell_size: f32,
        cell_radius: f32,
        max_level: u8,
        active: Option<usize>,
    },
    TimeSeriesChart {
        values: Arc<[f64]>,
    },
    ReorderList {
        rows: Arc<[crate::reorder_list::ReorderRowPaint]>,
        size: ControlSize,
        spacing: f32,
        insert: Option<LayoutBox>,
    },
    NativeMarkdown {
        text: Arc<str>,
        selection: Option<(usize, usize)>,
    },
    SelectableRichText {
        text: Arc<str>,
        selection: Option<(usize, usize)>,
    },
    GraphCanvas {
        nodes: Arc<[crate::graph_canvas::GraphNodePaint]>,
        ports: Arc<[crate::graph_canvas::GraphPortPaint]>,
        edges: Arc<[crate::graph_canvas::GraphEdgePaint]>,
        connecting: Option<crate::graph_canvas::GraphEdgePaint>,
        grid_spacing: f32,
        viewport_offset_x: f32,
        viewport_offset_y: f32,
        viewport_zoom: f32,
    },
    ImageViewer {
        name: Option<Arc<str>>,
        metadata: Option<Arc<str>>,
        zoom: f32,
        offset_x: f32,
        offset_y: f32,
    },
    KeyCaptureLayer {
        recording: bool,
    },
    KeymapLayer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOptionData {
    pub label: Arc<str>,
    pub hint: Option<Arc<str>>,
    pub disabled: bool,
    pub checked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuSurfaceKind {
    Popover,
    ActionMenu,
    ContextMenu,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComponentTextRegion {
    pub bounds: LayoutBox,
    pub content: Arc<str>,
    pub color: Option<[f32; 4]>,
    pub font_size: f32,
    pub font_weight: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComponentElevation {
    pub color: [f32; 4],
    pub offset_y: f32,
    pub blur_radius: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComponentGeometry {
    ModalFrame {
        scrim: LayoutBox,
        surface: LayoutBox,
        body: LayoutBox,
        title: ComponentTextRegion,
        description: Option<ComponentTextRegion>,
        body_text: Option<ComponentTextRegion>,
        background: [f32; 4],
        border: [f32; 4],
        elevation: ComponentElevation,
    },
    Button {
        label: ComponentTextRegion,
        spinner: Option<LayoutBox>,
        background: Option<[f32; 4]>,
        border: Option<[f32; 4]>,
        border_width: f32,
        focus_ring: Option<[f32; 4]>,
    },
    TextInput {
        text: ComponentTextRegion,
        multiline: bool,
        selection: Vec<LayoutBox>,
        caret: Option<LayoutBox>,
        preedit: Vec<LayoutBox>,
        background: Option<[f32; 4]>,
        border: Option<[f32; 4]>,
        border_width: f32,
        focus_ring: Option<[f32; 4]>,
        selection_color: [f32; 4],
        caret_color: [f32; 4],
        preedit_color: [f32; 4],
    },
    Switch {
        label: ComponentTextRegion,
        hint: Option<ComponentTextRegion>,
        control: LayoutBox,
        track_background: [f32; 4],
        track_border: [f32; 4],
        thumb_background: [f32; 4],
    },
    Range {
        label: Option<ComponentTextRegion>,
        value: ComponentTextRegion,
        unit: Option<ComponentTextRegion>,
        track: LayoutBox,
    },
    Card {
        title: Option<ComponentTextRegion>,
        content: LayoutBox,
        elevation: Option<ComponentElevation>,
        spinner: Option<LayoutBox>,
    },
    ListItem {
        leading: Option<LayoutBox>,
        content: Option<LayoutBox>,
        trailing: Option<LayoutBox>,
    },
    StatusBadge {
        indicator: LayoutBox,
        label: ComponentTextRegion,
        background: [f32; 4],
        foreground: [f32; 4],
    },
    ValidationMessage {
        indicator: LayoutBox,
        label: ComponentTextRegion,
        foreground: [f32; 4],
    },
    EmptyState {
        root_clip: LayoutBox,
        content_clip: LayoutBox,
        icon: Option<(Icon, LayoutBox, [f32; 4])>,
        title: ComponentTextRegion,
        message: Option<ComponentTextRegion>,
        action: Option<LayoutBox>,
    },
    LabeledValue {
        label: ComponentTextRegion,
        value: ComponentTextRegion,
        action: Option<LayoutBox>,
    },
    SelectionOption {
        icon: Option<(Icon, LayoutBox, [f32; 4])>,
        label: ComponentTextRegion,
        focus_ring: Option<[f32; 4]>,
    },
    Progress {
        track: LayoutBox,
        fill: LayoutBox,
        label: Option<ComponentTextRegion>,
        cancel: Option<LayoutBox>,
        corner_radius: f32,
    },
    FormField {
        label: ComponentTextRegion,
        support: Option<ComponentTextRegion>,
        indicator: Option<(LayoutBox, [f32; 4])>,
        control: Option<LayoutBox>,
    },
    QrCode {
        field: LayoutBox,
        module_size: f32,
        dark: Vec<LayoutBox>,
    },
    Toast {
        indicator: LayoutBox,
        title: ComponentTextRegion,
        description: Option<ComponentTextRegion>,
        dismiss: Option<LayoutBox>,
    },
    XYPad {
        pad: LayoutBox,
        thumb: LayoutBox,
        h_axis: LayoutBox,
        v_axis: LayoutBox,
        background: Option<[f32; 4]>,
        border: Option<[f32; 4]>,
        border_width: f32,
        thumb_color: [f32; 4],
        axis_color: [f32; 4],
    },
    Select {
        label: ComponentTextRegion,
        handle: LayoutBox,
        handle_color: [f32; 4],
        background: Option<[f32; 4]>,
        border: Option<[f32; 4]>,
        border_width: f32,
        menu: Option<SelectMenuGeometry>,
    },
    MenuSurface {
        trigger: Option<ComponentTextRegion>,
        surface: LayoutBox,
        search: Option<ComponentTextRegion>,
        search_field: Option<LayoutBox>,
        options: Vec<SelectOptionGeometry>,
        elevation: ComponentElevation,
        background: [f32; 4],
        border: [f32; 4],
    },
    ActionMenuItem {
        icon: Option<(Icon, LayoutBox, [f32; 4])>,
        label: ComponentTextRegion,
        hint: Option<ComponentTextRegion>,
        background: Option<[f32; 4]>,
    },
    TreeView {
        rows: Vec<crate::tree_view::TreeRowGeometry>,
    },
    CommandPalette {
        scrim: LayoutBox,
        surface: LayoutBox,
        title: ComponentTextRegion,
        input: ComponentTextRegion,
        empty: Option<ComponentTextRegion>,
        rows: Vec<crate::command_palette::PaletteRowGeometry>,
        background: [f32; 4],
        input_background: [f32; 4],
        input_border: [f32; 4],
        elevation: ComponentElevation,
    },
    CalendarHeatmap {
        cells: Vec<(LayoutBox, [f32; 4])>,
        labels: Vec<ComponentTextRegion>,
    },
    TimeSeriesChart {
        grid: Vec<LayoutBox>,
        area: Vec<LayoutBox>,
        line: Vec<LayoutBox>,
        grid_color: [f32; 4],
        area_color: [f32; 4],
        line_color: [f32; 4],
    },
    ReorderList {
        rows: Vec<(LayoutBox, ComponentTextRegion, Option<[f32; 4]>)>,
        insert: Option<(LayoutBox, [f32; 4])>,
    },
    NativeMarkdown {
        text: ComponentTextRegion,
        selection: Vec<LayoutBox>,
        selection_color: [f32; 4],
    },
    SelectableRichText {
        text: ComponentTextRegion,
        selection: Vec<LayoutBox>,
        selection_color: [f32; 4],
    },
    GraphCanvas {
        nodes: Vec<(LayoutBox, ComponentTextRegion, [f32; 4], Option<[f32; 4]>)>,
        separators: Vec<LayoutBox>,
        ports: Vec<(LayoutBox, [f32; 4], [f32; 4], f32)>,
        port_labels: Vec<(ComponentTextRegion, crate::TextHorizontalAlignment)>,
        edges: Vec<(LayoutBox, [f32; 4])>,
        edge_labels: Vec<ComponentTextRegion>,
        grid: Vec<LayoutBox>,
        background: [f32; 4],
        grid_color: [f32; 4],
        separator_color: [f32; 4],
    },
    ImageViewer {
        scrim: LayoutBox,
        surface: LayoutBox,
        stage: LayoutBox,
        close: LayoutBox,
        name: Option<ComponentTextRegion>,
        metadata: Option<ComponentTextRegion>,
        content: LayoutBox,
        scrim_color: [f32; 4],
        surface_color: [f32; 4],
        stage_color: [f32; 4],
    },
    KeyCaptureLayer {
        badge: ComponentTextRegion,
        background: Option<[f32; 4]>,
    },
    KeymapLayer {
        badge: ComponentTextRegion,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectMenuGeometry {
    pub surface: LayoutBox,
    pub elevation: ComponentElevation,
    pub background: [f32; 4],
    pub border: [f32; 4],
    pub options: Vec<SelectOptionGeometry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectOptionGeometry {
    pub bounds: LayoutBox,
    pub label: ComponentTextRegion,
    pub selected: bool,
    pub checked: bool,
    pub disabled: bool,
    pub background: Option<[f32; 4]>,
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

/// Shaped intrinsic text owned by an EmptyState rather than application child
/// nodes. Runtime layout and every renderer consume the same measured runs.
#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct EmptyStateTextPresentation {
    pub title: TextMetrics,
    pub message: Option<TextMetrics>,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct ModalTextPresentation {
    pub title: TextMetrics,
    pub description: Option<TextMetrics>,
    pub body: Option<TextMetrics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextShapeConstraints {
    pub max_width: Option<f32>,
    pub max_height: Option<f32>,
    pub wrap: bool,
    pub ellipsis: bool,
    pub shaping: TextShaping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextShaping {
    #[default]
    Auto,
    Advanced,
}

pub trait TextShaper {
    fn shape(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics;

    /// Return the exact horizontal position of a UTF-8 boundary in one
    /// unwrapped line. Backends should override this with their paragraph
    /// engine; the prefix-shaped default keeps lightweight test shapers valid.
    fn horizontal_offset(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        style: &ComputedStyle,
    ) -> f32 {
        if byte_offset > text.value.len() || !text.value.is_char_boundary(byte_offset) {
            return 0.0;
        }
        self.shape(
            id,
            &TextContent {
                value: text.value[..byte_offset].to_owned(),
            },
            style,
            TextShapeConstraints {
                shaping: TextShaping::Advanced,
                ..TextShapeConstraints::default()
            },
        )
        .width
    }

    /// Return the visual origin of a UTF-8 boundary in multiline text.
    ///
    /// The backend-neutral fallback handles explicit line breaks and delegates
    /// each line's horizontal shaping to [`Self::horizontal_offset`]. Backends
    /// with paragraph-level wrapping information may override this method.
    fn text_position(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        style: &ComputedStyle,
        _constraints: TextShapeConstraints,
    ) -> (f32, f32, f32) {
        if byte_offset > text.value.len()
            || !text.value.is_char_boundary(byte_offset)
            || !is_grapheme_boundary(&text.value, byte_offset)
        {
            return (0.0, 0.0, 0.0);
        }
        let (line, line_start, line_end) = explicit_line_at(&text.value, byte_offset);
        let line_height = resolved_text_line_height(style);
        let line_text = TextContent {
            value: text.value[line_start..line_end].to_owned(),
        };
        (
            self.horizontal_offset(id, &line_text, byte_offset - line_start, style),
            line as f32 * line_height,
            line_height,
        )
    }

    /// Return highlight rectangles in paragraph-local coordinates.
    ///
    /// The default preserves the backend-neutral explicit-newline behavior.
    /// Paragraph backends should override this to split ranges at their real
    /// visual wrapping boundaries.
    fn text_highlights(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        selection: (usize, usize),
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> Vec<LayoutBox> {
        let (start, end) = selection;
        if start >= end
            || end > text.value.len()
            || !text.value.is_char_boundary(start)
            || !text.value.is_char_boundary(end)
            || !is_grapheme_boundary(&text.value, start)
            || !is_grapheme_boundary(&text.value, end)
        {
            return Vec::new();
        }
        let mut highlights = Vec::new();
        for (line_start, line_end, next_line_start) in explicit_lines(&text.value) {
            let segment_start = start.max(line_start).min(line_end);
            let segment_end = end.min(line_end).max(line_start);
            let includes_line_break =
                next_line_start > line_end && start <= line_end && end >= next_line_start;
            if segment_start < segment_end || includes_line_break {
                let (from_x, from_y, line_height) =
                    self.text_position(id, text, segment_start, style, constraints);
                let (to_x, _, _) = self.text_position(id, text, segment_end, style, constraints);
                highlights.push(LayoutBox {
                    x: from_x,
                    y: from_y,
                    width: (to_x - from_x).max(if includes_line_break { 1.0 } else { 0.0 }),
                    height: line_height,
                });
            }
            if line_end == text.value.len() || line_end >= end {
                break;
            }
        }
        highlights
    }
}

fn explicit_lines(value: &str) -> Vec<(usize, usize, usize)> {
    use unicode_segmentation::UnicodeSegmentation;

    let mut lines = Vec::new();
    let mut start = 0;
    for (index, grapheme) in value.grapheme_indices(true) {
        if matches!(grapheme, "\n" | "\r" | "\r\n" | "\n\r") {
            lines.push((start, index, index + grapheme.len()));
            start = index + grapheme.len();
        }
    }
    lines.push((start, value.len(), value.len()));
    lines
}

fn explicit_line_at(value: &str, offset: usize) -> (usize, usize, usize) {
    let lines = explicit_lines(value);
    let last_line = lines.len().saturating_sub(1);
    lines
        .into_iter()
        .enumerate()
        .find_map(|(line, (start, end, next))| {
            (offset <= end || offset < next).then_some((line, start, end))
        })
        .unwrap_or((last_line, value.len(), value.len()))
}

fn resolved_text_line_height(style: &ComputedStyle) -> f32 {
    match style.line_height {
        Some(LineHeightSpec::Absolute(value)) => value.max(0.0),
        Some(LineHeightSpec::Relative(value)) => style.font_size * value.max(0.0),
        None => style.font_size * 1.2,
    }
}

/// Shaped editing presentation. The committed value remains in
/// [`TextInputState`]; this derived component only carries renderer geometry.
#[derive(Component, Debug, Clone, PartialEq, Default)]
pub struct TextInputPresentation {
    pub display_value: String,
    pub placeholder: bool,
    pub selection: Option<(f32, f32)>,
    pub selection_lines: Vec<LayoutBox>,
    pub caret_x: f32,
    pub caret_y: f32,
    pub line_height: f32,
    pub preedit: Option<(f32, f32)>,
    pub preedit_lines: Vec<LayoutBox>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutInput {
    pub id: StableNodeId,
    pub parent: Option<StableNodeId>,
    pub children: Vec<StableNodeId>,
    pub style: Arc<LayoutStyle>,
    pub text_metrics: Option<TextMetrics>,
    pub modal: Option<ModalLayoutInput>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModalLayoutInput {
    pub kind: crate::ModalSurfaceKind,
    pub slots: crate::ModalSlots,
    pub title: TextMetrics,
    pub description: Option<TextMetrics>,
    pub body_text: Option<TextMetrics>,
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
    RadioGroup,
    Radio,
    Dialog,
    AlertDialog,
    Menu,
    MenuItem,
    Tooltip,
    Image,
    #[default]
    Generic,
}

#[derive(Component, Debug, Clone, PartialEq, Default)]
pub struct AccessibilityState {
    pub role: AccessibilityRole,
    pub label: Option<Arc<str>>,
    pub value: Option<Arc<str>>,
    pub description: Option<Arc<str>>,
    pub disabled: bool,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub multiline: bool,
    pub editable: bool,
    pub modal: bool,
    pub busy: bool,
    pub invalid: bool,
    pub numeric_minimum: Option<f64>,
    pub numeric_maximum: Option<f64>,
    pub numeric_step: Option<f64>,
    pub numeric_value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccessibilityNode {
    pub id: StableNodeId,
    pub parent: Option<StableNodeId>,
    pub children: Vec<StableNodeId>,
    pub role: AccessibilityRole,
    pub label: Option<Arc<str>>,
    pub value: Option<Arc<str>>,
    pub description: Option<Arc<str>>,
    pub disabled: bool,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub multiline: bool,
    pub editable: bool,
    pub selection: Option<TextSelection>,
    pub modal: bool,
    pub busy: bool,
    pub invalid: bool,
    pub numeric_minimum: Option<f64>,
    pub numeric_maximum: Option<f64>,
    pub numeric_step: Option<f64>,
    pub numeric_value: Option<f64>,
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
            && is_grapheme_boundary(value, self.anchor)
            && is_grapheme_boundary(value, self.focus)
    }
}

fn is_grapheme_boundary(value: &str, offset: usize) -> bool {
    use unicode_segmentation::UnicodeSegmentation;

    offset == value.len()
        || value
            .grapheme_indices(true)
            .any(|(boundary, _)| boundary == offset)
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
    pub text_spans: Vec<ExtractedTextSpan>,
    pub standard_visual: Option<StandardVisual>,
    pub component_geometry: Option<ComponentGeometry>,
    pub standard_visual_foreground: Option<[f32; 4]>,
    pub custom_render: Option<CustomRenderNode>,
}

/// Theme-resolved committed-text span ready for Scene paint.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedTextSpan {
    pub start: usize,
    pub end: usize,
    pub color: [f32; 4],
}
