use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};

use nana_ui_core::{
    CardKind, ControlSize, FontFeatureSetting, FontKerningSpec, FontVariationSetting, Icon,
    LayoutStyle, LineBreakSpec, LineHeightSpec, SemanticColorRole, SwitchControlPosition,
    UI_BASE_TEXT_SIZE, WordBreakSpec,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

/// 诊断标记的严重级别（编辑器下划线颜色随之变化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDiagnosticSeverity {
    Error,
    Warning,
}

/// 编辑器诊断 span。`offset`/`length` 为字节偏移，宿主负责在文本变化后
/// 更新或清除；越界部分在几何计算时被钳制。
#[derive(Debug, Clone, PartialEq)]
pub struct TextDiagnosticSpan {
    pub offset: usize,
    pub length: usize,
    pub severity: TextDiagnosticSeverity,
}

impl TextDiagnosticSpan {
    pub fn new(offset: usize, length: usize, severity: TextDiagnosticSeverity) -> Self {
        Self {
            offset,
            length,
            severity,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
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
        /// Numeric spinner affordance at the trailing edge. `NumberInput` is a
        /// text input that also steps, so it reuses this visual instead of
        /// growing a second editable contract.
        steppers: bool,
        /// 代码编辑器扩展：编译诊断 span 标记。偏移由宿主维护——文本变化后
        /// 由宿主更新或清除，渲染层仅做越界钳制，不做偏移迁移。
        diagnostics: Arc<[TextDiagnosticSpan]>,
        /// 行号栏。行号绘制在节点左内边距区域，宿主需预留足够的 padding。
        line_numbers: bool,
    },
    Checkbox {
        checked: bool,
        /// Mixed state. Wins over `checked` when painting and in a11y.
        indeterminate: bool,
        size: ControlSize,
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
    Range {
        label: Option<Arc<str>>,
        value: Arc<str>,
        unit: Option<Arc<str>>,
        size: ControlSize,
        ratio: f32,
        invalid: bool,
    },
    /// Scroll container chrome. Carries policy only: the track and thumb boxes
    /// come from the authoritative [`ScrollOffset`] / [`ScrollMetrics`] at
    /// extraction time, so no scroll position is duplicated here.
    Scrollbar {
        axes: crate::ScrollAxes,
        visibility: nana_ui_core::ScrollbarVisibility,
        revealed: bool,
        dragging: Option<nana_ui_core::ScrollbarAxis>,
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
        /// 单行补充信息；几何端生成右对齐的小字号 muted 文本区。
        detail: Option<Arc<str>>,
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
        /// Draws a radio ring, and a dot when selected, before the label.
        indicator: bool,
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
        open: bool,
        trigger: Option<Arc<str>>,
        trigger_icon: Option<Icon>,
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
        active_title: Option<Arc<str>>,
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
    /// Overview minimap policy. Node rectangles and the indicator stay in
    /// world space; the uniform map projection resolves at extraction time
    /// against the final widget box.
    GraphMinimap {
        bounds: nana_ui_core::GraphRect,
        nodes: Arc<[nana_ui_core::GraphRect]>,
        indicator: Option<nana_ui_core::GraphRect>,
        node_fill: Option<nana_ui_core::SemanticColorRole>,
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
    pub icon: Option<Icon>,
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

/// Button chrome drawn behind a component's own trigger, so a menu can look
/// pressable without the caller supplying a separate button node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComponentTriggerSurface {
    pub bounds: LayoutBox,
    pub background: Option<[f32; 4]>,
    pub border: Option<[f32; 4]>,
}

/// Resolved spinner chrome at the trailing edge of a numeric field.
///
/// Both halves stay drawn while the field is enabled; `increment_enabled` /
/// `decrement_enabled` only report whether the value can still move, so the
/// control does not resize as it reaches a bound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberSteppers {
    pub increment: LayoutBox,
    pub decrement: LayoutBox,
    pub increment_color: [f32; 4],
    pub decrement_color: [f32; 4],
    pub increment_enabled: bool,
    pub decrement_enabled: bool,
    pub glyph_size: f32,
}

impl NumberSteppers {
    /// Signed step count for a point, or `None` when it misses both halves.
    pub fn step_at(&self, x: f32, y: f32) -> Option<i32> {
        if contains(self.increment, x, y) {
            return self.increment_enabled.then_some(1);
        }
        if contains(self.decrement, x, y) {
            return self.decrement_enabled.then_some(-1);
        }
        None
    }
}

fn contains(bounds: LayoutBox, x: f32, y: f32) -> bool {
    x >= bounds.x && x < bounds.x + bounds.width && y >= bounds.y && y < bounds.y + bounds.height
}

/// Resolved radio ring chrome for one selection option.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadioIndicator {
    pub ring: LayoutBox,
    pub ring_color: [f32; 4],
    /// Present only while the option is selected.
    pub dot: Option<(LayoutBox, [f32; 4])>,
}

/// One axis of resolved scrollbar chrome.
///
/// `track` is the full band along the viewport edge and is also the drag hit
/// region; `thumb` is the draggable handle inside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarBar {
    pub track: LayoutBox,
    pub thumb: LayoutBox,
    /// Painted only when the container keeps its bars resident.
    pub track_background: Option<[f32; 4]>,
    pub thumb_background: [f32; 4],
    pub thumb_radius: f32,
    /// Largest content offset this axis can reach.
    pub max_offset: f32,
}

impl ScrollbarBar {
    /// Position along the scrolling axis for a viewport point.
    pub fn axis_position(&self, axis: nana_ui_core::ScrollbarAxis, x: f32, y: f32) -> f32 {
        match axis {
            nana_ui_core::ScrollbarAxis::Horizontal => x,
            nana_ui_core::ScrollbarAxis::Vertical => y,
        }
    }

    /// Re-derive the axis track so drag maths stay in one place.
    pub fn track_geometry(
        &self,
        axis: nana_ui_core::ScrollbarAxis,
    ) -> nana_ui_core::ScrollbarTrack {
        match axis {
            nana_ui_core::ScrollbarAxis::Horizontal => nana_ui_core::ScrollbarTrack {
                origin: self.track.x,
                length: self.track.width,
                thumb_origin: self.thumb.x,
                thumb_length: self.thumb.width,
                max_offset: self.max_offset,
            },
            nana_ui_core::ScrollbarAxis::Vertical => nana_ui_core::ScrollbarTrack {
                origin: self.track.y,
                length: self.track.height,
                thumb_origin: self.thumb.y,
                thumb_length: self.thumb.height,
                max_offset: self.max_offset,
            },
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.track.x
            && x < self.track.x + self.track.width
            && y >= self.track.y
            && y < self.track.y + self.track.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComponentElevation {
    pub color: [f32; 4],
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    /// CSS `box-shadow: inset`. Outset drop shadows keep this `false`.
    pub inset: bool,
}

impl ComponentElevation {
    /// Lilia `--shadow-surface`: `0 10px 30px -24px` (dark) / `0 10px 26px -24px` (light).
    pub fn surface_shadow(theme_mode: nana_ui_core::ThemeMode) -> Self {
        match theme_mode {
            nana_ui_core::ThemeMode::Dark => Self {
                color: [0.0, 0.0, 0.0, 0.62],
                offset_x: 0.0,
                offset_y: 10.0,
                blur_radius: 30.0,
                spread_radius: -24.0,
                inset: false,
            },
            nana_ui_core::ThemeMode::Light => Self {
                color: [17.0 / 255.0, 24.0 / 255.0, 39.0 / 255.0, 0.24],
                offset_x: 0.0,
                offset_y: 10.0,
                blur_radius: 26.0,
                spread_radius: -24.0,
                inset: false,
            },
        }
    }

    pub fn from_box_shadow(shadow: nana_ui_core::BoxShadowSpec) -> Self {
        Self {
            color: shadow.color,
            offset_x: shadow.offset_x,
            offset_y: shadow.offset_y,
            blur_radius: shadow.blur_radius,
            spread_radius: shadow.spread_radius,
            inset: shadow.inset,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarHoverGeometry {
    pub ring: LayoutBox,
    pub tooltip: LayoutBox,
    pub title: ComponentTextRegion,
    pub ring_color: [f32; 4],
    pub tooltip_fill: [f32; 4],
    pub tooltip_border: [f32; 4],
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
        /// 诊断下划线条带（节点空间矩形 + 已解析的颜色）。
        diagnostic_markers: Vec<(LayoutBox, [f32; 4])>,
        /// 行号标签（节点空间 y，行号从 1 起）。
        line_labels: Vec<LineLabel>,
        /// 行号文本颜色与字号。
        line_labels_color: [f32; 4],
        line_labels_font_size: f32,
        background: Option<[f32; 4]>,
        border: Option<[f32; 4]>,
        border_width: f32,
        focus_ring: Option<[f32; 4]>,
        selection_color: [f32; 4],
        caret_color: [f32; 4],
        preedit_color: [f32; 4],
        steppers: Option<NumberSteppers>,
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
    Scrollbar {
        horizontal: Option<ScrollbarBar>,
        vertical: Option<ScrollbarBar>,
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
        detail: Option<ComponentTextRegion>,
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
        indicator: Option<RadioIndicator>,
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
        trigger_icon: Option<(Icon, LayoutBox)>,
        trigger_surface: Option<ComponentTriggerSurface>,
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
        hover: Option<CalendarHoverGeometry>,
    },
    TimeSeriesChart {
        grid: Vec<LayoutBox>,
        area: Vec<LayoutBox>,
        line: Vec<[f32; 2]>,
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
        edges: Vec<(Vec<[f32; 2]>, [f32; 4])>,
        edge_labels: Vec<ComponentTextRegion>,
        grid: Vec<LayoutBox>,
        background: [f32; 4],
        grid_color: [f32; 4],
        separator_color: [f32; 4],
    },
    GraphMinimap {
        nodes: Vec<LayoutBox>,
        node_fill: [f32; 4],
        indicator: Option<LayoutBox>,
        indicator_fill: [f32; 4],
        indicator_border: [f32; 4],
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
    pub icon: Option<(Icon, LayoutBox, [f32; 4])>,
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

#[derive(Debug, Clone, PartialEq)]
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

    /// 设置语义背景色角色。
    pub fn surface(mut self, role: SemanticColorRole) -> Self {
        self.background = Some(role);
        self
    }

    /// 一次性写全边框：语义色角色与宽度必须同时给出，缺一边框不会绘制。
    pub fn outline(mut self, role: SemanticColorRole, width: f32) -> Self {
        self.border = Some(role);
        let layout = Arc::make_mut(&mut self.layout);
        layout.border_width = Some(width.max(0.0));
        self
    }

    /// 设置圆角半径（物理 px）。
    pub fn radius(mut self, radius: f32) -> Self {
        Arc::make_mut(&mut self.layout).border_radius = Some(radius.max(0.0));
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub foreground: SemanticColorRole,
    pub color: Option<[f32; 4]>,
    pub background: Option<[f32; 4]>,
    pub border_color: Option<[f32; 4]>,
    pub opacity: f32,
    /// CSS `visibility` after inheritance (`visible` / `hidden`).
    pub visibility: nana_ui_core::VisibilitySpec,
    /// Self and every ancestor generate a layout box (`hidden` /
    /// `display: none` anywhere up the chain makes this `false`). Unlike CSS
    /// `visibility`, a descendant cannot override it back to `true`.
    pub box_visible: bool,
    pub visible: bool,
    /// CSS `pointer-events` after inheritance (`auto` / `none`).
    pub pointer_events: nana_ui_core::PointerEventsSpec,
    pub font_size: f32,
    pub font_weight: Option<u16>,
    pub italic: bool,
    pub font_family: Option<Arc<str>>,
    pub line_height: Option<LineHeightSpec>,
    pub letter_spacing: f32,
    pub font_features: Vec<FontFeatureSetting>,
    pub font_variations: Vec<FontVariationSetting>,
    pub font_kerning: FontKerningSpec,
    pub word_break: WordBreakSpec,
    pub line_break: LineBreakSpec,
    /// CSS `direction` after inherit (initial LTR).
    pub direction: nana_ui_core::DirSpec,
    /// CSS `writing-mode` after inherit (initial `horizontal-tb`).
    pub writing_mode: nana_ui_core::WritingModeSpec,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            foreground: SemanticColorRole::Text,
            color: None,
            background: None,
            border_color: None,
            opacity: 1.0,
            visibility: nana_ui_core::VisibilitySpec::Visible,
            box_visible: true,
            visible: true,
            pointer_events: nana_ui_core::PointerEventsSpec::Auto,
            font_size: UI_BASE_TEXT_SIZE,
            font_weight: None,
            italic: false,
            font_family: None,
            line_height: None,
            letter_spacing: 0.0,
            font_features: Vec::new(),
            font_variations: Vec::new(),
            font_kerning: FontKerningSpec::Auto,
            word_break: WordBreakSpec::Normal,
            line_break: LineBreakSpec::Auto,
            direction: nana_ui_core::DirSpec::Ltr,
            writing_mode: nana_ui_core::WritingModeSpec::HorizontalTb,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextContent {
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
    /// First-line ascent in CSS px (line box top to alphabetic baseline).
    /// `None` = layout falls back to [`nana_ui_core::TEXT_APPROX_ASCENT_EM`].
    pub ascent: Option<f32>,
}

/// Shaped intrinsic text owned by an EmptyState rather than application child
/// nodes. Runtime layout and every renderer consume the same measured runs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct EmptyStateTextPresentation {
    pub title: TextMetrics,
    pub message: Option<TextMetrics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
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
    pub max_lines: Option<u16>,
    pub shaping: TextShaping,
    pub preserve_lines: bool,
    pub wrap_break: nana_ui_core::TextWrapBreak,
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

    /// Shape using the Runtime-owned [`crate::GlyphCache`].
    ///
    /// The default ignores the cache so hosts without a glyph backend leave
    /// `glyph_cache_*` as `None`. [`MeasureTextShaper`] and `NanaTextShaper`
    /// record per-glyph advances here.
    fn shape_cached(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
        glyphs: &mut crate::GlyphCache,
    ) -> TextMetrics {
        let _ = glyphs;
        self.shape(id, text, style, constraints)
    }

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

/// Finite-metrics shaper for CPU hosts (framework `text-table`, layout-stop).
/// Wrap height stays against the last content box. Per-character advances go
/// through Runtime [`crate::GlyphCache`] lookup/insert.
#[derive(Debug, Default, Clone, Copy)]
pub struct MeasureTextShaper;

impl TextShaper for MeasureTextShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics {
        measure_em_text(text, style, constraints)
    }

    fn shape_cached(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
        glyphs: &mut crate::GlyphCache,
    ) -> TextMetrics {
        if constraints.preserve_lines {
            return measure_em_text(text, style, constraints);
        }
        let em = style.font_size.max(1.0);
        let mut intrinsic = 0.0;
        for ch in text.value.chars() {
            intrinsic += glyphs.lookup_or_insert(ch, style, em);
        }
        em_metrics(intrinsic, em, constraints)
    }
}

fn measure_em_text(
    text: &TextContent,
    style: &ComputedStyle,
    constraints: TextShapeConstraints,
) -> TextMetrics {
    let em = style.font_size.max(1.0);
    let _ = style.writing_mode;
    if constraints.preserve_lines {
        let line_height = resolved_text_line_height(style).max(em);
        let mut max_w = 0.0f32;
        let mut lines = 0usize;
        let wrap_width = constraints.max_width.filter(|w| *w > 0.0);
        for line in text.value.split('\n') {
            let w = line.chars().count() as f32 * em;
            if constraints.wrap {
                let max = wrap_width.unwrap_or(w).max(em);
                let wrapped = ((w / max).ceil() as usize).max(1);
                lines += wrapped;
                max_w = max_w.max(w.min(max));
            } else {
                lines += 1;
                max_w = max_w.max(w);
            }
        }
        return TextMetrics {
            width: max_w,
            height: line_height * lines.max(1) as f32,
            ascent: Some(em * nana_ui_core::TEXT_APPROX_ASCENT_EM),
        };
    }
    em_metrics(text.value.chars().count() as f32 * em, em, constraints)
}

fn em_metrics(intrinsic: f32, em: f32, constraints: TextShapeConstraints) -> TextMetrics {
    let width = constraints.max_width.unwrap_or(intrinsic).min(intrinsic);
    let height = if constraints.wrap && width + f32::EPSILON < intrinsic {
        em * (intrinsic / width.max(em)).ceil()
    } else {
        em
    };
    TextMetrics {
        width,
        height,
        ascent: Some(em * nana_ui_core::TEXT_APPROX_ASCENT_EM),
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

/// 诊断标记的可视矩形（文本空间坐标，随行折叠切分）。
#[derive(Debug, Clone, PartialEq)]
pub struct TextDiagnosticMark {
    pub rect: LayoutBox,
    pub severity: TextDiagnosticSeverity,
}

/// 行号标签（文本空间 y 坐标，逻辑行序号从 1 起）。
#[derive(Debug, Clone, PartialEq)]
pub struct LineLabel {
    pub y: f32,
    pub height: f32,
    pub number: u32,
}

/// Shaped editing presentation. The committed value remains in
/// [`TextInputState`]; this derived component only carries renderer geometry.
#[derive(Debug, Clone, PartialEq, Default)]
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
    /// 诊断下划线条带（文本空间），仅多行态计算。
    pub diagnostic_marks: Vec<TextDiagnosticMark>,
    /// 各逻辑行的 y 起点（启用行号栏时计算）。
    pub line_tops: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutInput {
    pub id: StableNodeId,
    pub parent: Option<StableNodeId>,
    pub children: Arc<Vec<StableNodeId>>,
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

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollOffset {
    pub x: f32,
    pub y: f32,
}

/// Derived scrollport and content extents in logical pixels. Absence means
/// the layout backend has not measured this scroll container yet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollMetrics {
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub content_width: f32,
    pub content_height: f32,
}

/// Exclusive overlay state attached to an overlay host. `active` must be a
/// direct child of the host; `restore_focus` remains in the same document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Separator,
    Dialog,
    AlertDialog,
    Menu,
    MenuItem,
    Tooltip,
    Status,
    Image,
    Main,
    Navigation,
    Banner,
    ContentInfo,
    Complementary,
    Region,
    Search,
    Form,
    #[default]
    Generic,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccessibilityState {
    pub role: AccessibilityRole,
    pub label: Option<Arc<str>>,
    pub value: Option<Arc<str>>,
    pub description: Option<Arc<str>>,
    pub disabled: bool,
    pub checked: Option<bool>,
    /// Tri-state checkbox in its mixed state. Wins over `checked`.
    pub mixed: bool,
    /// Layout direction of a composite such as a radio group or tab list.
    pub orientation: Option<crate::SelectionOrientation>,
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
    pub mixed: bool,
    pub orientation: Option<crate::SelectionOrientation>,
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

/// Subscribed DOM/Vue event names. Capture/bubble paths stay on [`EventRoute`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventListeners {
    events: BTreeSet<String>,
}

impl EventListeners {
    pub fn contains(&self, event: &str) -> bool {
        self.events.contains(event)
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.events.iter().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub(crate) fn set(&mut self, event: String, enabled: bool) {
        if enabled {
            self.events.insert(event);
        } else {
            self.events.remove(&event);
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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

    /// Delete UTF-8 bytes before and after the current selection.
    ///
    /// IME preedit is stored separately and is not touched. Returns false when
    /// the selection is invalid, the span is empty, the range would overflow,
    /// or either end is not a character boundary.
    pub fn delete_surrounding(&mut self, before_bytes: usize, after_bytes: usize) -> bool {
        if !self.selection.is_valid_for(&self.value) {
            return false;
        }
        let range = self.selection.ordered();
        let Some(start) = range.start.checked_sub(before_bytes) else {
            return false;
        };
        let Some(end) = range.end.checked_add(after_bytes) else {
            return false;
        };
        if end > self.value.len() || start == end {
            return false;
        }
        if !self.value.is_char_boundary(start) || !self.value.is_char_boundary(end) {
            return false;
        }
        self.value.replace_range(start..end, "");
        self.selection = TextSelection::caret(start);
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
/// First-class layout/Scene citizen: the same clip, z-index, hit-test, and
/// document order as Quad/Text. `renderer` selects an installed renderer
/// extension and `resource` is an opaque application-owned lookup key. Neither
/// field exposes a GPU backend object, so the same extraction can be consumed
/// by WGPU or a future RHI.
///
/// [`Self::fit`] is presentation-only. Host-texture sampling uses it to choose
/// a destination rect; other renderers ignore Fill.
///
/// [`Self::params`] carries per-node presentation values whose meaning is
/// defined by `renderer` alone. Runtime never interprets them, so a renderer
/// can reach its own painter without Runtime learning a shading model.
/// [`Self::dedicated_pass`] asks the painter to open a pass for this node
/// instead of joining the current one.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomRenderNode {
    pub renderer: Arc<str>,
    pub resource: Arc<str>,
    pub revision: u64,
    pub fit: nana_ui_core::ContentFit,
    pub params: Option<Arc<[f32]>>,
    pub dedicated_pass: bool,
    /// 在纹理下方绘制 alpha 棋盘底（host texture 渲染器契约）。
    pub checkerboard: bool,
    /// 纹理缩放系数，1.0 为适配原始大小（host texture 渲染器契约）。
    pub zoom: f32,
}

impl CustomRenderNode {
    pub fn new(
        renderer: impl Into<Arc<str>>,
        resource: impl Into<Arc<str>>,
        revision: u64,
    ) -> Self {
        Self {
            renderer: renderer.into(),
            resource: resource.into(),
            revision,
            fit: nana_ui_core::ContentFit::Fill,
            params: None,
            dedicated_pass: false,
            checkerboard: false,
            zoom: 1.0,
        }
    }

    pub const fn with_fit(mut self, fit: nana_ui_core::ContentFit) -> Self {
        self.fit = fit;
        self
    }

    /// 在纹理下方绘制 alpha 棋盘底。
    pub const fn with_checkerboard(mut self, checkerboard: bool) -> Self {
        self.checkerboard = checkerboard;
        self
    }

    /// 设置纹理缩放系数（非有限值忽略，保持原值语义为 1.0 由调用方保证）。
    pub const fn with_zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom;
        self
    }

    /// Attaches renderer-defined presentation values. Non-finite entries become
    /// zero so a painter never uploads NaN into a uniform.
    pub fn with_params(mut self, params: impl IntoIterator<Item = f32>) -> Self {
        self.params = Some(
            params
                .into_iter()
                .map(|value| if value.is_finite() { value } else { 0.0 })
                .collect(),
        );
        self
    }

    /// Requests a dedicated render pass for this node.
    pub const fn with_dedicated_pass(mut self, dedicated_pass: bool) -> Self {
        self.dedicated_pass = dedicated_pass;
        self
    }

    /// Renderer-defined parameter at `index`, or `None` when absent.
    pub fn param(&self, index: usize) -> Option<f32> {
        self.params
            .as_ref()
            .and_then(|params| params.get(index))
            .copied()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedNode {
    pub id: StableNodeId,
    /// Shared with `UiWorld` so idle extract is an Arc bump, not a `NodeKind` clone.
    pub kind: Arc<NodeKind>,
    pub parent: Option<StableNodeId>,
    /// Shared with retained hierarchy. Idle extract does not clone the child list.
    pub children: Arc<Vec<StableNodeId>>,
    pub layout: LayoutBox,
    pub scroll_offset: ScrollOffset,
    pub source_style: NodeStyle,
    /// Shared with `UiWorld` resolved style. Copy-on-write when a dirty node mutates.
    pub style: Arc<ComputedStyle>,
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
