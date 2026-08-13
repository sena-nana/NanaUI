//! Style Model **Layout** — box flex intent (not a CSSOM, not workspace regions).
//!
//! Pure data consumed by L3 draw (`nana-ui` / iced) and L1/L2 adapters.
//! CSS declaration / class parsing stays in `nana-ui-vue::css_map` /
//! `nana-ui-vue::shell_contract`.
//! Workspace region layout lives in [`crate::layout`].
//!
//! Shared geometry helpers used by both `measure` (pre-paint / parity) and
//! iced adapters: [`LayoutStyle::resolve_content_box`], [`LayoutStyle::resolve_inset`],
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AlignSpec {
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

/// 主轴分布（justify-content）—— iced 用 Space / Fill 实现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JustifySpec {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
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
        matches!(self, Self::Hidden)
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
    /// `inline-grid`：与 [`Self::Grid`] 同属网格容器；1D 轨子集布局消费与 `grid` 相同。
    InlineGrid,
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
}

/// `grid-template-*` 中已识别但布局仍 defer 的语法（勿当成 `none` / 静默丢弃）。
///
/// 对照 [MDN: repeat()](https://developer.mozilla.org/en-US/docs/Web/CSS/repeat)：
/// `auto-fit` / `auto-fill` 依赖隐式轨折叠，完整 2D grid 未兑现前不得假展开。
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
}

/// `grid-auto-flow`（解析保留；1D 轨子集**不**消费 — 完整 2D / auto-placement defer）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridAutoFlow {
    Row,
    Column,
    RowDense,
    ColumnDense,
}

/// `position` — Style Model 子集。
///
/// - `Static`：忽略 inset
/// - `Relative`：`top`/`left`/`right`/`bottom` 偏移进入 measure（及 iced 近似）
/// - `Absolute`：measure 最小子集（脱流 + 相对 nearest positioned padding box）；
///   iced 流内跳过；产品浮层仍走 Nana Overlay，不实现完整定位引擎
/// - `Fixed`：视口 containing block + inset 子集（脱流；iced 根层绘制）
/// - `Sticky`：仍 defer（缺口）
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
    /// 缺口（defer）：粘性定位未兑现。
    Sticky,
}

impl PositionSpec {
    /// 尚未兑现的定位模式（目前仅 `Sticky`）。
    pub fn is_unsupported_positioning(self) -> bool {
        matches!(self, Self::Sticky)
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
/// - `measure_layout`：`Wrap` / `WrapReverse` 按主轴分行兑现（`WrapReverse` 反转行序）
/// - iced Row：`nana-ui-vue::iced_app` 的 borrowed（`layout_row`）与 owned
///   （`wrap_layout_owned`）路径均做多行拆分与 `WrapReverse` 行序反转
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

/// CSS `line-height` 子集：无单位倍数或绝对 px（`normal` → 不写入，用 iced 默认）。
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
/// Used by iced `letter-spacing` glyph rows (which otherwise measure ~0 tall)
/// and by measure for typography leaves under `height:auto`.
pub fn text_line_box_height_px(font_px: f32, line_height: Option<LineHeightSpec>) -> f32 {
    let px = font_px.max(0.0);
    match line_height {
        Some(spec) => spec.resolve_px(px),
        None => px * 1.2,
    }
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
}

impl LengthSpec {
    /// 解析为逻辑像素；无 viewport 时视口单位返回 `None`。
    pub fn resolve_px(self, percent_base: Option<f32>) -> Option<f32> {
        self.resolve_with(percent_base, None)
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
            Self::Fill | Self::Shrink | Self::Auto => None,
        }
    }

    /// 非负长度（width/height/padding/min-size）；`None` 若无法解析。
    pub fn resolve_non_negative(
        self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> Option<f32> {
        self.resolve_with(percent_base, viewport)
            .map(|v| v.max(0.0))
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
    /// `max-width`：同上。`Fill` 表示无有限上限（iced Fill 标记）。
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
    /// `align-content`（多行 flex 线间分布；自动交叉尺寸时常无无剩余空间）。
    /// 复用 [`JustifySpec`]（含 `space-*`）；`stretch`/`normal` ≈ Start。
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
    pub flex_shrink: Option<f32>,
    pub flex_basis: Option<LengthSpec>,
    pub overflow_x: OverflowSpec,
    pub overflow_y: OverflowSpec,
    /// `text-overflow: ellipsis`（需配合 nowrap / 定宽；iced Text 路径兑现）。
    #[serde(default)]
    pub text_overflow_ellipsis: bool,
    /// `white-space: nowrap`。
    #[serde(default)]
    pub white_space_nowrap: bool,
    /// Computed `font-size` in CSS px. `None` = inherit (then initial / ControlSize).
    #[serde(default)]
    pub font_size: Option<f32>,
    /// CSS `font-weight` as 100..=900. `None` = inherit / normal.
    #[serde(default)]
    pub font_weight: Option<u16>,
    /// Preferred named family from `font-family` (generics stripped). `None` = UI default.
    #[serde(default)]
    pub font_family: Option<String>,
    /// CSS `line-height` subset. `None` = inherit / iced default.
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
    /// `grid-template-columns` 含 defer 语法（如 `repeat(auto-fit)`）；**非** `none`。
    /// 布局仍只读 [`Self::grid_columns`]；本字段供诊断，避免静默丢轨。
    #[serde(default)]
    pub grid_columns_unsupported: Option<GridTrackListUnsupported>,
    /// 同 [`Self::grid_columns_unsupported`]，针对 `grid-template-rows`。
    #[serde(default)]
    pub grid_rows_unsupported: Option<GridTrackListUnsupported>,
    /// `grid-auto-columns`：解析保留；**布局不消费**（隐式列轨 / 2D defer）。
    #[serde(default)]
    pub grid_auto_columns: Option<Vec<GridTrack>>,
    /// `grid-auto-rows`：解析保留；**布局不消费**（隐式行轨 / 2D defer）。
    #[serde(default)]
    pub grid_auto_rows: Option<Vec<GridTrack>>,
    /// `grid-auto-flow`：解析保留；**布局不消费**（auto-placement defer）。
    #[serde(default)]
    pub grid_auto_flow: Option<GridAutoFlow>,
    pub hidden: bool,
    /// CSS `opacity` (0..=1). `None` = unset / inherit (treated as 1.0 at paint).
    /// Parsed with other declarations so L1 adapters need not re-scan the style
    /// string.
    #[serde(default)]
    pub opacity: Option<f32>,
    /// Instance surface paint from L1 style/class (not ThemeTokens).
    /// RGBA 0..=1; applied by iced `container` style.
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
            hidden: false,
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
            || self.border_radius.unwrap_or(0.0) > 0.0
            || self.border_width.unwrap_or(0.0) > 0.0
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
        self.row_gap
            .or(self.gap)
            .and_then(|s| s.resolve_px(percent_base))
            .unwrap_or(0.0)
            .max(0.0)
    }

    /// CSS `column-gap`（回退到 uniform `gap`）。无 `%` 基时仅兑现 `Px`。
    pub fn resolved_column_gap(&self) -> f32 {
        self.resolved_column_gap_against(None)
    }

    /// `column-gap` / uniform `gap`，`%` 相对包含块宽度。
    pub fn resolved_column_gap_against(&self, percent_base: Option<f32>) -> f32 {
        self.column_gap
            .or(self.gap)
            .and_then(|s| s.resolve_px(percent_base))
            .unwrap_or(0.0)
            .max(0.0)
    }

    /// Flex 主轴 gap：Row→column-gap；Column→row-gap。无 `%` 基时仅兑现 `Px`。
    pub fn main_gap(&self, direction: FlexDirection) -> f32 {
        self.main_gap_against(direction, ParentBox::default())
    }

    /// 主轴 gap，携带 CB 供 `%` 解析（column-gap→宽；row-gap→高，缺省回退宽）。
    pub fn main_gap_against(&self, direction: FlexDirection, cb: ParentBox) -> f32 {
        match direction {
            FlexDirection::Row => self.resolved_column_gap_against(cb.width),
            FlexDirection::Column => {
                self.resolved_row_gap_against(definite_length(cb.height).or(cb.width))
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
        match direction {
            FlexDirection::Row => {
                self.resolved_row_gap_against(definite_length(cb.height).or(cb.width))
            }
            FlexDirection::Column => self.resolved_column_gap_against(cb.width),
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
    pub fn resolved_padding_against(&self, percent_base: Option<f32>) -> PaddingSpec {
        resolve_box_edge_specs(
            self.padding,
            self.padding_top,
            self.padding_right,
            self.padding_bottom,
            self.padding_left,
            percent_base,
        )
    }

    /// Resolve margin to px without a containing-block width (`%` → 0).
    pub fn resolved_margin(&self) -> PaddingSpec {
        self.resolved_margin_against(None)
    }

    /// Resolve margin; `%` / calc 相对包含块宽度 `percent_base`（含上下边）。
    /// CSS 允许负 margin；不钳制到 0。
    pub fn resolved_margin_against(&self, percent_base: Option<f32>) -> PaddingSpec {
        resolve_box_edge_specs_signed(
            self.margin,
            self.margin_top,
            self.margin_right,
            self.margin_bottom,
            self.margin_left,
            percent_base,
        )
    }

    /// `min-width` → 非负 px；无法解析时 0。
    pub fn resolved_min_width(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> f32 {
        resolve_min_size(self.min_width, percent_base, viewport)
    }

    /// `max-width` → 有限非负 px；`Fill`/`Auto`/`Shrink` 或无法解析 → `None`（不钳制）。
    pub fn resolved_max_width(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> Option<f32> {
        resolve_max_size(self.max_width, percent_base, viewport)
    }

    /// `min-height` → 非负 px；无法解析时 0。
    pub fn resolved_min_height(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> f32 {
        resolve_min_size(self.min_height, percent_base, viewport)
    }

    /// `max-height` → 有限非负 px；无有限上限 → `None`。
    pub fn resolved_max_height(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> Option<f32> {
        resolve_max_size(self.max_height, percent_base, viewport)
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
        if !self.position.applies_relative_offset() {
            return (0.0, 0.0);
        }
        let dx = self
            .offset_left
            .and_then(|l| l.resolve_px(base_w))
            .or_else(|| {
                self.offset_right
                    .and_then(|r| r.resolve_px(base_w))
                    .map(|v| -v)
            })
            .unwrap_or(0.0);
        let dy = self
            .offset_top
            .and_then(|t| t.resolve_px(base_h))
            .or_else(|| {
                self.offset_bottom
                    .and_then(|b| b.resolve_px(base_h))
                    .map(|v| -v)
            })
            .unwrap_or(0.0);
        (dx, dy)
    }

    /// Resolve a single inset against a containing-block edge length.
    pub fn resolve_inset(spec: Option<LengthSpec>, base: f32) -> Option<f32> {
        spec.and_then(|s| s.resolve_px(Some(base)))
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

    /// Column tracks that participate in layout.
    ///
    /// CSS: `grid-template-columns` is inert on flex containers. Tracks remain
    /// active when `display` is `grid` / `inline-grid` / unset (Nana 1D-grid
    /// subset compat) or other non-flex values.
    ///
    /// **不**消费 [`Self::grid_auto_columns`]（隐式轨 defer）。若
    /// [`Self::grid_columns_unsupported`] 有值且 `grid_columns` 为空，表示作者
    /// 写了 defer 语法而非未声明。
    pub fn active_grid_columns(&self) -> Option<&[GridTrack]> {
        if self.display.is_some_and(DisplaySpec::is_flex_container) {
            return None;
        }
        self.grid_columns.as_deref().filter(|t| !t.is_empty())
    }

    /// Row tracks that participate in layout (see [`Self::active_grid_columns`]).
    ///
    /// **不**消费 [`Self::grid_auto_rows`]。
    pub fn active_grid_rows(&self) -> Option<&[GridTrack]> {
        if self.display.is_some_and(DisplaySpec::is_flex_container) {
            return None;
        }
        self.grid_rows.as_deref().filter(|t| !t.is_empty())
    }

    /// `grid-auto-*` 已解析但布局仍 defer（完整 2D / auto-placement）。
    pub fn has_deferred_grid_auto(&self) -> bool {
        self.grid_auto_columns
            .as_ref()
            .is_some_and(|t| !t.is_empty())
            || self.grid_auto_rows.as_ref().is_some_and(|t| !t.is_empty())
            || self.grid_auto_flow.is_some()
    }

    /// `grid-template-columns` / `rows` 含明确 Unsupported（如 `repeat(auto-fit)`）。
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

fn resolve_edge_length(spec: Option<LengthSpec>, percent_base: Option<f32>) -> Option<f32> {
    spec.and_then(|s| match s {
        LengthSpec::Fill | LengthSpec::Shrink | LengthSpec::Auto => None,
        other => other.resolve_non_negative(percent_base, None),
    })
}

/// Margin 边长：允许负值（CSS Values / Box Model）。
fn resolve_edge_length_signed(spec: Option<LengthSpec>, percent_base: Option<f32>) -> Option<f32> {
    spec.and_then(|s| match s {
        LengthSpec::Fill | LengthSpec::Shrink | LengthSpec::Auto => None,
        other => other.resolve_with(percent_base, None),
    })
}

fn resolve_min_size(
    spec: Option<LengthSpec>,
    percent_base: Option<f32>,
    viewport: Option<(f32, f32)>,
) -> f32 {
    match spec {
        None | Some(LengthSpec::Auto) | Some(LengthSpec::Shrink) | Some(LengthSpec::Fill) => 0.0,
        Some(other) => other
            .resolve_non_negative(percent_base, viewport)
            .unwrap_or(0.0),
    }
}

fn resolve_max_size(
    spec: Option<LengthSpec>,
    percent_base: Option<f32>,
    viewport: Option<(f32, f32)>,
) -> Option<f32> {
    match spec? {
        LengthSpec::Fill | LengthSpec::Auto | LengthSpec::Shrink => None,
        other => other
            .resolve_non_negative(percent_base, viewport)
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
) -> PaddingSpec {
    let u = resolve_edge_length(uniform, percent_base).unwrap_or(0.0);
    PaddingSpec {
        top: resolve_edge_length(top, percent_base).unwrap_or(u).max(0.0),
        right: resolve_edge_length(right, percent_base)
            .unwrap_or(u)
            .max(0.0),
        bottom: resolve_edge_length(bottom, percent_base)
            .unwrap_or(u)
            .max(0.0),
        left: resolve_edge_length(left, percent_base)
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
) -> PaddingSpec {
    let u = resolve_edge_length_signed(uniform, percent_base).unwrap_or(0.0);
    PaddingSpec {
        top: resolve_edge_length_signed(top, percent_base).unwrap_or(u),
        right: resolve_edge_length_signed(right, percent_base).unwrap_or(u),
        bottom: resolve_edge_length_signed(bottom, percent_base).unwrap_or(u),
        left: resolve_edge_length_signed(left, percent_base).unwrap_or(u),
    }
}

#[cfg(test)]
mod tests {
    use super::{BoxSizing, FlexDirection, LayoutStyle, LengthSpec, ParentBox};

    #[test]
    fn child_main_length_grows_to_fill() {
        let mut layout = LayoutStyle::default();
        layout.flex_grow = Some(1.0);
        assert_eq!(
            layout.child_main_length(FlexDirection::Row),
            Some(LengthSpec::Fill)
        );
    }

    #[test]
    fn resolve_content_box_height_chain() {
        let mut shell = LayoutStyle::default();
        shell.height = Some(LengthSpec::Fill);
        let parent = ParentBox::from_viewport(800.0, 600.0);
        let box_ = shell.resolve_content_box(parent);
        assert_eq!(box_.height, Some(600.0));
    }

    #[test]
    fn resolve_content_box_keeps_declared_width_under_content_box() {
        let mut layout = LayoutStyle::default();
        layout.width = Some(LengthSpec::Px(100.0));
        layout.padding = Some(LengthSpec::Px(10.0));
        layout.box_sizing = BoxSizing::ContentBox;
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
        let mut layout = LayoutStyle::default();
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        layout.padding = Some(LengthSpec::Px(10.0));
        layout.border_width = Some(5.0);
        layout.box_sizing = BoxSizing::ContentBox;
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
        let mut layout = LayoutStyle::default();
        layout.flex_grow = Some(1.0);
        layout.width = None;
        layout.height = None;
        layout.padding = Some(LengthSpec::Px(8.0));
        layout.border_width = Some(2.0);
        layout.box_sizing = BoxSizing::ContentBox;
        let parent = ParentBox::from_viewport(300.0, 150.0);
        let box_ = layout.resolve_content_box(parent);
        assert_eq!(box_.width, Some(280.0), "300 − 16pad − 4border");
        assert_eq!(box_.height, Some(130.0), "150 − 16pad − 4border");
    }

    #[test]
    fn resolve_content_box_subtracts_padding_and_border_under_border_box() {
        let mut layout = LayoutStyle::default();
        layout.width = Some(LengthSpec::Px(100.0));
        layout.padding = Some(LengthSpec::Px(10.0));
        layout.border_width = Some(5.0);
        layout.box_sizing = BoxSizing::BorderBox;
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
        let mut layout = LayoutStyle::default();
        layout.min_height = Some(LengthSpec::Px(0.0));
        let parent = ParentBox::from_viewport(800.0, 600.0);
        let box_ = layout.resolve_content_box(parent);
        assert_eq!(
            box_.height, None,
            "min-height:0 only allows shrink; Fill comes from height/flex-grow"
        );
    }

    #[test]
    fn min_height_zero_with_fill_height_still_chains() {
        let mut layout = LayoutStyle::default();
        layout.height = Some(LengthSpec::Fill);
        layout.min_height = Some(LengthSpec::Px(0.0)); // sentinel from CSS min-height:100%
        let parent = ParentBox::from_viewport(800.0, 600.0);
        let box_ = layout.resolve_content_box(parent);
        assert_eq!(box_.height, Some(600.0));
    }

    #[test]
    fn row_cross_gap_percent_falls_back_when_height_indefinite() {
        let mut layout = LayoutStyle::default();
        layout.gap = Some(LengthSpec::Percent(10.0));
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
        let mut layout = LayoutStyle::default();
        layout.padding = Some(LengthSpec::Percent(10.0));
        layout.margin_top = Some(LengthSpec::Percent(5.0));
        layout.margin_left = Some(LengthSpec::Px(8.0));
        let pad = layout.resolved_padding_against(Some(200.0));
        let margin = layout.resolved_margin_against(Some(200.0));
        assert_eq!(pad.top, 20.0);
        assert_eq!(pad.left, 20.0);
        assert_eq!(margin.top, 10.0);
        assert_eq!(margin.left, 8.0);
        assert!(layout.resolved_padding().is_zero());
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
}
