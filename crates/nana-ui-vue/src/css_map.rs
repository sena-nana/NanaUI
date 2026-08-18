//! L1 adapter: CSS 子集 → Nana **Style Model** 的 Layout 切片（非 CSS 引擎）。
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
//! CB，根层绘制（非 Overlay 特判）。`sticky` defer。
//!
//! **Overlay 分工**：L2 Dialog/Popover/Drawer/ContextMenu 剥离 companion CSS 的
//! `fixed`/`sticky`；匿名 Vue/CSS 的 `position:fixed` 走视口子集。
//!
//! ## margin / padding / gap
//! 边长与 gap 存 [`LengthSpec`]（px / `%` / 轻量 calc）。margin/padding `%`
//! （含上下边）相对包含块**宽度**；`column-gap` `%` 相对宽度、`row-gap` `%`
//! 相对高度（缺省回退宽度）。均在 measure / Scene 布局时解析；解析期无 CB
//! 时不得静默丢弃 `%`。
//!
//! ## 逻辑盒属性（CSS Logical Properties）
//! `padding|margin|inset-{inline|block}[-start|-end]` 在默认
//! `writing-mode: horizontal-tb` + `direction: ltr` 下映射到 physical 字段
//!（inline→left/right，block→top/bottom）。`direction:rtl` /
//! `writing-mode` 竖排 / 双向复杂链 **defer**（勿假翻轴）。
//!
//! Layout length / padding / alignment live on `LayoutStyle`; Scene host consumes them
//!（feature `scene-view`）。

pub use nana_ui_core::box_layout::{
    AlignSpec, BoxSizing, DisplaySpec, FlexDirection, FlexWrap, FontSizeContext, GridAutoFlow,
    GridTrack, GridTrackListUnsupported, JustifySpec, LayoutStyle, LengthAtom, LengthSpec,
    LineHeightSpec, OverflowSpec, PaddingSpec, PaintTransform, ParentBox, PositionSpec,
    ViewportAxis, resolve_grid_column_widths, resolve_grid_track_sizes, text_line_box_height_px,
};

/// CSS keyword / length parsing for Style Model layout enums (L1 only).
pub trait CssLayoutParse: Sized {
    fn parse(raw: &str) -> Option<Self>;
}

impl CssLayoutParse for AlignSpec {
    fn parse(raw: &str) -> Option<Self> {
        Some(match raw.trim().to_ascii_lowercase().as_str() {
            "flex-start" | "start" | "left" | "top" => Self::Start,
            // baseline ≈ start in iced row/column (no true baseline alignment).
            "baseline" | "first baseline" | "last baseline" => Self::Start,
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
            "stretch" | "normal" => Self::Start,
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
        if s.eq_ignore_ascii_case("max-content")
            || s.eq_ignore_ascii_case("min-content")
            || s.eq_ignore_ascii_case("fit-content")
        {
            return Some(Self::Shrink);
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
        if let Some(spec) = parse_viewport_length(s) {
            return Some(spec);
        }
        if let Some(spec) = parse_font_relative_length(s) {
            return Some(spec);
        }
        if let Some(p) = s.strip_suffix('%') {
            let pct = p.trim().parse::<f32>().ok()?;
            if (pct - 100.0).abs() < 0.5 {
                return Some(Self::Fill);
            }
            return Some(Self::Percent(pct.clamp(0.0, 100.0)));
        }
        let num: f32 = s
            .trim_end_matches("px")
            .trim_end_matches("PX")
            .trim()
            .parse()
            .ok()?;
        Some(Self::Px(num.max(0.0)))
    }
}

/// 轻量 calc（非 AST）：`P%±Npx` / `Npx±P%` / `P%±Q%` / `Npx±Mpx` /
/// `Nv*±Mpx` / 同单位加减 / 单值。
fn parse_calc_percent_offset(raw: &str) -> Option<LengthSpec> {
    let s = raw.trim();
    let inner = s
        .strip_prefix("calc(")
        .or_else(|| s.strip_prefix("CALC("))?
        .strip_suffix(')')?
        .trim();
    parse_additive_length_expr(inner)
}

/// Parse `A ± B` / single term into LengthSpec (px / % / viewport).
fn parse_additive_length_expr(inner: &str) -> Option<LengthSpec> {
    let inner = inner.trim();
    let bytes = inner.as_bytes();
    let mut op_at = None;
    for i in 1..bytes.len() {
        if bytes[i] == b'+' || bytes[i] == b'-' {
            let prev = bytes[i - 1];
            if prev.is_ascii_whitespace()
                || prev.is_ascii_digit()
                || prev == b'%'
                || prev == b'x'
                || prev == b'h'
                || prev == b'w'
                || prev == b'n'
                || prev == b'm'
            // em / rem
            {
                // Avoid splitting inside identifiers; require unit/digit boundary.
                op_at = Some(i);
                break;
            }
        }
    }
    let Some(op_at) = op_at else {
        return parse_length_term_to_spec(inner);
    };
    let left = inner[..op_at].trim();
    let sign = if bytes[op_at] == b'+' { 1.0 } else { -1.0 };
    let right = inner[op_at + 1..].trim();
    let l = parse_length_term_parts(left)?;
    let r = parse_length_term_parts(right)?;
    match (l, r) {
        (LengthTerm::Percent(p), LengthTerm::Px(px)) => Some(LengthSpec::CalcPercentOffset {
            percent: p,
            offset_px: sign * px,
        }),
        (LengthTerm::Px(px), LengthTerm::Percent(p)) => Some(LengthSpec::CalcPercentOffset {
            percent: sign * p,
            offset_px: px,
        }),
        (LengthTerm::Percent(p1), LengthTerm::Percent(p2)) => {
            let pct = p1 + sign * p2;
            if (pct - 100.0).abs() < 0.5 {
                Some(LengthSpec::Fill)
            } else {
                Some(LengthSpec::Percent(pct.clamp(0.0, 100.0)))
            }
        }
        (LengthTerm::Px(px1), LengthTerm::Px(px2)) => {
            Some(LengthSpec::Px((px1 + sign * px2).max(0.0)))
        }
        (LengthTerm::Viewport { axis, value }, LengthTerm::Px(px)) => {
            Some(LengthSpec::CalcViewportOffset {
                axis,
                value,
                offset_px: sign * px,
            })
        }
        (LengthTerm::Px(px), LengthTerm::Viewport { axis, value }) => {
            // Npx ± Mvh → treat as viewport ± px with flipped sign on px term when
            // viewport is on the right: px + vh = vh + px; px - vh unsupported → None.
            if sign > 0.0 {
                Some(LengthSpec::CalcViewportOffset {
                    axis,
                    value,
                    offset_px: px,
                })
            } else {
                None
            }
        }
        (
            LengthTerm::Viewport {
                axis: a1,
                value: v1,
            },
            LengthTerm::Viewport {
                axis: a2,
                value: v2,
            },
        ) if a1 == a2 => Some(LengthSpec::Viewport {
            axis: a1,
            value: (v1 + sign * v2).max(0.0),
        }),
        (LengthTerm::Em(e), LengthTerm::Px(px)) => Some(LengthSpec::CalcEmOffset {
            em: e,
            offset_px: sign * px,
        }),
        (LengthTerm::Px(px), LengthTerm::Em(e)) if sign > 0.0 => Some(LengthSpec::CalcEmOffset {
            em: e,
            offset_px: px,
        }),
        (LengthTerm::Em(e1), LengthTerm::Em(e2)) => Some(LengthSpec::Em((e1 + sign * e2).max(0.0))),
        (LengthTerm::Rem(r), LengthTerm::Px(px)) => Some(LengthSpec::CalcRemOffset {
            rem: r,
            offset_px: sign * px,
        }),
        (LengthTerm::Px(px), LengthTerm::Rem(r)) if sign > 0.0 => Some(LengthSpec::CalcRemOffset {
            rem: r,
            offset_px: px,
        }),
        (LengthTerm::Rem(r1), LengthTerm::Rem(r2)) => {
            Some(LengthSpec::Rem((r1 + sign * r2).max(0.0)))
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum LengthTerm {
    Px(f32),
    Percent(f32),
    Em(f32),
    Rem(f32),
    Viewport { axis: ViewportAxis, value: f32 },
}

fn parse_length_term_parts(raw: &str) -> Option<LengthTerm> {
    let s = raw.trim();
    if let Some(p) = parse_percent_term(s) {
        return Some(LengthTerm::Percent(p));
    }
    if let Some((axis, value)) = parse_viewport_term(s) {
        return Some(LengthTerm::Viewport { axis, value });
    }
    if let Some(spec) = parse_font_relative_length(s) {
        return match spec {
            LengthSpec::Em(v) => Some(LengthTerm::Em(v)),
            LengthSpec::Rem(v) => Some(LengthTerm::Rem(v)),
            _ => None,
        };
    }
    if let Some(px) = parse_px_term(s) {
        return Some(LengthTerm::Px(px));
    }
    None
}

fn parse_length_term_to_spec(raw: &str) -> Option<LengthSpec> {
    match parse_length_term_parts(raw)? {
        LengthTerm::Px(px) => Some(LengthSpec::Px(px.max(0.0))),
        LengthTerm::Percent(p) => {
            if (p - 100.0).abs() < 0.5 {
                Some(LengthSpec::Fill)
            } else {
                Some(LengthSpec::Percent(p.clamp(0.0, 100.0)))
            }
        }
        LengthTerm::Em(v) => Some(LengthSpec::Em(v)),
        LengthTerm::Rem(v) => Some(LengthSpec::Rem(v)),
        LengthTerm::Viewport { axis, value } => Some(LengthSpec::Viewport { axis, value }),
    }
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
    } else if let Some(n) = s.strip_suffix("vw") {
        (n, ViewportAxis::Width)
    } else {
        return None;
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
    parse_additive_length_expr(s)
}

/// `min(a,b)` / `max(a,b)` / `clamp(min,val,max)` — args are LengthAtom-capable terms.
fn parse_css_min_max_clamp(raw: &str) -> Option<LengthSpec> {
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("min(") && !lower.starts_with("minmax(") {
        let (inner, rest) = split_paren_inner(&s[4..])?;
        if !rest.trim().is_empty() {
            return None;
        }
        let (a, b) = split_css_fn_args2(inner)?;
        return Some(LengthSpec::Min2(
            parse_length_atom(a)?,
            parse_length_atom(b)?,
        ));
    }
    if lower.starts_with("max(") && !lower.starts_with("minmax(") {
        let (inner, rest) = split_paren_inner(&s[4..])?;
        if !rest.trim().is_empty() {
            return None;
        }
        let (a, b) = split_css_fn_args2(inner)?;
        return Some(LengthSpec::Max2(
            parse_length_atom(a)?,
            parse_length_atom(b)?,
        ));
    }
    if lower.starts_with("clamp(") {
        let (inner, rest) = split_paren_inner(&s[6..])?;
        if !rest.trim().is_empty() {
            return None;
        }
        let (a, b, c) = split_css_fn_args3(inner)?;
        return Some(LengthSpec::Clamp3(
            parse_length_atom(a)?,
            parse_length_atom(b)?,
            parse_length_atom(c)?,
        ));
    }
    None
}

fn split_css_fn_args2(inner: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (i, ch) in inner.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                let a = inner[..i].trim();
                let b = inner[i + 1..].trim();
                if a.is_empty() || b.is_empty() {
                    return None;
                }
                return Some((a, b));
            }
            _ => {}
        }
    }
    None
}

fn split_css_fn_args3(inner: &str) -> Option<(&str, &str, &str)> {
    let (a, rest) = split_css_fn_args2(inner)?;
    let (b, c) = split_css_fn_args2(rest)?;
    Some((a, b, c))
}

fn parse_length_atom(raw: &str) -> Option<LengthAtom> {
    let s = raw.trim();
    // Do not re-enter min/max/clamp (no nested math in L1).
    let spec = parse_calc_percent_offset(s)
        .or_else(|| parse_viewport_px_sum(s))
        .or_else(|| parse_viewport_length(s))
        .or_else(|| parse_length_term_to_spec(s))?;
    length_spec_to_atom(spec)
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
        // Nested calc / min-max atoms are not re-entered into LengthAtom.
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
    /// Stylesheet cascade hot path prefers cached
    /// [`crate::css_cascade::DeclarationEntry`] → [`Self::apply_css_property`]
    /// instead of re-splitting rule text on every match.
    fn apply_css_text(&mut self, style: &str, percent_w: Option<f32>, percent_h: Option<f32>) {
        for decl in style.split(';') {
            let decl = decl.trim();
            if decl.is_empty() {
                continue;
            }
            let Some((raw_key, raw_val)) = decl.split_once(':') else {
                continue;
            };
            self.apply_css_property(raw_key.trim(), raw_val.trim(), percent_w, percent_h);
        }
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
        // Author stylesheets often use `var(--token, fallback)` — expand fallback
        // so overflow/length keywords still parse (tokens themselves stay unresolved).
        let val_owned = expand_css_var_fallback(val);
        let val = val_owned.as_str();
        match key.as_str() {
            "display" if val.eq_ignore_ascii_case("none") => {
                self.display = Some(DisplaySpec::None);
                self.hidden = true;
            }
            "display" if val.eq_ignore_ascii_case("contents") => {}
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
                self.grid_auto_columns = None;
                self.grid_auto_rows = None;
                self.grid_auto_flow = None;
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
            "padding" => apply_box_edge_shorthand(
                val,
                &mut self.padding,
                &mut self.padding_top,
                &mut self.padding_right,
                &mut self.padding_bottom,
                &mut self.padding_left,
                parse_box_edge_length,
            ),
            // Longhand margin/padding % — including top/bottom — use containing-block width
            // at layout time (store LengthSpec; do not drop % when percent_w is None).
            "padding-top" => self.padding_top = parse_box_edge_length(val),
            "padding-right" => self.padding_right = parse_box_edge_length(val),
            "padding-bottom" => self.padding_bottom = parse_box_edge_length(val),
            "padding-left" => self.padding_left = parse_box_edge_length(val),
            // Logical padding → physical (default LTR / horizontal-tb).
            "padding-inline" => apply_logical_pair_shorthand(
                val,
                &mut self.padding_left,
                &mut self.padding_right,
                parse_box_edge_length,
            ),
            "padding-block" => apply_logical_pair_shorthand(
                val,
                &mut self.padding_top,
                &mut self.padding_bottom,
                parse_box_edge_length,
            ),
            "padding-inline-start" => self.padding_left = parse_box_edge_length(val),
            "padding-inline-end" => self.padding_right = parse_box_edge_length(val),
            "padding-block-start" => self.padding_top = parse_box_edge_length(val),
            "padding-block-end" => self.padding_bottom = parse_box_edge_length(val),
            "margin" => apply_box_edge_shorthand(
                val,
                &mut self.margin,
                &mut self.margin_top,
                &mut self.margin_right,
                &mut self.margin_bottom,
                &mut self.margin_left,
                parse_margin_length,
            ),
            "margin-top" => self.margin_top = parse_margin_length(val),
            "margin-right" => self.margin_right = parse_margin_length(val),
            "margin-bottom" => self.margin_bottom = parse_margin_length(val),
            "margin-left" => self.margin_left = parse_margin_length(val),
            // Logical margin → physical (default LTR / horizontal-tb).
            "margin-inline" => apply_logical_pair_shorthand(
                val,
                &mut self.margin_left,
                &mut self.margin_right,
                parse_margin_length,
            ),
            "margin-block" => apply_logical_pair_shorthand(
                val,
                &mut self.margin_top,
                &mut self.margin_bottom,
                parse_margin_length,
            ),
            "margin-inline-start" => self.margin_left = parse_margin_length(val),
            "margin-inline-end" => self.margin_right = parse_margin_length(val),
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
                    // Historical iced Fill marker (no finite clamp).
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
                if let Some(o) = OverflowSpec::parse(val) {
                    self.overflow_x = o;
                    self.overflow_y = o;
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
            "background" | "background-color" | "fill" => {
                let v = val.trim();
                if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("transparent") {
                    // SVG `fill: none` — clear any inherited paint so stroke rings
                    // do not get a solid fill from cascade leftovers.
                    self.background = None;
                } else if let Some(c) = resolve_paint_color(val) {
                    self.background = Some(c);
                }
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
                if let Some(v) = parse_css_length_px(val, None) {
                    self.border_radius = Some(v.max(0.0));
                }
            }
            "border-width" | "border-top-width" => {
                if let Some(v) = parse_css_length_px(val, None) {
                    self.border_width = Some(v.max(0.0));
                }
            }
            "border-color" | "border-top-color" => {
                if let Some(c) = crate::style::parse_css_color(val) {
                    self.border_color = Some(c);
                }
            }
            "border" => {
                // Minimal: "1px solid #ccc" / "1px solid rgb(...)"
                let parts: Vec<_> = val.split_whitespace().collect();
                for part in &parts {
                    if let Some(v) = parse_css_length_px(part, None) {
                        self.border_width = Some(v.max(0.0));
                    } else if let Some(c) = crate::style::parse_css_color(part) {
                        self.border_color = Some(c);
                    }
                }
            }
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
            "transform" => {
                if val.trim().eq_ignore_ascii_case("none") {
                    self.transform = None;
                    self.unsupported_transform = None;
                } else if let Some(transform) = parse_paint_transform(val) {
                    self.transform = (!transform.is_identity()).then_some(transform);
                    self.unsupported_transform = None;
                } else {
                    self.transform = None;
                    self.unsupported_transform = Some(val.trim().to_owned());
                }
            }
            "top" => self.offset_top = parse_inset_length(val),
            "right" => self.offset_right = parse_inset_length(val),
            "bottom" => self.offset_bottom = parse_inset_length(val),
            "left" => self.offset_left = parse_inset_length(val),
            "inset" => apply_position_inset_shorthand(
                val,
                &mut self.offset_top,
                &mut self.offset_right,
                &mut self.offset_bottom,
                &mut self.offset_left,
            ),
            // Logical inset → physical (default LTR / horizontal-tb).
            "inset-inline" => apply_logical_pair_shorthand(
                val,
                &mut self.offset_left,
                &mut self.offset_right,
                parse_inset_length,
            ),
            "inset-block" => apply_logical_pair_shorthand(
                val,
                &mut self.offset_top,
                &mut self.offset_bottom,
                parse_inset_length,
            ),
            "inset-inline-start" => self.offset_left = parse_inset_length(val),
            "inset-inline-end" => self.offset_right = parse_inset_length(val),
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
            }
            "white-space"
                if matches!(
                    val.to_ascii_lowercase().as_str(),
                    "normal" | "wrap" | "pre-wrap" | "pre-line"
                ) =>
            {
                self.white_space_nowrap = false;
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
                    self.grid_columns = None;
                    self.grid_columns_unsupported = None;
                    recompute_grid_axis_direction(self);
                } else if self.display.is_some_and(DisplaySpec::is_flex_container) {
                    // Inert under display:flex — do not author competing tracks.
                } else {
                    apply_grid_template_axis(self, trimmed, percent_w, true);
                }
            }
            "grid-template-rows" => {
                let trimmed = val.trim();
                if trimmed.eq_ignore_ascii_case("none") || trimmed.is_empty() {
                    self.grid_rows = None;
                    self.grid_rows_unsupported = None;
                    recompute_grid_axis_direction(self);
                } else if self.display.is_some_and(DisplaySpec::is_flex_container) {
                    // Inert under display:flex — do not author competing tracks.
                } else {
                    apply_grid_template_axis(self, trimmed, percent_h, false);
                }
            }
            // grid-auto-*: parse & store; layout (measure/iced) does **not** consume
            // (implicit tracks / auto-placement = full 2D defer). Same track grammar
            // as template; auto-fit/fill → Unsupported flag, not silent drop.
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
            "visibility" if val.eq_ignore_ascii_case("hidden") => self.hidden = true,
            "visibility" if val.eq_ignore_ascii_case("visible") => {
                // do not unhide if display:none
                if self.display != Some(DisplaySpec::None) {
                    self.hidden = false;
                }
            }
            "opacity" => {
                if let Ok(v) = val.trim().parse::<f32>() {
                    self.opacity = Some(v.clamp(0.0, 1.0));
                }
            }
            _ => {}
        }
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
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        match parts.len() {
            1 => {
                if let Ok(g) = parts[0].parse::<f32>() {
                    self.flex_grow = Some(g.max(0.0));
                    self.flex_shrink = Some(1.0);
                    self.flex_basis = Some(LengthSpec::Px(0.0));
                } else if let Some(basis) = LengthSpec::parse(parts[0]) {
                    self.flex_basis = Some(basis);
                }
            }
            2 => {
                if let Ok(g) = parts[0].parse::<f32>() {
                    self.flex_grow = Some(g.max(0.0));
                }
                if let Ok(s) = parts[1].parse::<f32>() {
                    self.flex_shrink = Some(s.max(0.0));
                } else if let Some(basis) = LengthSpec::parse(parts[1]) {
                    self.flex_basis = Some(basis);
                }
            }
            _ => {
                if let Ok(g) = parts[0].parse::<f32>() {
                    self.flex_grow = Some(g.max(0.0));
                }
                if parts.len() > 1 {
                    if let Ok(s) = parts[1].parse::<f32>() {
                        self.flex_shrink = Some(s.max(0.0));
                    }
                }
                if parts.len() > 2 {
                    self.flex_basis = LengthSpec::parse(parts[2]);
                }
            }
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
                for (k, v) in map {
                    let s = match v {
                        nana_js_engine::HostValue::String(s) => s.clone(),
                        nana_js_engine::HostValue::Number(n) => format!("{n}px"),
                        nana_js_engine::HostValue::Bool(b) => b.to_string(),
                        nana_js_engine::HostValue::Null => continue,
                        other => host_value_debug(other),
                    };
                    self.apply_css_property(k, &s, percent_w, percent_h);
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

/// Parse CSS 2D affine transforms. Function order is preserved, so
/// `scale(2) translate(4px)` produces an 8px translation.
fn parse_paint_transform(raw: &str) -> Option<PaintTransform> {
    let mut rest = raw.trim();
    let mut result = PaintTransform::default();
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let name = rest[..open].trim().to_ascii_lowercase();
        let close = rest[open + 1..].find(')')? + open + 1;
        let args = rest[open + 1..close]
            .split(|ch: char| ch == ',' || ch.is_whitespace())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        match name.as_str() {
            "translate" => {
                if !(1..=2).contains(&args.len()) {
                    return None;
                }
                let x = parse_transform_length(args.first().copied()?)?;
                let y = match args.get(1).copied() {
                    Some(value) => parse_transform_length(value)?,
                    None => 0.0,
                };
                result = result.then(translation(x, y));
            }
            "translatex" => {
                if args.len() != 1 {
                    return None;
                }
                result = result.then(translation(
                    parse_transform_length(args.first().copied()?)?,
                    0.0,
                ));
            }
            "translatey" => {
                if args.len() != 1 {
                    return None;
                }
                result = result.then(translation(
                    0.0,
                    parse_transform_length(args.first().copied()?)?,
                ));
            }
            "translate3d" if args.len() == 3 => {
                let z = parse_transform_length(args[2])?;
                if z != 0.0 {
                    return None;
                }
                result = result.then(translation(
                    parse_transform_length(args[0])?,
                    parse_transform_length(args[1])?,
                ));
            }
            "scale" | "scalex" | "scaley" => {
                let valid_len = if name == "scale" {
                    (1..=2).contains(&args.len())
                } else {
                    args.len() == 1
                };
                if !valid_len {
                    return None;
                }
                let x = args.first()?.parse::<f32>().ok()?;
                let (x, y) = match name.as_str() {
                    "scalex" => (x, 1.0),
                    "scaley" => (1.0, x),
                    _ => (
                        x,
                        match args.get(1) {
                            Some(value) => value.parse::<f32>().ok()?,
                            None => x,
                        },
                    ),
                };
                result = result.then(scaling(x, y));
            }
            "scale3d" if args.len() == 3 => {
                let x = args[0].parse::<f32>().ok()?;
                let y = args[1].parse::<f32>().ok()?;
                let z = args[2].parse::<f32>().ok()?;
                if (z - 1.0).abs() > 0.0001 {
                    return None;
                }
                result = result.then(scaling(x, y));
            }
            "matrix" if args.len() == 6 => {
                let values = args
                    .iter()
                    .map(|value| value.parse::<f32>().ok())
                    .collect::<Option<Vec<_>>>()?;
                result = result.then(PaintTransform {
                    a: values[0],
                    b: values[1],
                    c: values[2],
                    d: values[3],
                    e: values[4],
                    f: values[5],
                });
            }
            "rotate" | "rotatez" => {
                if args.len() != 1 {
                    return None;
                }
                let angle = parse_transform_angle(args.first().copied()?)?;
                let (sin, cos) = angle.sin_cos();
                result = result.then(PaintTransform {
                    a: cos,
                    b: sin,
                    c: -sin,
                    d: cos,
                    ..PaintTransform::default()
                });
            }
            "skew" => {
                if !(1..=2).contains(&args.len()) {
                    return None;
                }
                let x = parse_transform_angle(args.first().copied()?)?.tan();
                let y = match args.get(1).copied() {
                    Some(value) => parse_transform_angle(value)?,
                    None => 0.0,
                }
                .tan();
                result = result.then(PaintTransform {
                    b: y,
                    c: x,
                    ..PaintTransform::default()
                });
            }
            "skewx" => {
                if args.len() != 1 {
                    return None;
                }
                result = result.then(PaintTransform {
                    c: parse_transform_angle(args.first().copied()?)?.tan(),
                    ..PaintTransform::default()
                });
            }
            "skewy" => {
                if args.len() != 1 {
                    return None;
                }
                result = result.then(PaintTransform {
                    b: parse_transform_angle(args.first().copied()?)?.tan(),
                    ..PaintTransform::default()
                });
            }
            _ => return None,
        }
        rest = rest[close + 1..].trim_start();
    }
    [result.a, result.b, result.c, result.d, result.e, result.f]
        .into_iter()
        .all(f32::is_finite)
        .then_some(result)
}

fn translation(x: f32, y: f32) -> PaintTransform {
    PaintTransform {
        e: x,
        f: y,
        ..PaintTransform::default()
    }
}

fn scaling(x: f32, y: f32) -> PaintTransform {
    PaintTransform {
        a: x,
        d: y,
        ..PaintTransform::default()
    }
}

fn parse_transform_angle(raw: &str) -> Option<f32> {
    let raw = raw.trim().to_ascii_lowercase();
    if let Some(value) = raw.strip_suffix("deg") {
        value.trim().parse::<f32>().ok().map(f32::to_radians)
    } else if let Some(value) = raw.strip_suffix("rad") {
        value.trim().parse().ok()
    } else if let Some(value) = raw.strip_suffix("turn") {
        value
            .trim()
            .parse::<f32>()
            .ok()
            .map(|turns| turns * std::f32::consts::TAU)
    } else if let Some(value) = raw.strip_suffix("grad") {
        value
            .trim()
            .parse::<f32>()
            .ok()
            .map(|grads| grads * std::f32::consts::PI / 200.0)
    } else if raw == "0" || raw == "+0" || raw == "-0" {
        Some(0.0)
    } else {
        None
    }
}

fn parse_transform_length(raw: &str) -> Option<f32> {
    if raw == "0" || raw == "+0" || raw == "-0" {
        Some(0.0)
    } else if let Some(value) = raw
        .trim()
        .strip_suffix("px")
        .or_else(|| raw.trim().strip_suffix("PX"))
    {
        value.trim().parse().ok()
    } else {
        parse_css_length_px(raw, None)
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
        LengthSpec::Fill | LengthSpec::Shrink | LengthSpec::Auto => None,
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

/// CSS Logical Properties 1–2 值简写（默认 LTR）：`start` / `end` → 两 physical 边。
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

/// After clearing columns/rows with `none`, pick 1D axis:
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

/// 解析结果：支持轨列表 / 明确 Unsupported / 非法。
#[derive(Debug, Clone, PartialEq)]
pub enum GridTrackListParse {
    Tracks(Vec<GridTrack>),
    /// `repeat(auto-fit|auto-fill)` 等 defer 语法；**不是**解析失败、也不是 `none`。
    Unsupported(GridTrackListUnsupported),
    Invalid,
}

/// Apply `grid-template-columns` (`columns=true`) or `grid-template-rows`.
///
/// `repeat(auto-fit|fill)` → [`GridTrackListUnsupported`] 旗标；**不**发明假轨、
/// **不**静默当作未声明。布局仍只读 `grid_columns` / `grid_rows`。
fn apply_grid_template_axis(
    layout: &mut LayoutStyle,
    raw: &str,
    percent_base: Option<f32>,
    columns: bool,
) {
    match parse_grid_track_list_result(raw, percent_base) {
        GridTrackListParse::Tracks(tracks) => {
            if columns {
                layout.grid_columns = Some(tracks);
                layout.grid_columns_unsupported = None;
            } else {
                layout.grid_rows = Some(tracks);
                layout.grid_rows_unsupported = None;
            }
            if layout.display.is_none() {
                layout.display = Some(DisplaySpec::Grid);
            }
            recompute_grid_axis_direction(layout);
        }
        GridTrackListParse::Unsupported(unsup) => {
            // Explicit Unsupported — clear this axis's tracks (value is not a
            // supported list) but record why; do not pretend the property was absent.
            if columns {
                layout.grid_columns = None;
                layout.grid_columns_unsupported = Some(unsup);
            } else {
                layout.grid_rows = None;
                layout.grid_rows_unsupported = Some(unsup);
            }
            if layout.display.is_none() {
                layout.display = Some(DisplaySpec::Grid);
            }
            recompute_grid_axis_direction(layout);
        }
        GridTrackListParse::Invalid => {}
    }
}

/// Parse `grid-auto-columns` / `grid-auto-rows`（存储；布局 **不**消费 — 2D defer）。
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
        // auto-fit/fill on auto tracks: leave unchanged (also deferred).
        GridTrackListParse::Unsupported(_) | GridTrackListParse::Invalid => {}
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

/// 解析 `grid-template-columns` / `rows`（及 `grid-auto-*` 轨表）轻量子集。
///
/// 支持：`px` / `%` / `fr` / `auto` / `max-content`/`min-content`/`fit-content`、
/// `fit-content(<length-percentage>)`、`minmax(min, Nfr|px|%|auto|*-content)`、
/// `repeat(N, …)`（固定次数）。
///
/// `repeat(auto-fit|auto-fill)` → [`GridTrackListParse::Unsupported`]（勿静默丢弃）。
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
        // repeat(N, tracks…) — fixed N only；auto-fit/fill → Unsupported。
        if let Some(after) = strip_prefix_ci(rest, "repeat(") {
            let Some((inner, next)) = split_paren_inner(after) else {
                return GridTrackListParse::Invalid;
            };
            let Some((count_raw, pattern)) = inner.split_once(',') else {
                return GridTrackListParse::Invalid;
            };
            let count_raw = count_raw.trim();
            if count_raw.eq_ignore_ascii_case("auto-fit") {
                return GridTrackListParse::Unsupported(GridTrackListUnsupported::RepeatAutoFit);
            }
            if count_raw.eq_ignore_ascii_case("auto-fill") {
                return GridTrackListParse::Unsupported(GridTrackListUnsupported::RepeatAutoFill);
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
                // ≈ minmax(auto, <arg>)：柔性轨 + 像素上限（1D 子集）。
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
    if min_raw == "0" || min_raw == "0px" {
        0.0
    } else if is_content_sized_keyword(min_raw) || min_raw.eq_ignore_ascii_case("auto") {
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
    } else if let Some(px) = parse_css_length_px(token, percent_base) {
        Some(GridTrack::Px(px))
    } else {
        None
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
        std::cell::RefCell::new(std::collections::BTreeMap::new());
    static ACTIVE_VIEWPORT: std::cell::RefCell<Option<(f32, f32)>> =
        std::cell::RefCell::new(None);
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
pub fn extract_css_custom_properties_from_decls(
    decls: &str,
) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    for decl in decls.split(';') {
        let decl = decl.trim();
        let Some((raw_key, raw_val)) = decl.split_once(':') else {
            continue;
        };
        let key = raw_key.trim();
        if let Some(name) = key.strip_prefix("--") {
            if !name.is_empty() {
                map.insert(format!("--{name}"), raw_val.trim().to_string());
            }
        }
    }
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
    let Some(idx) = lower.find(key) else {
        return None;
    };
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

/// Iteratively expand `var(--x)` / simple `calc(Npx * k)` inside the custom-prop map.
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
    let Some(inner) = s
        .strip_prefix("calc(")
        .or_else(|| s.strip_prefix("CALC("))
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return input.to_string();
    };
    let inner = inner.trim();
    // calc(16px * .75) / calc(16px * 0.75)
    if let Some((left, right)) = inner.split_once('*') {
        let left = left.trim();
        let right = right.trim();
        if let (Some(px), Ok(k)) = (parse_css_length_px(left, None), right.parse::<f32>()) {
            return format!("{}px", (px * k).max(0.0));
        }
        if let (Ok(k), Some(px)) = (left.parse::<f32>(), parse_css_length_px(right, None)) {
            return format!("{}px", (px * k).max(0.0));
        }
    }
    input.to_string()
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
    if let Some(pct) = s.strip_suffix('%') {
        if let Ok(p) = pct.trim().parse::<f32>() {
            return Some((fonts.element_px * p / 100.0).max(0.0));
        }
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
    if let Some(pct) = s.strip_suffix('%') {
        if let Ok(p) = pct.trim().parse::<f32>() {
            return Some(LineHeightSpec::Relative((p / 100.0).max(0.0)));
        }
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
    LengthSpec::parse(s)?
        .resolve_with_fonts(None, active_viewport(), active_font_sizes())
        .map(|v| v) // allow negative tracking
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

/// Resolve min/max [`LengthSpec`] into the px slots stored on [`LayoutStyle`].

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
    if s.is_empty() || s.eq_ignore_ascii_case("auto") || s.eq_ignore_ascii_case("none") {
        return None;
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

        assert!(
            matches!(
                parse_grid_track_list_result("repeat(auto-fit,minmax(220px,1fr))", None),
                GridTrackListParse::Unsupported(GridTrackListUnsupported::RepeatAutoFit)
            ),
            "auto-fit must be explicit Unsupported, not silent drop"
        );
        assert!(
            matches!(
                parse_grid_track_list_result("repeat(auto-fill, minmax(100px, 1fr))", None),
                GridTrackListParse::Unsupported(GridTrackListUnsupported::RepeatAutoFill)
            ),
            "auto-fill must be explicit Unsupported"
        );
        assert!(
            parse_grid_template_columns("repeat(auto-fit,minmax(220px,1fr))", None).is_none(),
            "compat Option API returns None for Unsupported"
        );

        let mut auto_fit_layout = LayoutStyle::default();
        auto_fit_layout.apply_css_text(
            "display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr))",
            Some(800.0),
            None,
        );
        assert_eq!(
            auto_fit_layout.grid_columns_unsupported,
            Some(GridTrackListUnsupported::RepeatAutoFit)
        );
        assert!(auto_fit_layout.grid_columns.is_none());
        assert!(auto_fit_layout.has_unsupported_grid_template());

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
            "layout must mark grid-auto-* as deferred for consumers"
        );
        // 1D layout still consumes only template columns — not auto tracks.
        assert_eq!(layout.active_grid_columns().map(|c| c.len()), Some(2));
        assert_eq!(layout.gap, None);
        assert_eq!(layout.row_gap, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.column_gap, Some(LengthSpec::Px(12.0)));
    }

    #[test]
    fn place_items_and_baseline_align() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("place-items: center; align-items: baseline", None, None);
        // Later align-items wins over place-items when both present in one block…
        // apply order is declaration order: place-items then align-items → baseline≈Start.
        assert_eq!(layout.align_items, AlignSpec::Start);

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
            assert_eq!(layout.border_radius, Some(12.0));
            assert_eq!(layout.gap, Some(LengthSpec::Px(8.0)));
        });
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
        assert!(sticky.position.is_unsupported_positioning());
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
        assert_eq!(actions.border_radius, Some(10.0));

        let mut btn = LayoutStyle::default();
        btn.apply_css_text(
            "width:32px;height:32px;border-radius:6px;flex-grow:0;flex-shrink:0",
            None,
            None,
        );
        assert_eq!(btn.width, Some(LengthSpec::Px(32.0)));
        assert_eq!(btn.height, Some(LengthSpec::Px(32.0)));
        assert_eq!(btn.border_radius, Some(6.0));

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
