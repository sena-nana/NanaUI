//! L1 adapter: CSS 子集 → Nana **Style Model** 的 Layout 切片（非 CSS 引擎）。
//!
//! **Adapter internals.** Application hosts should use [`crate::prelude`].
//!
//! Inline `style` / 布局相关 prop 解析为 [`LayoutStyle`]（**中立**声明路径）。
//! Stylesheet 规则经 [`crate::css_cascade`] 匹配后写入同一 [`LayoutStyle`]。
//! `nana-*` / 工具 class 合同在 [`crate::shell_contract`]；本模块经
//! [`LayoutStyleCss::apply_class_layout_hints`] 薄委托，不在此扩展 class 特判。
//!
//! ## 映射归属（Style Model）
//!
//! | CSS / class 意图 | 进入 Style Model | 禁止 |
//! |------------------|------------------|------|
//! | flex / gap / padding / 尺寸 | **Layout**（[`LayoutStyle`]） | 把盒模型塞进 ThemeTokens |
//! | `opacity` | **Layout**（可选；供诊断投影，产品 paint 可忽略） | 在 `style` 二次扫串 |
//! | `font-size` / `font-family` / `font-weight` / `line-height` / `letter-spacing` / `color` | **Layout**（排版子集 → Scene 文本） | 业务 class 特判字号 |
//! | 已知控件 class（`nana-btn--primary` 等） | **Semantics**（经 `widget_map`） | 用任意 paint CSS 当 token 工厂 |
//! | 主题色 / 间距 / 圆角档位 | **Tokens**（`ThemeMetrics` / 语义色） | 业务 `#rrggbb` 发明正式 token |
//!
//! 纯数据 [`LayoutStyle`] / [`LengthSpec`] / [`ParentBox`] 住在 `nana-ui-core::box_layout`。
//! **本模块只做 CSS 子集解析**；禁止把解析器放进 `nana-ui` / `nana-ui-core`。
//!
//! ## 盒模型约定
//! Scene 长度 + 外包 padding（含 border chrome）默认 **border-box**。
//! `content-box`：声明宽为内容，border box = 声明 + padding + border（T-B08/T-B09）。
//!
//! ## 定位
//! `position: static` 忽略 inset。`relative` / `absolute` / `fixed` inset 存为
//! [`LengthSpec`]（px 或 `%`，measure 相对 CB 解析）。`absolute`：脱流 + nearest
//! positioned；流内跳过，产品浮层走 Nana Overlay。`fixed`：脱流 + **视口**
//! CB，根层绘制（非 Overlay 特判）。`sticky`：流内粘性定位（投影在布局侧）。
//!
//! **Overlay 分工**：L2 Dialog/Popover/Drawer/ContextMenu 剥离 companion CSS 的
//! `fixed`/`sticky`；匿名 Vue/CSS 的 `position:fixed` 走视口子集。
//!
//! ## display / grid
//! `display: contents` → [`DisplaySpec::Contents`]（不生成盒，亦非 `omits_box`）。
//! `grid-column` / `grid-row` / `grid-area` 写入 [`GridPlacement`]。
//! `grid-auto-*` 与整表 / 混写 `repeat(auto-fit|auto-fill, <track-list>)` 存入
//! Style Model（`grid_*_repeat`），由布局展开。`subgrid`、嵌套 auto-fit / auto-fill
//! 或无法展开的语法才置 [`GridTrackListUnsupported`]（不假装继承父轨）。
//!
//! ## margin / padding / gap
//! 边长与 gap 存 [`LengthSpec`]（px / `%` / 轻量 calc）。margin/padding `%`
//! （含上下边）相对包含块**宽度**；`column-gap` `%` 相对宽度、`row-gap` `%`
//! 相对高度（缺省回退宽度）。均在 measure / Scene 布局时解析；解析期无 CB
//! 时不得静默丢弃 `%`。
//!
//! ## 逻辑盒属性（CSS Logical Properties）
//! `padding|margin|inset-{inline|block}[-start|-end]` 在
//! `writing-mode: horizontal-tb` 下映射到 physical 字段。
//! 逻辑 inline 规格保留到 used-value：最终 `direction` 才映到
//! `padding_left` / `padding_right` 等（跨层 `direction: rtl` 会 remap）。
//! HTML `dir="rtl"|"ltr"` 是同一 used 值的 presentational hint；`dir="auto"`
//! fail-closed（不做 first-strong bidi，不假装 `ltr`）。
//! `direction: rtl` 把 **inline** 的 start/end 对调到 right/left
//!（`padding-inline-start` → `padding-right`）。block 轴仍是 top/bottom。
//! `text-align: start | end` 随 `direction`；`left` / `right` 保持物理边。
//! **不**翻转 flex/grid 主轴 / 交叉轴起点或 item 序（不是完整 rtl 映射）。
//! `writing-mode` 竖排 / `unicode-bidi` 隔离 / 完整 IFC 双向 **fail-closed**
//!（勿假翻轴，勿假装 bidi isolation）。
//!
//! Layout length / padding / alignment live on `LayoutStyle`; Scene host consumes them
//!（feature `scene-view`）。

pub use nana_ui_core::box_layout::{
    AlignSpec, BorderStyle, BoxShadowSpec, BoxSizing, ClearSpec, DirSpec, DisplaySpec,
    FlexDirection, FlexWrap, FloatSpec, FontSizeContext, GridAutoFlow, GridLine, GridPlacement,
    GridRepeatAuto, GridTemplateAreas, GridTrack, GridTrackListUnsupported, JustifySpec,
    LayoutStyle, LengthAtom, LengthSpec, LineHeightSpec, LogicalInlineEdges, OverflowSpec,
    PaddingSpec, PaintTransform, ParentBox, PositionSpec, TextAlignSpec, TextShadowSpec,
    ViewportAxis, VisibilitySpec, WhiteSpaceSpec, resolve_grid_column_widths,
    resolve_grid_track_sizes,
};

/// CSS keyword / length parsing for Style Model layout enums (L1 only).
pub trait CssLayoutParse: Sized {
    fn parse(raw: &str) -> Option<Self>;
}

impl CssLayoutParse for AlignSpec {
    fn parse(raw: &str) -> Option<Self> {
        Some(match raw.trim().to_ascii_lowercase().as_str() {
            "flex-start" | "start" | "left" | "top" => Self::Start,
            "baseline" | "first baseline" | "last baseline" => Self::Baseline,
            "center" => Self::Center,
            "flex-end" | "end" | "right" | "bottom" => Self::End,
            "stretch" | "normal" => Self::Stretch,
            _ => return None,
        })
    }
}

impl CssLayoutParse for JustifySpec {
    fn parse(raw: &str) -> Option<Self> {
        Some(match raw.trim().to_ascii_lowercase().as_str() {
            "flex-start" | "start" | "left" | "top" => Self::Start,
            "center" => Self::Center,
            "flex-end" | "end" | "right" | "bottom" => Self::End,
            "space-between" => Self::SpaceBetween,
            "space-around" => Self::SpaceAround,
            "space-evenly" => Self::SpaceEvenly,
            "stretch" | "normal" => Self::Stretch,
            _ => return None,
        })
    }
}

impl CssLayoutParse for OverflowSpec {
    fn parse(raw: &str) -> Option<Self> {
        Some(match raw.trim().to_ascii_lowercase().as_str() {
            "visible" => Self::Visible,
            "hidden" | "clip" => Self::Hidden,
            "auto" => Self::Auto,
            "scroll" | "overlay" => Self::Scroll,
            _ => return None,
        })
    }
}

impl CssLayoutParse for PositionSpec {
    fn parse(raw: &str) -> Option<Self> {
        Some(match raw.trim().to_ascii_lowercase().as_str() {
            "static" => Self::Static,
            "relative" => Self::Relative,
            "absolute" => Self::Absolute,
            "fixed" => Self::Fixed,
            "sticky" => Self::Sticky,
            _ => return None,
        })
    }
}

impl CssLayoutParse for LengthSpec {
    fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("none") {
            return None;
        }
        if s.eq_ignore_ascii_case("auto") {
            return Some(Self::Auto);
        }
        if s.eq_ignore_ascii_case("100%") || s.eq_ignore_ascii_case("fill") {
            return Some(Self::Fill);
        }
        if s.eq_ignore_ascii_case("max-content") {
            return Some(Self::MaxContent);
        }
        if s.eq_ignore_ascii_case("min-content") {
            return Some(Self::MinContent);
        }
        if s.eq_ignore_ascii_case("fit-content") {
            return Some(Self::FitContent);
        }
        // min() / max() / clamp() before bare calc / units.
        if let Some(spec) = parse_css_min_max_clamp(s) {
            return Some(spec);
        }
        if let Some(calc) = parse_calc_percent_offset(s) {
            return Some(calc);
        }
        // Bare `100vh - 32px` (common inside min() args; also accept top-level).
        if let Some(spec) = parse_viewport_px_sum(s) {
            return Some(spec);
        }
        parse_length_term_to_spec(s)
    }
}

/// Nesting cap for `calc()` / `min()` / `max()` / `clamp()` / parentheses.
/// Counts CSS grouping (one per `(` or math function), not unary `+/-`.
/// Unary sign runs are capped separately at the same limit.
const MAX_CALC_DEPTH: u8 = 16;
const UNIT_PX: u16 = 1 << 0;
const UNIT_PERCENT: u16 = 1 << 1;
const UNIT_EM: u16 = 1 << 2;
const UNIT_REM: u16 = 1 << 3;
const UNIT_VW: u16 = 1 << 4;
const UNIT_VH: u16 = 1 << 5;
const UNIT_VMIN: u16 = 1 << 6;
const UNIT_VMAX: u16 = 1 << 7;

/// 轻量 calc（非 AST）：线性组合折进既有 [`LengthSpec`]。
/// `+` `-` `*` `/`、括号、嵌套 `calc`，以及可折成单一维度的 `min`/`max`/`clamp`。
fn parse_calc_percent_offset(raw: &str) -> Option<LengthSpec> {
    let s = raw.trim();
    if !starts_with_ci(s, "calc(") {
        return None;
    }
    parse_calc_expr_to_spec_at(s, 0)
}

/// Parse a calc math expression (optional `calc()` wrapper) into LengthSpec.
fn parse_calc_expr_to_spec_at(raw: &str, depth: u8) -> Option<LengthSpec> {
    let s = raw.trim();
    // `calc(min/max/clamp(...))` may be mixed-unit Min2/Max2/Clamp3; CalcSum
    // cannot hold that, so keep the already-folded LengthSpec.
    if let Some(inner) = strip_calc_wrapper(s) {
        if let Some(spec) = parse_css_min_max_clamp_at(inner, depth.saturating_add(1)) {
            return Some(spec);
        }
    }
    parse_calc_expr_to_sum_at(s, depth)?.to_length_spec()
}

fn strip_calc_wrapper(s: &str) -> Option<&str> {
    if !starts_with_ci(s, "calc(") {
        return None;
    }
    let (inner, rest) = split_paren_inner(&s[5..])?;
    rest.trim().is_empty().then_some(inner)
}

fn parse_calc_expr_to_sum_at(raw: &str, depth: u8) -> Option<CalcSum> {
    if depth >= MAX_CALC_DEPTH {
        return None;
    }
    let mut parser = CalcParser {
        s: raw.trim(),
        i: 0,
        depth,
    };
    let sum = parser.parse_add()?;
    parser.skip_ws();
    if parser.i != parser.s.len() {
        return None;
    }
    Some(sum)
}

/// Linear combination of CSS length units + a unitless number.
/// Folded into existing LengthSpec variants; not a second layout engine.
/// `units` keeps zero-valued dimensions (`0px` is still a length).
#[derive(Clone, Copy, Debug, Default)]
struct CalcSum {
    number: f32,
    px: f32,
    percent: f32,
    em: f32,
    rem: f32,
    vw: f32,
    vh: f32,
    vmin: f32,
    vmax: f32,
    units: u16,
}

impl CalcSum {
    fn number(v: f32) -> Self {
        Self {
            number: v,
            ..Self::default()
        }
    }
    fn px(v: f32) -> Self {
        Self {
            px: v,
            units: UNIT_PX,
            ..Self::default()
        }
    }
    fn percent(v: f32) -> Self {
        Self {
            percent: v,
            units: UNIT_PERCENT,
            ..Self::default()
        }
    }
    fn em(v: f32) -> Self {
        Self {
            em: v,
            units: UNIT_EM,
            ..Self::default()
        }
    }
    fn rem(v: f32) -> Self {
        Self {
            rem: v,
            units: UNIT_REM,
            ..Self::default()
        }
    }
    fn viewport(axis: ViewportAxis, value: f32) -> Self {
        let mut s = Self::default();
        match axis {
            ViewportAxis::Width => {
                s.vw = value;
                s.units = UNIT_VW;
            }
            ViewportAxis::Height => {
                s.vh = value;
                s.units = UNIT_VH;
            }
            ViewportAxis::Min => {
                s.vmin = value;
                s.units = UNIT_VMIN;
            }
            ViewportAxis::Max => {
                s.vmax = value;
                s.units = UNIT_VMAX;
            }
        }
        s
    }

    fn has_length(self) -> bool {
        self.units != 0
    }

    fn is_pure_number(self) -> bool {
        self.units == 0
    }

    fn is_finite(self) -> bool {
        self.number.is_finite()
            && self.px.is_finite()
            && self.percent.is_finite()
            && self.em.is_finite()
            && self.rem.is_finite()
            && self.vw.is_finite()
            && self.vh.is_finite()
            && self.vmin.is_finite()
            && self.vmax.is_finite()
    }

    fn finish(self) -> Option<Self> {
        self.is_finite().then_some(self)
    }

    fn scale(self, k: f32) -> Option<Self> {
        if !k.is_finite() {
            return None;
        }
        Self {
            number: self.number * k,
            px: self.px * k,
            percent: self.percent * k,
            em: self.em * k,
            rem: self.rem * k,
            vw: self.vw * k,
            vh: self.vh * k,
            vmin: self.vmin * k,
            vmax: self.vmax * k,
            units: self.units,
        }
        .finish()
    }

    fn neg(self) -> Option<Self> {
        self.scale(-1.0)
    }

    fn add(self, rhs: Self) -> Option<Self> {
        match (self.is_pure_number(), rhs.is_pure_number()) {
            (true, true) => Self::number(self.number + rhs.number).finish(),
            (false, false) => {
                if self.number != 0.0 || rhs.number != 0.0 {
                    return None;
                }
                Self {
                    px: self.px + rhs.px,
                    percent: self.percent + rhs.percent,
                    em: self.em + rhs.em,
                    rem: self.rem + rhs.rem,
                    vw: self.vw + rhs.vw,
                    vh: self.vh + rhs.vh,
                    vmin: self.vmin + rhs.vmin,
                    vmax: self.vmax + rhs.vmax,
                    number: 0.0,
                    units: self.units | rhs.units,
                }
                .finish()
            }
            // CSS: unitless 0 may add to a length; any other number may not.
            (true, false) => (self.number == 0.0).then_some(rhs)?.finish(),
            (false, true) => (rhs.number == 0.0).then_some(self)?.finish(),
        }
    }

    fn mul(self, rhs: Self) -> Option<Self> {
        match (self.is_pure_number(), rhs.is_pure_number()) {
            (true, true) => Self::number(self.number * rhs.number).finish(),
            (true, false) => rhs.scale(self.number),
            (false, true) => self.scale(rhs.number),
            (false, false) => None,
        }
    }

    fn div(self, rhs: Self) -> Option<Self> {
        if !rhs.is_pure_number() || rhs.number == 0.0 || !rhs.number.is_finite() {
            return None;
        }
        self.scale(1.0 / rhs.number)
    }

    fn to_length_spec(self) -> Option<LengthSpec> {
        if !self.is_finite() {
            return None;
        }
        // Unitless calc (`calc(2 * 3)`) is not a length.
        if !self.has_length() {
            return None;
        }
        if self.number != 0.0 {
            return None;
        }
        match self.units {
            UNIT_PX => Some(LengthSpec::Px(self.px)),
            UNIT_PERCENT => percent_to_spec(self.percent),
            UNIT_EM => Some(LengthSpec::Em(self.em)),
            UNIT_REM => Some(LengthSpec::Rem(self.rem)),
            UNIT_VW => Some(LengthSpec::Viewport {
                axis: ViewportAxis::Width,
                value: self.vw,
            }),
            UNIT_VH => Some(LengthSpec::Viewport {
                axis: ViewportAxis::Height,
                value: self.vh,
            }),
            UNIT_VMIN => Some(LengthSpec::Viewport {
                axis: ViewportAxis::Min,
                value: self.vmin,
            }),
            UNIT_VMAX => Some(LengthSpec::Viewport {
                axis: ViewportAxis::Max,
                value: self.vmax,
            }),
            units if units == UNIT_PX | UNIT_PERCENT => Some(LengthSpec::CalcPercentOffset {
                percent: self.percent,
                offset_px: self.px,
            }),
            units if units == UNIT_PX | UNIT_EM => Some(LengthSpec::CalcEmOffset {
                em: self.em,
                offset_px: self.px,
            }),
            units if units == UNIT_PX | UNIT_REM => Some(LengthSpec::CalcRemOffset {
                rem: self.rem,
                offset_px: self.px,
            }),
            units if units == UNIT_PX | UNIT_VW => Some(LengthSpec::CalcViewportOffset {
                axis: ViewportAxis::Width,
                value: self.vw,
                offset_px: self.px,
            }),
            units if units == UNIT_PX | UNIT_VH => Some(LengthSpec::CalcViewportOffset {
                axis: ViewportAxis::Height,
                value: self.vh,
                offset_px: self.px,
            }),
            units if units == UNIT_PX | UNIT_VMIN => Some(LengthSpec::CalcViewportOffset {
                axis: ViewportAxis::Min,
                value: self.vmin,
                offset_px: self.px,
            }),
            units if units == UNIT_PX | UNIT_VMAX => Some(LengthSpec::CalcViewportOffset {
                axis: ViewportAxis::Max,
                value: self.vmax,
                offset_px: self.px,
            }),
            _ => None,
        }
    }
}

fn percent_to_spec(p: f32) -> Option<LengthSpec> {
    p.is_finite().then_some(LengthSpec::Percent(p))
}

fn calc_sum_from_length_spec(spec: LengthSpec) -> Option<CalcSum> {
    Some(match spec {
        LengthSpec::Px(v) => CalcSum::px(v),
        LengthSpec::Percent(p) => CalcSum::percent(p),
        LengthSpec::Fill => CalcSum::percent(100.0),
        LengthSpec::Em(v) => CalcSum::em(v),
        LengthSpec::Rem(v) => CalcSum::rem(v),
        LengthSpec::Viewport { axis, value } => CalcSum::viewport(axis, value),
        LengthSpec::CalcPercentOffset { percent, offset_px } => {
            let mut s = CalcSum::percent(percent);
            s.px = offset_px;
            s.units |= UNIT_PX;
            s
        }
        LengthSpec::CalcViewportOffset {
            axis,
            value,
            offset_px,
        } => {
            let mut s = CalcSum::viewport(axis, value);
            s.px = offset_px;
            s.units |= UNIT_PX;
            s
        }
        LengthSpec::CalcEmOffset { em, offset_px } => {
            let mut s = CalcSum::em(em);
            s.px = offset_px;
            s.units |= UNIT_PX;
            s
        }
        LengthSpec::CalcRemOffset { rem, offset_px } => {
            let mut s = CalcSum::rem(rem);
            s.px = offset_px;
            s.units |= UNIT_PX;
            s
        }
        LengthSpec::Min2(_, _)
        | LengthSpec::Max2(_, _)
        | LengthSpec::Clamp3(_, _, _)
        | LengthSpec::Shrink
        | LengthSpec::Auto
        | LengthSpec::MinContent
        | LengthSpec::MaxContent
        | LengthSpec::FitContent => return None,
    })
}

struct CalcParser<'a> {
    s: &'a str,
    i: usize,
    depth: u8,
}

impl<'a> CalcParser<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }

    fn skip_ws(&mut self) {
        let bytes = self.s.as_bytes();
        while self.i < bytes.len() && bytes[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.as_bytes().get(self.i).copied()
    }

    fn eat(&mut self, ch: u8) -> bool {
        if self.peek() == Some(ch) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn enter(&mut self) -> bool {
        if self.depth >= MAX_CALC_DEPTH {
            return false;
        }
        self.depth += 1;
        true
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn parse_add(&mut self) -> Option<CalcSum> {
        let mut acc = self.parse_mul()?;
        loop {
            self.skip_ws();
            let sign = match self.peek() {
                Some(b'+') => {
                    self.i += 1;
                    1.0
                }
                Some(b'-') => {
                    self.i += 1;
                    -1.0
                }
                _ => break,
            };
            let rhs = self.parse_mul()?;
            acc = if sign > 0.0 {
                acc.add(rhs)?
            } else {
                acc.add(rhs.neg()?)?
            };
        }
        Some(acc)
    }

    fn parse_mul(&mut self) -> Option<CalcSum> {
        let mut acc = self.parse_unary()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'*') => {
                    self.i += 1;
                    acc = acc.mul(self.parse_unary()?)?;
                }
                Some(b'/') => {
                    self.i += 1;
                    acc = acc.div(self.parse_unary()?)?;
                }
                _ => break,
            }
        }
        Some(acc)
    }

    fn parse_unary(&mut self) -> Option<CalcSum> {
        self.skip_ws();
        let mut neg = false;
        let mut signs = 0u8;
        loop {
            self.skip_ws();
            if self.eat(b'+') {
                signs += 1;
            } else if self.eat(b'-') {
                neg = !neg;
                signs += 1;
            } else {
                break;
            }
            if signs > MAX_CALC_DEPTH {
                return None;
            }
        }
        let value = self.parse_primary()?;
        if neg { value.neg() } else { Some(value) }
    }

    fn parse_primary(&mut self) -> Option<CalcSum> {
        self.skip_ws();
        if self.eat(b'(') {
            if !self.enter() {
                return None;
            }
            let inner = self.parse_add();
            self.skip_ws();
            let closed = self.eat(b')');
            self.leave();
            if !closed {
                return None;
            }
            return inner;
        }
        if let Some(name) = self.peek_ident() {
            let lower = name.to_ascii_lowercase();
            return match lower.as_str() {
                "calc" | "min" | "max" | "clamp" => self.parse_fn(&lower),
                // Unknown math functions (tan, atan2, sin, …) fail closed.
                _ => None,
            };
        }
        self.parse_number_or_dimension()
    }

    fn peek_ident(&self) -> Option<&'a str> {
        let rest = self.rest();
        let mut end = 0usize;
        for (i, ch) in rest.char_indices() {
            if i == 0 {
                if !ch.is_ascii_alphabetic() && ch != '_' {
                    return None;
                }
                end = i + ch.len_utf8();
            } else if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                end = i + ch.len_utf8();
            } else {
                break;
            }
        }
        if end == 0 { None } else { Some(&rest[..end]) }
    }

    fn consume_ident(&mut self) {
        if let Some(id) = self.peek_ident() {
            self.i += id.len();
        }
    }

    fn parse_fn(&mut self, name: &str) -> Option<CalcSum> {
        self.consume_ident();
        self.skip_ws();
        if !self.eat(b'(') {
            return None;
        }
        if !self.enter() {
            return None;
        }
        let inner_start = self.i;
        let bytes = self.s.as_bytes();
        let mut depth = 1i32;
        while self.i < bytes.len() {
            match bytes[self.i] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        let inner = &self.s[inner_start..self.i];
                        self.i += 1;
                        let out = self.eval_fn(name, inner);
                        self.leave();
                        return out;
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
        self.leave();
        None
    }

    fn eval_fn(&self, name: &str, inner: &str) -> Option<CalcSum> {
        match name {
            "calc" => parse_calc_expr_to_sum_at(inner, self.depth),
            "min" | "max" | "clamp" => {
                let spec = parse_min_max_clamp_inner(name, inner, self.depth)?;
                calc_sum_from_length_spec(spec)
            }
            _ => None,
        }
    }

    fn parse_number_or_dimension(&mut self) -> Option<CalcSum> {
        let n = self.parse_number()?;
        let rest = self.rest();
        if rest.starts_with('%') {
            self.i += 1;
            return Some(CalcSum::percent(n));
        }
        let Some(ident) = self.peek_ident() else {
            return Some(CalcSum::number(n));
        };
        match ident.to_ascii_lowercase().as_str() {
            "px" => {
                self.consume_ident();
                Some(CalcSum::px(n))
            }
            "em" => {
                self.consume_ident();
                Some(CalcSum::em(n))
            }
            "rem" => {
                self.consume_ident();
                Some(CalcSum::rem(n))
            }
            "vw" => {
                self.consume_ident();
                Some(CalcSum::viewport(ViewportAxis::Width, n))
            }
            "vh" => {
                self.consume_ident();
                Some(CalcSum::viewport(ViewportAxis::Height, n))
            }
            "vmin" => {
                self.consume_ident();
                Some(CalcSum::viewport(ViewportAxis::Min, n))
            }
            "vmax" => {
                self.consume_ident();
                Some(CalcSum::viewport(ViewportAxis::Max, n))
            }
            // Unknown unit (`deg`, `in`, …) fails closed.
            _ => None,
        }
    }

    fn parse_number(&mut self) -> Option<f32> {
        let start = self.i;
        let b = self.s.as_bytes();
        let mut i = self.i;
        let mut saw_digit = false;
        while i < b.len() && b[i].is_ascii_digit() {
            saw_digit = true;
            i += 1;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                saw_digit = true;
                i += 1;
            }
        }
        if !saw_digit {
            return None;
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            let exp_start = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > exp_start {
                i = j;
            }
        }
        let num = self.s[start..i].parse::<f32>().ok()?;
        if !num.is_finite() {
            return None;
        }
        self.i = i;
        Some(num)
    }
}

fn parse_length_term_to_spec(raw: &str) -> Option<LengthSpec> {
    let s = raw.trim();
    if let Some(p) = parse_percent_term(s) {
        return percent_to_spec(p);
    }
    if let Some(spec) = parse_viewport_length(s) {
        return Some(spec);
    }
    if let Some(spec) = parse_font_relative_length(s) {
        return Some(spec);
    }
    if let Some(px) = parse_px_term(s) {
        return Some(LengthSpec::Px(px.max(0.0)));
    }
    None
}

fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn parse_percent_term(raw: &str) -> Option<f32> {
    let p = raw.trim().strip_suffix('%')?.trim().parse::<f32>().ok()?;
    Some(p)
}

fn parse_px_term(raw: &str) -> Option<f32> {
    let s = raw.trim();
    let num: f32 = s
        .trim_end_matches("px")
        .trim_end_matches("PX")
        .trim()
        .parse()
        .ok()?;
    Some(num)
}

fn parse_viewport_term(raw: &str) -> Option<(ViewportAxis, f32)> {
    let s = raw.trim().to_ascii_lowercase();
    let (num, axis) = if let Some(n) = s.strip_suffix("vmin") {
        (n, ViewportAxis::Min)
    } else if let Some(n) = s.strip_suffix("vmax") {
        (n, ViewportAxis::Max)
    } else if let Some(n) = s.strip_suffix("vh") {
        (n, ViewportAxis::Height)
    } else {
        let n = s.strip_suffix("vw")?;
        (n, ViewportAxis::Width)
    };
    let value = num.trim().parse::<f32>().ok()?;
    Some((axis, value.max(0.0)))
}

fn parse_viewport_length(raw: &str) -> Option<LengthSpec> {
    let (axis, value) = parse_viewport_term(raw)?;
    Some(LengthSpec::Viewport { axis, value })
}

/// Bare `100vh - 32px` / `100vw + 8px` (no calc wrapper).
fn parse_viewport_px_sum(raw: &str) -> Option<LengthSpec> {
    let s = raw.trim();
    if s.to_ascii_lowercase().starts_with("calc(") {
        return None;
    }
    // Must involve a viewport unit.
    let lower = s.to_ascii_lowercase();
    if !(lower.contains("vh")
        || lower.contains("vw")
        || lower.contains("vmin")
        || lower.contains("vmax"))
    {
        return None;
    }
    parse_calc_expr_to_spec_at(s, 0)
}

/// `min(a,b)` / `max(a,b)` / `clamp(min,val,max)` — args are math expressions.
fn parse_css_min_max_clamp(raw: &str) -> Option<LengthSpec> {
    parse_css_min_max_clamp_at(raw, 0)
}

fn parse_css_min_max_clamp_at(raw: &str, depth: u8) -> Option<LengthSpec> {
    if depth >= MAX_CALC_DEPTH {
        return None;
    }
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("min(") && !lower.starts_with("minmax(") {
        let (inner, rest) = split_paren_inner(&s[4..])?;
        if !rest.trim().is_empty() {
            return None;
        }
        return parse_min_max_clamp_inner("min", inner, depth);
    }
    if lower.starts_with("max(") && !lower.starts_with("minmax(") {
        let (inner, rest) = split_paren_inner(&s[4..])?;
        if !rest.trim().is_empty() {
            return None;
        }
        return parse_min_max_clamp_inner("max", inner, depth);
    }
    if lower.starts_with("clamp(") {
        let (inner, rest) = split_paren_inner(&s[6..])?;
        if !rest.trim().is_empty() {
            return None;
        }
        return parse_min_max_clamp_inner("clamp", inner, depth);
    }
    None
}

fn parse_min_max_clamp_inner(kind: &str, inner: &str, depth: u8) -> Option<LengthSpec> {
    let args = split_top_level_commas(inner)?;
    if args.is_empty() || args.len() > 8 {
        return None;
    }
    let next = depth.saturating_add(1);
    match kind {
        "min" => parse_min_or_max_args(args, next, true),
        "max" => parse_min_or_max_args(args, next, false),
        "clamp" => {
            if args.len() != 3 {
                return None;
            }
            let lo = parse_length_atom_at(args[0], next)?;
            let val = parse_length_atom_at(args[1], next)?;
            let hi = parse_length_atom_at(args[2], next)?;
            if let Some(folded) = fold_clamp_atoms(lo, val, hi) {
                return Some(length_atom_to_spec(folded));
            }
            Some(LengthSpec::Clamp3(lo, val, hi))
        }
        _ => None,
    }
}

fn parse_min_or_max_args(args: Vec<&str>, depth: u8, is_min: bool) -> Option<LengthSpec> {
    if args.len() == 1 {
        return parse_math_length_at(args[0], depth);
    }
    let mut specs = Vec::with_capacity(args.len());
    for arg in args {
        specs.push(parse_math_length_at(arg, depth)?);
    }
    reduce_min_max_specs(specs, is_min)
}

/// Fold same-dimension atoms; leftover mixed pair → Min2/Max2.
/// `min(A, max(B, C))` / `max(A, min(B, C))` → Clamp3 when A shares a
/// dimension with B or C. Three incomparable leftovers fail closed (no Min list).
fn reduce_min_max_specs(specs: Vec<LengthSpec>, is_min: bool) -> Option<LengthSpec> {
    let mut atoms: Vec<LengthAtom> = Vec::new();
    let mut opposite: Option<LengthSpec> = None;
    for spec in specs {
        collect_min_max_operand(spec, is_min, &mut atoms, &mut opposite)?;
    }
    let atoms = fold_atom_list(atoms, is_min);
    match (atoms.as_slice(), opposite) {
        ([], None) => None,
        ([one], None) => Some(length_atom_to_spec(*one)),
        ([a, b], None) => Some(min_max_two_atoms(*a, *b, is_min)),
        (_, None) => None,
        ([], Some(other)) => Some(other),
        ([one], Some(other)) => merge_atom_with_opposite(*one, other, is_min),
        _ => None,
    }
}

fn collect_min_max_operand(
    spec: LengthSpec,
    is_min: bool,
    atoms: &mut Vec<LengthAtom>,
    opposite: &mut Option<LengthSpec>,
) -> Option<()> {
    if let Some(atom) = length_spec_to_atom(spec) {
        atoms.push(atom);
        return Some(());
    }
    match spec {
        LengthSpec::Min2(a, b) if is_min => {
            atoms.push(a);
            atoms.push(b);
            Some(())
        }
        LengthSpec::Max2(a, b) if !is_min => {
            atoms.push(a);
            atoms.push(b);
            Some(())
        }
        LengthSpec::Min2(_, _) | LengthSpec::Max2(_, _) | LengthSpec::Clamp3(_, _, _) => {
            if opposite.is_some() {
                return None;
            }
            *opposite = Some(spec);
            Some(())
        }
        _ => None,
    }
}

fn fold_atom_list(atoms: Vec<LengthAtom>, is_min: bool) -> Vec<LengthAtom> {
    let mut out: Vec<LengthAtom> = Vec::new();
    for atom in atoms {
        let mut merged = false;
        for slot in &mut out {
            if let Some(folded) = fold_min_or_max_atoms(*slot, atom, is_min) {
                *slot = folded;
                merged = true;
                break;
            }
        }
        if !merged {
            out.push(atom);
        }
    }
    out
}

fn min_max_two_atoms(a: LengthAtom, b: LengthAtom, is_min: bool) -> LengthSpec {
    if let Some(folded) = fold_min_or_max_atoms(a, b, is_min) {
        return length_atom_to_spec(folded);
    }
    if is_min {
        LengthSpec::Min2(a, b)
    } else {
        LengthSpec::Max2(a, b)
    }
}

fn merge_atom_with_opposite(
    atom: LengthAtom,
    other: LengthSpec,
    is_min: bool,
) -> Option<LengthSpec> {
    match (other, is_min) {
        (LengthSpec::Max2(x, y), true) => min_atom_and_max2(atom, x, y),
        (LengthSpec::Min2(x, y), false) => max_atom_and_min2(atom, x, y),
        (LengthSpec::Clamp3(lo, val, hi), _) => merge_atom_with_clamp3(atom, lo, val, hi, is_min),
        (LengthSpec::Min2(x, y), true) => reduce_min_max_specs(
            vec![
                length_atom_to_spec(atom),
                length_atom_to_spec(x),
                length_atom_to_spec(y),
            ],
            true,
        ),
        (LengthSpec::Max2(x, y), false) => reduce_min_max_specs(
            vec![
                length_atom_to_spec(atom),
                length_atom_to_spec(x),
                length_atom_to_spec(y),
            ],
            false,
        ),
        (spec, _) => {
            let b = length_spec_to_atom(spec)?;
            Some(min_max_two_atoms(atom, b, is_min))
        }
    }
}

fn atom_order_same_dim(a: LengthAtom, b: LengthAtom) -> Option<std::cmp::Ordering> {
    let min_ab = fold_min_or_max_atoms(a, b, true)?;
    if min_ab == a && min_ab == b {
        Some(std::cmp::Ordering::Equal)
    } else if min_ab == a {
        Some(std::cmp::Ordering::Less)
    } else {
        Some(std::cmp::Ordering::Greater)
    }
}

/// `min(a, max(x, y))` → atom or Clamp3 when `a` shares a dimension with x or y.
fn min_atom_and_max2(a: LengthAtom, x: LengthAtom, y: LengthAtom) -> Option<LengthSpec> {
    if let Some(ord) = atom_order_same_dim(a, x) {
        return Some(match ord {
            std::cmp::Ordering::Less | std::cmp::Ordering::Equal => length_atom_to_spec(a),
            std::cmp::Ordering::Greater => LengthSpec::Clamp3(x, y, a),
        });
    }
    if let Some(ord) = atom_order_same_dim(a, y) {
        return Some(match ord {
            std::cmp::Ordering::Less | std::cmp::Ordering::Equal => length_atom_to_spec(a),
            std::cmp::Ordering::Greater => LengthSpec::Clamp3(y, x, a),
        });
    }
    None
}

/// `max(a, min(x, y))` → atom or Clamp3 when `a` shares a dimension with x or y.
fn max_atom_and_min2(a: LengthAtom, x: LengthAtom, y: LengthAtom) -> Option<LengthSpec> {
    if let Some(ord) = atom_order_same_dim(a, x) {
        return Some(match ord {
            std::cmp::Ordering::Greater | std::cmp::Ordering::Equal => length_atom_to_spec(a),
            std::cmp::Ordering::Less => LengthSpec::Clamp3(a, y, x),
        });
    }
    if let Some(ord) = atom_order_same_dim(a, y) {
        return Some(match ord {
            std::cmp::Ordering::Greater | std::cmp::Ordering::Equal => length_atom_to_spec(a),
            std::cmp::Ordering::Less => LengthSpec::Clamp3(a, x, y),
        });
    }
    None
}

fn merge_atom_with_clamp3(
    atom: LengthAtom,
    lo: LengthAtom,
    val: LengthAtom,
    hi: LengthAtom,
    is_min: bool,
) -> Option<LengthSpec> {
    let (lo_b, hi_b) = match atom_order_same_dim(lo, hi) {
        Some(std::cmp::Ordering::Greater) => (hi, lo),
        Some(_) => (lo, hi),
        None => return None,
    };
    let vs_lo = atom_order_same_dim(atom, lo_b)?;
    let vs_hi = atom_order_same_dim(atom, hi_b)?;
    match (is_min, vs_lo, vs_hi) {
        (true, std::cmp::Ordering::Less | std::cmp::Ordering::Equal, _) => {
            Some(length_atom_to_spec(atom))
        }
        (true, _, std::cmp::Ordering::Greater | std::cmp::Ordering::Equal) => {
            Some(LengthSpec::Clamp3(lo_b, val, hi_b))
        }
        (true, std::cmp::Ordering::Greater, std::cmp::Ordering::Less) => {
            Some(LengthSpec::Clamp3(lo_b, val, atom))
        }
        (false, std::cmp::Ordering::Less | std::cmp::Ordering::Equal, _) => {
            Some(LengthSpec::Clamp3(lo_b, val, hi_b))
        }
        (false, _, std::cmp::Ordering::Greater | std::cmp::Ordering::Equal) => {
            Some(length_atom_to_spec(atom))
        }
        (false, std::cmp::Ordering::Greater, std::cmp::Ordering::Less) => {
            Some(LengthSpec::Clamp3(atom, val, hi_b))
        }
    }
}

fn split_top_level_commas(inner: &str) -> Option<Vec<&str>> {
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, ch) in inner.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                let a = inner[start..i].trim();
                if a.is_empty() {
                    return None;
                }
                args.push(a);
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = inner[start..].trim();
    if last.is_empty() {
        return None;
    }
    args.push(last);
    Some(args)
}

fn parse_math_length_at(raw: &str, depth: u8) -> Option<LengthSpec> {
    if depth >= MAX_CALC_DEPTH {
        return None;
    }
    let s = raw.trim();
    if let Some(spec) = parse_css_min_max_clamp_at(s, depth) {
        return Some(spec);
    }
    parse_calc_expr_to_spec_at(s, depth)
}

fn parse_length_atom_at(raw: &str, depth: u8) -> Option<LengthAtom> {
    length_spec_to_atom(parse_math_length_at(raw, depth)?)
}

fn fold_min_or_max_atoms(a: LengthAtom, b: LengthAtom, is_min: bool) -> Option<LengthAtom> {
    match (a, b) {
        (LengthAtom::Px(x), LengthAtom::Px(y)) => {
            Some(LengthAtom::Px(if is_min { x.min(y) } else { x.max(y) }))
        }
        (LengthAtom::Percent(x), LengthAtom::Percent(y)) => Some(LengthAtom::Percent(if is_min {
            x.min(y)
        } else {
            x.max(y)
        })),
        (LengthAtom::Em(x), LengthAtom::Em(y)) => {
            Some(LengthAtom::Em(if is_min { x.min(y) } else { x.max(y) }))
        }
        (LengthAtom::Rem(x), LengthAtom::Rem(y)) => {
            Some(LengthAtom::Rem(if is_min { x.min(y) } else { x.max(y) }))
        }
        (
            LengthAtom::Viewport {
                axis: a1,
                value: v1,
            },
            LengthAtom::Viewport {
                axis: a2,
                value: v2,
            },
        ) if a1 == a2 => Some(LengthAtom::Viewport {
            axis: a1,
            value: if is_min { v1.min(v2) } else { v1.max(v2) },
        }),
        (
            LengthAtom::CalcPercent {
                percent: p1,
                offset_px: o1,
            },
            LengthAtom::CalcPercent {
                percent: p2,
                offset_px: o2,
            },
        ) if (p1 - p2).abs() < 1e-4 => Some(LengthAtom::CalcPercent {
            percent: p1,
            offset_px: if is_min { o1.min(o2) } else { o1.max(o2) },
        }),
        (
            LengthAtom::CalcViewport {
                axis: a1,
                value: v1,
                offset_px: o1,
            },
            LengthAtom::CalcViewport {
                axis: a2,
                value: v2,
                offset_px: o2,
            },
        ) if a1 == a2 && (v1 - v2).abs() < 1e-4 => Some(LengthAtom::CalcViewport {
            axis: a1,
            value: v1,
            offset_px: if is_min { o1.min(o2) } else { o1.max(o2) },
        }),
        (
            LengthAtom::CalcEm {
                em: e1,
                offset_px: o1,
            },
            LengthAtom::CalcEm {
                em: e2,
                offset_px: o2,
            },
        ) if (e1 - e2).abs() < 1e-4 => Some(LengthAtom::CalcEm {
            em: e1,
            offset_px: if is_min { o1.min(o2) } else { o1.max(o2) },
        }),
        (
            LengthAtom::CalcRem {
                rem: r1,
                offset_px: o1,
            },
            LengthAtom::CalcRem {
                rem: r2,
                offset_px: o2,
            },
        ) if (r1 - r2).abs() < 1e-4 => Some(LengthAtom::CalcRem {
            rem: r1,
            offset_px: if is_min { o1.min(o2) } else { o1.max(o2) },
        }),
        _ => None,
    }
}

fn fold_clamp_atoms(lo: LengthAtom, val: LengthAtom, hi: LengthAtom) -> Option<LengthAtom> {
    match (lo, val, hi) {
        (LengthAtom::Px(a), LengthAtom::Px(v), LengthAtom::Px(b)) => {
            Some(LengthAtom::Px(v.clamp(a.min(b), a.max(b))))
        }
        (LengthAtom::Percent(a), LengthAtom::Percent(v), LengthAtom::Percent(b)) => {
            Some(LengthAtom::Percent(v.clamp(a.min(b), a.max(b))))
        }
        (LengthAtom::Em(a), LengthAtom::Em(v), LengthAtom::Em(b)) => {
            Some(LengthAtom::Em(v.clamp(a.min(b), a.max(b))))
        }
        (LengthAtom::Rem(a), LengthAtom::Rem(v), LengthAtom::Rem(b)) => {
            Some(LengthAtom::Rem(v.clamp(a.min(b), a.max(b))))
        }
        (
            LengthAtom::Viewport {
                axis: a1,
                value: lo_v,
            },
            LengthAtom::Viewport {
                axis: a2,
                value: val_v,
            },
            LengthAtom::Viewport {
                axis: a3,
                value: hi_v,
            },
        ) if a1 == a2 && a2 == a3 => Some(LengthAtom::Viewport {
            axis: a1,
            value: val_v.clamp(lo_v.min(hi_v), lo_v.max(hi_v)),
        }),
        _ => None,
    }
}

fn length_atom_to_spec(atom: LengthAtom) -> LengthSpec {
    match atom {
        LengthAtom::Px(v) => LengthSpec::Px(v),
        LengthAtom::Percent(p) => percent_to_spec(p).unwrap_or(LengthSpec::Percent(p)),
        LengthAtom::Em(v) => LengthSpec::Em(v),
        LengthAtom::Rem(v) => LengthSpec::Rem(v),
        LengthAtom::Viewport { axis, value } => LengthSpec::Viewport { axis, value },
        LengthAtom::CalcPercent { percent, offset_px } => {
            LengthSpec::CalcPercentOffset { percent, offset_px }
        }
        LengthAtom::CalcViewport {
            axis,
            value,
            offset_px,
        } => LengthSpec::CalcViewportOffset {
            axis,
            value,
            offset_px,
        },
        LengthAtom::CalcEm { em, offset_px } => LengthSpec::CalcEmOffset { em, offset_px },
        LengthAtom::CalcRem { rem, offset_px } => LengthSpec::CalcRemOffset { rem, offset_px },
    }
}

fn length_spec_to_atom(spec: LengthSpec) -> Option<LengthAtom> {
    match spec {
        LengthSpec::Px(v) => Some(LengthAtom::Px(v)),
        LengthSpec::Percent(p) => Some(LengthAtom::Percent(p)),
        LengthSpec::Em(v) => Some(LengthAtom::Em(v)),
        LengthSpec::Rem(v) => Some(LengthAtom::Rem(v)),
        LengthSpec::Fill => Some(LengthAtom::Percent(100.0)),
        LengthSpec::CalcPercentOffset { percent, offset_px } => {
            Some(LengthAtom::CalcPercent { percent, offset_px })
        }
        LengthSpec::Viewport { axis, value } => Some(LengthAtom::Viewport { axis, value }),
        LengthSpec::CalcViewportOffset {
            axis,
            value,
            offset_px,
        } => Some(LengthAtom::CalcViewport {
            axis,
            value,
            offset_px,
        }),
        LengthSpec::CalcEmOffset { em, offset_px } => Some(LengthAtom::CalcEm { em, offset_px }),
        LengthSpec::CalcRemOffset { rem, offset_px } => {
            Some(LengthAtom::CalcRem { rem, offset_px })
        }
        // Nested mixed min/max/clamp that did not fold stay LengthSpec-only.
        _ => None,
    }
}

/// L1 CSS / class → [`LayoutStyle`] mutation (parse stays in this crate).
pub trait LayoutStyleCss {
    fn apply_class_layout_hints(&mut self, class_names: &[String]);
    fn apply_css_text(&mut self, style: &str, percent_w: Option<f32>, percent_h: Option<f32>);
    fn apply_css_property(
        &mut self,
        key: &str,
        val: &str,
        percent_w: Option<f32>,
        percent_h: Option<f32>,
    );
    fn apply_flex_shorthand(&mut self, val: &str);
    fn apply_host_style(
        &mut self,
        value: &nana_js_engine::HostValue,
        percent_w: Option<f32>,
        percent_h: Option<f32>,
    );
}

impl LayoutStyleCss for LayoutStyle {
    /// Nana shell / controls class → partial `LayoutStyle`.
    ///
    /// Delegates to [`crate::shell_contract::apply_class_layout_hints`] so this
    /// module stays the neutral CSS → [`LayoutStyle`] parse path.
    fn apply_class_layout_hints(&mut self, class_names: &[String]) {
        crate::shell_contract::apply_class_layout_hints(self, class_names);
    }

    /// 解析 CSS 声明串（`a: b; c: d`）写入自身。
    ///
    /// Trailing `!important` is stripped so the length/keyword still parses.
    /// Cascade promotion of the flag is [`crate::css_cascade::rebuild_layout_style`].
    /// Stylesheet cascade hot path prefers cached
    /// [`crate::css_cascade::DeclarationEntry`] → [`Self::apply_css_property`]
    /// instead of re-splitting rule text on every match.
    fn apply_css_text(&mut self, style: &str, percent_w: Option<f32>, percent_h: Option<f32>) {
        // `direction` / `writing-mode` first so later logical props in the same
        // declaration block map against the used dir (not source order).
        for_each_css_decl(style, |key, val| {
            if css_key_is_direction_or_writing_mode(key) {
                self.apply_css_property(key, val, percent_w, percent_h);
            }
        });
        for_each_css_decl(style, |key, val| {
            if !css_key_is_direction_or_writing_mode(key) {
                self.apply_css_property(key, val, percent_w, percent_h);
            }
        });
    }

    /// 解析单个 CSS 属性。
    fn apply_css_property(
        &mut self,
        key: &str,
        val: &str,
        percent_w: Option<f32>,
        percent_h: Option<f32>,
    ) {
        let key = key.trim().replace('_', "-");
        let key = if key.chars().any(|c| c.is_ascii_uppercase()) {
            camel_to_kebab(&key)
        } else {
            key.to_ascii_lowercase()
        };
        // Inline / prop / object style may still carry `!important`; strip so
        // `width:100px !important` parses as 100px. Flag precedence is cascade.
        let (stripped, _) = split_important_flag(val);
        // Author stylesheets often use `var(--token, fallback)` — expand fallback
        // so overflow/length keywords still parse (tokens themselves stay unresolved).
        let val_owned = expand_css_var_fallback(&stripped);
        let val = val_owned.as_str();
        match key.as_str() {
            "display" if val.eq_ignore_ascii_case("none") => {
                self.display = Some(DisplaySpec::None);
                self.hidden = true;
            }
            "display" if val.eq_ignore_ascii_case("contents") => {
                self.display = Some(DisplaySpec::Contents);
                self.hidden = false;
            }
            "display"
                if val.eq_ignore_ascii_case("flex") || val.eq_ignore_ascii_case("inline-flex") =>
            {
                self.display = Some(if val.eq_ignore_ascii_case("inline-flex") {
                    DisplaySpec::InlineFlex
                } else {
                    DisplaySpec::Flex
                });
                // CSS: grid tracks do not apply to flex containers.
                self.grid_columns = None;
                self.grid_rows = None;
                self.grid_columns_unsupported = None;
                self.grid_rows_unsupported = None;
                self.grid_columns_repeat = None;
                self.grid_rows_repeat = None;
                self.grid_auto_columns = None;
                self.grid_auto_rows = None;
                self.grid_auto_flow = None;
                self.grid_template_areas = None;
                self.grid_column_line_names = None;
                self.grid_row_line_names = None;
                if self.direction.is_none() {
                    self.direction = Some(FlexDirection::Row);
                }
                // CSS initial `align-items: stretch`. Seed only while still at the
                // engine default so a later `align-items:` in the same rule wins.
                if self.align_items == AlignSpec::Start {
                    self.align_items = AlignSpec::Stretch;
                }
            }
            "display"
                if val.eq_ignore_ascii_case("block") || val.eq_ignore_ascii_case("flow-root") =>
            {
                self.display = Some(DisplaySpec::Block);
                if self.direction.is_none() {
                    self.direction = Some(FlexDirection::Column);
                }
                // Block formatting: width:auto children stretch to the CB.
                if self.align_items == AlignSpec::Start {
                    self.align_items = AlignSpec::Stretch;
                }
            }
            "display" if val.eq_ignore_ascii_case("inline") => {
                self.display = Some(DisplaySpec::Inline);
                self.hidden = false;
            }
            "display" if val.eq_ignore_ascii_case("inline-block") => {
                self.display = Some(DisplaySpec::InlineBlock);
                self.hidden = false;
            }
            "display"
                if val.eq_ignore_ascii_case("grid") || val.eq_ignore_ascii_case("inline-grid") =>
            {
                self.display = Some(if val.eq_ignore_ascii_case("inline-grid") {
                    DisplaySpec::InlineGrid
                } else {
                    DisplaySpec::Grid
                });
                if self.direction.is_none() {
                    self.direction = Some(FlexDirection::Row);
                }
                // CSS Grid initial `align-items: stretch` (same seed as flex).
                if self.align_items == AlignSpec::Start {
                    self.align_items = AlignSpec::Stretch;
                }
            }
            "flex-direction" => apply_flex_direction(self, val),
            "flex-wrap" => {
                let v = val.trim().to_ascii_lowercase();
                match v.as_str() {
                    "wrap" => self.flex_wrap = FlexWrap::Wrap,
                    "wrap-reverse" => self.flex_wrap = FlexWrap::WrapReverse,
                    "nowrap" | "initial" | "unset" => self.flex_wrap = FlexWrap::NoWrap,
                    _ => {}
                }
            }
            "flex-flow" => apply_flex_flow(self, val),
            "flex" => self.apply_flex_shorthand(val),
            "order" => {
                // MDN Flexbox `order`: <integer>（含负值）；初始 0。
                if let Ok(o) = val.trim().parse::<i32>() {
                    self.order = o;
                }
            }
            "flex-grow" => {
                if let Ok(g) = val.trim().parse::<f32>() {
                    self.flex_grow = Some(g.max(0.0));
                }
            }
            "flex-shrink" => {
                if let Ok(s) = val.trim().parse::<f32>() {
                    self.flex_shrink = Some(s.max(0.0));
                }
            }
            "flex-basis" => {
                self.flex_basis = LengthSpec::parse(val);
            }
            "gap" => apply_gap_shorthand(val, self),
            "row-gap" => {
                if let Some(spec) = parse_gap_length(val) {
                    self.row_gap = Some(spec);
                }
            }
            "column-gap" => {
                if let Some(spec) = parse_gap_length(val) {
                    self.column_gap = Some(spec);
                }
            }
            "padding" => {
                apply_box_edge_shorthand(
                    val,
                    &mut self.padding,
                    &mut self.padding_top,
                    &mut self.padding_right,
                    &mut self.padding_bottom,
                    &mut self.padding_left,
                    parse_box_edge_length,
                );
                self.logical_padding
                    .set_phys_left(self.padding_left.or(self.padding));
                self.logical_padding
                    .set_phys_right(self.padding_right.or(self.padding));
            }
            // Longhand margin/padding % — including top/bottom — use containing-block width
            // at layout time (store LengthSpec; do not drop % when percent_w is None).
            "padding-top" => self.padding_top = parse_box_edge_length(val),
            "padding-right" => {
                self.padding_right = parse_box_edge_length(val);
                self.logical_padding.set_phys_right(self.padding_right);
            }
            "padding-bottom" => self.padding_bottom = parse_box_edge_length(val),
            "padding-left" => {
                self.padding_left = parse_box_edge_length(val);
                self.logical_padding.set_phys_left(self.padding_left);
            }
            // Logical padding kept until used-value resolve (final `direction`).
            "padding-inline" => {
                apply_logical_inline_edges(val, &mut self.logical_padding, parse_box_edge_length);
            }
            "padding-block" => apply_logical_pair_shorthand(
                val,
                &mut self.padding_top,
                &mut self.padding_bottom,
                parse_box_edge_length,
            ),
            "padding-inline-start" => {
                self.logical_padding.set_start(parse_box_edge_length(val));
            }
            "padding-inline-end" => {
                self.logical_padding.set_end(parse_box_edge_length(val));
            }
            "padding-block-start" => self.padding_top = parse_box_edge_length(val),
            "padding-block-end" => self.padding_bottom = parse_box_edge_length(val),
            "margin" => {
                apply_box_edge_shorthand(
                    val,
                    &mut self.margin,
                    &mut self.margin_top,
                    &mut self.margin_right,
                    &mut self.margin_bottom,
                    &mut self.margin_left,
                    parse_margin_length,
                );
                self.logical_margin
                    .set_phys_left(self.margin_left.or(self.margin));
                self.logical_margin
                    .set_phys_right(self.margin_right.or(self.margin));
            }
            "margin-top" => self.margin_top = parse_margin_length(val),
            "margin-right" => {
                self.margin_right = parse_margin_length(val);
                self.logical_margin.set_phys_right(self.margin_right);
            }
            "margin-bottom" => self.margin_bottom = parse_margin_length(val),
            "margin-left" => {
                self.margin_left = parse_margin_length(val);
                self.logical_margin.set_phys_left(self.margin_left);
            }
            "margin-inline" => {
                apply_logical_inline_edges(val, &mut self.logical_margin, parse_margin_length);
            }
            "margin-block" => apply_logical_pair_shorthand(
                val,
                &mut self.margin_top,
                &mut self.margin_bottom,
                parse_margin_length,
            ),
            "margin-inline-start" => {
                self.logical_margin.set_start(parse_margin_length(val));
            }
            "margin-inline-end" => {
                self.logical_margin.set_end(parse_margin_length(val));
            }
            "margin-block-start" => self.margin_top = parse_margin_length(val),
            "margin-block-end" => self.margin_bottom = parse_margin_length(val),
            "width" => self.width = LengthSpec::parse(val),
            "height" => {
                // Keep Fill for 100% even without percent base (定高链 P0-4)。
                if val.trim() == "100%" {
                    self.height = Some(LengthSpec::Fill);
                } else {
                    self.height = LengthSpec::parse(val);
                }
            }
            // Box-model sizes: store LengthSpec (defer %/em/vh until measure).
            "min-width" => {
                if let Some(spec) = parse_min_max_size(val) {
                    if matches!(spec, LengthSpec::Px(0.0) | LengthSpec::Percent(0.0)) {
                        self.allow_shrink = true;
                    }
                    self.min_width = Some(spec);
                }
            }
            "max-width" => {
                let t = val.trim();
                if t.eq_ignore_ascii_case("none") {
                    self.max_width = None;
                } else if t == "100%" {
                    // Unbounded Fill marker (no finite clamp).
                    self.max_width = Some(LengthSpec::Fill);
                } else if let Some(spec) = parse_min_max_size(val) {
                    self.max_width = Some(spec);
                }
            }
            "min-height" => {
                if val.trim() == "100%" {
                    // 定高链：min-height:100% → 参与 Fill 高度传播
                    self.height = Some(self.height.unwrap_or(LengthSpec::Fill));
                    self.min_height = Some(LengthSpec::Px(0.0));
                } else if let Some(spec) = parse_min_max_size(val) {
                    self.min_height = Some(spec);
                }
            }
            "max-height" => {
                let t = val.trim();
                if t.eq_ignore_ascii_case("none") {
                    self.max_height = None;
                } else if let Some(spec) = parse_min_max_size(val) {
                    self.max_height = Some(spec);
                }
            }
            "align-items" => {
                if let Some(a) = AlignSpec::parse(alignment_keyword(val)) {
                    self.align_items = a;
                }
            }
            "align-self" => {
                let kw = alignment_keyword(val);
                if kw.eq_ignore_ascii_case("auto") {
                    self.align_self = None;
                } else if let Some(a) = AlignSpec::parse(kw) {
                    self.align_self = Some(a);
                }
            }
            "align-content" => {
                if let Some(j) = JustifySpec::parse(alignment_keyword(val)) {
                    self.align_content = j;
                }
            }
            "justify-items" => {
                let kw = alignment_keyword(val);
                if kw.eq_ignore_ascii_case("auto") || kw.eq_ignore_ascii_case("legacy") {
                    self.justify_items = None;
                } else if let Some(a) = AlignSpec::parse(kw) {
                    self.justify_items = Some(a);
                }
            }
            "justify-self" => {
                let kw = alignment_keyword(val);
                if kw.eq_ignore_ascii_case("auto") {
                    self.justify_self = None;
                } else if let Some(a) = AlignSpec::parse(kw) {
                    self.justify_self = Some(a);
                }
            }
            "place-items" => {
                // place-items: <align-items> [ <justify-items> ]
                let parts = alignment_tokens(val);
                if let Some(align) = parts.first().and_then(|p| AlignSpec::parse(p)) {
                    self.align_items = align;
                }
                let justify_raw = parts.get(1).copied().or_else(|| parts.first().copied());
                if let Some(j) = justify_raw.and_then(AlignSpec::parse) {
                    self.justify_items = Some(j);
                }
            }
            "place-self" => {
                // place-self: <align-self> [ <justify-self> ]
                let parts = alignment_tokens(val);
                if let Some(align_kw) = parts.first().copied() {
                    if align_kw.eq_ignore_ascii_case("auto") {
                        self.align_self = None;
                    } else if let Some(a) = AlignSpec::parse(align_kw) {
                        self.align_self = Some(a);
                    }
                }
                let justify_raw = parts.get(1).copied().or_else(|| parts.first().copied());
                if let Some(jkw) = justify_raw {
                    if jkw.eq_ignore_ascii_case("auto") {
                        self.justify_self = None;
                    } else if let Some(j) = AlignSpec::parse(jkw) {
                        self.justify_self = Some(j);
                    }
                }
            }
            "place-content" => {
                // place-content: <align-content> [ <justify-content> ]
                // （非 align-items；对照 CSS Box Alignment）
                let parts = alignment_tokens(val);
                if let Some(ac) = parts.first().and_then(|p| JustifySpec::parse(p)) {
                    self.align_content = ac;
                }
                let justify_raw = parts.get(1).copied().or_else(|| parts.first().copied());
                if let Some(j) = justify_raw.and_then(JustifySpec::parse) {
                    self.justify_content = j;
                }
            }
            "justify-content" => {
                if let Some(j) = JustifySpec::parse(alignment_keyword(val)) {
                    self.justify_content = j;
                }
            }
            "overflow" => {
                let parts = split_css_space_tokens(val);
                match parts.as_slice() {
                    [one] => {
                        if let Some(o) = OverflowSpec::parse(one) {
                            self.overflow_x = o;
                            self.overflow_y = o;
                        }
                    }
                    [x, y, ..] => {
                        if let Some(o) = OverflowSpec::parse(x) {
                            self.overflow_x = o;
                        }
                        if let Some(o) = OverflowSpec::parse(y) {
                            self.overflow_y = o;
                        }
                    }
                    _ => {}
                }
            }
            "overflow-x" => {
                if let Some(o) = OverflowSpec::parse(val) {
                    self.overflow_x = o;
                }
            }
            "overflow-y" => {
                if let Some(o) = OverflowSpec::parse(val) {
                    self.overflow_y = o;
                }
            }
            "box-sizing" if val.eq_ignore_ascii_case("border-box") => {
                self.box_sizing = BoxSizing::BorderBox;
            }
            "box-sizing" if val.eq_ignore_ascii_case("content-box") => {
                self.box_sizing = BoxSizing::ContentBox;
            }
            "background"
            | "background-color"
            | "background-image"
            | "background-size"
            | "background-position"
            | "background-repeat"
            | "object-fit"
            | "object-position"
            | "fill"
            | "mask-image"
            | "-webkit-mask-image"
            | "clip-path"
            | "filter"
            | "backdrop-filter"
            | "-webkit-backdrop-filter"
            | "box-shadow"
            | "outline"
            | "outline-width"
            | "outline-style"
            | "outline-color"
            | "mix-blend-mode"
            | "line-clamp"
            | "-webkit-line-clamp"
            | "text-decoration"
            | "text-decoration-line"
            | "font-feature-settings"
            | "font-variation-settings"
            | "pointer-events"
            | "border-image"
            | "border-image-source"
            | "border-image-slice"
            | "border-image-width"
            | "border-image-outset"
            | "border-image-repeat" => {
                crate::css_paint::apply_css_paint_property(self, &key, val);
            }
            "stroke" => {
                if let Some(c) = resolve_paint_color(val) {
                    self.border_color = Some(c);
                    if self.border_width.is_none() {
                        self.border_width = Some(8.0);
                    }
                }
            }
            "stroke-width" => {
                if let Some(v) = parse_css_length_px(val, None) {
                    self.border_width = Some(v.max(0.0));
                }
            }
            "border-radius" => {
                if let Some(corners) = parse_border_radius_shorthand(val) {
                    self.paint.border_radii = Some(corners);
                }
            }
            "text-shadow" => {
                self.paint.text_shadow = parse_text_shadow(val);
            }
            "border-width" => apply_border_width_shorthand(self, val),
            "border-top-width" => {
                if let Some(v) = parse_css_length_px(val, None) {
                    self.border_top_width = Some(v.max(0.0));
                }
            }
            "border-right-width" => {
                if let Some(v) = parse_css_length_px(val, None) {
                    self.border_right_width = Some(v.max(0.0));
                }
            }
            "border-bottom-width" => {
                if let Some(v) = parse_css_length_px(val, None) {
                    self.border_bottom_width = Some(v.max(0.0));
                }
            }
            "border-left-width" => {
                if let Some(v) = parse_css_length_px(val, None) {
                    self.border_left_width = Some(v.max(0.0));
                }
            }
            "border-color" => apply_border_color_shorthand(self, val),
            "border-top-color" => {
                if let Some(c) = crate::style::parse_css_color(val) {
                    self.border_top_color = Some(c);
                }
            }
            "border-right-color" => {
                if let Some(c) = crate::style::parse_css_color(val) {
                    self.border_right_color = Some(c);
                }
            }
            "border-bottom-color" => {
                if let Some(c) = crate::style::parse_css_color(val) {
                    self.border_bottom_color = Some(c);
                }
            }
            "border-left-color" => {
                if let Some(c) = crate::style::parse_css_color(val) {
                    self.border_left_color = Some(c);
                }
            }
            "border-style" => apply_border_style_shorthand(self, val),
            "border-top-style" => apply_border_side_style(self, val, 0),
            "border-right-style" => apply_border_side_style(self, val, 1),
            "border-bottom-style" => apply_border_side_style(self, val, 2),
            "border-left-style" => apply_border_side_style(self, val, 3),
            "border" => apply_border_shorthand(self, val, None),
            "border-top" => apply_border_shorthand(self, val, Some(0)),
            "border-right" => apply_border_shorthand(self, val, Some(1)),
            "border-bottom" => apply_border_shorthand(self, val, Some(2)),
            "border-left" => apply_border_shorthand(self, val, Some(3)),
            "position" => {
                if let Some(p) = PositionSpec::parse(val) {
                    self.position = p;
                }
            }
            "z-index" => {
                let s = val.trim();
                if s.eq_ignore_ascii_case("auto") {
                    self.z_index = None;
                } else if let Ok(z) = s.parse::<i32>() {
                    self.z_index = Some(z);
                }
            }
            "isolation" => {
                let v = val.trim().to_ascii_lowercase();
                match v.as_str() {
                    "isolate" => self.isolation = true,
                    "auto" | "initial" | "unset" => self.isolation = false,
                    _ => {}
                }
            }
            "transform" => {
                if val.trim().eq_ignore_ascii_case("none") {
                    self.transform = None;
                    self.transform_3d = None;
                    self.unsupported_transform = None;
                } else if let Some(parsed) = crate::css_paint_transform::parse_css_transform(val) {
                    match parsed {
                        crate::css_paint_transform::ParsedPaintTransform::Affine(transform) => {
                            self.transform = (!transform.is_identity()).then_some(transform);
                            self.transform_3d = None;
                        }
                        crate::css_paint_transform::ParsedPaintTransform::Mat4(mat4) => {
                            self.transform = None;
                            self.transform_3d = Some(mat4);
                        }
                    }
                    self.unsupported_transform = None;
                } else {
                    self.transform = None;
                    self.transform_3d = None;
                    self.unsupported_transform = Some(val.trim().to_owned());
                }
            }
            "transform-origin" => {
                let v = val.trim();
                if v.eq_ignore_ascii_case("initial") || v.eq_ignore_ascii_case("unset") {
                    self.transform_origin = None;
                } else if let Some(origin) = crate::css_paint_transform::parse_transform_origin(v) {
                    self.transform_origin = Some(origin);
                }
            }
            "transform-box" => {
                if let Some(box_) = crate::css_paint_transform::parse_transform_box(val) {
                    self.transform_box = box_;
                }
            }
            // CSS `perspective` property establishes a parent 3D context we do
            // not paint. Store the length so Scene can fail closed instead of
            // pretending `rotateY` on children used this vanishing point.
            "perspective" => {
                let v = val.trim();
                if v.eq_ignore_ascii_case("none")
                    || v.eq_ignore_ascii_case("initial")
                    || v.eq_ignore_ascii_case("unset")
                {
                    self.css_perspective = None;
                } else if let Some(px) = parse_css_length_px(v, None).filter(|d| d.abs() > 1e-5) {
                    self.css_perspective = Some(px);
                }
            }
            "transform-style" => {
                let v = val.trim().to_ascii_lowercase();
                self.preserve_3d = v == "preserve-3d";
            }
            // `perspective-origin` skipped: parent perspective is fail-closed,
            // so there is no stored vanishing point to offset.
            "perspective-origin" => {}
            "top" => self.offset_top = parse_inset_length(val),
            "right" => {
                self.offset_right = parse_inset_length(val);
                self.logical_inset.set_phys_right(self.offset_right);
            }
            "bottom" => self.offset_bottom = parse_inset_length(val),
            "left" => {
                self.offset_left = parse_inset_length(val);
                self.logical_inset.set_phys_left(self.offset_left);
            }
            "inset" => {
                apply_position_inset_shorthand(
                    val,
                    &mut self.offset_top,
                    &mut self.offset_right,
                    &mut self.offset_bottom,
                    &mut self.offset_left,
                );
                self.logical_inset.set_phys_left(self.offset_left);
                self.logical_inset.set_phys_right(self.offset_right);
            }
            // Logical inset kept until used-value resolve (final `direction`).
            "inset-inline" => {
                apply_logical_inline_edges(val, &mut self.logical_inset, parse_inset_length);
            }
            "inset-block" => apply_logical_pair_shorthand(
                val,
                &mut self.offset_top,
                &mut self.offset_bottom,
                parse_inset_length,
            ),
            "inset-inline-start" => {
                self.logical_inset.set_start(parse_inset_length(val));
            }
            "inset-inline-end" => {
                self.logical_inset.set_end(parse_inset_length(val));
            }
            "inset-block-start" => self.offset_top = parse_inset_length(val),
            "inset-block-end" => self.offset_bottom = parse_inset_length(val),
            "text-overflow" if val.eq_ignore_ascii_case("ellipsis") => {
                self.text_overflow_ellipsis = true;
            }
            "text-overflow" if val.eq_ignore_ascii_case("clip") => {
                self.text_overflow_ellipsis = false;
            }
            "white-space" if val.eq_ignore_ascii_case("nowrap") => {
                self.white_space_nowrap = true;
                self.white_space = WhiteSpaceSpec::Nowrap;
            }
            "white-space" if val.eq_ignore_ascii_case("pre") => {
                self.white_space_nowrap = false;
                self.white_space = WhiteSpaceSpec::Pre;
            }
            "white-space"
                if matches!(
                    val.to_ascii_lowercase().as_str(),
                    "normal" | "wrap" | "pre-wrap" | "pre-line"
                ) =>
            {
                self.white_space_nowrap = false;
                self.white_space = WhiteSpaceSpec::Normal;
            }
            "text-align" => {
                let kw = val.trim().to_ascii_lowercase();
                self.text_align = match kw.as_str() {
                    "start" => TextAlignSpec::Start,
                    "end" => TextAlignSpec::End,
                    "left" => TextAlignSpec::Left,
                    "right" => TextAlignSpec::Right,
                    "center" => TextAlignSpec::Center,
                    _ => self.text_align,
                };
            }
            "direction" => apply_css_direction(self, val),
            "writing-mode" => apply_css_writing_mode(self, val),
            // Fail-closed: do not pretend bidi isolation or IFC bidi.
            "unicode-bidi" => {}
            "float" => {
                let kw = val.trim().to_ascii_lowercase();
                self.float = match kw.as_str() {
                    "left" => FloatSpec::Left,
                    "right" => FloatSpec::Right,
                    "none" => FloatSpec::None,
                    _ => self.float,
                };
            }
            "clear" => {
                let kw = val.trim().to_ascii_lowercase();
                self.clear = match kw.as_str() {
                    "left" => ClearSpec::Left,
                    "right" => ClearSpec::Right,
                    "both" => ClearSpec::Both,
                    "none" => ClearSpec::None,
                    _ => self.clear,
                };
            }
            "font-size" => {
                if let Some(px) = parse_css_font_size(val) {
                    self.font_size = Some(px);
                }
            }
            "font-weight" => {
                if let Some(w) = parse_css_font_weight(val) {
                    self.font_weight = Some(w);
                }
            }
            "font-family" => {
                if let Some(name) = parse_css_font_family(val) {
                    self.font_family = Some(name);
                }
            }
            "line-height" => {
                if let Some(lh) = parse_css_line_height(val) {
                    self.line_height = Some(lh);
                }
            }
            "letter-spacing" => {
                if let Some(px) = parse_css_letter_spacing(val) {
                    self.letter_spacing = Some(px);
                }
            }
            "color" => {
                if let Some(c) = resolve_paint_color(val) {
                    self.color = Some(c);
                }
            }
            "grid-template-columns" => {
                let trimmed = val.trim();
                if trimmed.eq_ignore_ascii_case("none") || trimmed.is_empty() {
                    clear_grid_template_axis(self, true);
                } else if self.display.is_some_and(DisplaySpec::is_flex_container) {
                    // Inert under display:flex — do not author competing tracks.
                } else {
                    apply_grid_template_axis(self, trimmed, percent_w, true);
                }
            }
            "grid-template-rows" => {
                let trimmed = val.trim();
                if trimmed.eq_ignore_ascii_case("none") || trimmed.is_empty() {
                    clear_grid_template_axis(self, false);
                } else if self.display.is_some_and(DisplaySpec::is_flex_container) {
                    // Inert under display:flex — do not author competing tracks.
                } else {
                    apply_grid_template_axis(self, trimmed, percent_h, false);
                }
            }
            // grid-auto-*: parse & store for layout (implicit tracks / auto-placement).
            // Same track grammar as template (including mixed auto-fit / auto-fill).
            "grid-auto-columns" => {
                if self.display.is_some_and(DisplaySpec::is_flex_container) {
                    // inert under flex
                } else {
                    apply_grid_auto_tracks(val, percent_w, &mut self.grid_auto_columns);
                }
            }
            "grid-auto-rows" => {
                if self.display.is_some_and(DisplaySpec::is_flex_container) {
                } else {
                    apply_grid_auto_tracks(val, percent_h, &mut self.grid_auto_rows);
                }
            }
            "grid-auto-flow" => {
                if self.display.is_some_and(DisplaySpec::is_flex_container) {
                } else if let Some(flow) = parse_grid_auto_flow(val) {
                    self.grid_auto_flow = Some(flow);
                }
            }
            "grid-template-areas" => {
                if self.display.is_some_and(DisplaySpec::is_flex_container) {
                } else if let Some(areas) = parse_grid_template_areas(val) {
                    self.grid_template_areas = Some(areas);
                    if self.display.is_none() {
                        self.display = Some(DisplaySpec::Grid);
                    }
                }
            }
            "grid-column" => {
                if let Some((start, end)) = parse_grid_axis_placement(val) {
                    self.grid_placement.column_start = start;
                    self.grid_placement.column_end = end;
                }
            }
            "grid-row" => {
                if let Some((start, end)) = parse_grid_axis_placement(val) {
                    self.grid_placement.row_start = start;
                    self.grid_placement.row_end = end;
                }
            }
            "grid-column-start" => {
                if let Some(line) = parse_grid_line(val) {
                    self.grid_placement.column_start = line;
                }
            }
            "grid-column-end" => {
                if let Some(line) = parse_grid_line(val) {
                    self.grid_placement.column_end = line;
                }
            }
            "grid-row-start" => {
                if let Some(line) = parse_grid_line(val) {
                    self.grid_placement.row_start = line;
                }
            }
            "grid-row-end" => {
                if let Some(line) = parse_grid_line(val) {
                    self.grid_placement.row_end = line;
                }
            }
            "grid-area" => apply_grid_area(&mut self.grid_placement, val),
            "visibility" if val.eq_ignore_ascii_case("hidden") => {
                self.paint.visibility = Some(VisibilitySpec::Hidden);
            }
            "visibility" if val.eq_ignore_ascii_case("visible") => {
                self.paint.visibility = Some(VisibilitySpec::Visible);
            }
            "opacity" => {
                if let Ok(v) = val.trim().parse::<f32>() {
                    self.opacity = Some(v.clamp(0.0, 1.0));
                }
            }
            // Window-level / chrome: fail closed. `cursor` has no CSS→window
            // mapping (winit cursor is chrome resize only). `user-select` has
            // no L1 selection gate. `-webkit-app-region` / `app-region` is
            // Electron caption CSS on arbitrary boxes; Nana drag is only
            // AppTitleBar → nana-window, not a CSS region map.
            "cursor"
            | "user-select"
            | "-webkit-user-select"
            | "-webkit-app-region"
            | "app-region" => {}
            _ => {}
        }
        self.resolve_logical_box_edges();
    }

    fn apply_flex_shorthand(&mut self, val: &str) {
        let trimmed = val.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return;
        }
        if trimmed == "none" {
            self.flex_grow = Some(0.0);
            self.flex_shrink = Some(0.0);
            self.flex_basis = Some(LengthSpec::Auto);
            return;
        }
        if trimmed == "auto" {
            self.flex_grow = Some(1.0);
            self.flex_shrink = Some(1.0);
            self.flex_basis = Some(LengthSpec::Auto);
            return;
        }
        if trimmed == "initial" {
            self.flex_grow = Some(0.0);
            self.flex_shrink = Some(1.0);
            self.flex_basis = Some(LengthSpec::Auto);
            return;
        }
        // CSS `flex` omitted components: grow=1, shrink=1, basis=0.
        // Unspecified *longhand* `flex-shrink` stays `None` (layout treats as 0).
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        match parts.as_slice() {
            [one] => {
                if let Ok(g) = one.parse::<f32>() {
                    self.flex_grow = Some(g.max(0.0));
                    self.flex_shrink = Some(1.0);
                    self.flex_basis = Some(LengthSpec::Px(0.0));
                } else if let Some(basis) = LengthSpec::parse(one) {
                    self.flex_grow = Some(1.0);
                    self.flex_shrink = Some(1.0);
                    self.flex_basis = Some(basis);
                }
            }
            [a, b] => {
                if let Ok(g) = a.parse::<f32>() {
                    self.flex_grow = Some(g.max(0.0));
                    if let Ok(s) = b.parse::<f32>() {
                        self.flex_shrink = Some(s.max(0.0));
                        self.flex_basis = Some(LengthSpec::Px(0.0));
                    } else if let Some(basis) = LengthSpec::parse(b) {
                        self.flex_shrink = Some(1.0);
                        self.flex_basis = Some(basis);
                    }
                } else if let Some(basis) = LengthSpec::parse(a)
                    && let Ok(g) = b.parse::<f32>()
                {
                    self.flex_grow = Some(g.max(0.0));
                    self.flex_shrink = Some(1.0);
                    self.flex_basis = Some(basis);
                }
            }
            [g, s, basis, ..] => {
                if let Ok(g) = g.parse::<f32>() {
                    self.flex_grow = Some(g.max(0.0));
                }
                if let Ok(s) = s.parse::<f32>() {
                    self.flex_shrink = Some(s.max(0.0));
                }
                self.flex_basis = LengthSpec::parse(basis);
            }
            _ => {}
        }
    }

    /// 从 HostValue（string 或 object）应用 style。
    fn apply_host_style(
        &mut self,
        value: &nana_js_engine::HostValue,
        percent_w: Option<f32>,
        percent_h: Option<f32>,
    ) {
        match value {
            nana_js_engine::HostValue::String(s) => {
                self.apply_css_text(s, percent_w, percent_h);
            }
            nana_js_engine::HostValue::Object(map) => {
                let mut pairs: Vec<(String, String)> = Vec::with_capacity(map.len());
                for (k, v) in map {
                    let s = match v {
                        nana_js_engine::HostValue::String(s) => s.clone(),
                        nana_js_engine::HostValue::Number(n) => format!("{n}px"),
                        nana_js_engine::HostValue::Bool(b) => b.to_string(),
                        nana_js_engine::HostValue::Null => continue,
                        other => host_value_debug(other),
                    };
                    pairs.push((k.clone(), s));
                }
                for (k, s) in &pairs {
                    if css_key_is_direction_or_writing_mode(k) {
                        self.apply_css_property(k, s, percent_w, percent_h);
                    }
                }
                for (k, s) in &pairs {
                    if !css_key_is_direction_or_writing_mode(k) {
                        self.apply_css_property(k, s, percent_w, percent_h);
                    }
                }
            }
            nana_js_engine::HostValue::Null => {}
            other => {
                let s = host_value_debug(other);
                if !s.is_empty() {
                    self.apply_css_text(&s, percent_w, percent_h);
                }
            }
        }
    }
}

/// 去掉 Box Alignment 的 `safe` / `unsafe` 前缀，保留对齐关键字序列。
fn alignment_tokens(raw: &str) -> Vec<&str> {
    raw.split_whitespace()
        .filter(|p| !p.eq_ignore_ascii_case("safe") && !p.eq_ignore_ascii_case("unsafe"))
        .collect()
}

/// 单值对齐属性：取第一个有效关键字。
fn alignment_keyword(raw: &str) -> &str {
    let tokens = alignment_tokens(raw);
    tokens.first().copied().unwrap_or(raw.trim())
}

/// `flex-direction`：更新方向，并记录 `*-reverse` → [`LayoutStyle::flex_reverse`]。
fn apply_flex_direction(layout: &mut LayoutStyle, val: &str) {
    let v = val.trim().to_ascii_lowercase();
    match v.as_str() {
        "row" => {
            layout.direction = Some(FlexDirection::Row);
            layout.flex_reverse = false;
        }
        "row-reverse" => {
            layout.direction = Some(FlexDirection::Row);
            layout.flex_reverse = true;
        }
        "column" => {
            layout.direction = Some(FlexDirection::Column);
            layout.flex_reverse = false;
        }
        "column-reverse" => {
            layout.direction = Some(FlexDirection::Column);
            layout.flex_reverse = true;
        }
        _ => {}
    }
}

/// `flex-flow: <flex-direction> || <flex-wrap>`（对照 MDN / CSS Flexible Box）。
fn apply_flex_flow(layout: &mut LayoutStyle, val: &str) {
    for part in val.split_whitespace() {
        let p = part.trim().to_ascii_lowercase();
        match p.as_str() {
            "row" | "row-reverse" | "column" | "column-reverse" => {
                apply_flex_direction(layout, &p);
            }
            "wrap" => layout.flex_wrap = FlexWrap::Wrap,
            "wrap-reverse" => layout.flex_wrap = FlexWrap::WrapReverse,
            "nowrap" => layout.flex_wrap = FlexWrap::NoWrap,
            _ => {}
        }
    }
}

/// Gap 边长：px / `%` / 轻量 calc；保留 [`LengthSpec`]，布局时相对 CB 兑现。
pub fn parse_gap_length(input: &str) -> Option<LengthSpec> {
    let spec = parse_box_edge_length(input)?;
    match spec {
        LengthSpec::Px(px) => Some(LengthSpec::Px(px.max(0.0))),
        LengthSpec::Percent(p) => Some(LengthSpec::Percent(p.clamp(0.0, 100.0))),
        LengthSpec::Em(v) => Some(LengthSpec::Em(v.max(0.0))),
        LengthSpec::Rem(v) => Some(LengthSpec::Rem(v.max(0.0))),
        LengthSpec::CalcPercentOffset { percent, offset_px } => {
            Some(LengthSpec::CalcPercentOffset {
                percent: percent.clamp(0.0, 100.0),
                offset_px,
            })
        }
        LengthSpec::Viewport { .. }
        | LengthSpec::CalcViewportOffset { .. }
        | LengthSpec::CalcEmOffset { .. }
        | LengthSpec::CalcRemOffset { .. }
        | LengthSpec::Min2(_, _)
        | LengthSpec::Max2(_, _)
        | LengthSpec::Clamp3(_, _, _) => Some(spec),
        LengthSpec::Fill
        | LengthSpec::Shrink
        | LengthSpec::Auto
        | LengthSpec::MinContent
        | LengthSpec::MaxContent
        | LengthSpec::FitContent => None,
    }
}

/// `gap` 简写：`10px` → uniform；`10px 20px` → row-gap / column-gap。
/// `%` 不在解析期收成 px（同 margin/padding）。
fn apply_gap_shorthand(val: &str, layout: &mut LayoutStyle) {
    let parts: Vec<_> = val.split_whitespace().collect();
    match parts.len() {
        1 => {
            if let Some(spec) = parse_gap_length(parts[0]) {
                layout.gap = Some(spec);
                // Shorthand resets axis longhands (CSS cascade subset).
                layout.row_gap = None;
                layout.column_gap = None;
            }
        }
        n if n >= 2 => {
            if let (Some(row), Some(col)) = (parse_gap_length(parts[0]), parse_gap_length(parts[1]))
            {
                layout.gap = None;
                layout.row_gap = Some(row);
                layout.column_gap = Some(col);
            }
        }
        _ => {}
    }
}

/// CSS `inset` 1–4 值简写 → top/right/bottom/left（px 或 `%`）。
fn apply_position_inset_shorthand(
    val: &str,
    top: &mut Option<LengthSpec>,
    right: &mut Option<LengthSpec>,
    bottom: &mut Option<LengthSpec>,
    left: &mut Option<LengthSpec>,
) {
    let parts: Vec<_> = val.split_whitespace().collect();
    match parts.len() {
        1 => {
            if let Some(v) = parse_inset_length(parts[0]) {
                *top = Some(v);
                *right = Some(v);
                *bottom = Some(v);
                *left = Some(v);
            }
        }
        2 => {
            if let (Some(y), Some(x)) = (parse_inset_length(parts[0]), parse_inset_length(parts[1]))
            {
                *top = Some(y);
                *bottom = Some(y);
                *left = Some(x);
                *right = Some(x);
            }
        }
        3 => {
            *top = parse_inset_length(parts[0]);
            let x = parse_inset_length(parts[1]);
            *right = x;
            *left = x;
            *bottom = parse_inset_length(parts[2]);
        }
        4 => {
            *top = parse_inset_length(parts[0]);
            *right = parse_inset_length(parts[1]);
            *bottom = parse_inset_length(parts[2]);
            *left = parse_inset_length(parts[3]);
        }
        _ => {
            if let Some(v) = parse_inset_length(parts[0]) {
                *top = Some(v);
                *right = Some(v);
                *bottom = Some(v);
                *left = Some(v);
            }
        }
    }
}

/// `direction` / `writing-mode` must be applied before logical box properties
/// in the same declaration batch so inline start/end map against used dir.
pub(crate) fn css_key_is_direction_or_writing_mode(key: &str) -> bool {
    matches!(
        normalize_css_prop_key(key).as_str(),
        "direction" | "writing-mode"
    )
}

fn normalize_css_prop_key(key: &str) -> String {
    let key = key.trim().replace('_', "-");
    if key.chars().any(|c| c.is_ascii_uppercase()) {
        camel_to_kebab(&key)
    } else {
        key.to_ascii_lowercase()
    }
}

fn apply_css_direction(layout: &mut LayoutStyle, val: &str) {
    let v = val.trim().to_ascii_lowercase();
    match v.as_str() {
        "rtl" => layout.dir = Some(DirSpec::Rtl),
        "ltr" | "initial" => layout.dir = Some(DirSpec::Ltr),
        // Inherited property: `unset` / `inherit` keep the seeded parent value.
        "inherit" | "unset" => {}
        _ => {}
    }
}

/// HTML `dir` presentational hint → CSS `direction` used value.
///
/// `auto` needs first-strong bidi; fail-closed (do not write [`DirSpec::Ltr`]).
pub(crate) fn dir_spec_from_html_attr(raw: &str) -> Option<DirSpec> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "rtl" => Some(DirSpec::Rtl),
        "ltr" => Some(DirSpec::Ltr),
        _ => None,
    }
}

fn apply_css_writing_mode(layout: &mut LayoutStyle, val: &str) {
    let v = val.trim().to_ascii_lowercase();
    match v.as_str() {
        "horizontal-tb" | "initial" | "unset" | "inherit" | "lr-tb" | "lr" => {
            layout.unsupported_writing_mode = false;
        }
        "vertical-rl" | "vertical-lr" | "sideways-rl" | "sideways-lr" | "tb-rl" | "tb" | "bt" => {
            layout.unsupported_writing_mode = true;
        }
        _ => {}
    }
}

/// Store inline-axis logical pair; used left/right come from later resolve.
fn apply_logical_inline_edges(
    val: &str,
    edges: &mut LogicalInlineEdges,
    parse_edge: fn(&str) -> Option<LengthSpec>,
) {
    let parts: Vec<_> = val.split_whitespace().collect();
    match parts.len() {
        1 => {
            if let Some(v) = parse_edge(parts[0]) {
                edges.set_start(Some(v));
                edges.set_end(Some(v));
            }
        }
        n if n >= 2 => {
            edges.set_start(parse_edge(parts[0]));
            edges.set_end(parse_edge(parts[1]));
        }
        _ => {}
    }
}

/// CSS Logical Properties 1–2 值简写：`start` / `end` → 两 physical 边。
/// 单值两边同值；双值分别为 start、end。不改动未映射轴（与 MDN 轴简写一致）。
fn apply_logical_pair_shorthand(
    val: &str,
    start: &mut Option<LengthSpec>,
    end: &mut Option<LengthSpec>,
    parse_edge: fn(&str) -> Option<LengthSpec>,
) {
    let parts: Vec<_> = val.split_whitespace().collect();
    match parts.len() {
        1 => {
            if let Some(v) = parse_edge(parts[0]) {
                *start = Some(v);
                *end = Some(v);
            }
        }
        n if n >= 2 => {
            *start = parse_edge(parts[0]);
            *end = parse_edge(parts[1]);
        }
        _ => {}
    }
}

/// CSS margin/padding 简写：所有边的 `%` 均保留为 [`LengthSpec`]，布局时相对包含块**宽度**。
fn apply_box_edge_shorthand(
    val: &str,
    uniform: &mut Option<LengthSpec>,
    top: &mut Option<LengthSpec>,
    right: &mut Option<LengthSpec>,
    bottom: &mut Option<LengthSpec>,
    left: &mut Option<LengthSpec>,
    parse_edge: fn(&str) -> Option<LengthSpec>,
) {
    let parts: Vec<_> = val.split_whitespace().collect();
    match parts.len() {
        1 => {
            if let Some(v) = parse_edge(parts[0]) {
                *uniform = Some(v);
                *top = None;
                *right = None;
                *bottom = None;
                *left = None;
            }
        }
        2 => {
            if let (Some(y), Some(x)) = (parse_edge(parts[0]), parse_edge(parts[1])) {
                *uniform = None;
                *top = Some(y);
                *bottom = Some(y);
                *left = Some(x);
                *right = Some(x);
            }
        }
        3 => {
            // top | horizontal | bottom — vertical % still uses width at resolve.
            *uniform = None;
            *top = parse_edge(parts[0]);
            let x = parse_edge(parts[1]);
            *right = x;
            *left = x;
            *bottom = parse_edge(parts[2]);
        }
        4 => {
            *uniform = None;
            *top = parse_edge(parts[0]);
            *right = parse_edge(parts[1]);
            *bottom = parse_edge(parts[2]);
            *left = parse_edge(parts[3]);
        }
        _ => {
            if let Some(v) = parse_edge(parts[0]) {
                *uniform = Some(v);
            }
        }
    }
}

/// After clearing a template axis, pick remaining tracks / auto-fill:
/// columns → Row；仅 rows → Column；双边皆空 → `display:grid|inline-grid` 默认 Row（勿残留 Column）。
fn recompute_grid_axis_direction(layout: &mut LayoutStyle) {
    let has_cols = layout.grid_columns.as_ref().is_some_and(|t| !t.is_empty());
    let has_rows = layout.grid_rows.as_ref().is_some_and(|t| !t.is_empty());
    if has_cols {
        layout.direction = Some(FlexDirection::Row);
    } else if has_rows {
        layout.direction = Some(FlexDirection::Column);
    } else if layout.display.is_some_and(DisplaySpec::is_grid_container) {
        layout.direction = Some(FlexDirection::Row);
    }
}

/// 解析结果：支持轨列表 / auto-fit|auto-fill 模式 / 明确 Unsupported / 非法。
#[derive(Debug, Clone, PartialEq)]
pub enum GridTrackListParse {
    Tracks(Vec<GridTrack>),
    /// 整表 `repeat(auto-fit|auto-fill, <track-list>)`；布局按容器展开。
    RepeatAuto(GridRepeatAuto),
    /// 混写 auto-fit/auto-fill 等无法展开的语法；**不是**解析失败、也不是 `none`。
    Unsupported(GridTrackListUnsupported),
    Invalid,
}

fn set_grid_template_axis(
    layout: &mut LayoutStyle,
    columns: bool,
    tracks: Option<Vec<GridTrack>>,
    unsupported: Option<GridTrackListUnsupported>,
    repeat: Option<GridRepeatAuto>,
    names: Option<Vec<Vec<String>>>,
) {
    if columns {
        layout.grid_columns = tracks;
        layout.grid_columns_unsupported = unsupported;
        layout.grid_columns_repeat = repeat;
        layout.grid_column_line_names = names;
    } else {
        layout.grid_rows = tracks;
        layout.grid_rows_unsupported = unsupported;
        layout.grid_rows_repeat = repeat;
        layout.grid_row_line_names = names;
    }
}

fn clear_grid_template_axis(layout: &mut LayoutStyle, columns: bool) {
    set_grid_template_axis(layout, columns, None, None, None, None);
    recompute_grid_axis_direction(layout);
}

/// Apply `grid-template-columns` (`columns=true`) or `grid-template-rows`.
///
/// 固定轨写入 `grid_columns` / `grid_rows`。整表 / 混写 `repeat(auto-fit|auto-fill, …)`
/// 写入 `grid_*_repeat`（布局展开）。成功的 auto-fit / auto-fill **不**置 unsupported。
/// 无法解析的值清空该轴，避免留下旧模板。
fn apply_grid_template_axis(
    layout: &mut LayoutStyle,
    raw: &str,
    percent_base: Option<f32>,
    columns: bool,
) {
    match parse_grid_track_list_result(raw, percent_base) {
        GridTrackListParse::Tracks(tracks) => {
            set_grid_template_axis(
                layout,
                columns,
                Some(tracks),
                None,
                None,
                parse_grid_line_names(raw),
            );
            if layout.display.is_none() {
                layout.display = Some(DisplaySpec::Grid);
            }
            recompute_grid_axis_direction(layout);
        }
        GridTrackListParse::RepeatAuto(mut rep) => {
            attach_repeat_line_name_patterns(&mut rep, raw);
            set_grid_template_axis(
                layout,
                columns,
                None,
                None,
                Some(rep),
                parse_grid_line_names(raw),
            );
            if layout.display.is_none() {
                layout.display = Some(DisplaySpec::Grid);
            }
            recompute_grid_axis_direction(layout);
        }
        GridTrackListParse::Unsupported(unsup) => {
            // Explicit Unsupported — clear this axis's tracks (value is not a
            // supported list) but record why; do not pretend the property was absent.
            set_grid_template_axis(layout, columns, None, Some(unsup), None, None);
            if layout.display.is_none() {
                layout.display = Some(DisplaySpec::Grid);
            }
            recompute_grid_axis_direction(layout);
        }
        GridTrackListParse::Invalid => {
            // Do not keep a previous template when the new value fails to parse
            // (`80px repeat(auto-fit, garbage)` must not silently stay `80px`).
            set_grid_template_axis(layout, columns, None, None, None, None);
        }
    }
}

/// Parse `grid-auto-columns` / `grid-auto-rows`（存储，供布局消费）。
fn apply_grid_auto_tracks(raw: &str, percent_base: Option<f32>, dest: &mut Option<Vec<GridTrack>>) {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("auto") {
        *dest = Some(vec![GridTrack::Auto]);
        return;
    }
    if trimmed.eq_ignore_ascii_case("none") {
        *dest = None;
        return;
    }
    match parse_grid_track_list_result(trimmed, percent_base) {
        GridTrackListParse::Tracks(tracks) => *dest = Some(tracks),
        // auto-fit/fill on auto tracks: leave unchanged.
        GridTrackListParse::RepeatAuto(_)
        | GridTrackListParse::Unsupported(_)
        | GridTrackListParse::Invalid => {}
    }
}

fn parse_grid_auto_flow(raw: &str) -> Option<GridAutoFlow> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "row" => Some(GridAutoFlow::Row),
        "column" => Some(GridAutoFlow::Column),
        "dense" | "row dense" => Some(GridAutoFlow::RowDense),
        "column dense" => Some(GridAutoFlow::ColumnDense),
        _ => None,
    }
}

fn is_css_ident(raw: &str) -> bool {
    let mut chars = raw.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first.is_ascii_digit() || first == '-' && raw[1..].starts_with(|c: char| c.is_ascii_digit())
    {
        return false;
    }
    let ident_char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii();
    ident_char(first) && chars.all(ident_char)
}

fn parse_grid_template_areas(raw: &str) -> Option<GridTemplateAreas> {
    let mut cells = Vec::new();
    let mut rest = raw.trim();
    if rest.eq_ignore_ascii_case("none") || rest.is_empty() {
        return None;
    }
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if !rest.starts_with('"') && !rest.starts_with('\'') {
            return None;
        }
        let quote = rest.as_bytes()[0] as char;
        rest = &rest[1..];
        let end = rest.find(quote)?;
        let row: Vec<String> = rest[..end]
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        if row.is_empty() {
            return None;
        }
        if let Some(width) = cells.first().map(|r: &Vec<String>| r.len())
            && row.len() != width
        {
            return None;
        }
        cells.push(row);
        rest = rest[end + 1..].trim_start();
    }
    if cells.is_empty() {
        return None;
    }
    Some(GridTemplateAreas { cells })
}

fn parse_repeat_line_name_count(count_raw: &str) -> usize {
    let count = count_raw.trim();
    if count.eq_ignore_ascii_case("auto-fit") || count.eq_ignore_ascii_case("auto-fill") {
        return 1;
    }
    count
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=64).contains(n))
        .unwrap_or(1)
}

fn split_auto_repeat_segments(raw: &str) -> Option<(&str, &str, &str)> {
    let mut rest = raw;
    while !rest.is_empty() {
        rest = rest.trim_start();
        let offset = raw.len() - rest.len();
        if let Some(after) = strip_prefix_ci(rest, "repeat(")
            && let Some((inner, suffix)) = split_paren_inner(after)
            && let Some((count_raw, pattern)) = inner.split_once(',')
        {
            let count = count_raw.trim();
            if count.eq_ignore_ascii_case("auto-fit") || count.eq_ignore_ascii_case("auto-fill") {
                return Some((raw[..offset].trim(), pattern.trim(), suffix.trim()));
            }
        }
        if rest.starts_with('[') {
            let end = rest.find(']')? + 1;
            rest = &rest[end..];
            continue;
        }
        let token_end = rest
            .find(|c: char| c.is_whitespace() || c == '[')
            .unwrap_or(rest.len());
        if token_end == 0 {
            break;
        }
        rest = &rest[token_end..];
    }
    None
}

fn attach_repeat_line_name_patterns(rep: &mut GridRepeatAuto, raw: &str) {
    let Some((prefix, pattern, suffix)) = split_auto_repeat_segments(raw) else {
        return;
    };
    if let Some(names) = parse_grid_line_names(prefix) {
        rep.prefix_line_names = names;
    }
    if let Some(names) = parse_grid_line_names(pattern) {
        rep.pattern_line_names = names;
    }
    if let Some(names) = parse_grid_line_names(suffix) {
        rep.suffix_line_names = names;
    }
}

fn parse_grid_line_names(raw: &str) -> Option<Vec<Vec<String>>> {
    // n tracks ⇒ n+1 lines. `[name]` attaches to the current line; a track
    // token opens the next line.
    let mut names = vec![Vec::new()];
    let mut rest = raw.trim();
    let mut saw = false;
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if rest.starts_with('[') {
            let end = rest.find(']')?;
            let group: Vec<String> = rest[1..end]
                .split_whitespace()
                .filter(|s| is_css_ident(s))
                .map(|s| s.to_string())
                .collect();
            names
                .last_mut()
                .expect("line table starts with line 1")
                .extend(group);
            saw = true;
            rest = rest[end + 1..].trim_start();
            continue;
        }
        if let Some(after) = strip_prefix_ci(rest, "repeat(") {
            if let Some((inner, next)) = split_paren_inner(after) {
                if let Some((count_raw, pattern)) = inner.split_once(',')
                    && let Some(inner_names) = parse_grid_line_names(pattern)
                {
                    let reps = parse_repeat_line_name_count(count_raw);
                    let inner_names = GridRepeatAuto::merge_line_name_pattern(&inner_names, reps);
                    names = GridRepeatAuto::join_line_name_lists(&names, &inner_names);
                    saw = true;
                }
                rest = next.trim_start();
                continue;
            }
            break;
        }
        let token_end = rest
            .find(|c: char| c.is_whitespace() || c == '[')
            .unwrap_or(rest.len());
        if token_end == 0 {
            break;
        }
        names.push(Vec::new());
        rest = rest[token_end..].trim_start();
    }
    if !saw {
        return None;
    }
    Some(names)
}

/// CSS `<grid-line>` subset: `auto` / integer / `span N` / custom-ident.
fn parse_grid_line(raw: &str) -> Option<GridLine> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if s.eq_ignore_ascii_case("auto") {
        return Some(GridLine::Auto);
    }
    if let Some(rest) = strip_prefix_ci(s, "span") {
        let rest = rest.trim();
        if rest.is_empty() {
            return Some(GridLine::Span(1));
        }
        if let Some(n) = rest
            .split_whitespace()
            .find_map(|part| part.parse::<u16>().ok())
        {
            if n >= 1 {
                return Some(GridLine::Span(n));
            }
            return None;
        }
        // `span name` — treat as span 1 of that named line (name kept).
        if is_css_ident(rest) {
            return Some(GridLine::Name(rest.to_string()));
        }
        return None;
    }
    if let Ok(n) = s.parse::<i32>() {
        if n == 0 {
            return None;
        }
        return Some(GridLine::Index(n));
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    match parts.as_slice() {
        [ident] if is_css_ident(ident) => Some(GridLine::Name((*ident).to_string())),
        [ident, n] if is_css_ident(ident) => {
            let occ = n.parse::<u16>().ok().filter(|v| *v >= 1)?;
            Some(if occ == 1 {
                GridLine::Name((*ident).to_string())
            } else {
                GridLine::NthName((*ident).to_string(), occ)
            })
        }
        [n, ident] if is_css_ident(ident) => {
            let occ = n.parse::<u16>().ok().filter(|v| *v >= 1)?;
            Some(if occ == 1 {
                GridLine::Name((*ident).to_string())
            } else {
                GridLine::NthName((*ident).to_string(), occ)
            })
        }
        _ => None,
    }
}

/// `grid-column` / `grid-row`: `<grid-line> [ / <grid-line> ]?`.
/// Omitted end is `auto`. `none` / empty / unparsed → `None` (leave unchanged).
fn parse_grid_axis_placement(raw: &str) -> Option<(GridLine, GridLine)> {
    let s = raw.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        return None;
    }
    if let Some((start_raw, end_raw)) = s.split_once('/') {
        Some((parse_grid_line(start_raw)?, parse_grid_line(end_raw)?))
    } else {
        Some((parse_grid_line(s)?, GridLine::Auto))
    }
}

/// `grid-area`: 命名区域，或 row-start / column-start / row-end / column-end。
fn apply_grid_area(placement: &mut GridPlacement, raw: &str) {
    let s = raw.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        return;
    }
    if !s.contains('/') && is_css_ident(s) && s.parse::<i32>().is_err() {
        placement.area = Some(s.to_string());
        placement.row_start = GridLine::Auto;
        placement.column_start = GridLine::Auto;
        placement.row_end = GridLine::Auto;
        placement.column_end = GridLine::Auto;
        return;
    }
    let parts: Vec<&str> = s.split('/').collect();
    if parts.is_empty() || parts.len() > 4 {
        return;
    }
    let Some(lines) = parts
        .iter()
        .map(|p| parse_grid_line(p))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    placement.area = None;
    match lines.as_slice() {
        [row_start] => {
            placement.row_start = row_start.clone();
            placement.column_start = GridLine::Auto;
            placement.row_end = GridLine::Auto;
            placement.column_end = GridLine::Auto;
        }
        [row_start, column_start] => {
            placement.row_start = row_start.clone();
            placement.column_start = column_start.clone();
            placement.row_end = GridLine::Auto;
            placement.column_end = GridLine::Auto;
        }
        [row_start, column_start, row_end] => {
            placement.row_start = row_start.clone();
            placement.column_start = column_start.clone();
            placement.row_end = row_end.clone();
            placement.column_end = GridLine::Auto;
        }
        [row_start, column_start, row_end, column_end] => {
            placement.row_start = row_start.clone();
            placement.column_start = column_start.clone();
            placement.row_end = row_end.clone();
            placement.column_end = column_end.clone();
        }
        _ => {}
    }
}

/// 解析 `grid-template-columns` / `rows`（及 `grid-auto-*` 轨表）轻量子集。
///
/// 支持：`px` / `%` / `fr` / `auto` / `max-content`/`min-content`/`fit-content`、
/// `fit-content(<length-percentage>)`、`minmax(min, Nfr|px|%|auto|*-content)`、
/// `repeat(N, …)`（固定次数）。
///
/// 整表或混写 `repeat(auto-fit|auto-fill, <track-list>)` →
/// [`GridTrackListParse::RepeatAuto`]（`prefix` / pattern / `suffix`）。
/// 嵌套 auto-fit / auto-fill 或无法展开的 pattern → [`GridTrackListParse::Unsupported`]。
/// `percent_base` 用于把轨上的 `%` 收成 px。
///
/// 未解析的 `var(--token)`（无 lookup / 无 fallback）在该轨位置降级为
/// [`GridTrack::Auto`]，**不得**从列表删除（否则列数错位）。
pub fn parse_grid_track_list_result(raw: &str, percent_base: Option<f32>) -> GridTrackListParse {
    let s = raw.trim();
    if s.is_empty() {
        return GridTrackListParse::Invalid;
    }
    let normalized = expand_css_vars_for_grid_tracks(s);
    parse_grid_track_list(normalized.trim(), percent_base)
}

/// 兼容入口：仅成功轨列表；Unsupported / Invalid → `None`。
///
/// 需要区分 Unsupported 时请用 [`parse_grid_track_list_result`]。
pub fn parse_grid_template_columns(raw: &str, percent_base: Option<f32>) -> Option<Vec<GridTrack>> {
    match parse_grid_track_list_result(raw, percent_base) {
        GridTrackListParse::Tracks(tracks) if !tracks.is_empty() => Some(tracks),
        _ => None,
    }
}

fn parse_grid_track_list(raw: &str, percent_base: Option<f32>) -> GridTrackListParse {
    let mut tracks = Vec::new();
    let mut rest = raw;
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if is_subgrid_track_token(rest) {
            return GridTrackListParse::Unsupported(GridTrackListUnsupported::Subgrid);
        }
        // repeat(N, tracks…) — fixed N expands；整表 auto-fit/fill 保留 pattern。
        if let Some(after) = strip_prefix_ci(rest, "repeat(") {
            let Some((inner, next)) = split_paren_inner(after) else {
                return GridTrackListParse::Invalid;
            };
            let Some((count_raw, pattern)) = inner.split_once(',') else {
                return GridTrackListParse::Invalid;
            };
            let count_raw = count_raw.trim();
            if count_raw.eq_ignore_ascii_case("auto-fit")
                || count_raw.eq_ignore_ascii_case("auto-fill")
            {
                let kind = if count_raw.eq_ignore_ascii_case("auto-fit") {
                    GridTrackListUnsupported::RepeatAutoFit
                } else {
                    GridTrackListUnsupported::RepeatAutoFill
                };
                let unit = match parse_grid_track_list(pattern.trim(), percent_base) {
                    GridTrackListParse::Tracks(unit) if !unit.is_empty() => unit,
                    GridTrackListParse::RepeatAuto(_) | GridTrackListParse::Unsupported(_) => {
                        return GridTrackListParse::Unsupported(kind);
                    }
                    GridTrackListParse::Invalid | GridTrackListParse::Tracks(_) => {
                        return GridTrackListParse::Invalid;
                    }
                };
                let suffix = match parse_grid_track_list(next.trim(), percent_base) {
                    GridTrackListParse::Tracks(extra) => extra,
                    GridTrackListParse::Invalid if next.trim().is_empty() => Vec::new(),
                    GridTrackListParse::RepeatAuto(_) | GridTrackListParse::Unsupported(_) => {
                        return GridTrackListParse::Unsupported(kind);
                    }
                    GridTrackListParse::Invalid => {
                        return GridTrackListParse::Invalid;
                    }
                };
                return GridTrackListParse::RepeatAuto(GridRepeatAuto {
                    kind,
                    tracks: unit,
                    prefix: tracks,
                    suffix,
                    ..Default::default()
                });
            }
            let Ok(count) = count_raw.parse::<usize>() else {
                return GridTrackListParse::Invalid;
            };
            if count == 0 || count > 64 {
                return GridTrackListParse::Invalid;
            }
            match parse_grid_track_list(pattern.trim(), percent_base) {
                GridTrackListParse::Tracks(unit) if !unit.is_empty() => {
                    for _ in 0..count {
                        tracks.extend(unit.iter().copied());
                    }
                }
                other @ (GridTrackListParse::Unsupported(_) | GridTrackListParse::Invalid) => {
                    return other;
                }
                GridTrackListParse::RepeatAuto(rep) => {
                    return GridTrackListParse::Unsupported(rep.kind);
                }
                GridTrackListParse::Tracks(_) => return GridTrackListParse::Invalid,
            }
            rest = next;
            continue;
        }
        // fit-content(<length-percentage>) — MDN function form.
        if let Some(after) = strip_prefix_ci(rest, "fit-content(") {
            let Some((inner, next)) = split_paren_inner(after) else {
                return GridTrackListParse::Invalid;
            };
            let inner = inner.trim();
            let track = if inner.is_empty()
                || inner.eq_ignore_ascii_case("auto")
                || is_content_sized_keyword(inner)
            {
                GridTrack::Auto
            } else if let Some(px) = parse_css_length_px(inner, percent_base) {
                // ≈ minmax(auto, <arg>)：柔性轨 + 像素上限。
                GridTrack::MinMax {
                    min_px: 0.0,
                    fr: 1.0,
                    max_px: Some(px.max(0.0)),
                }
            } else {
                return GridTrackListParse::Invalid;
            };
            tracks.push(track);
            rest = next;
            continue;
        }
        if let Some(after) = strip_prefix_ci(rest, "minmax(") {
            let Some((inner, next)) = split_paren_inner(after) else {
                return GridTrackListParse::Invalid;
            };
            let Some((min_raw, max_raw)) = inner.split_once(',') else {
                return GridTrackListParse::Invalid;
            };
            let min_raw = min_raw.trim();
            let max_raw = max_raw.trim();
            let min_px = parse_grid_track_min_px(min_raw, percent_base);
            let Some(track) = parse_grid_minmax_max(min_px, max_raw, percent_base) else {
                return GridTrackListParse::Invalid;
            };
            tracks.push(track);
            rest = next;
            continue;
        }
        if rest.starts_with('[') {
            let Some(end) = rest.find(']') else {
                return GridTrackListParse::Invalid;
            };
            rest = rest[end + 1..].trim_start();
            continue;
        }
        let token_end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        let token = &rest[..token_end];
        rest = &rest[token_end..];
        let Some(track) = parse_grid_single_track(token, percent_base) else {
            return GridTrackListParse::Invalid;
        };
        tracks.push(track);
    }
    if tracks.is_empty() {
        GridTrackListParse::Invalid
    } else {
        GridTrackListParse::Tracks(tracks)
    }
}

fn parse_grid_track_min_px(min_raw: &str, percent_base: Option<f32>) -> f32 {
    if min_raw == "0"
        || min_raw == "0px"
        || is_content_sized_keyword(min_raw)
        || min_raw.eq_ignore_ascii_case("auto")
    {
        0.0
    } else {
        parse_css_length_px(min_raw, percent_base).unwrap_or(0.0)
    }
}

fn parse_grid_minmax_max(
    min_px: f32,
    max_raw: &str,
    percent_base: Option<f32>,
) -> Option<GridTrack> {
    if let Some(fr) = parse_fr(max_raw) {
        return Some(GridTrack::MinMax {
            min_px,
            fr,
            max_px: None,
        });
    }
    if max_raw.eq_ignore_ascii_case("auto") || is_content_sized_keyword(max_raw) {
        // minmax(min, auto|*-content) → flexible track with min floor（≈ minmax(min,1fr)）。
        return Some(GridTrack::MinMax {
            min_px,
            fr: 1.0,
            max_px: None,
        });
    }
    if let Some(max_px) = parse_css_length_px(max_raw, percent_base) {
        return Some(GridTrack::MinMax {
            min_px,
            fr: 1.0,
            max_px: Some(max_px.max(min_px)),
        });
    }
    None
}

fn parse_grid_single_track(token: &str, percent_base: Option<f32>) -> Option<GridTrack> {
    let token = token.trim();
    if token.eq_ignore_ascii_case("auto") || is_content_sized_keyword(token) {
        Some(GridTrack::Auto)
    } else if token.to_ascii_lowercase().starts_with("var(") {
        // Defense: unresolved var track → Auto (never drop the slot).
        Some(GridTrack::Auto)
    } else if let Some(fr) = parse_fr(token) {
        Some(GridTrack::Fr(fr))
    } else if let Some(p) = token.strip_suffix('%') {
        // Preserve `%` without CB (parity / cascade often parse before measure).
        let pct = p.trim().parse::<f32>().ok()?;
        Some(GridTrack::Percent(pct.clamp(0.0, 100.0)))
    } else {
        parse_css_length_px(token, percent_base).map(GridTrack::Px)
    }
}

fn is_content_sized_keyword(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "max-content" | "min-content" | "fit-content"
    )
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn is_subgrid_track_token(rest: &str) -> bool {
    let Some(after) = strip_prefix_ci(rest, "subgrid") else {
        return false;
    };
    match after.chars().next() {
        None => true,
        Some(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_' => false,
        Some(_) => true,
    }
}

/// Split `inner…)rest` where `inner` is balanced relative to an already-consumed `(`.
fn split_paren_inner(after_open: &str) -> Option<(&str, &str)> {
    let mut depth = 1i32;
    for (i, ch) in after_open.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&after_open[..i], &after_open[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_fr(raw: &str) -> Option<f32> {
    let s = raw.trim();
    let n = s.strip_suffix("fr")?.trim().parse::<f32>().ok()?;
    Some(n.max(0.0))
}

// Stylesheet-authored custom properties (`--name: value`) for `var(--name)` lookup.
// Set by [`with_active_css_vars`] around cascade rebuilds. Document-level base
// plus per-element inheritance is assembled by the bridge before install.
thread_local! {
    static ACTIVE_CSS_VARS: std::cell::RefCell<std::collections::BTreeMap<String, String>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    static ACTIVE_VIEWPORT: std::cell::RefCell<Option<(f32, f32)>> =
        const { std::cell::RefCell::new(None) };
    static ACTIVE_FONT_SIZES: std::cell::RefCell<FontSizeContext> =
        std::cell::RefCell::new(FontSizeContext::default());
    /// Prefer `light-dark()` dark branch when true (document `data-theme=dark`).
    static ACTIVE_COLOR_SCHEME_DARK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether `light-dark(light, dark)` should pick the dark argument.
pub fn active_color_scheme_is_dark() -> bool {
    ACTIVE_COLOR_SCHEME_DARK.with(|cell| cell.get())
}

/// Install color-scheme for `light-dark()` resolve during `f`.
pub fn with_active_color_scheme_dark<R>(dark: bool, f: impl FnOnce() -> R) -> R {
    ACTIVE_COLOR_SCHEME_DARK.with(|cell| {
        let previous = cell.replace(dark);
        let out = f();
        cell.set(previous);
        out
    })
}

/// Install stylesheet custom properties for the duration of `f`.
pub fn with_active_css_vars<R>(
    vars: &std::collections::BTreeMap<String, String>,
    f: impl FnOnce() -> R,
) -> R {
    ACTIVE_CSS_VARS.with(|cell| {
        let previous = cell.replace(vars.clone());
        let out = f();
        *cell.borrow_mut() = previous;
        out
    })
}

/// Install viewport size for `vw`/`vh`/`vmin`/`vmax` / `min()` resolve during `f`.
pub fn with_active_viewport<R>(viewport_w: f32, viewport_h: f32, f: impl FnOnce() -> R) -> R {
    ACTIVE_VIEWPORT.with(|cell| {
        let previous = cell.replace(Some((viewport_w.max(0.0), viewport_h.max(0.0))));
        let out = f();
        *cell.borrow_mut() = previous;
        out
    })
}

/// Currently installed viewport for length resolve (`None` if unset).
pub fn active_viewport() -> Option<(f32, f32)> {
    ACTIVE_VIEWPORT.with(|cell| *cell.borrow())
}

/// Install `em`/`rem` font-size context for the duration of `f`.
pub fn with_active_font_sizes<R>(fonts: FontSizeContext, f: impl FnOnce() -> R) -> R {
    ACTIVE_FONT_SIZES.with(|cell| {
        let previous = cell.replace(fonts);
        let out = f();
        *cell.borrow_mut() = previous;
        out
    })
}

/// Currently installed font-size context for `em`/`rem`.
pub fn active_font_sizes() -> FontSizeContext {
    ACTIVE_FONT_SIZES.with(|cell| *cell.borrow())
}

/// Extract `--name: value` declarations from a declaration block (not a full sheet).
///
/// Trailing `!important` is stripped the same way [`LayoutStyleCss::apply_css_property`]
/// does, so `var(--gap)` receives `8px` rather than `8px !important`.
pub fn extract_css_custom_properties_from_decls(
    decls: &str,
) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    for_each_css_decl(decls, |key, raw_val| {
        if let Some(name) = key.strip_prefix("--")
            && !name.is_empty()
        {
            let (value, _) = split_important_flag(raw_val);
            if !value.is_empty() {
                map.insert(format!("--{name}"), value);
            }
        }
    });
    map
}

/// Merge `overlay` into `base` (overlay wins), then resolve nested `var()` / simple calc.
pub fn merge_css_custom_properties(
    base: &std::collections::BTreeMap<String, String>,
    overlay: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    let mut map = base.clone();
    for (k, v) in overlay {
        map.insert(k.clone(), v.clone());
    }
    resolve_css_custom_property_map(&mut map);
    map
}

/// Collect document-level custom properties (`:root` / `html` / `body` / `*`).
///
/// Theme-conditional selectors (`:root[data-theme=…]`, `[data-theme=…]`, …)
/// are included only when they match `theme` (`light` / `dark`). Unconditional
/// document selectors always apply. Selector lists are OR (any match includes
/// the block), matching CSS. Source order; last write wins.
///
/// Skips `@supports` / `@media` / other at-rules so LightningCSS P3/lab
/// fallbacks cannot clobber the hex/`oklch` author layer with unparsed
/// `color(display-p3 …)` values.
pub fn collect_document_css_custom_properties(
    css: &str,
    theme: &str,
) -> std::collections::BTreeMap<String, String> {
    let stripped = strip_css_comments_local(css);
    let mut map = std::collections::BTreeMap::new();
    let mut rest = stripped.as_str();
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if rest.starts_with('@') {
            rest = skip_at_rule_local(rest);
            continue;
        }
        let Some((selector, body, next)) = split_rule_local(rest) else {
            break;
        };
        rest = next;
        if !document_level_selector_applies(selector, theme) {
            continue;
        }
        for (k, v) in extract_css_custom_properties_from_decls(body) {
            map.insert(k, v);
        }
    }
    resolve_css_custom_property_map(&mut map);
    map
}

fn skip_at_rule_local(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] != b'{' && bytes[i] != b';' {
        i += 1;
    }
    if i >= bytes.len() {
        return "";
    }
    if bytes[i] == b';' {
        return &s[i + 1..];
    }
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &s[i + 1..];
                }
            }
            _ => {}
        }
        i += 1;
    }
    ""
}

fn split_rule_local(s: &str) -> Option<(&str, &str, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut depth_br = 0i32;
    let mut depth_paren = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => depth_br += 1,
            b']' => depth_br -= 1,
            b'(' => depth_paren += 1,
            b')' => depth_paren -= 1,
            b'{' if depth_br == 0 && depth_paren == 0 => break,
            _ => {}
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let selector = s[..i].trim();
    i += 1;
    let body_start = i;
    let mut depth = 1i32;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((selector, &s[body_start..i], &s[i + 1..]));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn is_document_level_selector_atom(sel: &str) -> bool {
    let s = sel.trim().to_ascii_lowercase();
    s == ":root"
        || s == "html"
        || s == "body"
        || s == "*"
        || s.starts_with(":root[")
        || s.starts_with("html[")
        || s.starts_with("body[")
        || s.starts_with(":root:")
        || s.starts_with("[data-theme")
        || s.starts_with(":root ")
}

fn document_level_selector_applies(selector_list: &str, theme: &str) -> bool {
    let theme = theme.trim();
    selector_list.split(',').any(|sel| {
        if !is_document_level_selector_atom(sel) {
            return false;
        }
        match data_theme_constraint(sel) {
            Some(want) => want.eq_ignore_ascii_case(theme),
            None => true,
        }
    })
}

/// `Some(theme)` when the atom requires `data-theme=…`; `None` when unconstrained.
fn data_theme_constraint(sel: &str) -> Option<String> {
    let lower = sel.trim().to_ascii_lowercase();
    let key = "data-theme";
    let idx = lower.find(key)?;
    let after = lower[idx + key.len()..].trim_start();
    if after.starts_with(']') {
        // `[data-theme]` presence-only — treat as unconstrained for document scrape.
        return None;
    }
    if !after.starts_with('=') {
        return None;
    }
    let mut value = after[1..].trim_start();
    let quote = value.chars().next();
    if quote == Some('"') || quote == Some('\'') {
        value = &value[1..];
        let end = value.find(quote.unwrap()).unwrap_or(value.len());
        return Some(value[..end].to_string());
    }
    let end = value
        .find(|c: char| c == ']' || c.is_whitespace())
        .unwrap_or(value.len());
    Some(value[..end].to_string())
}

/// Collect `--name: value` declarations from stylesheet text (source order; last wins).
///
/// Prefer [`collect_document_css_custom_properties`] for inheritance bases; this
/// flat scrape remains for tests / legacy callers.
#[cfg(test)]
pub fn collect_css_custom_properties(css: &str) -> std::collections::BTreeMap<String, String> {
    let stripped = strip_css_comments_local(css);
    let mut map = std::collections::BTreeMap::new();
    for block in stripped.split('}') {
        let Some((_, body)) = block.split_once('{') else {
            continue;
        };
        for (k, v) in extract_css_custom_properties_from_decls(body) {
            map.insert(k, v);
        }
    }
    resolve_css_custom_property_map(&mut map);
    map
}

fn strip_css_comments_local(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Iteratively expand `var(--x)` / foldable `calc()` inside the custom-prop map.
fn resolve_css_custom_property_map(map: &mut std::collections::BTreeMap<String, String>) {
    for _ in 0..8 {
        let snapshot = map.clone();
        let mut changed = false;
        for value in map.values_mut() {
            let expanded = expand_css_vars_with_lookup(value, &snapshot);
            let simplified = simplify_simple_calc(&expanded);
            if simplified != *value {
                *value = simplified;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

fn simplify_simple_calc(input: &str) -> String {
    let s = input.trim();
    if !starts_with_ci(s, "calc(") {
        return input.to_string();
    }
    // Same engine as LengthSpec::parse — fold only context-free results.
    match LengthSpec::parse(s) {
        Some(LengthSpec::Px(px)) => format!("{px}px"),
        Some(LengthSpec::Em(v)) => format!("{v}em"),
        Some(LengthSpec::Rem(v)) => format!("{v}rem"),
        _ => input.to_string(),
    }
}

/// CSS custom-property `initial` computes to the guaranteed-invalid value, so
/// `var(--x, fallback)` must use the fallback. LightningCSS encodes
/// `light-dark(A, B)` as `var(--lightningcss-light, A)var(--lightningcss-dark, B)`
/// and toggles one side to `initial` per theme — without this, heatmap fills
/// collapse to `"initial"` and sticky cascade paint can leave dark `#1c1c1c`.
fn is_guaranteed_invalid_custom_prop_value(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("initial")
}

fn expand_css_vars_with_lookup(
    input: &str,
    lookup: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("var(") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 4..];
        let Some((inner, next)) = split_paren_inner(after) else {
            out.push_str(rest);
            return out;
        };
        let (name_part, fallback) = match inner.split_once(',') {
            Some((n, fb)) => (n.trim(), Some(fb.trim())),
            None => (inner.trim(), None),
        };
        if let Some(resolved) = lookup.get(name_part) {
            if is_guaranteed_invalid_custom_prop_value(resolved) {
                if let Some(fb) = fallback {
                    out.push_str(fb);
                }
            } else {
                out.push_str(resolved);
            }
        } else if let Some(fb) = fallback {
            out.push_str(fb);
        }
        rest = next;
    }
    out.push_str(rest);
    out
}

fn expand_css_var_fallback(input: &str) -> String {
    // 1) stylesheet custom-prop lookup  2) var(--name, fallback) → fallback
    // Unresolved var() without fallback is removed (single-value props fail closed).
    // Grid track lists must use [`expand_css_vars_for_grid_tracks`] instead.
    expand_css_vars_with_unresolved(input, None)
}

fn for_each_css_decl(style: &str, mut visit: impl FnMut(&str, &str)) {
    for decl in style.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let Some((raw_key, raw_val)) = decl.split_once(':') else {
            continue;
        };
        visit(raw_key.trim(), raw_val.trim());
    }
}

/// Parse trailing `!important` (case-insensitive; whitespace around `!` / ident).
///
/// Returns `(value without flag, is_important)`.
pub(crate) fn split_important_flag(value: &str) -> (String, bool) {
    let trimmed = value.trim();
    let Some(bang) = trimmed.rfind('!') else {
        return (trimmed.to_string(), false);
    };
    let after = trimmed[bang + 1..].trim();
    if after.eq_ignore_ascii_case("important") {
        return (trimmed[..bang].trim_end().to_string(), true);
    }
    (trimmed.to_string(), false)
}

/// Expand `var()` for grid track lists: unresolved tokens become `auto` so the
/// track slot remains (never delete a column/row from the list).
fn expand_css_vars_for_grid_tracks(input: &str) -> String {
    expand_css_vars_with_unresolved(input, Some("auto"))
}

/// Single-pass `var()` expand against [`ACTIVE_CSS_VARS`].
/// When a reference has no lookup and no fallback, emit `unresolved` (or omit).
fn expand_css_vars_with_unresolved(input: &str, unresolved: Option<&str>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("var(") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 4..];
        let Some((inner, next)) = split_paren_inner(after) else {
            out.push_str(rest);
            return simplify_simple_calc(&out);
        };
        let (name_part, fallback) = match inner.split_once(',') {
            Some((n, fb)) => (n.trim(), Some(fb.trim())),
            None => (inner.trim(), None),
        };
        let resolved = ACTIVE_CSS_VARS.with(|cell| cell.borrow().get(name_part).cloned());
        if let Some(value) = resolved {
            if is_guaranteed_invalid_custom_prop_value(&value) {
                // `initial` on a custom prop → guaranteed-invalid → fallback.
                if let Some(fb) = fallback.filter(|f| !f.is_empty()) {
                    out.push_str(fb);
                } else if let Some(placeholder) = unresolved {
                    out.push_str(placeholder);
                }
            } else if value.contains("var(") {
                // Nested/unfinished — try expanding the value recursively once.
                let nested = expand_css_vars_with_unresolved(&value, unresolved);
                if nested.contains("var(")
                    || nested.trim().is_empty()
                    || is_guaranteed_invalid_custom_prop_value(&nested)
                {
                    if let Some(fb) = fallback.filter(|f| !f.is_empty()) {
                        out.push_str(fb);
                    } else if let Some(placeholder) = unresolved {
                        out.push_str(placeholder);
                    }
                } else {
                    out.push_str(&nested);
                }
            } else {
                out.push_str(&value);
            }
        } else if let Some(fb) = fallback.filter(|f| !f.is_empty()) {
            out.push_str(fb);
        } else if let Some(placeholder) = unresolved {
            out.push_str(placeholder);
        }
        rest = next;
    }
    out.push_str(rest);
    // Fold residual simple calc(Npx * k) after var expansion (radius tokens).
    simplify_simple_calc(&out)
}

/// Resolve `#rgb` / `rgb()` / `var(--x, fallback)` paint colors (no app token maps).
pub fn resolve_paint_color(input: &str) -> Option<[f32; 4]> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(c) = crate::style::parse_css_color(s) {
        return Some(c);
    }
    let expanded = expand_css_var_fallback(s);
    if expanded != s {
        return crate::style::parse_css_color(&expanded);
    }
    None
}

/// CSS `font-size` → computed px using [`active_font_sizes`] (parent = `element_px`).
///
/// Supports lengths (`px`/`em`/`rem`/calc), `%` of parent, and absolute-size keywords
/// (`medium`≈16, `small`≈13, `large`≈18). `inherit`/`unset` → `None`.
pub fn parse_css_font_size(input: &str) -> Option<f32> {
    let expanded = expand_css_var_fallback(input.trim());
    let s = expanded.trim();
    if s.is_empty()
        || s.eq_ignore_ascii_case("inherit")
        || s.eq_ignore_ascii_case("unset")
        || s.eq_ignore_ascii_case("revert")
    {
        return None;
    }
    let fonts = active_font_sizes();
    match s.to_ascii_lowercase().as_str() {
        "initial" | "medium" => return Some(16.0),
        "xx-small" => return Some(9.0),
        "x-small" => return Some(10.0),
        "small" => return Some(13.0),
        "large" => return Some(18.0),
        "x-large" => return Some(24.0),
        "xx-large" => return Some(32.0),
        "xxx-large" => return Some(48.0),
        "smaller" => return Some((fonts.element_px * 0.83).max(1.0)),
        "larger" => return Some((fonts.element_px * 1.2).max(1.0)),
        _ => {}
    }
    if let Some(pct) = s.strip_suffix('%')
        && let Ok(p) = pct.trim().parse::<f32>()
    {
        return Some((fonts.element_px * p / 100.0).max(0.0));
    }
    LengthSpec::parse(s)?
        .resolve_with_fonts(None, active_viewport(), fonts)
        .map(|v| v.max(0.0))
}

/// CSS `font-weight` → 100..=900. `inherit`/`unset` → `None`.
pub fn parse_css_font_weight(input: &str) -> Option<u16> {
    let expanded = expand_css_var_fallback(input.trim());
    let s = expanded.trim();
    if s.is_empty()
        || s.eq_ignore_ascii_case("inherit")
        || s.eq_ignore_ascii_case("unset")
        || s.eq_ignore_ascii_case("revert")
    {
        return None;
    }
    match s.to_ascii_lowercase().as_str() {
        "normal" | "initial" => Some(400),
        "bold" => Some(700),
        // Relative to parent weight — approximate without full cascade of computed weight.
        "lighter" => Some(300),
        "bolder" => Some(700),
        _ => {
            let n: f32 = s.parse().ok()?;
            let stepped = ((n / 100.0).round() as i32 * 100).clamp(100, 900) as u16;
            Some(stepped)
        }
    }
}

/// First usable named family from a `font-family` list (skip CSS generics).
/// Prefers `Noto Sans SC` when present in the stack (Nana / Lilia bundled face).
pub fn parse_css_font_family(input: &str) -> Option<String> {
    let expanded = expand_css_var_fallback(input.trim());
    let s = expanded.trim();
    if s.is_empty()
        || s.eq_ignore_ascii_case("inherit")
        || s.eq_ignore_ascii_case("unset")
        || s.eq_ignore_ascii_case("revert")
        || s.eq_ignore_ascii_case("initial")
    {
        return None;
    }
    let mut first_named: Option<String> = None;
    let mut prefers_noto = false;
    let mut prefers_mono = false;
    for raw in split_font_family_list(s) {
        let name = strip_css_quotes(raw.trim());
        if name.is_empty() {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "serif"
                | "sans-serif"
                | "monospace"
                | "cursive"
                | "fantasy"
                | "system-ui"
                | "ui-serif"
                | "ui-sans-serif"
                | "ui-monospace"
                | "ui-rounded"
                | "-apple-system"
                | "blinkmacsystemfont"
        ) {
            if lower == "monospace" || lower == "ui-monospace" {
                prefers_mono = true;
            }
            continue;
        }
        if lower == "noto sans sc" || lower.contains("noto sans sc") {
            prefers_noto = true;
        }
        if first_named.is_none() {
            first_named = Some(name);
        }
    }
    if prefers_noto {
        return Some("Noto Sans SC".into());
    }
    if let Some(name) = first_named {
        return Some(name);
    }
    if prefers_mono {
        return Some("monospace".into());
    }
    None
}

fn split_font_family_list(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_quote: Option<char> = None;
    for (i, ch) in input.char_indices() {
        match (ch, in_quote) {
            ('"' | '\'', None) => in_quote = Some(ch),
            (q, Some(open)) if q == open => in_quote = None,
            (',', None) => {
                out.push(&input[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start <= input.len() {
        out.push(&input[start..]);
    }
    out
}

fn strip_css_quotes(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 {
        let bytes = t.as_bytes();
        if (bytes[0] == b'"' && bytes[t.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[t.len() - 1] == b'\'')
        {
            return t[1..t.len() - 1].trim().to_string();
        }
    }
    t.to_string()
}

/// CSS `line-height` → [`LineHeightSpec`]. `normal`/`inherit` → `None`.
pub fn parse_css_line_height(input: &str) -> Option<LineHeightSpec> {
    let expanded = expand_css_var_fallback(input.trim());
    let s = expanded.trim();
    if s.is_empty()
        || s.eq_ignore_ascii_case("normal")
        || s.eq_ignore_ascii_case("inherit")
        || s.eq_ignore_ascii_case("unset")
        || s.eq_ignore_ascii_case("revert")
        || s.eq_ignore_ascii_case("initial")
    {
        return None;
    }
    if let Some(pct) = s.strip_suffix('%')
        && let Ok(p) = pct.trim().parse::<f32>()
    {
        return Some(LineHeightSpec::Relative((p / 100.0).max(0.0)));
    }
    // Unitless number = multiplier (MDN). Reject function tokens.
    if !s.contains('(')
        && s.chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '+' || c == '-')
        && let Ok(n) = s.parse::<f32>()
    {
        return Some(LineHeightSpec::Relative(n.max(0.0)));
    }
    if let Some(px) =
        LengthSpec::parse(s)?.resolve_with_fonts(None, active_viewport(), active_font_sizes())
    {
        return Some(LineHeightSpec::Absolute(px.max(0.0)));
    }
    None
}

/// CSS `letter-spacing` → px. `normal` → `Some(0.0)`; `inherit` → `None`.
pub fn parse_css_letter_spacing(input: &str) -> Option<f32> {
    let expanded = expand_css_var_fallback(input.trim());
    let s = expanded.trim();
    if s.is_empty()
        || s.eq_ignore_ascii_case("inherit")
        || s.eq_ignore_ascii_case("unset")
        || s.eq_ignore_ascii_case("revert")
    {
        return None;
    }
    if s.eq_ignore_ascii_case("normal") || s.eq_ignore_ascii_case("initial") {
        return Some(0.0);
    }
    LengthSpec::parse(s)?.resolve_with_fonts(None, active_viewport(), active_font_sizes())
}

fn host_value_debug(value: &nana_js_engine::HostValue) -> String {
    match value {
        nana_js_engine::HostValue::String(s) => s.clone(),
        nana_js_engine::HostValue::Number(n) => n.to_string(),
        nana_js_engine::HostValue::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

fn camel_to_kebab(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    for (i, ch) in input.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Inset length：`12px` / `12` / `10%` / `em`/`rem`（`%` 保留为 [`LengthSpec::Percent`]，含 100%）。
/// 不用 [`LengthSpec::parse`]：后者把 `100%` 收成 `Fill`，不适合作 inset。
pub fn parse_inset_length(input: &str) -> Option<LengthSpec> {
    let s = input.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("auto") || s.eq_ignore_ascii_case("none") {
        return None;
    }
    if let Some(spec) = parse_font_relative_length(s) {
        return Some(spec);
    }
    if let Some(p) = s.strip_suffix('%') {
        let pct = p.trim().parse::<f32>().ok()?;
        return Some(LengthSpec::Percent(pct.max(0.0)));
    }
    let num: f32 = s
        .trim_end_matches("px")
        .trim_end_matches("PX")
        .trim()
        .parse()
        .ok()?;
    Some(LengthSpec::Px(num))
}

/// `Nem` / `Nrem`（CSS Values）；`rem` 须先于 `em` 匹配。
fn parse_font_relative_length(raw: &str) -> Option<LengthSpec> {
    let s = raw.trim().to_ascii_lowercase();
    if let Some(n) = s.strip_suffix("rem") {
        let v = n.trim().parse::<f32>().ok()?;
        return Some(LengthSpec::Rem(v));
    }
    if let Some(n) = s.strip_suffix("em") {
        // Avoid matching bare unit-less leftovers; require a number (incl. `.5em`).
        let v = n.trim().parse::<f32>().ok()?;
        return Some(LengthSpec::Em(v));
    }
    None
}

/// `min-*` / `max-*`：保留 [`LengthSpec`]，布局时相对 CB / viewport / font 解析。
fn parse_min_max_size(raw: &str) -> Option<LengthSpec> {
    let s = raw.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        return None;
    }
    if s.eq_ignore_ascii_case("auto") {
        return Some(LengthSpec::Auto);
    }
    match LengthSpec::parse(s)? {
        LengthSpec::Fill => Some(LengthSpec::Percent(100.0)),
        LengthSpec::Shrink => Some(LengthSpec::Shrink),
        LengthSpec::Auto => Some(LengthSpec::Auto),
        other => Some(other),
    }
}

/// margin / padding 边长：px / `%` / em / rem / 轻量 calc / viewport / min-max；
/// `%` 与 `100%` 均保留为 [`LengthSpec`]（不收成 `Fill`），布局时相对包含块宽度解析。
/// Padding 路径钳制 `Px` ≥ 0；见 [`parse_margin_length`] 允许负值。
pub fn parse_box_edge_length(input: &str) -> Option<LengthSpec> {
    parse_box_edge_length_inner(input, /* clamp_px_non_negative */ true)
}

/// CSS margin 边长（允许负 `px` / `em` 等）。
pub fn parse_margin_length(input: &str) -> Option<LengthSpec> {
    parse_box_edge_length_inner(input, /* clamp_px_non_negative */ false)
}

fn parse_box_edge_length_inner(input: &str, clamp_px_non_negative: bool) -> Option<LengthSpec> {
    let s = input.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        return None;
    }
    if s.eq_ignore_ascii_case("auto") {
        return Some(LengthSpec::Auto);
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("calc(")
        || lower.starts_with("min(")
        || lower.starts_with("max(")
        || lower.starts_with("clamp(")
        || lower.contains("vh")
        || lower.contains("vw")
        || lower.contains("vmin")
        || lower.contains("vmax")
        || lower.contains("rem")
        || lower.ends_with("em")
    {
        return match LengthSpec::parse(s)? {
            LengthSpec::Fill => Some(LengthSpec::Percent(100.0)),
            LengthSpec::Shrink | LengthSpec::Auto => None,
            LengthSpec::Px(px) if clamp_px_non_negative => Some(LengthSpec::Px(px.max(0.0))),
            other => Some(other),
        };
    }
    parse_inset_length(s).map(|spec| match spec {
        LengthSpec::Px(px) if clamp_px_non_negative => LengthSpec::Px(px.max(0.0)),
        other => other,
    })
}

/// CSS 1–4 box sides. Any unparsed token must have already failed (no 4→3 collapse).
fn css_four_sides<T: Copy>(parsed: &[T]) -> Option<[T; 4]> {
    Some(match parsed.len() {
        1 => [parsed[0]; 4],
        2 => [parsed[0], parsed[1], parsed[0], parsed[1]],
        3 => [parsed[0], parsed[1], parsed[2], parsed[1]],
        4.. => [parsed[0], parsed[1], parsed[2], parsed[3]],
        0 => return None,
    })
}

fn apply_border_width_shorthand(style: &mut LayoutStyle, val: &str) {
    let parts = split_css_space_tokens(val);
    let Some(parsed) = parts
        .iter()
        .map(|part| parse_css_length_px(part, None).map(|v| v.max(0.0)))
        .collect::<Option<Vec<f32>>>()
    else {
        return;
    };
    let Some([top, right, bottom, left]) = css_four_sides(&parsed) else {
        return;
    };
    style.border_width = Some(top.max(right).max(bottom).max(left));
    style.border_top_width = Some(top);
    style.border_right_width = Some(right);
    style.border_bottom_width = Some(bottom);
    style.border_left_width = Some(left);
}

fn apply_border_color_shorthand(style: &mut LayoutStyle, val: &str) {
    let parts = split_css_space_tokens(val);
    let Some(parsed) = parts
        .iter()
        .map(|part| crate::style::parse_css_color(part))
        .collect::<Option<Vec<[f32; 4]>>>()
    else {
        return;
    };
    let Some([top, right, bottom, left]) = css_four_sides(&parsed) else {
        return;
    };
    style.border_color = Some(top);
    style.border_top_color = Some(top);
    style.border_right_color = Some(right);
    style.border_bottom_color = Some(bottom);
    style.border_left_color = Some(left);
}

fn apply_border_style_shorthand(style: &mut LayoutStyle, val: &str) {
    let parts = split_css_space_tokens(val);
    let Some(parsed) = parts
        .iter()
        .map(|part| BorderStyle::parse(part))
        .collect::<Option<Vec<BorderStyle>>>()
    else {
        return;
    };
    let Some([top, right, bottom, left]) = css_four_sides(&parsed) else {
        return;
    };
    style.border_style = Some(top);
    style.border_top_style = Some(top);
    style.border_right_style = Some(right);
    style.border_bottom_style = Some(bottom);
    style.border_left_style = Some(left);
}

fn apply_border_side_style(style: &mut LayoutStyle, val: &str, side: usize) {
    let Some(parsed) = BorderStyle::parse(val) else {
        return;
    };
    match side {
        0 => style.border_top_style = Some(parsed),
        1 => style.border_right_style = Some(parsed),
        2 => style.border_bottom_style = Some(parsed),
        _ => style.border_left_style = Some(parsed),
    }
}

/// `border` / `border-top` … : width || style || color. `side` None = all four.
fn apply_border_shorthand(style: &mut LayoutStyle, val: &str, side: Option<usize>) {
    let trimmed = val.trim();
    if trimmed.is_empty() {
        return;
    }
    if trimmed.eq_ignore_ascii_case("none") {
        assign_border_side_style(style, side, BorderStyle::None);
        assign_border_side_width(style, side, 0.0);
        return;
    }
    let mut width = None;
    let mut parsed_style = None;
    let mut color = None;
    for part in split_css_space_tokens(trimmed) {
        if let Some(s) = BorderStyle::parse(&part) {
            parsed_style = Some(s);
        } else if let Some(v) = parse_css_length_px(&part, None) {
            width = Some(v.max(0.0));
        } else if let Some(c) = crate::style::parse_css_color(&part) {
            color = Some(c);
        }
    }
    if let Some(w) = width {
        assign_border_side_width(style, side, w);
    }
    if let Some(s) = parsed_style {
        assign_border_side_style(style, side, s);
    }
    if let Some(c) = color {
        assign_border_side_color(style, side, c);
    }
}

fn assign_border_side_width(style: &mut LayoutStyle, side: Option<usize>, width: f32) {
    match side {
        None => {
            style.border_width = Some(width);
            style.border_top_width = Some(width);
            style.border_right_width = Some(width);
            style.border_bottom_width = Some(width);
            style.border_left_width = Some(width);
        }
        Some(0) => style.border_top_width = Some(width),
        Some(1) => style.border_right_width = Some(width),
        Some(2) => style.border_bottom_width = Some(width),
        _ => style.border_left_width = Some(width),
    }
}

fn assign_border_side_color(style: &mut LayoutStyle, side: Option<usize>, color: [f32; 4]) {
    match side {
        None => {
            style.border_color = Some(color);
            style.border_top_color = Some(color);
            style.border_right_color = Some(color);
            style.border_bottom_color = Some(color);
            style.border_left_color = Some(color);
        }
        Some(0) => style.border_top_color = Some(color),
        Some(1) => style.border_right_color = Some(color),
        Some(2) => style.border_bottom_color = Some(color),
        _ => style.border_left_color = Some(color),
    }
}

fn assign_border_side_style(style: &mut LayoutStyle, side: Option<usize>, parsed: BorderStyle) {
    match side {
        None => {
            style.border_style = Some(parsed);
            style.border_top_style = Some(parsed);
            style.border_right_style = Some(parsed);
            style.border_bottom_style = Some(parsed);
            style.border_left_style = Some(parsed);
        }
        Some(0) => style.border_top_style = Some(parsed),
        Some(1) => style.border_right_style = Some(parsed),
        Some(2) => style.border_bottom_style = Some(parsed),
        _ => style.border_left_style = Some(parsed),
    }
}

fn split_css_space_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ' ' | '\t' | '\n' if depth == 0 => {
                if start < idx {
                    tokens.push(input[start..idx].trim().to_string());
                }
                start = idx + 1;
            }
            _ => {}
        }
    }
    if start < input.len() {
        tokens.push(input[start..].trim().to_string());
    }
    tokens
        .into_iter()
        .filter(|token| !token.is_empty())
        .collect()
}

/// Parse CSS `border-radius` shorthand (1–4 values, px or `%`).
fn parse_border_radius_shorthand(input: &str) -> Option<[LengthSpec; 4]> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let parsed: Vec<LengthSpec> = parts
        .iter()
        .filter_map(|part| parse_inset_length(part))
        .collect();
    css_four_sides(&parsed)
}

/// Parse `box-shadow` layers (`inset` + comma list, GPU-capped).
pub(crate) fn parse_box_shadows(input: &str) -> Option<Vec<BoxShadowSpec>> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut layers = Vec::new();
    for part in split_shadow_layers(trimmed) {
        if let Some(layer) = parse_one_box_shadow_layer(&part) {
            layers.push(layer);
            if layers.len() >= nana_ui_core::MAX_BOX_SHADOWS {
                break;
            }
        }
    }
    Some(layers)
}

fn split_shadow_layers(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(input[start..idx].trim().to_string());
                start = idx + 1;
            }
            _ => {}
        }
    }
    if start <= input.len() {
        parts.push(input[start..].trim().to_string());
    }
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

fn parse_one_box_shadow_layer(input: &str) -> Option<BoxShadowSpec> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (length_tokens, color_token, inset) = split_box_shadow_tokens(trimmed)?;
    if length_tokens.is_empty() {
        return None;
    }
    let offset_x = parse_shadow_length_px(&length_tokens[0])?;
    let offset_y = length_tokens
        .get(1)
        .and_then(|t| parse_shadow_length_px(t))
        .unwrap_or(0.0);
    let blur_radius = length_tokens
        .get(2)
        .and_then(|t| parse_shadow_blur_length_px(t))
        .unwrap_or(0.0);
    let spread_radius = length_tokens
        .get(3)
        .and_then(|t| parse_shadow_length_px(t))
        .unwrap_or(0.0);
    let color = color_token
        .as_deref()
        .and_then(crate::style::parse_css_color)
        .unwrap_or([0.0, 0.0, 0.0, 1.0]);
    Some(BoxShadowSpec {
        offset_x,
        offset_y,
        blur_radius,
        spread_radius,
        color,
        inset,
    })
}

/// Parse `filter: drop-shadow()` args. No inset, no spread (4th length).
pub(crate) fn parse_drop_shadow(input: &str) -> Option<nana_ui_core::FilterDropShadow> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (length_tokens, color_token, inset) = split_box_shadow_tokens(trimmed)?;
    if inset || length_tokens.len() < 2 || length_tokens.len() > 3 {
        return None;
    }
    let offset_x = parse_shadow_length_px(&length_tokens[0])?;
    let offset_y = parse_shadow_length_px(&length_tokens[1])?;
    let blur_radius = match length_tokens.get(2) {
        Some(token) => parse_shadow_blur_length_px(token)?,
        None => 0.0,
    };
    let color = match color_token.as_deref() {
        Some(token) => crate::style::parse_css_color(token)?,
        None => [0.0, 0.0, 0.0, 1.0],
    };
    Some(nana_ui_core::FilterDropShadow {
        offset_x,
        offset_y,
        blur_radius,
        color,
    })
}

/// Parse single-layer `text-shadow` (`offset-x offset-y [blur-radius] color`).
pub(crate) fn parse_text_shadow(input: &str) -> Option<TextShadowSpec> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return None;
    }
    if trimmed.to_ascii_lowercase().contains("inset") {
        return None;
    }
    let (length_tokens, color_token, _inset) = split_box_shadow_tokens(trimmed)?;
    if length_tokens.is_empty() {
        return None;
    }
    let offset_x = parse_shadow_length_px(&length_tokens[0])?;
    let offset_y = length_tokens
        .get(1)
        .and_then(|t| parse_shadow_length_px(t))
        .unwrap_or(0.0);
    let blur_radius = length_tokens
        .get(2)
        .and_then(|t| parse_shadow_blur_length_px(t))
        .unwrap_or(0.0);
    let color = color_token
        .as_deref()
        .and_then(crate::style::parse_css_color)
        .unwrap_or([0.0, 0.0, 0.0, 0.5]);
    Some(TextShadowSpec {
        offset_x,
        offset_y,
        blur_radius,
        color,
    })
}

/// Signed physical length for shadow offsets and spread (`px`, `%`, `em`, …).
fn parse_shadow_length_px(input: &str) -> Option<f32> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(spec) = parse_inset_length(s) {
        let vp = ACTIVE_VIEWPORT.with(|cell| *cell.borrow());
        let fonts = ACTIVE_FONT_SIZES.with(|cell| *cell.borrow());
        return spec.resolve_with_fonts(None, vp, fonts);
    }
    if let Some(spec) = LengthSpec::parse(s) {
        let vp = ACTIVE_VIEWPORT.with(|cell| *cell.borrow());
        let fonts = ACTIVE_FONT_SIZES.with(|cell| *cell.borrow());
        return spec.resolve_with_fonts(None, vp, fonts);
    }
    None
}

/// Blur radius must be non-negative; negative values reject the shadow.
fn parse_shadow_blur_length_px(input: &str) -> Option<f32> {
    let px = parse_shadow_length_px(input)?;
    (px >= 0.0).then_some(px)
}

fn split_box_shadow_tokens(input: &str) -> Option<(Vec<String>, Option<String>, bool)> {
    let mut length_tokens = Vec::new();
    let mut color_token = None;
    let mut inset = false;
    let mut paren_depth = 0i32;
    let mut token_start = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ' ' | '\t' if paren_depth == 0 => {
                if token_start < idx {
                    classify_box_shadow_token(
                        input[token_start..idx].trim(),
                        &mut length_tokens,
                        &mut color_token,
                        &mut inset,
                    )?;
                }
                token_start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    if token_start < input.len() {
        classify_box_shadow_token(
            input[token_start..].trim(),
            &mut length_tokens,
            &mut color_token,
            &mut inset,
        )?;
    }
    Some((length_tokens, color_token, inset))
}

fn classify_box_shadow_token(
    token: &str,
    length_tokens: &mut Vec<String>,
    color_token: &mut Option<String>,
    inset: &mut bool,
) -> Option<()> {
    let token = token.trim();
    if token.is_empty() {
        return Some(());
    }
    if token.eq_ignore_ascii_case("inset") {
        *inset = true;
        return Some(());
    }
    if is_css_color_token(token) {
        if color_token.is_some() {
            return None;
        }
        *color_token = Some(token.to_string());
        return Some(());
    }
    if parse_shadow_length_px(token).is_some() {
        length_tokens.push(token.to_string());
        return Some(());
    }
    None
}

fn is_css_color_token(token: &str) -> bool {
    token.starts_with('#')
        || token.starts_with("rgb")
        || token.starts_with("hsl")
        || is_css_named_color(token)
}

fn is_css_named_color(token: &str) -> bool {
    token.eq_ignore_ascii_case("currentcolor")
        || crate::style::parse_css_named_color(token).is_some()
}

/// 解析 `12px` / `12` / `50%` / `em` / `vh` / `min()`（相对 base + 活跃 viewport）为逻辑像素。
pub fn parse_css_length_px(input: &str, percent_base: Option<f32>) -> Option<f32> {
    let s = input.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("auto") || s.eq_ignore_ascii_case("none") {
        return None;
    }
    if let Some(spec) = LengthSpec::parse(s) {
        let vp = ACTIVE_VIEWPORT.with(|cell| *cell.borrow());
        let fonts = ACTIVE_FONT_SIZES.with(|cell| *cell.borrow());
        if let Some(px) = spec.resolve_with_fonts(percent_base, vp, fonts) {
            return Some(px.max(0.0));
        }
    }
    if let Some(p) = s.strip_suffix('%') {
        let pct = p.trim().parse::<f32>().ok()?;
        return percent_base.map(|base| base * pct / 100.0);
    }
    let num: f32 = s
        .trim_end_matches("px")
        .trim_end_matches("PX")
        .trim()
        .parse()
        .ok()?;
    Some(num)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_important_parses_as_normal_declaration() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("width:100px !important", None, None);
        assert_eq!(
            layout.width,
            Some(LengthSpec::Px(100.0)),
            "inline-only width:100px !important must parse as 100, not drop the declaration"
        );

        let mut cased = LayoutStyle::default();
        cased.apply_css_property("height", "40px!IMPORTANT", None, None);
        assert_eq!(cased.height, Some(LengthSpec::Px(40.0)));

        let mut spaced = LayoutStyle::default();
        spaced.apply_css_text("width: 80px ! Important; height: 20px", None, None);
        assert_eq!(spaced.width, Some(LengthSpec::Px(80.0)));
        assert_eq!(spaced.height, Some(LengthSpec::Px(20.0)));
    }

    #[test]
    fn parses_flex_row_gap_padding_percent_width() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:flex; flex-direction:row; gap:12px; padding:8px; width:50%; height:100px; align-items:center",
            Some(400.0),
            Some(300.0),
        );
        assert_eq!(layout.direction, Some(FlexDirection::Row));
        assert_eq!(layout.gap, Some(LengthSpec::Px(12.0)));
        assert_eq!(layout.padding, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.width, Some(LengthSpec::Percent(50.0)));
        assert_eq!(layout.height, Some(LengthSpec::Px(100.0)));
        assert_eq!(layout.align_items, AlignSpec::Center);
        assert_eq!(layout.width.unwrap().resolve_px(Some(400.0)), Some(200.0));
    }

    #[test]
    fn class_hints_set_row_and_gap() {
        let mut layout = LayoutStyle::default();
        layout.apply_class_layout_hints(&["flex-row".into(), "gap-lg".into()]);
        assert_eq!(layout.direction, Some(FlexDirection::Row));
        assert_eq!(layout.gap, Some(LengthSpec::Px(16.0)));
    }

    #[test]
    fn class_gap_token_clears_row_and_column_gap_longhands() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("row-gap: 4px; column-gap: 20px", None, None);
        assert!(layout.gap.is_none());
        assert_eq!(layout.row_gap, Some(LengthSpec::Px(4.0)));
        assert_eq!(layout.column_gap, Some(LengthSpec::Px(20.0)));
        // Without clear, longhands would still win over the new uniform gap.
        assert_eq!(layout.resolved_row_gap(), 4.0);
        assert_eq!(layout.resolved_column_gap(), 20.0);

        layout.apply_class_layout_hints(&["gap-lg".into()]);
        assert_eq!(layout.gap, Some(LengthSpec::Px(16.0)));
        assert!(layout.row_gap.is_none(), "row-gap longhand cleared");
        assert!(layout.column_gap.is_none(), "column-gap longhand cleared");
        assert_eq!(layout.resolved_row_gap(), 16.0);
        assert_eq!(layout.resolved_column_gap(), 16.0);
    }

    #[test]
    fn class_gap_token_skips_when_uniform_gap_already_set() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("gap: 10px; row-gap: 4px", None, None);
        layout.apply_class_layout_hints(&["gap-sm".into()]);
        // Existing uniform gap: class must not rewrite or clear longhands.
        assert_eq!(layout.gap, Some(LengthSpec::Px(10.0)));
        assert_eq!(layout.row_gap, Some(LengthSpec::Px(4.0)));
        assert_eq!(layout.resolved_row_gap(), 4.0);
        assert_eq!(layout.resolved_column_gap(), 10.0);
    }

    #[test]
    fn padding_shorthand_four_values() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("padding: 1px 2px 3px 4px", None, None);
        assert_eq!(layout.padding_top, Some(LengthSpec::Px(1.0)));
        assert_eq!(layout.padding_right, Some(LengthSpec::Px(2.0)));
        assert_eq!(layout.padding_bottom, Some(LengthSpec::Px(3.0)));
        assert_eq!(layout.padding_left, Some(LengthSpec::Px(4.0)));
        let p = layout.resolved_padding();
        assert_eq!(p.top, 1.0);
        assert_eq!(p.right, 2.0);
        assert_eq!(p.bottom, 3.0);
        assert_eq!(p.left, 4.0);
    }

    #[test]
    fn padding_shorthand_three_values() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("padding: 1px 2px 3px", None, None);
        assert_eq!(layout.padding_top, Some(LengthSpec::Px(1.0)));
        assert_eq!(layout.padding_left, Some(LengthSpec::Px(2.0)));
        assert_eq!(layout.padding_right, Some(LengthSpec::Px(2.0)));
        assert_eq!(layout.padding_bottom, Some(LengthSpec::Px(3.0)));
    }

    #[test]
    fn logical_padding_maps_ltr_to_physical() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "padding-block: 1px 3px; padding-inline: 4px 2px",
            None,
            None,
        );
        assert_eq!(layout.padding_top, Some(LengthSpec::Px(1.0)));
        assert_eq!(layout.padding_right, Some(LengthSpec::Px(2.0)));
        assert_eq!(layout.padding_bottom, Some(LengthSpec::Px(3.0)));
        assert_eq!(layout.padding_left, Some(LengthSpec::Px(4.0)));
        let p = layout.resolved_padding();
        assert_eq!((p.top, p.right, p.bottom, p.left), (1.0, 2.0, 3.0, 4.0));
    }

    #[test]
    fn logical_margin_longhands_map_ltr() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "margin-block-start:4px; margin-inline-end:8px; margin-block-end:6px; margin-inline-start:2px",
            None,
            None,
        );
        assert_eq!(layout.margin_top, Some(LengthSpec::Px(4.0)));
        assert_eq!(layout.margin_right, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.margin_bottom, Some(LengthSpec::Px(6.0)));
        assert_eq!(layout.margin_left, Some(LengthSpec::Px(2.0)));
    }

    #[test]
    fn logical_inset_shorthand_maps_ltr() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "position:absolute; inset-block:8px; inset-inline:24px 16px",
            None,
            None,
        );
        assert_eq!(layout.offset_top, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.offset_bottom, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.offset_left, Some(LengthSpec::Px(24.0)));
        assert_eq!(layout.offset_right, Some(LengthSpec::Px(16.0)));
    }

    #[test]
    fn logical_padding_overrides_uniform_on_mapped_sides() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("padding: 5px; padding-inline-start: 12px", None, None);
        assert_eq!(layout.padding, Some(LengthSpec::Px(5.0)));
        assert_eq!(layout.padding_left, Some(LengthSpec::Px(12.0)));
        let p = layout.resolved_padding();
        assert_eq!(p.left, 12.0);
        assert_eq!(p.right, 5.0);
        assert_eq!(p.top, 5.0);
        assert_eq!(p.bottom, 5.0);
    }

    #[test]
    fn direction_rtl_padding_inline_start_applies_to_right() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("direction: rtl; padding-inline-start: 12px", None, None);
        assert_eq!(layout.dir, Some(DirSpec::Rtl));
        assert_eq!(layout.padding_right, Some(LengthSpec::Px(12.0)));
        assert!(layout.padding_left.is_none());
    }

    #[test]
    fn direction_rtl_after_padding_inline_start_still_maps_to_right() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("padding-inline-start: 12px; direction: rtl", None, None);
        assert_eq!(layout.padding_right, Some(LengthSpec::Px(12.0)));
        assert!(layout.padding_left.is_none());
    }

    #[test]
    fn direction_rtl_maps_margin_and_inset_inline() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "direction:rtl; margin-inline-start:8px; margin-inline-end:2px; \
             inset-inline: 24px 16px",
            None,
            None,
        );
        assert_eq!(layout.margin_right, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.margin_left, Some(LengthSpec::Px(2.0)));
        assert_eq!(layout.offset_right, Some(LengthSpec::Px(24.0)));
        assert_eq!(layout.offset_left, Some(LengthSpec::Px(16.0)));
    }

    #[test]
    fn direction_rtl_padding_inline_shorthand_swaps_start_end() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("direction:rtl; padding-inline: 4px 2px", None, None);
        assert_eq!(layout.padding_right, Some(LengthSpec::Px(4.0)));
        assert_eq!(layout.padding_left, Some(LengthSpec::Px(2.0)));
    }

    #[test]
    fn direction_rtl_text_align_start_end_are_logical() {
        let mut start = LayoutStyle::default();
        start.apply_css_text("direction:rtl; text-align:start", None, None);
        assert_eq!(start.text_align, TextAlignSpec::Start);
        assert_eq!(
            start.text_align.to_justify(start.is_rtl()),
            JustifySpec::End
        );

        let mut end = LayoutStyle::default();
        end.apply_css_text("direction:rtl; text-align:end", None, None);
        assert_eq!(end.text_align, TextAlignSpec::End);
        assert_eq!(end.text_align.to_justify(end.is_rtl()), JustifySpec::Start);

        let mut left = LayoutStyle::default();
        left.apply_css_text("direction:rtl; text-align:left", None, None);
        assert_eq!(left.text_align, TextAlignSpec::Left);
        assert_eq!(
            left.text_align.to_justify(left.is_rtl()),
            JustifySpec::Start
        );
    }

    #[test]
    fn direction_rtl_inherit_typography_remaps_logical_padding() {
        let mut child = LayoutStyle::default();
        child.apply_css_text("padding-inline-start: 12px", None, None);
        assert_eq!(child.padding_left, Some(LengthSpec::Px(12.0)));
        assert!(child.padding_right.is_none());

        let mut parent = LayoutStyle::default();
        parent.dir = Some(DirSpec::Rtl);
        child.inherit_typography_from(&parent);
        assert_eq!(child.dir, Some(DirSpec::Rtl));
        assert_eq!(child.padding_right, Some(LengthSpec::Px(12.0)));
        assert!(child.padding_left.is_none());
    }

    #[test]
    fn direction_rtl_does_not_reverse_flex_or_grid_start() {
        let mut flex = LayoutStyle::default();
        flex.apply_css_text(
            "display:flex; flex-direction:row; direction:rtl; justify-content:start",
            None,
            None,
        );
        assert_eq!(flex.dir, Some(DirSpec::Rtl));
        assert_eq!(flex.direction, Some(FlexDirection::Row));
        assert!(!flex.flex_reverse);
        assert_eq!(flex.justify_content, JustifySpec::Start);

        let mut grid = LayoutStyle::default();
        grid.apply_css_text(
            "display:grid; direction:rtl; justify-content:start; justify-items:start",
            None,
            None,
        );
        assert_eq!(grid.dir, Some(DirSpec::Rtl));
        assert!(!grid.flex_reverse);
        assert_eq!(grid.justify_content, JustifySpec::Start);
        assert_eq!(grid.justify_items, Some(AlignSpec::Start));
    }

    #[test]
    fn writing_mode_vertical_fail_closed_does_not_flip_axis() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "writing-mode: vertical-rl; padding-inline-start: 12px",
            None,
            None,
        );
        assert!(layout.unsupported_writing_mode);
        assert_eq!(layout.padding_left, Some(LengthSpec::Px(12.0)));
        assert!(layout.padding_top.is_none());
    }

    #[test]
    fn unicode_bidi_isolate_does_not_fake_isolation() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("unicode-bidi: isolate; isolation: isolate", None, None);
        assert!(layout.isolation);
        let mut bidi_only = LayoutStyle::default();
        bidi_only.apply_css_text("unicode-bidi: isolate", None, None);
        assert!(!bidi_only.isolation);
    }

    #[test]
    fn flex_direction_column_overrides_display_flex_default_row() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("display:flex; flex-direction:column; gap:4px", None, None);
        assert_eq!(layout.direction, Some(FlexDirection::Column));
        assert_eq!(layout.gap, Some(LengthSpec::Px(4.0)));
    }

    #[test]
    fn calc_percent_minus_px_parses() {
        assert_eq!(
            LengthSpec::parse("calc(100% - 40px)"),
            Some(LengthSpec::CalcPercentOffset {
                percent: 100.0,
                offset_px: -40.0,
            })
        );
        assert_eq!(
            LengthSpec::parse("calc(50% + 10px)"),
            Some(LengthSpec::CalcPercentOffset {
                percent: 50.0,
                offset_px: 10.0,
            })
        );
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("width: calc(100% - 40px)", None, None);
        assert_eq!(
            layout.width,
            Some(LengthSpec::CalcPercentOffset {
                percent: 100.0,
                offset_px: -40.0,
            })
        );
    }

    #[test]
    fn calc_lightweight_extra_forms_parse() {
        assert_eq!(
            LengthSpec::parse("calc(40px + 50%)"),
            Some(LengthSpec::CalcPercentOffset {
                percent: 50.0,
                offset_px: 40.0,
            })
        );
        assert_eq!(
            LengthSpec::parse("calc(25% + 25%)"),
            Some(LengthSpec::Percent(50.0))
        );
        assert_eq!(
            LengthSpec::parse("calc(80px - 20px)"),
            Some(LengthSpec::Px(60.0))
        );
        assert_eq!(
            LengthSpec::parse("calc(50%)"),
            Some(LengthSpec::Percent(50.0))
        );
        assert_eq!(LengthSpec::parse("calc(40px)"), Some(LengthSpec::Px(40.0)));
    }

    #[test]
    fn calc_mul_div_nested_and_var_parse() {
        assert_eq!(
            LengthSpec::parse("calc(12 * 2px)"),
            Some(LengthSpec::Px(24.0))
        );
        assert_eq!(
            LengthSpec::parse("calc(2px * 12)"),
            Some(LengthSpec::Px(24.0))
        );
        assert_eq!(
            LengthSpec::parse("calc(100vw / 1280)"),
            Some(LengthSpec::Viewport {
                axis: ViewportAxis::Width,
                value: 100.0 / 1280.0,
            })
        );
        assert_eq!(
            LengthSpec::parse("calc(100% - 12 * 1px)"),
            Some(LengthSpec::CalcPercentOffset {
                percent: 100.0,
                offset_px: -12.0,
            })
        );
        assert_eq!(
            LengthSpec::parse("calc((100% - 12px) / 2)"),
            Some(LengthSpec::CalcPercentOffset {
                percent: 50.0,
                offset_px: -6.0,
            })
        );
        assert_eq!(
            LengthSpec::parse("calc(calc(10px) + 2px)"),
            Some(LengthSpec::Px(12.0))
        );
        assert_eq!(
            LengthSpec::parse("min(calc(10px), max(1px, 2px))"),
            Some(LengthSpec::Px(2.0))
        );
        // Unknown math functions fail closed (no pretend-success).
        assert_eq!(LengthSpec::parse("calc(tan(45deg) * 10px)"), None);
        assert_eq!(LengthSpec::parse("calc(atan2(1, 1) * 10px)"), None);

        with_active_viewport(1280.0, 800.0, || {
            assert_eq!(
                LengthSpec::parse("calc(100vw / 1280)")
                    .unwrap()
                    .resolve_with(None, active_viewport()),
                Some(1.0)
            );
        });

        let mut vars = std::collections::BTreeMap::new();
        vars.insert("--x".into(), "10px".into());
        with_active_css_vars(&vars, || {
            let mut layout = LayoutStyle::default();
            layout.apply_css_text(
                "width: calc(2 * var(--x)); height: calc(12 * 2px)",
                None,
                None,
            );
            assert_eq!(layout.width, Some(LengthSpec::Px(20.0)));
            assert_eq!(layout.height, Some(LengthSpec::Px(24.0)));
        });
    }

    #[test]
    fn calc_fail_closed_and_honest_fold() {
        assert_eq!(
            LengthSpec::parse("calc(2 * 3)"),
            None,
            "unitless calc is not a length"
        );
        assert_eq!(
            LengthSpec::parse("calc(0px + 10)"),
            None,
            "0px stays a length; number + length is invalid"
        );
        assert_eq!(
            LengthSpec::parse("calc(0px * 10px)"),
            None,
            "length times length is unsupported"
        );
        assert_eq!(
            LengthSpec::parse("calc(50% * 3)"),
            Some(LengthSpec::Percent(150.0))
        );
        assert_eq!(
            LengthSpec::parse("calc(99.6%)"),
            Some(LengthSpec::Percent(99.6)),
            "must not snap near-100% to Fill"
        );
        assert_eq!(LengthSpec::parse("calc(10px / 0)"), None);
        assert_eq!(
            LengthSpec::parse("calc(1e20px * 1e20)"),
            None,
            "non-finite f32 must fail closed"
        );
        assert_eq!(
            LengthSpec::parse("calc(-1 * 8px)"),
            Some(LengthSpec::Px(-8.0))
        );
        assert_eq!(
            LengthSpec::parse("calc(-1em + 20px)"),
            Some(LengthSpec::CalcEmOffset {
                em: -1.0,
                offset_px: 20.0,
            })
        );
        assert_eq!(
            LengthSpec::parse("calc(1em - 20px)"),
            Some(LengthSpec::CalcEmOffset {
                em: 1.0,
                offset_px: -20.0,
            })
        );

        let mut margin = LayoutStyle::default();
        margin.apply_css_text("margin-left: calc(-1 * 8px)", None, None);
        assert_eq!(margin.margin_left, Some(LengthSpec::Px(-8.0)));
        assert_eq!(margin.resolved_margin_against(None).left, -8.0);

        let props = collect_css_custom_properties(":root { --n: calc(2 * 3); --x: 10px; }");
        assert_ne!(
            props.get("--n").map(String::as_str),
            Some("6px"),
            "unitless custom-prop calc must not fold to px"
        );
        assert_eq!(LengthSpec::parse(props.get("--n").expect("--n")), None);
        assert_eq!(props.get("--x").map(String::as_str), Some("10px"));
        with_active_css_vars(&props, || {
            let mut layout = LayoutStyle::default();
            layout.apply_css_text("width: calc(2 * var(--x))", None, None);
            assert_eq!(layout.width, Some(LengthSpec::Px(20.0)));
        });
    }

    #[test]
    fn em_rem_and_calc_font_relative_parse() {
        assert_eq!(LengthSpec::parse("2em"), Some(LengthSpec::Em(2.0)));
        assert_eq!(LengthSpec::parse("1.5rem"), Some(LengthSpec::Rem(1.5)));
        assert_eq!(
            LengthSpec::parse("calc(2em + 4px)"),
            Some(LengthSpec::CalcEmOffset {
                em: 2.0,
                offset_px: 4.0,
            })
        );
        assert_eq!(
            LengthSpec::parse("calc(1rem - 2px)"),
            Some(LengthSpec::CalcRemOffset {
                rem: 1.0,
                offset_px: -2.0,
            })
        );
        // Default CSS initial font-size = 16px.
        assert_eq!(LengthSpec::Em(2.0).resolve_px(None), Some(32.0));
        assert_eq!(
            LengthSpec::Rem(1.5).resolve_with_fonts(None, None, FontSizeContext::uniform(20.0)),
            Some(30.0)
        );
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("width: 2em; padding: 0.5rem; margin: -8px 0", None, None);
        assert_eq!(layout.width, Some(LengthSpec::Em(2.0)));
        assert_eq!(layout.padding, Some(LengthSpec::Rem(0.5)));
        assert_eq!(layout.margin_top, Some(LengthSpec::Px(-8.0)));
        assert_eq!(layout.margin_bottom, Some(LengthSpec::Px(-8.0)));
        let m = layout.resolved_margin_against(None);
        assert_eq!(m.top, -8.0);
        assert_eq!(m.bottom, -8.0);
    }

    #[test]
    fn typography_css_parses_into_layout_style() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "font-size: 13px; font-weight: 600; font-family: \"Noto Sans SC\", system-ui, sans-serif; \
             line-height: 1.55; letter-spacing: 0.5px; color: #5a616e",
            None,
            None,
        );
        assert_eq!(layout.font_size, Some(13.0));
        assert_eq!(layout.font_weight, Some(600));
        assert_eq!(layout.font_family.as_deref(), Some("Noto Sans SC"));
        assert_eq!(layout.line_height, Some(LineHeightSpec::Relative(1.55)));
        assert_eq!(layout.letter_spacing, Some(0.5));
        let c = layout.color.expect("color");
        assert!((c[0] - 90.0 / 255.0).abs() < 0.02);
        assert!((c[1] - 97.0 / 255.0).abs() < 0.02);
        assert!((c[2] - 110.0 / 255.0).abs() < 0.02);

        // em font-size uses active (parent) element_px.
        with_active_font_sizes(FontSizeContext::uniform(20.0), || {
            let mut child = LayoutStyle::default();
            child.apply_css_text("font-size: 0.65em; letter-spacing: 0.02em", None, None);
            assert!((child.font_size.unwrap() - 13.0).abs() < 0.01);
            assert!((child.letter_spacing.unwrap() - 0.4).abs() < 0.01);
        });

        // var(--font-sans) / var(--text) resolve via active custom properties.
        let mut vars = std::collections::BTreeMap::new();
        vars.insert(
            "--font-sans".into(),
            "system-ui, \"Noto Sans SC\", sans-serif".into(),
        );
        vars.insert("--text-muted".into(), "#5a616e".into());
        with_active_css_vars(&vars, || {
            let mut themed = LayoutStyle::default();
            themed.apply_css_text(
                "font-family: var(--font-sans); color: var(--text-muted); font-size: 18px; font-weight: bold",
                None,
                None,
            );
            assert_eq!(themed.font_family.as_deref(), Some("Noto Sans SC"));
            assert_eq!(themed.font_size, Some(18.0));
            assert_eq!(themed.font_weight, Some(700));
            assert!(themed.color.is_some());
        });

        let mut parent = LayoutStyle::default();
        parent.font_size = Some(13.0);
        parent.font_weight = Some(600);
        parent.color = Some([0.1, 0.1, 0.1, 1.0]);
        let mut child = LayoutStyle::default();
        child.inherit_typography_from(&parent);
        assert_eq!(child.font_size, Some(13.0));
        assert_eq!(child.font_weight, Some(600));
        assert_eq!(child.color, parent.color);
        child.font_size = Some(18.0);
        child.inherit_typography_from(&parent);
        assert_eq!(
            child.font_size,
            Some(18.0),
            "authored size must not be overwritten"
        );
    }

    #[test]
    fn min_max_size_preserves_length_spec_without_containing_block() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "min-width: 50%; max-width: min(200px, 40%); min-height: 2em",
            None,
            None,
        );
        assert_eq!(layout.min_width, Some(LengthSpec::Percent(50.0)));
        assert!(matches!(layout.max_width, Some(LengthSpec::Min2(_, _))));
        assert_eq!(layout.min_height, Some(LengthSpec::Em(2.0)));
        assert_eq!(layout.resolved_min_width(Some(300.0), None), 150.0);
        assert_eq!(layout.resolved_max_width(Some(300.0), None), Some(120.0));
        assert_eq!(layout.resolved_min_height(None, None), 32.0);
    }

    #[test]
    fn gap_percent_preserved_without_containing_block() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("gap: 10%", None, None);
        assert_eq!(layout.gap, Some(LengthSpec::Percent(10.0)));
        assert_eq!(layout.resolved_column_gap(), 0.0);
        assert_eq!(layout.resolved_column_gap_against(Some(200.0)), 20.0);
        layout.apply_css_text("gap: 8% 12%", None, None);
        assert!(layout.gap.is_none());
        assert_eq!(layout.row_gap, Some(LengthSpec::Percent(8.0)));
        assert_eq!(layout.column_gap, Some(LengthSpec::Percent(12.0)));
        let cb = ParentBox::new(Some(100.0), Some(50.0));
        assert_eq!(layout.main_gap_against(FlexDirection::Row, cb), 12.0);
        assert_eq!(layout.cross_gap_against(FlexDirection::Row, cb), 4.0);
    }

    #[test]
    fn gap_two_value_sets_row_and_column_gap() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("gap: 8px 20px", None, None);
        assert!(layout.gap.is_none());
        assert_eq!(layout.row_gap, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.column_gap, Some(LengthSpec::Px(20.0)));
        assert_eq!(layout.main_gap(FlexDirection::Row), 20.0);
        assert_eq!(layout.cross_gap(FlexDirection::Row), 8.0);
        assert_eq!(layout.main_gap(FlexDirection::Column), 8.0);
        assert_eq!(layout.cross_gap(FlexDirection::Column), 20.0);
    }

    #[test]
    fn row_gap_longhand_overrides_uniform_gap_on_row_axis() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("gap: 10px; row-gap: 4px", None, None);
        assert_eq!(layout.gap, Some(LengthSpec::Px(10.0)));
        assert_eq!(layout.row_gap, Some(LengthSpec::Px(4.0)));
        assert_eq!(layout.resolved_row_gap(), 4.0);
        assert_eq!(layout.resolved_column_gap(), 10.0);
    }

    #[test]
    fn justify_content_space_between_preserved() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("justify-content: space-between", None, None);
        assert_eq!(layout.justify_content, JustifySpec::SpaceBetween);
    }

    #[test]
    fn omitted_flex_shrink_stays_unspecified() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("width:150px;height:40px", None, None);
        assert_eq!(layout.flex_shrink, None);
    }

    #[test]
    fn flex_initial_writes_css_shrink() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("flex: initial", None, None);
        assert_eq!(layout.flex_grow, Some(0.0));
        assert_eq!(layout.flex_shrink, Some(1.0));
        assert_eq!(layout.flex_basis, Some(LengthSpec::Auto));
    }

    #[test]
    fn flex_none_writes_zero_grow_shrink_auto_basis() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("flex: none", None, None);
        assert_eq!(layout.flex_grow, Some(0.0));
        assert_eq!(layout.flex_shrink, Some(0.0));
        assert_eq!(layout.flex_basis, Some(LengthSpec::Auto));
        assert!(!layout.grows());
    }

    #[test]
    fn flex_auto_writes_css_grow_and_shrink() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("flex: auto", None, None);
        assert_eq!(layout.flex_grow, Some(1.0));
        assert_eq!(layout.flex_shrink, Some(1.0));
        assert_eq!(layout.flex_basis, Some(LengthSpec::Auto));
        assert!(layout.grows());
    }

    #[test]
    fn flex_one_sets_grow_not_blind_width() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("flex: 1", None, None);
        assert_eq!(layout.flex_grow, Some(1.0));
        assert!(layout.grows());
        assert!(layout.width.is_none(), "flex:1 must not force width Fill");
        assert_eq!(
            layout.child_main_length(FlexDirection::Row),
            Some(LengthSpec::Fill)
        );
        assert_eq!(
            layout.child_main_length(FlexDirection::Column),
            Some(LengthSpec::Fill)
        );
    }

    #[test]
    fn flex_basis_without_width_drives_main_axis() {
        // T-L04 / sidebar class hint: `flex:0 0 220px` with no width.
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("flex:0 0 220px", None, None);
        assert_eq!(layout.flex_grow, Some(0.0));
        assert_eq!(layout.flex_shrink, Some(0.0));
        assert_eq!(layout.flex_basis, Some(LengthSpec::Px(220.0)));
        assert!(layout.width.is_none());
        assert!(
            !layout.grows(),
            "grow 0 must not map to Fill via child_main_length"
        );
        assert_eq!(
            layout.child_main_length(FlexDirection::Row),
            Some(LengthSpec::Px(220.0))
        );
    }

    #[test]
    fn flex_shorthand_omitted_shrink_is_one() {
        let mut grow_basis = LayoutStyle::default();
        grow_basis.apply_css_text("flex: 1 150px", None, None);
        assert_eq!(grow_basis.flex_grow, Some(1.0));
        assert_eq!(grow_basis.flex_shrink, Some(1.0));
        assert_eq!(grow_basis.flex_basis, Some(LengthSpec::Px(150.0)));

        let mut basis_only = LayoutStyle::default();
        basis_only.apply_css_text("flex: 100px", None, None);
        assert_eq!(basis_only.flex_grow, Some(1.0));
        assert_eq!(basis_only.flex_shrink, Some(1.0));
        assert_eq!(basis_only.flex_basis, Some(LengthSpec::Px(100.0)));

        let mut grow_shrink = LayoutStyle::default();
        grow_shrink.apply_css_text("flex: 1 2", None, None);
        assert_eq!(grow_shrink.flex_grow, Some(1.0));
        assert_eq!(grow_shrink.flex_shrink, Some(2.0));
        assert_eq!(grow_shrink.flex_basis, Some(LengthSpec::Px(0.0)));
    }

    #[test]
    fn isolation_isolate_creates_paint_stacking_context() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("isolation: isolate", None, None);
        assert!(layout.isolation);
        assert!(layout.creates_paint_stacking_context());
        layout.apply_css_text("isolation: auto", None, None);
        assert!(!layout.isolation);
        assert!(!layout.creates_paint_stacking_context());
    }

    #[test]
    fn positioned_z_index_creates_paint_stacking_context() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("z-index: 3", None, None);
        assert_eq!(layout.z_index, Some(3));
        assert!(
            !layout.creates_paint_stacking_context(),
            "static + z-index is not a stacking context"
        );
        layout.apply_css_text("position: relative", None, None);
        assert!(layout.creates_paint_stacking_context());
    }

    #[test]
    fn subgrid_is_explicit_grid_track_unsupported() {
        assert_eq!(
            parse_grid_track_list_result("subgrid", None),
            GridTrackListParse::Unsupported(GridTrackListUnsupported::Subgrid)
        );
        assert_eq!(
            parse_grid_track_list_result("subgrid [foo] [bar]", None),
            GridTrackListParse::Unsupported(GridTrackListUnsupported::Subgrid)
        );
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("display:grid;grid-template-columns:subgrid", None, None);
        assert_eq!(
            layout.grid_columns_unsupported,
            Some(GridTrackListUnsupported::Subgrid)
        );
        assert!(layout.grid_columns.is_none());
        assert!(layout.has_unsupported_grid_template());
    }

    #[test]
    fn min_width_zero_allows_shrink() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("min-width: 0", None, None);
        assert_eq!(layout.min_width, Some(LengthSpec::Px(0.0)));
        assert!(layout.allow_shrink);
    }

    #[test]
    fn height_100_percent_is_fill_chain() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("height: 100%; min-height: 100%", None, None);
        assert_eq!(layout.height, Some(LengthSpec::Fill));
    }

    #[test]
    fn overflow_y_auto_parsed() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("overflow-y: auto", None, None);
        assert_eq!(layout.overflow_y, OverflowSpec::Auto);
        assert!(layout.scrolls_y());
    }

    #[test]
    fn overflow_two_value_shorthand_is_independent() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("overflow: hidden visible", None, None);
        assert_eq!(layout.overflow_x, OverflowSpec::Hidden);
        assert_eq!(layout.overflow_y, OverflowSpec::Visible);
        let (x, _, w, h) = layout.overflow_clip_box(0.0, 0.0, 40.0, 20.0).unwrap();
        assert!((x).abs() < 0.01);
        assert!((w - 40.0).abs() < 0.01);
        assert!(h > 20.0);
    }

    #[test]
    fn grid_template_sidebar_main() {
        let tracks =
            parse_grid_template_columns("var(--sidebar-width, 220px) minmax(0, 1fr)", None)
                .unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0], GridTrack::Px(220.0));
        assert!(matches!(
            tracks[1],
            GridTrack::MinMax {
                min_px: 0.0,
                fr: 1.0,
                max_px: None,
            }
        ));
        let px_max = parse_grid_template_columns("minmax(50px, 120px) 1fr", None).unwrap();
        assert_eq!(
            px_max[0],
            GridTrack::MinMax {
                min_px: 50.0,
                fr: 1.0,
                max_px: Some(120.0),
            }
        );
    }

    #[test]
    fn unresolved_var_grid_track_degrades_to_auto_keeps_column_count() {
        // Bugbot: expand must not delete unresolved var() from the track list.
        let tracks =
            parse_grid_template_columns("var(--sidebar-width) minmax(0, 1fr)", None).unwrap();
        assert_eq!(
            tracks.len(),
            2,
            "unresolved var track must keep column count"
        );
        assert_eq!(tracks[0], GridTrack::Auto);
        assert!(matches!(
            tracks[1],
            GridTrack::MinMax {
                min_px: 0.0,
                fr: 1.0,
                max_px: None,
            }
        ));
        // Three-track: unresolved middle must not collapse neighbors.
        let three = parse_grid_template_columns("80px var(--missing-track) 1fr", None).unwrap();
        assert_eq!(three.len(), 3);
        assert_eq!(three[0], GridTrack::Px(80.0));
        assert_eq!(three[1], GridTrack::Auto);
        assert_eq!(three[2], GridTrack::Fr(1.0));
    }

    #[test]
    fn flex_row_class_hint_does_not_clobber_grid_rows_axis() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:grid;grid-template-rows:100px 1fr;gap:8px",
            None,
            None,
        );
        assert_eq!(layout.direction, Some(FlexDirection::Column));
        // Class hints run after stylesheet in rebuild_layout_style.
        layout.apply_class_layout_hints(&["flex-row".into(), "nana-row".into()]);
        assert_eq!(
            layout.direction,
            Some(FlexDirection::Column),
            "flex-row must not override authored grid-template-rows axis"
        );
        assert!(layout.grid_rows.as_ref().is_some_and(|r| r.len() == 2));
    }

    #[test]
    fn viewport_units_and_min_max_clamp_parse() {
        assert_eq!(
            LengthSpec::parse("50vh"),
            Some(LengthSpec::Viewport {
                axis: ViewportAxis::Height,
                value: 50.0
            })
        );
        assert_eq!(
            LengthSpec::parse("calc(100vh - 32px)"),
            Some(LengthSpec::CalcViewportOffset {
                axis: ViewportAxis::Height,
                value: 100.0,
                offset_px: -32.0
            })
        );
        assert_eq!(
            LengthSpec::parse("min(520px, 92vw)"),
            Some(LengthSpec::Min2(
                LengthAtom::Px(520.0),
                LengthAtom::Viewport {
                    axis: ViewportAxis::Width,
                    value: 92.0
                }
            ))
        );
        assert_eq!(
            LengthSpec::parse("min(600px, 100vw - 32px)"),
            Some(LengthSpec::Min2(
                LengthAtom::Px(600.0),
                LengthAtom::CalcViewport {
                    axis: ViewportAxis::Width,
                    value: 100.0,
                    offset_px: -32.0
                }
            ))
        );
        assert_eq!(
            LengthSpec::parse("clamp(176px, 38vw, 260px)"),
            Some(LengthSpec::Clamp3(
                LengthAtom::Px(176.0),
                LengthAtom::Viewport {
                    axis: ViewportAxis::Width,
                    value: 38.0
                },
                LengthAtom::Px(260.0)
            ))
        );
        with_active_viewport(400.0, 800.0, || {
            assert_eq!(
                LengthSpec::parse("50vh")
                    .unwrap()
                    .resolve_with(None, active_viewport()),
                Some(400.0)
            );
            assert_eq!(
                LengthSpec::parse("min(520px, 92vw)")
                    .unwrap()
                    .resolve_with(None, active_viewport()),
                Some(368.0) // 92% of 400
            );
            assert_eq!(
                LengthSpec::parse("clamp(176px, 38vw, 260px)")
                    .unwrap()
                    .resolve_with(None, active_viewport()),
                Some(176.0) // 38% of 400 = 152 → clamped to 176
            );
        });
    }

    #[test]
    fn nested_mixed_min_max_resolves_against_containing_block() {
        assert_eq!(
            LengthSpec::parse("min(1px, 2%)"),
            Some(LengthSpec::Min2(
                LengthAtom::Px(1.0),
                LengthAtom::Percent(2.0)
            ))
        );
        assert_eq!(
            LengthSpec::parse("min(10px, max(1px, 50%))"),
            Some(LengthSpec::Clamp3(
                LengthAtom::Px(1.0),
                LengthAtom::Percent(50.0),
                LengthAtom::Px(10.0)
            ))
        );
        assert_eq!(
            LengthSpec::parse("min(1px, 2%, 3px)"),
            Some(LengthSpec::Min2(
                LengthAtom::Px(1.0),
                LengthAtom::Percent(2.0)
            ))
        );
        assert_eq!(
            LengthSpec::parse("max(10px, min(50%, 30px))"),
            Some(LengthSpec::Clamp3(
                LengthAtom::Px(10.0),
                LengthAtom::Percent(50.0),
                LengthAtom::Px(30.0)
            ))
        );
        assert_eq!(
            LengthSpec::parse("calc(min(10px, max(1px, 50%)))"),
            Some(LengthSpec::Clamp3(
                LengthAtom::Px(1.0),
                LengthAtom::Percent(50.0),
                LengthAtom::Px(10.0)
            ))
        );

        let nest = LengthSpec::parse("min(10px, max(1px, 50%))").unwrap();
        assert_eq!(nest.resolve_px(Some(200.0)), Some(10.0));
        assert_eq!(nest.resolve_px(Some(10.0)), Some(5.0));
        assert_eq!(nest.resolve_px(Some(0.0)), Some(1.0));

        let nary = LengthSpec::parse("min(1px, 2%, 3px)").unwrap();
        assert_eq!(nary.resolve_px(Some(200.0)), Some(1.0));
        assert_eq!(nary.resolve_px(Some(0.0)), Some(0.0));

        let mut layout = LayoutStyle::default();
        layout.apply_css_text("width: min(80px, max(10px, 20%))", None, None);
        assert_eq!(
            layout.width.and_then(|w| w.resolve_px(Some(200.0))),
            Some(40.0)
        );

        assert_eq!(LengthSpec::parse("min(1px, 2%, 3em)"), None);
        assert_eq!(LengthSpec::parse("min(1, 2px)"), None);
        assert_eq!(LengthSpec::parse("calc(2 * 3)"), None);
    }

    #[test]
    fn document_custom_props_exclude_class_scoped() {
        let doc = collect_document_css_custom_properties(
            ":root { --radius: 8px; } .menu { --row-h: 28px; } html { --gap: 4px; }",
            "light",
        );
        assert_eq!(doc.get("--radius").map(String::as_str), Some("8px"));
        assert_eq!(doc.get("--gap").map(String::as_str), Some("4px"));
        assert!(
            !doc.contains_key("--row-h"),
            "class-scoped custom props must not enter document base"
        );
    }

    #[test]
    fn document_custom_props_honor_data_theme() {
        // Lilia page.css shape: dark defaults on `:root`, light overlay on
        // `:root[data-theme=light]`. Blind last-wins would stick on light.
        let css = r#"
:root { --bg: #181818; --text: #dddddd; }
:root[data-theme="light"] { --bg: #ffffff; --text: #1a1a1f; }
"#;
        let light = collect_document_css_custom_properties(css, "light");
        assert_eq!(light.get("--bg").map(String::as_str), Some("#ffffff"));
        assert_eq!(light.get("--text").map(String::as_str), Some("#1a1a1f"));

        let dark = collect_document_css_custom_properties(css, "dark");
        assert_eq!(dark.get("--bg").map(String::as_str), Some("#181818"));
        assert_eq!(dark.get("--text").map(String::as_str), Some("#dddddd"));

        // Selector-list OR: `:root` always matches, so light base still applies;
        // dark overlay then wins when theme=dark.
        let tokens = r#"
:root, html, [data-theme="light"] { --bg: #ffffff; }
html[data-theme="dark"], [data-theme="dark"] { --bg: #181818; }
"#;
        assert_eq!(
            collect_document_css_custom_properties(tokens, "light")
                .get("--bg")
                .map(String::as_str),
            Some("#ffffff")
        );
        assert_eq!(
            collect_document_css_custom_properties(tokens, "dark")
                .get("--bg")
                .map(String::as_str),
            Some("#181818")
        );
    }

    #[test]
    fn document_custom_props_honor_lightningcss_bundle_shape() {
        // Vite/LightningCSS companion CSS: unquoted [data-theme=light], hex
        // tokens, plus @supports P3/lab fallbacks that must not clobber.
        let css = r#"
:root{color-scheme:dark;--bg:#181818;--bg-elev:#202020}
@supports (color:color(display-p3 0 0 0)){:root{--bg:color(display-p3 .0940855 .0940855 .0940855);--bg-elev:color(display-p3 .125 .125 .125)}}
@supports (color:lab(0% 0 0)){:root{--bg:lab(8.244% 0 -.00000298023);--bg-elev:lab(12.246% 0 0)}}
:root[data-theme=light]{color-scheme:light;--bg:#fff;--bg-elev:#f3f4f6}
@supports (color:color(display-p3 0 0 0)){:root[data-theme=light]{--bg:color(display-p3 1 1 1);--bg-elev:color(display-p3 .953 .957 .965)}}
@supports (color:lab(0% 0 0)){:root[data-theme=light]{--bg:lab(100% 0 0);--bg-elev:lab(96.7% 0 0)}}
"#;
        let light = collect_document_css_custom_properties(css, "light");
        assert_eq!(light.get("--bg").map(String::as_str), Some("#fff"));
        assert_eq!(light.get("--bg-elev").map(String::as_str), Some("#f3f4f6"));
        let dark = collect_document_css_custom_properties(css, "dark");
        assert_eq!(dark.get("--bg").map(String::as_str), Some("#181818"));
        assert_eq!(dark.get("--bg-elev").map(String::as_str), Some("#202020"));
    }

    #[test]
    fn lightningcss_light_dark_polyfill_var_initial_uses_fallback() {
        // HomeContributionCard: light-dark(#eff2f5,#151b23) → dual var() polyfill.
        let dual = "var(--lightningcss-light,#eff2f5)var(--lightningcss-dark,#151b23)";
        let mut light = std::collections::BTreeMap::new();
        light.insert("--lightningcss-light".into(), "initial".into());
        light.insert("--lightningcss-dark".into(), String::new());
        light.insert("--calendar-heatmap-level-0".into(), dual.into());
        resolve_css_custom_property_map(&mut light);
        assert_eq!(
            light.get("--calendar-heatmap-level-0").map(String::as_str),
            Some("#eff2f5"),
            "light: initial side must fall back to #eff2f5, not literal 'initial'"
        );

        let mut dark = std::collections::BTreeMap::new();
        dark.insert("--lightningcss-light".into(), String::new());
        dark.insert("--lightningcss-dark".into(), "initial".into());
        dark.insert("--calendar-heatmap-level-0".into(), dual.into());
        resolve_css_custom_property_map(&mut dark);
        assert_eq!(
            dark.get("--calendar-heatmap-level-0").map(String::as_str),
            Some("#151b23"),
            "dark: initial side must fall back to #151b23"
        );

        with_active_css_vars(&light, || {
            let c = resolve_paint_color(
                "var(--calendar-heatmap-level-0, var(--calendar-heatmap-level-default))",
            )
            .expect("light heatmap fill");
            assert!((c[0] - 0xef as f32 / 255.0).abs() < 0.02);
            assert!((c[1] - 0xf2 as f32 / 255.0).abs() < 0.02);
            assert!((c[2] - 0xf5 as f32 / 255.0).abs() < 0.02);
        });
    }

    #[test]
    fn grid_template_max_content_repeat_and_percent_max() {
        let status = parse_grid_template_columns("minmax(0,1fr) max-content", None).unwrap();
        assert_eq!(status.len(), 2);
        assert!(matches!(
            status[0],
            GridTrack::MinMax {
                min_px: 0.0,
                fr: 1.0,
                max_px: None,
            }
        ));
        assert_eq!(status[1], GridTrack::Auto);

        let pending =
            parse_grid_template_columns("18px minmax(0,1fr) minmax(104px,32%)", Some(400.0))
                .unwrap();
        assert_eq!(pending[0], GridTrack::Px(18.0));
        assert_eq!(
            pending[2],
            GridTrack::MinMax {
                min_px: 104.0,
                fr: 1.0,
                max_px: Some(128.0), // 32% of 400
            }
        );

        let sync = parse_grid_template_columns("repeat(3,minmax(0,1fr))", None).unwrap();
        assert_eq!(sync.len(), 3);
        assert!(sync.iter().all(|t| matches!(
            t,
            GridTrack::MinMax {
                min_px: 0.0,
                fr: 1.0,
                max_px: None,
            }
        )));

        assert_eq!(
            parse_grid_track_list_result("repeat(auto-fit,minmax(220px,1fr))", None),
            GridTrackListParse::RepeatAuto(GridRepeatAuto {
                kind: GridTrackListUnsupported::RepeatAutoFit,
                tracks: vec![GridTrack::MinMax {
                    min_px: 220.0,
                    fr: 1.0,
                    max_px: None,
                }],
                ..Default::default()
            }),
        );
        assert_eq!(
            parse_grid_track_list_result("repeat(auto-fill, minmax(100px, 1fr))", None),
            GridTrackListParse::RepeatAuto(GridRepeatAuto {
                kind: GridTrackListUnsupported::RepeatAutoFill,
                tracks: vec![GridTrack::MinMax {
                    min_px: 100.0,
                    fr: 1.0,
                    max_px: None,
                }],
                ..Default::default()
            }),
        );
        assert!(
            parse_grid_template_columns("repeat(auto-fit,minmax(220px,1fr))", None).is_none(),
            "compat Option API returns None for auto-fit (layout expands via RepeatAuto)"
        );
        assert_eq!(
            parse_grid_track_list_result("100px repeat(auto-fit,minmax(220px,1fr))", None),
            GridTrackListParse::RepeatAuto(GridRepeatAuto {
                kind: GridTrackListUnsupported::RepeatAutoFit,
                tracks: vec![GridTrack::MinMax {
                    min_px: 220.0,
                    fr: 1.0,
                    max_px: None,
                }],
                prefix: vec![GridTrack::Px(100.0)],
                suffix: Vec::new(),
                ..Default::default()
            }),
            "mixed 100px + repeat(auto-fit) must expand against available size"
        );

        let mut auto_fit_layout = LayoutStyle::default();
        auto_fit_layout.apply_css_text(
            "display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr))",
            Some(800.0),
            None,
        );
        assert!(
            auto_fit_layout.grid_columns_unsupported.is_none(),
            "successful auto-fit is not unsupported"
        );
        assert!(auto_fit_layout.grid_columns.is_none());
        assert!(
            !auto_fit_layout.has_unsupported_grid_template(),
            "successful auto-fit must not set has_unsupported_grid_template"
        );
        let auto_fit_repeat = auto_fit_layout
            .grid_columns_repeat
            .as_ref()
            .expect("auto-fit must store GridRepeatAuto pattern");
        assert_eq!(
            auto_fit_repeat.kind,
            GridTrackListUnsupported::RepeatAutoFit
        );
        assert_eq!(
            auto_fit_repeat.tracks,
            vec![GridTrack::MinMax {
                min_px: 220.0,
                fr: 1.0,
                max_px: None,
            }]
        );

        // NanaRepoPage authoring: honest fixed-count tracks (not auto-fit).
        let repo = parse_grid_template_columns("repeat(2,minmax(240px,1fr))", None).unwrap();
        assert_eq!(repo.len(), 2);
        assert!(repo.iter().all(|t| matches!(
            t,
            GridTrack::MinMax {
                min_px: 240.0,
                fr: 1.0,
                max_px: None,
            }
        )));
        let repo_widths = resolve_grid_column_widths(&repo, 600.0, 12.0);
        assert!((repo_widths[0] - 294.0).abs() < 0.01);
        assert!((repo_widths[1] - 294.0).abs() < 0.01);

        let fit_fn = parse_grid_template_columns("fit-content(120px) 1fr", None).unwrap();
        assert_eq!(
            fit_fn[0],
            GridTrack::MinMax {
                min_px: 0.0,
                fr: 1.0,
                max_px: Some(120.0),
            }
        );
        assert_eq!(fit_fn[1], GridTrack::Fr(1.0));

        let pct = parse_grid_template_columns("25% 1fr", None).unwrap();
        assert_eq!(pct[0], GridTrack::Percent(25.0));
        assert_eq!(pct[1], GridTrack::Fr(1.0));
        let resolved = resolve_grid_column_widths(&pct, 400.0, 0.0);
        assert!((resolved[0] - 100.0).abs() < 0.01);
        assert!((resolved[1] - 300.0).abs() < 0.01);

        let auto_max = parse_grid_template_columns("16px minmax(0,auto) 1fr", None).unwrap();
        assert!(matches!(
            auto_max[1],
            GridTrack::MinMax {
                min_px: 0.0,
                fr: 1.0,
                max_px: None,
            }
        ));
    }

    #[test]
    fn inline_grid_and_grid_auto_parse_deferred() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:inline-grid;grid-template-columns:100px 1fr;\
             grid-auto-columns:minmax(0,1fr);grid-auto-rows:auto;\
             grid-auto-flow:column dense;gap:8px 12px",
            Some(400.0),
            None,
        );
        assert_eq!(layout.display, Some(DisplaySpec::InlineGrid));
        assert!(layout.display.unwrap().is_grid_container());
        assert_eq!(layout.grid_columns.as_ref().map(|c| c.len()), Some(2));
        assert_eq!(
            layout.grid_auto_columns.as_ref().map(|c| c.len()),
            Some(1),
            "grid-auto-columns parsed"
        );
        assert_eq!(
            layout.grid_auto_rows,
            Some(vec![GridTrack::Auto]),
            "grid-auto-rows:auto stored"
        );
        assert_eq!(layout.grid_auto_flow, Some(GridAutoFlow::ColumnDense));
        assert!(
            layout.has_deferred_grid_auto(),
            "grid-auto-* fields stored for layout"
        );
        // Explicit template columns stay on `grid_columns`; auto tracks are
        // implicit and consumed by 2D placement, not this list.
        assert_eq!(layout.active_grid_columns().map(|c| c.len()), Some(2));
        assert_eq!(layout.gap, None);
        assert_eq!(layout.row_gap, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.column_gap, Some(LengthSpec::Px(12.0)));
    }

    #[test]
    fn place_items_and_baseline_align() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("place-items: center; align-items: baseline", None, None);
        // Later align-items wins over place-items when both present in one block.
        assert_eq!(layout.align_items, AlignSpec::Baseline);

        let mut placed = LayoutStyle::default();
        placed.apply_css_text("place-items: center", None, None);
        assert_eq!(placed.align_items, AlignSpec::Center);

        let mut content = LayoutStyle::default();
        content.apply_css_text("place-content: center", None, None);
        // place-content → align-content + justify-content（非 align-items）
        assert_eq!(content.align_content, JustifySpec::Center);
        assert_eq!(content.justify_content, JustifySpec::Center);
        assert_eq!(content.align_items, AlignSpec::Start);
    }

    #[test]
    fn paint_transform_preserves_order_and_supports_common_affine_forms() {
        // 2D list order. Planar 3D is `transform_3d` in `css_paint_transform`.
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("transform: scale(2) translate(4px, -3px)", None, None);
        assert_eq!(
            layout.transform,
            Some(PaintTransform {
                a: 2.0,
                b: 0.0,
                c: 0.0,
                d: 2.0,
                e: 8.0,
                f: -6.0,
            })
        );
        assert_eq!(layout.unsupported_transform, None);

        layout.apply_css_text(
            "transform: rotate(90deg) scale(2, -3) skewX(10deg)",
            None,
            None,
        );
        let transform = layout.transform.expect("2D affine transform");
        assert!(transform.a.abs() < 1e-5);
        assert!((transform.b - 2.0).abs() < 1e-5);
        assert!((transform.c - 3.0).abs() < 1e-5);
        assert!((transform.d - 2.0 * 10_f32.to_radians().tan()).abs() < 1e-5);
        assert_eq!(layout.unsupported_transform, None);
    }

    #[test]
    fn flex_flow_and_align_self_place_self_parse() {
        let mut flow = LayoutStyle::default();
        flow.apply_css_text("display:flex;flex-flow:column-reverse wrap", None, None);
        assert_eq!(flow.direction, Some(FlexDirection::Column));
        assert!(flow.flex_reverse);
        assert_eq!(flow.flex_wrap, FlexWrap::Wrap);

        let mut row_flow = LayoutStyle::default();
        row_flow.apply_css_text("flex-flow: wrap-reverse row", None, None);
        assert_eq!(row_flow.direction, Some(FlexDirection::Row));
        assert!(!row_flow.flex_reverse);
        assert_eq!(row_flow.flex_wrap, FlexWrap::WrapReverse);

        let mut self_align = LayoutStyle::default();
        self_align.apply_css_text("align-self: flex-end", None, None);
        assert_eq!(self_align.align_self, Some(AlignSpec::End));
        self_align.apply_css_text("align-self: auto", None, None);
        assert_eq!(self_align.align_self, None);

        let mut ord = LayoutStyle::default();
        assert_eq!(ord.order, 0);
        ord.apply_css_text("order: -3", None, None);
        assert_eq!(ord.order, -3);
        ord.apply_css_text("order: 12", None, None);
        assert_eq!(ord.order, 12);
        // Non-integer ignored (keep prior).
        ord.apply_css_text("order: 1.5", None, None);
        assert_eq!(ord.order, 12);

        let mut place = LayoutStyle::default();
        place.apply_css_text("place-self: center end", None, None);
        assert_eq!(place.align_self, Some(AlignSpec::Center));
        assert_eq!(place.justify_self, Some(AlignSpec::End));

        let mut items = LayoutStyle::default();
        items.apply_css_text("place-items: stretch center", None, None);
        assert_eq!(items.align_items, AlignSpec::Stretch);
        assert_eq!(items.justify_items, Some(AlignSpec::Center));

        let mut content = LayoutStyle::default();
        content.apply_css_text("align-content: space-between", None, None);
        assert_eq!(content.align_content, JustifySpec::SpaceBetween);

        let mut safe = LayoutStyle::default();
        safe.apply_css_text("justify-content: safe center", None, None);
        assert_eq!(safe.justify_content, JustifySpec::Center);
    }

    #[test]
    fn extract_custom_properties_strips_important_flag() {
        let map = extract_css_custom_properties_from_decls(
            "--gap: 8px !important; --w: 80px!IMPORTANT; --plain: 4px",
        );
        assert_eq!(
            map.get("--gap").map(String::as_str),
            Some("8px"),
            "custom-prop !important must strip so var(--gap) is a length, not '8px !important'"
        );
        assert_eq!(map.get("--w").map(String::as_str), Some("80px"));
        assert_eq!(map.get("--plain").map(String::as_str), Some("4px"));

        // calc(var(--w) + 10px) fails to parse if the flag stays attached.
        with_active_css_vars(&map, || {
            let mut layout = LayoutStyle::default();
            layout.apply_css_text("width: calc(var(--w) + 10px); gap: var(--gap)", None, None);
            assert_eq!(
                layout.width,
                Some(LengthSpec::Px(90.0)),
                "width via var() + calc must resolve after stripping !important"
            );
            assert_eq!(layout.gap, Some(LengthSpec::Px(8.0)));
        });

        let doc =
            collect_document_css_custom_properties(":root { --gap: 8px !important; }", "light");
        assert_eq!(
            doc.get("--gap").map(String::as_str),
            Some("8px"),
            "document scrape must strip custom-prop !important"
        );
    }

    #[test]
    fn stylesheet_custom_props_resolve_var_without_fallback() {
        let vars = collect_css_custom_properties(
            ":root { --app-corner-radius: 16px; --radius-md: var(--app-corner-radius); --radius-sm: calc(var(--app-corner-radius) * .75); }",
        );
        assert_eq!(vars.get("--radius-md").map(String::as_str), Some("16px"));
        assert_eq!(vars.get("--radius-sm").map(String::as_str), Some("12px"));
        with_active_css_vars(&vars, || {
            let mut layout = LayoutStyle::default();
            layout.apply_css_text(
                "border-radius: var(--radius-sm); gap: var(--missing, 8px)",
                None,
                None,
            );
            let radii = layout.paint.border_radii.expect("corners");
            assert_eq!(radii[0], LengthSpec::Px(12.0));
            assert_eq!(layout.gap, Some(LengthSpec::Px(8.0)));
        });
    }

    #[test]
    fn display_contents_does_not_generate_or_omit_box() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("display:contents", None, None);
        assert_eq!(layout.display, Some(DisplaySpec::Contents));
        assert!(!layout.hidden);
        assert!(!layout.omits_box());
        assert!(!layout.generates_box());
    }

    #[test]
    fn border_radius_four_value_shorthand() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("border-radius: 4px 8px 12px 16px", None, None);
        let radii = layout.paint.border_radii.expect("four corners");
        assert_eq!(radii[0], LengthSpec::Px(4.0));
        assert_eq!(radii[1], LengthSpec::Px(8.0));
        assert_eq!(radii[2], LengthSpec::Px(12.0));
        assert_eq!(radii[3], LengthSpec::Px(16.0));
        assert!(layout.border_radius.is_none());
    }

    #[test]
    fn border_radius_100_percent_resolves_to_circle_on_square() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("border-radius: 100%", None, None);
        let radii = layout.paint.border_radii.expect("corners");
        assert_eq!(radii[0], LengthSpec::Percent(100.0));
        let resolved = layout.resolved_border_radii(100.0, 100.0);
        assert!((resolved[0] - 50.0).abs() < 0.01);
        assert!(layout.border_radius.is_none());
    }

    #[test]
    fn border_radius_percent_resolves_against_box_size() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("border-radius: 50%", None, None);
        let radii = layout.resolved_border_radii(100.0, 100.0);
        assert!((radii[0] - 50.0).abs() < 0.01);
        let rect = layout.resolved_border_radii(200.0, 100.0);
        assert!((rect[0] - 50.0).abs() < 0.01, "min(50% w, 50% h)");
    }

    #[test]
    fn box_shadow_parses_single_outset_layer() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "box-shadow: 4px 6px 8px 2px rgba(0, 0, 0, 0.25)",
            None,
            None,
        );
        let shadow = layout.paint.primary_box_shadow().expect("shadow");
        assert!((shadow.offset_x - 4.0).abs() < 0.01);
        assert!((shadow.offset_y - 6.0).abs() < 0.01);
        assert!((shadow.blur_radius - 8.0).abs() < 0.01);
        assert!((shadow.spread_radius - 2.0).abs() < 0.01);
        assert!((shadow.color[3] - 0.25).abs() < 0.01);
    }

    #[test]
    fn visibility_hidden_is_paint_only() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("visibility:hidden", None, None);
        assert_eq!(layout.paint.visibility, Some(VisibilitySpec::Hidden));
        assert!(!layout.hidden);
        assert!(!layout.omits_box());
        assert!(!layout.is_paint_visible());

        layout.apply_css_text("visibility:visible", None, None);
        assert_eq!(layout.paint.visibility, Some(VisibilitySpec::Visible));
        assert!(layout.is_paint_visible());
    }

    #[test]
    fn cursor_user_select_and_app_region_fail_closed() {
        let mut layout = LayoutStyle::default();
        let before = layout.clone();
        layout.apply_css_text(
            "cursor:pointer;user-select:none;-webkit-user-select:none;-webkit-app-region:drag;app-region:drag",
            None,
            None,
        );
        assert_eq!(
            layout, before,
            "window chrome CSS must not mutate LayoutStyle"
        );
        layout.apply_css_text("app-region:no-drag;-webkit-app-region:no-drag", None, None);
        assert_eq!(
            layout, before,
            "app-region:no-drag must not punch a drag map"
        );
        assert_eq!(layout.pointer_events, None);
    }

    #[test]
    fn visibility_visible_inside_display_none_is_stored() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("display:none;visibility:visible", None, None);
        assert_eq!(layout.paint.visibility, Some(VisibilitySpec::Visible));
        assert!(layout.omits_box());
    }

    #[test]
    fn box_shadow_parses_negative_offsets_and_spread() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("box-shadow: -4px 6px 8px -24px", None, None);
        let shadow = layout.paint.primary_box_shadow().expect("shadow");
        assert!((shadow.offset_x + 4.0).abs() < 0.01);
        assert!((shadow.offset_y - 6.0).abs() < 0.01);
        assert!((shadow.blur_radius - 8.0).abs() < 0.01);
        assert!((shadow.spread_radius + 24.0).abs() < 0.01);
    }

    #[test]
    fn box_shadow_lilia_like_negative_spread() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("box-shadow: 0 10px 30px -24px rgba(0,0,0,0.12)", None, None);
        let shadow = layout.paint.primary_box_shadow().expect("shadow");
        assert!((shadow.offset_y - 10.0).abs() < 0.01);
        assert!((shadow.blur_radius - 30.0).abs() < 0.01);
        assert!((shadow.spread_radius + 24.0).abs() < 0.01);
    }

    #[test]
    fn box_shadow_parses_inset() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("box-shadow: inset 2px 2px 4px black", None, None);
        let shadow = layout.paint.primary_box_shadow().expect("inset");
        assert!(shadow.inset);
        assert!((shadow.offset_x - 2.0).abs() < 0.01);
    }

    #[test]
    fn border_width_four_sides() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("border-width: 1px 2px 3px 4px", None, None);
        let edges = layout.resolved_border_edges();
        assert!((edges.top - 1.0).abs() < 0.01);
        assert!((edges.right - 2.0).abs() < 0.01);
        assert!((edges.bottom - 3.0).abs() < 0.01);
        assert!((edges.left - 4.0).abs() < 0.01);
        layout.apply_css_text("border-left-width: 8px", None, None);
        assert!((layout.resolved_border_edges().left - 8.0).abs() < 0.01);
        assert!((layout.resolved_border_width() - 8.0).abs() < 0.01);
    }

    #[test]
    fn border_four_side_colors_and_styles_parse() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "border-width: 1px 2px 3px 4px; border-color: red blue green yellow; border-style: solid",
            None,
            None,
        );
        let colors = layout.resolved_border_edge_colors();
        assert_eq!(colors[0], Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(colors[1], Some([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(colors[2], Some([0.0, 0.5, 0.0, 1.0]));
        assert_eq!(colors[3], Some([1.0, 1.0, 0.0, 1.0]));
        assert_eq!(
            layout.resolved_border_styles(),
            [Some(BorderStyle::Solid); 4]
        );
        assert!(layout.paints_any_border());

        layout.apply_css_text(
            "border-top-color: black; border-right-style: none",
            None,
            None,
        );
        assert_eq!(layout.border_top_color, Some([0.0, 0.0, 0.0, 1.0]));
        let edges = layout.resolved_border_edges();
        assert!((edges.right).abs() < 0.01, "none style zeros used width");
        assert!((edges.top - 1.0).abs() < 0.01);

        let mut sides = LayoutStyle::default();
        sides.apply_css_text(
            "border-top: 2px solid red; border-right: 4px solid blue",
            None,
            None,
        );
        let side_edges = sides.resolved_border_edges();
        assert!((side_edges.top - 2.0).abs() < 0.01);
        assert!((side_edges.right - 4.0).abs() < 0.01);
        assert_eq!(sides.border_top_color, Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(sides.border_right_color, Some([0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn border_four_side_shorthand_fail_closed() {
        let mut widths = LayoutStyle::default();
        widths.apply_css_text("border-width: 1px 2px bogus 4px", None, None);
        assert_eq!(
            (
                widths.border_width,
                widths.border_top_width,
                widths.border_right_width,
                widths.border_bottom_width,
                widths.border_left_width,
            ),
            (None, None, None, None, None),
            "unknown width token must not collapse 4-value to 3-value (1/2/4/2)"
        );

        let mut styles = LayoutStyle::default();
        styles.apply_css_text("border-style: solid dashed bogus dotted", None, None);
        assert_eq!(
            (
                styles.border_style,
                styles.border_top_style,
                styles.border_right_style,
                styles.border_bottom_style,
                styles.border_left_style,
            ),
            (None, None, None, None, None),
            "unknown style token must not collapse 4-value to 3-value (solid/dashed/dotted/dashed)"
        );

        let mut colors = LayoutStyle::default();
        colors.apply_css_text("border-color: red blue bogus yellow", None, None);
        assert_eq!(
            (
                colors.border_color,
                colors.border_top_color,
                colors.border_right_color,
                colors.border_bottom_color,
                colors.border_left_color,
            ),
            (None, None, None, None, None),
            "unknown color token must not collapse 4-value to 3-value (red/blue/yellow/blue)"
        );
    }

    #[test]
    fn border_style_dashed_dotted_parse() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("border: 4px dashed red", None, None);
        assert_eq!(layout.border_style, Some(BorderStyle::Dashed));
        assert_eq!(
            layout.paint_border_style_codes(),
            [BorderStyle::SHADER_DASHED; 4]
        );
        assert!(layout.paints_any_border());

        layout.apply_css_text("border-style: dotted", None, None);
        assert_eq!(
            layout.resolved_border_styles(),
            [Some(BorderStyle::Dotted); 4]
        );
        assert!(layout.paints_any_border());

        layout.apply_css_text("border-style: double", None, None);
        assert_eq!(
            layout.resolved_border_styles(),
            [Some(BorderStyle::Unsupported); 4]
        );
        assert!(!layout.paints_any_border());
        assert!((layout.resolved_border_edges().top - 4.0).abs() < 0.01);
    }

    #[test]
    fn box_shadow_color_before_lengths() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("box-shadow: red 4px 6px", None, None);
        let shadow = layout.paint.primary_box_shadow().expect("shadow");
        assert!((shadow.offset_x - 4.0).abs() < 0.01);
        assert!((shadow.offset_y - 6.0).abs() < 0.01);
        assert!((shadow.color[0] - 1.0).abs() < 0.01);
    }

    #[test]
    fn box_shadow_color_after_lengths() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("box-shadow: 4px 6px red", None, None);
        let shadow = layout.paint.primary_box_shadow().expect("shadow");
        assert!((shadow.offset_x - 4.0).abs() < 0.01);
        assert!((shadow.offset_y - 6.0).abs() < 0.01);
        assert!((shadow.color[0] - 1.0).abs() < 0.01);
    }

    #[test]
    fn box_shadow_css_beats_card_elevation_when_set() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("box-shadow: 0 4px 8px rgba(0,0,0,0.5)", None, None);
        assert!(layout.paint.primary_box_shadow().is_some());
        assert!(layout.has_surface_paint());
    }

    #[test]
    fn text_shadow_parses_offset_and_color() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("text-shadow: 2px 3px rgba(0, 0, 0, 0.5)", None, None);
        let shadow = layout.paint.text_shadow.expect("text-shadow");
        assert_eq!(shadow.offset_x, 2.0);
        assert_eq!(shadow.offset_y, 3.0);
        assert!((shadow.color[3] - 0.5).abs() < 0.01);
    }

    #[test]
    fn grid_column_start_span() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("grid-column: 1 / span 2", None, None);
        assert_eq!(layout.grid_placement.column_start, GridLine::Index(1));
        assert_eq!(layout.grid_placement.column_end, GridLine::Span(2));
        assert!(layout.grid_placement.row_start.is_auto());
        assert!(layout.grid_placement.row_end.is_auto());

        layout.apply_css_text("grid-column: none", None, None);
        assert_eq!(layout.grid_placement.column_start, GridLine::Index(1));
        assert_eq!(layout.grid_placement.column_end, GridLine::Span(2));

        layout.apply_css_text("grid-column: auto", None, None);
        assert_eq!(layout.grid_placement.column_start, GridLine::Auto);
        assert_eq!(layout.grid_placement.column_end, GridLine::Auto);
    }

    #[test]
    fn grid_row_span() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("grid-row: span 2", None, None);
        assert_eq!(layout.grid_placement.row_start, GridLine::Span(2));
        assert_eq!(layout.grid_placement.row_end, GridLine::Auto);
        assert!(layout.grid_placement.column_start.is_auto());
        assert!(layout.grid_placement.column_end.is_auto());
    }

    #[test]
    fn grid_area_four_lines() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("grid-area: 1 / 2 / 3 / 4", None, None);
        assert_eq!(layout.grid_placement.row_start, GridLine::Index(1));
        assert_eq!(layout.grid_placement.column_start, GridLine::Index(2));
        assert_eq!(layout.grid_placement.row_end, GridLine::Index(3));
        assert_eq!(layout.grid_placement.column_end, GridLine::Index(4));
    }

    #[test]
    fn grid_named_area_and_named_lines() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:grid;grid-template-columns:[start] 80px [mid] 120px [end];\
             grid-template-areas:\"header header\" \"nav main\"",
            None,
            None,
        );
        assert_eq!(layout.named_column_line("start"), Some(1));
        assert_eq!(layout.named_column_line("mid"), Some(2));
        assert_eq!(layout.named_column_line("end"), Some(3));
        assert_eq!(
            layout
                .grid_template_areas
                .as_ref()
                .and_then(|a| a.lookup("header")),
            Some((0, 0, 2, 1))
        );

        let mut item = LayoutStyle::default();
        item.apply_css_text("grid-area: header", None, None);
        assert_eq!(item.grid_placement.area.as_deref(), Some("header"));

        item.apply_css_text("grid-column: start / end", None, None);
        assert_eq!(
            item.grid_placement.column_start,
            GridLine::Name("start".into())
        );
        assert_eq!(item.grid_placement.column_end, GridLine::Name("end".into()));
    }

    #[test]
    fn auto_fit_stores_repeat_pattern() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr))",
            None,
            None,
        );
        assert!(layout.grid_columns.is_none());
        assert!(
            layout.grid_columns_unsupported.is_none(),
            "successful auto-fit is stored on grid_columns_repeat, not the unsupported flag"
        );
        assert!(!layout.has_unsupported_grid_template());
        let rep = layout
            .grid_columns_repeat
            .as_ref()
            .expect("auto-fit stores GridRepeatAuto, not just the flag");
        assert_eq!(rep.kind, GridTrackListUnsupported::RepeatAutoFit);
        assert_eq!(
            rep.tracks,
            vec![GridTrack::MinMax {
                min_px: 220.0,
                fr: 1.0,
                max_px: None,
            }]
        );
    }

    #[test]
    fn invalid_mixed_auto_fit_clears_previous_tracks() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("display:grid;grid-template-columns:80px", None, None);
        assert_eq!(layout.grid_columns, Some(vec![GridTrack::Px(80.0)]));
        layout.apply_css_text(
            "grid-template-columns:80px repeat(auto-fit, garbage)",
            None,
            None,
        );
        assert!(
            layout.grid_columns.is_none(),
            "invalid mixed auto-fit must not keep leftover 80px, got {:?}",
            layout.grid_columns
        );
        assert!(layout.grid_columns_repeat.is_none());
    }

    #[test]
    fn auto_fit_keeps_prefix_line_names() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:grid;grid-template-columns:[start] 80px repeat(auto-fit, [mid] 1fr [end])",
            None,
            None,
        );
        let names = layout
            .grid_column_line_names
            .as_ref()
            .expect("prefix + in-repeat names must reach the engine");
        assert!(
            names
                .first()
                .is_some_and(|line| line.iter().any(|n| n == "start")),
            "prefix [start] dropped: {names:?}"
        );
        assert!(
            names.iter().any(|line| line.iter().any(|n| n == "mid")),
            "in-repeat [mid] dropped: {names:?}"
        );
        let repeat = layout
            .grid_columns_repeat
            .as_ref()
            .expect("mixed auto-fit stores GridRepeatAuto");
        assert!(
            repeat
                .pattern_line_names
                .iter()
                .any(|line| line.iter().any(|n| n == "mid")),
            "pattern [mid] must be stored for layout expansion, got {:?}",
            repeat.pattern_line_names
        );
        let expanded = repeat.expand_line_names(3);
        let mid_count = expanded
            .iter()
            .filter(|line| line.iter().any(|n| n == "mid"))
            .count();
        assert_eq!(
            mid_count, 3,
            "auto-fit line names must copy per repetition, got {expanded:?}"
        );
    }

    #[test]
    fn fixed_repeat_copies_line_names_per_count() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:grid;grid-template-columns:repeat(3,[mid] 80px)",
            None,
            None,
        );
        let names = layout
            .grid_column_line_names
            .as_ref()
            .expect("fixed repeat names");
        let mid_count = names
            .iter()
            .filter(|line| line.iter().any(|n| n == "mid"))
            .count();
        assert_eq!(
            mid_count, 3,
            "repeat(3, [mid] 80px) must copy [mid] three times, got {names:?}"
        );
    }

    #[test]
    fn grid_line_nth_name_parses() {
        assert_eq!(
            parse_grid_line("foo 2"),
            Some(GridLine::NthName("foo".into(), 2))
        );
        assert_eq!(
            parse_grid_line("2 foo"),
            Some(GridLine::NthName("foo".into(), 2))
        );
    }

    #[test]
    fn display_flex_clears_grid_columns_repeat() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr))",
            None,
            None,
        );
        assert!(layout.grid_columns_repeat.is_some());
        layout.apply_css_text("display:flex", None, None);
        assert_eq!(layout.display, Some(DisplaySpec::Flex));
        assert!(layout.grid_columns_repeat.is_none());
        assert!(layout.grid_rows_repeat.is_none());
        assert!(layout.grid_columns_unsupported.is_none());
    }

    #[test]
    fn display_flex_clears_and_ignores_grid_template() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:grid;grid-template-columns:220px 1fr;grid-template-rows:1fr",
            None,
            None,
        );
        assert!(layout.grid_columns.is_some());
        layout.apply_css_text("display:flex;flex-direction:row", None, None);
        assert_eq!(layout.display, Some(DisplaySpec::Flex));
        assert!(layout.grid_columns.is_none());
        assert!(layout.grid_rows.is_none());
        // Later grid-template under flex stays inert.
        layout.apply_css_text(
            "grid-template-columns:minmax(180px,220px) minmax(320px,1fr)",
            None,
            None,
        );
        assert!(
            layout.grid_columns.is_none(),
            "grid-template-columns must not author tracks under display:flex"
        );
        assert!(layout.active_grid_columns().is_none());
    }

    #[test]
    fn grid_template_rows_sets_column_direction() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:grid;grid-template-rows:100px 1fr 1fr;gap:20px 40px",
            None,
            None,
        );
        assert_eq!(layout.direction, Some(FlexDirection::Column));
        assert_eq!(layout.resolved_row_gap(), 20.0);
        assert_eq!(layout.resolved_column_gap(), 40.0);
        let rows = layout.grid_rows.as_ref().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], GridTrack::Px(100.0));
        assert_eq!(rows[1], GridTrack::Fr(1.0));
        assert_eq!(rows[2], GridTrack::Fr(1.0));
    }

    #[test]
    fn grid_columns_win_over_rows_same_node_regardless_of_order() {
        let mut rows_first = LayoutStyle::default();
        rows_first.apply_css_text(
            "display:grid;grid-template-rows:80px 1fr;grid-template-columns:100px 1fr",
            None,
            None,
        );
        assert_eq!(rows_first.direction, Some(FlexDirection::Row));
        assert!(rows_first.grid_columns.is_some());
        assert!(rows_first.grid_rows.is_some());

        let mut cols_first = LayoutStyle::default();
        cols_first.apply_css_text(
            "display:grid;grid-template-columns:100px 1fr;grid-template-rows:80px 1fr",
            None,
            None,
        );
        assert_eq!(cols_first.direction, Some(FlexDirection::Row));
    }

    #[test]
    fn grid_template_none_switches_rows_only_and_columns_only() {
        let mut to_rows = LayoutStyle::default();
        to_rows.apply_css_text(
            "display:grid;grid-template-columns:100px 1fr;grid-template-rows:80px 1fr;grid-template-columns:none",
            None,
            None,
        );
        assert!(to_rows.grid_columns.is_none());
        assert!(to_rows.grid_rows.is_some());
        assert_eq!(to_rows.direction, Some(FlexDirection::Column));

        let mut to_cols = LayoutStyle::default();
        to_cols.apply_css_text(
            "display:grid;grid-template-rows:80px 1fr;grid-template-columns:100px 1fr;grid-template-rows:none",
            None,
            None,
        );
        assert!(to_cols.grid_rows.is_none());
        assert!(to_cols.grid_columns.is_some());
        assert_eq!(to_cols.direction, Some(FlexDirection::Row));
    }

    #[test]
    fn grid_template_both_none_falls_back_to_row() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:grid;grid-template-rows:80px 1fr;grid-template-rows:none",
            None,
            None,
        );
        assert!(layout.grid_rows.is_none());
        assert!(layout.grid_columns.is_none());
        assert_eq!(
            layout.direction,
            Some(FlexDirection::Row),
            "clearing last tracks must not leave stale Column"
        );

        let mut both = LayoutStyle::default();
        both.apply_css_text(
            "display:grid;grid-template-columns:100px 1fr;grid-template-rows:80px 1fr;grid-template-columns:none;grid-template-rows:none",
            None,
            None,
        );
        assert!(both.grid_columns.is_none());
        assert!(both.grid_rows.is_none());
        assert_eq!(both.direction, Some(FlexDirection::Row));
    }

    #[test]
    fn margin_shorthand() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("margin: 8px 4px", None, None);
        let m = layout.resolved_margin();
        assert_eq!(m.top, 8.0);
        assert_eq!(m.bottom, 8.0);
        assert_eq!(m.left, 4.0);
        assert_eq!(m.right, 4.0);
    }

    #[test]
    fn relative_position_parses_inset() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("position:relative;top:12px;left:20px", None, None);
        assert_eq!(layout.position, PositionSpec::Relative);
        assert_eq!(layout.offset_top, Some(LengthSpec::Px(12.0)));
        assert_eq!(layout.offset_left, Some(LengthSpec::Px(20.0)));
        assert_eq!(layout.relative_offset(), (20.0, 12.0));
        let mut static_layout = LayoutStyle::default();
        static_layout.apply_css_text("top:12px;left:20px", None, None);
        assert_eq!(static_layout.relative_offset(), (0.0, 0.0));
    }

    #[test]
    fn absolute_position_parses_inset_not_relative_offset() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "position:absolute;top:8px;left:100px;right:4px;bottom:2px",
            None,
            None,
        );
        assert_eq!(layout.position, PositionSpec::Absolute);
        assert!(layout.is_absolute());
        assert_eq!(layout.offset_top, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.offset_left, Some(LengthSpec::Px(100.0)));
        assert_eq!(layout.offset_right, Some(LengthSpec::Px(4.0)));
        assert_eq!(layout.offset_bottom, Some(LengthSpec::Px(2.0)));
        // relative_offset only applies to Relative.
        assert_eq!(layout.relative_offset(), (0.0, 0.0));
        assert!(!layout.position.is_unsupported_positioning());
        let mut fixed = LayoutStyle::default();
        fixed.apply_css_text("position:fixed;top:0;left:0;z-index:10", None, None);
        assert_eq!(fixed.position, PositionSpec::Fixed);
        assert!(fixed.is_fixed());
        assert!(fixed.is_out_of_flow());
        assert!(!fixed.position.is_unsupported_positioning());
        assert_eq!(fixed.z_index, Some(10));
        let mut sticky = LayoutStyle::default();
        sticky.apply_css_text("position:sticky;top:0", None, None);
        assert_eq!(sticky.position, PositionSpec::Sticky);
        assert!(!sticky.position.is_unsupported_positioning());
    }

    #[test]
    fn inset_two_and_four_value_shorthand() {
        let mut two = LayoutStyle::default();
        two.apply_css_text("position:absolute;inset:8px 24px", None, None);
        assert_eq!(two.offset_top, Some(LengthSpec::Px(8.0)));
        assert_eq!(two.offset_bottom, Some(LengthSpec::Px(8.0)));
        assert_eq!(two.offset_left, Some(LengthSpec::Px(24.0)));
        assert_eq!(two.offset_right, Some(LengthSpec::Px(24.0)));

        let mut four = LayoutStyle::default();
        four.apply_css_text("position:absolute;inset:1px 2px 3px 4px", None, None);
        assert_eq!(four.offset_top, Some(LengthSpec::Px(1.0)));
        assert_eq!(four.offset_right, Some(LengthSpec::Px(2.0)));
        assert_eq!(four.offset_bottom, Some(LengthSpec::Px(3.0)));
        assert_eq!(four.offset_left, Some(LengthSpec::Px(4.0)));
    }

    #[test]
    fn inset_mixed_percent_and_px_shorthand() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("position:absolute;inset:10% 8px", None, None);
        assert_eq!(layout.offset_top, Some(LengthSpec::Percent(10.0)));
        assert_eq!(layout.offset_bottom, Some(LengthSpec::Percent(10.0)));
        assert_eq!(layout.offset_left, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.offset_right, Some(LengthSpec::Px(8.0)));
        assert_eq!(
            LayoutStyle::resolve_inset(layout.offset_top, 100.0),
            Some(10.0)
        );
        assert_eq!(
            LayoutStyle::resolve_inset(layout.offset_left, 200.0),
            Some(8.0)
        );
    }

    #[test]
    fn inset_three_and_four_value_mixed_percent_px() {
        let mut three = LayoutStyle::default();
        three.apply_css_text("position:absolute;inset:10% 8px 12px", None, None);
        assert_eq!(three.offset_top, Some(LengthSpec::Percent(10.0)));
        assert_eq!(three.offset_left, Some(LengthSpec::Px(8.0)));
        assert_eq!(three.offset_right, Some(LengthSpec::Px(8.0)));
        assert_eq!(three.offset_bottom, Some(LengthSpec::Px(12.0)));

        let mut four = LayoutStyle::default();
        four.apply_css_text("position:absolute;inset:5% 10px 8px 15%", None, None);
        assert_eq!(four.offset_top, Some(LengthSpec::Percent(5.0)));
        assert_eq!(four.offset_right, Some(LengthSpec::Px(10.0)));
        assert_eq!(four.offset_bottom, Some(LengthSpec::Px(8.0)));
        assert_eq!(four.offset_left, Some(LengthSpec::Percent(15.0)));
    }

    #[test]
    fn margin_three_value_shorthand() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("margin:4px 8px 12px", None, None);
        assert_eq!(layout.margin_top, Some(LengthSpec::Px(4.0)));
        assert_eq!(layout.margin_left, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.margin_right, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.margin_bottom, Some(LengthSpec::Px(12.0)));
        let m = layout.resolved_margin();
        assert_eq!((m.top, m.right, m.bottom, m.left), (4.0, 8.0, 12.0, 8.0));
    }

    #[test]
    fn inset_shorthand_vertical_percent_uses_containing_block_width() {
        // % is preserved at parse; resolve against CB width (not height).
        // percent_base=200 — wrong height base (50) would yield top=5, not 20.
        let mut two = LayoutStyle::default();
        two.apply_css_text("margin:10% 5%", None, None);
        assert_eq!(two.margin_top, Some(LengthSpec::Percent(10.0)));
        assert_eq!(two.margin_bottom, Some(LengthSpec::Percent(10.0)));
        assert_eq!(two.margin_left, Some(LengthSpec::Percent(5.0)));
        assert_eq!(two.margin_right, Some(LengthSpec::Percent(5.0)));
        let m = two.resolved_margin_against(Some(200.0));
        assert_eq!((m.top, m.right, m.bottom, m.left), (20.0, 10.0, 20.0, 10.0));
        // Height-sized base must not be used for vertical margin %.
        let wrong = two.resolved_margin_against(Some(50.0));
        assert_ne!(wrong.top, 20.0);

        let mut three = LayoutStyle::default();
        three.apply_css_text("padding:10% 8px 5%", None, None);
        assert_eq!(three.padding_top, Some(LengthSpec::Percent(10.0)));
        assert_eq!(three.padding_left, Some(LengthSpec::Px(8.0)));
        assert_eq!(three.padding_right, Some(LengthSpec::Px(8.0)));
        assert_eq!(three.padding_bottom, Some(LengthSpec::Percent(5.0)));
        let p = three.resolved_padding_against(Some(200.0));
        assert_eq!((p.top, p.right, p.bottom, p.left), (20.0, 8.0, 10.0, 8.0));

        let mut four = LayoutStyle::default();
        four.apply_css_text("margin:10% 4px 5% 8px", None, None);
        assert_eq!(four.margin_top, Some(LengthSpec::Percent(10.0)));
        assert_eq!(four.margin_right, Some(LengthSpec::Px(4.0)));
        assert_eq!(four.margin_bottom, Some(LengthSpec::Percent(5.0)));
        assert_eq!(four.margin_left, Some(LengthSpec::Px(8.0)));
        let m4 = four.resolved_margin_against(Some(200.0));
        assert_eq!(
            (m4.top, m4.right, m4.bottom, m4.left),
            (20.0, 4.0, 10.0, 8.0)
        );
    }

    #[test]
    fn inset_longhand_vertical_percent_uses_containing_block_width() {
        // Same contract: store %; resolve vs width base 200 (not height 50).
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "margin-top:10%;margin-bottom:5%;padding-top:10%;padding-bottom:5%;margin-left:8%;padding-right:4%",
            None,
            None,
        );
        assert_eq!(layout.margin_top, Some(LengthSpec::Percent(10.0)));
        assert_eq!(layout.margin_bottom, Some(LengthSpec::Percent(5.0)));
        assert_eq!(layout.padding_top, Some(LengthSpec::Percent(10.0)));
        assert_eq!(layout.padding_bottom, Some(LengthSpec::Percent(5.0)));
        assert_eq!(layout.margin_left, Some(LengthSpec::Percent(8.0)));
        assert_eq!(layout.padding_right, Some(LengthSpec::Percent(4.0)));
        let m = layout.resolved_margin_against(Some(200.0));
        let p = layout.resolved_padding_against(Some(200.0));
        assert_eq!(m.top, 20.0);
        assert_eq!(m.bottom, 10.0);
        assert_eq!(m.left, 16.0);
        assert_eq!(p.top, 20.0);
        assert_eq!(p.bottom, 10.0);
        assert_eq!(p.right, 8.0);
    }

    #[test]
    fn box_edge_percent_preserved_without_containing_block() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("padding:10%;margin-left:5%", None, None);
        assert_eq!(layout.padding, Some(LengthSpec::Percent(10.0)));
        assert_eq!(layout.margin_left, Some(LengthSpec::Percent(5.0)));
        // Without CB width, % resolves to 0 (spec still retained).
        assert!(layout.resolved_padding().is_zero());
        assert_eq!(layout.resolved_margin().left, 0.0);
        assert_eq!(layout.resolved_padding_against(Some(400.0)).top, 40.0);
    }

    #[test]
    fn text_overflow_ellipsis_and_sidebar_label_class() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "text-overflow:ellipsis;white-space:nowrap;overflow:hidden;min-width:0;flex:1",
            None,
            None,
        );
        assert!(layout.text_overflow_ellipsis);
        assert!(layout.white_space_nowrap);
        assert!(layout.uses_text_ellipsis());

        let mut label = LayoutStyle::default();
        label.apply_class_layout_hints(&["nana-sidebar-row__label".into()]);
        assert!(label.uses_text_ellipsis());
        assert!(label.white_space_nowrap);
        assert_eq!(label.min_width, Some(LengthSpec::Px(0.0)));
    }

    #[test]
    fn inset_percent_parses_without_percent_base() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "position:absolute;left:10%;top:20%;right:100%;bottom:5%",
            None,
            None,
        );
        assert_eq!(layout.offset_left, Some(LengthSpec::Percent(10.0)));
        assert_eq!(layout.offset_top, Some(LengthSpec::Percent(20.0)));
        // 100% must stay Percent for inset (not LengthSpec::Fill).
        assert_eq!(layout.offset_right, Some(LengthSpec::Percent(100.0)));
        assert_eq!(layout.offset_bottom, Some(LengthSpec::Percent(5.0)));
        assert_eq!(
            LayoutStyle::resolve_inset(layout.offset_left, 200.0),
            Some(20.0)
        );
    }

    #[test]
    fn display_block_sets_column() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("display: block", None, None);
        assert_eq!(layout.display, Some(DisplaySpec::Block));
        assert_eq!(layout.direction, Some(FlexDirection::Column));
    }

    #[test]
    fn white_space_pre_and_font_metrics_parse() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:block;width:200px;font-size:16px;line-height:20px;white-space:pre",
            None,
            None,
        );
        assert_eq!(layout.white_space, WhiteSpaceSpec::Pre);
        assert_eq!(layout.font_size, Some(16.0));
        assert_eq!(layout.line_height, Some(LineHeightSpec::Absolute(20.0)));
    }

    #[test]
    fn nana_controls_settings_row_class_hints() {
        let mut layout = LayoutStyle::default();
        layout.apply_class_layout_hints(&["nana-settings-row".into()]);
        assert_eq!(layout.direction, Some(FlexDirection::Row));
        assert_eq!(layout.justify_content, JustifySpec::SpaceBetween);
        assert_eq!(layout.gap, Some(LengthSpec::Px(14.0)));
        assert_eq!(layout.align_items, AlignSpec::Center);
    }

    #[test]
    fn nana_document_root_class_hints_fill() {
        for class in ["nana-html-root", "nana-mount-root"] {
            let mut layout = LayoutStyle::default();
            layout.apply_class_layout_hints(&[class.into()]);
            assert_eq!(layout.width, Some(LengthSpec::Fill), "{class}");
            assert_eq!(layout.height, Some(LengthSpec::Fill), "{class}");
            assert_eq!(layout.direction, Some(FlexDirection::Column), "{class}");
        }
    }

    #[test]
    fn nana_workspace_shell_body_grid_220_1fr() {
        let mut layout = LayoutStyle::default();
        layout.apply_class_layout_hints(&["nana-workspace-shell__body".into()]);
        assert_eq!(layout.direction, Some(FlexDirection::Row));
        let cols = layout.grid_columns.as_ref().unwrap();
        assert_eq!(cols[0], GridTrack::Px(220.0));
        assert!(layout.grows());
    }

    #[test]
    fn sidebar_body_overflow_auto_and_flex_grow() {
        let mut layout = LayoutStyle::default();
        layout.apply_class_layout_hints(&["nana-sidebar-frame__body".into()]);
        assert!(layout.grows());
        assert_eq!(layout.overflow_y, OverflowSpec::Auto);
        assert_eq!(layout.min_height, Some(LengthSpec::Px(0.0)));
        assert_eq!(layout.width, Some(LengthSpec::Fill));
        assert_eq!(layout.height, Some(LengthSpec::Fill));
    }

    #[test]
    fn generic_css_drives_tray_grid_and_chrome_without_app_classes() {
        // Anonymous class + CSS must yield the same contracts formerly
        // hard-wired to overview-actions / home-page / sb-section / …
        let mut header = LayoutStyle::default();
        header.apply_css_text(
            "display:flex;flex-direction:row;align-items:center;gap:12px;height:48px;width:100%",
            None,
            None,
        );
        assert_eq!(header.direction, Some(FlexDirection::Row));
        assert_eq!(header.height, Some(LengthSpec::Px(48.0)));

        let mut actions = LayoutStyle::default();
        actions.apply_class_layout_hints(&["anon-tray".into()]);
        actions.apply_css_text(
            "display:inline-flex;flex-direction:row;align-items:center;gap:2px;\
             height:40px;width:auto;padding:4px;border-radius:10px;\
             border:1px solid #9ca3af;background:#f3f4f6;flex-shrink:0",
            None,
            None,
        );
        assert_eq!(actions.direction, Some(FlexDirection::Row));
        assert_eq!(actions.height, Some(LengthSpec::Px(40.0)));
        assert_eq!(actions.width, Some(LengthSpec::Auto));
        assert_eq!(actions.padding, Some(LengthSpec::Px(4.0)));
        assert_eq!(actions.border_width, Some(1.0));
        assert!(actions.background.is_some());
        let actions_radii = actions.paint.border_radii.expect("actions corners");
        assert_eq!(actions_radii[0], LengthSpec::Px(10.0));

        let mut btn = LayoutStyle::default();
        btn.apply_css_text(
            "width:32px;height:32px;border-radius:6px;flex-grow:0;flex-shrink:0",
            None,
            None,
        );
        assert_eq!(btn.width, Some(LengthSpec::Px(32.0)));
        assert_eq!(btn.height, Some(LengthSpec::Px(32.0)));
        let btn_radii = btn.paint.border_radii.expect("btn corners");
        assert_eq!(btn_radii[0], LengthSpec::Px(6.0));

        let mut primary = LayoutStyle::default();
        primary.apply_css_text(
            "width:auto;min-width:72px;height:32px;padding-left:12px;padding-right:12px;\
             border-radius:6px;flex-shrink:0",
            None,
            None,
        );
        assert_eq!(primary.width, Some(LengthSpec::Auto));
        assert_eq!(primary.min_width, Some(LengthSpec::Px(72.0)));
        assert_eq!(primary.padding_left, Some(LengthSpec::Px(12.0)));

        let mut section = LayoutStyle::default();
        section.apply_css_text(
            "display:flex;flex-direction:row;align-items:center;gap:5px;height:24px;\
             width:100%;padding:0 6px 0 8px;white-space:nowrap;flex-shrink:0",
            None,
            None,
        );
        assert_eq!(section.direction, Some(FlexDirection::Row));
        assert_eq!(section.height, Some(LengthSpec::Px(24.0)));
        assert!(section.white_space_nowrap);

        let mut sort = LayoutStyle::default();
        sort.apply_css_text(
            "display:flex;flex-direction:row;align-items:center;width:auto;height:22px;\
             padding:0 5px;white-space:nowrap;flex-grow:0;flex-shrink:0;gap:3px",
            None,
            None,
        );
        assert_eq!(sort.width, Some(LengthSpec::Auto));
        assert!(sort.white_space_nowrap);
        assert_eq!(sort.height, Some(LengthSpec::Px(22.0)));

        let mut card = LayoutStyle::default();
        card.apply_class_layout_hints(&["card".into()]);
        assert_eq!(card.border_radius, Some(16.0));
        assert_eq!(card.padding, Some(LengthSpec::Px(12.0)));
        assert!(card.border_width.is_none());
        assert!(card.border_color.is_none());
        assert!(card.background.is_none());

        let mut primary_region = LayoutStyle::default();
        primary_region.apply_class_layout_hints(&["nana-workspace-shell__primary".into()]);
        assert_eq!(primary_region.padding_top, Some(LengthSpec::Px(20.0)));
        assert_eq!(primary_region.padding_bottom, Some(LengthSpec::Px(20.0)));
        assert_eq!(primary_region.padding_left, Some(LengthSpec::Px(24.0)));

        let mut handle = LayoutStyle::default();
        handle.apply_css_text("position:absolute;width:8px;height:100%", None, None);
        assert_eq!(handle.position, PositionSpec::Absolute);

        let mut workspace = LayoutStyle::default();
        workspace.apply_class_layout_hints(&["flex-row".into()]);
        workspace.apply_css_text("gap:0", None, None);
        assert_eq!(workspace.gap, Some(LengthSpec::Px(0.0)));

        let mut gpu = LayoutStyle::default();
        gpu.apply_class_layout_hints(&["nana-gpu-preview".into()]);
        assert_eq!(gpu.width, Some(LengthSpec::Fill));
        assert_eq!(gpu.height, Some(LengthSpec::Px(100.0)));
        assert_eq!(gpu.border_radius, Some(10.0));
        assert_eq!(gpu.flex_grow, Some(0.0));

        // Navy host-marker must clear under the public nana-gpu contract.
        let mut slot = LayoutStyle::default();
        slot.background = Some([30.0 / 255.0, 41.0 / 255.0, 59.0 / 255.0, 1.0]);
        slot.height = Some(LengthSpec::Px(120.0));
        slot.apply_class_layout_hints(&["nana-gpu".into()]);
        assert!(slot.background.is_none(), "navy marker must not paint");
        assert_eq!(slot.border_radius, Some(10.0));

        // grid-template-rows is the generic path (not inventing rows from a page class).
        let mut page = LayoutStyle::default();
        page.apply_css_text(
            "display:grid;grid-template-rows:auto auto auto minmax(0,1fr);\
             gap:12px;overflow:hidden;width:100%",
            None,
            None,
        );
        let rows = page.grid_rows.as_ref().expect("grid rows from CSS");
        assert_eq!(rows.len(), 4);
        assert!(matches!(rows[3], GridTrack::MinMax { fr, .. } if (fr - 1.0).abs() < 0.01));
        assert_eq!(page.overflow_y, OverflowSpec::Hidden);
        assert_eq!(page.direction, Some(FlexDirection::Column));

        let mut wrap_grid = LayoutStyle::default();
        wrap_grid.apply_css_text(
            "display:flex;flex-direction:row;flex-wrap:wrap;gap:12px;height:100%",
            None,
            None,
        );
        assert_eq!(wrap_grid.flex_wrap, FlexWrap::Wrap);
        assert_eq!(wrap_grid.height, Some(LengthSpec::Fill));

        // Documented host contract: stretch width:auto page grids (fr tracks).
        let mut region_content = LayoutStyle::default();
        region_content.apply_class_layout_hints(&["nana-workspace-region__content".into()]);
        assert_eq!(region_content.width, Some(LengthSpec::Fill));
        assert_eq!(region_content.height, Some(LengthSpec::Fill));
        assert_eq!(region_content.align_items, AlignSpec::Stretch);
        assert_eq!(region_content.direction, Some(FlexDirection::Column));

        // Product / page BEM must not get the same hints (CSS owns those).
        let mut lilia_alias = LayoutStyle::default();
        lilia_alias.apply_class_layout_hints(&["lilia-workspace-region__content".into()]);
        assert!(lilia_alias.width.is_none());
        assert!(lilia_alias.height.is_none());
        assert!(lilia_alias.direction.is_none());

        let mut block = LayoutStyle::default();
        block.apply_css_text("display:block", None, None);
        assert_eq!(block.align_items, AlignSpec::Stretch);

        // App class names alone must not invent layout (incl. --row / horizontal).
        let mut orphan = LayoutStyle::default();
        orphan.apply_class_layout_hints(&[
            "home-page".into(),
            "overview-actions".into(),
            "sb-section__header".into(),
            "lilia-workspace".into(),
            "toolbar--row".into(),
            "panel-horizontal".into(),
        ]);
        assert!(orphan.grid_rows.is_none());
        assert!(orphan.background.is_none());
        assert!(orphan.height.is_none());
        assert!(orphan.direction.is_none());
    }

    #[test]
    fn parent_box_percent_resolve() {
        let mut child = LayoutStyle::default();
        child.apply_css_text("width: 50%", None, None);
        let parent = ParentBox::from_viewport(400.0, 300.0);
        assert_eq!(child.width.unwrap().resolve_px(parent.width), Some(200.0));
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
    fn svg_stroke_width_and_fill_none_map_to_layout() {
        let mut ring = LayoutStyle::default();
        ring.apply_css_text("fill:none;stroke:#4c8bf5;stroke-width:28px", None, None);
        assert!(ring.background.is_none(), "fill:none must clear background");
        assert_eq!(ring.border_width, Some(28.0));
        assert!(ring.border_color.is_some());
    }
}
