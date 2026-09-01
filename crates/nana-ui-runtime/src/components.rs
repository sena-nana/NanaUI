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

/// 编辑器查找匹配 span。`offset`/`length` 为字节偏移，`current` 标记"当前
/// 匹配"（渲染强调样式）。与诊断 span 相同，宿主负责在文本变化后更新或
/// 清除；越界部分在几何计算时被钳制。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMatchSpan {
    pub offset: usize,
    pub length: usize,
    pub current: bool,
}

impl TextMatchSpan {
    pub fn new(offset: usize, length: usize) -> Self {
        Self {
            offset,
            length,
            current: false,
        }
    }

    /// 标记为"当前匹配"。
    pub fn current(mut self) -> Self {
        self.current = true;
        self
    }
}

/// 代码折叠区间。字节偏移覆盖整个块（含首尾花括号），`start < end`。
///
/// 宿主在每次文本变化后重新喂 [`crate::TextArea::code_folds`]；哪些区间
/// 处于折叠态由组件内部（Runtime 侧按节点维护）管理，宿主不感知折叠
/// 状态的存储。折叠是纯视图状态：不改变 committed value。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextCodeFold {
    pub start: usize,
    pub end: usize,
}

impl TextCodeFold {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// 是否可折叠：区间内至少有一个换行（单行区间没有可隐藏的行）。
    pub fn collapsible_in(self, value: &str) -> bool {
        self.start < self.end
            && self.end <= value.len()
            && value[self.start.min(value.len())..self.end].contains('\n')
    }

    /// 折叠后第一个被隐藏的字节：`start` 所在行的行尾换行符位置
    /// （区间无换行时等于 `end`，即无可隐藏内容）。
    pub fn hidden_start_in(self, value: &str) -> usize {
        value[self.start.min(value.len())..]
            .find('\n')
            .map_or(self.end, |index| self.start + index)
            .min(self.end.max(self.start))
    }
}

/// 宿主触发的代码片段。`body` 中的 `$N`（N 为十进制数字）是占位标记：
/// 插入时从文本中移除，`$1..$N` 按序号成为 Tab 跳位，`$0` 是插入后的
/// 初始光标位置（缺省为插入文本末尾）。其余字符原样插入，`$` 后不跟
/// 数字时保持字面量。不支持 `${N:default}` 与占位镜显。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSnippet {
    pub label: String,
    pub body: String,
}

impl TextSnippet {
    pub fn new(label: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            body: body.into(),
        }
    }
}

/// 活跃的 snippet 会话（每个编辑器节点至多一个）。`stops` 是 `$1..$N`
/// 在 committed value 中的光标偏移，`index` 指向当前跳位。文本被外部
/// 编辑时跳位按最小变更区间重映射，失效即结束会话。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextSnippetSession {
    pub stops: Vec<usize>,
    pub index: usize,
}

/// 文本空间内的查找匹配高亮条带（按折行拆分后可能一条 span 对应多条）。
#[derive(Debug, Clone, PartialEq)]
pub struct TextMatchMark {
    pub rect: LayoutBox,
    pub current: bool,
}

/// 折叠摘要标记（文本空间）：折叠起始行尾的 ` …N` 文本框，可点击
/// 切换该折叠。矩形由 shape 管线用真实字形度量计算。
#[derive(Debug, Clone, PartialEq)]
pub struct TextFoldMark {
    pub rect: LayoutBox,
    pub fold: TextCodeFold,
}

/// gutter 折叠箭头（节点空间）：可点击切换的命中框，`collapsed` 决定
/// 画收起（右箭头）还是展开（下箭头）形态。颜色由世界按低对比令牌解析。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextFoldGutter {
    pub bounds: LayoutBox,
    pub fold: TextCodeFold,
    pub collapsed: bool,
    pub color: [f32; 4],
}

/// 折叠摘要标记命中框（节点空间），落在折叠起始行行尾。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextFoldMarker {
    pub bounds: LayoutBox,
    pub fold: TextCodeFold,
}

/// [`crate::ComponentGeometry::TextInput`] 的折叠几何集合。空集合即
/// 默认值，零额外成本。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextFoldGeometry {
    pub gutters: Vec<TextFoldGutter>,
    pub markers: Vec<TextFoldMarker>,
}

/// 节点空间内的查找匹配高亮条带，颜色由世界按当前/普通匹配解析。
#[derive(Debug, Clone, PartialEq)]
pub struct TextMatchMarker {
    pub rect: LayoutBox,
    pub color: [f32; 4],
    pub current: bool,
}

/// 一条补全候选。`label` 是接受后插入的主体；`kind_label` 是右侧类型
/// 标注（如 `fn` / `struct` / `关键字`，宿主决定文案）；`detail` 是签名等
/// 次要说明，可为空。
///
/// 过滤完全由宿主负责：宿主按当前词前缀过滤后喂入
/// [`crate::TextArea::completions`]，组件只负责展示、键盘导航与接受，
/// 不做任何候选匹配。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCompletion {
    pub label: String,
    pub kind_label: String,
    pub detail: String,
}

impl TextCompletion {
    pub fn new(label: impl Into<String>, kind_label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            kind_label: kind_label.into(),
            detail: String::new(),
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
}

/// 锚定某个偏移的 hover 文档浮窗内容。纯展示：触发与生命周期完全由
/// 宿主决定（文本编辑时宿主负责撤掉，组件不自动隐藏）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextHover {
    pub offset: usize,
    pub title: String,
    pub body: String,
}

impl TextHover {
    pub fn new(offset: usize, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            offset,
            title: title.into(),
            body: body.into(),
        }
    }
}

/// 补全弹层可见行数上限（同时是键盘导航的滚动窗口大小）。
pub const TEXT_COMPLETION_VISIBLE_ROWS: usize = 8;

/// 补全弹层面板的水平内边距与内容宽度上限（"宽度自适应最长行"的
/// 上限；超出后先压缩 detail 区域，label/kind 保持完整）。
pub const TEXT_COMPLETION_PANEL_PAD: f32 = 8.0;
pub const TEXT_COMPLETION_MAX_CONTENT_WIDTH: f32 = 344.0;

/// 补全会话的只读快照（宿主查询入口；会话不存在时无值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextCompletionSnapshot {
    /// 当前候选总数。
    pub count: usize,
    /// 键盘选中的候选下标。
    pub selected: usize,
    /// 第一条可见候选的绝对下标。
    pub scroll: usize,
    /// Esc 关闭标记：弹层是否处于关闭态。
    pub dismissed: bool,
}

/// shape 管线按候选列表缓存的行宽度量。`items` 是指针相等短路键：
/// 列表未变时整个度量直接复用，零测量、零分配。
#[derive(Debug, Clone, PartialEq)]
pub struct TextCompletionPopupMetrics {
    pub items: Arc<[TextCompletion]>,
    /// 最宽 label（未裁剪）。
    pub label_width: f32,
    /// 最宽 detail（未裁剪）。
    pub detail_width: f32,
    /// 最宽 kind 标注（未裁剪）。
    pub kind_width: f32,
}

/// hover 浮窗正文的最大可见行数；超出部分滚轮滚动查看。
pub const TEXT_HOVER_MAX_BODY_ROWS: usize = 10;

/// 补全弹层的一行几何（节点空间）。`bounds` 是整行（选中高亮底），
/// 文本框带内容与颜色，由场景绘制（不换行，超宽省略号截断）。
#[derive(Debug, Clone, PartialEq)]
pub struct TextCompletionRow {
    pub bounds: LayoutBox,
    pub label: ComponentTextRegion,
    /// 次要说明；`detail` 为空的候选没有。
    pub detail: Option<ComponentTextRegion>,
    /// 右对齐类型标注；所有候选的 kind 都为空时整列省略。
    pub kind: Option<ComponentTextRegion>,
}

/// [`crate::ComponentGeometry::TextInput`] 的补全弹层几何。仅聚焦多行
/// 编辑器且候选会话非空时存在；绘制在编辑器所有覆盖层（折叠、诊断、
/// 行号）之上，点击命中与键盘交互由框架命令处理。
#[derive(Debug, Clone, PartialEq)]
pub struct TextCompletionPopup {
    pub panel: LayoutBox,
    /// 当前选中项的绝对候选下标。
    pub selected: usize,
    /// 第一条可见候选的绝对下标（滚动位置）。
    pub first_row: usize,
    pub rows: Vec<TextCompletionRow>,
    pub background: [f32; 4],
    pub border: [f32; 4],
    pub selected_background: [f32; 4],
    pub label_color: [f32; 4],
    pub detail_color: [f32; 4],
    pub kind_color: [f32; 4],
}

/// [`crate::ComponentGeometry::TextInput`] 的 hover 文档浮窗几何。宿主
/// 喂入 [`crate::TextArea::hover`] 时存在，纯展示；滚轮滚动由框架处理。
#[derive(Debug, Clone, PartialEq)]
pub struct TextHoverPopup {
    pub panel: LayoutBox,
    pub title: ComponentTextRegion,
    /// 可见正文行（按逻辑行滚动切片）。多行、超长裁剪，滚轮消费。
    pub body_rows: Vec<ComponentTextRegion>,
    pub background: [f32; 4],
    pub border: [f32; 4],
    pub title_color: [f32; 4],
    pub body_color: [f32; 4],
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
        /// 查找匹配高亮 span（普通匹配与当前匹配，见 [`TextMatchSpan`]）。
        /// 偏移同样由宿主维护。
        matches: Arc<[TextMatchSpan]>,
        /// 行号栏。行号绘制在节点左内边距区域，宿主需预留足够的 padding。
        line_numbers: bool,
        /// 缩进参考线。`Some(indent_unit)` 时在每个逻辑行的前导空白处按
        /// 缩进单位宽度画竖线（仅多行态生效）。
        indent_guides: Option<Arc<str>>,
        /// 代码折叠区间（见 [`TextCodeFold`]）。宿主在文本变化后重新喂；
        /// 哪些区间处于折叠态由组件内部维护，渲染按折叠后的视图展示。
        folds: Arc<[TextCodeFold]>,
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
        /// 附加多光标（收起态）的 caret 矩形；主光标仍在 `caret`。
        additional_carets: Vec<LayoutBox>,
        /// 附加光标颜色（主光标 `caret_color` 的半透明变体，保持同形）。
        additional_caret_color: [f32; 4],
        preedit: Vec<LayoutBox>,
        /// 诊断下划线条带（节点空间矩形 + 已解析的颜色）。
        diagnostic_markers: Vec<(LayoutBox, [f32; 4])>,
        /// 查找匹配高亮条带（节点空间矩形 + 已解析的颜色；`current` 为当前
        /// 匹配，绘制层级在普通匹配之上）。
        match_markers: Vec<TextMatchMarker>,
        /// 光标所在行的低对比背景条。仅聚焦且选区收起时存在，占用选区层
        /// （选区与当前行条互斥）。
        caret_line: Option<(LayoutBox, [f32; 4])>,
        /// 光标相邻括号与其配对端的描边框（节点空间矩形 + 已解析的颜色）。
        bracket_markers: Vec<(LayoutBox, [f32; 4])>,
        /// 缩进参考线竖线（节点空间矩形 + 已解析的颜色）。
        indent_guides: Vec<(LayoutBox, [f32; 4])>,
        /// 行号标签（节点空间 y，行号从 1 起）。
        line_labels: Vec<LineLabel>,
        /// 折叠几何：gutter 箭头（可点击切换）与折叠摘要标记命中框。
        folds: TextFoldGeometry,
        /// 行号文本颜色与字号。
        line_labels_color: [f32; 4],
        line_labels_font_size: f32,
        /// 补全弹层（聚焦多行编辑器 + 非空候选会话时存在）。绘制在
        /// 全部编辑器覆盖层之上（slot 90+）。
        completion_popup: Option<TextCompletionPopup>,
        /// hover 文档浮窗（宿主喂入时存在，纯展示；slot 120+）。
        hover_popup: Option<TextHoverPopup>,
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
    /// 选区条带（文本空间）：主选区与附加光标选区的视觉行矩形合并在同一
    /// 向量里（多光标选区集互不重叠，条带天然不重叠）。
    pub selection_lines: Vec<LayoutBox>,
    pub caret_x: f32,
    pub caret_y: f32,
    pub line_height: f32,
    pub preedit: Option<(f32, f32)>,
    pub preedit_lines: Vec<LayoutBox>,
    /// 附加光标（收起的 additional selection）的文本空间位置；主光标仍在
    /// `caret_x`/`caret_y`。仅多行态计算。
    pub additional_carets: Vec<(f32, f32)>,
    /// 诊断下划线条带（文本空间），仅多行态计算。
    pub diagnostic_marks: Vec<TextDiagnosticMark>,
    /// 查找匹配高亮条带（文本空间），仅多行态计算。
    pub match_marks: Vec<TextMatchMark>,
    /// 光标相邻括号与其配对端的描边框（文本空间，仅聚焦多行态计算）。
    pub bracket_marks: Vec<LayoutBox>,
    /// 缩进参考线竖线（文本空间，仅多行代码编辑态计算）。
    pub indent_guides: Vec<LayoutBox>,
    /// 各逻辑行的 y 起点（启用行号栏时计算）。
    pub line_tops: Vec<f32>,
    /// 与 `line_tops` 对齐的原始逻辑行号（折叠隐藏行后显示行索引不再
    /// 等于行号；为空时行号 = 索引 + 1）。
    pub line_numbers: Vec<u32>,
    /// 折叠摘要标记（文本空间，存在折叠态区间时计算）。
    pub fold_marks: Vec<TextFoldMark>,
    /// 锚定浮层度量（补全弹层行宽缓存 + hover 锚点）。列表指针相等时
    /// 行宽度量整段复用；无浮层时全部为 `None`（零分配）。
    pub overlay_metrics: TextOverlayMetrics,
}

/// [`TextInputPresentation`] 的浮层度量集合。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextOverlayMetrics {
    /// 补全弹层行宽度量；候选会话不存在时为 `None`。
    pub completion: Option<TextCompletionPopupMetrics>,
    /// hover 锚点（文本空间 `(x, y)`，`offset` 所在行的字形位置）。
    pub hover_anchor: Option<(f32, f32)>,
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

/// Sort key for a selection: span start, then span end.
fn selection_order(selection: TextSelection) -> (usize, usize) {
    let range = selection.ordered();
    (range.start, range.end)
}

/// Merge in-place: sort by span, fuse overlapping/touching spans, and carry a
/// `primary` flag through fusions so callers can keep the primary cursor's
/// identity. Returns the primary span plus the remaining sorted spans.
fn merge_selection_set(
    selections: &mut Vec<(TextSelection, bool)>,
) -> (TextSelection, Vec<TextSelection>) {
    selections.sort_by_key(|(selection, _)| selection_order(*selection));
    let mut merged: Vec<(TextSelection, bool)> = Vec::with_capacity(selections.len());
    for &(next, is_primary) in selections.iter() {
        match merged.last_mut() {
            Some((last, last_is_primary)) if last.ordered().end >= next.ordered().start => {
                let start = last.ordered().start;
                let end = last.ordered().end.max(next.ordered().end);
                *last = TextSelection {
                    anchor: start,
                    focus: end,
                };
                *last_is_primary |= is_primary;
            }
            _ => merged.push((next, is_primary)),
        }
    }
    let mut primary = None;
    let mut additional = Vec::new();
    for (selection, is_primary) in merged {
        if is_primary && primary.is_none() {
            primary = Some(selection);
        } else {
            additional.push(selection);
        }
    }
    (primary.unwrap_or_default(), additional)
}

/// Committed editable text and its selection. IME preedit remains separate in
/// [`ImeComposition`], so cancelling composition never corrupts committed text.
///
/// Beyond the primary [`TextSelection`], a multiline editor can hold
/// `additional_selections` (Zed-style multiple cursors). The invariant set:
/// spans are valid for `value`, sorted by offset, and pairwise disjoint (the
/// full collection is the merge of `selection` and `additional_selections`;
/// [`TextInputState::selections`] reports it). Keep the collection normalized
/// after every edit with [`TextInputState::normalize_selections`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextInputState {
    pub value: String,
    pub selection: TextSelection,
    /// Extra cursors/selections, kept empty for the single-cursor fast path.
    pub additional_selections: Vec<TextSelection>,
}

impl TextInputState {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let selection = TextSelection::caret(value.len());
        Self {
            value,
            selection,
            additional_selections: Vec::new(),
        }
    }

    /// Whether more than one cursor/selection is active. Hot paths check this
    /// first so single-cursor editing and painting stay allocation-free.
    pub fn has_additional_selections(&self) -> bool {
        !self.additional_selections.is_empty()
    }

    /// The complete selection set: [`TextInputState::selection`] plus
    /// [`TextInputState::additional_selections`], sorted by span start with
    /// overlapping/touching spans fused. With one cursor this borrows a
    /// one-element slice instead of allocating.
    pub fn selections(&self) -> std::borrow::Cow<'_, [TextSelection]> {
        if self.additional_selections.is_empty() {
            std::borrow::Cow::Borrowed(std::slice::from_ref(&self.selection))
        } else {
            let mut flagged: Vec<(TextSelection, bool)> =
                Vec::with_capacity(self.additional_selections.len() + 1);
            flagged.push((self.selection, true));
            flagged.extend(self.additional_selections.iter().map(|&s| (s, false)));
            let (primary, additional) = merge_selection_set(&mut flagged);
            let mut all = Vec::with_capacity(additional.len() + 1);
            all.push(primary);
            all.extend(additional);
            all.sort_by_key(|selection| selection_order(*selection));
            std::borrow::Cow::Owned(all)
        }
    }

    /// Restore the multi-selection invariants: invalid spans clamp onto the
    /// nearest char boundary, overlapping/touching spans fuse, and a span that
    /// fuses with the primary becomes the primary (the primary cursor's
    /// identity never moves onto another span). No-op with one cursor.
    pub fn normalize_selections(&mut self) {
        if !self.selection.is_valid_for(&self.value) {
            let fallback = crate::text_editing::clamp_boundary(&self.value, self.selection.focus);
            self.selection = TextSelection::caret(fallback);
        }
        if self.additional_selections.is_empty() {
            return;
        }
        let mut flagged: Vec<(TextSelection, bool)> =
            Vec::with_capacity(self.additional_selections.len() + 1);
        flagged.push((self.selection, true));
        for selection in self.additional_selections.drain(..) {
            let normalized = if selection.is_valid_for(&self.value) {
                selection
            } else {
                TextSelection::caret(crate::text_editing::clamp_boundary(
                    &self.value,
                    selection.focus,
                ))
            };
            flagged.push((normalized, false));
        }
        let (primary, additional) = merge_selection_set(&mut flagged);
        self.selection = primary;
        self.additional_selections = additional;
    }

    /// Add candidate selections, fusing overlaps/touching spans and dropping
    /// candidates that already exist in the set. Reports whether the set grew.
    pub fn add_selections(&mut self, candidates: &[TextSelection]) -> bool {
        if candidates.is_empty() {
            return false;
        }
        let existing = self.selections();
        let mut added = false;
        let mut flagged: Vec<(TextSelection, bool)> =
            Vec::with_capacity(self.additional_selections.len() + candidates.len() + 1);
        flagged.push((self.selection, true));
        flagged.extend(
            self.additional_selections
                .iter()
                .map(|&selection| (selection, false)),
        );
        for &candidate in candidates {
            let candidate = if candidate.is_valid_for(&self.value) {
                candidate
            } else {
                TextSelection::caret(crate::text_editing::clamp_boundary(
                    &self.value,
                    candidate.focus,
                ))
            };
            if existing.contains(&candidate) || flagged.iter().any(|(s, _)| *s == candidate) {
                continue;
            }
            flagged.push((candidate, false));
            added = true;
        }
        if !added {
            return false;
        }
        let (primary, additional) = merge_selection_set(&mut flagged);
        self.selection = primary;
        self.additional_selections = additional;
        true
    }

    /// Drop every cursor except the primary selection.
    pub fn collapse_selections(&mut self) {
        self.additional_selections.clear();
    }

    /// Remap `additional_selections` across one committed edit that replaced
    /// `removed` bytes at `start` with `inserted` bytes, then restore the
    /// invariants. Single-cursor states stay untouched.
    fn remap_selections_after_edit(&mut self, start: usize, removed: usize, inserted: usize) {
        if self.additional_selections.is_empty() {
            return;
        }
        let end = start + removed;
        let delta = inserted as isize - removed as isize;
        for selection in &mut self.additional_selections {
            let map = |offset: usize| -> usize {
                if offset <= start {
                    offset
                } else if offset >= end {
                    ((offset as isize + delta).max(start as isize)) as usize
                } else {
                    // Inside the replaced span: collapse onto the edit point.
                    start + inserted
                }
            };
            selection.anchor = map(selection.anchor);
            selection.focus = map(selection.focus);
        }
    }

    /// Replace the text of every selection with `text` (one cursor replaces
    /// its own selection; multiple cursors each receive an insertion) and
    /// park every cursor at the end of its inserted text.
    pub fn replace_selection(&mut self, text: &str) -> bool {
        if !self.selection.is_valid_for(&self.value) {
            return false;
        }
        if !self.has_additional_selections() {
            let range = self.selection.ordered();
            let caret = range.start + text.len();
            self.value.replace_range(range.clone(), text);
            self.selection = TextSelection::caret(caret);
            return true;
        }
        // Multi-cursor: one insertion per selection against the pre-edit
        // value, spliced in a single pass.
        let selections = self.selections();
        let primary_index = selections
            .iter()
            .position(|selection| *selection == self.selection)
            .unwrap_or(0);
        let edits: Vec<(TextSelection, Option<crate::text_editing::CursorEdit>)> = selections
            .iter()
            .map(|&selection| {
                let edit = if selection.is_valid_for(&self.value) {
                    let range = selection.ordered();
                    let caret = range.start + text.len();
                    Some(crate::text_editing::CursorEdit::Span(
                        crate::text_editing::TextReplacement {
                            range,
                            insert: text.to_owned(),
                            caret,
                        },
                    ))
                } else {
                    None
                };
                (selection, edit)
            })
            .collect();
        let Some((next_value, next_selections)) =
            crate::text_editing::apply_cursor_edits(&self.value, &edits)
        else {
            return false;
        };
        self.value = next_value;
        self.selection = next_selections
            .get(primary_index)
            .copied()
            .unwrap_or_else(|| TextSelection::caret(self.value.len()));
        self.additional_selections = next_selections
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != primary_index)
            .map(|(_, selection)| *selection)
            .collect();
        self.normalize_selections();
        true
    }

    /// Replace only the primary selection's text (IME commit path); every
    /// other cursor survives the edit through offset remapping. This is the
    /// documented multi-cursor IME restriction: composition commits to the
    /// primary cursor alone.
    pub fn replace_primary_selection(&mut self, text: &str) -> bool {
        if !self.selection.is_valid_for(&self.value) {
            return false;
        }
        let range = self.selection.ordered();
        let caret = range.start + text.len();
        self.value.replace_range(range.clone(), text);
        self.selection = TextSelection::caret(caret);
        self.remap_selections_after_edit(range.start, range.len(), text.len());
        self.normalize_selections();
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
        self.remap_selections_after_edit(start, end - start, 0);
        self.normalize_selections();
        true
    }

    /// Replace a controlled value while keeping a valid selection when
    /// possible. If the old offsets no longer land on UTF-8 boundaries, move
    /// the caret to the new end. A wholesale value replacement also drops any
    /// additional cursors: their offsets have no meaning in foreign text.
    pub fn replace_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        if !self.selection.is_valid_for(&self.value) {
            self.selection = TextSelection::caret(self.value.len());
        }
        self.additional_selections.clear();
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
        // Native editor snapshots carry a single selection; drop the rest.
        self.additional_selections.clear();
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

#[cfg(test)]
mod tests {
    use super::{TextInputState, TextSelection};

    fn state(
        value: &str,
        selection: TextSelection,
        additional_selections: Vec<TextSelection>,
    ) -> TextInputState {
        TextInputState {
            value: value.into(),
            selection,
            additional_selections,
        }
    }

    #[test]
    fn selections_reports_the_sorted_set_and_borrows_a_single_cursor() {
        let single = state("abcd", TextSelection::caret(2), Vec::new());
        assert!(matches!(single.selections(), std::borrow::Cow::Borrowed(_)));
        assert_eq!(single.selections().len(), 1);

        // 附加光标按 offset 排序；主光标保持自身身份（不必排在最前）。
        let multi = state(
            "abcd",
            TextSelection {
                anchor: 3,
                focus: 4,
            },
            vec![TextSelection::caret(0)],
        );
        assert_eq!(
            multi.selections().into_owned(),
            vec![
                TextSelection::caret(0),
                TextSelection {
                    anchor: 3,
                    focus: 4
                }
            ]
        );
        assert_eq!(
            multi.selection,
            TextSelection {
                anchor: 3,
                focus: 4
            }
        );
    }

    #[test]
    fn normalize_fuses_touching_spans_into_the_primary() {
        let mut multi = state(
            "abcdef",
            TextSelection {
                anchor: 2,
                focus: 3,
            },
            vec![
                TextSelection {
                    anchor: 3,
                    focus: 5,
                },
                TextSelection::caret(5),
                TextSelection::caret(0),
            ],
        );
        multi.normalize_selections();
        // 主光标吸收与其相接的 span（身份不转移），其余保持附加集合。
        // caret(5) 与 [2,5) 相接但为空跨度，不延长并集。
        assert_eq!(
            multi.selection,
            TextSelection {
                anchor: 2,
                focus: 5
            }
        );
        assert_eq!(multi.additional_selections, vec![TextSelection::caret(0)]);
    }

    #[test]
    fn normalize_clamps_invalid_offsets_onto_boundaries() {
        let value = "ab\u{1F600}cd";
        let mut multi = state(
            value,
            TextSelection::caret(1),
            vec![TextSelection::caret(3)],
        );
        multi.normalize_selections();
        // 偏移 3 落在 emoji 中间，收敛到最近的字符边界 2。
        assert_eq!(multi.additional_selections, vec![TextSelection::caret(2)]);
    }

    #[test]
    fn replace_selection_splices_every_cursor_in_one_pass() {
        let mut multi = state(
            "ab_cd",
            TextSelection::caret(2),
            vec![TextSelection::caret(5)],
        );
        assert!(multi.replace_selection("X"));
        assert_eq!(multi.value, "abX_cdX");
        assert_eq!(multi.selection, TextSelection::caret(3));
        assert_eq!(multi.additional_selections, vec![TextSelection::caret(7)]);

        // 相邻光标会先合并，只插入一次。
        let mut touching = state(
            "abc",
            TextSelection::caret(1),
            vec![TextSelection::caret(1)],
        );
        assert!(touching.replace_selection("X"));
        assert_eq!(touching.value, "aXbc");
        assert!(touching.additional_selections.is_empty());
    }

    #[test]
    fn replace_primary_selection_scopes_ime_commits_and_remaps_others() {
        let mut multi = state(
            "ab_cd",
            TextSelection::caret(2),
            vec![TextSelection::caret(5)],
        );
        assert!(multi.replace_primary_selection("X"));
        assert_eq!(multi.value, "abX_cd");
        assert_eq!(multi.selection, TextSelection::caret(3));
        // 附加光标随编辑平移，仍然有效。
        assert_eq!(multi.additional_selections, vec![TextSelection::caret(6)]);
    }

    #[test]
    fn add_selections_skips_duplicates_and_collapses_back() {
        let mut multi = state("abcdef", TextSelection::caret(0), Vec::new());
        assert!(multi.add_selections(&[
            TextSelection::caret(3),
            TextSelection::caret(3),
            TextSelection::caret(5),
        ]));
        assert_eq!(
            multi.additional_selections,
            vec![TextSelection::caret(3), TextSelection::caret(5)]
        );
        // 已有光标处不再添加。
        assert!(!multi.add_selections(&[TextSelection::caret(5)]));
        multi.collapse_selections();
        assert!(multi.additional_selections.is_empty());
        assert_eq!(multi.selection, TextSelection::caret(0));
    }

    #[test]
    fn wholesale_value_replacements_drop_additional_cursors() {
        let mut multi = state(
            "abcd",
            TextSelection::caret(1),
            vec![TextSelection::caret(3)],
        );
        multi.replace_value("xy");
        assert!(multi.additional_selections.is_empty());

        let mut multi = state(
            "abcd",
            TextSelection::caret(1),
            vec![TextSelection::caret(4)],
        );
        multi.synchronize_editor_value("abcdefg");
        assert!(multi.additional_selections.is_empty());
        // 最小变更区间的末端落在新值的插入点。
        assert_eq!(multi.selection, TextSelection::caret(7));
    }
}
