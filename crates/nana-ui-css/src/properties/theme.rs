//! Theme-token arithmetic (`theme(...)` / `var(--nana-*)`) mapped directly to
//! `StyleOp::Theme*` plus fallback literal `StyleOp`s.

use nana_ui_core::{LengthSpec, StyleOp, ThemeAppearance, ThemeExpr, builtin_theme};

use crate::syntax::diag::{CssDiag, DiagCode};
use crate::syntax::parser::{ComponentValue, split_whitespace_items, trim_ws};
use crate::syntax::span::Span;
use crate::syntax::values::parse_theme_expr;

/// Whether a component-value tree contains any `theme(...)` or `var(--nana-...)`
/// token expression (for layout/spacing/typography/radius/border theme props).
pub fn contains_theme_token_expr(values: &[ComponentValue]) -> bool {
    for cv in values {
        match cv {
            ComponentValue::Function { name, args, .. } => {
                if name.eq_ignore_ascii_case("theme") {
                    return true;
                }
                if name.eq_ignore_ascii_case("var")
                    && let Some(first) = trim_ws(args).first()
                    && first.as_ident().is_some_and(|id| {
                        id.starts_with("--nana-")
                            && crate::syntax::values::parse_theme_token_name(id).is_some()
                    })
                {
                    return true;
                }
                if contains_theme_token_expr(args) {
                    return true;
                }
            }
            ComponentValue::SimpleBlock { values, .. } => {
                if contains_theme_token_expr(values) {
                    return true;
                }
            }
            ComponentValue::Token(_) => {}
        }
    }
    false
}

/// Emit `StyleOp::Theme*(None)` clearing ops for any theme-bindable property or
/// shorthand when an ordinary literal value overrides it.
pub fn clear_theme_ops_for_property(prop: &str) -> Vec<StyleOp> {
    match prop {
        "gap" => vec![
            StyleOp::ThemeGap(None),
            StyleOp::ThemeRowGap(None),
            StyleOp::ThemeColumnGap(None),
        ],
        "row-gap" => vec![StyleOp::ThemeRowGap(None)],
        "column-gap" => vec![StyleOp::ThemeColumnGap(None)],
        "padding" => vec![
            StyleOp::ThemePadding(None),
            StyleOp::ThemePaddingTop(None),
            StyleOp::ThemePaddingRight(None),
            StyleOp::ThemePaddingBottom(None),
            StyleOp::ThemePaddingLeft(None),
        ],
        "padding-top" => vec![StyleOp::ThemePaddingTop(None)],
        "padding-right" => vec![StyleOp::ThemePaddingRight(None)],
        "padding-bottom" => vec![StyleOp::ThemePaddingBottom(None)],
        "padding-left" => vec![StyleOp::ThemePaddingLeft(None)],
        "margin" => vec![
            StyleOp::ThemeMargin(None),
            StyleOp::ThemeMarginTop(None),
            StyleOp::ThemeMarginRight(None),
            StyleOp::ThemeMarginBottom(None),
            StyleOp::ThemeMarginLeft(None),
        ],
        "margin-top" => vec![StyleOp::ThemeMarginTop(None)],
        "margin-right" => vec![StyleOp::ThemeMarginRight(None)],
        "margin-bottom" => vec![StyleOp::ThemeMarginBottom(None)],
        "margin-left" => vec![StyleOp::ThemeMarginLeft(None)],
        "inset" => vec![
            StyleOp::ThemeOffsetTop(None),
            StyleOp::ThemeOffsetRight(None),
            StyleOp::ThemeOffsetBottom(None),
            StyleOp::ThemeOffsetLeft(None),
        ],
        "top" => vec![StyleOp::ThemeOffsetTop(None)],
        "right" => vec![StyleOp::ThemeOffsetRight(None)],
        "bottom" => vec![StyleOp::ThemeOffsetBottom(None)],
        "left" => vec![StyleOp::ThemeOffsetLeft(None)],
        "width" => vec![StyleOp::ThemeWidth(None)],
        "height" => vec![StyleOp::ThemeHeight(None)],
        "min-width" => vec![StyleOp::ThemeMinWidth(None)],
        "max-width" => vec![StyleOp::ThemeMaxWidth(None)],
        "min-height" => vec![StyleOp::ThemeMinHeight(None)],
        "max-height" => vec![StyleOp::ThemeMaxHeight(None)],
        "flex-basis" => vec![StyleOp::ThemeFlexBasis(None)],
        "font-size" => vec![StyleOp::ThemeFontSize(None)],
        "font-weight" => vec![StyleOp::ThemeFontWeight(None)],
        "line-height" => vec![StyleOp::ThemeLineHeight(None)],
        "letter-spacing" => vec![StyleOp::ThemeLetterSpacing(None)],
        "border-width" => vec![
            StyleOp::ThemeBorderWidth(None),
            StyleOp::ThemeBorderTopWidth(None),
            StyleOp::ThemeBorderRightWidth(None),
            StyleOp::ThemeBorderBottomWidth(None),
            StyleOp::ThemeBorderLeftWidth(None),
        ],
        "border-top-width" => vec![StyleOp::ThemeBorderTopWidth(None)],
        "border-right-width" => vec![StyleOp::ThemeBorderRightWidth(None)],
        "border-bottom-width" => vec![StyleOp::ThemeBorderBottomWidth(None)],
        "border-left-width" => vec![StyleOp::ThemeBorderLeftWidth(None)],
        "border-radius" => vec![StyleOp::ThemeBorderRadius(None)],
        _ => Vec::new(),
    }
}

/// Try to compile a theme-token declaration directly to `Vec<StyleOp>`.
///
/// Returns `None` if `values` does not reference any layout theme token (so the
/// caller proceeds to the ordinary CSS property parser). Returns `Some(Ok(ops))`
/// or `Some(Err(diag))` when the declaration is a theme-token expression.
pub fn try_parse_theme_property(
    prop: &str,
    values: &[ComponentValue],
    span: Span,
) -> Option<Result<Vec<StyleOp>, CssDiag>> {
    if !contains_theme_token_expr(values) {
        return None;
    }

    // Check shorthands that may mix theme expressions and literal lengths.
    if let Some(targets) = shorthand_targets(prop) {
        return Some(parse_theme_shorthand(prop, targets, values, span));
    }

    // Single theme-bindable property.
    if !is_single_theme_property(prop) {
        return None;
    }

    let Some(expr) = parse_theme_expr(values) else {
        return Some(Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid theme expression for `{prop}`"),
        )));
    };
    Some(Ok(emit_single_theme_ops(prop, expr)))
}

fn shorthand_targets(prop: &str) -> Option<&'static [&'static str]> {
    Some(match prop {
        "padding" => &[
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ],
        "margin" => &["margin-top", "margin-right", "margin-bottom", "margin-left"],
        "inset" => &["top", "right", "bottom", "left"],
        "border-width" => &[
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ],
        "gap" => &["row-gap", "column-gap"],
        _ => return None,
    })
}

fn is_single_theme_property(prop: &str) -> bool {
    matches!(
        prop,
        "row-gap"
            | "column-gap"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "width"
            | "height"
            | "min-width"
            | "max-width"
            | "min-height"
            | "max-height"
            | "flex-basis"
            | "font-size"
            | "font-weight"
            | "line-height"
            | "letter-spacing"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "border-radius"
    )
}

enum ShorthandSlot {
    Theme(ThemeExpr),
    Literal(LengthSpec),
}

fn parse_theme_shorthand(
    prop: &str,
    targets: &[&str],
    values: &[ComponentValue],
    span: Span,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() || items.len() > targets.len() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid number of values in `{prop}`"),
        ));
    }

    // Parse and validate every item BEFORE emitting any StyleOp.
    let mut parsed = Vec::with_capacity(items.len());
    for item in items {
        if contains_theme_token_expr(item) {
            let expr = parse_theme_expr(item).ok_or_else(|| {
                CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    format!("invalid theme expression in `{prop}`"),
                )
            })?;
            parsed.push(ShorthandSlot::Theme(expr));
        } else {
            let len = crate::syntax::values::parse_length_spec(item).ok_or_else(|| {
                CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    format!("invalid length in `{prop}`"),
                )
            })?;
            parsed.push(ShorthandSlot::Literal(len));
        }
    }

    let indices: Vec<usize> = if targets.len() == 2 {
        vec![0, usize::from(parsed.len() > 1)]
    } else {
        match parsed.len() {
            1 => vec![0, 0, 0, 0],
            2 => vec![0, 1, 0, 1],
            3 => vec![0, 1, 2, 1],
            _ => vec![0, 1, 2, 3],
        }
    };

    let mut ops = Vec::new();
    // Clear uniform shorthand slot & theme shorthand binding.
    match prop {
        "padding" => {
            ops.push(StyleOp::Padding(None));
            ops.push(StyleOp::ThemePadding(None));
        }
        "margin" => {
            ops.push(StyleOp::Margin(None));
            ops.push(StyleOp::ThemeMargin(None));
        }
        "gap" => {
            ops.push(StyleOp::Gap(None));
            ops.push(StyleOp::ThemeGap(None));
        }
        "border-width" => {
            ops.push(StyleOp::ThemeBorderWidth(None));
        }
        _ => {}
    }

    let mut max_bw: Option<f32> = None;
    for (target, idx) in targets.iter().zip(indices) {
        match &parsed[idx] {
            ShorthandSlot::Theme(expr) => {
                if prop == "border-width" {
                    let fb = resolve_fallback(expr).max(0.0);
                    max_bw = Some(max_bw.map_or(fb, |m| m.max(fb)));
                }
                ops.extend(emit_single_theme_ops(target, expr.clone()));
            }
            ShorthandSlot::Literal(len) => {
                ops.extend(clear_theme_ops_for_property(target));
                if prop == "border-width"
                    && let Some(px) = len.resolve_px(None)
                {
                    let px = px.max(0.0);
                    max_bw = Some(max_bw.map_or(px, |m| m.max(px)));
                }
                ops.extend(emit_literal_side_op(target, *len));
            }
        }
    }
    if prop == "border-width"
        && let Some(bw) = max_bw
    {
        ops.push(StyleOp::BorderWidth(Some(bw)));
    }
    Ok(ops)
}

fn resolve_fallback(expr: &ThemeExpr) -> f32 {
    expr.resolve(builtin_theme(ThemeAppearance::Light))
}

fn emit_literal_side_op(prop: &str, len: LengthSpec) -> Vec<StyleOp> {
    let px_non_neg = len.resolve_px(None).unwrap_or(0.0).max(0.0);
    match prop {
        "row-gap" => vec![StyleOp::RowGap(Some(len))],
        "column-gap" => vec![StyleOp::ColumnGap(Some(len))],
        "padding-top" => vec![StyleOp::PaddingTop(Some(len))],
        "padding-right" => vec![StyleOp::PaddingRight(Some(len))],
        "padding-bottom" => vec![StyleOp::PaddingBottom(Some(len))],
        "padding-left" => vec![StyleOp::PaddingLeft(Some(len))],
        "margin-top" => vec![StyleOp::MarginTop(Some(len))],
        "margin-right" => vec![StyleOp::MarginRight(Some(len))],
        "margin-bottom" => vec![StyleOp::MarginBottom(Some(len))],
        "margin-left" => vec![StyleOp::MarginLeft(Some(len))],
        "top" => vec![StyleOp::OffsetTop(Some(len))],
        "right" => vec![StyleOp::OffsetRight(Some(len))],
        "bottom" => vec![StyleOp::OffsetBottom(Some(len))],
        "left" => vec![StyleOp::OffsetLeft(Some(len))],
        "border-top-width" => vec![StyleOp::BorderTopWidth(Some(px_non_neg))],
        "border-right-width" => vec![StyleOp::BorderRightWidth(Some(px_non_neg))],
        "border-bottom-width" => vec![StyleOp::BorderBottomWidth(Some(px_non_neg))],
        "border-left-width" => vec![StyleOp::BorderLeftWidth(Some(px_non_neg))],
        _ => Vec::new(),
    }
}

fn emit_single_theme_ops(prop: &str, expr: ThemeExpr) -> Vec<StyleOp> {
    let fb = resolve_fallback(&expr);
    let fb_px = LengthSpec::Px(fb.max(0.0));
    let fb_signed = LengthSpec::Px(fb);
    match prop {
        "gap" => vec![
            StyleOp::Gap(Some(fb_px)),
            StyleOp::RowGap(None),
            StyleOp::ColumnGap(None),
            StyleOp::ThemeGap(Some(expr)),
            StyleOp::ThemeRowGap(None),
            StyleOp::ThemeColumnGap(None),
        ],
        "row-gap" => vec![
            StyleOp::RowGap(Some(fb_px)),
            StyleOp::ThemeRowGap(Some(expr)),
        ],
        "column-gap" => vec![
            StyleOp::ColumnGap(Some(fb_px)),
            StyleOp::ThemeColumnGap(Some(expr)),
        ],
        "padding" => vec![
            StyleOp::Padding(Some(fb_px)),
            StyleOp::ThemePadding(Some(expr)),
        ],
        "padding-top" => vec![
            StyleOp::PaddingTop(Some(fb_px)),
            StyleOp::ThemePaddingTop(Some(expr)),
        ],
        "padding-right" => vec![
            StyleOp::PaddingRight(Some(fb_px)),
            StyleOp::ThemePaddingRight(Some(expr)),
        ],
        "padding-bottom" => vec![
            StyleOp::PaddingBottom(Some(fb_px)),
            StyleOp::ThemePaddingBottom(Some(expr)),
        ],
        "padding-left" => vec![
            StyleOp::PaddingLeft(Some(fb_px)),
            StyleOp::ThemePaddingLeft(Some(expr)),
        ],
        "margin" => vec![
            StyleOp::Margin(Some(fb_signed)),
            StyleOp::ThemeMargin(Some(expr)),
        ],
        "margin-top" => vec![
            StyleOp::MarginTop(Some(fb_signed)),
            StyleOp::ThemeMarginTop(Some(expr)),
        ],
        "margin-right" => vec![
            StyleOp::MarginRight(Some(fb_signed)),
            StyleOp::ThemeMarginRight(Some(expr)),
        ],
        "margin-bottom" => vec![
            StyleOp::MarginBottom(Some(fb_signed)),
            StyleOp::ThemeMarginBottom(Some(expr)),
        ],
        "margin-left" => vec![
            StyleOp::MarginLeft(Some(fb_signed)),
            StyleOp::ThemeMarginLeft(Some(expr)),
        ],
        "top" => vec![
            StyleOp::OffsetTop(Some(fb_signed)),
            StyleOp::ThemeOffsetTop(Some(expr)),
        ],
        "right" => vec![
            StyleOp::OffsetRight(Some(fb_signed)),
            StyleOp::ThemeOffsetRight(Some(expr)),
        ],
        "bottom" => vec![
            StyleOp::OffsetBottom(Some(fb_signed)),
            StyleOp::ThemeOffsetBottom(Some(expr)),
        ],
        "left" => vec![
            StyleOp::OffsetLeft(Some(fb_signed)),
            StyleOp::ThemeOffsetLeft(Some(expr)),
        ],
        "width" => vec![StyleOp::Width(Some(fb_px)), StyleOp::ThemeWidth(Some(expr))],
        "height" => vec![
            StyleOp::Height(Some(fb_px)),
            StyleOp::ThemeHeight(Some(expr)),
        ],
        "min-width" => vec![
            StyleOp::MinWidth(Some(fb_px)),
            StyleOp::ThemeMinWidth(Some(expr)),
        ],
        "max-width" => vec![
            StyleOp::MaxWidth(Some(fb_px)),
            StyleOp::ThemeMaxWidth(Some(expr)),
        ],
        "min-height" => vec![
            StyleOp::MinHeight(Some(fb_px)),
            StyleOp::ThemeMinHeight(Some(expr)),
        ],
        "max-height" => vec![
            StyleOp::MaxHeight(Some(fb_px)),
            StyleOp::ThemeMaxHeight(Some(expr)),
        ],
        "flex-basis" => vec![
            StyleOp::FlexBasis(Some(fb_px)),
            StyleOp::ThemeFlexBasis(Some(expr)),
        ],
        "font-size" => vec![
            StyleOp::FontSize(Some(fb.max(0.0))),
            StyleOp::ThemeFontSize(Some(expr)),
        ],
        "font-weight" => vec![
            StyleOp::FontWeight(Some(fb.round().clamp(1.0, 1000.0) as u16)),
            StyleOp::ThemeFontWeight(Some(expr)),
        ],
        "line-height" => vec![
            StyleOp::LineHeight(Some(nana_ui_core::LineHeightSpec::Absolute(fb.max(0.0)))),
            StyleOp::ThemeLineHeight(Some(expr)),
        ],
        "letter-spacing" => vec![
            StyleOp::LetterSpacing(Some(fb)),
            StyleOp::ThemeLetterSpacing(Some(expr)),
        ],
        "border-width" => vec![
            StyleOp::BorderWidth(Some(fb.max(0.0))),
            StyleOp::BorderTopWidth(Some(fb.max(0.0))),
            StyleOp::BorderRightWidth(Some(fb.max(0.0))),
            StyleOp::BorderBottomWidth(Some(fb.max(0.0))),
            StyleOp::BorderLeftWidth(Some(fb.max(0.0))),
            StyleOp::ThemeBorderWidth(Some(expr)),
        ],
        "border-top-width" => vec![
            StyleOp::BorderTopWidth(Some(fb.max(0.0))),
            StyleOp::ThemeBorderTopWidth(Some(expr)),
        ],
        "border-right-width" => vec![
            StyleOp::BorderRightWidth(Some(fb.max(0.0))),
            StyleOp::ThemeBorderRightWidth(Some(expr)),
        ],
        "border-bottom-width" => vec![
            StyleOp::BorderBottomWidth(Some(fb.max(0.0))),
            StyleOp::ThemeBorderBottomWidth(Some(expr)),
        ],
        "border-left-width" => vec![
            StyleOp::BorderLeftWidth(Some(fb.max(0.0))),
            StyleOp::ThemeBorderLeftWidth(Some(expr)),
        ],
        "border-radius" => vec![
            StyleOp::BorderRadius(Some(fb.max(0.0))),
            StyleOp::BorderRadii(Some([fb_px; 4])),
            StyleOp::ThemeBorderRadius(Some(expr)),
        ],
        _ => Vec::new(),
    }
}
