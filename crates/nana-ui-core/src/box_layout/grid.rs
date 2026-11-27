//! CSS Grid track, placement, area, and subgrid style contracts.

use crate::sort::CompactSort as _;

use super::units::LengthSpec;

/// Track-list syntax the grid resolver will not pretend to honor.
///
/// Successful `repeat(auto-fit|auto-fill)` is stored on [`GridRepeatAuto`] and
/// expanded at layout; those variants remain as the repeat *kind*.
/// Whole-value `subgrid` sets [`GridStyle::columns_subgrid`] /
/// [`GridStyle::rows_subgrid`] and inherits parent tracks at layout;
/// [`Self::Subgrid`] tags `subgrid` written as a track-list token (e.g.
/// `subgrid 80px`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridTrackListUnsupported {
    /// `repeat(auto-fit, …)` (repeat kind; layout expands it).
    RepeatAutoFit,
    /// `repeat(auto-fill, …)` (repeat kind; layout expands it).
    RepeatAutoFill,
    /// `repeat()` inside another `repeat()`. CSS Grid L1 gives `<track-repeat>`
    /// a `<track-size>` body and `<auto-repeat>` a `<fixed-size>` body, so
    /// neither nests; browsers drop the whole declaration.
    NestedRepeat,
    /// `subgrid` as a track-list token (e.g. `subgrid 80px`) — not a whole value.
    Subgrid,
}

impl GridTrackListUnsupported {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RepeatAutoFit => "repeat(auto-fit)",
            Self::RepeatAutoFill => "repeat(auto-fill)",
            Self::NestedRepeat => "nested repeat()",
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
#[derive(Debug, Clone, PartialEq)]
pub struct GridRepeatAuto {
    pub kind: GridTrackListUnsupported,
    pub tracks: Vec<GridTrack>,
    pub prefix: Vec<GridTrack>,
    pub suffix: Vec<GridTrack>,
    /// Prefix track list line names (`prefix.len() + 1` 行，可为空)。
    pub prefix_line_names: Vec<Vec<String>>,
    /// In-repeat pattern line names (`tracks.len() + 1` 行)；布局按次数展开。
    pub pattern_line_names: Vec<Vec<String>>,
    /// Suffix track list line names.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GridPlacement {
    pub column_start: GridLine,
    pub column_end: GridLine,
    pub row_start: GridLine,
    pub row_end: GridLine,
    /// `grid-area: header` 命名区域（优先于线号）。
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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

/// 网格轨道（轻量；由 CSS 解析填入）。
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// `fit-content(<length>)`: sized like `auto` from its items' content,
    /// then clamped to `limit_px` (CSS Grid §7.2.4). Not flexible, and not
    /// stretched by `align-content` / `justify-content: normal`.
    FitContent {
        limit_px: f32,
    },
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

    /// `auto` or `fit-content()`: sized from its items' content contributions.
    pub fn is_content_sized(self) -> bool {
        matches!(self, Self::Auto | Self::FitContent { .. })
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
            Self::Auto | Self::FitContent { .. } | Self::Px(_) | Self::Percent(_) => None,
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
            Self::Auto | Self::FitContent { .. } => LengthSpec::Shrink,
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
        } else if track.is_content_sized() {
            let mut px = auto_sizes.get(i).copied().unwrap_or(0.0).max(0.0);
            if let GridTrack::FitContent { limit_px } = track {
                px = px.min(limit_px.max(0.0));
            }
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

        freeze.compact_sort_by_key(|(idx, _)| *idx);
        for (idx, frozen_w) in freeze.into_iter().rev() {
            let (i, _, _, _) = active.remove(idx);
            widths[i] = frozen_w;
            free = (free - frozen_w).max(0.0);
        }
    }

    widths
}

/// Cold CSS Grid container and track properties shared out of [`LayoutStyle`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridStyle {
    /// `grid-template-columns` 轻量轨道（侧栏|主区）。
    pub columns: Option<Vec<GridTrack>>,
    /// `grid-template-rows` 轻量轨道（堆叠区；Column 主轴）。
    pub rows: Option<Vec<GridTrack>>,
    /// `grid-template-columns` 无法兑现的语法（嵌套 auto-fit / auto-fill 等）。
    /// 成功的 `repeat(auto-fit|auto-fill)` **不**走这里，见 [`Self::columns_repeat`]。
    pub columns_unsupported: Option<GridTrackListUnsupported>,
    /// 同 [`Self::columns_unsupported`]，针对 `grid-template-rows`。
    pub rows_unsupported: Option<GridTrackListUnsupported>,
    /// `grid-auto-columns`：隐式列轨（2D 自动放置超出模板时追加）。
    pub auto_columns: Option<Vec<GridTrack>>,
    /// `grid-auto-rows`：隐式行轨（2D 自动放置超出模板时追加）。
    pub auto_rows: Option<Vec<GridTrack>>,
    /// `grid-auto-flow`：2D 自动放置方向（缺省 row）。
    pub auto_flow: Option<GridAutoFlow>,
    /// `repeat(auto-fit|auto-fill, …)` 列模式；布局按容器展开。
    pub columns_repeat: Option<crate::shared::Shared<GridRepeatAuto>>,
    /// 同行模式，针对 `grid-template-rows`。
    pub rows_repeat: Option<crate::shared::Shared<GridRepeatAuto>>,
    /// `grid-template-columns: subgrid`：布局时继承父网格所跨列轨几何。
    pub columns_subgrid: bool,
    /// `grid-template-rows: subgrid`：继承父网格所跨行轨几何。
    pub rows_subgrid: bool,
    /// `grid-column` / `grid-row` 项放置。
    pub placement: crate::shared::Shared<GridPlacement>,
    /// `grid-template-areas` 命名区域。
    pub template_areas: Option<GridTemplateAreas>,
    /// 列线名称（线 i 的名字列表；长度 = 轨数 + 1）。
    pub column_line_names: Option<Vec<Vec<String>>>,
    /// 行线名称。
    pub row_line_names: Option<Vec<Vec<String>>>,
}

pub(super) fn lookup_named_line_after(
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

pub(super) fn lookup_named_line_nth(
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
