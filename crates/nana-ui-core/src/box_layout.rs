//! Style Model **Layout** — box flex intent (not a CSSOM, not workspace regions).
//!
//! Pure data consumed by L3 draw (`nana-ui` Scene painter) and L1/L2 adapters.
//! CSS declaration / class parsing stays in `nana-ui-vue::css_map` /
//! `nana-ui-vue::shell_contract`.
//! Workspace region layout lives in [`crate::layout`].
//!
//! Shared geometry helpers used by `measure` (pre-paint / parity) and Runtime
//! layout: [`LayoutStyle::resolve_content_box`], [`LayoutStyle::resolve_inset`],
//! `resolved_padding_against` / `resolved_margin_against` / gap helpers.
//! Prefer extending these over duplicating box math in vue adapters.

use serde::{Deserialize, Serialize};

/// Flex 主轴方向（`row` / `column`；`*-reverse` 见 [`LayoutStyle::flex_reverse`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FlexDirection {
    #[default]
    Column,
    Row,
}

impl FlexDirection {
    /// 水平主轴（`row` / `row-reverse`）。
    pub fn is_row(self) -> bool {
        matches!(self, Self::Row)
    }

    /// 垂直主轴（`column` / `column-reverse`）。
    pub fn is_column(self) -> bool {
        matches!(self, Self::Column)
    }
}

/// CSS `direction` (`ltr` / `rtl`). Not [`FlexDirection`] (`flex-direction`).
///
/// Vertical `writing-mode` stays fail-closed. This remaps logical box edges
/// and `text-align: start | end` only — it does **not** flip flex/grid
/// main/cross start or item order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DirSpec {
    #[default]
    Ltr,
    Rtl,
}

impl DirSpec {
    pub fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }
}

/// 交叉轴对齐（`align-items` / `align-self`）。
///
/// `Baseline` 用字号近似第一行基线（`0.8em`）；无字号时回退 Start。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AlignSpec {
    #[default]
    Start,
    Center,
    End,
    Stretch,
    Baseline,
}

/// 主轴分布（justify-content）与多行 `align-content`。
///
/// `Stretch` 只对多行 `align-content` 有意义：把剩余交叉空间均分给各行。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JustifySpec {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Stretch,
}

/// overflow / overflow-x / overflow-y。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OverflowSpec {
    #[default]
    Visible,
    Hidden,
    Auto,
    Scroll,
}

/// Half-extent used to leave an overflow axis unclipped when `overflow-x` /
/// `overflow-y` disagree (`hidden` on one axis, `visible` on the other).
pub const OVERFLOW_OPEN_AXIS_EXTENT: f32 = 100_000.0;

impl OverflowSpec {
    pub fn scrolls(self) -> bool {
        matches!(self, Self::Auto | Self::Scroll)
    }

    /// `overflow: hidden` / `clip` — paint must clip descendants to the padding box.
    pub fn clips(self) -> bool {
        matches!(self, Self::Hidden | Self::Auto | Self::Scroll)
    }
}

/// Expand `bounds` on axes that do not clip so `overflow-x` / `overflow-y` can
/// clip independently without a second clip pipeline.
pub fn overflow_clip_rect(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    clip_x: bool,
    clip_y: bool,
) -> Option<(f32, f32, f32, f32)> {
    if !clip_x && !clip_y {
        return None;
    }
    let (mut rx, mut ry, mut rw, mut rh) = (x, y, width, height);
    if !clip_x {
        rx = x + width * 0.5 - OVERFLOW_OPEN_AXIS_EXTENT;
        rw = OVERFLOW_OPEN_AXIS_EXTENT * 2.0;
    }
    if !clip_y {
        ry = y + height * 0.5 - OVERFLOW_OPEN_AXIS_EXTENT;
        rh = OVERFLOW_OPEN_AXIS_EXTENT * 2.0;
    }
    Some((rx, ry, rw, rh))
}

/// CSS `display` 子集（作为布局意图枚举，非 CSSOM）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplaySpec {
    None,
    Flex,
    InlineFlex,
    Block,
    Grid,
    /// `inline-grid`：与 [`Self::Grid`] 同属网格容器；轨 / auto-fill 布局消费与 `grid` 相同。
    InlineGrid,
    /// `display: contents`：自身不生成盒子，子项提升到父级格式化上下文。
    Contents,
    /// `display: inline`：行内级；在块容器里参与 IFC 子集。
    Inline,
    /// `display: inline-block`：行内级盒子，可有独立宽高。
    InlineBlock,
}

impl DisplaySpec {
    /// `display: grid` 或 `inline-grid`。
    pub fn is_grid_container(self) -> bool {
        matches!(self, Self::Grid | Self::InlineGrid)
    }

    /// `display: flex` 或 `inline-flex`（网格轨在此惰性）。
    pub fn is_flex_container(self) -> bool {
        matches!(self, Self::Flex | Self::InlineFlex)
    }

    /// `display: contents`。
    pub fn is_contents(self) -> bool {
        matches!(self, Self::Contents)
    }

    /// 行内级（`inline` / `inline-block`）。flex/grid 项会被块化。
    pub fn is_inline_level(self) -> bool {
        matches!(self, Self::Inline | Self::InlineBlock)
    }
}

/// `text-align` 子集（IFC 行内对齐；块/列容器上的行内子项）。
///
/// `Start` / `End` are logical (follow [`LayoutStyle::dir`]). `Left` / `Right`
/// are physical and do not flip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextAlignSpec {
    #[default]
    Start,
    Center,
    End,
    Left,
    Right,
}

impl TextAlignSpec {
    /// Map to a physical main-axis justify keyword for IFC packing.
    pub fn to_justify(self, rtl: bool) -> JustifySpec {
        match self {
            Self::Center => JustifySpec::Center,
            Self::Left => JustifySpec::Start,
            Self::Right => JustifySpec::End,
            Self::Start => {
                if rtl {
                    JustifySpec::End
                } else {
                    JustifySpec::Start
                }
            }
            Self::End => {
                if rtl {
                    JustifySpec::Start
                } else {
                    JustifySpec::End
                }
            }
        }
    }
}

/// `white-space` 子集。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WhiteSpaceSpec {
    #[default]
    Normal,
    Nowrap,
    /// 保留空格与换行；按换行拆成多行量测。
    Pre,
}

/// `float` 子集（块/IFC：左/右浮动 + `clear`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FloatSpec {
    #[default]
    None,
    Left,
    Right,
}

impl FloatSpec {
    pub fn is_none(self) -> bool {
        matches!(self, Self::None)
    }
}

/// `clear` 子集。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ClearSpec {
    #[default]
    None,
    Left,
    Right,
    Both,
}

/// Track-list syntax the grid resolver will not pretend to honor.
///
/// Successful `repeat(auto-fit|auto-fill)` is stored on [`GridRepeatAuto`] and
/// expanded at layout; those variants remain as the repeat *kind*. [`Self::Subgrid`]
/// is an actual gap: nested grids do not inherit parent tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridTrackListUnsupported {
    /// `repeat(auto-fit, …)` (repeat kind; layout expands it).
    RepeatAutoFit,
    /// `repeat(auto-fill, …)` (repeat kind; layout expands it).
    RepeatAutoFill,
    /// `subgrid` — needs parent tracks in the grid resolver; not faked.
    Subgrid,
}

impl GridTrackListUnsupported {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RepeatAutoFit => "repeat(auto-fit)",
            Self::RepeatAutoFill => "repeat(auto-fill)",
            Self::Subgrid => "subgrid",
        }
    }

    pub fn is_auto_fit(self) -> bool {
        matches!(self, Self::RepeatAutoFit)
    }
}

/// `repeat(auto-fit|auto-fill, <track-list>)` 的可展开模式。
///
/// 可与前后固定轨混写：`prefix + repeat*N + suffix`。
/// 线名按 CSS 在每次重复时复制，接缝处相邻名字合并。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridRepeatAuto {
    pub kind: GridTrackListUnsupported,
    pub tracks: Vec<GridTrack>,
    #[serde(default)]
    pub prefix: Vec<GridTrack>,
    #[serde(default)]
    pub suffix: Vec<GridTrack>,
    /// Prefix track list line names (`prefix.len() + 1` 行，可为空)。
    #[serde(default)]
    pub prefix_line_names: Vec<Vec<String>>,
    /// In-repeat pattern line names (`tracks.len() + 1` 行)；布局按次数展开。
    #[serde(default)]
    pub pattern_line_names: Vec<Vec<String>>,
    /// Suffix track list line names.
    #[serde(default)]
    pub suffix_line_names: Vec<Vec<String>>,
}

impl Default for GridRepeatAuto {
    fn default() -> Self {
        Self {
            kind: GridTrackListUnsupported::RepeatAutoFit,
            tracks: Vec::new(),
            prefix: Vec::new(),
            suffix: Vec::new(),
            prefix_line_names: Vec::new(),
            pattern_line_names: Vec::new(),
            suffix_line_names: Vec::new(),
        }
    }
}

impl GridRepeatAuto {
    fn track_min(track: GridTrack, container: f32) -> f32 {
        let fixed = track.as_fixed_against(container).unwrap_or(0.0);
        track.min_px().max(fixed)
    }

    fn list_min(tracks: &[GridTrack], container: f32, gap: f32) -> f32 {
        if tracks.is_empty() {
            return 0.0;
        }
        let mut min = 0.0f32;
        for (i, track) in tracks.iter().copied().enumerate() {
            if i > 0 {
                min += gap.max(0.0);
            }
            min += Self::track_min(track, container);
        }
        min
    }

    /// How many pattern repetitions fit in `container` with `gap` between tracks.
    pub fn fill_count(&self, container: f32, gap: f32) -> usize {
        if self.tracks.is_empty() {
            return 1;
        }
        let gap = gap.max(0.0);
        let prefix = Self::list_min(&self.prefix, container, gap);
        let suffix = Self::list_min(&self.suffix, container, gap);
        let mut reserved = prefix + suffix;
        if !self.prefix.is_empty() {
            reserved += gap;
        }
        if !self.suffix.is_empty() {
            reserved += gap;
        }
        let remaining = (container.max(0.0) - reserved).max(0.0);
        let min_rep = Self::list_min(&self.tracks, container, gap);
        if min_rep <= 1e-6 {
            return 1;
        }
        let step = min_rep + gap;
        ((remaining + gap) / step).floor().max(1.0) as usize
    }

    /// Repeat the pattern `n` times (at least once) and wrap prefix/suffix.
    pub fn expand_n(&self, n: usize) -> Vec<GridTrack> {
        let n = n.max(1);
        let mut out =
            Vec::with_capacity(self.prefix.len() + n * self.tracks.len() + self.suffix.len());
        out.extend_from_slice(&self.prefix);
        for _ in 0..n {
            out.extend_from_slice(&self.tracks);
        }
        out.extend_from_slice(&self.suffix);
        out
    }

    /// `auto-fill`: as many repetitions as fit. `auto-fit`: same count; caller
    /// collapses empty tracks after placement.
    pub fn expand(&self, container: f32, gap: f32) -> Vec<GridTrack> {
        self.expand_n(self.fill_count(container, gap))
    }

    /// CSS: repeating a named-line pattern joins adjacent names at the seam.
    /// `repeat(3, [a] 1fr [b])` → `[a] 1fr [b a] 1fr [b a] 1fr [b]`.
    pub fn merge_line_name_pattern(pattern: &[Vec<String>], n: usize) -> Vec<Vec<String>> {
        if pattern.is_empty() || n == 0 {
            return Vec::new();
        }
        let mut out = pattern.to_vec();
        for _ in 1..n {
            out = Self::join_line_name_lists(&out, pattern);
        }
        out
    }

    pub fn join_line_name_lists(left: &[Vec<String>], right: &[Vec<String>]) -> Vec<Vec<String>> {
        if left.is_empty() {
            return right.to_vec();
        }
        if right.is_empty() {
            return left.to_vec();
        }
        let mut out = left.to_vec();
        if let Some(first) = right.first()
            && let Some(last) = out.last_mut()
        {
            last.extend(first.iter().cloned());
        }
        if right.len() > 1 {
            out.extend(right.iter().skip(1).cloned());
        }
        out
    }

    /// Expand stored prefix / pattern / suffix line names for `n` repetitions.
    pub fn expand_line_names(&self, n: usize) -> Vec<Vec<String>> {
        let n = n.max(1);
        let pattern = if self.pattern_line_names.is_empty() {
            if self.tracks.is_empty() {
                Vec::new()
            } else {
                vec![Vec::new(); self.tracks.len() + 1]
            }
        } else {
            self.pattern_line_names.clone()
        };
        let expanded = Self::merge_line_name_pattern(&pattern, n);
        let prefix = if self.prefix_line_names.is_empty() && !self.prefix.is_empty() {
            vec![Vec::new(); self.prefix.len() + 1]
        } else {
            self.prefix_line_names.clone()
        };
        let suffix = if self.suffix_line_names.is_empty() && !self.suffix.is_empty() {
            vec![Vec::new(); self.suffix.len() + 1]
        } else {
            self.suffix_line_names.clone()
        };
        let mid = Self::join_line_name_lists(&prefix, &expanded);
        Self::join_line_name_lists(&mid, &suffix)
    }

    pub fn has_line_names(&self) -> bool {
        self.pattern_line_names.iter().any(|line| !line.is_empty())
            || self.prefix_line_names.iter().any(|line| !line.is_empty())
            || self.suffix_line_names.iter().any(|line| !line.is_empty())
    }
}

/// `grid-auto-flow`（2D 自动放置：row / column，可选 dense）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridAutoFlow {
    Row,
    Column,
    RowDense,
    ColumnDense,
}

impl GridAutoFlow {
    pub fn is_column(self) -> bool {
        matches!(self, Self::Column | Self::ColumnDense)
    }

    pub fn is_dense(self) -> bool {
        matches!(self, Self::RowDense | Self::ColumnDense)
    }
}

/// CSS grid 线：`auto` / 1-based 索引（负值从末尾） / `span N` / 命名线。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GridLine {
    #[default]
    Auto,
    /// 1-based line index; negative counts from the end.
    Index(i32),
    Span(u16),
    /// Custom ident（`grid-column: main` / `[header-start]`）。第一根同名线。
    Name(String),
    /// `foo 2` / `2 foo`：第 N 根同名线（N ≥ 2）。
    NthName(String, u16),
}

impl GridLine {
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    pub fn as_name(&self) -> Option<&str> {
        match self {
            Self::Name(name) | Self::NthName(name, _) => Some(name.as_str()),
            _ => None,
        }
    }

    /// 1-based 同名线序号；匿名 / 非 Name 为 `None`。
    pub fn name_occurrence(&self) -> Option<u16> {
        match self {
            Self::Name(_) => Some(1),
            Self::NthName(_, n) => Some((*n).max(1)),
            _ => None,
        }
    }
}

/// Item `grid-column` / `grid-row` / `grid-area` placement.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GridPlacement {
    pub column_start: GridLine,
    pub column_end: GridLine,
    pub row_start: GridLine,
    pub row_end: GridLine,
    /// `grid-area: header` 命名区域（优先于线号）。
    #[serde(default)]
    pub area: Option<String>,
}

impl GridPlacement {
    pub fn is_auto(&self) -> bool {
        self.area.is_none()
            && matches!(self.column_start, GridLine::Auto)
            && matches!(self.column_end, GridLine::Auto)
            && matches!(self.row_start, GridLine::Auto)
            && matches!(self.row_end, GridLine::Auto)
    }
}

/// `grid-template-areas` 行×列名称表。`"."` 是空洞。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GridTemplateAreas {
    pub cells: Vec<Vec<String>>,
}

impl GridTemplateAreas {
    /// 命名区域的 (col, row, col_span, row_span)，0-based。
    pub fn lookup(&self, name: &str) -> Option<(usize, usize, usize, usize)> {
        if name.is_empty() || name == "." {
            return None;
        }
        let mut min_r = usize::MAX;
        let mut min_c = usize::MAX;
        let mut max_r = 0usize;
        let mut max_c = 0usize;
        let mut found = false;
        for (r, row) in self.cells.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                if cell == name {
                    found = true;
                    min_r = min_r.min(r);
                    min_c = min_c.min(c);
                    max_r = max_r.max(r);
                    max_c = max_c.max(c);
                }
            }
        }
        if !found {
            return None;
        }
        Some((
            min_c,
            min_r,
            max_c.saturating_sub(min_c) + 1,
            max_r.saturating_sub(min_r) + 1,
        ))
    }

    pub fn column_count(&self) -> usize {
        self.cells.iter().map(|row| row.len()).max().unwrap_or(0)
    }

    pub fn row_count(&self) -> usize {
        self.cells.len()
    }
}

/// `position` — Style Model 子集。
///
/// - `Static`：忽略 inset
/// - `Relative`：`top`/`left`/`right`/`bottom` 偏移进入 measure
/// - `Absolute`：measure 最小子集（脱流 + 相对 nearest positioned padding box）；
///   流内跳过；产品浮层仍走 Nana Overlay，不实现完整定位引擎
/// - `Fixed`：视口 containing block + inset 子集（脱流；根层绘制）
/// - `Sticky`：流内布局；滚动投影按 nearest scrollport + inset 钳制
///
/// **与 Nana Overlay 分工**：L2 Dialog/Popover/Drawer/ContextMenu 剥离 CSS
/// `fixed`/`sticky`，走 Overlay 合同；普通节点的 `position:fixed` 走本视口子集。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PositionSpec {
    #[default]
    Static,
    Relative,
    /// measure 脱流 + inset；非完整 CSS 定位引擎。
    Absolute,
    /// 视口固定定位子集（CB = 当前窗口/content viewport）。
    Fixed,
    /// 流内粘性定位：未滚动时与 static 同盒；滚动后按 inset 钳制。
    Sticky,
}

impl PositionSpec {
    /// 已兑现全部定位模式；保留给诊断兼容。
    pub fn is_unsupported_positioning(self) -> bool {
        false
    }

    pub fn applies_relative_offset(self) -> bool {
        matches!(self, Self::Relative)
    }

    /// 为绝对定位后代建立 containing block（relative / absolute / fixed）。
    pub fn establishes_containing_block(self) -> bool {
        matches!(self, Self::Relative | Self::Absolute | Self::Fixed)
    }

    pub fn is_out_of_flow_absolute(self) -> bool {
        matches!(self, Self::Absolute)
    }

    pub fn is_out_of_flow_fixed(self) -> bool {
        matches!(self, Self::Fixed)
    }

    /// `absolute` 或 `fixed`：均脱正常文档流。
    pub fn is_out_of_flow(self) -> bool {
        matches!(self, Self::Absolute | Self::Fixed)
    }

    /// CSS positioned: not `static`. `z-index` on these creates a stacking context.
    pub fn is_positioned(self) -> bool {
        !matches!(self, Self::Static)
    }
}

/// box-sizing：`BorderBox`（默认）声明宽含 padding + border；`ContentBox` 声明宽为内容，
/// border box = 声明 + padding + border（T-B08/T-B09）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BoxSizing {
    #[default]
    BorderBox,
    ContentBox,
}

/// flex-wrap 意图。
///
/// `measure_layout`：`Wrap` / `WrapReverse` 按主轴分行兑现（`WrapReverse` 反转行序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}

/// 网格轨道（轻量；由 L1 解析填入）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GridTrack {
    Px(f32),
    /// `%` of the grid container's content size on that axis（布局时相对 `content_w` 兑现）。
    Percent(f32),
    Fr(f32),
    /// `minmax(min, Nfr)` 或 `minmax(min, maxPx)`（`max_px` 有值时 `fr` 仍参与分配后钳制）。
    MinMax {
        min_px: f32,
        fr: f32,
        max_px: Option<f32>,
    },
    Auto,
}

impl GridTrack {
    pub fn as_fixed_px(self) -> Option<f32> {
        match self {
            Self::Px(v) => Some(v),
            _ => None,
        }
    }

    /// 固定轨（含 `%`）相对容器尺寸兑现为 px。
    pub fn as_fixed_against(self, container: f32) -> Option<f32> {
        match self {
            Self::Px(v) => Some(v.max(0.0)),
            Self::Percent(p) => Some((container.max(0.0) * p.clamp(0.0, 100.0) / 100.0).max(0.0)),
            _ => None,
        }
    }

    pub fn is_flexible(self) -> bool {
        // `auto` is content-sized (not a free-space fraction).
        matches!(self, Self::Fr(_) | Self::MinMax { .. })
    }

    /// `fr` 权重；固定 / `auto` 轨 `None`（`auto` 需 intrinsic，不得当成 `1fr`）。
    pub fn fr_weight(self) -> Option<f32> {
        match self {
            Self::Fr(f) => Some(f.max(0.0)),
            Self::MinMax { fr, .. } => Some(fr.max(0.0)),
            Self::Auto | Self::Px(_) | Self::Percent(_) => None,
        }
    }

    /// minmax 下限；其它轨 0。
    pub fn min_px(self) -> f32 {
        match self {
            Self::MinMax { min_px, .. } => min_px.max(0.0),
            _ => 0.0,
        }
    }

    /// minmax 像素上限（`minmax(min, maxPx)`）；`minmax(min, Nfr)` 为 `None`。
    pub fn max_px(self) -> Option<f32> {
        match self {
            Self::MinMax {
                min_px,
                max_px: Some(max),
                ..
            } => Some(max.max(min_px).max(0.0)),
            _ => None,
        }
    }

    /// Flex 近似：固定轨 → Px/%；`fr` → Fill；`auto` → Shrink（内容尺寸）。
    pub fn as_row_main_length(self) -> LengthSpec {
        match self {
            Self::Px(px) => LengthSpec::Px(px),
            Self::Percent(p) => LengthSpec::Percent(p.clamp(0.0, 100.0)),
            Self::Fr(_) | Self::MinMax { .. } => LengthSpec::Fill,
            Self::Auto => LengthSpec::Shrink,
        }
    }
}

/// Resolve column track widths for `content_w` (gap between tracks).
///
/// Lightweight CSS-like pass: distribute free space by `fr` weight, then freeze
/// tracks that violate `minmax` min/max and redistribute (may freeze many at once).
///
/// `auto` tracks without [`resolve_grid_track_sizes`] intrinsics size to 0 here —
/// callers that know content contributions should pass them via that API.
pub fn resolve_grid_column_widths(tracks: &[GridTrack], content_w: f32, gap: f32) -> Vec<f32> {
    resolve_grid_track_sizes(tracks, content_w, gap, &[])
}

/// Like [`resolve_grid_column_widths`], but `auto_sizes[i]` supplies the content
/// contribution for `GridTrack::Auto` (missing entries → 0).
pub fn resolve_grid_track_sizes(
    tracks: &[GridTrack],
    content: f32,
    gap: f32,
    auto_sizes: &[f32],
) -> Vec<f32> {
    let n = tracks.len();
    if n == 0 {
        return Vec::new();
    }
    let gap_total = gap * n.saturating_sub(1) as f32;
    let mut widths = vec![0.0f32; n];
    let mut fixed_sum = 0.0f32;
    // (index, weight, min_px, max_px)
    let mut active: Vec<(usize, f32, f32, Option<f32>)> = Vec::new();

    for (i, track) in tracks.iter().copied().enumerate() {
        if let Some(px) = track.as_fixed_against(content) {
            widths[i] = px.max(0.0);
            fixed_sum += widths[i];
        } else if matches!(track, GridTrack::Auto) {
            let px = auto_sizes.get(i).copied().unwrap_or(0.0).max(0.0);
            widths[i] = px;
            fixed_sum += px;
        } else if let Some(weight) = track.fr_weight() {
            active.push((i, weight.max(0.0), track.min_px(), track.max_px()));
        } else {
            widths[i] = 0.0;
        }
    }

    let mut free = (content - fixed_sum - gap_total).max(0.0);
    loop {
        if active.is_empty() {
            break;
        }
        let fr_total: f32 = active.iter().map(|(_, w, _, _)| *w).sum();
        if fr_total <= 1e-6 {
            let share = free / active.len() as f32;
            for (i, _, min, max) in active.drain(..) {
                let mut w = share.max(min);
                if let Some(max) = max {
                    w = w.min(max);
                }
                widths[i] = w;
            }
            break;
        }

        // Collect all violations in one pass (multi-min / multi-max freeze).
        let mut freeze: Vec<(usize, f32)> = Vec::new(); // (active_idx, frozen_width)
        for (idx, &(_, w, min, max)) in active.iter().enumerate() {
            let share = free * (w / fr_total);
            if share + 1e-3 < min {
                freeze.push((idx, min));
            } else if let Some(max) = max
                && share > max + 1e-3
            {
                freeze.push((idx, max));
            }
        }

        if freeze.is_empty() {
            for (i, w, min, max) in active.drain(..) {
                let mut width = (free * (w / fr_total)).max(min);
                if let Some(max) = max {
                    width = width.min(max);
                }
                widths[i] = width;
            }
            break;
        }

        freeze.sort_by_key(|(idx, _)| *idx);
        for (idx, frozen_w) in freeze.into_iter().rev() {
            let (i, _, _, _) = active.remove(idx);
            widths[i] = frozen_w;
            free = (free - frozen_w).max(0.0);
        }
    }

    widths
}

/// 视口相对单位轴（`vw` / `vh` / `vmin` / `vmax`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewportAxis {
    /// `vw` — 相对 viewport 宽度。
    Width,
    /// `vh` — 相对 viewport 高度。
    Height,
    /// `vmin` — min(vw, vh)。
    Min,
    /// `vmax` — max(vw, vh)。
    Max,
}

impl ViewportAxis {
    pub fn base(self, viewport_w: f32, viewport_h: f32) -> f32 {
        let w = viewport_w.max(0.0);
        let h = viewport_h.max(0.0);
        match self {
            Self::Width => w,
            Self::Height => h,
            Self::Min => w.min(h),
            Self::Max => w.max(h),
        }
    }
}

/// CSS Values：`em` / `rem` 解析上下文（逻辑像素）。
///
/// - `element_px`：当前元素 `font-size`（`em`）
/// - `root_px`：根元素 `font-size`（`rem`）
///
/// 缺省均为 CSS initial `medium` ≈ 16px（非 Nana `UI_BASE_TEXT_SIZE`）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FontSizeContext {
    pub root_px: f32,
    pub element_px: f32,
}

impl Default for FontSizeContext {
    fn default() -> Self {
        Self {
            root_px: 16.0,
            element_px: 16.0,
        }
    }
}

impl FontSizeContext {
    pub fn new(root_px: f32, element_px: f32) -> Self {
        Self {
            root_px: root_px.max(0.0),
            element_px: element_px.max(0.0),
        }
    }

    pub fn uniform(px: f32) -> Self {
        let px = px.max(0.0);
        Self {
            root_px: px,
            element_px: px,
        }
    }
}

/// CSS `line-height` 子集：无单位倍数或绝对 px（`normal` → 不写入，用引擎默认）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LineHeightSpec {
    /// Unitless / `%` → 相对当前 `font-size` 的倍数。
    Relative(f32),
    /// `Npx` / resolved length.
    Absolute(f32),
}

impl LineHeightSpec {
    /// Used line-box height in CSS px for a given computed `font-size`.
    pub fn resolve_px(self, font_px: f32) -> f32 {
        match self {
            Self::Absolute(v) => v.max(font_px).max(0.0),
            Self::Relative(f) => (font_px * f.max(0.0)).max(font_px),
        }
    }
}

/// Intrinsic text line-box height when `font-size` is known.
///
/// Used by Scene/Runtime letter-spacing glyph rows (which otherwise measure ~0 tall)
/// and by measure for typography leaves under `height:auto`.
pub fn text_line_box_height_px(font_px: f32, line_height: Option<LineHeightSpec>) -> f32 {
    let px = font_px.max(0.0);
    match line_height {
        Some(spec) => spec.resolve_px(px),
        None => px * 1.2,
    }
}

/// Ascent ratio used by [`LayoutStyle::approximate_baseline`] (0.8em).
pub const TEXT_APPROX_ASCENT_EM: f32 = 0.8;

/// Center of a CJK-oriented em square inside a line, measured from the line top.
///
/// cosmic-text 0.19 splits extra leading above and below the (ascent+descent)
/// box, then places the baseline at `ascent` into that box. Nana approximates
/// ascent as 0.8em; the em square sits on the baseline, so its center is above
/// the line-box midpoint by half the descent.
pub fn glyph_box_center_from_line_top(line_height: f32, font_px: f32) -> f32 {
    let font_px = font_px.max(0.0);
    let ascent = font_px * TEXT_APPROX_ASCENT_EM;
    let descent = (font_px - ascent).max(0.0);
    let glyph_height = ascent + descent;
    let centering = (line_height - glyph_height) * 0.5;
    centering + ascent - glyph_height * 0.5
}

/// Top of a square `extent` whose center matches the text line box.
pub fn icon_y_on_text_glyph_center(
    text_bounds_y: f32,
    text_bounds_height: f32,
    font_px: f32,
    line_height: Option<LineHeightSpec>,
    vertical_center: bool,
    extent: f32,
) -> f32 {
    if vertical_center {
        text_bounds_y + (text_bounds_height - extent) * 0.5
    } else {
        let line_h = text_line_box_height_px(font_px, line_height);
        text_bounds_y + line_h * 0.5 - extent * 0.5
    }
}

/// 可参与 `min`/`max`/`clamp` 的轻量长度原子（Copy；calc 在解析期折进这些变体，非完整 AST）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LengthAtom {
    Px(f32),
    Percent(f32),
    /// `Nem`（相对元素 font-size）。
    Em(f32),
    /// `Nrem`（相对根 font-size）。
    Rem(f32),
    Viewport {
        axis: ViewportAxis,
        /// 例如 `50vh` → value=50。
        value: f32,
    },
    CalcPercent {
        percent: f32,
        offset_px: f32,
    },
    CalcViewport {
        axis: ViewportAxis,
        value: f32,
        offset_px: f32,
    },
    /// `calc(Nem ± Mpx)`。
    CalcEm {
        em: f32,
        offset_px: f32,
    },
    /// `calc(Nrem ± Mpx)`。
    CalcRem {
        rem: f32,
        offset_px: f32,
    },
}

impl LengthAtom {
    pub fn resolve_with(
        self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> Option<f32> {
        self.resolve_with_fonts(percent_base, viewport, FontSizeContext::default())
    }

    /// `true` when resolution reads the viewport.
    pub fn depends_on_viewport(self) -> bool {
        matches!(self, Self::Viewport { .. } | Self::CalcViewport { .. })
    }

    pub fn resolve_with_fonts(
        self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> Option<f32> {
        match self {
            Self::Px(v) => Some(v),
            Self::Percent(p) => percent_base.map(|base| base * p / 100.0),
            Self::Em(v) => Some(fonts.element_px * v),
            Self::Rem(v) => Some(fonts.root_px * v),
            Self::Viewport { axis, value } => {
                viewport.map(|(w, h)| axis.base(w, h) * value / 100.0)
            }
            Self::CalcPercent { percent, offset_px } => {
                percent_base.map(|base| base * percent / 100.0 + offset_px)
            }
            Self::CalcViewport {
                axis,
                value,
                offset_px,
            } => viewport.map(|(w, h)| axis.base(w, h) * value / 100.0 + offset_px),
            Self::CalcEm { em, offset_px } => Some(fonts.element_px * em + offset_px),
            Self::CalcRem { rem, offset_px } => Some(fonts.root_px * rem + offset_px),
        }
    }
}

/// 宽度 / 高度规格。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LengthSpec {
    Px(f32),
    /// 相对父 content box 的百分比（calc 可超出 0..=100，不钳制）。
    Percent(f32),
    /// `Nem`（相对元素 font-size；缺省 16px）。
    Em(f32),
    /// `Nrem`（相对根 font-size；缺省 16px）。
    Rem(f32),
    /// 轻量 `calc(P% ± Npx)`（非完整 calc AST）。
    CalcPercentOffset {
        percent: f32,
        offset_px: f32,
    },
    /// `Nvw` / `Nvh` / `Nvmin` / `Nvmax`。
    Viewport {
        axis: ViewportAxis,
        value: f32,
    },
    /// 轻量 `calc(Nv* ± Mpx)` / 裸 `100vh - 32px`。
    CalcViewportOffset {
        axis: ViewportAxis,
        value: f32,
        offset_px: f32,
    },
    /// 轻量 `calc(Nem ± Mpx)`。
    CalcEmOffset {
        em: f32,
        offset_px: f32,
    },
    /// 轻量 `calc(Nrem ± Mpx)`。
    CalcRemOffset {
        rem: f32,
        offset_px: f32,
    },
    /// `min(a, b)` 两原子。
    Min2(LengthAtom, LengthAtom),
    /// `max(a, b)` 两原子（长度上下文；非 grid `minmax`）。
    Max2(LengthAtom, LengthAtom),
    /// `clamp(min, val, max)`。
    Clamp3(LengthAtom, LengthAtom, LengthAtom),
    Fill,
    Shrink,
    Auto,
    /// CSS `min-content`：尽量收缩的固有尺寸。
    MinContent,
    /// CSS `max-content`：不折行的固有尺寸。
    MaxContent,
    /// CSS `fit-content`：`min(max-content, max(min-content, available))`。
    FitContent,
}

impl LengthSpec {
    /// 解析为逻辑像素；无 viewport 时视口单位返回 `None`。
    pub fn resolve_px(self, percent_base: Option<f32>) -> Option<f32> {
        self.resolve_with(percent_base, None)
    }

    /// `true` when resolution reads the viewport, so this length moves on a
    /// viewport change even when its containing block does not.
    pub fn depends_on_viewport(self) -> bool {
        match self {
            Self::Viewport { .. } | Self::CalcViewportOffset { .. } => true,
            Self::Min2(a, b) | Self::Max2(a, b) => {
                a.depends_on_viewport() || b.depends_on_viewport()
            }
            Self::Clamp3(a, b, c) => {
                a.depends_on_viewport() || b.depends_on_viewport() || c.depends_on_viewport()
            }
            _ => false,
        }
    }

    /// 解析为逻辑像素；`Fill`/`Shrink`/`Auto` 返回 `None`。
    /// `em`/`rem` 使用 [`FontSizeContext::default`]（16px）。
    pub fn resolve_with(
        self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> Option<f32> {
        self.resolve_with_fonts(percent_base, viewport, FontSizeContext::default())
    }

    /// 同 [`Self::resolve_with`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolve_with_fonts(
        self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> Option<f32> {
        match self {
            Self::Px(v) => Some(v),
            Self::Percent(p) => percent_base.map(|base| base * p / 100.0),
            Self::Em(v) => Some(fonts.element_px * v),
            Self::Rem(v) => Some(fonts.root_px * v),
            Self::CalcPercentOffset { percent, offset_px } => {
                percent_base.map(|base| base * percent / 100.0 + offset_px)
            }
            Self::Viewport { axis, value } => {
                viewport.map(|(w, h)| axis.base(w, h) * value / 100.0)
            }
            Self::CalcViewportOffset {
                axis,
                value,
                offset_px,
            } => viewport.map(|(w, h)| axis.base(w, h) * value / 100.0 + offset_px),
            Self::CalcEmOffset { em, offset_px } => Some(fonts.element_px * em + offset_px),
            Self::CalcRemOffset { rem, offset_px } => Some(fonts.root_px * rem + offset_px),
            Self::Min2(a, b) => {
                let av = a.resolve_with_fonts(percent_base, viewport, fonts)?;
                let bv = b.resolve_with_fonts(percent_base, viewport, fonts)?;
                Some(av.min(bv))
            }
            Self::Max2(a, b) => {
                let av = a.resolve_with_fonts(percent_base, viewport, fonts)?;
                let bv = b.resolve_with_fonts(percent_base, viewport, fonts)?;
                Some(av.max(bv))
            }
            Self::Clamp3(min, val, max) => {
                let lo = min.resolve_with_fonts(percent_base, viewport, fonts)?;
                let v = val.resolve_with_fonts(percent_base, viewport, fonts)?;
                let hi = max.resolve_with_fonts(percent_base, viewport, fonts)?;
                Some(v.clamp(lo.min(hi), lo.max(hi)))
            }
            Self::Fill
            | Self::Shrink
            | Self::Auto
            | Self::MinContent
            | Self::MaxContent
            | Self::FitContent => None,
        }
    }

    /// 非负长度（width/height/padding/min-size）；`None` 若无法解析。
    /// `em`/`rem` 使用 [`FontSizeContext::default`]（16px）。
    pub fn resolve_non_negative(
        self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> Option<f32> {
        self.resolve_non_negative_fonts(percent_base, viewport, FontSizeContext::default())
    }

    /// 同 [`Self::resolve_non_negative`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolve_non_negative_fonts(
        self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> Option<f32> {
        self.resolve_with_fonts(percent_base, viewport, fonts)
            .map(|v| v.max(0.0))
    }

    pub fn is_content_sized(self) -> bool {
        matches!(
            self,
            Self::Shrink | Self::MinContent | Self::MaxContent | Self::FitContent
        )
    }

    /// 声明尺寸是否按 content-box 字面长度理解（非 Fill 分配）。
    pub fn is_definite_declared(self) -> bool {
        matches!(
            self,
            Self::Px(_)
                | Self::Percent(_)
                | Self::Em(_)
                | Self::Rem(_)
                | Self::CalcPercentOffset { .. }
                | Self::Viewport { .. }
                | Self::CalcViewportOffset { .. }
                | Self::CalcEmOffset { .. }
                | Self::CalcRemOffset { .. }
                | Self::Min2(_, _)
                | Self::Max2(_, _)
                | Self::Clamp3(_, _, _)
        )
    }
}

/// 四边 inset（padding / margin，逻辑像素）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PaddingSpec {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl PaddingSpec {
    pub fn uniform(v: f32) -> Self {
        let v = v.max(0.0);
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    pub fn is_zero(self) -> bool {
        self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0 && self.left == 0.0
    }
}

/// 父盒尺寸上下文（百分比 / 定高链）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ParentBox {
    pub width: Option<f32>,
    pub height: Option<f32>,
}

/// CSS `visibility` (layout placeholder vs paint/hit-test).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VisibilitySpec {
    #[default]
    Visible,
    Hidden,
}

/// GPU cap for comma-separated `box-shadow` layers.
pub const MAX_BOX_SHADOWS: usize = 4;

/// One `box-shadow` layer (physical px after parse).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoxShadowSpec {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: [f32; 4],
    /// CSS `inset`. Outset (false) paints outside the border box.
    #[serde(default)]
    pub inset: bool,
}

/// GPU-capable CSS `mix-blend-mode` subset. Unknown values fail closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MixBlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
}

impl MixBlendMode {
    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(Self::Normal),
            "multiply" => Some(Self::Multiply),
            "screen" => Some(Self::Screen),
            _ => None,
        }
    }

    pub fn is_normal(self) -> bool {
        matches!(self, Self::Normal)
    }

    /// Dest-group BlendState selector (`0` normal, `1` multiply, `2` screen).
    pub fn gpu_index(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::Multiply => 1,
            Self::Screen => 2,
        }
    }
}

/// CSS `pointer-events` (`auto` hits, `none` skips).
///
/// Inherited. Unspecified specified-value is [`None`] on
/// [`LayoutStyle::pointer_events`]; used value is [`Self::inherit_from`].
/// Root initial used value is [`Self::Auto`]. Unknown keywords fail closed
/// and do not write a specified value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PointerEventsSpec {
    #[default]
    Auto,
    None,
}

impl PointerEventsSpec {
    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub fn hittable(self) -> bool {
        matches!(self, Self::Auto)
    }

    /// Used value: specified wins, otherwise inherit `parent`.
    pub fn inherit_from(specified: Option<Self>, parent: Self) -> Self {
        specified.unwrap_or(parent)
    }
}

/// CSS `border-style` subset. Dashed/dotted stroke the existing rounded-box
/// SDF ring; `double` / 3D keywords occupy used width but do not paint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BorderStyle {
    None,
    Solid,
    Dashed,
    Dotted,
    /// Parsed but not stroked (`double` / groove / ridge / inset / outset).
    Unsupported,
}

impl BorderStyle {
    /// Packed into [`LayoutStyle::paint_border_style_codes`] / the quad shader.
    pub const SHADER_SOLID: u8 = 0;
    pub const SHADER_DASHED: u8 = 1;
    pub const SHADER_DOTTED: u8 = 2;

    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "none" | "hidden" => Some(Self::None),
            "solid" => Some(Self::Solid),
            "dashed" => Some(Self::Dashed),
            "dotted" => Some(Self::Dotted),
            "double" | "groove" | "ridge" | "inset" | "outset" => Some(Self::Unsupported),
            _ => None,
        }
    }

    /// `none` / `hidden` zero the used border width (CSS).
    pub fn zeros_used_width(self) -> bool {
        matches!(self, Self::None)
    }

    /// Solid / dashed / dotted (and unspecified-compat) use the rounded-box stroke.
    pub fn paints_stroke(self) -> bool {
        matches!(self, Self::Solid | Self::Dashed | Self::Dotted)
    }

    pub fn shader_code(self) -> u8 {
        match self {
            Self::Dashed => Self::SHADER_DASHED,
            Self::Dotted => Self::SHADER_DOTTED,
            Self::None | Self::Solid | Self::Unsupported => Self::SHADER_SOLID,
        }
    }
}

fn used_border_width(width: f32, style: Option<BorderStyle>) -> f32 {
    match style {
        Some(s) if s.zeros_used_width() => 0.0,
        // Unspecified style keeps the declared width (legacy NanaUI / T-B09).
        // Explicit `none` / `hidden` still zero the used width.
        None | Some(_) => width.max(0.0),
    }
}

fn paint_border_width(width: f32, style: Option<BorderStyle>, color: Option<[f32; 4]>) -> f32 {
    if width > 0.0 && color.is_some() && edge_paints_stroke(style) {
        width
    } else {
        0.0
    }
}

fn edge_paints_stroke(style: Option<BorderStyle>) -> bool {
    style.map(BorderStyle::paints_stroke).unwrap_or(true)
}

/// CSS `outline-style` subset. Dashed/dotted fail closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OutlineStyle {
    #[default]
    None,
    Solid,
}

/// Paint-only CSS `outline` (does not affect layout).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct OutlineSpec {
    #[serde(default)]
    pub width: f32,
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    #[serde(default)]
    pub style: OutlineStyle,
}

impl OutlineSpec {
    pub fn is_active(self) -> bool {
        self.style == OutlineStyle::Solid && self.width > 0.0
    }
}

/// Single-layer `text-shadow` (physical px after parse; blur is paint-hint only).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextShadowSpec {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: [f32; 4],
}

/// CSS `text-decoration-line` subset Scene can stroke (underline / line-through).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextDecorationLine {
    pub underline: bool,
    pub line_through: bool,
}

impl TextDecorationLine {
    pub fn is_active(self) -> bool {
        self.underline || self.line_through
    }
}

/// One OpenType `font-feature-settings` tag (`"liga" 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FontFeatureSetting {
    pub tag: [u8; 4],
    pub value: u32,
}

/// One stop in a CSS `linear-gradient` (position 0..=1 along the gradient line).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub position: f32,
    pub color: [f32; 4],
}

/// CSS `linear-gradient` background / mask (2..=8 stops, angle in degrees).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearGradient {
    /// CSS angle: `0deg` = to top, `90deg` = to right.
    pub angle_deg: f32,
    pub stops: Vec<GradientStop>,
}

impl LinearGradient {
    pub fn stop_count(&self) -> usize {
        self.stops.len().min(8)
    }
}

/// CSS `radial-gradient` background / mask (2..=8 stops).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadialGradient {
    /// `true` = `circle`, `false` = `ellipse`.
    pub circle: bool,
    /// Center in border-box normalized coordinates (0..1).
    pub center: [f32; 2],
    pub stops: Vec<GradientStop>,
}

impl RadialGradient {
    pub fn stop_count(&self) -> usize {
        self.stops.len().min(8)
    }
}

/// Parsed CSS gradient fill (`linear-gradient` or `radial-gradient`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CssGradient {
    Linear(LinearGradient),
    Radial(RadialGradient),
}

impl CssGradient {
    pub fn stops(&self) -> &[GradientStop] {
        match self {
            Self::Linear(g) => &g.stops,
            Self::Radial(g) => &g.stops,
        }
    }

    pub fn stop_count(&self) -> usize {
        match self {
            Self::Linear(g) => g.stop_count(),
            Self::Radial(g) => g.stop_count(),
        }
    }
}

/// Product cap for comma-separated `background-image` layers (plus `<img src>`).
pub const MAX_BACKGROUND_LAYERS: usize = 8;

/// `background-size` / `object-fit` mapping for `url()` fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BackgroundImageFit {
    Cover,
    Contain,
    Stretch,
    /// CSS initial `background-size: auto` / `object-fit: none`.
    #[default]
    Auto,
    /// Explicit `background-size` lengths (`32px`, `50%`, `auto 24px`).
    Length,
}

/// CSS `background-repeat` (shader tiling; `space`/`round` map to [`Self::Repeat`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BackgroundRepeat {
    NoRepeat,
    /// CSS initial value for unspecified `background-repeat`.
    #[default]
    Repeat,
    RepeatX,
    RepeatY,
}

/// `background-position` / `object-position` (percent of free space, or px).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BackgroundPosition {
    pub x: LengthSpec,
    pub y: LengthSpec,
}

impl Default for BackgroundPosition {
    fn default() -> Self {
        Self {
            x: LengthSpec::Percent(0.0),
            y: LengthSpec::Percent(0.0),
        }
    }
}

impl BackgroundPosition {
    pub fn center() -> Self {
        Self {
            x: LengthSpec::Percent(50.0),
            y: LengthSpec::Percent(50.0),
        }
    }
}

/// Parsed `background-image` (`linear-gradient`, `radial-gradient`, or `url()`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BackgroundImage {
    Gradient(CssGradient),
    Url {
        url: String,
        fit: BackgroundImageFit,
        #[serde(default)]
        size_width: Option<LengthSpec>,
        #[serde(default)]
        size_height: Option<LengthSpec>,
        #[serde(default)]
        position: BackgroundPosition,
        #[serde(default)]
        repeat: BackgroundRepeat,
    },
}

impl BackgroundImage {
    pub fn url(url: impl Into<String>) -> Self {
        Self::url_with_fit(url, BackgroundImageFit::Auto)
    }

    pub fn url_with_fit(url: impl Into<String>, fit: BackgroundImageFit) -> Self {
        Self::Url {
            url: url.into(),
            fit,
            size_width: None,
            size_height: None,
            position: BackgroundPosition::default(),
            repeat: BackgroundRepeat::Repeat,
        }
    }

    pub fn url_str(&self) -> Option<&str> {
        match self {
            Self::Url { url, .. } => Some(url.as_str()),
            Self::Gradient(_) => None,
        }
    }
}

/// One `border-image-slice` edge. Unitless numbers are source pixels;
/// percentages are of the source image's corresponding axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BorderImageSlice {
    Number(f32),
    Percent(f32),
}

impl Default for BorderImageSlice {
    fn default() -> Self {
        Self::Percent(100.0)
    }
}

impl BorderImageSlice {
    pub fn to_px(self, image_len: f32) -> f32 {
        match self {
            Self::Number(value) => value.max(0.0),
            Self::Percent(percent) => (percent.max(0.0) / 100.0) * image_len.max(0.0),
        }
    }
}

/// One 9-slice tile: dest rect in border-box px, source UV 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderImageTile {
    pub dest_x: f32,
    pub dest_y: f32,
    pub dest_w: f32,
    pub dest_h: f32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

/// Minimal CSS `border-image`: `url()` or `linear-gradient` source, slice,
/// optional `fill`. Width defaults to `1`×slice; outset/repeat stay stretch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BorderImageSpec {
    pub source: BackgroundImage,
    /// Top, right, bottom, left.
    pub slice: [BorderImageSlice; 4],
    pub fill: bool,
}

impl BorderImageSpec {
    pub fn from_source(source: BackgroundImage) -> Self {
        Self {
            source,
            slice: [BorderImageSlice::Percent(100.0); 4],
            fill: false,
        }
    }

    pub fn paints_linear_or_url(&self) -> bool {
        match &self.source {
            BackgroundImage::Url { .. } => true,
            BackgroundImage::Gradient(CssGradient::Linear(_)) => true,
            BackgroundImage::Gradient(CssGradient::Radial(_)) => false,
        }
    }

    /// `image_w` / `image_h` are the CSS border-image intrinsic size (bitmap
    /// pixels, or the border box for a gradient).
    pub fn tiles(
        &self,
        image_w: f32,
        image_h: f32,
        box_w: f32,
        box_h: f32,
    ) -> Vec<BorderImageTile> {
        let image_w = image_w.max(1.0);
        let image_h = image_h.max(1.0);
        let mut top_px = self.slice[0].to_px(image_h);
        let mut right_px = self.slice[1].to_px(image_w);
        let mut bottom_px = self.slice[2].to_px(image_h);
        let mut left_px = self.slice[3].to_px(image_w);
        (top_px, bottom_px) = clamp_pair(top_px, bottom_px, image_h);
        (left_px, right_px) = clamp_pair(left_px, right_px, image_w);
        let u_l = left_px / image_w;
        let u_r = right_px / image_w;
        let v_t = top_px / image_h;
        let v_b = bottom_px / image_h;
        let (dest_top, dest_bottom) = clamp_pair(top_px, bottom_px, box_h.max(0.0));
        let (dest_left, dest_right) = clamp_pair(left_px, right_px, box_w.max(0.0));
        let mid_w = (box_w - dest_left - dest_right).max(0.0);
        let mid_h = (box_h - dest_top - dest_bottom).max(0.0);
        let mut tiles = vec![
            BorderImageTile {
                dest_x: 0.0,
                dest_y: 0.0,
                dest_w: dest_left,
                dest_h: dest_top,
                u0: 0.0,
                v0: 0.0,
                u1: u_l,
                v1: v_t,
            },
            BorderImageTile {
                dest_x: dest_left,
                dest_y: 0.0,
                dest_w: mid_w,
                dest_h: dest_top,
                u0: u_l,
                v0: 0.0,
                u1: 1.0 - u_r,
                v1: v_t,
            },
            BorderImageTile {
                dest_x: dest_left + mid_w,
                dest_y: 0.0,
                dest_w: dest_right,
                dest_h: dest_top,
                u0: 1.0 - u_r,
                v0: 0.0,
                u1: 1.0,
                v1: v_t,
            },
            BorderImageTile {
                dest_x: 0.0,
                dest_y: dest_top,
                dest_w: dest_left,
                dest_h: mid_h,
                u0: 0.0,
                v0: v_t,
                u1: u_l,
                v1: 1.0 - v_b,
            },
        ];
        if self.fill {
            tiles.push(BorderImageTile {
                dest_x: dest_left,
                dest_y: dest_top,
                dest_w: mid_w,
                dest_h: mid_h,
                u0: u_l,
                v0: v_t,
                u1: 1.0 - u_r,
                v1: 1.0 - v_b,
            });
        }
        tiles.extend([
            BorderImageTile {
                dest_x: dest_left + mid_w,
                dest_y: dest_top,
                dest_w: dest_right,
                dest_h: mid_h,
                u0: 1.0 - u_r,
                v0: v_t,
                u1: 1.0,
                v1: 1.0 - v_b,
            },
            BorderImageTile {
                dest_x: 0.0,
                dest_y: dest_top + mid_h,
                dest_w: dest_left,
                dest_h: dest_bottom,
                u0: 0.0,
                v0: 1.0 - v_b,
                u1: u_l,
                v1: 1.0,
            },
            BorderImageTile {
                dest_x: dest_left,
                dest_y: dest_top + mid_h,
                dest_w: mid_w,
                dest_h: dest_bottom,
                u0: u_l,
                v0: 1.0 - v_b,
                u1: 1.0 - u_r,
                v1: 1.0,
            },
            BorderImageTile {
                dest_x: dest_left + mid_w,
                dest_y: dest_top + mid_h,
                dest_w: dest_right,
                dest_h: dest_bottom,
                u0: 1.0 - u_r,
                v0: 1.0 - v_b,
                u1: 1.0,
                v1: 1.0,
            },
        ]);
        tiles.retain(|tile| {
            tile.dest_w > 0.0001
                && tile.dest_h > 0.0001
                && (tile.u1 - tile.u0) > 1.0e-6
                && (tile.v1 - tile.v0) > 1.0e-6
        });
        tiles
    }
}

fn clamp_pair(a: f32, b: f32, max: f32) -> (f32, f32) {
    let max = max.max(0.0);
    let sum = a + b;
    if sum > max && sum > 0.0 {
        let scale = max / sum;
        (a * scale, b * scale)
    } else {
        (a, b)
    }
}

/// `clip-path: inset(...)` components (lengths resolve against the border box).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipInset {
    pub top: LengthSpec,
    pub right: LengthSpec,
    pub bottom: LengthSpec,
    pub left: LengthSpec,
    pub round: Option<LengthSpec>,
}

impl ClipInset {
    /// Resolve inset offsets to physical px `[top, right, bottom, left]`.
    pub fn resolve_offsets(&self, width: f32, height: f32) -> [f32; 4] {
        [
            resolve_inset_edge(self.top, width, height, true),
            resolve_inset_edge(self.right, width, height, false),
            resolve_inset_edge(self.bottom, width, height, true),
            resolve_inset_edge(self.left, width, height, false),
        ]
    }

    pub fn resolve_round(&self, width: f32, height: f32) -> f32 {
        self.round
            .map(|spec| {
                resolve_inset_edge(spec, width, height, false)
                    .min(resolve_inset_edge(spec, width, height, true))
            })
            .unwrap_or(0.0)
            .max(0.0)
    }
}

fn resolve_inset_edge(spec: LengthSpec, width: f32, height: f32, vertical: bool) -> f32 {
    let percent_base = if vertical { height } else { width };
    match spec {
        LengthSpec::Percent(p) => percent_base.max(0.0) * p / 100.0,
        LengthSpec::Px(v) => v.max(0.0),
        other => other
            .resolve_px(Some(percent_base.max(0.0)))
            .unwrap_or(0.0)
            .max(0.0),
    }
}

/// One vertex in `clip-path: polygon(...)` (percent or px against the border box).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClipPoint {
    pub x: LengthSpec,
    pub y: LengthSpec,
}

/// Parsed `clip-path` (`inset` or `polygon`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClipPath {
    Inset(ClipInset),
    Polygon(Vec<ClipPoint>),
}

impl ClipPath {
    pub fn resolve_polygon_points(&self, width: f32, height: f32) -> Option<Vec<[f32; 2]>> {
        let ClipPath::Polygon(points) = self else {
            return None;
        };
        if points.len() < 3 {
            return None;
        }
        Some(
            points
                .iter()
                .map(|point| {
                    [
                        resolve_clip_axis(point.x, width),
                        resolve_clip_axis(point.y, height),
                    ]
                })
                .collect(),
        )
    }
}

fn resolve_clip_axis(spec: LengthSpec, axis: f32) -> f32 {
    match spec {
        LengthSpec::Percent(p) => axis.max(0.0) * p / 100.0,
        LengthSpec::Px(v) => v,
        other => other.resolve_px(Some(axis.max(0.0))).unwrap_or(0.0),
    }
}

/// CSS `filter: drop-shadow()` — alpha silhouette, not border-box geometry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FilterDropShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: [f32; 4],
}

/// CSS `filter` brightness / saturate / contrast / hue-rotate / blur / drop-shadow.
///
/// `blur` and `drop-shadow` are the element's own filters (dest-group), distinct
/// from [`BackdropFilter`]. Exotic functions are omitted (fail closed).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorFilter {
    pub brightness: f32,
    pub saturate: f32,
    pub contrast: f32,
    /// `hue-rotate()` in degrees.
    #[serde(default)]
    pub hue_rotate_deg: f32,
    /// Element `blur(Npx)`, clamped at parse time. Not backdrop-filter.
    #[serde(default)]
    pub blur_radius: f32,
    /// Single `drop-shadow()`; extra layers and spread stay fail-closed.
    #[serde(default)]
    pub drop_shadow: Option<FilterDropShadow>,
}

impl Default for ColorFilter {
    fn default() -> Self {
        Self {
            brightness: 1.0,
            saturate: 1.0,
            contrast: 1.0,
            hue_rotate_deg: 0.0,
            blur_radius: 0.0,
            drop_shadow: None,
        }
    }
}

impl ColorFilter {
    /// Product cap for element-filter blur (tighter than backdrop frost).
    pub const MAX_BLUR_RADIUS: f32 = 16.0;

    pub fn is_identity(self) -> bool {
        (self.brightness - 1.0).abs() < 1e-5
            && (self.saturate - 1.0).abs() < 1e-5
            && (self.contrast - 1.0).abs() < 1e-5
            && self.hue_rotate_deg.abs() < 1e-5
            && self.blur_radius <= 0.0
            && self.drop_shadow.is_none()
    }

    /// Dest-group composite clip pad: element blur and/or drop-shadow extent.
    pub fn dest_extent_pad(self) -> f32 {
        let drop = self
            .drop_shadow
            .map(|shadow| shadow.offset_x.abs().max(shadow.offset_y.abs()) + shadow.blur_radius)
            .unwrap_or(0.0);
        self.blur_radius.max(drop)
    }
}

/// Per-node CSS `backdrop-filter` (dest sampling, not window material).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BackdropFilter {
    /// Gaussian blur radius in logical px (`blur(Npx)`), clamped at parse time.
    pub blur_radius: f32,
    /// `saturate()` multiplier (default 1).
    pub saturate: f32,
}

impl Default for BackdropFilter {
    fn default() -> Self {
        Self {
            blur_radius: 0.0,
            saturate: 1.0,
        }
    }
}

impl BackdropFilter {
    /// Product cap for frosted-glass cost control.
    pub const MAX_BLUR_RADIUS: f32 = 64.0;

    pub fn is_active(self) -> bool {
        self.blur_radius > 0.0 || (self.saturate - 1.0).abs() > 1e-5
    }
}

/// Paint-only surface properties (radii, shadow, visibility).
///
/// Layout measurement ignores this bucket except [`VisibilitySpec::Hidden`],
/// which keeps the border box but suppresses paint and hit-testing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PaintStyle {
    /// `None` inherits computed visibility from the parent.
    #[serde(default)]
    pub visibility: Option<VisibilitySpec>,
    /// Per-corner radii (TL, TR, BR, BL). Overrides uniform
    /// [`LayoutStyle::border_radius`] when set.
    #[serde(default)]
    pub border_radii: Option<[LengthSpec; 4]>,
    /// `box-shadow` layers in CSS order (first is top-most). Capped at
    /// [`MAX_BOX_SHADOWS`].
    #[serde(default)]
    pub box_shadows: Vec<BoxShadowSpec>,
    #[serde(default)]
    pub text_shadow: Option<TextShadowSpec>,
    /// Extra stroke outside the border box. Does not affect layout.
    #[serde(default)]
    pub outline: OutlineSpec,
    /// GPU-capable `mix-blend-mode`. Other keywords stay [`MixBlendMode::Normal`].
    #[serde(default)]
    pub mix_blend: MixBlendMode,
    /// `url()` / `linear-gradient` + slice 9-slice. Other sources/repeat/width
    /// stay fail-closed and sticky; source/slice longhands must not clear it.
    #[serde(default)]
    pub unsupported_border_image: bool,
    /// Painted 9-slice when [`Self::unsupported_border_image`] is false.
    #[serde(default)]
    pub border_image: Option<BorderImageSpec>,
    #[serde(default)]
    pub background_image: Option<BackgroundImage>,
    /// Additional `background-image` layers after the first (CSS comma list).
    #[serde(default)]
    pub background_layers: Vec<BackgroundImage>,
    /// Last parsed `background-size` list, zipped onto a later `background-image`.
    /// Cleared by the `background` shorthand so leftover longhands do not leak.
    #[serde(default)]
    pub background_size_list: Vec<BackgroundImageFit>,
    /// Explicit width/height for [`BackgroundImageFit::Length`] entries, parallel
    /// to [`Self::background_size_list`].
    #[serde(default)]
    pub background_size_lengths: Vec<(Option<LengthSpec>, Option<LengthSpec>)>,
    /// Last parsed `background-position` list.
    #[serde(default)]
    pub background_position_list: Vec<BackgroundPosition>,
    /// Last parsed `background-repeat` list.
    #[serde(default)]
    pub background_repeat_list: Vec<BackgroundRepeat>,
    /// `<img src>` replaced content, painted above background layers.
    #[serde(default)]
    pub content_image: Option<BackgroundImage>,
    /// Replaced media that L1 will not pretend to execute (`iframe`, `video`
    /// without a poster, `<canvas>` without a HostTexture slot). Empty means
    /// the node is not an explicit skip.
    #[serde(default)]
    pub skipped_replaced: Option<String>,
    /// `object-fit` for [`Self::content_image`] (`None` = CSS `fill` / stretch).
    #[serde(default)]
    pub object_fit: Option<BackgroundImageFit>,
    /// `object-position` for [`Self::content_image`] (`None` = `50% 50%`).
    #[serde(default)]
    pub object_position: Option<BackgroundPosition>,
    /// `mask-image` / `-webkit-mask-image` as linear or radial gradient alpha.
    #[serde(default)]
    pub mask: Option<CssGradient>,
    #[serde(default)]
    pub clip_path: Option<ClipPath>,
    #[serde(default)]
    pub filter: Option<ColorFilter>,
    #[serde(default)]
    pub backdrop_filter: Option<BackdropFilter>,
}

impl PaintStyle {
    pub fn is_visible(&self) -> bool {
        self.visibility != Some(VisibilitySpec::Hidden)
    }

    pub fn has_advanced_paint(&self) -> bool {
        self.background_image.is_some()
            || !self.background_layers.is_empty()
            || self.content_image.is_some()
            || self.mask.is_some()
            || self.clip_path.is_some()
            || self.filter.is_some_and(|filter| !filter.is_identity())
            || self.backdrop_filter.is_some_and(BackdropFilter::is_active)
            || !self.box_shadows.is_empty()
            || self.outline.is_active()
            || !self.mix_blend.is_normal()
            || self.border_image.is_some()
    }

    pub fn primary_box_shadow(&self) -> Option<BoxShadowSpec> {
        self.box_shadows.first().copied()
    }

    /// Background layers in CSS paint order (bottom → top), then `<img src>`.
    pub fn paint_image_layers(&self) -> impl Iterator<Item = &BackgroundImage> {
        self.background_layers
            .iter()
            .rev()
            .chain(self.background_image.as_ref())
            .chain(self.content_image.as_ref())
    }
}

/// CSS 2D affine paint transform applied without changing layout.
///
/// The six fields use the Canvas/CSS `matrix(a, b, c, d, e, f)` convention.
/// NanaUI applies the matrix around [`LayoutStyle::transform_origin`] (default
/// CSS `50% 50%` = box center) via [`Self::around_center`] /
/// [`Self::around_origin`]. Translation is expressed in logical pixels.
///
/// 2D lists stay on this 2×3. Planar CSS 3D (`perspective()` + `rotateY`, true
/// `matrix3d`) is stored separately as [`PaintMat4`] and painted as the z=0
/// homography in the existing Scene quad pass. Parent `perspective` /
/// `preserve-3d` stay fail-closed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PaintTransform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

const PAINT_MAT4_EPS: f32 = 1e-5;

/// CSS `matrix3d` 4×4 in argument/column-major order (16 floats).
///
/// Scene paints the z=0 plane as a 3×3 homography in the existing quad pass
/// (same 4 vertices, perspective divide). Not a second engine.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PaintMat4 {
    pub m: [f32; 16],
}

impl Default for PaintTransform {
    fn default() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }
}

impl PaintTransform {
    pub fn is_identity(self) -> bool {
        self == Self::default()
    }

    /// Concatenates a transform function on the right, preserving CSS list order.
    pub fn then(self, rhs: Self) -> Self {
        Self {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            e: self.a * rhs.e + self.c * rhs.f + self.e,
            f: self.b * rhs.e + self.d * rhs.f + self.f,
        }
    }

    /// World-space 2×3 using the CSS default origin (`50% 50%` = box center).
    pub fn around_center(self, x: f32, y: f32, width: f32, height: f32) -> [f32; 6] {
        self.around_origin(x, y, width * 0.5, height * 0.5)
    }

    /// World-space 2×3 pivoted at box-local `(origin_x, origin_y)` pixels.
    ///
    /// Same six `Copy` coefficients as [`Self::around_center`]; only the pivot
    /// changes. `origin_x` / `origin_y` are offsets from the box origin `(x, y)`.
    pub fn around_origin(self, x: f32, y: f32, origin_x: f32, origin_y: f32) -> [f32; 6] {
        let ox = x + origin_x;
        let oy = y + origin_y;
        [
            self.a,
            self.b,
            self.c,
            self.d,
            ox + self.e - self.a * ox - self.c * oy,
            oy + self.f - self.b * ox - self.d * oy,
        ]
    }
}

impl Default for PaintMat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl PaintMat4 {
    pub const IDENTITY: Self = Self {
        m: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
    };

    pub fn from_affine(t: PaintTransform) -> Self {
        Self {
            m: [
                t.a, t.b, 0.0, 0.0, t.c, t.d, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, t.e, t.f, 0.0, 1.0,
            ],
        }
    }

    pub fn from_matrix3d(m: [f32; 16]) -> Option<Self> {
        m.iter().all(|v| v.is_finite()).then_some(Self { m })
    }

    pub fn translation(x: f32, y: f32, z: f32) -> Self {
        let mut m = Self::IDENTITY;
        m.m[12] = x;
        m.m[13] = y;
        m.m[14] = z;
        m
    }

    pub fn scaling(x: f32, y: f32, z: f32) -> Self {
        let mut m = Self::IDENTITY;
        m.m[0] = x;
        m.m[5] = y;
        m.m[10] = z;
        m
    }

    pub fn rotate_x(angle: f32) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self {
            m: [
                1.0, 0.0, 0.0, 0.0, 0.0, cos, sin, 0.0, 0.0, -sin, cos, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn rotate_y(angle: f32) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self {
            m: [
                cos, 0.0, -sin, 0.0, 0.0, 1.0, 0.0, 0.0, sin, 0.0, cos, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn rotate_z(angle: f32) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self {
            m: [
                cos, sin, 0.0, 0.0, -sin, cos, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    /// CSS `rotate3d(x, y, z, angle)` around a (possibly unnormalized) axis.
    pub fn rotate3d(x: f32, y: f32, z: f32, angle: f32) -> Option<Self> {
        let len = (x * x + y * y + z * z).sqrt();
        if !len.is_finite() || len < PAINT_MAT4_EPS {
            return None;
        }
        let x = x / len;
        let y = y / len;
        let z = z / len;
        let (sin, cos) = angle.sin_cos();
        let c = 1.0 - cos;
        Some(Self {
            m: [
                cos + x * x * c,
                y * x * c + z * sin,
                z * x * c - y * sin,
                0.0,
                x * y * c - z * sin,
                cos + y * y * c,
                z * y * c + x * sin,
                0.0,
                x * z * c + y * sin,
                y * z * c - x * sin,
                cos + z * z * c,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
            ],
        })
    }

    /// CSS `perspective(d)`: `matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,-1/d, 0,0,0,1)`.
    pub fn perspective(d: f32) -> Option<Self> {
        if !d.is_finite() || d.abs() < PAINT_MAT4_EPS {
            return None;
        }
        let mut m = Self::IDENTITY;
        m.m[11] = -1.0 / d;
        Some(m)
    }

    pub fn is_identity(self) -> bool {
        self.m
            .iter()
            .zip(Self::IDENTITY.m.iter())
            .all(|(a, b)| (*a - *b).abs() <= PAINT_MAT4_EPS)
    }

    /// Concatenates `rhs` on the right (CSS list order: `self` then `rhs`).
    pub fn then(self, rhs: Self) -> Self {
        let a = self.m;
        let b = rhs.m;
        let mut m = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                m[col * 4 + row] = a[row] * b[col * 4]
                    + a[4 + row] * b[col * 4 + 1]
                    + a[8 + row] * b[col * 4 + 2]
                    + a[12 + row] * b[col * 4 + 3];
            }
        }
        Self { m }
    }

    /// `T(pivot) * self * T(-pivot)` with a 2D origin (`z = 0`).
    pub fn around_origin(self, x: f32, y: f32, origin_x: f32, origin_y: f32) -> Self {
        let ox = x + origin_x;
        let oy = y + origin_y;
        Self::translation(ox, oy, 0.0)
            .then(self)
            .then(Self::translation(-ox, -oy, 0.0))
    }

    /// Map `(x, y, 0, 1)` through this matrix and perspective-divide.
    pub fn project_xy(self, x: f32, y: f32) -> Option<[f32; 2]> {
        let m = self.m;
        let xp = m[0] * x + m[4] * y + m[12];
        let yp = m[1] * x + m[5] * y + m[13];
        let wp = m[3] * x + m[7] * y + m[15];
        if !wp.is_finite() || wp.abs() < 1e-8 {
            return None;
        }
        let sx = xp / wp;
        let sy = yp / wp;
        (sx.is_finite() && sy.is_finite()).then_some([sx, sy])
    }

    /// z=0 homography `(a,b,c,d,e,f)` plus `(g,h)` with `w = g x + h y + 1`.
    pub fn planar_homography(self) -> Option<([f32; 6], [f32; 2])> {
        let m = self.m;
        let mut a = m[0];
        let mut b = m[1];
        let mut c = m[4];
        let mut d = m[5];
        let mut e = m[12];
        let mut f = m[13];
        let mut g = m[3];
        let mut h = m[7];
        let i = m[15];
        if !i.is_finite() || i.abs() < 1e-8 {
            return None;
        }
        let inv = 1.0 / i;
        a *= inv;
        b *= inv;
        c *= inv;
        d *= inv;
        e *= inv;
        f *= inv;
        g *= inv;
        h *= inv;
        [a, b, c, d, e, f, g, h]
            .into_iter()
            .all(f32::is_finite)
            .then_some(([a, b, c, d, e, f], [g, h]))
    }

    pub fn is_planar_affine(self) -> bool {
        let Some((_, [g, h])) = self.planar_homography() else {
            return false;
        };
        g.abs() <= PAINT_MAT4_EPS && h.abs() <= PAINT_MAT4_EPS
    }

    pub fn as_affine(self) -> Option<PaintTransform> {
        if !self.is_planar_affine() {
            return None;
        }
        let ([a, b, c, d, e, f], _) = self.planar_homography()?;
        Some(PaintTransform { a, b, c, d, e, f })
    }

    /// Projected corners of a border box (TL, TR, BR, BL).
    pub fn projected_corners(
        self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<[[f32; 2]; 4]> {
        Some([
            self.project_xy(x, y)?,
            self.project_xy(x + width, y)?,
            self.project_xy(x + width, y + height)?,
            self.project_xy(x, y + height)?,
        ])
    }
}

/// CSS `transform-box` (2D origin reference).
///
/// CSS initial is `view-box`. For CSS layout boxes (HTML), `view-box` uses the
/// border box and `fill-box` uses the content box — there is no SVG viewport
/// origin space. Percent [`TransformOrigin`] is relative to this box; the
/// resulting pivot is still applied via [`PaintTransform::around_origin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TransformBox {
    /// CSS initial. Used as the border box for CSS layout boxes.
    #[default]
    ViewBox,
    BorderBox,
    FillBox,
    ContentBox,
}

impl TransformBox {
    /// `content-box` / `fill-box` for HTML; `view-box` / `border-box` stay on
    /// the layout (border) box.
    pub fn uses_content_box(self) -> bool {
        matches!(self, Self::ContentBox | Self::FillBox)
    }
}

/// CSS `transform-origin` (2D). Percent is relative to [`LayoutStyle::transform_box`].
/// A third z length is accepted by the parser and dropped on this 2×3 path.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TransformOrigin {
    pub x: LengthSpec,
    pub y: LengthSpec,
}

impl Default for TransformOrigin {
    fn default() -> Self {
        Self {
            x: LengthSpec::Percent(50.0),
            y: LengthSpec::Percent(50.0),
        }
    }
}

impl TransformOrigin {
    /// Resolve to box-local pixels (`50%` of a 40×20 box is `(20, 10)`).
    pub fn resolve(self, width: f32, height: f32) -> [f32; 2] {
        [
            self.x.resolve_px(Some(width)).unwrap_or(width * 0.5),
            self.y.resolve_px(Some(height)).unwrap_or(height * 0.5),
        ]
    }
}

impl ParentBox {
    pub fn new(width: Option<f32>, height: Option<f32>) -> Self {
        Self { width, height }
    }

    pub fn from_viewport(width: f32, height: f32) -> Self {
        Self {
            width: Some(width.max(0.0)),
            height: Some(height.max(0.0)),
        }
    }
}

/// Specified `*-inline-start/end` plus the physical left/right they compete with.
///
/// Cascade order is preserved with generation stamps so a later longhand
/// (logical or physical) wins the used edge after the final `direction`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct LogicalInlineEdges {
    pub start: Option<LengthSpec>,
    pub end: Option<LengthSpec>,
    pub phys_left: Option<LengthSpec>,
    pub phys_right: Option<LengthSpec>,
    #[serde(default)]
    pub start_gen: u32,
    #[serde(default)]
    pub end_gen: u32,
    #[serde(default)]
    pub phys_left_gen: u32,
    #[serde(default)]
    pub phys_right_gen: u32,
    #[serde(default)]
    pub next_gen: u32,
}

impl LogicalInlineEdges {
    pub fn has_logical(&self) -> bool {
        self.start.is_some() || self.end.is_some()
    }

    fn bump(&mut self) -> u32 {
        self.next_gen = self.next_gen.saturating_add(1);
        self.next_gen
    }

    pub fn set_start(&mut self, spec: Option<LengthSpec>) {
        self.start = spec;
        self.start_gen = self.bump();
    }

    pub fn set_end(&mut self, spec: Option<LengthSpec>) {
        self.end = spec;
        self.end_gen = self.bump();
    }

    pub fn set_phys_left(&mut self, spec: Option<LengthSpec>) {
        self.phys_left = spec;
        self.phys_left_gen = self.bump();
    }

    pub fn set_phys_right(&mut self, spec: Option<LengthSpec>) {
        self.phys_right = spec;
        self.phys_right_gen = self.bump();
    }

    fn pick(
        logical: Option<LengthSpec>,
        logical_gen: u32,
        phys: Option<LengthSpec>,
        phys_gen: u32,
    ) -> Option<LengthSpec> {
        match (logical, phys) {
            (None, None) => None,
            (Some(v), None) => Some(v),
            (None, Some(v)) => Some(v),
            (Some(lv), Some(pv)) => {
                if phys_gen >= logical_gen {
                    Some(pv)
                } else {
                    Some(lv)
                }
            }
        }
    }

    pub fn used_left(&self, rtl: bool) -> Option<LengthSpec> {
        let (logical, logical_gen) = if rtl {
            (self.end, self.end_gen)
        } else {
            (self.start, self.start_gen)
        };
        Self::pick(logical, logical_gen, self.phys_left, self.phys_left_gen)
    }

    pub fn used_right(&self, rtl: bool) -> Option<LengthSpec> {
        let (logical, logical_gen) = if rtl {
            (self.start, self.start_gen)
        } else {
            (self.end, self.end_gen)
        };
        Self::pick(logical, logical_gen, self.phys_right, self.phys_right_gen)
    }
}

/// 可测布局意图（Style Model Layout 盒切片）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutStyle {
    pub direction: Option<FlexDirection>,
    /// CSS `direction` (`ltr` / `rtl`). `None` = inherit / initial `ltr`.
    /// Not [`Self::direction`] (`flex-direction`). Remaps logical box edges
    /// and `text-align: start | end` only — not flex/grid start or item order.
    #[serde(default)]
    pub dir: Option<DirSpec>,
    /// Vertical / sideways `writing-mode` is fail-closed (no axis remap).
    #[serde(default)]
    pub unsupported_writing_mode: bool,
    /// `row-reverse` / `column-reverse`：主轴起点对调（measure 反转子项序）。
    #[serde(default)]
    pub flex_reverse: bool,
    /// CSS `order`（flex/grid 项）。默认 `0`；可为负。布局按升序，同值保留源序；
    /// 再应用 [`Self::flex_reverse`]。
    #[serde(default)]
    pub order: i32,
    pub flex_wrap: FlexWrap,
    pub display: Option<DisplaySpec>,
    pub box_sizing: BoxSizing,
    pub position: PositionSpec,
    /// CSS `z-index`（整数）。`None` = auto；fixed 层默认按树序叠在内容之上。
    #[serde(default)]
    pub z_index: Option<i32>,
    /// CSS `isolation: isolate`. Paint-order stacking context; not a layout box.
    #[serde(default)]
    pub isolation: bool,
    /// Paint-only 2D `transform` subset. It never participates in flex/grid
    /// measurement. Planar CSS 3D is [`Self::transform_3d`]; parent
    /// `perspective` / `preserve-3d` are [`Self::css_perspective`] /
    /// [`Self::preserve_3d`] and fail-close child 3D at paint.
    #[serde(default)]
    pub transform: Option<PaintTransform>,
    /// Planar 4×4 (`perspective()` + `rotateY`, true `matrix3d`). Additive to
    /// the 2×3: 2D lists stay on [`Self::transform`].
    #[serde(default)]
    pub transform_3d: Option<PaintMat4>,
    #[serde(default)]
    pub unsupported_transform: Option<String>,
    /// CSS `transform-origin`. `None` = `50% 50%` (box center).
    /// Applied to both the 2×3 and the 4×4 planar path.
    #[serde(default)]
    pub transform_origin: Option<TransformOrigin>,
    /// CSS `perspective` property (not the transform function). Stored so we
    /// fail closed instead of pretending a parent 3D rendering context exists.
    #[serde(default)]
    pub css_perspective: Option<f32>,
    /// CSS `transform-style: preserve-3d`. Flattening is not implemented.
    #[serde(default)]
    pub preserve_3d: bool,
    /// CSS `transform-box`. Default `view-box` (used as border-box for HTML).
    #[serde(default)]
    pub transform_box: TransformBox,
    /// Uniform `gap` shorthand residue / class defaults. Axis overrides win.
    /// `%` / calc 保留为 [`LengthSpec`]（同 margin/padding），布局时相对 CB 解析；
    /// 解析期无 CB 时不得收成 px 或静默丢弃。
    pub gap: Option<LengthSpec>,
    /// `row-gap`（或 `gap: row column` 的第一值）。
    #[serde(default)]
    pub row_gap: Option<LengthSpec>,
    /// `column-gap`（或 `gap: row column` 的第二值）。
    #[serde(default)]
    pub column_gap: Option<LengthSpec>,
    /// Uniform padding shorthand residue. `%` / calc 保留为 [`LengthSpec`]，
    /// 布局时相对包含块**宽度**解析（含上下边）。
    pub padding: Option<LengthSpec>,
    pub padding_top: Option<LengthSpec>,
    pub padding_right: Option<LengthSpec>,
    pub padding_bottom: Option<LengthSpec>,
    pub padding_left: Option<LengthSpec>,
    /// Specified `padding-inline-*` plus physical left/right they compete with.
    #[serde(default)]
    pub logical_padding: LogicalInlineEdges,
    /// Uniform margin shorthand residue（`%` 合同同 padding）。
    pub margin: Option<LengthSpec>,
    pub margin_top: Option<LengthSpec>,
    pub margin_right: Option<LengthSpec>,
    pub margin_bottom: Option<LengthSpec>,
    pub margin_left: Option<LengthSpec>,
    #[serde(default)]
    pub logical_margin: LogicalInlineEdges,
    /// Inset：`relative` / `absolute` / `fixed` 用（`Px` 或 `%`；measure 时相对 CB 解析）。
    /// `Static` 忽略；`sticky` defer。
    #[serde(default)]
    pub offset_top: Option<LengthSpec>,
    #[serde(default)]
    pub offset_right: Option<LengthSpec>,
    #[serde(default)]
    pub offset_bottom: Option<LengthSpec>,
    #[serde(default)]
    pub offset_left: Option<LengthSpec>,
    #[serde(default)]
    pub logical_inset: LogicalInlineEdges,
    pub width: Option<LengthSpec>,
    pub height: Option<LengthSpec>,
    /// `min-width`：保留 [`LengthSpec`]（px / `%` / calc / em / viewport），布局时解析。
    pub min_width: Option<LengthSpec>,
    /// `max-width`：同上。`Fill` 表示无有限上限。
    pub max_width: Option<LengthSpec>,
    /// `min-height`：同上。
    pub min_height: Option<LengthSpec>,
    /// `max-height`：同上。
    pub max_height: Option<LengthSpec>,
    /// `min-width: 0` → 允许 flex 子项收缩。
    pub allow_shrink: bool,
    pub align_items: AlignSpec,
    /// `align-self`；`None` = `auto`（继承容器 `align-items`）。
    #[serde(default)]
    pub align_self: Option<AlignSpec>,
    /// `align-content`（多行 flex 线间分布；`stretch`/`normal` 均分剩余交叉空间）。
    #[serde(default)]
    pub align_content: JustifySpec,
    pub justify_content: JustifySpec,
    /// `justify-items`（Box Alignment；flex 项忽略，供 grid/place-items 第二值）。
    #[serde(default)]
    pub justify_items: Option<AlignSpec>,
    /// `justify-self`（Box Alignment；flex 项忽略，供 place-self 第二值）。
    #[serde(default)]
    pub justify_self: Option<AlignSpec>,
    /// `flex-grow`；>0 时主轴随父方向 Fill。
    pub flex_grow: Option<f32>,
    /// `flex-shrink`。未写长手时 `None` 按 **0**（不是 CSS initial 1），避免溢出的
    /// 定宽行被压扁。`flex` 简写省略 shrink 时仍按 CSS 写成 `Some(1.0)`
    ///（`flex: initial` / `flex: N` / `flex: N <basis>`）。
    pub flex_shrink: Option<f32>,
    pub flex_basis: Option<LengthSpec>,
    pub overflow_x: OverflowSpec,
    pub overflow_y: OverflowSpec,
    /// `text-overflow: ellipsis`（需配合 nowrap / 定宽；Scene text 路径兑现）。
    #[serde(default)]
    pub text_overflow_ellipsis: bool,
    /// `line-clamp` / `-webkit-line-clamp` (wrap + ellipsis, max N lines).
    #[serde(default)]
    pub line_clamp: Option<u16>,
    /// Specified CSS `pointer-events`. `None` means inherit (not `auto`).
    #[serde(default)]
    pub pointer_events: Option<PointerEventsSpec>,
    /// `white-space: nowrap`（与 [`Self::white_space`] 同步）。
    #[serde(default)]
    pub white_space_nowrap: bool,
    /// `white-space` 子集。
    #[serde(default)]
    pub white_space: WhiteSpaceSpec,
    /// `text-align`（IFC 行对齐）。
    #[serde(default)]
    pub text_align: TextAlignSpec,
    /// `float` 子集。
    #[serde(default)]
    pub float: FloatSpec,
    /// `clear` 子集。
    #[serde(default)]
    pub clear: ClearSpec,
    /// Computed `font-size` in CSS px. `None` = inherit (then initial / ControlSize).
    #[serde(default)]
    pub font_size: Option<f32>,
    /// CSS `font-weight` as 100..=900. `None` = inherit / normal.
    #[serde(default)]
    pub font_weight: Option<u16>,
    /// Preferred named family from `font-family` (generics stripped). `None` = UI default.
    #[serde(default)]
    pub font_family: Option<String>,
    /// CSS `line-height` subset. `None` = inherit / engine default.
    #[serde(default)]
    pub line_height: Option<LineHeightSpec>,
    /// CSS `letter-spacing` in px. `None` = inherit / normal (0).
    #[serde(default)]
    pub letter_spacing: Option<f32>,
    /// Foreground `color` (RGBA 0..=1). `None` = inherit / theme text.
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    /// CSS `text-decoration` / `text-decoration-line`. `None` = inherit / none.
    #[serde(default)]
    pub text_decoration: Option<TextDecorationLine>,
    /// CSS `font-feature-settings`. `None` = inherit; `Some([])` = `normal`.
    #[serde(default)]
    pub font_features: Option<Vec<FontFeatureSetting>>,
    /// `font-variation-settings` axes other than `wght`. cosmic-text Attrs
    /// only expose `wght` via [`Self::font_weight`]; `BEVL` and the rest fail
    /// closed.
    #[serde(default)]
    pub unsupported_font_variation: bool,
    /// `::placeholder` color on a text input (RGBA 0..=1). Not a generated box.
    #[serde(default)]
    pub placeholder_color: Option<[f32; 4]>,
    /// `::placeholder` opacity multiplier. Combined with
    /// [`Self::placeholder_color`] (or theme faint) at TextInput paint.
    #[serde(default)]
    pub placeholder_opacity: Option<f32>,
    /// `grid-template-columns` 轻量轨道（侧栏|主区）。
    pub grid_columns: Option<Vec<GridTrack>>,
    /// `grid-template-rows` 轻量轨道（堆叠区；Column 主轴）。
    #[serde(default)]
    pub grid_rows: Option<Vec<GridTrack>>,
    /// `grid-template-columns` 无法兑现的语法（`subgrid`、嵌套 auto-fit / auto-fill）。
    /// 成功的 `repeat(auto-fit|auto-fill)` **不**走这里，见 [`Self::grid_columns_repeat`]。
    #[serde(default)]
    pub grid_columns_unsupported: Option<GridTrackListUnsupported>,
    /// 同 [`Self::grid_columns_unsupported`]，针对 `grid-template-rows`。
    #[serde(default)]
    pub grid_rows_unsupported: Option<GridTrackListUnsupported>,
    /// `grid-auto-columns`：隐式列轨（2D 自动放置超出模板时追加）。
    #[serde(default)]
    pub grid_auto_columns: Option<Vec<GridTrack>>,
    /// `grid-auto-rows`：隐式行轨（2D 自动放置超出模板时追加）。
    #[serde(default)]
    pub grid_auto_rows: Option<Vec<GridTrack>>,
    /// `grid-auto-flow`：2D 自动放置方向（缺省 row）。
    #[serde(default)]
    pub grid_auto_flow: Option<GridAutoFlow>,
    /// `repeat(auto-fit|auto-fill, …)` 列模式；布局按容器展开。
    #[serde(default)]
    pub grid_columns_repeat: Option<GridRepeatAuto>,
    /// 同行模式，针对 `grid-template-rows`。
    #[serde(default)]
    pub grid_rows_repeat: Option<GridRepeatAuto>,
    /// `grid-column` / `grid-row` 项放置。
    #[serde(default)]
    pub grid_placement: GridPlacement,
    /// `grid-template-areas` 命名区域。
    #[serde(default)]
    pub grid_template_areas: Option<GridTemplateAreas>,
    /// 列线名称（线 i 的名字列表；长度 = 轨数 + 1）。
    #[serde(default)]
    pub grid_column_line_names: Option<Vec<Vec<String>>>,
    /// 行线名称。
    #[serde(default)]
    pub grid_row_line_names: Option<Vec<Vec<String>>>,
    pub hidden: bool,
    /// Paint-only surface: corner radii, box-shadow, CSS visibility.
    #[serde(default)]
    pub paint: PaintStyle,
    /// CSS `opacity` (0..=1). `None` = unset / inherit (treated as 1.0 at paint).
    /// Parsed with other declarations so L1 adapters need not re-scan the style
    /// string.
    #[serde(default)]
    pub opacity: Option<f32>,
    /// Instance surface paint from L1 style/class (not ThemeTokens).
    /// RGBA 0..=1; applied by Scene paint.
    #[serde(default)]
    pub background: Option<[f32; 4]>,
    #[serde(default)]
    pub border_radius: Option<f32>,
    #[serde(default)]
    pub border_width: Option<f32>,
    #[serde(default)]
    pub border_top_width: Option<f32>,
    #[serde(default)]
    pub border_right_width: Option<f32>,
    #[serde(default)]
    pub border_bottom_width: Option<f32>,
    #[serde(default)]
    pub border_left_width: Option<f32>,
    #[serde(default)]
    pub border_color: Option<[f32; 4]>,
    #[serde(default)]
    pub border_top_color: Option<[f32; 4]>,
    #[serde(default)]
    pub border_right_color: Option<[f32; 4]>,
    #[serde(default)]
    pub border_bottom_color: Option<[f32; 4]>,
    #[serde(default)]
    pub border_left_color: Option<[f32; 4]>,
    /// Uniform `border-style` shorthand residue. Longhands override.
    #[serde(default)]
    pub border_style: Option<BorderStyle>,
    #[serde(default)]
    pub border_top_style: Option<BorderStyle>,
    #[serde(default)]
    pub border_right_style: Option<BorderStyle>,
    #[serde(default)]
    pub border_bottom_style: Option<BorderStyle>,
    #[serde(default)]
    pub border_left_style: Option<BorderStyle>,
}

impl Default for LayoutStyle {
    fn default() -> Self {
        Self {
            direction: None,
            dir: None,
            unsupported_writing_mode: false,
            flex_reverse: false,
            order: 0,
            flex_wrap: FlexWrap::NoWrap,
            display: None,
            box_sizing: BoxSizing::BorderBox,
            position: PositionSpec::Static,
            z_index: None,
            isolation: false,
            transform: None,
            transform_3d: None,
            unsupported_transform: None,
            transform_origin: None,
            transform_box: TransformBox::ViewBox,
            css_perspective: None,
            preserve_3d: false,
            gap: None,
            row_gap: None,
            column_gap: None,
            padding: None,
            padding_top: None,
            padding_right: None,
            padding_bottom: None,
            padding_left: None,
            logical_padding: LogicalInlineEdges::default(),
            margin: None,
            margin_top: None,
            margin_right: None,
            margin_bottom: None,
            margin_left: None,
            logical_margin: LogicalInlineEdges::default(),
            offset_top: None,
            offset_right: None,
            offset_bottom: None,
            offset_left: None,
            logical_inset: LogicalInlineEdges::default(),
            width: None,
            height: None,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            allow_shrink: false,
            align_items: AlignSpec::Start,
            align_self: None,
            align_content: JustifySpec::Start,
            justify_content: JustifySpec::Start,
            justify_items: None,
            justify_self: None,
            flex_grow: None,
            flex_shrink: None,
            flex_basis: None,
            overflow_x: OverflowSpec::Visible,
            overflow_y: OverflowSpec::Visible,
            text_overflow_ellipsis: false,
            line_clamp: None,
            pointer_events: None,
            white_space_nowrap: false,
            white_space: WhiteSpaceSpec::Normal,
            text_align: TextAlignSpec::Start,
            float: FloatSpec::None,
            clear: ClearSpec::None,
            font_size: None,
            font_weight: None,
            font_family: None,
            line_height: None,
            letter_spacing: None,
            color: None,
            text_decoration: None,
            font_features: None,
            unsupported_font_variation: false,
            placeholder_color: None,
            placeholder_opacity: None,
            grid_columns: None,
            grid_rows: None,
            grid_columns_unsupported: None,
            grid_rows_unsupported: None,
            grid_auto_columns: None,
            grid_auto_rows: None,
            grid_auto_flow: None,
            grid_columns_repeat: None,
            grid_rows_repeat: None,
            grid_placement: GridPlacement::default(),
            grid_template_areas: None,
            grid_column_line_names: None,
            grid_row_line_names: None,
            hidden: false,
            paint: PaintStyle::default(),
            opacity: None,
            background: None,
            border_radius: None,
            border_width: None,
            border_top_width: None,
            border_right_width: None,
            border_bottom_width: None,
            border_left_width: None,
            border_color: None,
            border_top_color: None,
            border_right_color: None,
            border_bottom_color: None,
            border_left_color: None,
            border_style: None,
            border_top_style: None,
            border_right_style: None,
            border_bottom_style: None,
            border_left_style: None,
        }
    }
}

impl LayoutStyle {
    /// Box-local pivot in px, relative to the layout (border) box origin.
    /// Missing origin is CSS `50% 50%`. `content-box` / `fill-box` subtract
    /// border + padding (`resolved_padding`: `%` without a CB is 0).
    pub fn resolved_transform_origin(&self, width: f32, height: f32) -> [f32; 2] {
        let origin = self.transform_origin.unwrap_or_default();
        if !self.transform_box.uses_content_box() {
            return origin.resolve(width, height);
        }
        let border = self.resolved_border_edges();
        let pad = self.resolved_padding();
        let inset_x = border.left + pad.left;
        let inset_y = border.top + pad.top;
        let content_w = (width - inset_x - border.right - pad.right).max(0.0);
        let content_h = (height - inset_y - border.bottom - pad.bottom).max(0.0);
        let [ox, oy] = origin.resolve(content_w, content_h);
        [inset_x + ox, inset_y + oy]
    }

    /// World-space 2×3 for this node's paint transform, pivoted at origin.
    /// Planar 3D uses [`Self::world_scene_transform`] instead.
    pub fn world_paint_transform(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<[f32; 6]> {
        let (matrix, persp) = self.world_scene_transform(x, y, width, height)?;
        (persp[0].abs() <= PAINT_MAT4_EPS && persp[1].abs() <= PAINT_MAT4_EPS).then_some(matrix)
    }

    /// World-space homography of the z=0 plane, pivoted at transform-origin.
    ///
    /// `persp` is `(g, h)` in `w = g x + h y + 1`. Zeroes keep a 2×3 affine.
    pub fn world_scene_transform(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<([f32; 6], [f32; 2])> {
        let [ox, oy] = self.resolved_transform_origin(width, height);
        if let Some(mat4) = self.transform_3d {
            return mat4.around_origin(x, y, ox, oy).planar_homography();
        }
        let transform = self.transform?;
        Some((transform.around_origin(x, y, ox, oy), [0.0, 0.0]))
    }

    /// True when this node asked for a parent 3D rendering context we do not paint.
    pub fn fails_closed_3d_context(&self) -> bool {
        self.css_perspective.is_some() || self.preserve_3d
    }

    pub fn has_surface_paint(&self) -> bool {
        self.background.is_some()
            || self
                .resolved_border_radii(0.0, 0.0)
                .iter()
                .any(|r| *r > 0.0)
            || self.resolved_border_width() > 0.0
            || !self.paint.box_shadows.is_empty()
            || self.paint.has_advanced_paint()
    }

    /// CSS `visibility: hidden` keeps layout but suppresses paint / hit-test.
    pub fn is_paint_visible(&self) -> bool {
        self.paint.is_visible()
    }

    /// Resolve corner radii (TL, TR, BR, BL) to physical px for a border box.
    pub fn resolved_border_radii(&self, width: f32, height: f32) -> [f32; 4] {
        if let Some(specs) = self.paint.border_radii {
            [
                resolve_corner_radius(specs[0], width, height),
                resolve_corner_radius(specs[1], width, height),
                resolve_corner_radius(specs[2], width, height),
                resolve_corner_radius(specs[3], width, height),
            ]
        } else {
            let uniform = self.border_radius.unwrap_or(0.0).max(0.0);
            [uniform; 4]
        }
    }

    /// `align-self: auto` → 容器 `align-items`；否则用自身值。
    pub fn resolved_align_self(&self, parent_align_items: AlignSpec) -> AlignSpec {
        self.align_self.unwrap_or(parent_align_items)
    }

    pub fn gap_or(&self, default: f32) -> f32 {
        self.gap
            .or(self.row_gap)
            .or(self.column_gap)
            .and_then(|s| s.resolve_px(None))
            .unwrap_or(default)
            .max(0.0)
    }

    /// CSS `row-gap`（回退到 uniform `gap`）。无 `%` 基时仅兑现 `Px`。
    pub fn resolved_row_gap(&self) -> f32 {
        self.resolved_row_gap_against(None)
    }

    /// `row-gap` / uniform `gap`，`%` 相对 `percent_base`（通常为 CB 高度，缺省回退宽度）。
    pub fn resolved_row_gap_against(&self, percent_base: Option<f32>) -> f32 {
        self.resolved_row_gap_against_fonts(percent_base, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_row_gap_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_row_gap_against_fonts(
        &self,
        percent_base: Option<f32>,
        fonts: FontSizeContext,
    ) -> f32 {
        self.row_gap
            .or(self.gap)
            .and_then(|s| s.resolve_with_fonts(percent_base, None, fonts))
            .unwrap_or(0.0)
            .max(0.0)
    }

    /// CSS `column-gap`（回退到 uniform `gap`）。无 `%` 基时仅兑现 `Px`。
    pub fn resolved_column_gap(&self) -> f32 {
        self.resolved_column_gap_against(None)
    }

    /// `column-gap` / uniform `gap`，`%` 相对包含块宽度。
    pub fn resolved_column_gap_against(&self, percent_base: Option<f32>) -> f32 {
        self.resolved_column_gap_against_fonts(percent_base, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_column_gap_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_column_gap_against_fonts(
        &self,
        percent_base: Option<f32>,
        fonts: FontSizeContext,
    ) -> f32 {
        self.column_gap
            .or(self.gap)
            .and_then(|s| s.resolve_with_fonts(percent_base, None, fonts))
            .unwrap_or(0.0)
            .max(0.0)
    }

    /// Flex 主轴 gap：Row→column-gap；Column→row-gap。无 `%` 基时仅兑现 `Px`。
    pub fn main_gap(&self, direction: FlexDirection) -> f32 {
        self.main_gap_against(direction, ParentBox::default())
    }

    /// 主轴 gap，携带 CB 供 `%` 解析（column-gap→宽；row-gap→高，缺省回退宽）。
    pub fn main_gap_against(&self, direction: FlexDirection, cb: ParentBox) -> f32 {
        self.main_gap_against_fonts(direction, cb, FontSizeContext::default())
    }

    /// 同 [`Self::main_gap_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn main_gap_against_fonts(
        &self,
        direction: FlexDirection,
        cb: ParentBox,
        fonts: FontSizeContext,
    ) -> f32 {
        match direction {
            FlexDirection::Row => self.resolved_column_gap_against_fonts(cb.width, fonts),
            FlexDirection::Column => {
                self.resolved_row_gap_against_fonts(definite_length(cb.height).or(cb.width), fonts)
            }
        }
    }

    /// Flex 交叉轴 / wrap 行间 gap：Row→row-gap；Column→column-gap。
    pub fn cross_gap(&self, direction: FlexDirection) -> f32 {
        self.cross_gap_against(direction, ParentBox::default())
    }

    /// 交叉轴 gap，携带 CB 供 `%` 解析。
    /// Row 的 row-gap `%`：定高优先，否则回退宽度（wrap 自动高常见路径）。
    pub fn cross_gap_against(&self, direction: FlexDirection, cb: ParentBox) -> f32 {
        self.cross_gap_against_fonts(direction, cb, FontSizeContext::default())
    }

    /// 同 [`Self::cross_gap_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn cross_gap_against_fonts(
        &self,
        direction: FlexDirection,
        cb: ParentBox,
        fonts: FontSizeContext,
    ) -> f32 {
        match direction {
            FlexDirection::Row => {
                self.resolved_row_gap_against_fonts(definite_length(cb.height).or(cb.width), fonts)
            }
            FlexDirection::Column => self.resolved_column_gap_against_fonts(cb.width, fonts),
        }
    }

    /// Per-side border widths (physical px). Longhands override the uniform
    /// [`Self::border_width`] shorthand. `border-style: none` zeros that side.
    pub fn resolved_border_edges(&self) -> PaddingSpec {
        let uniform = self.border_width.unwrap_or(0.0).max(0.0);
        let styles = self.resolved_border_styles();
        PaddingSpec {
            top: used_border_width(self.border_top_width.unwrap_or(uniform), styles[0]),
            right: used_border_width(self.border_right_width.unwrap_or(uniform), styles[1]),
            bottom: used_border_width(self.border_bottom_width.unwrap_or(uniform), styles[2]),
            left: used_border_width(self.border_left_width.unwrap_or(uniform), styles[3]),
        }
    }

    /// Per-side styles. Longhands override [`Self::border_style`].
    pub fn resolved_border_styles(&self) -> [Option<BorderStyle>; 4] {
        let uniform = self.border_style;
        [
            self.border_top_style.or(uniform),
            self.border_right_style.or(uniform),
            self.border_bottom_style.or(uniform),
            self.border_left_style.or(uniform),
        ]
    }

    /// Per-side colors. Longhands override [`Self::border_color`].
    pub fn resolved_border_edge_colors(&self) -> [Option<[f32; 4]>; 4] {
        let uniform = self.border_color;
        [
            self.border_top_color.or(uniform),
            self.border_right_color.or(uniform),
            self.border_bottom_color.or(uniform),
            self.border_left_color.or(uniform),
        ]
    }

    /// First available border color (uniform or any side).
    pub fn resolved_border_color(&self) -> Option<[f32; 4]> {
        self.resolved_border_edge_colors()
            .into_iter()
            .flatten()
            .next()
    }

    /// Stroke widths for the existing rounded-box path (`none` / unsupported → 0).
    pub fn paint_border_edges(&self) -> PaddingSpec {
        let layout = self.resolved_border_edges();
        let styles = self.resolved_border_styles();
        let colors = self.resolved_border_edge_colors();
        PaddingSpec {
            top: paint_border_width(layout.top, styles[0], colors[0]),
            right: paint_border_width(layout.right, styles[1], colors[1]),
            bottom: paint_border_width(layout.bottom, styles[2], colors[2]),
            left: paint_border_width(layout.left, styles[3], colors[3]),
        }
    }

    /// Per-side colors with zero-alpha on sides that do not stroke.
    pub fn paint_border_edge_colors(&self) -> [[f32; 4]; 4] {
        let colors = self.resolved_border_edge_colors();
        let styles = self.resolved_border_styles();
        let mut out = [[0.0; 4]; 4];
        for (i, color) in colors.iter().enumerate() {
            if edge_paints_stroke(styles[i])
                && let Some(c) = *color
            {
                out[i] = c;
            }
        }
        out
    }

    /// Per-side shader codes (T,R,B,L): 0 solid, 1 dashed, 2 dotted.
    pub fn paint_border_style_codes(&self) -> [u8; 4] {
        let styles = self.resolved_border_styles();
        [
            styles[0].map(BorderStyle::shader_code).unwrap_or(0),
            styles[1].map(BorderStyle::shader_code).unwrap_or(0),
            styles[2].map(BorderStyle::shader_code).unwrap_or(0),
            styles[3].map(BorderStyle::shader_code).unwrap_or(0),
        ]
    }

    pub fn paints_any_border(&self) -> bool {
        let edges = self.paint_border_edges();
        !edges.is_zero()
    }

    /// Uniform stroke used by the current rounded-box shader (max of four sides).
    pub fn resolved_border_width(&self) -> f32 {
        let edges = self.resolved_border_edges();
        edges.top.max(edges.right).max(edges.bottom).max(edges.left)
    }

    /// Clip rect for descendants, expanded on axes that stay `overflow: visible`.
    pub fn overflow_clip_box(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<(f32, f32, f32, f32)> {
        overflow_clip_rect(
            x,
            y,
            width,
            height,
            self.overflow_x.clips(),
            self.overflow_y.clips(),
        )
    }

    /// Resolve padding to px without a containing-block width (`%` → 0).
    pub fn resolved_padding(&self) -> PaddingSpec {
        self.resolved_padding_against(None)
    }

    /// Resolve padding; `%` / calc 相对包含块宽度 `percent_base`。
    /// `em`/`rem` 用本节点 [`Self::font_size_context`]（缺省 16px）。
    pub fn resolved_padding_against(&self, percent_base: Option<f32>) -> PaddingSpec {
        self.resolved_padding_against_fonts(percent_base, self.font_size_context(16.0))
    }

    /// 同 [`Self::resolved_padding_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_padding_against_fonts(
        &self,
        percent_base: Option<f32>,
        fonts: FontSizeContext,
    ) -> PaddingSpec {
        resolve_box_edge_specs(
            self.padding,
            self.padding_top,
            self.padding_right,
            self.padding_bottom,
            self.padding_left,
            percent_base,
            fonts,
        )
    }

    /// Resolve margin to px without a containing-block width (`%` → 0).
    pub fn resolved_margin(&self) -> PaddingSpec {
        self.resolved_margin_against(None)
    }

    /// Resolve margin; `%` / calc 相对包含块宽度 `percent_base`（含上下边）。
    /// CSS 允许负 margin；不钳制到 0。
    pub fn resolved_margin_against(&self, percent_base: Option<f32>) -> PaddingSpec {
        self.resolved_margin_against_fonts(percent_base, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_margin_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_margin_against_fonts(
        &self,
        percent_base: Option<f32>,
        fonts: FontSizeContext,
    ) -> PaddingSpec {
        resolve_box_edge_specs_signed(
            self.margin,
            self.margin_top,
            self.margin_right,
            self.margin_bottom,
            self.margin_left,
            percent_base,
            fonts,
        )
    }

    /// `min-width` → 非负 px；无法解析时 0。
    pub fn resolved_min_width(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> f32 {
        self.resolved_min_width_fonts(percent_base, viewport, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_min_width`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_min_width_fonts(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> f32 {
        resolve_min_size(self.min_width, percent_base, viewport, fonts)
    }

    /// `max-width` → 有限非负 px；`Fill`/`Auto`/`Shrink` 或无法解析 → `None`（不钳制）。
    pub fn resolved_max_width(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> Option<f32> {
        self.resolved_max_width_fonts(percent_base, viewport, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_max_width`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_max_width_fonts(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> Option<f32> {
        resolve_max_size(self.max_width, percent_base, viewport, fonts)
    }

    /// `min-height` → 非负 px；无法解析时 0。
    pub fn resolved_min_height(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> f32 {
        self.resolved_min_height_fonts(percent_base, viewport, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_min_height`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_min_height_fonts(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> f32 {
        resolve_min_size(self.min_height, percent_base, viewport, fonts)
    }

    /// `max-height` → 有限非负 px；无有限上限 → `None`。
    pub fn resolved_max_height(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> Option<f32> {
        self.resolved_max_height_fonts(percent_base, viewport, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_max_height`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_max_height_fonts(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> Option<f32> {
        resolve_max_size(self.max_height, percent_base, viewport, fonts)
    }

    /// `min-width: 0` / `0px` / `0%` 等零值。
    pub fn has_zero_min_width(&self) -> bool {
        matches!(self.min_width, Some(LengthSpec::Px(v)) if v == 0.0)
            || matches!(self.min_width, Some(LengthSpec::Percent(p)) if p == 0.0)
            || matches!(self.min_width, Some(LengthSpec::Em(v) | LengthSpec::Rem(v)) if v == 0.0)
    }

    /// `position: relative` 画盒偏移（逻辑像素）。无 `%` 基时仅兑现 `Px`。
    pub fn relative_offset(&self) -> (f32, f32) {
        self.relative_offset_against(None, None)
    }

    /// `relative` inset；`%` 相对 containing block 宽/高。优先 `left`/`top`。
    pub fn relative_offset_against(&self, base_w: Option<f32>, base_h: Option<f32>) -> (f32, f32) {
        self.relative_offset_against_fonts(base_w, base_h, FontSizeContext::default())
    }

    /// 同 [`Self::relative_offset_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn relative_offset_against_fonts(
        &self,
        base_w: Option<f32>,
        base_h: Option<f32>,
        fonts: FontSizeContext,
    ) -> (f32, f32) {
        if !self.position.applies_relative_offset() {
            return (0.0, 0.0);
        }
        let dx = self
            .offset_left
            .and_then(|l| l.resolve_with_fonts(base_w, None, fonts))
            .or_else(|| {
                self.offset_right
                    .and_then(|r| r.resolve_with_fonts(base_w, None, fonts))
                    .map(|v| -v)
            })
            .unwrap_or(0.0);
        let dy = self
            .offset_top
            .and_then(|t| t.resolve_with_fonts(base_h, None, fonts))
            .or_else(|| {
                self.offset_bottom
                    .and_then(|b| b.resolve_with_fonts(base_h, None, fonts))
                    .map(|v| -v)
            })
            .unwrap_or(0.0);
        (dx, dy)
    }

    /// Resolve a single inset against a containing-block edge length.
    /// `em`/`rem` 使用 [`FontSizeContext::default`]（16px）。
    pub fn resolve_inset(spec: Option<LengthSpec>, base: f32) -> Option<f32> {
        Self::resolve_inset_fonts(spec, base, FontSizeContext::default())
    }

    /// 同 [`Self::resolve_inset`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolve_inset_fonts(
        spec: Option<LengthSpec>,
        base: f32,
        fonts: FontSizeContext,
    ) -> Option<f32> {
        spec.and_then(|s| s.resolve_with_fonts(Some(base), None, fonts))
    }

    pub fn is_absolute(&self) -> bool {
        self.position.is_out_of_flow_absolute()
    }

    pub fn is_fixed(&self) -> bool {
        self.position.is_out_of_flow_fixed()
    }

    /// `absolute` 或 `fixed`：脱正常文档流。
    pub fn is_out_of_flow(&self) -> bool {
        self.position.is_out_of_flow()
    }

    /// Ellipsis paint intent (`text-overflow` or `line-clamp`).
    pub fn uses_text_ellipsis(&self) -> bool {
        self.text_overflow_ellipsis || self.line_clamp.is_some_and(|n| n > 0)
    }

    /// `line-clamp` / `-webkit-line-clamp` line cap.
    pub fn resolved_line_clamp(&self) -> Option<u16> {
        self.line_clamp.filter(|n| *n > 0)
    }

    /// CSS `direction: rtl` used value (`None` / `ltr` → false).
    pub fn is_rtl(&self) -> bool {
        matches!(self.dir, Some(DirSpec::Rtl))
    }

    /// Map stored logical inline edges onto used `padding_*` / `margin_*` /
    /// `offset_*` left/right using the current [`Self::dir`].
    ///
    /// Physical-only styles (no logical inline specs) are left untouched so
    /// hand-built `LayoutStyle { padding_left, .. }` stays intact.
    pub fn resolve_logical_box_edges(&mut self) {
        let rtl = self.is_rtl();
        if self.logical_padding.has_logical() {
            self.padding_left = self.logical_padding.used_left(rtl);
            self.padding_right = self.logical_padding.used_right(rtl);
        }
        if self.logical_margin.has_logical() {
            self.margin_left = self.logical_margin.used_left(rtl);
            self.margin_right = self.logical_margin.used_right(rtl);
        }
        if self.logical_inset.has_logical() {
            self.offset_left = self.logical_inset.used_left(rtl);
            self.offset_right = self.logical_inset.used_right(rtl);
        }
    }

    /// Fill unset inherited typography from `parent` (CSS inheritance).
    pub fn inherit_typography_from(&mut self, parent: &Self) {
        if self.dir.is_none() {
            self.dir = parent.dir;
        }
        self.resolve_logical_box_edges();
        if self.font_size.is_none() {
            self.font_size = parent.font_size;
        }
        if self.font_weight.is_none() {
            self.font_weight = parent.font_weight;
        }
        if self.font_family.is_none() {
            self.font_family = parent.font_family.clone();
        }
        if self.line_height.is_none() {
            self.line_height = parent.line_height;
        }
        if self.letter_spacing.is_none() {
            self.letter_spacing = parent.letter_spacing;
        }
        if self.color.is_none() {
            self.color = parent.color;
        }
        if self.text_decoration.is_none() {
            self.text_decoration = parent.text_decoration;
        }
        if self.font_features.is_none() {
            self.font_features = parent.font_features.clone();
        }
    }

    /// `em`/`rem` context for this element's lengths (`em` uses computed font-size).
    pub fn font_size_context(&self, root_px: f32) -> FontSizeContext {
        let root = root_px.max(0.0);
        FontSizeContext::new(root, self.font_size.unwrap_or(root))
    }

    pub fn grows(&self) -> bool {
        self.flex_grow.map(|g| g > 0.0).unwrap_or(false)
    }

    pub fn scrolls_y(&self) -> bool {
        self.overflow_y.scrolls()
    }

    /// Either axis is `overflow: hidden` / `clip` — paint must clip children.
    pub fn clips_overflow(&self) -> bool {
        self.overflow_x.clips() || self.overflow_y.clips()
    }

    /// This box resolves against the viewport, not only against its containing
    /// block: `position: fixed`, or any `vw` / `vh` / `vmin` / `vmax` length.
    ///
    /// A viewport change moves such a box even when its containing block keeps
    /// the exact same size, so incremental relayout cannot reuse it.
    pub fn depends_on_viewport(&self) -> bool {
        if self.position == PositionSpec::Fixed {
            return true;
        }
        [
            self.gap,
            self.row_gap,
            self.column_gap,
            self.padding,
            self.padding_top,
            self.padding_right,
            self.padding_bottom,
            self.padding_left,
            self.logical_padding.start,
            self.logical_padding.end,
            self.logical_padding.phys_left,
            self.logical_padding.phys_right,
            self.logical_margin.start,
            self.logical_margin.end,
            self.logical_margin.phys_left,
            self.logical_margin.phys_right,
            self.margin,
            self.margin_top,
            self.margin_right,
            self.margin_bottom,
            self.margin_left,
            self.offset_top,
            self.offset_right,
            self.offset_bottom,
            self.offset_left,
            self.logical_inset.start,
            self.logical_inset.end,
            self.logical_inset.phys_left,
            self.logical_inset.phys_right,
            self.width,
            self.height,
            self.min_width,
            self.max_width,
            self.min_height,
            self.max_height,
            self.flex_basis,
        ]
        .into_iter()
        .flatten()
        .any(LengthSpec::depends_on_viewport)
    }

    /// Internal `hidden` or `display: none` — skip layout flow.
    ///
    /// CSS `visibility: hidden` does **not** omit the box; see
    /// [`Self::is_paint_visible`].
    pub fn omits_box(&self) -> bool {
        self.hidden || matches!(self.display, Some(DisplaySpec::None))
    }

    /// Generates a layout / paint box. `display:contents` does not.
    pub fn generates_box(&self) -> bool {
        !self.omits_box() && !self.display.is_some_and(DisplaySpec::is_contents)
    }

    /// 行内级盒子（块容器里走 IFC 子集；flex/grid 项会被块化）。
    pub fn is_inline_level(&self) -> bool {
        self.display.is_some_and(DisplaySpec::is_inline_level)
    }

    pub fn is_floated(&self) -> bool {
        !self.float.is_none() && !self.position.is_out_of_flow()
    }

    /// 某边 margin 是否为 `auto`（含 `margin: 0 auto` 简写）。
    pub fn margin_auto_left(&self) -> bool {
        edge_is_auto(self.margin_left, self.margin)
    }

    pub fn margin_auto_right(&self) -> bool {
        edge_is_auto(self.margin_right, self.margin)
    }

    pub fn margin_auto_top(&self) -> bool {
        edge_is_auto(self.margin_top, self.margin)
    }

    pub fn margin_auto_bottom(&self) -> bool {
        edge_is_auto(self.margin_bottom, self.margin)
    }

    /// 第一行基线相对 border-box 顶边的近似偏移。
    pub fn approximate_baseline(&self, fallback_font_px: f32) -> f32 {
        let font = self.font_size.unwrap_or(fallback_font_px).max(0.0);
        let pad = self.resolved_padding_against(None);
        let border = self.resolved_border_edges();
        pad.top + border.top + font * 0.8
    }

    /// 查找命名网格线（1-based）。含 `name-start` / `name-end` 由 areas 推导。
    pub fn named_column_line(&self, name: &str) -> Option<i32> {
        self.named_column_line_nth(name, 1)
    }

    /// 第 `occurrence` 根同名列线（1-based occurrence，1 = 第一根）。
    pub fn named_column_line_nth(&self, name: &str, occurrence: u32) -> Option<i32> {
        self.named_column_line_nth_from(name, occurrence, None)
    }

    /// Like [`Self::named_column_line_nth`], but resolve against expanded names
    /// (auto-fit / repeat copies) when `names` is `Some`.
    pub fn named_column_line_nth_from(
        &self,
        name: &str,
        occurrence: u32,
        names: Option<&[Vec<String>]>,
    ) -> Option<i32> {
        lookup_named_line_nth(
            name,
            names.or(self.grid_column_line_names.as_deref()),
            self.grid_template_areas.as_ref(),
            true,
            occurrence,
        )
    }

    /// 列线上 `after_line`（1-based）之后的下一根同名线。
    pub fn named_column_line_after(&self, name: &str, after_line: i32) -> Option<i32> {
        self.named_column_line_after_from(name, after_line, None)
    }

    pub fn named_column_line_after_from(
        &self,
        name: &str,
        after_line: i32,
        names: Option<&[Vec<String>]>,
    ) -> Option<i32> {
        lookup_named_line_after(
            name,
            names.or(self.grid_column_line_names.as_deref()),
            after_line,
        )
    }

    pub fn named_row_line(&self, name: &str) -> Option<i32> {
        self.named_row_line_nth(name, 1)
    }

    pub fn named_row_line_nth(&self, name: &str, occurrence: u32) -> Option<i32> {
        self.named_row_line_nth_from(name, occurrence, None)
    }

    pub fn named_row_line_nth_from(
        &self,
        name: &str,
        occurrence: u32,
        names: Option<&[Vec<String>]>,
    ) -> Option<i32> {
        lookup_named_line_nth(
            name,
            names.or(self.grid_row_line_names.as_deref()),
            self.grid_template_areas.as_ref(),
            false,
            occurrence,
        )
    }

    pub fn named_row_line_after(&self, name: &str, after_line: i32) -> Option<i32> {
        self.named_row_line_after_from(name, after_line, None)
    }

    pub fn named_row_line_after_from(
        &self,
        name: &str,
        after_line: i32,
        names: Option<&[Vec<String>]>,
    ) -> Option<i32> {
        lookup_named_line_after(
            name,
            names.or(self.grid_row_line_names.as_deref()),
            after_line,
        )
    }

    /// `justify-self: auto` → 容器 `justify-items`（缺省 Stretch，对齐 CSS Grid）。
    pub fn resolved_justify_self(&self, parent_justify_items: Option<AlignSpec>) -> AlignSpec {
        self.justify_self
            .or(parent_justify_items)
            .unwrap_or(AlignSpec::Stretch)
    }

    /// Explicit `grid-template-columns` tracks that participate in layout.
    ///
    /// CSS: `grid-template-columns` is inert on flex containers. Tracks remain
    /// active when `display` is `grid` / `inline-grid` / unset (compat) or other
    /// non-flex values.
    ///
    /// Implicit columns come from [`Self::grid_auto_columns`] / auto-placement.
    /// [`Self::grid_columns_repeat`] expands `repeat(auto-fit|auto-fill)` at layout.
    /// If [`Self::grid_columns_unsupported`] is set, the author wrote
    /// mixed/unexpandable syntax (not a successful `repeat(auto-fit|fill)`).
    pub fn active_grid_columns(&self) -> Option<&[GridTrack]> {
        if self.display.is_some_and(DisplaySpec::is_flex_container) {
            return None;
        }
        self.grid_columns.as_deref().filter(|t| !t.is_empty())
    }

    /// Row tracks that participate in layout (see [`Self::active_grid_columns`]).
    ///
    /// Implicit rows come from [`Self::grid_auto_rows`] / auto-placement.
    pub fn active_grid_rows(&self) -> Option<&[GridTrack]> {
        if self.display.is_some_and(DisplaySpec::is_flex_container) {
            return None;
        }
        self.grid_rows.as_deref().filter(|t| !t.is_empty())
    }

    /// Author wrote `grid-auto-*` / `grid-auto-flow` (consumed by 2D placement).
    pub fn has_deferred_grid_auto(&self) -> bool {
        self.grid_auto_columns
            .as_ref()
            .is_some_and(|t| !t.is_empty())
            || self.grid_auto_rows.as_ref().is_some_and(|t| !t.is_empty())
            || self.grid_auto_flow.is_some()
    }

    /// 作者写了无法展开的模板（`subgrid`、嵌套 auto-fit / auto-fill、混写垃圾等）。
    ///
    /// 成功的 `repeat(auto-fit|auto-fill)` 写在 [`Self::grid_columns_repeat`] /
    /// [`Self::grid_rows_repeat`]，**不**置本旗标。
    pub fn has_unsupported_grid_template(&self) -> bool {
        self.grid_columns_unsupported.is_some() || self.grid_rows_unsupported.is_some()
    }

    /// Paint-order stacking context (not compositing). Opacity groups are
    /// separate in Scene; this covers `isolation` and positioned + `z-index`.
    pub fn creates_paint_stacking_context(&self) -> bool {
        self.isolation || (self.z_index.is_some() && self.position.is_positioned())
    }

    /// 子项主轴 Length：Row→width，Column→height。
    pub fn child_main_length(&self, parent_direction: FlexDirection) -> Option<LengthSpec> {
        if self.grows() {
            return Some(LengthSpec::Fill);
        }
        if let Some(basis) = self.flex_basis
            && !matches!(basis, LengthSpec::Auto)
        {
            return Some(basis);
        }
        match parent_direction {
            FlexDirection::Row => self.width,
            FlexDirection::Column => self.height,
        }
    }

    /// 根据自身 LengthSpec + 父盒推算子级可用 ParentBox（定高链 / %）。
    ///
    /// `ParentBox` 始终是**内容区**（供后代 `%` / 定高链）：
    /// - `content-box` + 声明 `Px`/`%`/`calc`：声明值即内容宽/高，不减 chrome
    /// - `Fill` / `100%`→Fill / grow 分配：分配量是 **border-box**，内容区 =
    ///   分配 − padding − border（与 CSS 在 content-box 下对 flexed/filled
    ///   border-box 的内容区一致；勿把分配量直接当内容区）
    /// - `border-box`：声明/分配含 padding+border，内容区同样减 chrome
    pub fn resolve_content_box(&self, parent: ParentBox) -> ParentBox {
        self.resolve_content_box_with_viewport(parent, None)
    }

    /// Same as [`Self::resolve_content_box`], with viewport for `vw`/`vh`/`min()`…
    pub fn resolve_content_box_with_viewport(
        &self,
        parent: ParentBox,
        viewport: Option<(f32, f32)>,
    ) -> ParentBox {
        let pad = self.resolved_padding_against(parent.width);
        let border = self.resolved_border_edges();
        let content_box = matches!(self.box_sizing, BoxSizing::ContentBox);
        let chrome_w = pad.left + pad.right + border.left + border.right;
        let chrome_h = pad.top + pad.bottom + border.top + border.bottom;
        let width = self
            .width
            .and_then(|w| w.resolve_with(parent.width, viewport))
            .or_else(|| {
                if matches!(self.width, Some(LengthSpec::Fill))
                    || (self.width.is_none() && self.grows())
                {
                    parent.width
                } else {
                    None
                }
            })
            .map(|w| Self::axis_content_extent(w, chrome_w, content_box, self.width));
        let height = self
            .height
            .and_then(|h| h.resolve_with(parent.height, viewport))
            .or_else(|| {
                if matches!(self.height, Some(LengthSpec::Fill)) || self.grows() {
                    parent.height
                } else {
                    let mh = self.resolved_min_height(parent.height, viewport);
                    if mh > 0.0 { Some(mh) } else { None }
                }
            })
            .or(parent
                .height
                .filter(|_| matches!(self.height, Some(LengthSpec::Fill))))
            .map(|h| Self::axis_content_extent(h, chrome_h, content_box, self.height));
        ParentBox { width, height }
    }

    /// One axis of [`Self::resolve_content_box`]: map resolved/allocated size → content extent.
    fn axis_content_extent(
        resolved: f32,
        chrome: f32,
        content_box: bool,
        axis_spec: Option<LengthSpec>,
    ) -> f32 {
        if content_box && axis_spec.is_some_and(LengthSpec::is_definite_declared) {
            // Declared length is already the content size.
            resolved.max(0.0)
        } else {
            // Border-box declaration, or Fill/grow allocation (border-box share).
            (resolved - chrome).max(0.0)
        }
    }

    pub fn ensure_direction(&mut self, dir: FlexDirection) {
        if self.direction.is_none() {
            self.direction = Some(dir);
        }
    }
}

/// Positive definite length for `%` bases; `None` / `≤0` treated as indefinite.
fn definite_length(v: Option<f32>) -> Option<f32> {
    v.filter(|n| *n > 0.0)
}

fn resolve_corner_radius(spec: LengthSpec, width: f32, height: f32) -> f32 {
    match spec {
        LengthSpec::Px(v) => v.max(0.0),
        LengthSpec::Percent(p) => {
            let horizontal = width.max(0.0) * p / 100.0;
            let vertical = height.max(0.0) * p / 100.0;
            let raw = horizontal.min(vertical).max(0.0);
            let max_corner = width.min(height).max(0.0) / 2.0;
            raw.min(max_corner)
        }
        other => other
            .resolve_px(Some(width.max(0.0)))
            .unwrap_or(0.0)
            .max(0.0),
    }
}

fn edge_is_auto(longhand: Option<LengthSpec>, uniform: Option<LengthSpec>) -> bool {
    matches!(longhand.or(uniform), Some(LengthSpec::Auto))
}

fn lookup_named_line_after(
    name: &str,
    lines: Option<&[Vec<String>]>,
    after_line: i32,
) -> Option<i32> {
    let lines = lines?;
    for (i, names) in lines.iter().enumerate() {
        let line = (i as i32) + 1;
        if line > after_line && names.iter().any(|n| n == name) {
            return Some(line);
        }
    }
    None
}

fn lookup_named_line_nth(
    name: &str,
    lines: Option<&[Vec<String>]>,
    areas: Option<&GridTemplateAreas>,
    columns: bool,
    occurrence: u32,
) -> Option<i32> {
    let want = occurrence.max(1);
    let mut seen = 0u32;
    if let Some(lines) = lines {
        for (i, names) in lines.iter().enumerate() {
            if names.iter().any(|n| n == name) {
                seen += 1;
                if seen == want {
                    return Some((i as i32) + 1);
                }
            }
        }
    }
    if want != 1 {
        return None;
    }
    let areas = areas?;
    if let Some((col, row, _col_span, _row_span)) = areas.lookup(name) {
        return Some(if columns {
            col as i32 + 1
        } else {
            row as i32 + 1
        });
    }
    let start = name.strip_suffix("-start");
    let end = name.strip_suffix("-end");
    if let Some(base) = start {
        let (col, row, _, _) = areas.lookup(base)?;
        return Some(if columns {
            col as i32 + 1
        } else {
            row as i32 + 1
        });
    }
    if let Some(base) = end {
        let (col, row, col_span, row_span) = areas.lookup(base)?;
        return Some(if columns {
            col as i32 + col_span as i32 + 1
        } else {
            row as i32 + row_span as i32 + 1
        });
    }
    None
}

fn resolve_edge_length(
    spec: Option<LengthSpec>,
    percent_base: Option<f32>,
    fonts: FontSizeContext,
) -> Option<f32> {
    spec.and_then(|s| match s {
        LengthSpec::Fill
        | LengthSpec::Shrink
        | LengthSpec::Auto
        | LengthSpec::MinContent
        | LengthSpec::MaxContent
        | LengthSpec::FitContent => None,
        other => other.resolve_non_negative_fonts(percent_base, None, fonts),
    })
}

/// Margin 边长：允许负值（CSS Values / Box Model）。
fn resolve_edge_length_signed(
    spec: Option<LengthSpec>,
    percent_base: Option<f32>,
    fonts: FontSizeContext,
) -> Option<f32> {
    spec.and_then(|s| match s {
        LengthSpec::Fill
        | LengthSpec::Shrink
        | LengthSpec::Auto
        | LengthSpec::MinContent
        | LengthSpec::MaxContent
        | LengthSpec::FitContent => None,
        other => other.resolve_with_fonts(percent_base, None, fonts),
    })
}

fn resolve_min_size(
    spec: Option<LengthSpec>,
    percent_base: Option<f32>,
    viewport: Option<(f32, f32)>,
    fonts: FontSizeContext,
) -> f32 {
    match spec {
        None
        | Some(LengthSpec::Auto)
        | Some(LengthSpec::Shrink)
        | Some(LengthSpec::Fill)
        | Some(LengthSpec::MinContent)
        | Some(LengthSpec::MaxContent)
        | Some(LengthSpec::FitContent) => 0.0,
        Some(other) => other
            .resolve_non_negative_fonts(percent_base, viewport, fonts)
            .unwrap_or(0.0),
    }
}

fn resolve_max_size(
    spec: Option<LengthSpec>,
    percent_base: Option<f32>,
    viewport: Option<(f32, f32)>,
    fonts: FontSizeContext,
) -> Option<f32> {
    match spec? {
        LengthSpec::Fill
        | LengthSpec::Auto
        | LengthSpec::Shrink
        | LengthSpec::MinContent
        | LengthSpec::MaxContent
        | LengthSpec::FitContent => None,
        other => other
            .resolve_non_negative_fonts(percent_base, viewport, fonts)
            .filter(|v| v.is_finite()),
    }
}

fn resolve_box_edge_specs(
    uniform: Option<LengthSpec>,
    top: Option<LengthSpec>,
    right: Option<LengthSpec>,
    bottom: Option<LengthSpec>,
    left: Option<LengthSpec>,
    percent_base: Option<f32>,
    fonts: FontSizeContext,
) -> PaddingSpec {
    let u = resolve_edge_length(uniform, percent_base, fonts).unwrap_or(0.0);
    PaddingSpec {
        top: resolve_edge_length(top, percent_base, fonts)
            .unwrap_or(u)
            .max(0.0),
        right: resolve_edge_length(right, percent_base, fonts)
            .unwrap_or(u)
            .max(0.0),
        bottom: resolve_edge_length(bottom, percent_base, fonts)
            .unwrap_or(u)
            .max(0.0),
        left: resolve_edge_length(left, percent_base, fonts)
            .unwrap_or(u)
            .max(0.0),
    }
}

fn resolve_box_edge_specs_signed(
    uniform: Option<LengthSpec>,
    top: Option<LengthSpec>,
    right: Option<LengthSpec>,
    bottom: Option<LengthSpec>,
    left: Option<LengthSpec>,
    percent_base: Option<f32>,
    fonts: FontSizeContext,
) -> PaddingSpec {
    let u = resolve_edge_length_signed(uniform, percent_base, fonts).unwrap_or(0.0);
    PaddingSpec {
        top: resolve_edge_length_signed(top, percent_base, fonts).unwrap_or(u),
        right: resolve_edge_length_signed(right, percent_base, fonts).unwrap_or(u),
        bottom: resolve_edge_length_signed(bottom, percent_base, fonts).unwrap_or(u),
        left: resolve_edge_length_signed(left, percent_base, fonts).unwrap_or(u),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BackgroundImage, BorderImageSlice, BorderImageSpec, BorderStyle, BoxSizing, DisplaySpec,
        FlexDirection, FontSizeContext, GridRepeatAuto, LayoutStyle, LengthSpec, LineHeightSpec,
        OverflowSpec, PaintMat4, PaintTransform, ParentBox, PointerEventsSpec,
        TEXT_APPROX_ASCENT_EM, TransformBox, TransformOrigin, VisibilitySpec,
        glyph_box_center_from_line_top, icon_y_on_text_glyph_center, text_line_box_height_px,
    };

    #[test]
    fn scrollable_overflow_is_also_a_paint_clip() {
        assert!(OverflowSpec::Hidden.clips());
        assert!(OverflowSpec::Auto.clips());
        assert!(OverflowSpec::Scroll.clips());
        assert!(!OverflowSpec::Visible.clips());
    }

    #[test]
    fn border_image_tiles_use_slice_and_clamp_to_box() {
        let spec = BorderImageSpec {
            source: BackgroundImage::url("frame.png"),
            slice: [BorderImageSlice::Number(30.0); 4],
            fill: true,
        };
        let tiles = spec.tiles(90.0, 90.0, 100.0, 80.0);
        let center = tiles
            .iter()
            .find(|tile| (tile.dest_x - 30.0).abs() < 0.01 && (tile.dest_y - 30.0).abs() < 0.01)
            .expect("fill center");
        assert!((center.dest_w - 40.0).abs() < 0.01);
        assert!((center.dest_h - 20.0).abs() < 0.01);
        assert!((center.u0 - 30.0 / 90.0).abs() < 0.01);

        let tight = spec.tiles(90.0, 90.0, 40.0, 40.0);
        let tl = tight
            .iter()
            .find(|tile| tile.dest_x.abs() < 0.01 && tile.dest_y.abs() < 0.01)
            .expect("tl");
        assert!((tl.dest_w - 20.0).abs() < 0.01);
        assert!((tl.dest_h - 20.0).abs() < 0.01);
    }

    #[test]
    fn four_side_border_widths_and_none_style() {
        let layout = LayoutStyle {
            border_top_width: Some(1.0),
            border_right_width: Some(2.0),
            border_bottom_width: Some(3.0),
            border_left_width: Some(4.0),
            border_top_color: Some([1.0, 0.0, 0.0, 1.0]),
            border_right_color: Some([0.0, 1.0, 0.0, 1.0]),
            border_bottom_color: Some([0.0, 0.0, 1.0, 1.0]),
            border_left_color: Some([1.0, 1.0, 0.0, 1.0]),
            ..LayoutStyle::default()
        };
        let edges = layout.resolved_border_edges();
        assert!((edges.top - 1.0).abs() < f32::EPSILON);
        assert!((edges.right - 2.0).abs() < f32::EPSILON);
        assert!((edges.bottom - 3.0).abs() < f32::EPSILON);
        assert!((edges.left - 4.0).abs() < f32::EPSILON);
        assert!((layout.resolved_border_width() - 4.0).abs() < f32::EPSILON);
        let colors = layout.resolved_border_edge_colors();
        assert_eq!(colors[0], Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(colors[1], Some([0.0, 1.0, 0.0, 1.0]));
        assert_eq!(colors[2], Some([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(colors[3], Some([1.0, 1.0, 0.0, 1.0]));

        let none_left = LayoutStyle {
            border_width: Some(5.0),
            border_left_style: Some(BorderStyle::None),
            ..LayoutStyle::default()
        };
        let used = none_left.resolved_border_edges();
        assert!((used.top - 5.0).abs() < f32::EPSILON);
        assert!(used.left.abs() < f32::EPSILON);
    }

    #[test]
    fn dashed_dotted_paint_stroke_double_stays_closed() {
        let dashed = LayoutStyle {
            border_width: Some(4.0),
            border_color: Some([1.0, 0.0, 0.0, 1.0]),
            border_style: Some(BorderStyle::Dashed),
            ..LayoutStyle::default()
        };
        assert!((dashed.resolved_border_edges().top - 4.0).abs() < f32::EPSILON);
        assert!((dashed.paint_border_edges().top - 4.0).abs() < f32::EPSILON);
        assert_eq!(
            dashed.paint_border_style_codes(),
            [BorderStyle::SHADER_DASHED; 4]
        );
        assert!(dashed.paints_any_border());

        let dotted = LayoutStyle {
            border_width: Some(4.0),
            border_color: Some([1.0, 0.0, 0.0, 1.0]),
            border_style: Some(BorderStyle::Dotted),
            ..LayoutStyle::default()
        };
        assert!((dotted.paint_border_edges().top - 4.0).abs() < f32::EPSILON);
        assert_eq!(
            dotted.paint_border_style_codes(),
            [BorderStyle::SHADER_DOTTED; 4]
        );

        let double = LayoutStyle {
            border_width: Some(4.0),
            border_color: Some([1.0, 0.0, 0.0, 1.0]),
            border_style: Some(BorderStyle::Unsupported),
            ..LayoutStyle::default()
        };
        assert!((double.resolved_border_edges().top - 4.0).abs() < f32::EPSILON);
        assert!(double.paint_border_edges().is_zero());
        assert!(!double.paints_any_border());
    }

    #[test]
    fn overflow_clip_box_opens_visible_axis() {
        let mut layout = LayoutStyle::default();
        layout.overflow_x = OverflowSpec::Hidden;
        layout.overflow_y = OverflowSpec::Visible;
        let (x, y, w, h) = layout.overflow_clip_box(10.0, 20.0, 30.0, 40.0).unwrap();
        assert!((x - 10.0).abs() < f32::EPSILON);
        assert!((w - 30.0).abs() < f32::EPSILON);
        assert!(y < 20.0);
        assert!(h > 40.0);

        layout.overflow_x = OverflowSpec::Visible;
        layout.overflow_y = OverflowSpec::Hidden;
        let (x, y, w, h) = layout.overflow_clip_box(10.0, 20.0, 30.0, 40.0).unwrap();
        assert!(x < 10.0);
        assert!(w > 30.0);
        assert!((y - 20.0).abs() < f32::EPSILON);
        assert!((h - 40.0).abs() < f32::EPSILON);

        layout.overflow_x = OverflowSpec::Visible;
        layout.overflow_y = OverflowSpec::Visible;
        assert!(layout.overflow_clip_box(0.0, 0.0, 10.0, 10.0).is_none());
    }

    #[test]
    fn child_main_length_grows_to_fill() {
        let layout = LayoutStyle {
            flex_grow: Some(1.0),
            ..LayoutStyle::default()
        };
        assert_eq!(
            layout.child_main_length(FlexDirection::Row),
            Some(LengthSpec::Fill)
        );
    }

    #[test]
    fn resolve_content_box_height_chain() {
        let shell = LayoutStyle {
            height: Some(LengthSpec::Fill),
            ..LayoutStyle::default()
        };
        let parent = ParentBox::from_viewport(800.0, 600.0);
        let box_ = shell.resolve_content_box(parent);
        assert_eq!(box_.height, Some(600.0));
    }

    #[test]
    fn resolve_content_box_keeps_declared_width_under_content_box() {
        let layout = LayoutStyle {
            width: Some(LengthSpec::Px(100.0)),
            padding: Some(LengthSpec::Px(10.0)),
            box_sizing: BoxSizing::ContentBox,
            ..LayoutStyle::default()
        };
        let parent = ParentBox::from_viewport(400.0, 200.0);
        let box_ = layout.resolve_content_box(parent);
        assert_eq!(
            box_.width,
            Some(100.0),
            "content-box width is content, not border"
        );
    }

    #[test]
    fn resolve_content_box_fill_subtracts_chrome_under_content_box() {
        // Fill / 100% allocation is border-box; content area = allocation − pad − border.
        let layout = LayoutStyle {
            width: Some(LengthSpec::Fill),
            height: Some(LengthSpec::Fill),
            padding: Some(LengthSpec::Px(10.0)),
            border_width: Some(5.0),
            box_sizing: BoxSizing::ContentBox,
            ..LayoutStyle::default()
        };
        let parent = ParentBox::from_viewport(400.0, 200.0);
        let box_ = layout.resolve_content_box(parent);
        assert_eq!(
            box_.width,
            Some(370.0),
            "Fill border-box 400 − 20pad − 10border"
        );
        assert_eq!(
            box_.height,
            Some(170.0),
            "Fill border-box 200 − 20pad − 10border"
        );
    }

    #[test]
    fn resolve_content_box_grow_auto_subtracts_chrome_under_content_box() {
        // flex-grow>0 with auto main: allocated share is border-box.
        let layout = LayoutStyle {
            flex_grow: Some(1.0),
            padding: Some(LengthSpec::Px(8.0)),
            border_width: Some(2.0),
            box_sizing: BoxSizing::ContentBox,
            ..LayoutStyle::default()
        };
        let parent = ParentBox::from_viewport(300.0, 150.0);
        let box_ = layout.resolve_content_box(parent);
        assert_eq!(box_.width, Some(280.0), "300 − 16pad − 4border");
        assert_eq!(box_.height, Some(130.0), "150 − 16pad − 4border");
    }

    #[test]
    fn resolve_content_box_subtracts_padding_and_border_under_border_box() {
        let layout = LayoutStyle {
            width: Some(LengthSpec::Px(100.0)),
            padding: Some(LengthSpec::Px(10.0)),
            border_width: Some(5.0),
            box_sizing: BoxSizing::BorderBox,
            ..LayoutStyle::default()
        };
        let parent = ParentBox::from_viewport(400.0, 200.0);
        let box_ = layout.resolve_content_box(parent);
        assert_eq!(
            box_.width,
            Some(70.0),
            "border-box content = 100 - 20pad - 10border"
        );
    }

    #[test]
    fn min_height_zero_does_not_inherit_parent_height() {
        let layout = LayoutStyle {
            min_height: Some(LengthSpec::Px(0.0)),
            ..LayoutStyle::default()
        };
        let parent = ParentBox::from_viewport(800.0, 600.0);
        let box_ = layout.resolve_content_box(parent);
        assert_eq!(
            box_.height, None,
            "min-height:0 only allows shrink; Fill comes from height/flex-grow"
        );
    }

    #[test]
    fn min_height_zero_with_fill_height_still_chains() {
        let layout = LayoutStyle {
            height: Some(LengthSpec::Fill),
            min_height: Some(LengthSpec::Px(0.0)), // sentinel from CSS min-height:100%
            ..LayoutStyle::default()
        };
        let parent = ParentBox::from_viewport(800.0, 600.0);
        let box_ = layout.resolve_content_box(parent);
        assert_eq!(box_.height, Some(600.0));
    }

    #[test]
    fn row_cross_gap_percent_falls_back_when_height_indefinite() {
        let layout = LayoutStyle {
            gap: Some(LengthSpec::Percent(10.0)),
            ..LayoutStyle::default()
        };
        // Auto-height wrap: content_h is 0 before shrink-to-fit — must not lock % to 0.
        let indefinite = ParentBox::new(Some(200.0), Some(0.0));
        assert_eq!(
            layout.cross_gap_against(FlexDirection::Row, indefinite),
            20.0
        );
        let definite = ParentBox::new(Some(200.0), Some(100.0));
        assert_eq!(layout.cross_gap_against(FlexDirection::Row, definite), 10.0);
    }

    #[test]
    fn percent_resolve_px() {
        assert_eq!(
            LengthSpec::Percent(50.0).resolve_px(Some(400.0)),
            Some(200.0)
        );
    }

    #[test]
    fn padding_margin_percent_resolves_against_containing_block_width() {
        let layout = LayoutStyle {
            padding: Some(LengthSpec::Percent(10.0)),
            margin_top: Some(LengthSpec::Percent(5.0)),
            margin_left: Some(LengthSpec::Px(8.0)),
            ..LayoutStyle::default()
        };
        let pad = layout.resolved_padding_against(Some(200.0));
        let margin = layout.resolved_margin_against(Some(200.0));
        assert_eq!(pad.top, 20.0);
        assert_eq!(pad.left, 20.0);
        assert_eq!(margin.top, 10.0);
        assert_eq!(margin.left, 8.0);
        assert!(layout.resolved_padding().is_zero());
    }

    #[test]
    fn padding_em_default_api_stays_16px_fonts_uses_element_px() {
        let layout = LayoutStyle {
            padding: Some(LengthSpec::Em(1.0)),
            ..LayoutStyle::default()
        };
        let pad = layout.resolved_padding_against(None);
        assert_eq!(pad.left, 16.0);
        assert_eq!(pad.top, 16.0);
        let with_font = LayoutStyle {
            font_size: Some(32.0),
            padding: Some(LengthSpec::Em(1.0)),
            ..LayoutStyle::default()
        };
        assert_eq!(with_font.resolved_padding_against(None).left, 32.0);
        let pad32 = layout.resolved_padding_against_fonts(None, FontSizeContext::new(16.0, 32.0));
        assert_eq!(pad32.left, 32.0);
        assert_eq!(pad32.top, 32.0);
    }

    #[test]
    fn resolve_inset_em_default_api_stays_16px_fonts_uses_element_px() {
        let spec = Some(LengthSpec::Em(1.0));
        assert_eq!(LayoutStyle::resolve_inset(spec, 200.0), Some(16.0));
        assert_eq!(
            LayoutStyle::resolve_inset_fonts(spec, 200.0, FontSizeContext::new(16.0, 32.0)),
            Some(32.0)
        );
    }

    #[test]
    fn calc_percent_offset_resolve_px() {
        assert_eq!(
            LengthSpec::CalcPercentOffset {
                percent: 100.0,
                offset_px: -40.0,
            }
            .resolve_px(Some(400.0)),
            Some(360.0)
        );
        assert_eq!(
            LengthSpec::CalcPercentOffset {
                percent: 50.0,
                offset_px: 10.0,
            }
            .resolve_px(Some(200.0)),
            Some(110.0)
        );
    }

    #[test]
    fn resolve_grid_minmax_nonzero_min_freezes() {
        use super::{GridTrack, resolve_grid_column_widths};
        let tracks = [
            GridTrack::MinMax {
                min_px: 400.0,
                fr: 1.0,
                max_px: None,
            },
            GridTrack::Fr(1.0),
        ];
        let w = resolve_grid_column_widths(&tracks, 600.0, 0.0);
        assert!((w[0] - 400.0).abs() < 0.01);
        assert!((w[1] - 200.0).abs() < 0.01);
    }

    #[test]
    fn resolve_grid_auto_tracks_use_intrinsics_not_fr() {
        use super::{GridTrack, resolve_grid_column_widths, resolve_grid_track_sizes};
        let tracks = [
            GridTrack::Auto,
            GridTrack::Auto,
            GridTrack::MinMax {
                min_px: 0.0,
                fr: 1.0,
                max_px: None,
            },
        ];
        // Without intrinsics, auto → 0 and fr takes the rest.
        let bare = resolve_grid_column_widths(&tracks, 400.0, 12.0);
        assert!((bare[0] - 0.0).abs() < 0.01);
        assert!((bare[1] - 0.0).abs() < 0.01);
        assert!((bare[2] - 376.0).abs() < 0.01, "got {}", bare[2]);
        // With content contributions, auto keeps size; fr takes remainder.
        let sized = resolve_grid_track_sizes(&tracks, 400.0, 12.0, &[40.0, 80.0]);
        assert!((sized[0] - 40.0).abs() < 0.01);
        assert!((sized[1] - 80.0).abs() < 0.01);
        assert!((sized[2] - 256.0).abs() < 0.01, "got {}", sized[2]);
        assert_eq!(GridTrack::Auto.fr_weight(), None);
        assert_eq!(
            GridTrack::Auto.as_row_main_length(),
            super::LengthSpec::Shrink
        );
    }

    #[test]
    fn resolve_grid_fractional_fr_weights() {
        use super::{GridTrack, resolve_grid_column_widths};
        let tracks = [GridTrack::Fr(1.0), GridTrack::Fr(1.5)];
        let w = resolve_grid_column_widths(&tracks, 500.0, 0.0);
        assert!((w[0] - 200.0).abs() < 0.01);
        assert!((w[1] - 300.0).abs() < 0.01);
    }

    #[test]
    fn resolve_grid_multi_min_freeze_same_pass() {
        use super::{GridTrack, resolve_grid_column_widths};
        let tracks = [
            GridTrack::MinMax {
                min_px: 250.0,
                fr: 1.0,
                max_px: None,
            },
            GridTrack::MinMax {
                min_px: 250.0,
                fr: 1.0,
                max_px: None,
            },
            GridTrack::Fr(1.0),
        ];
        let w = resolve_grid_column_widths(&tracks, 600.0, 0.0);
        assert!((w[0] - 250.0).abs() < 0.01);
        assert!((w[1] - 250.0).abs() < 0.01);
        assert!((w[2] - 100.0).abs() < 0.01);
    }

    #[test]
    fn resolve_grid_minmax_px_max_clamps() {
        use super::{GridTrack, resolve_grid_column_widths};
        let tracks = [
            GridTrack::MinMax {
                min_px: 50.0,
                fr: 1.0,
                max_px: Some(120.0),
            },
            GridTrack::Fr(1.0),
        ];
        let w = resolve_grid_column_widths(&tracks, 400.0, 0.0);
        assert!((w[0] - 120.0).abs() < 0.01);
        assert!((w[1] - 280.0).abs() < 0.01);
    }

    #[test]
    fn resolve_grid_multi_max_freeze_same_pass() {
        use super::{GridTrack, resolve_grid_column_widths};
        let tracks = [
            GridTrack::MinMax {
                min_px: 0.0,
                fr: 1.0,
                max_px: Some(100.0),
            },
            GridTrack::MinMax {
                min_px: 0.0,
                fr: 1.0,
                max_px: Some(100.0),
            },
            GridTrack::Fr(1.0),
        ];
        let w = resolve_grid_column_widths(&tracks, 400.0, 0.0);
        assert!((w[0] - 100.0).abs() < 0.01);
        assert!((w[1] - 100.0).abs() < 0.01);
        assert!((w[2] - 200.0).abs() < 0.01);
    }

    #[test]
    fn resolve_grid_rows_reuse_track_resolver_with_gap() {
        use super::{GridTrack, resolve_grid_column_widths};
        // height 400, gap 20 → free after 100px + 2×gap = 260 → two 1fr = 130
        let tracks = [GridTrack::Px(100.0), GridTrack::Fr(1.0), GridTrack::Fr(1.0)];
        let h = resolve_grid_column_widths(&tracks, 400.0, 20.0);
        assert!((h[0] - 100.0).abs() < 0.01);
        assert!((h[1] - 130.0).abs() < 0.01);
        assert!((h[2] - 130.0).abs() < 0.01);
    }

    #[test]
    fn named_grid_line_nth_skips_first_occurrence() {
        let layout = LayoutStyle {
            grid_column_line_names: Some(vec![
                vec!["foo".into()],
                vec!["foo".into()],
                vec!["foo".into()],
            ]),
            ..LayoutStyle::default()
        };
        assert_eq!(layout.named_column_line("foo"), Some(1));
        assert_eq!(layout.named_column_line_nth("foo", 2), Some(2));
        assert_eq!(layout.named_column_line_nth("foo", 3), Some(3));
        assert_eq!(layout.named_column_line_after("foo", 1), Some(2));
        assert_eq!(layout.named_column_line_after("foo", 2), Some(3));
        assert_eq!(layout.named_column_line_after("foo", 3), None);
    }

    #[test]
    fn omits_box_skips_display_none_not_visibility_hidden() {
        let mut none = LayoutStyle::default();
        none.display = Some(DisplaySpec::None);
        assert!(none.omits_box());

        let mut hidden = LayoutStyle::default();
        hidden.paint.visibility = Some(VisibilitySpec::Hidden);
        assert!(!hidden.omits_box());
        assert!(!hidden.is_paint_visible());
    }

    #[test]
    fn pointer_events_none_is_not_hittable() {
        assert_eq!(
            PointerEventsSpec::parse("none"),
            Some(PointerEventsSpec::None)
        );
        assert_eq!(
            PointerEventsSpec::parse("auto"),
            Some(PointerEventsSpec::Auto)
        );
        assert_eq!(PointerEventsSpec::parse("visiblePainted"), None);
        assert!(!PointerEventsSpec::None.hittable());
        assert!(PointerEventsSpec::Auto.hittable());
        assert_eq!(
            PointerEventsSpec::inherit_from(None, PointerEventsSpec::None),
            PointerEventsSpec::None
        );
        assert_eq!(
            PointerEventsSpec::inherit_from(Some(PointerEventsSpec::Auto), PointerEventsSpec::None),
            PointerEventsSpec::Auto
        );
    }

    #[test]
    fn merge_line_name_pattern_copies_per_repetition() {
        let pattern = vec![vec!["mid".to_string()], vec!["end".to_string()]];
        let once = GridRepeatAuto::merge_line_name_pattern(&pattern, 1);
        assert_eq!(once, pattern);
        let twice = GridRepeatAuto::merge_line_name_pattern(&pattern, 2);
        assert_eq!(
            twice,
            vec![
                vec!["mid".to_string()],
                vec!["end".to_string(), "mid".to_string()],
                vec!["end".to_string()],
            ]
        );
        assert_eq!(
            twice
                .iter()
                .filter(|line| line.iter().any(|n| n == "mid"))
                .count(),
            2
        );
    }

    #[test]
    fn glyph_box_center_sits_on_the_em_square_midline() {
        let font = 12.0;
        let line = text_line_box_height_px(font, Some(LineHeightSpec::Absolute(font)));
        assert!((line - font).abs() < f32::EPSILON);
        let center = glyph_box_center_from_line_top(line, font);
        let expected = font * TEXT_APPROX_ASCENT_EM - font * 0.5;
        assert!((center - expected).abs() < 1e-5);
        assert!((center - 3.6).abs() < 1e-5);
    }

    #[test]
    fn icon_y_on_centered_text_shares_the_line_box_midline() {
        let y = icon_y_on_text_glyph_center(
            10.0,
            28.0,
            12.0,
            Some(LineHeightSpec::Absolute(12.0)),
            true,
            12.0,
        );
        assert!((y - 18.0).abs() < 1e-5);
    }

    #[test]
    fn paint_transform_then_scales_translation_on_the_same_2x3() {
        let scale_x = PaintTransform {
            a: 0.5,
            ..PaintTransform::default()
        };
        let composed = scale_x.then(PaintTransform {
            e: 10.0,
            ..PaintTransform::default()
        });
        assert!((composed.a - 0.5).abs() < 1e-5);
        assert!((composed.e - 5.0).abs() < 1e-5);
        let world = composed.around_center(0.0, 0.0, 20.0, 20.0);
        assert_eq!(world.len(), 6);
    }

    #[test]
    fn around_center_matches_50_percent_origin_not_zero_zero() {
        let rotate_90 = PaintTransform {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            ..PaintTransform::default()
        };
        let center = rotate_90.around_center(10.0, 20.0, 40.0, 20.0);
        let pct = rotate_90.around_origin(10.0, 20.0, 20.0, 10.0);
        let zero = rotate_90.around_origin(10.0, 20.0, 0.0, 0.0);
        assert_eq!(center, pct);
        assert_ne!(center, zero);
        assert_eq!(
            LayoutStyle {
                transform: Some(rotate_90),
                ..LayoutStyle::default()
            }
            .world_paint_transform(10.0, 20.0, 40.0, 20.0),
            Some(center)
        );
        assert_eq!(
            LayoutStyle {
                transform: Some(rotate_90),
                transform_origin: Some(TransformOrigin {
                    x: LengthSpec::Px(0.0),
                    y: LengthSpec::Px(0.0),
                }),
                ..LayoutStyle::default()
            }
            .world_paint_transform(10.0, 20.0, 40.0, 20.0),
            Some(zero)
        );
        assert_eq!(
            LayoutStyle {
                transform: Some(rotate_90),
                transform_origin: Some(TransformOrigin {
                    x: LengthSpec::Percent(50.0),
                    y: LengthSpec::Percent(50.0),
                }),
                ..LayoutStyle::default()
            }
            .world_paint_transform(10.0, 20.0, 40.0, 20.0),
            Some(center)
        );
    }

    #[test]
    fn transform_box_shifts_origin_onto_content_box() {
        let rotate_90 = PaintTransform {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            ..PaintTransform::default()
        };
        let origin = TransformOrigin {
            x: LengthSpec::Px(0.0),
            y: LengthSpec::Px(0.0),
        };
        let mut style = LayoutStyle {
            transform: Some(rotate_90),
            transform_origin: Some(origin),
            padding: Some(LengthSpec::Px(10.0)),
            border_width: Some(5.0),
            border_style: Some(BorderStyle::Solid),
            ..LayoutStyle::default()
        };
        let border = style.world_paint_transform(10.0, 20.0, 100.0, 50.0);
        assert_eq!(border, Some(rotate_90.around_origin(10.0, 20.0, 0.0, 0.0)));
        style.transform_box = TransformBox::ContentBox;
        let content = style.world_paint_transform(10.0, 20.0, 100.0, 50.0);
        assert_eq!(
            content,
            Some(rotate_90.around_origin(10.0, 20.0, 15.0, 15.0))
        );
        style.transform_box = TransformBox::FillBox;
        assert_eq!(
            style.world_paint_transform(10.0, 20.0, 100.0, 50.0),
            content
        );
        style.transform_box = TransformBox::ViewBox;
        assert_eq!(style.world_paint_transform(10.0, 20.0, 100.0, 50.0), border);
        style.transform_box = TransformBox::BorderBox;
        assert_eq!(style.world_paint_transform(10.0, 20.0, 100.0, 50.0), border);
        style.transform_box = TransformBox::ContentBox;
        style.transform_origin = Some(TransformOrigin {
            x: LengthSpec::Percent(100.0),
            y: LengthSpec::Percent(100.0),
        });
        assert_eq!(style.resolved_transform_origin(100.0, 50.0), [85.0, 35.0]);
        style.padding_left = Some(LengthSpec::Px(20.0));
        style.padding = None;
        style.border_width = None;
        style.border_style = None;
        style.transform_origin = Some(TransformOrigin {
            x: LengthSpec::Percent(50.0),
            y: LengthSpec::Percent(50.0),
        });
        assert_eq!(style.resolved_transform_origin(100.0, 50.0), [60.0, 25.0]);
    }

    fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
        let dx = a[0] - b[0];
        let dy = a[1] - b[1];
        (dx * dx + dy * dy).sqrt()
    }

    #[test]
    fn perspective_rotate_y_projects_a_trapezoid() {
        let mat = PaintMat4::perspective(800.0)
            .expect("d")
            .then(PaintMat4::rotate_y(30_f32.to_radians()));
        let pivoted = mat.around_origin(0.0, 0.0, 100.0, 40.0);
        let corners = pivoted
            .projected_corners(0.0, 0.0, 200.0, 80.0)
            .expect("corners");
        let left = dist(corners[0], corners[3]);
        let right = dist(corners[1], corners[2]);
        assert!(
            (left - right).abs() > 4.0,
            "rotateY+perspective must not be a parallelogram: left={left} right={right}"
        );
        let top = dist(corners[0], corners[1]);
        let bottom = dist(corners[3], corners[2]);
        assert!(
            (top - 200.0).abs() > 1.0 || (left - 80.0).abs() > 1.0,
            "projected size must change from the layout box"
        );
        assert!(
            (top - bottom).abs() < 1.0,
            "rotateY trapezoid keeps parallel top/bottom, got top={top} bottom={bottom}"
        );
        let (_, persp) = pivoted.planar_homography().expect("homography");
        assert!(
            persp[0].abs() > 1e-4,
            "perspective row g must be nonzero, got {persp:?}"
        );
        assert!(mat.as_affine().is_none(), "must not squash to 2×3");
    }

    #[test]
    fn transform_origin_moves_rotate_y_perspective_pivot() {
        let mat = PaintMat4::perspective(800.0)
            .expect("d")
            .then(PaintMat4::rotate_y(30_f32.to_radians()));
        let center = mat.around_origin(0.0, 0.0, 100.0, 40.0);
        let zero = mat.around_origin(0.0, 0.0, 0.0, 0.0);
        let c0 = center.project_xy(0.0, 0.0).unwrap();
        let z0 = zero.project_xy(0.0, 0.0).unwrap();
        assert!(
            (c0[0] - z0[0]).abs() > 1.0,
            "origin must move the projected corner, center={c0:?} zero={z0:?}"
        );
        let mut style = LayoutStyle {
            transform_3d: Some(mat),
            ..LayoutStyle::default()
        };
        let via_default = style
            .world_scene_transform(0.0, 0.0, 200.0, 80.0)
            .expect("3d");
        style.transform_origin = Some(TransformOrigin {
            x: LengthSpec::Px(0.0),
            y: LengthSpec::Px(0.0),
        });
        let via_zero = style
            .world_scene_transform(0.0, 0.0, 200.0, 80.0)
            .expect("3d zero origin");
        assert_ne!(via_default, via_zero);
        assert_eq!(style.world_paint_transform(0.0, 0.0, 200.0, 80.0), None);
    }

    #[test]
    fn rotate_x_matches_rotate3d_x_axis() {
        let angle = 25_f32.to_radians();
        let a = PaintMat4::rotate_x(angle);
        let b = PaintMat4::rotate3d(1.0, 0.0, 0.0, angle).expect("axis");
        for i in 0..16 {
            assert!(
                (a.m[i] - b.m[i]).abs() < 1e-5,
                "m[{i}] rotateX={} rotate3d={}",
                a.m[i],
                b.m[i]
            );
        }
    }
}
