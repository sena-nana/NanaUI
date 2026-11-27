//! Unified CSS value primitives operating directly on `&[ComponentValue]`:
//! - Angles (`deg`, `rad`, `grad`, `turn`)
//! - Lengths & percentages (preserving negative values on all units)
//! - `calc()` / `min()` / `max()` / `clamp()` expressions (strict [`MAX_NESTING`])
//! - Linear `ThemeExpr` arithmetic (`theme(...)` and `var(--nana-*)`, strict [`MAX_NESTING`])
//! - Colors (`#hex`, `rgb()`, `rgba()`, `hsl()`, `hsla()`, `oklch()`, `hsv()`,
//!   `color(srgb-linear|scrgb ...)`, `color-mix()`, `light-dark()`, named colors,
//!   `transparent`, `currentcolor`, theme tokens; rejects empty input cleanly)
//! - 1/2/3/4-value `background-position` / `object-position`
//! - Gradient stop parsing & CSS Images 3/4 stop fixup (preserving explicit
//!   first/last positions and never dividing `px` by hardcoded `100.0`)

use nana_ui_core::theme::{BorderWidth, LineRole, RadiusTier, SpacingStep, TextWeight, TypeRole};
use nana_ui_core::{
    BackgroundPosition, CalcBinOp, CalcExpr, ControlSize, GradientStop, LengthSpec, PaintColor,
    SemanticColorMix, SemanticColorRole, StyleModelRef, ThemeAppearance, ThemeExpr, ThemeToken,
    ViewportAxis,
};

use super::MAX_NESTING;
use super::diag::CssDiag;
use super::parser::{
    BlockKind, ComponentValue, parse_component_values, split_comma_groups, split_whitespace_items,
    trim_ws,
};
use super::tokenizer::{SpannedToken, Token};

// ============================================================================
// 1. Unified Angle Parser
// ============================================================================

/// Parse a single CSS `<angle>` component value into degrees (`deg`, `rad`, `grad`, `turn`).
pub fn parse_angle_token_deg(cv: &ComponentValue) -> Option<f32> {
    let Token::Dimension { value, unit } = cv.as_token()? else {
        return None;
    };
    if !value.is_finite() {
        return None;
    }
    let scale = match unit.to_ascii_lowercase().as_str() {
        "deg" => 1.0,
        "rad" => 180.0 / std::f64::consts::PI,
        "grad" => 0.9,
        "turn" => 360.0,
        _ => return None,
    };
    let deg = (*value * scale) as f32;
    deg.is_finite().then_some(deg)
}

/// Parse a CSS `<angle>` slice into degrees (`deg`, `rad`, `grad`, `turn`).
///
/// Rejects unitless non-zero numbers; pass `allow_zero_unitless = true` via
/// [`parse_angle_deg_or_zero`] where CSS permits unitless `0`.
pub fn parse_angle_deg(values: &[ComponentValue]) -> Option<f32> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    parse_angle_token_deg(cv)
}

/// Like [`parse_angle_deg`], additionally accepting unitless `0` as `0.0deg`.
pub fn parse_angle_deg_or_zero(values: &[ComponentValue]) -> Option<f32> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    if let Some(Token::Number { value, .. }) = cv.as_token()
        && *value == 0.0
    {
        return Some(0.0);
    }
    parse_angle_token_deg(cv)
}

/// Parse a CSS `<hue>` (`<angle>` or unitless degrees) wrapped to `[0, 360)`.
pub fn parse_hue_token_deg(cv: &ComponentValue) -> Option<f32> {
    if let Some(deg) = parse_angle_token_deg(cv) {
        return Some(wrap_hue(deg));
    }
    if let Some(Token::Number { value, .. }) = cv.as_token()
        && value.is_finite()
    {
        return Some(wrap_hue(*value as f32));
    }
    None
}

#[inline]
fn wrap_hue(deg: f32) -> f32 {
    let m = deg % 360.0;
    if m < 0.0 { m + 360.0 } else { m }
}

// ============================================================================
// 2. Unified Length, Percentage & Calc Parser
// ============================================================================

/// Parse a single non-function `<length-percentage>` token into [`LengthSpec`].
///
/// Negative values are preserved verbatim on ALL units (`px`, `%`, `em`, `rem`,
/// `vw`, `vh`, `vmin`, `vmax`, `pt`, `pc`, `in`, `cm`, `mm`, `q`) — never clamped.
pub fn parse_length_token(cv: &ComponentValue) -> Option<LengthSpec> {
    match cv.as_token()? {
        Token::Number { value, .. } if *value == 0.0 => Some(LengthSpec::Px(0.0)),
        Token::Percentage(p) if p.is_finite() => Some(LengthSpec::Percent(*p as f32)),
        Token::Dimension { value, unit } if value.is_finite() => {
            let v = *value as f32;
            match unit.to_ascii_lowercase().as_str() {
                "px" => Some(LengthSpec::Px(v)),
                "pt" => Some(LengthSpec::Px(v * (96.0 / 72.0))),
                "pc" => Some(LengthSpec::Px(v * 16.0)),
                "in" => Some(LengthSpec::Px(v * 96.0)),
                "cm" => Some(LengthSpec::Px(v * (96.0 / 2.54))),
                "mm" => Some(LengthSpec::Px(v * (96.0 / 25.4))),
                "q" => Some(LengthSpec::Px(v * (96.0 / 101.6))),
                "em" => Some(LengthSpec::Em(v)),
                "rem" => Some(LengthSpec::Rem(v)),
                "vw" => Some(LengthSpec::Viewport {
                    axis: ViewportAxis::Width,
                    value: v,
                }),
                "vh" => Some(LengthSpec::Viewport {
                    axis: ViewportAxis::Height,
                    value: v,
                }),
                "vmin" => Some(LengthSpec::Viewport {
                    axis: ViewportAxis::Min,
                    value: v,
                }),
                "vmax" => Some(LengthSpec::Viewport {
                    axis: ViewportAxis::Max,
                    value: v,
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Parse a CSS `<length-percentage>` value (atomic length, `%`, `0`, or
/// `calc()` / `min()` / `max()` / `clamp()`) into [`LengthSpec`].
///
/// Preserves negative lengths & percentages without clamping.
pub fn parse_length_spec(values: &[ComponentValue]) -> Option<LengthSpec> {
    let trimmed = trim_ws(values);
    let [cv] = trimmed else {
        return None;
    };
    if let Some(simple) = parse_length_token(cv) {
        return Some(simple);
    }
    if is_math_fn(cv) {
        let expr = parse_calc_expr(trimmed)?;
        return Some(LengthSpec::from_calc(expr));
    }
    None
}

/// Like [`parse_length_spec`], additionally accepting intrinsic sizing keywords
/// (`auto`, `min-content`, `max-content`, `fit-content`).
pub fn parse_size_length_spec(values: &[ComponentValue]) -> Option<LengthSpec> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && let Some(ident) = cv.as_ident()
    {
        return match ident.to_ascii_lowercase().as_str() {
            "auto" => Some(LengthSpec::Auto),
            "min-content" => Some(LengthSpec::MinContent),
            "max-content" => Some(LengthSpec::MaxContent),
            "fit-content" => Some(LengthSpec::FitContent),
            _ => None,
        };
    }
    parse_length_spec(trimmed)
}

fn is_math_fn(cv: &ComponentValue) -> bool {
    matches!(
        cv,
        ComponentValue::Function { name, .. }
            if name.eq_ignore_ascii_case("calc")
                || name.eq_ignore_ascii_case("min")
                || name.eq_ignore_ascii_case("max")
                || name.eq_ignore_ascii_case("clamp")
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CalcValType {
    Number,
    Length,
}

/// Parse a CSS math function (`calc(...)`, `min(...)`, `max(...)`, `clamp(...)`)
/// into a type-checked [`CalcExpr`], strictly bounded by [`MAX_NESTING`] across
/// functions, parenthesized sub-expressions, and unary `+/-` chains.
pub fn parse_calc_expr(values: &[ComponentValue]) -> Option<CalcExpr> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    let (expr, ty) = parse_calc_math_fn(cv, 0)?;
    (ty == CalcValType::Length).then_some(expr)
}

/// Parse a raw `calc()` / `min()` / `max()` / `clamp()` expression returning
/// either a numeric or length [`CalcExpr`].
pub fn parse_calc_expr_any(values: &[ComponentValue]) -> Option<CalcExpr> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    parse_calc_math_fn(cv, 0).map(|(e, _)| e)
}

fn parse_calc_math_fn(cv: &ComponentValue, depth: usize) -> Option<(CalcExpr, CalcValType)> {
    if depth >= MAX_NESTING {
        return None;
    }
    let ComponentValue::Function { name, args, .. } = cv else {
        return None;
    };
    match name.to_ascii_lowercase().as_str() {
        "calc" => parse_calc_sum_slice(args, depth + 1),
        "min" | "max" => {
            let is_min = name.eq_ignore_ascii_case("min");
            let groups = split_comma_groups(args);
            if groups.is_empty() {
                return None;
            }
            let (mut acc, ty) = parse_calc_sum_slice(groups[0], depth + 1)?;
            for g in &groups[1..] {
                let (next, next_ty) = parse_calc_sum_slice(g, depth + 1)?;
                if next_ty != ty {
                    return None;
                }
                acc = if is_min {
                    CalcExpr::Min(Box::new(acc), Box::new(next))
                } else {
                    CalcExpr::Max(Box::new(acc), Box::new(next))
                };
            }
            Some((acc, ty))
        }
        "clamp" => {
            let groups = split_comma_groups(args);
            if groups.len() != 3 {
                return None;
            }
            let (min_e, t1) = parse_calc_sum_slice(groups[0], depth + 1)?;
            let (val_e, t2) = parse_calc_sum_slice(groups[1], depth + 1)?;
            let (max_e, t3) = parse_calc_sum_slice(groups[2], depth + 1)?;
            if t1 != t2 || t2 != t3 {
                return None;
            }
            Some((
                CalcExpr::Clamp {
                    min: Box::new(min_e),
                    val: Box::new(val_e),
                    max: Box::new(max_e),
                },
                t1,
            ))
        }
        _ => None,
    }
}

fn parse_calc_sum_slice(
    values: &[ComponentValue],
    depth: usize,
) -> Option<(CalcExpr, CalcValType)> {
    if depth > MAX_NESTING {
        return None;
    }
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }
    let mut cursor = CalcCursor {
        values: trimmed,
        pos: 0,
    };
    let result = cursor.parse_sum(depth)?;
    cursor.skip_ws();
    (cursor.pos == cursor.values.len()).then_some(result)
}

struct CalcCursor<'a> {
    values: &'a [ComponentValue],
    pos: usize,
}

impl CalcCursor<'_> {
    fn skip_ws(&mut self) -> bool {
        let start = self.pos;
        while self.pos < self.values.len() && self.values[self.pos].is_whitespace() {
            self.pos += 1;
        }
        self.pos > start
    }

    fn parse_sum(&mut self, depth: usize) -> Option<(CalcExpr, CalcValType)> {
        let (mut lhs, lhs_ty) = self.parse_product(depth)?;
        loop {
            let ws_before = self.skip_ws();
            let Some(op_cv) = self.values.get(self.pos) else {
                break;
            };
            let op = match op_cv.as_token() {
                Some(Token::Delim('+')) => CalcBinOp::Add,
                Some(Token::Delim('-')) => CalcBinOp::Sub,
                _ => break,
            };
            // CSS Values 4 §10.6: binary `+` and `-` must be surrounded by whitespace.
            if !ws_before {
                return None;
            }
            self.pos += 1;
            let ws_after = self.skip_ws();
            if !ws_after {
                return None;
            }
            let (rhs, rhs_ty) = self.parse_product(depth)?;
            if lhs_ty != rhs_ty {
                return None;
            }
            lhs = CalcExpr::Binary {
                op,
                left: Box::new(lhs),
                right: Box::new(rhs),
            };
        }
        Some((lhs, lhs_ty))
    }

    fn parse_product(&mut self, depth: usize) -> Option<(CalcExpr, CalcValType)> {
        let (mut lhs, mut lhs_ty) = self.parse_unary(depth)?;
        loop {
            let saved = self.pos;
            self.skip_ws();
            let Some(op_cv) = self.values.get(self.pos) else {
                self.pos = saved;
                break;
            };
            let op = match op_cv.as_token() {
                Some(Token::Delim('*')) => CalcBinOp::Mul,
                Some(Token::Delim('/')) => CalcBinOp::Div,
                _ => {
                    self.pos = saved;
                    break;
                }
            };
            self.pos += 1;
            self.skip_ws();
            let (rhs, rhs_ty) = self.parse_unary(depth)?;
            match op {
                CalcBinOp::Mul => {
                    lhs_ty = match (lhs_ty, rhs_ty) {
                        (CalcValType::Number, CalcValType::Number) => CalcValType::Number,
                        (CalcValType::Length, CalcValType::Number)
                        | (CalcValType::Number, CalcValType::Length) => CalcValType::Length,
                        (CalcValType::Length, CalcValType::Length) => return None,
                    };
                }
                CalcBinOp::Div => {
                    if rhs_ty != CalcValType::Number {
                        return None;
                    }
                    if matches!(rhs, CalcExpr::Number(n) if n == 0.0) {
                        return None;
                    }
                }
                _ => unreachable!(),
            }
            lhs = CalcExpr::Binary {
                op,
                left: Box::new(lhs),
                right: Box::new(rhs),
            };
        }
        Some((lhs, lhs_ty))
    }

    /// Iterative bounded unary `+/-` prefix loop (stack-safe against long chains).
    fn parse_unary(&mut self, depth: usize) -> Option<(CalcExpr, CalcValType)> {
        self.skip_ws();
        let mut neg = false;
        let mut unary_count = 0usize;
        while let Some(cv) = self.values.get(self.pos) {
            match cv.as_token() {
                Some(Token::Delim('+')) => {
                    unary_count += 1;
                    if depth + unary_count > MAX_NESTING {
                        return None;
                    }
                    self.pos += 1;
                    self.skip_ws();
                }
                Some(Token::Delim('-')) => {
                    unary_count += 1;
                    if depth + unary_count > MAX_NESTING {
                        return None;
                    }
                    neg = !neg;
                    self.pos += 1;
                    self.skip_ws();
                }
                _ => break,
            }
        }
        let (expr, ty) = self.parse_atom(depth + unary_count)?;
        if neg {
            Some((negate_calc_expr(expr), ty))
        } else {
            Some((expr, ty))
        }
    }

    fn parse_atom(&mut self, depth: usize) -> Option<(CalcExpr, CalcValType)> {
        if depth > MAX_NESTING {
            return None;
        }
        self.skip_ws();
        let cv = self.values.get(self.pos)?;
        match cv {
            ComponentValue::SimpleBlock {
                kind: BlockKind::Paren,
                values: inner,
                ..
            } => {
                self.pos += 1;
                parse_calc_sum_slice(inner, depth + 1)
            }
            ComponentValue::Function { .. } if is_math_fn(cv) => {
                self.pos += 1;
                parse_calc_math_fn(cv, depth)
            }
            ComponentValue::Token(SpannedToken { token, .. }) => {
                self.pos += 1;
                match token {
                    Token::Number { value, .. } if value.is_finite() => {
                        Some((CalcExpr::Number(*value as f32), CalcValType::Number))
                    }
                    Token::Percentage(p) if p.is_finite() => {
                        Some((CalcExpr::Percent(*p as f32), CalcValType::Length))
                    }
                    Token::Dimension { value, unit } if value.is_finite() => {
                        let v = *value as f32;
                        let expr = match unit.to_ascii_lowercase().as_str() {
                            "px" => CalcExpr::Px(v),
                            "pt" => CalcExpr::Px(v * (96.0 / 72.0)),
                            "pc" => CalcExpr::Px(v * 16.0),
                            "in" => CalcExpr::Px(v * 96.0),
                            "cm" => CalcExpr::Px(v * (96.0 / 2.54)),
                            "mm" => CalcExpr::Px(v * (96.0 / 25.4)),
                            "q" => CalcExpr::Px(v * (96.0 / 101.6)),
                            "em" => CalcExpr::Em(v),
                            "rem" => CalcExpr::Rem(v),
                            "vw" => CalcExpr::Viewport {
                                axis: ViewportAxis::Width,
                                value: v,
                            },
                            "vh" => CalcExpr::Viewport {
                                axis: ViewportAxis::Height,
                                value: v,
                            },
                            "vmin" => CalcExpr::Viewport {
                                axis: ViewportAxis::Min,
                                value: v,
                            },
                            "vmax" => CalcExpr::Viewport {
                                axis: ViewportAxis::Max,
                                value: v,
                            },
                            _ => return None,
                        };
                        Some((expr, CalcValType::Length))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

fn negate_calc_expr(expr: CalcExpr) -> CalcExpr {
    match expr {
        CalcExpr::Number(v) => CalcExpr::Number(-v),
        CalcExpr::Px(v) => CalcExpr::Px(-v),
        CalcExpr::Percent(v) => CalcExpr::Percent(-v),
        CalcExpr::Em(v) => CalcExpr::Em(-v),
        CalcExpr::Rem(v) => CalcExpr::Rem(-v),
        CalcExpr::Viewport { axis, value } => CalcExpr::Viewport {
            axis,
            value: -value,
        },
        other => CalcExpr::Neg(Box::new(other)),
    }
}

// ============================================================================
// 3. Unified ThemeExpr Linear Arithmetic Parser
// ============================================================================

/// Resolve a NanaUI layout theme token name (with or without `--nana-` prefix).
pub fn parse_theme_token_name(raw: &str) -> Option<ThemeToken> {
    let lower = raw.trim().to_ascii_lowercase();
    let name = lower.strip_prefix("--nana-").unwrap_or(&lower);
    Some(match name {
        "space-xxs" => ThemeToken::Space(SpacingStep::Xxs),
        "space-xs" => ThemeToken::Space(SpacingStep::Xs),
        "space-sm" => ThemeToken::Space(SpacingStep::Sm),
        "space-md" => ThemeToken::Space(SpacingStep::Md),
        "space-lg" => ThemeToken::Space(SpacingStep::Lg),
        "space-xl" => ThemeToken::Space(SpacingStep::Xl),
        "space-xxl" => ThemeToken::Space(SpacingStep::Xxl),
        "space-xxxl" => ThemeToken::Space(SpacingStep::Xxxl),
        "space-page-tight" => ThemeToken::Space(SpacingStep::PageTight),
        "space-page" => ThemeToken::Space(SpacingStep::Page),
        "type-hint" => ThemeToken::Type(TypeRole::Hint),
        "type-meta" => ThemeToken::Type(TypeRole::Meta),
        "type-body" => ThemeToken::Type(TypeRole::Body),
        "type-section" => ThemeToken::Type(TypeRole::Section),
        "type-card-title" => ThemeToken::Type(TypeRole::CardTitle),
        "type-heading" => ThemeToken::Type(TypeRole::Heading),
        "type-title" => ThemeToken::Type(TypeRole::Title),
        "type-display" => ThemeToken::Type(TypeRole::Display),
        "weight-regular" => ThemeToken::Weight(TextWeight::Regular),
        "weight-medium" => ThemeToken::Weight(TextWeight::Medium),
        "weight-semibold" => ThemeToken::Weight(TextWeight::Semibold),
        "weight-bold" => ThemeToken::Weight(TextWeight::Bold),
        "line-line" => ThemeToken::Line(LineRole::Line),
        "line-tall" => ThemeToken::Line(LineRole::Tall),
        "border-hairline" => ThemeToken::Border(BorderWidth::Hairline),
        "radius-xs" => ThemeToken::Radius(RadiusTier::Xs),
        "radius-sm" => ThemeToken::Radius(RadiusTier::Sm),
        "radius-md" => ThemeToken::Radius(RadiusTier::Md),
        "radius-lg" => ThemeToken::Radius(RadiusTier::Lg),
        "radius-xl" => ThemeToken::Radius(RadiusTier::Xl),
        "radius-xxl" => ThemeToken::Radius(RadiusTier::Xxl),
        "control-small" => ThemeToken::Control(ControlSize::Small),
        "control-medium" => ThemeToken::Control(ControlSize::Medium),
        "control-large" => ThemeToken::Control(ControlSize::Large),
        _ => return None,
    })
}

/// Parse linear token arithmetic (`theme(space-md) * 2 + 4px`,
/// `calc(var(--nana-space-lg) + 8px)`, etc.) directly on `&[ComponentValue]`,
/// strictly bounded by [`MAX_NESTING`] (zero unbounded recursion on `calc(`, `(`,
/// or unary `+-`).
///
/// Returns `None` if no theme token term is present or if any term is non-linear
/// or non-finite.
pub fn parse_theme_expr(values: &[ComponentValue]) -> Option<ThemeExpr> {
    let result = parse_theme_sum_slice(values, 0)?;
    (!result.terms.is_empty()
        && result.constant.is_finite()
        && result.terms.iter().all(|(_, k)| k.is_finite()))
    .then_some(result)
}

fn parse_theme_sum_slice(values: &[ComponentValue], depth: usize) -> Option<ThemeExpr> {
    if depth > MAX_NESTING {
        return None;
    }
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }
    let mut cursor = ThemeCursor {
        values: trimmed,
        pos: 0,
    };
    let expr = cursor.parse_sum(depth)?;
    cursor.skip_ws();
    (cursor.pos == cursor.values.len()).then_some(expr)
}

struct ThemeCursor<'a> {
    values: &'a [ComponentValue],
    pos: usize,
}

impl ThemeCursor<'_> {
    fn skip_ws(&mut self) {
        while self.pos < self.values.len() && self.values[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn parse_sum(&mut self, depth: usize) -> Option<ThemeExpr> {
        let mut lhs = self.parse_product(depth)?;
        loop {
            self.skip_ws();
            let Some(cv) = self.values.get(self.pos) else {
                break;
            };
            match cv.as_token() {
                Some(Token::Delim('+')) => {
                    self.pos += 1;
                    lhs = lhs.plus(self.parse_product(depth)?);
                }
                Some(Token::Delim('-')) => {
                    self.pos += 1;
                    lhs = lhs.plus(self.parse_product(depth)?.scaled(-1.0));
                }
                _ => break,
            }
        }
        Some(lhs)
    }

    fn parse_product(&mut self, depth: usize) -> Option<ThemeExpr> {
        let mut lhs = self.parse_unary(depth)?;
        loop {
            self.skip_ws();
            let Some(cv) = self.values.get(self.pos) else {
                break;
            };
            match cv.as_token() {
                Some(Token::Delim('*')) => {
                    self.pos += 1;
                    let rhs = self.parse_unary(depth)?;
                    lhs = if rhs.terms.is_empty() {
                        lhs.scaled(rhs.constant)
                    } else if lhs.terms.is_empty() {
                        rhs.scaled(lhs.constant)
                    } else {
                        return None;
                    };
                }
                Some(Token::Delim('/')) => {
                    self.pos += 1;
                    let rhs = self.parse_unary(depth)?;
                    if !rhs.terms.is_empty() || rhs.constant == 0.0 || !rhs.constant.is_finite() {
                        return None;
                    }
                    lhs = lhs.scaled(1.0 / rhs.constant);
                }
                _ => break,
            }
        }
        Some(lhs)
    }

    /// Iterative bounded unary `+/-` loop (stack-safe against long chains).
    fn parse_unary(&mut self, depth: usize) -> Option<ThemeExpr> {
        self.skip_ws();
        let mut neg = false;
        let mut unary_count = 0usize;
        while let Some(cv) = self.values.get(self.pos) {
            match cv.as_token() {
                Some(Token::Delim('+')) => {
                    unary_count += 1;
                    if depth + unary_count > MAX_NESTING {
                        return None;
                    }
                    self.pos += 1;
                    self.skip_ws();
                }
                Some(Token::Delim('-')) => {
                    unary_count += 1;
                    if depth + unary_count > MAX_NESTING {
                        return None;
                    }
                    neg = !neg;
                    self.pos += 1;
                    self.skip_ws();
                }
                _ => break,
            }
        }
        let expr = self.parse_atom(depth + unary_count)?;
        Some(if neg { expr.scaled(-1.0) } else { expr })
    }

    fn parse_atom(&mut self, depth: usize) -> Option<ThemeExpr> {
        if depth > MAX_NESTING {
            return None;
        }
        self.skip_ws();
        let cv = self.values.get(self.pos)?;
        match cv {
            ComponentValue::SimpleBlock {
                kind: BlockKind::Paren,
                values: inner,
                ..
            } => {
                self.pos += 1;
                parse_theme_sum_slice(inner, depth + 1)
            }
            ComponentValue::Function { name, args, .. } => {
                self.pos += 1;
                if name.eq_ignore_ascii_case("calc") {
                    return parse_theme_sum_slice(args, depth + 1);
                }
                if name.eq_ignore_ascii_case("theme") {
                    let trimmed = trim_ws(args);
                    let raw = match trimmed {
                        [
                            ComponentValue::Token(SpannedToken {
                                token: Token::Ident(s) | Token::String(s),
                                ..
                            }),
                        ] => s.as_str(),
                        _ => return None,
                    };
                    return parse_theme_token_name(raw).map(ThemeExpr::token);
                }
                if name.eq_ignore_ascii_case("var") {
                    let groups = split_comma_groups(args);
                    let first = groups.first()?;
                    let [
                        ComponentValue::Token(SpannedToken {
                            token: Token::Ident(var_name),
                            ..
                        }),
                    ] = *first
                    else {
                        return None;
                    };
                    if var_name.starts_with("--nana-") {
                        return parse_theme_token_name(var_name).map(ThemeExpr::token);
                    }
                    return None;
                }
                None
            }
            ComponentValue::Token(SpannedToken { token, .. }) => {
                self.pos += 1;
                match token {
                    Token::Number { value, .. } if value.is_finite() => {
                        Some(ThemeExpr::px(*value as f32))
                    }
                    Token::Dimension { value, unit }
                        if value.is_finite() && unit.eq_ignore_ascii_case("px") =>
                    {
                        Some(ThemeExpr::px(*value as f32))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

// ============================================================================
// 4. Unified Color Parser
// ============================================================================

/// Explicit options for color parsing (no hidden global/thread-local state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColorParseOptions {
    /// Active scheme when statically evaluating `light-dark(...)`.
    pub dark_scheme: bool,
    /// When `true`, `light-dark(...)` produces a dynamic [`PaintColor::ThemePair`].
    pub dynamic_light_dark: bool,
}

/// Result of parsing a CSS `<color>`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParsedColor {
    /// Concrete or theme-bound [`PaintColor`].
    Paint(PaintColor),
    /// CSS `currentColor` keyword.
    CurrentColor,
}

impl ParsedColor {
    /// Extract concrete sRGB `[r, g, b, a]` when static; returns `None` for
    /// `CurrentColor`.
    pub fn to_srgb_opt(self) -> Option<[f32; 4]> {
        match self {
            Self::Paint(p) => Some(p.to_srgb()),
            Self::CurrentColor => None,
        }
    }

    pub fn into_paint(self) -> Option<PaintColor> {
        match self {
            Self::Paint(p) => Some(p),
            Self::CurrentColor => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HueInterpolationMethod {
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorInterpolationMethod {
    Srgb,
    SrgbLinear,
    Oklch(HueInterpolationMethod),
    Hsv(HueInterpolationMethod),
}

/// Convenience helper parsing a CSS `<color>` directly from `&str`.
///
/// Rejects empty or whitespace-only strings cleanly (`None`).
pub fn parse_color_str(input: &str, opts: ColorParseOptions) -> Option<ParsedColor> {
    if input.trim().is_empty() {
        return None;
    }
    let mut diags = Vec::<CssDiag>::new();
    let values = parse_component_values(input, &mut diags);
    parse_color(&values, opts)
}

/// Parse a CSS `<color>` from `&[ComponentValue]`.
///
/// Rejects empty input (`&[]` or whitespace-only) cleanly by returning `None`.
pub fn parse_color(values: &[ComponentValue], opts: ColorParseOptions) -> Option<ParsedColor> {
    parse_color_depth(values, opts, 0)
}

fn parse_color_depth(
    values: &[ComponentValue],
    opts: ColorParseOptions,
    depth: usize,
) -> Option<ParsedColor> {
    if depth >= MAX_NESTING {
        return None;
    }
    let trimmed = trim_ws(values);
    let [cv] = trimmed else {
        return None;
    };

    match cv {
        ComponentValue::Token(SpannedToken {
            token: Token::Hash { value, .. },
            ..
        }) => parse_hex_color(value).map(|rgba| ParsedColor::Paint(PaintColor::Srgb { rgba })),

        ComponentValue::Token(SpannedToken {
            token: Token::Ident(ident),
            ..
        }) => {
            if ident.eq_ignore_ascii_case("currentcolor") {
                return Some(ParsedColor::CurrentColor);
            }
            if ident.eq_ignore_ascii_case("transparent") {
                return Some(ParsedColor::Paint(PaintColor::Srgb {
                    rgba: [0.0, 0.0, 0.0, 0.0],
                }));
            }
            if let Some(rgba) = parse_named_color(ident) {
                return Some(ParsedColor::Paint(PaintColor::Srgb { rgba }));
            }
            if !ident.starts_with("--")
                && let Some(role) = SemanticColorRole::from_css_token_name(ident)
            {
                return Some(ParsedColor::Paint(PaintColor::ThemeRole { role }));
            }
            None
        }

        ComponentValue::Function { name, args, .. } => match name.to_ascii_lowercase().as_str() {
            "rgb" | "rgba" => {
                parse_rgb_fn(args).map(|rgba| ParsedColor::Paint(PaintColor::Srgb { rgba }))
            }
            "hsl" | "hsla" => {
                parse_hsl_fn(args).map(|rgba| ParsedColor::Paint(PaintColor::Srgb { rgba }))
            }
            "oklch" => parse_oklch_fn(args).map(ParsedColor::Paint),
            "hsv" => parse_hsv_fn(args).map(ParsedColor::Paint),
            "color" => parse_color_fn(args).map(ParsedColor::Paint),
            "color-mix" => parse_color_mix_fn(args, opts, depth + 1).map(ParsedColor::Paint),
            "light-dark" => parse_light_dark_fn(args, opts, depth + 1),
            "theme" => parse_theme_color_fn(args).map(ParsedColor::Paint),
            "var" => parse_nana_var_color_fn(args).map(ParsedColor::Paint),
            _ => None,
        },

        _ => None,
    }
}

fn parse_hex_color(hex: &str) -> Option<[f32; 4]> {
    let bytes = hex.as_bytes();
    let hex_digit = |b: u8| -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    };
    let pair = |a: u8, b: u8| -> Option<f32> {
        Some(((hex_digit(a)? << 4) | hex_digit(b)?) as f32 / 255.0)
    };
    let dup = |a: u8| -> Option<f32> {
        let d = hex_digit(a)?;
        Some(((d << 4) | d) as f32 / 255.0)
    };
    match bytes {
        [r, g, b] => Some([dup(*r)?, dup(*g)?, dup(*b)?, 1.0]),
        [r, g, b, a] => Some([dup(*r)?, dup(*g)?, dup(*b)?, dup(*a)?]),
        [r1, r2, g1, g2, b1, b2] => Some([pair(*r1, *r2)?, pair(*g1, *g2)?, pair(*b1, *b2)?, 1.0]),
        [r1, r2, g1, g2, b1, b2, a1, a2] => Some([
            pair(*r1, *r2)?,
            pair(*g1, *g2)?,
            pair(*b1, *b2)?,
            pair(*a1, *a2)?,
        ]),
        _ => None,
    }
}

fn parse_rgb_fn(args: &[ComponentValue]) -> Option<[f32; 4]> {
    let trimmed = trim_ws(args);
    let has_comma = trimmed
        .iter()
        .any(|cv| matches!(cv.as_token(), Some(Token::Comma)));
    let has_slash = trimmed
        .iter()
        .any(|cv| matches!(cv.as_token(), Some(Token::Delim('/'))));

    if has_comma && has_slash {
        return None;
    }

    if has_comma {
        let groups = split_comma_groups(trimmed);
        if groups.len() != 3 && groups.len() != 4 {
            return None;
        }
        let mut is_pct = None;
        let mut rgb = [0.0f32; 3];
        for (i, g) in groups[..3].iter().enumerate() {
            let [cv] = *g else { return None };
            match cv.as_token()? {
                Token::Number { value, .. } if value.is_finite() => {
                    if is_pct == Some(true) {
                        return None;
                    }
                    is_pct = Some(false);
                    rgb[i] = (*value as f32 / 255.0).clamp(0.0, 1.0);
                }
                Token::Percentage(p) if p.is_finite() => {
                    if is_pct == Some(false) {
                        return None;
                    }
                    is_pct = Some(true);
                    rgb[i] = (*p as f32 / 100.0).clamp(0.0, 1.0);
                }
                _ => return None,
            }
        }
        let alpha = if groups.len() == 4 {
            parse_alpha_slice(groups[3])?
        } else {
            1.0
        };
        return Some([rgb[0], rgb[1], rgb[2], alpha]);
    }

    // Modern CSS Color 4 space-separated syntax: `rgb(R G B [/ A])`
    let (channels, alpha) = split_optional_slash(trimmed)?;
    let items = split_whitespace_items(channels);
    if items.len() != 3 {
        return None;
    }
    let mut rgb = [0.0f32; 3];
    for (i, item) in items.iter().enumerate() {
        let [cv] = *item else { return None };
        rgb[i] = parse_modern_rgb_channel(cv)?;
    }
    let a = match alpha {
        Some(a_slice) => parse_alpha_slice(a_slice)?,
        None => 1.0,
    };
    Some([rgb[0], rgb[1], rgb[2], a])
}

fn parse_modern_rgb_channel(cv: &ComponentValue) -> Option<f32> {
    match cv.as_token()? {
        Token::Ident(s) if s.eq_ignore_ascii_case("none") => Some(0.0),
        Token::Number { value, .. } if value.is_finite() => {
            Some((*value as f32 / 255.0).clamp(0.0, 1.0))
        }
        Token::Percentage(p) if p.is_finite() => Some((*p as f32 / 100.0).clamp(0.0, 1.0)),
        _ => None,
    }
}

fn parse_hsl_fn(args: &[ComponentValue]) -> Option<[f32; 4]> {
    let trimmed = trim_ws(args);
    let has_comma = trimmed
        .iter()
        .any(|cv| matches!(cv.as_token(), Some(Token::Comma)));
    let has_slash = trimmed
        .iter()
        .any(|cv| matches!(cv.as_token(), Some(Token::Delim('/'))));
    if has_comma && has_slash {
        return None;
    }

    let (h, s, l, a) = if has_comma {
        let groups = split_comma_groups(trimmed);
        if groups.len() != 3 && groups.len() != 4 {
            return None;
        }
        let ([h_cv], [s_cv], [l_cv]) = (groups[0], groups[1], groups[2]) else {
            return None;
        };
        let h = parse_hue_token_deg(h_cv)?;
        let s = parse_percent_0_1(s_cv)?;
        let l = parse_percent_0_1(l_cv)?;
        let a = if groups.len() == 4 {
            parse_alpha_slice(groups[3])?
        } else {
            1.0
        };
        (h, s, l, a)
    } else {
        let (channels, alpha) = split_optional_slash(trimmed)?;
        let items = split_whitespace_items(channels);
        if items.len() != 3 {
            return None;
        }
        let ([h_cv], [s_cv], [l_cv]) = (items[0], items[1], items[2]) else {
            return None;
        };
        let h = parse_optional_hue(h_cv)?.unwrap_or(0.0);
        let s = parse_hsl_sl_modern(s_cv)?;
        let l = parse_hsl_sl_modern(l_cv)?;
        let a = match alpha {
            Some(a_slice) => parse_alpha_slice(a_slice)?,
            None => 1.0,
        };
        (h, s, l, a)
    };

    Some(hsl_to_srgb(h, s, l, a))
}

fn parse_percent_0_1(cv: &ComponentValue) -> Option<f32> {
    if let Some(Token::Percentage(p)) = cv.as_token()
        && p.is_finite()
    {
        return Some((*p as f32 / 100.0).clamp(0.0, 1.0));
    }
    None
}

fn parse_hsl_sl_modern(cv: &ComponentValue) -> Option<f32> {
    match cv.as_token()? {
        Token::Ident(s) if s.eq_ignore_ascii_case("none") => Some(0.0),
        Token::Percentage(p) | Token::Number { value: p, .. } if p.is_finite() => {
            Some((*p as f32 / 100.0).clamp(0.0, 1.0))
        }
        _ => None,
    }
}

fn hsl_to_srgb(h_deg: f32, s: f32, l: f32, a: f32) -> [f32; 4] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = wrap_hue(h_deg) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = if hp < 1.0 {
        (c, x, 0.0)
    } else if hp < 2.0 {
        (x, c, 0.0)
    } else if hp < 3.0 {
        (0.0, c, x)
    } else if hp < 4.0 {
        (0.0, x, c)
    } else if hp < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = l - c * 0.5;
    [
        (r1 + m).clamp(0.0, 1.0),
        (g1 + m).clamp(0.0, 1.0),
        (b1 + m).clamp(0.0, 1.0),
        a.clamp(0.0, 1.0),
    ]
}

fn parse_oklch_fn(args: &[ComponentValue]) -> Option<PaintColor> {
    let (channels, alpha) = split_optional_slash(trim_ws(args))?;
    let items = split_whitespace_items(channels);
    if items.len() != 3 {
        return None;
    }
    let ([l_cv], [c_cv], [h_cv]) = (items[0], items[1], items[2]) else {
        return None;
    };
    let l = match l_cv.as_token()? {
        Token::Ident(s) if s.eq_ignore_ascii_case("none") => 0.0,
        Token::Percentage(p) if p.is_finite() => (*p as f32 / 100.0).max(0.0),
        Token::Number { value, .. } if value.is_finite() => (*value as f32).max(0.0),
        _ => return None,
    };
    let c = match c_cv.as_token()? {
        Token::Ident(s) if s.eq_ignore_ascii_case("none") => 0.0,
        Token::Percentage(p) if p.is_finite() => (*p as f32 * 0.4 / 100.0).max(0.0),
        Token::Number { value, .. } if value.is_finite() => (*value as f32).max(0.0),
        _ => return None,
    };
    let h = parse_optional_hue(h_cv)?;
    let alpha = match alpha {
        Some(a_slice) => parse_alpha_slice(a_slice)?,
        None => 1.0,
    };
    Some(PaintColor::Oklch { l, c, h, alpha })
}

fn parse_hsv_fn(args: &[ComponentValue]) -> Option<PaintColor> {
    let (channels, alpha) = split_optional_slash(trim_ws(args))?;
    let items = split_whitespace_items(channels);
    if items.len() != 3 {
        return None;
    }
    let ([h_cv], [s_cv], [v_cv]) = (items[0], items[1], items[2]) else {
        return None;
    };
    let h = parse_optional_hue(h_cv)?.unwrap_or(0.0);
    let s = parse_unit_interval_cv(s_cv)?;
    let v = parse_unit_interval_cv(v_cv)?;
    let alpha = match alpha {
        Some(a_slice) => parse_alpha_slice(a_slice)?,
        None => 1.0,
    };
    Some(PaintColor::Hsv { h, s, v, alpha })
}

fn parse_color_fn(args: &[ComponentValue]) -> Option<PaintColor> {
    let (channels, alpha) = split_optional_slash(trim_ws(args))?;
    let items = split_whitespace_items(channels);
    if items.len() != 4 {
        return None;
    }
    let [space_cv] = items[0] else { return None };
    let space = space_cv.as_ident()?.to_ascii_lowercase();
    if space != "srgb-linear" && space != "scrgb" {
        return None;
    }
    let mut vals = [0.0f32; 3];
    for (slot, item) in vals.iter_mut().zip(&items[1..]) {
        let [cv] = *item else { return None };
        *slot = match cv.as_token()? {
            Token::Ident(s) if s.eq_ignore_ascii_case("none") => 0.0,
            Token::Percentage(p) if p.is_finite() => *p as f32 / 100.0,
            Token::Number { value, .. } if value.is_finite() => *value as f32,
            _ => return None,
        };
    }
    let alpha = match alpha {
        Some(a_slice) => parse_alpha_slice(a_slice)?,
        None => 1.0,
    };
    Some(PaintColor::LinearScRgb {
        channels: vals,
        alpha,
    })
}

fn parse_light_dark_fn(
    args: &[ComponentValue],
    opts: ColorParseOptions,
    depth: usize,
) -> Option<ParsedColor> {
    let groups = split_comma_groups(args);
    if groups.len() != 2 {
        return None;
    }
    if opts.dynamic_light_dark {
        let light = parse_color_depth(groups[0], opts, depth)?.into_paint()?;
        let dark = parse_color_depth(groups[1], opts, depth)?.into_paint()?;
        return Some(ParsedColor::Paint(PaintColor::light_dark(light, dark)));
    }
    let chosen = if opts.dark_scheme {
        groups[1]
    } else {
        groups[0]
    };
    parse_color_depth(chosen, opts, depth)
}

fn parse_color_mix_fn(
    args: &[ComponentValue],
    opts: ColorParseOptions,
    depth: usize,
) -> Option<PaintColor> {
    let groups = split_comma_groups(args);
    if groups.len() != 3 {
        return None;
    }
    let method = parse_color_interpolation_method(groups[0])?;
    let (color_a_cvs, pct_a) = split_color_mix_operand(groups[1])?;
    let (color_b_cvs, pct_b) = split_color_mix_operand(groups[2])?;
    let a = parse_color_depth(color_a_cvs, opts, depth)?.into_paint()?;
    let b = parse_color_depth(color_b_cvs, opts, depth)?.into_paint()?;

    if let Some(p) = pct_a
        && !(0.0..=100.0).contains(&p)
    {
        return None;
    }
    if let Some(p) = pct_b
        && !(0.0..=100.0).contains(&p)
    {
        return None;
    }

    let (wa, wb) = match (pct_a, pct_b) {
        (Some(x), Some(y)) => (x, y),
        (Some(x), None) => (x, 100.0 - x),
        (None, Some(y)) => (100.0 - y, y),
        (None, None) => (50.0, 50.0),
    };
    let supplied = wa + wb;
    if supplied <= 0.0 {
        return None;
    }
    let t = (wa / supplied).clamp(0.0, 1.0);
    let alpha_scale = (supplied / 100.0).min(1.0);

    if a.is_theme_dynamic() || b.is_theme_dynamic() {
        let is_transparent =
            |c: PaintColor| matches!(c, PaintColor::Srgb { rgba } if rgba[3] == 0.0);
        if alpha_scale >= 1.0 - 1e-4
            && let (PaintColor::ThemeRole { role: r1 }, PaintColor::ThemeRole { role: r2 }) = (a, b)
        {
            return Some(PaintColor::ThemeMix(SemanticColorMix::new(r1, r2, t)));
        }
        if let PaintColor::ThemeRole { role } = a
            && is_transparent(b)
        {
            return Some(PaintColor::ThemeMix(SemanticColorMix::alpha(
                role,
                t * alpha_scale,
            )));
        }
        if let PaintColor::ThemeRole { role } = b
            && is_transparent(a)
        {
            return Some(PaintColor::ThemeMix(SemanticColorMix::alpha(
                role,
                (1.0 - t) * alpha_scale,
            )));
        }
        let resolve_branch = |appearance| {
            let model = StyleModelRef::new(appearance);
            interpolate_paint_colors(
                method,
                a.resolve(model),
                b.resolve(model),
                1.0 - t,
                alpha_scale,
            )
        };
        return Some(PaintColor::light_dark(
            resolve_branch(ThemeAppearance::Light),
            resolve_branch(ThemeAppearance::Dark),
        ));
    }

    Some(interpolate_paint_colors(method, a, b, 1.0 - t, alpha_scale))
}

fn parse_color_interpolation_method(values: &[ComponentValue]) -> Option<ColorInterpolationMethod> {
    let items = split_whitespace_items(values);
    let mut idents = Vec::with_capacity(items.len());
    for item in items {
        let [cv] = item else { return None };
        idents.push(cv.as_ident()?.to_ascii_lowercase());
    }
    let [in_kw, space, rest @ ..] = idents.as_slice() else {
        return None;
    };
    if in_kw != "in" {
        return None;
    }
    let parse_hue = |tail: &[String]| -> Option<HueInterpolationMethod> {
        match tail {
            [] => Some(HueInterpolationMethod::Shorter),
            [dir, hue] if hue == "hue" => match dir.as_str() {
                "shorter" => Some(HueInterpolationMethod::Shorter),
                "longer" => Some(HueInterpolationMethod::Longer),
                "increasing" => Some(HueInterpolationMethod::Increasing),
                "decreasing" => Some(HueInterpolationMethod::Decreasing),
                _ => None,
            },
            _ => None,
        }
    };
    match space.as_str() {
        "srgb" if rest.is_empty() => Some(ColorInterpolationMethod::Srgb),
        "srgb-linear" if rest.is_empty() => Some(ColorInterpolationMethod::SrgbLinear),
        "oklch" => parse_hue(rest).map(ColorInterpolationMethod::Oklch),
        "hsv" => parse_hue(rest).map(ColorInterpolationMethod::Hsv),
        _ => None,
    }
}

fn split_color_mix_operand(values: &[ComponentValue]) -> Option<(&[ComponentValue], Option<f32>)> {
    let items = split_whitespace_items(values);
    match items.as_slice() {
        [only] => Some((only, None)),
        [first, second] => {
            if let [cv] = *first
                && let Some(Token::Percentage(p)) = cv.as_token()
                && p.is_finite()
            {
                return Some((second, Some(*p as f32)));
            }
            if let [cv] = *second
                && let Some(Token::Percentage(p)) = cv.as_token()
                && p.is_finite()
            {
                return Some((first, Some(*p as f32)));
            }
            None
        }
        _ => None,
    }
}

fn parse_theme_color_fn(args: &[ComponentValue]) -> Option<PaintColor> {
    let [cv] = trim_ws(args) else { return None };
    let raw = match cv.as_token()? {
        Token::Ident(s) | Token::String(s) => s.as_str(),
        _ => return None,
    };
    let lower = raw.trim().to_ascii_lowercase();
    let stripped = lower
        .strip_prefix("colors.")
        .or_else(|| lower.strip_prefix("color."))
        .or_else(|| lower.strip_prefix("palette."))
        .unwrap_or(&lower);
    SemanticColorRole::from_css_token_name(stripped).map(PaintColor::theme_role)
}

fn parse_nana_var_color_fn(args: &[ComponentValue]) -> Option<PaintColor> {
    let groups = split_comma_groups(args);
    let first = groups.first()?;
    let [cv] = *first else { return None };
    let name = cv.as_ident()?;
    if name.starts_with("--nana-") {
        return SemanticColorRole::from_css_token_name(name).map(PaintColor::theme_role);
    }
    None
}

fn split_optional_slash(
    values: &[ComponentValue],
) -> Option<(&[ComponentValue], Option<&[ComponentValue]>)> {
    let mut slash_idx = None;
    for (i, cv) in values.iter().enumerate() {
        if matches!(cv.as_token(), Some(Token::Delim('/'))) {
            if slash_idx.is_some() {
                return None;
            }
            slash_idx = Some(i);
        }
    }
    match slash_idx {
        Some(idx) => {
            let rhs = trim_ws(&values[idx + 1..]);
            if rhs.is_empty() {
                return None;
            }
            Some((trim_ws(&values[..idx]), Some(rhs)))
        }
        None => Some((values, None)),
    }
}

fn parse_alpha_slice(values: &[ComponentValue]) -> Option<f32> {
    let [cv] = trim_ws(values) else { return None };
    match cv.as_token()? {
        Token::Ident(s) if s.eq_ignore_ascii_case("none") => Some(0.0),
        Token::Percentage(p) if p.is_finite() => Some((*p as f32 / 100.0).clamp(0.0, 1.0)),
        Token::Number { value, .. } if value.is_finite() => Some((*value as f32).clamp(0.0, 1.0)),
        _ => None,
    }
}

fn parse_unit_interval_cv(cv: &ComponentValue) -> Option<f32> {
    match cv.as_token()? {
        Token::Ident(s) if s.eq_ignore_ascii_case("none") => Some(0.0),
        Token::Percentage(p) if p.is_finite() => Some((*p as f32 / 100.0).clamp(0.0, 1.0)),
        Token::Number { value, .. } if value.is_finite() => Some((*value as f32).clamp(0.0, 1.0)),
        _ => None,
    }
}

fn parse_optional_hue(cv: &ComponentValue) -> Option<Option<f32>> {
    if let Some(s) = cv.as_ident()
        && s.eq_ignore_ascii_case("none")
    {
        return Some(None);
    }
    parse_hue_token_deg(cv).map(Some)
}

pub(crate) fn interpolate_paint_colors(
    method: ColorInterpolationMethod,
    a: PaintColor,
    b: PaintColor,
    progress: f32,
    alpha_scale: f32,
) -> PaintColor {
    let u = progress.clamp(0.0, 1.0);
    let t = 1.0 - u;
    let mix = |ca: f32, aa: f32, cb: f32, ab: f32| {
        let alpha = aa * t + ab * u;
        if alpha > 0.0 {
            (ca * aa * t + cb * ab * u) / alpha
        } else {
            ca * t + cb * u
        }
    };
    match method {
        ColorInterpolationMethod::Srgb => {
            let a = a.to_srgb();
            let b = b.to_srgb();
            PaintColor::Srgb {
                rgba: [
                    mix(a[0], a[3], b[0], b[3]),
                    mix(a[1], a[3], b[1], b[3]),
                    mix(a[2], a[3], b[2], b[3]),
                    (a[3] * t + b[3] * u) * alpha_scale,
                ],
            }
        }
        ColorInterpolationMethod::SrgbLinear => {
            let (ca, aa) = a.to_linear_sc_rgb();
            let (cb, ab) = b.to_linear_sc_rgb();
            PaintColor::LinearScRgb {
                channels: [
                    mix(ca[0], aa, cb[0], ab),
                    mix(ca[1], aa, cb[1], ab),
                    mix(ca[2], aa, cb[2], ab),
                ],
                alpha: (aa * t + ab * u) * alpha_scale,
            }
        }
        ColorInterpolationMethod::Oklch(hm) => {
            let a = paint_to_oklch(a);
            let b = paint_to_oklch(b);
            PaintColor::Oklch {
                l: mix(a.0, a.3, b.0, b.3),
                c: mix(a.1, a.3, b.1, b.3),
                h: interpolate_hue(a.2, b.2, a.1, b.1, u, hm),
                alpha: (a.3 * t + b.3 * u) * alpha_scale,
            }
        }
        ColorInterpolationMethod::Hsv(hm) => {
            let a = paint_to_hsv(a);
            let b = paint_to_hsv(b);
            PaintColor::Hsv {
                h: interpolate_hue(Some(a.0), Some(b.0), a.1, b.1, u, hm).unwrap_or(0.0),
                s: mix(a.1, a.3, b.1, b.3),
                v: mix(a.2, a.3, b.2, b.3),
                alpha: (a.3 * t + b.3 * u) * alpha_scale,
            }
        }
    }
}

fn paint_to_oklch(c: PaintColor) -> (f32, f32, Option<f32>, f32) {
    match c {
        PaintColor::Oklch { l, c, h, alpha } => (l, c, h, alpha),
        other => {
            let (linear, alpha) = other.to_linear_sc_rgb();
            linear_scrgb_to_oklch(linear, alpha)
        }
    }
}

#[allow(clippy::excessive_precision)]
fn linear_scrgb_to_oklch([r, g, b]: [f32; 3], alpha: f32) -> (f32, f32, Option<f32>, f32) {
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let l0 = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let b0 = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
    let chroma = (a * a + b0 * b0).sqrt();
    let hue = if chroma < 1e-7 {
        None
    } else {
        Some(wrap_hue(b0.atan2(a).to_degrees()))
    };
    (l0, chroma, hue, alpha)
}

fn paint_to_hsv(c: PaintColor) -> (f32, f32, f32, f32) {
    match c {
        PaintColor::Hsv { h, s, v, alpha } => (h, s, v, alpha),
        other => {
            let [r, g, b, a] = other.to_srgb();
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let d = max - min;
            let h = if d < 1e-7 {
                0.0
            } else if (max - r).abs() < 1e-7 {
                wrap_hue(60.0 * ((g - b) / d))
            } else if (max - g).abs() < 1e-7 {
                wrap_hue(60.0 * ((b - r) / d + 2.0))
            } else {
                wrap_hue(60.0 * ((r - g) / d + 4.0))
            };
            (h, if max < 1e-7 { 0.0 } else { d / max }, max, a)
        }
    }
}

fn interpolate_hue(
    a: Option<f32>,
    b: Option<f32>,
    ca: f32,
    cb: f32,
    progress: f32,
    method: HueInterpolationMethod,
) -> Option<f32> {
    let a_miss = a.is_none() || ca.abs() < 1e-7;
    let b_miss = b.is_none() || cb.abs() < 1e-7;
    if a_miss && b_miss {
        return None;
    }
    if a_miss {
        return Some(wrap_hue(b.unwrap_or(0.0)));
    }
    if b_miss {
        return Some(wrap_hue(a.unwrap_or(0.0)));
    }
    let ha = wrap_hue(a.unwrap_or(0.0));
    let hb = wrap_hue(b.unwrap_or(0.0));
    let mut delta = hb - ha;
    match method {
        HueInterpolationMethod::Shorter => {
            if delta > 180.0 {
                delta -= 360.0;
            } else if delta < -180.0 {
                delta += 360.0;
            }
        }
        HueInterpolationMethod::Longer => {
            if delta > 0.0 && delta < 180.0 {
                delta -= 360.0;
            } else if delta > -180.0 && delta <= 0.0 {
                delta += 360.0;
            }
        }
        HueInterpolationMethod::Increasing => {
            if delta < 0.0 {
                delta += 360.0;
            }
        }
        HueInterpolationMethod::Decreasing => {
            if delta > 0.0 {
                delta -= 360.0;
            }
        }
    }
    Some(wrap_hue(ha + delta * progress))
}

/// CSS named color lookup returning `[r, g, b, a]` in `0..=1` sRGB.
pub fn parse_named_color(name: &str) -> Option<[f32; 4]> {
    let rgb = match name.to_ascii_lowercase().as_str() {
        "transparent" => return Some([0.0, 0.0, 0.0, 0.0]),
        // Core CSS 2.1 + CSS Color 4 keywords
        "black" => (0, 0, 0),
        "silver" => (192, 192, 192),
        "gray" | "grey" => (128, 128, 128),
        "white" => (255, 255, 255),
        "maroon" => (128, 0, 0),
        "red" => (255, 0, 0),
        "purple" => (128, 0, 128),
        "fuchsia" | "magenta" => (255, 0, 255),
        "green" => (0, 128, 0),
        "lime" => (0, 255, 0),
        "olive" => (128, 128, 0),
        "yellow" => (255, 255, 0),
        "navy" => (0, 0, 128),
        "blue" => (0, 0, 255),
        "teal" => (0, 128, 128),
        "aqua" | "cyan" => (0, 255, 255),
        "orange" => (255, 165, 0),
        "rebeccapurple" => (102, 51, 153),
        // Common extended CSS Color 3/4 keywords
        "aliceblue" => (240, 248, 255),
        "antiquewhite" => (250, 235, 215),
        "aquamarine" => (127, 255, 212),
        "azure" => (240, 255, 255),
        "beige" => (245, 245, 220),
        "bisque" => (255, 228, 196),
        "blueviolet" => (138, 43, 226),
        "brown" => (165, 42, 42),
        "cadetblue" => (95, 158, 160),
        "chartreuse" => (127, 255, 0),
        "chocolate" => (210, 105, 30),
        "coral" => (255, 127, 80),
        "cornflowerblue" => (100, 149, 237),
        "crimson" => (220, 20, 60),
        "darkblue" => (0, 0, 139),
        "darkcyan" => (0, 139, 139),
        "darkgoldenrod" => (184, 134, 11),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "darkgreen" => (0, 100, 0),
        "darkmagenta" => (139, 0, 139),
        "darkorange" => (255, 140, 0),
        "darkred" => (139, 0, 0),
        "darksalmon" => (233, 150, 122),
        "darkslategray" | "darkslategrey" => (47, 79, 79),
        "darkturquoise" => (0, 206, 209),
        "darkviolet" => (148, 0, 211),
        "deeppink" => (255, 20, 147),
        "deepskyblue" => (0, 191, 255),
        "dimgray" | "dimgrey" => (105, 105, 105),
        "dodgerblue" => (30, 144, 255),
        "firebrick" => (178, 34, 34),
        "forestgreen" => (34, 139, 34),
        "gainsboro" => (220, 220, 220),
        "ghostwhite" => (248, 248, 255),
        "gold" => (255, 215, 0),
        "goldenrod" => (218, 165, 32),
        "greenyellow" => (173, 255, 47),
        "hotpink" => (255, 105, 180),
        "indianred" => (205, 92, 92),
        "indigo" => (75, 0, 130),
        "ivory" => (255, 255, 240),
        "khaki" => (240, 230, 140),
        "lavender" => (230, 230, 250),
        "lightblue" => (173, 216, 230),
        "lightcoral" => (240, 128, 128),
        "lightcyan" => (224, 255, 255),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "lightgreen" => (144, 238, 144),
        "lightpink" => (255, 182, 193),
        "lightsalmon" => (255, 160, 122),
        "lightseagreen" => (32, 178, 170),
        "lightskyblue" => (135, 206, 250),
        "lightslategray" | "lightslategrey" => (119, 136, 153),
        "lightsteelblue" => (176, 196, 222),
        "limegreen" => (50, 205, 50),
        "linen" => (250, 240, 230),
        "mediumblue" => (0, 0, 205),
        "mediumorchid" => (186, 85, 211),
        "mediumpurple" => (147, 112, 219),
        "mediumseagreen" => (60, 179, 113),
        "mediumslateblue" => (123, 104, 238),
        "mediumturquoise" => (72, 209, 204),
        "midnightblue" => (25, 25, 112),
        "mintcream" => (245, 255, 250),
        "mistyrose" => (255, 228, 225),
        "moccasin" => (255, 228, 181),
        "oldlace" => (253, 245, 230),
        "olivedrab" => (107, 142, 35),
        "orangered" => (255, 69, 0),
        "orchid" => (218, 112, 214),
        "palegoldenrod" => (238, 232, 170),
        "palegreen" => (152, 251, 152),
        "paleturquoise" => (175, 238, 238),
        "palevioletred" => (219, 112, 147),
        "papayawhip" => (255, 239, 213),
        "peachpuff" => (255, 218, 185),
        "peru" => (205, 133, 63),
        "pink" => (255, 192, 203),
        "plum" => (221, 160, 221),
        "powderblue" => (176, 224, 230),
        "rosybrown" => (188, 143, 143),
        "royalblue" => (65, 105, 225),
        "saddlebrown" => (139, 69, 19),
        "salmon" => (250, 128, 114),
        "sandybrown" => (244, 164, 96),
        "seagreen" => (46, 139, 87),
        "seashell" => (255, 245, 238),
        "sienna" => (160, 82, 45),
        "skyblue" => (135, 206, 235),
        "slateblue" => (106, 90, 205),
        "slategray" | "slategrey" => (112, 128, 144),
        "snow" => (255, 250, 250),
        "springgreen" => (0, 255, 127),
        "steelblue" => (70, 130, 180),
        "tan" => (210, 180, 140),
        "thistle" => (216, 191, 216),
        "tomato" => (255, 99, 71),
        "turquoise" => (64, 224, 208),
        "violet" => (238, 130, 238),
        "wheat" => (245, 222, 179),
        "whitesmoke" => (245, 245, 245),
        "yellowgreen" => (154, 205, 50),
        _ => return None,
    };
    Some([
        rgb.0 as f32 / 255.0,
        rgb.1 as f32 / 255.0,
        rgb.2 as f32 / 255.0,
        1.0,
    ])
}

// ============================================================================
// 5. Unified 1/2/3/4-Value Position Parser (background-position / object-position)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
enum PosToken {
    Left,
    Right,
    Top,
    Bottom,
    Center,
    Len(LengthSpec),
}

impl PosToken {
    fn parse(item: &[ComponentValue]) -> Option<Self> {
        if let [cv] = item
            && let Some(ident) = cv.as_ident()
        {
            return match ident.to_ascii_lowercase().as_str() {
                "left" => Some(Self::Left),
                "right" => Some(Self::Right),
                "top" => Some(Self::Top),
                "bottom" => Some(Self::Bottom),
                "center" => Some(Self::Center),
                _ => None,
            };
        }
        parse_length_spec(item).map(Self::Len)
    }

    fn as_x_only(self) -> Option<LengthSpec> {
        match self {
            Self::Left => Some(LengthSpec::Percent(0.0)),
            Self::Center => Some(LengthSpec::Percent(50.0)),
            Self::Right => Some(LengthSpec::Percent(100.0)),
            Self::Len(l) => Some(l),
            Self::Top | Self::Bottom => None,
        }
    }

    fn as_y_only(self) -> Option<LengthSpec> {
        match self {
            Self::Top => Some(LengthSpec::Percent(0.0)),
            Self::Center => Some(LengthSpec::Percent(50.0)),
            Self::Bottom => Some(LengthSpec::Percent(100.0)),
            Self::Len(l) => Some(l),
            Self::Left | Self::Right => None,
        }
    }

    fn is_horiz_edge(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    fn is_vert_edge(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// Convert `edge + offset` into a [`LengthSpec`] relative to the start edge
/// (`left`/`top` -> `offset`; `right`/`bottom` -> `calc(100% - offset)`).
fn edge_with_offset(edge: PosToken, offset: LengthSpec) -> Option<LengthSpec> {
    match edge {
        PosToken::Left | PosToken::Top => Some(offset),
        PosToken::Right | PosToken::Bottom => Some(match offset {
            LengthSpec::Percent(p) => LengthSpec::Percent(100.0 - p),
            LengthSpec::Px(px) => LengthSpec::percent_offset(100.0, -px),
            other => {
                let expr = CalcExpr::Binary {
                    op: CalcBinOp::Sub,
                    left: Box::new(CalcExpr::Percent(100.0)),
                    right: Box::new(length_spec_to_calc(other)?),
                };
                LengthSpec::from_calc(expr)
            }
        }),
        _ => None,
    }
}

fn length_spec_to_calc(spec: LengthSpec) -> Option<CalcExpr> {
    match spec {
        LengthSpec::Px(v) => Some(CalcExpr::Px(v)),
        LengthSpec::Percent(p) => Some(CalcExpr::Percent(p)),
        LengthSpec::Em(v) => Some(CalcExpr::Em(v)),
        LengthSpec::Rem(v) => Some(CalcExpr::Rem(v)),
        LengthSpec::Viewport { axis, value } => Some(CalcExpr::Viewport { axis, value }),
        LengthSpec::PercentOffset { offset_q7, percent } => Some(CalcExpr::Binary {
            op: CalcBinOp::Add,
            left: Box::new(CalcExpr::Percent(percent)),
            right: Box::new(CalcExpr::Px(f32::from(offset_q7) / 128.0)),
        }),
        LengthSpec::Calc(r) => Some(r.inner().clone()),
        _ => None,
    }
}

/// Parse a CSS `<position>` (`background-position` / `object-position`) with
/// full 1-, 2-, 3-, and 4-value CSS Backgrounds 3 / CSS Values 4 grammar:
///
/// - 1 value: `left` -> `(0%, 50%)`, `top` -> `(50%, 0%)`, `10px` -> `(10px, 50%)`
/// - 2 values:
///   - `right 10px` -> `(100%, 10px)`
///   - `center 20px` -> `(50%, 20px)`
///   - `top right` -> `(100%, 0%)`
///   - `10px bottom` -> `(10px, 100%)`
///   - Rejects invalid 2-value orderings like `top 10px` or `10px left`.
/// - 3 values: `right 10px bottom` -> `(calc(100% - 10px), 100%)`
/// - 4 values: `right 10px bottom 20px` -> `(calc(100% - 10px), calc(100% - 20px))`
pub fn parse_position(values: &[ComponentValue]) -> Option<BackgroundPosition> {
    let raw_items = split_whitespace_items(values);
    if raw_items.is_empty() || raw_items.len() > 4 {
        return None;
    }
    let mut toks = Vec::with_capacity(raw_items.len());
    for item in raw_items {
        toks.push(PosToken::parse(item)?);
    }

    match toks.as_slice() {
        [only] => match only {
            PosToken::Left => Some(BackgroundPosition {
                x: LengthSpec::Percent(0.0),
                y: LengthSpec::Percent(50.0),
            }),
            PosToken::Right => Some(BackgroundPosition {
                x: LengthSpec::Percent(100.0),
                y: LengthSpec::Percent(50.0),
            }),
            PosToken::Center => Some(BackgroundPosition::center()),
            PosToken::Top => Some(BackgroundPosition {
                x: LengthSpec::Percent(50.0),
                y: LengthSpec::Percent(0.0),
            }),
            PosToken::Bottom => Some(BackgroundPosition {
                x: LengthSpec::Percent(50.0),
                y: LengthSpec::Percent(100.0),
            }),
            PosToken::Len(x) => Some(BackgroundPosition {
                x: *x,
                y: LengthSpec::Percent(50.0),
            }),
        },

        [a, b] => {
            // Two keywords may appear in swapped order (`top right`, `bottom left`, `top center`).
            if (a.is_vert_edge() && (b.is_horiz_edge() || matches!(b, PosToken::Center)))
                || (matches!(a, PosToken::Center) && b.is_horiz_edge())
            {
                return Some(BackgroundPosition {
                    x: b.as_x_only()?,
                    y: a.as_y_only()?,
                });
            }
            // Otherwise strictly `[left|center|right|<lp>] [top|center|bottom|<lp>]`.
            Some(BackgroundPosition {
                x: a.as_x_only()?,
                y: b.as_y_only()?,
            })
        }

        [a, b, c] => {
            // Either `(edge1 offset1) kw2` or `kw1 (edge2 offset2)`.
            if let PosToken::Len(off) = *b {
                if a.is_horiz_edge() && (c.is_vert_edge() || matches!(c, PosToken::Center)) {
                    return Some(BackgroundPosition {
                        x: edge_with_offset(*a, off)?,
                        y: c.as_y_only()?,
                    });
                }
                if a.is_vert_edge() && (c.is_horiz_edge() || matches!(c, PosToken::Center)) {
                    return Some(BackgroundPosition {
                        x: c.as_x_only()?,
                        y: edge_with_offset(*a, off)?,
                    });
                }
            }
            if let PosToken::Len(off) = *c {
                if b.is_vert_edge() && (a.is_horiz_edge() || matches!(a, PosToken::Center)) {
                    return Some(BackgroundPosition {
                        x: a.as_x_only()?,
                        y: edge_with_offset(*b, off)?,
                    });
                }
                if b.is_horiz_edge() && (a.is_vert_edge() || matches!(a, PosToken::Center)) {
                    return Some(BackgroundPosition {
                        x: edge_with_offset(*b, off)?,
                        y: a.as_y_only()?,
                    });
                }
            }
            None
        }

        [a, b, c, d] => {
            let (PosToken::Len(off1), PosToken::Len(off2)) = (*b, *d) else {
                return None;
            };
            if a.is_horiz_edge() && c.is_vert_edge() {
                return Some(BackgroundPosition {
                    x: edge_with_offset(*a, off1)?,
                    y: edge_with_offset(*c, off2)?,
                });
            }
            if a.is_vert_edge() && c.is_horiz_edge() {
                return Some(BackgroundPosition {
                    x: edge_with_offset(*c, off2)?,
                    y: edge_with_offset(*a, off1)?,
                });
            }
            None
        }

        _ => None,
    }
}

// ============================================================================
// 6. Unified Gradient Stop Parser & Normalizer (CSS Images 3/4 §3.4.3)
// ============================================================================

/// Position of a gradient color stop prior to used-size resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientStopPosition {
    /// Percentage along the gradient line (e.g. `20.0` for `20%`).
    Percent(f32),
    /// Non-percentage length (e.g. `20px`, `calc(50% - 10px)`).
    /// Requires the concrete gradient-line length in px to resolve; NEVER
    /// divided by a hardcoded `100.0`.
    Length(LengthSpec),
}

/// One parsed color-stop item (supports 0, 1, or 2 positions per CSS Images 4).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedGradientStop {
    pub color: ParsedColor,
    pub positions: Vec<GradientStopPosition>,
}

/// Parse one comma-separated gradient color stop (e.g. `red`, `blue 25%`,
/// `#0f0 20% 40%`, `rgba(0,0,0,0.5) 12px`).
pub fn parse_gradient_stop(
    values: &[ComponentValue],
    opts: ColorParseOptions,
) -> Option<ParsedGradientStop> {
    let items = split_whitespace_items(values);
    if items.is_empty() || items.len() > 3 {
        return None;
    }
    let color = parse_color(items[0], opts)?;
    let mut positions = Vec::with_capacity(items.len() - 1);
    for pos_item in &items[1..] {
        let len = parse_length_spec(pos_item)?;
        match len {
            LengthSpec::Percent(p) => positions.push(GradientStopPosition::Percent(p)),
            other => positions.push(GradientStopPosition::Length(other)),
        }
    }
    Some(ParsedGradientStop { color, positions })
}

/// Normalize parsed gradient stops per CSS Images 3/4 §3.4.3 (Color Stop Fixup):
///
/// 1. Expands double-position stops (`color p1 p2` -> two stops at `p1` and `p2`).
/// 2. Resolves `%` directly to `p / 100.0` and [`GradientStopPosition::Length`]
///    against `gradient_line_px` (`Some(len)` where `len > 0.0`). Returns `None`
///    if a `Length` stop is present and `gradient_line_px` is `None` or `<= 0.0`
///    (NEVER divides `px` by hardcoded `100.0`!).
/// 3. Assigns `0.0` to the first stop ONLY if unspecified (preserving any explicit
///    first stop position such as `20%` -> `0.2`), and `1.0` to the last stop
///    ONLY if unspecified (preserving any explicit last stop such as `80%` -> `0.8`).
/// 4. Enforces non-decreasing stop positions (`pos[i] = pos[i].max(max_prev)`).
/// 5. Evenly distributes runs of unspecified stops between their bounding stops.
pub fn normalize_gradient_stops(
    raw: &[ParsedGradientStop],
    gradient_line_px: Option<f32>,
    resolve_color: impl Fn(&ParsedColor) -> PaintColor,
) -> Option<Vec<GradientStop>> {
    // Expand double-position stops into `(PaintColor, Option<f32>)`.
    let mut expanded: Vec<(PaintColor, Option<f32>)> = Vec::with_capacity(raw.len());
    for stop in raw {
        let color = resolve_color(&stop.color);
        if stop.positions.is_empty() {
            expanded.push((color, None));
        } else {
            for pos in &stop.positions {
                let frac = match *pos {
                    GradientStopPosition::Percent(p) => p / 100.0,
                    GradientStopPosition::Length(spec) => {
                        let line = gradient_line_px?;
                        if !line.is_finite() || line <= 0.0 {
                            return None;
                        }
                        let px = spec.resolve_px(Some(line))?;
                        px / line
                    }
                };
                if !frac.is_finite() {
                    return None;
                }
                expanded.push((color, Some(frac)));
            }
        }
    }

    if expanded.len() < 2 {
        return None;
    }

    // Step 1: Default first/last ONLY when unspecified (preserve explicit positions!).
    if expanded[0].1.is_none() {
        expanded[0].1 = Some(0.0);
    }
    let last_idx = expanded.len() - 1;
    if expanded[last_idx].1.is_none() {
        expanded[last_idx].1 = Some(1.0);
    }

    // Step 2: Monotonicity fixup over all specified positions.
    let mut max_so_far = expanded[0].1.unwrap();
    for entry in &mut expanded[1..] {
        if let Some(p) = &mut entry.1 {
            if *p < max_so_far {
                *p = max_so_far;
            } else {
                max_so_far = *p;
            }
        }
    }

    // Step 3: Evenly distribute any runs of unspecified (`None`) positions.
    let mut i = 0;
    while i < expanded.len() {
        if expanded[i].1.is_some() {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < expanded.len() && expanded[i].1.is_none() {
            i += 1;
        }
        let prev_pos = expanded[run_start - 1].1.unwrap();
        let next_pos = expanded[i].1.unwrap();
        let steps = (i - run_start + 1) as f32;
        for (offset, entry) in expanded[run_start..i].iter_mut().enumerate() {
            entry.1 = Some(prev_pos + (next_pos - prev_pos) * ((offset + 1) as f32 / steps));
        }
    }

    Some(
        expanded
            .into_iter()
            .map(|(color, pos)| GradientStop {
                position: pos.unwrap(),
                color,
            })
            .collect(),
    )
}
