//! Box units, writing/alignment specs, length resolution, and font metrics.

use serde::{Deserialize, Serialize};

use super::calc::{CalcBinOp, CalcExpr, CalcExprRef, intern_calc};

/// Flex 主轴方向（`row` / `column`；`*-reverse` 见 [`LayoutStyle::flex_reverse`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DirSpec {
    #[default]
    Ltr,
    Rtl,
}

/// Alias kept so stash / CSS `direction` call sites compile.
pub type DirectionSpec = DirSpec;

impl DirSpec {
    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }
}

/// CSS `writing-mode` 子集。`sideways-*` 不进此枚举（解析期跳过，fail-closed）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WritingModeSpec {
    #[default]
    HorizontalTb,
    /// Inline axis is vertical; block-start is the physical right edge.
    VerticalRl,
    /// Inline axis is vertical; block-start is the physical left edge.
    VerticalLr,
}

impl WritingModeSpec {
    pub const fn is_horizontal(self) -> bool {
        matches!(self, Self::HorizontalTb)
    }

    pub const fn is_vertical(self) -> bool {
        matches!(self, Self::VerticalRl | Self::VerticalLr)
    }
}

/// CSS `text-orientation`: how a vertical line sets its glyphs. It does
/// nothing in `horizontal-tb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TextOrientationSpec {
    /// Each character by its Unicode `Vertical_Orientation` (UAX #50): CJK
    /// upright, Latin and digits on their side.
    #[default]
    Mixed,
    /// Every character upright, and the text read as left-to-right: the used
    /// `direction` is `ltr` (CSS Writing Modes §5.1).
    Upright,
    /// Every character on its side, as a horizontal line turned 90°.
    Sideways,
}

/// 交叉轴对齐（`align-items` / `align-self`）。
///
/// `Baseline` 由 Runtime 的 shared intrinsic metrics 提供；没有 shaped
/// metrics 时才使用 host ascent 或明确的 replaced-content fallback。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplaySpec {
    None,
    #[default]
    Flex,
    InlineFlex,
    Grid,
    /// `inline-grid`：与 [`Self::Grid`] 同属网格容器；轨 / auto-fill 布局消费与 `grid` 相同。
    InlineGrid,
    /// `display: inline`：行内级；参与 IFC。
    Inline,
    /// `display: layers`（NanaUI 扩展）：叠层。所有在流子项共用同一块区域，
    /// 后写的盖在先写的上面；子项按 `justify-items` / `align-items`（或自身的
    /// `justify-self` / `align-self`）对齐，容器尺寸随最大的子项。对应 SwiftUI
    /// `ZStack`、Compose `Box`、Flutter `Stack`，也就是 CSS 里把每个子项都放进
    /// 同一格的单格网格。它按网格排版：轨道固定为一行一列 `auto`，放置时这一格
    /// 撑满容器内容盒；子项自己的网格位置不起作用。
    Layers,
    /// `display: free`（NanaUI 扩展）：自由容器。每个生成盒子、未声明
    /// `absolute` / `fixed` 的直接子项按 Runtime 里的显式矩形
    /// （`UiWorld::explicit_rect`）落位：矩形是父级 padding box 内的逻辑像素，
    /// 原点在左上、y 向下。子项之间没有布局依赖，也不撑大容器；容器自己仍是
    /// 文档流里的一个盒子，尺寸只取自己的样式与父级约束。矩形不写进
    /// `left` / `top` / `width` / `height`。白板、节点图、时间轴、地图钉都是这种参与。
    Free,
}

impl DisplaySpec {
    /// `display: grid`、`inline-grid`，以及按单格网格排版的 `layers`。
    pub fn is_grid_container(self) -> bool {
        matches!(self, Self::Grid | Self::InlineGrid | Self::Layers)
    }

    /// `display: layers`：每个在流子项都占同一格。
    pub fn is_layers(self) -> bool {
        matches!(self, Self::Layers)
    }

    /// `display: free`：直接子项按显式矩形落位，彼此不进入兄弟约束。
    pub fn is_free(self) -> bool {
        matches!(self, Self::Free)
    }

    /// `display: flex` 或 `inline-flex`（网格轨在此惰性）。
    pub fn is_flex_container(self) -> bool {
        matches!(self, Self::Flex | Self::InlineFlex)
    }

    /// 行内级（`inline` / `inline-flex` / `inline-grid`）。flex/grid 项会被块化。
    pub fn is_inline_level(self) -> bool {
        matches!(self, Self::Inline | Self::InlineFlex | Self::InlineGrid)
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
    /// Lines other than a paragraph's last fill the line box; boxes in an
    /// inline formatting context pack from the start.
    Justify,
}

impl TextAlignSpec {
    /// Map to a physical main-axis justify keyword for IFC packing.
    pub fn to_justify(self, rtl: bool) -> JustifySpec {
        match self {
            Self::Center => JustifySpec::Center,
            Self::Left => JustifySpec::Start,
            Self::Justify => {
                if rtl {
                    JustifySpec::End
                } else {
                    JustifySpec::Start
                }
            }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhiteSpaceSpec {
    #[default]
    Normal,
    Nowrap,
    /// 保留空格与换行；不在行内折行（仅显式 `\n`）。
    Pre,
    /// 保留空格与换行，并允许行内折行。
    PreWrap,
    /// 保留换行并允许折行；空格折叠是部分行为（不假装完整 CSS）。
    PreLine,
}

impl WhiteSpaceSpec {
    /// Soft wrap (CSS `normal` / `pre-wrap` / `pre-line`).
    pub fn wraps(self) -> bool {
        matches!(self, Self::Normal | Self::PreWrap | Self::PreLine)
    }

    /// Keep explicit line breaks as lines (`pre` / `pre-wrap` / `pre-line`).
    pub fn preserve_newlines(self) -> bool {
        matches!(self, Self::Pre | Self::PreWrap | Self::PreLine)
    }
}

/// CSS `word-break` subset. `keep-all` is not supported and parses as unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WordBreakSpec {
    #[default]
    Normal,
    /// `break-all` → glyph wrap.
    BreakAll,
    /// Legacy `break-word` → word wrap, then glyph if a word cannot fit.
    BreakWord,
}

/// CSS `overflow-wrap` / `word-wrap` subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OverflowWrapSpec {
    #[default]
    Normal,
    /// `anywhere` / `break-word` → word wrap with glyph fallback.
    Anywhere,
}

/// Where a wrapping line may break once wrapping is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub enum TextWrapBreak {
    #[default]
    Word,
    WordOrGlyph,
    Glyph,
}

/// Anchor positioning for a `position: fixed` box, as CSS `position-anchor`
/// with `position-area` and `position-try-fallbacks: flip-block` /
/// `flip-inline` give it: layout hangs the box from `rect` on `placement`'s
/// side with `alignment` along it, `gap` away. It flips to the other side
/// when only that side fits, and keeps the box inside `room` met with the
/// viewport. Its insets are ignored. Coordinates are the viewport's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnchorSpec {
    /// `[x, y, width, height]`; a zero-sized rect is a point (a pointer).
    pub rect: [f32; 4],
    pub placement: crate::PopoverPlacement,
    pub alignment: crate::PopoverAlignment,
    pub gap: f32,
    /// `[x, y, width, height]` the box stays inside, within the viewport;
    /// `None` is the viewport itself.
    pub room: Option<[f32; 4]>,
}

impl AnchorSpec {
    /// A surface hung from the point `(x, y)`.
    pub fn point(x: f32, y: f32, placement: crate::AnchoredMenuPlacement) -> Self {
        let (placement, alignment) = placement.popover();
        Self {
            rect: [x, y, 0.0, 0.0],
            placement,
            alignment,
            gap: 0.0,
            room: None,
        }
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
/// **与 Nana Overlay 分工**：Dialog/Popover/Drawer/ContextMenu 剥离 CSS
/// `fixed`/`sticky`，走 Overlay 合同；普通节点的 `position:fixed` 走本视口子集。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionSpec {
    #[default]
    Static,
    Relative,
    /// 脱流，按 inset 放在包含块里：最近的定位祖先的 padding 盒，没有定位祖先时是
    /// 视口大小的初始包含块（CSS 2.1 §10.1）。
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

    /// 为绝对定位后代建立 containing block：非 static 的定位盒子都算，
    /// sticky 也一样（CSS 2.1 §10.1、CSS Position 3 §2.1）。
    pub fn establishes_containing_block(self) -> bool {
        !matches!(self, Self::Static)
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxSizing {
    #[default]
    BorderBox,
    ContentBox,
}

/// flex-wrap 意图。
///
/// `measure_layout`：`Wrap` / `WrapReverse` 按主轴分行兑现（`WrapReverse` 反转行序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}

/// 视口相对单位轴（`vw` / `vh` / `vmin` / `vmax`）。
#[repr(u8)]
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

/// Which sides of the viewport a length or a box reads: `vw` the width,
/// `vh` the height, `vmin` / `vmax` both. A viewport resize that moves only
/// a side nothing reads leaves the length where it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewportAxes {
    pub width: bool,
    pub height: bool,
}

impl ViewportAxes {
    pub const NONE: Self = Self {
        width: false,
        height: false,
    };
    pub const BOTH: Self = Self {
        width: true,
        height: true,
    };

    pub const fn union(self, other: Self) -> Self {
        Self {
            width: self.width || other.width,
            height: self.height || other.height,
        }
    }

    pub const fn any(self) -> bool {
        self.width || self.height
    }

    pub const fn intersects(self, other: Self) -> bool {
        (self.width && other.width) || (self.height && other.height)
    }
}

impl ViewportAxis {
    /// The sides of the viewport this unit reads.
    pub const fn reads(self) -> ViewportAxes {
        match self {
            Self::Width => ViewportAxes {
                width: true,
                height: false,
            },
            Self::Height => ViewportAxes {
                width: false,
                height: true,
            },
            Self::Min | Self::Max => ViewportAxes::BOTH,
        }
    }

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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontSizeContext {
    pub root_px: f32,
    pub element_px: f32,
}

impl Default for FontSizeContext {
    fn default() -> Self {
        Self {
            root_px: Self::DEFAULT_ROOT_PX,
            element_px: Self::DEFAULT_ROOT_PX,
        }
    }
}

impl FontSizeContext {
    /// CSS initial `medium` (~16px) fallback root font size.
    pub const DEFAULT_ROOT_PX: f32 = 16.0;

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
/// The line box splits extra leading evenly above and below the
/// (ascent+descent) box (CSS half-leading), then places the baseline at
/// `ascent` into that box. Nana approximates
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

/// 可参与 `min`/`max`/`clamp` 的轻量长度原子（Copy；简单 calc 在解析期折进这些变体）。
///
/// Nested `calc()` / `min()` beyond two atoms can use [`LengthSpec::Calc`].
#[derive(Debug, Clone, Copy, PartialEq)]
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
        self.viewport_axes().any()
    }

    /// The sides of the viewport resolution reads.
    pub fn viewport_axes(self) -> ViewportAxes {
        match self {
            Self::Viewport { axis, .. } | Self::CalcViewport { axis, .. } => axis.reads(),
            _ => ViewportAxes::NONE,
        }
    }

    pub fn is_full_percent_fill(self) -> bool {
        match self {
            Self::Percent(percent) if (percent - 100.0).abs() < 0.5 => true,
            Self::CalcPercent { percent, offset_px }
                if (percent - 100.0).abs() < 0.5 && offset_px <= 0.0 =>
            {
                true
            }
            _ => false,
        }
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
///
/// Simple variants stay `Copy`. Full `calc()` trees are
/// [`Self::Calc`] — an interned [`Box<CalcExpr>`] handle — so this enum
/// remains `Copy` and layout can keep copying specs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LengthSpec {
    Px(f32),
    /// 相对父 content box 的百分比（calc 可超出 0..=100，不钳制）。
    Percent(f32),
    /// `Nem`（相对元素 font-size；缺省 16px）。
    Em(f32),
    /// `Nrem`（相对根 font-size；缺省 16px）。
    Rem(f32),
    /// `Nvw` / `Nvh` / `Nvmin` / `Nvmax`。
    Viewport {
        axis: ViewportAxis,
        value: f32,
    },
    /// 轻量 `calc(P% ± Npx)` 当偏移以 `1/128 px` 对齐并落在 `i16` 范围时内联于 8 字节内；
    /// 其余表达式退回 [`Self::Calc`]。
    PercentOffset {
        offset_q7: i16,
        percent: f32,
    },
    /// Full calc AST (`NonZeroU32` interned handle so this enum stays Copy & 8 bytes).
    Calc(CalcExprRef),
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

const _: () = {
    assert!(std::mem::size_of::<LengthSpec>() == 8);
    assert!(std::mem::size_of::<Option<LengthSpec>>() == 8);
};

impl LengthAtom {
    /// Lift this atom back into a [`LengthSpec`].
    pub fn into_spec(self) -> LengthSpec {
        match self {
            Self::Px(v) => LengthSpec::Px(v),
            Self::Percent(p) => LengthSpec::Percent(p),
            Self::Em(v) => LengthSpec::Em(v),
            Self::Rem(v) => LengthSpec::Rem(v),
            Self::Viewport { axis, value } => LengthSpec::Viewport { axis, value },
            Self::CalcPercent { percent, offset_px } => {
                LengthSpec::percent_offset(percent, offset_px)
            }
            Self::CalcViewport {
                axis,
                value,
                offset_px,
            } => LengthSpec::viewport_offset(axis, value, offset_px),
            Self::CalcEm { em, offset_px } => LengthSpec::em_offset(em, offset_px),
            Self::CalcRem { rem, offset_px } => LengthSpec::rem_offset(rem, offset_px),
        }
    }
}

fn quantize_offset_q7(offset_px: f32) -> Option<i16> {
    if !offset_px.is_finite() {
        return None;
    }
    let normalized = if offset_px == 0.0 { 0.0 } else { offset_px };
    let q = normalized * 128.0;
    let rounded = q.round();
    if (q - rounded).abs() >= 1e-3 {
        return None;
    }
    if rounded < i16::MIN as f32 || rounded > i16::MAX as f32 {
        return None;
    }
    Some(rounded as i16)
}

impl LengthSpec {
    fn intern_raw(expr: CalcExpr) -> Self {
        Self::Calc(intern_calc(Box::new(expr)))
    }

    /// Intern `expr` as [`Self::Calc`], or fold it to a simple Copy variant.
    pub fn from_calc(expr: CalcExpr) -> Self {
        if let Some(simple) = expr.simplify_to_length_spec() {
            return simple;
        }
        Self::intern_raw(expr)
    }

    /// Construct `calc(percent% + offset_px)`, packing inline into 8 bytes
    /// when `offset_px` fits 1/128px fixed-point `i16` and interning into
    /// [`Self::Calc`] otherwise.
    pub fn percent_offset(percent: f32, offset_px: f32) -> Self {
        let offset_px = if offset_px == 0.0 { 0.0 } else { offset_px };
        if let Some(offset_q7) = quantize_offset_q7(offset_px) {
            Self::PercentOffset { offset_q7, percent }
        } else {
            Self::intern_raw(CalcExpr::Binary {
                op: CalcBinOp::Add,
                left: Box::new(CalcExpr::Percent(percent)),
                right: Box::new(CalcExpr::Px(offset_px)),
            })
        }
    }

    /// Runtime drag-friendly constructor for `calc(percent% + offset_px)`:
    /// rounds any finite offset in `[-256, 255.9921875] px` to `1/128 px` (`i16`)
    /// so continuous pointer updates never intern into the static calc table.
    pub fn percent_offset_inline(percent: f32, offset_px: f32) -> Self {
        if offset_px.is_finite() {
            let normalized = if offset_px == 0.0 { 0.0 } else { offset_px };
            let rounded = (normalized * 128.0).round();
            if rounded >= i16::MIN as f32 && rounded <= i16::MAX as f32 {
                return Self::PercentOffset {
                    offset_q7: rounded as i16,
                    percent,
                };
            }
        }
        Self::percent_offset(percent, offset_px)
    }

    /// Construct `calc(em + offset_px)` in [`Self::Calc`].
    pub fn em_offset(em: f32, offset_px: f32) -> Self {
        Self::intern_raw(CalcExpr::Binary {
            op: CalcBinOp::Add,
            left: Box::new(CalcExpr::Em(em)),
            right: Box::new(CalcExpr::Px(offset_px)),
        })
    }

    /// Construct `calc(rem + offset_px)` in [`Self::Calc`].
    pub fn rem_offset(rem: f32, offset_px: f32) -> Self {
        Self::intern_raw(CalcExpr::Binary {
            op: CalcBinOp::Add,
            left: Box::new(CalcExpr::Rem(rem)),
            right: Box::new(CalcExpr::Px(offset_px)),
        })
    }

    /// Construct `calc(viewport + offset_px)` in [`Self::Calc`].
    pub fn viewport_offset(axis: ViewportAxis, value: f32, offset_px: f32) -> Self {
        Self::intern_raw(CalcExpr::Binary {
            op: CalcBinOp::Add,
            left: Box::new(CalcExpr::Viewport { axis, value }),
            right: Box::new(CalcExpr::Px(offset_px)),
        })
    }

    /// Construct `min(a, b)` in [`Self::Calc`].
    pub fn min2(a: LengthAtom, b: LengthAtom) -> Self {
        Self::intern_raw(CalcExpr::Min(
            Box::new(a.to_calc_expr()),
            Box::new(b.to_calc_expr()),
        ))
    }

    /// Construct `max(a, b)` in [`Self::Calc`].
    pub fn max2(a: LengthAtom, b: LengthAtom) -> Self {
        Self::intern_raw(CalcExpr::Max(
            Box::new(a.to_calc_expr()),
            Box::new(b.to_calc_expr()),
        ))
    }

    /// Construct `clamp(min, val, max)` in [`Self::Calc`].
    pub fn clamp3(min: LengthAtom, val: LengthAtom, max: LengthAtom) -> Self {
        Self::intern_raw(CalcExpr::Clamp {
            min: Box::new(min.to_calc_expr()),
            val: Box::new(val.to_calc_expr()),
            max: Box::new(max.to_calc_expr()),
        })
    }

    /// Unpack a single [`LengthAtom`] when this spec represents a scalar/offset unit.
    pub fn as_atom(self) -> Option<LengthAtom> {
        match self {
            Self::Px(v) => Some(LengthAtom::Px(v)),
            Self::Percent(p) => Some(LengthAtom::Percent(p)),
            Self::Em(v) => Some(LengthAtom::Em(v)),
            Self::Rem(v) => Some(LengthAtom::Rem(v)),
            Self::Viewport { axis, value } => Some(LengthAtom::Viewport { axis, value }),
            Self::PercentOffset { offset_q7, percent } => Some(LengthAtom::CalcPercent {
                percent,
                offset_px: f32::from(offset_q7) / 128.0,
            }),
            Self::Calc(expr) => expr.inner().as_length_atom(),
            Self::Fill
            | Self::Shrink
            | Self::Auto
            | Self::MinContent
            | Self::MaxContent
            | Self::FitContent => None,
        }
    }

    /// Unpack `calc(percent% + offset_px)` if this spec represents that form.
    pub fn as_percent_offset(self) -> Option<(f32, f32)> {
        match self {
            Self::PercentOffset { offset_q7, percent } => {
                Some((percent, f32::from(offset_q7) / 128.0))
            }
            Self::Calc(expr) => match expr.inner().as_length_atom()? {
                LengthAtom::CalcPercent { percent, offset_px } => Some((percent, offset_px)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Unpack `calc(em + offset_px)` if this spec represents that form.
    pub fn as_em_offset(self) -> Option<(f32, f32)> {
        let Self::Calc(expr) = self else {
            return None;
        };
        match expr.inner().as_length_atom()? {
            LengthAtom::CalcEm { em, offset_px } => Some((em, offset_px)),
            _ => None,
        }
    }

    /// Unpack `calc(rem + offset_px)` if this spec represents that form.
    pub fn as_rem_offset(self) -> Option<(f32, f32)> {
        let Self::Calc(expr) = self else {
            return None;
        };
        match expr.inner().as_length_atom()? {
            LengthAtom::CalcRem { rem, offset_px } => Some((rem, offset_px)),
            _ => None,
        }
    }

    /// Unpack `calc(viewport + offset_px)` if this spec represents that form.
    pub fn as_viewport_offset(self) -> Option<(ViewportAxis, f32, f32)> {
        let Self::Calc(expr) = self else {
            return None;
        };
        match expr.inner().as_length_atom()? {
            LengthAtom::CalcViewport {
                axis,
                value,
                offset_px,
            } => Some((axis, value, offset_px)),
            _ => None,
        }
    }

    /// Unpack `min(a, b)` when both operands fit [`LengthAtom`].
    pub fn as_min2(self) -> Option<(LengthAtom, LengthAtom)> {
        let Self::Calc(expr) = self else {
            return None;
        };
        let CalcExpr::Min(a, b) = expr.inner() else {
            return None;
        };
        Some((a.as_length_atom()?, b.as_length_atom()?))
    }

    /// Unpack `max(a, b)` when both operands fit [`LengthAtom`].
    pub fn as_max2(self) -> Option<(LengthAtom, LengthAtom)> {
        let Self::Calc(expr) = self else {
            return None;
        };
        let CalcExpr::Max(a, b) = expr.inner() else {
            return None;
        };
        Some((a.as_length_atom()?, b.as_length_atom()?))
    }

    /// Unpack `clamp(min, val, max)` when all operands fit [`LengthAtom`].
    pub fn as_clamp3(self) -> Option<(LengthAtom, LengthAtom, LengthAtom)> {
        let Self::Calc(expr) = self else {
            return None;
        };
        let CalcExpr::Clamp { min, val, max } = expr.inner() else {
            return None;
        };
        Some((
            min.as_length_atom()?,
            val.as_length_atom()?,
            max.as_length_atom()?,
        ))
    }

    /// 解析为逻辑像素；无 viewport 时视口单位返回 `None`。
    pub fn resolve_px(self, percent_base: Option<f32>) -> Option<f32> {
        self.resolve_with(percent_base, None)
    }

    /// `true` when resolution reads the viewport, so this length moves on a
    /// viewport change even when its containing block does not.
    pub fn depends_on_viewport(self) -> bool {
        self.viewport_axes().any()
    }

    /// The sides of the viewport resolution reads.
    pub fn viewport_axes(self) -> ViewportAxes {
        match self {
            Self::Viewport { axis, .. } => axis.reads(),
            Self::Calc(expr) => expr.inner().viewport_axes(),
            _ => ViewportAxes::NONE,
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
            Self::PercentOffset { offset_q7, percent } => {
                let offset_px = f32::from(offset_q7) / 128.0;
                percent_base.map(|base| base * percent / 100.0 + offset_px)
            }
            Self::Viewport { axis, value } => {
                viewport.map(|(w, h)| axis.base(w, h) * value / 100.0)
            }
            Self::Calc(expr) => expr
                .inner()
                .resolve_with_fonts(percent_base, viewport, fonts),
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
                | Self::PercentOffset { .. }
                | Self::Viewport { .. }
                | Self::Calc(_)
        )
    }

    /// `100%` / `Fill` / `calc(100% - Npx)` with `N ≥ 0` — treat as indefinite
    /// during grid auto-track measure so the item does not inflate the track.
    pub fn is_full_percent_fill(self) -> bool {
        match self {
            Self::Fill => true,
            Self::Percent(percent) if (percent - 100.0).abs() < 0.5 => true,
            Self::PercentOffset { offset_q7, percent }
                if (percent - 100.0).abs() < 0.5 && offset_q7 <= 0 =>
            {
                true
            }
            Self::Calc(expr) => expr.inner().is_full_percent_fill(),
            _ => false,
        }
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
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ParentBox {
    pub width: Option<f32>,
    pub height: Option<f32>,
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
