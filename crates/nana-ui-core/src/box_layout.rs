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

impl OverflowSpec {
    pub fn scrolls(self) -> bool {
        matches!(self, Self::Auto | Self::Scroll)
    }

    /// `overflow: hidden` / `clip` — paint must clip descendants to the padding box.
    pub fn clips(self) -> bool {
        matches!(self, Self::Hidden | Self::Auto | Self::Scroll)
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextAlignSpec {
    #[default]
    Start,
    Center,
    End,
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

/// `repeat(auto-fit|auto-fill)` 种类。布局按容器尺寸展开轨，不再当作缺口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridTrackListUnsupported {
    /// `repeat(auto-fit, …)`
    RepeatAutoFit,
    /// `repeat(auto-fill, …)`
    RepeatAutoFill,
}

impl GridTrackListUnsupported {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RepeatAutoFit => "repeat(auto-fit)",
            Self::RepeatAutoFill => "repeat(auto-fill)",
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

/// Top of a square `extent` whose center matches the glyph box of a text block.
///
/// `vertical_center` follows [`crate`] text alignment: the line box is centered
/// in `text_bounds_height` when true, otherwise it starts at `text_bounds_y`.
pub fn icon_y_on_text_glyph_center(
    text_bounds_y: f32,
    text_bounds_height: f32,
    font_px: f32,
    line_height: Option<LineHeightSpec>,
    vertical_center: bool,
    extent: f32,
) -> f32 {
    let line_h = text_line_box_height_px(font_px, line_height);
    let line_top = if vertical_center {
        text_bounds_y + (text_bounds_height - line_h) * 0.5
    } else {
        text_bounds_y
    };
    line_top + glyph_box_center_from_line_top(line_h, font_px) - extent * 0.5
}

/// 可参与 `min`/`max`/`clamp` 的轻量长度原子（Copy；非完整 calc AST）。
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
    /// 0..=100，相对父 content box。
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

/// Single-layer outset `box-shadow` (physical px after parse).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoxShadowSpec {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: [f32; 4],
}

/// Single-layer `text-shadow` (physical px after parse; blur is paint-hint only).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextShadowSpec {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: [f32; 4],
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

/// `background-size` subset for `url()` fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BackgroundImageFit {
    #[default]
    Cover,
    Contain,
    Stretch,
}

/// Parsed `background-image` (`linear-gradient`, `radial-gradient`, or `url()`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BackgroundImage {
    Gradient(CssGradient),
    Url {
        url: String,
        fit: BackgroundImageFit,
    },
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

/// CSS `filter` brightness / saturate / contrast multipliers (default 1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorFilter {
    pub brightness: f32,
    pub saturate: f32,
    pub contrast: f32,
}

impl Default for ColorFilter {
    fn default() -> Self {
        Self {
            brightness: 1.0,
            saturate: 1.0,
            contrast: 1.0,
        }
    }
}

impl ColorFilter {
    pub fn is_identity(self) -> bool {
        (self.brightness - 1.0).abs() < 1e-5
            && (self.saturate - 1.0).abs() < 1e-5
            && (self.contrast - 1.0).abs() < 1e-5
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
    #[serde(default)]
    pub box_shadow: Option<BoxShadowSpec>,
    #[serde(default)]
    pub text_shadow: Option<TextShadowSpec>,
    #[serde(default)]
    pub background_image: Option<BackgroundImage>,
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
            || self.mask.is_some()
            || self.clip_path.is_some()
            || self.filter.is_some_and(|filter| !filter.is_identity())
            || self.backdrop_filter.is_some_and(BackdropFilter::is_active)
    }
}

/// CSS 2D affine paint transform applied without changing layout.
///
/// The six fields use the Canvas/CSS `matrix(a, b, c, d, e, f)` convention.
/// NanaUI applies the matrix around the box center, matching the default CSS
/// transform origin. Translation is expressed in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PaintTransform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
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

    /// Returns the world-space matrix using the default center transform origin.
    pub fn around_center(self, x: f32, y: f32, width: f32, height: f32) -> [f32; 6] {
        let cx = x + width * 0.5;
        let cy = y + height * 0.5;
        [
            self.a,
            self.b,
            self.c,
            self.d,
            cx + self.e - self.a * cx - self.c * cy,
            cy + self.f - self.b * cx - self.d * cy,
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

/// 可测布局意图（Style Model Layout 盒切片）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutStyle {
    pub direction: Option<FlexDirection>,
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
    /// Paint-only `transform` subset. It never participates in flex/grid
    /// measurement. Unsupported affine forms remain in
    /// [`Self::unsupported_transform`] for compatibility diagnostics.
    #[serde(default)]
    pub transform: Option<PaintTransform>,
    #[serde(default)]
    pub unsupported_transform: Option<String>,
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
    /// Uniform margin shorthand residue（`%` 合同同 padding）。
    pub margin: Option<LengthSpec>,
    pub margin_top: Option<LengthSpec>,
    pub margin_right: Option<LengthSpec>,
    pub margin_bottom: Option<LengthSpec>,
    pub margin_left: Option<LengthSpec>,
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
    /// `flex-shrink`。`None` 按 0，不是 CSS initial 1，避免溢出的定宽行被压扁。
    /// Vue 写了 `flex-shrink` 或 `flex: initial` 时仍是 `Some(1.0)`。
    pub flex_shrink: Option<f32>,
    pub flex_basis: Option<LengthSpec>,
    pub overflow_x: OverflowSpec,
    pub overflow_y: OverflowSpec,
    /// `text-overflow: ellipsis`（需配合 nowrap / 定宽；Scene text 路径兑现）。
    #[serde(default)]
    pub text_overflow_ellipsis: bool,
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
    /// `grid-template-columns` 轻量轨道（侧栏|主区）。
    pub grid_columns: Option<Vec<GridTrack>>,
    /// `grid-template-rows` 轻量轨道（堆叠区；Column 主轴）。
    #[serde(default)]
    pub grid_rows: Option<Vec<GridTrack>>,
    /// `grid-template-columns` 无法展开的语法（嵌套 auto-fit / auto-fill、坏 token）。
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
    pub border_color: Option<[f32; 4]>,
}

impl Default for LayoutStyle {
    fn default() -> Self {
        Self {
            direction: None,
            flex_reverse: false,
            order: 0,
            flex_wrap: FlexWrap::NoWrap,
            display: None,
            box_sizing: BoxSizing::BorderBox,
            position: PositionSpec::Static,
            z_index: None,
            transform: None,
            unsupported_transform: None,
            gap: None,
            row_gap: None,
            column_gap: None,
            padding: None,
            padding_top: None,
            padding_right: None,
            padding_bottom: None,
            padding_left: None,
            margin: None,
            margin_top: None,
            margin_right: None,
            margin_bottom: None,
            margin_left: None,
            offset_top: None,
            offset_right: None,
            offset_bottom: None,
            offset_left: None,
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
            border_color: None,
        }
    }
}

impl LayoutStyle {
    pub fn has_surface_paint(&self) -> bool {
        self.background.is_some()
            || self
                .resolved_border_radii(0.0, 0.0)
                .iter()
                .any(|r| *r > 0.0)
            || self.border_width.unwrap_or(0.0) > 0.0
            || self.paint.box_shadow.is_some()
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

    /// Resolve padding to px without a containing-block width (`%` → 0).
    /// Uniform border width used in box geometry (all sides).
    pub fn resolved_border_width(&self) -> f32 {
        self.border_width.unwrap_or(0.0).max(0.0)
    }

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

    /// Single-line ellipsis paint intent (`text-overflow` + typically nowrap).
    pub fn uses_text_ellipsis(&self) -> bool {
        self.text_overflow_ellipsis
    }

    /// Fill unset inherited typography from `parent` (CSS inheritance).
    pub fn inherit_typography_from(&mut self, parent: &Self) {
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
            self.margin,
            self.margin_top,
            self.margin_right,
            self.margin_bottom,
            self.margin_left,
            self.offset_top,
            self.offset_right,
            self.offset_bottom,
            self.offset_left,
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
        let border = self.resolved_border_width();
        pad.top + border + font * 0.8
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

    /// 作者写了无法展开的模板（嵌套 auto-fit / auto-fill、混写垃圾等）。
    ///
    /// 成功的 `repeat(auto-fit|auto-fill)` 写在 [`Self::grid_columns_repeat`] /
    /// [`Self::grid_rows_repeat`]，**不**置本旗标。
    pub fn has_unsupported_grid_template(&self) -> bool {
        self.grid_columns_unsupported.is_some() || self.grid_rows_unsupported.is_some()
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
        let bw = self.resolved_border_width();
        let content_box = matches!(self.box_sizing, BoxSizing::ContentBox);
        let chrome_w = pad.left + pad.right + 2.0 * bw;
        let chrome_h = pad.top + pad.bottom + 2.0 * bw;
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
        BoxSizing, DisplaySpec, FlexDirection, FontSizeContext, GridRepeatAuto, LayoutStyle,
        LengthSpec, LineHeightSpec, OverflowSpec, ParentBox, TEXT_APPROX_ASCENT_EM, VisibilitySpec,
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
    fn icon_y_on_centered_text_shares_the_glyph_box_midline() {
        let font = 12.0;
        let extent = 12.0;
        let y = icon_y_on_text_glyph_center(
            10.0,
            28.0,
            font,
            Some(LineHeightSpec::Absolute(font)),
            true,
            extent,
        );
        let line_top = 10.0 + (28.0 - font) * 0.5;
        let expected = line_top + glyph_box_center_from_line_top(font, font) - extent * 0.5;
        assert!((y - expected).abs() < 1e-5);
        assert!((y - 15.6).abs() < 1e-5);
    }
}
