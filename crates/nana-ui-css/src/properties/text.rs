//! Typography and text CSS properties parsed directly from `&[ComponentValue]`.
//!
//! Confirmed fixes vs legacy `css_map.rs`:
//! - `font-weight` preserves variable numeric weights `1..=1000` verbatim
//!   (NEVER rounds `550` to `600`).
//! - `font-family` selects the first usable non-generic named family in CSS
//!   author order (NEVER unconditionally elevates `"Noto Sans SC"` ahead of an
//!   explicit primary font!).

use nana_ui_core::typography::{
    TextEmphasisColor, TextEmphasisPosition, TextEmphasisShape, TextEmphasisStyle,
};
use nana_ui_core::{
    DominantBaselineSpec, FontFeatureSetting, FontKerningSpec, FontVariationSetting, HyphensSpec,
    LengthSpec, LineBreakSpec, LineHeightSpec, OverflowSpec, OverflowWrapSpec, PaintColor, StyleOp,
    TextAlignSpec, TextAutospaceSpec, TextDecorationLine, TextJustifySpec, TextOrientationSpec,
    TextShadowSpec, TextSpacingTrimSpec, TextStrokeSpec, TextWrapStyleSpec, VerticalAlignSpec,
    WhiteSpaceSpec, WordBreakSpec,
};

use crate::properties::{CompileContext, DeclState};
use crate::syntax::diag::{CssDiag, DiagCode};
use crate::syntax::parser::{ComponentValue, split_comma_groups, split_whitespace_items, trim_ws};
use crate::syntax::span::Span;
use crate::syntax::tokenizer::{SpannedToken, Token};
use crate::syntax::values::{ParsedColor, parse_color, parse_length_spec};

pub(crate) fn parse_text_property(
    prop: &str,
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
) -> Option<Result<Vec<StyleOp>, CssDiag>> {
    match prop {
        "color" => Some(compile_color(values, span, ctx)),
        "font-size" => Some(compile_font_size(values, span, ctx)),
        "font-weight" => Some(compile_font_weight(values, span)),
        "font-style" => Some(compile_font_style(values, span)),
        "font-family" => Some(compile_font_family(values, span)),
        "line-height" => Some(compile_line_height(values, span, ctx)),
        "letter-spacing" => Some(compile_letter_spacing(values, span, ctx)),
        "font-kerning" => Some(compile_font_kerning(values, span)),
        "font-feature-settings" => Some(compile_font_feature_settings(values, span)),
        "font-variation-settings" => Some(compile_font_variation_settings(values, span)),
        "text-align" => Some(compile_text_align(values, span)),
        "text-overflow" => Some(compile_text_overflow(values, span)),
        "white-space" => Some(compile_white_space(values, span)),
        "word-break" => Some(compile_word_break(values, span)),
        "overflow-wrap" | "word-wrap" => Some(compile_overflow_wrap(values, span)),
        "line-break" => Some(compile_line_break(values, span)),
        "hyphens" => Some(compile_hyphens(values, span)),
        "dominant-baseline" => Some(compile_dominant_baseline(values, span)),
        "vertical-align" => Some(compile_vertical_align(values, span)),
        "text-orientation" => Some(compile_text_orientation(values, span)),
        "text-spacing-trim" => Some(compile_text_spacing_trim(values, span)),
        "text-autospace" => Some(compile_text_autospace(values, span)),
        "text-justify" => Some(compile_text_justify(values, span)),
        "text-wrap-style" => Some(compile_text_wrap_style(values, span)),
        "line-clamp" | "-webkit-line-clamp" => Some(compile_line_clamp(values, span)),
        "text-decoration" | "text-decoration-line" => Some(compile_text_decoration(values, span)),
        "text-emphasis" | "-webkit-text-emphasis" => {
            Some(compile_text_emphasis_shorthand(values, span, ctx))
        }
        "text-emphasis-style" | "-webkit-text-emphasis-style" => {
            Some(compile_text_emphasis_style(values, span))
        }
        "text-emphasis-position" | "-webkit-text-emphasis-position" => {
            Some(compile_text_emphasis_position(values, span))
        }
        "text-emphasis-color" | "-webkit-text-emphasis-color" => {
            Some(compile_text_emphasis_color(values, span, ctx))
        }
        "text-shadow" => Some(compile_text_shadow(values, span, ctx)),
        "-webkit-text-stroke" => Some(compile_text_stroke_shorthand(values, span, ctx, state)),
        "-webkit-text-stroke-width" => Some(compile_text_stroke_width(values, span, ctx, state)),
        "-webkit-text-stroke-color" => Some(compile_text_stroke_color(values, span, ctx, state)),
        "paint-order" => Some(compile_paint_order(values, span)),
        _ => None,
    }
}

fn compile_color(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if matches!(trimmed, [cv] if is_inherit_or_unset(cv)) {
        return Ok(vec![StyleOp::Color(None)]);
    }
    let pc = parse_color(trimmed, ctx.color_opts)
        .and_then(ParsedColor::into_paint)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `color` value"))?;
    Ok(vec![StyleOp::Color(Some(pc))])
}

fn compile_font_size(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed {
        if let Some(kw) = cv.as_ident() {
            let px = match kw.to_ascii_lowercase().as_str() {
                "inherit" | "unset" | "revert" => return Ok(Vec::new()),
                "initial" | "medium" => 16.0,
                "xx-small" => 9.0,
                "x-small" => 10.0,
                "small" => 13.0,
                "large" => 18.0,
                "x-large" => 24.0,
                "xx-large" => 32.0,
                "xxx-large" => 48.0,
                "smaller" => (ctx.font_sizes.element_px * 0.83).max(1.0),
                "larger" => (ctx.font_sizes.element_px * 1.2).max(1.0),
                _ => {
                    return Err(CssDiag::error(
                        span,
                        DiagCode::InvalidValue,
                        format!("invalid `font-size: {kw}`"),
                    ));
                }
            };
            return Ok(vec![
                StyleOp::FontSize(Some(px)),
                StyleOp::ThemeFontSize(None),
            ]);
        }
        if let Some(Token::Percentage(p)) = cv.as_token()
            && p.is_finite()
            && *p >= 0.0
        {
            let px = (ctx.font_sizes.element_px * (*p as f32) / 100.0).max(0.0);
            return Ok(vec![
                StyleOp::FontSize(Some(px)),
                StyleOp::ThemeFontSize(None),
            ]);
        }
    }
    let spec = parse_length_spec(trimmed)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `font-size`"))?;
    let px = ctx
        .resolve_length_px(spec)
        .filter(|v| *v >= 0.0)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `font-size`"))?;
    Ok(vec![
        StyleOp::FontSize(Some(px)),
        StyleOp::ThemeFontSize(None),
    ])
}

/// CSS Fonts 4 `font-weight`: preserves variable numeric weights `1..=1000`
/// verbatim without rounding to 100s!
fn compile_font_weight(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `font-weight`",
        ));
    };
    let w = match cv.as_token() {
        Some(Token::Ident(kw)) => match kw.to_ascii_lowercase().as_str() {
            "inherit" | "unset" | "revert" => return Ok(Vec::new()),
            "normal" | "initial" => 400u16,
            "bold" => 700,
            "lighter" => 300,
            "bolder" => 700,
            _ => {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    format!("invalid `font-weight: {kw}`"),
                ));
            }
        },
        Some(Token::Number { value, .. })
            if value.is_finite() && (1.0..=1000.0).contains(value) =>
        {
            value.round() as u16
        }
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `font-weight` value",
            ));
        }
    };
    Ok(vec![
        StyleOp::FontWeight(Some(w)),
        StyleOp::ThemeFontWeight(None),
    ])
}

fn compile_font_style(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    let Some(first) = items.first() else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `font-style`",
        ));
    };
    let [cv] = *first else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `font-style`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `font-style`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "normal" if items.len() == 1 => Ok(vec![StyleOp::FontItalic(Some(false))]),
        "italic" if items.len() == 1 => Ok(vec![StyleOp::FontItalic(Some(true))]),
        "oblique" if items.len() <= 2 => Ok(vec![StyleOp::FontItalic(Some(true))]),
        "initial" if items.len() == 1 => Ok(vec![StyleOp::FontItalic(Some(false))]),
        "inherit" | "unset" if items.len() == 1 => Ok(Vec::new()),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid `font-style: {kw}`"),
        )),
    }
}

/// First usable non-generic family in CSS author order (NEVER unconditionally
/// elevating `"Noto Sans SC"` above an earlier primary font).
fn compile_font_family(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `font-family`",
        ));
    }
    if let [cv] = trimmed
        && cv.as_ident().is_some_and(|kw| {
            matches!(
                kw.to_ascii_lowercase().as_str(),
                "inherit" | "unset" | "revert" | "initial"
            )
        })
    {
        return Ok(Vec::new());
    }

    let groups = split_comma_groups(trimmed);
    let mut first_named: Option<String> = None;
    let mut saw_mono = false;
    let mut saw_any_valid = false;

    for group in groups {
        let name = parse_single_family_name(group).ok_or_else(|| {
            CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid family name in `font-family`",
            )
        })?;
        saw_any_valid = true;
        let lower = name.to_ascii_lowercase();
        if is_generic_font_family(&lower) {
            if lower == "monospace" || lower == "ui-monospace" {
                saw_mono = true;
            }
            continue;
        }
        if first_named.is_none() {
            first_named = Some(name);
        }
    }

    if !saw_any_valid {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `font-family`",
        ));
    }

    if let Some(family) = first_named {
        Ok(vec![StyleOp::FontFamily(Some(family))])
    } else if saw_mono {
        Ok(vec![StyleOp::FontFamily(Some("monospace".into()))])
    } else {
        Ok(Vec::new())
    }
}

fn parse_single_family_name(values: &[ComponentValue]) -> Option<String> {
    let trimmed = trim_ws(values);
    if let [
        ComponentValue::Token(SpannedToken {
            token: Token::String(s),
            ..
        }),
    ] = trimmed
    {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_string());
    }
    let mut parts = Vec::new();
    for item in split_whitespace_items(trimmed) {
        let [cv] = item else { return None };
        let id = cv.as_ident()?;
        parts.push(id);
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn is_generic_font_family(lower: &str) -> bool {
    matches!(
        lower,
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
    )
}

fn compile_line_height(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed {
        if let Some(kw) = cv.as_ident() {
            return match kw.to_ascii_lowercase().as_str() {
                "normal" | "initial" => Ok(vec![
                    StyleOp::LineHeight(None),
                    StyleOp::ThemeLineHeight(None),
                ]),
                "inherit" | "unset" | "revert" => Ok(Vec::new()),
                _ => Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    format!("invalid `line-height: {kw}`"),
                )),
            };
        }
        match cv.as_token() {
            Some(Token::Number { value, .. }) if value.is_finite() && *value >= 0.0 => {
                return Ok(vec![
                    StyleOp::LineHeight(Some(LineHeightSpec::Relative(*value as f32))),
                    StyleOp::ThemeLineHeight(None),
                ]);
            }
            Some(Token::Percentage(p)) if p.is_finite() && *p >= 0.0 => {
                return Ok(vec![
                    StyleOp::LineHeight(Some(LineHeightSpec::Relative(*p as f32 / 100.0))),
                    StyleOp::ThemeLineHeight(None),
                ]);
            }
            _ => {}
        }
    }
    let spec = parse_length_spec(trimmed)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `line-height`"))?;
    let px = ctx
        .resolve_length_px(spec)
        .filter(|v| *v >= 0.0)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `line-height`"))?;
    Ok(vec![
        StyleOp::LineHeight(Some(LineHeightSpec::Absolute(px))),
        StyleOp::ThemeLineHeight(None),
    ])
}

fn compile_letter_spacing(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && let Some(kw) = cv.as_ident()
    {
        return match kw.to_ascii_lowercase().as_str() {
            "normal" | "initial" => Ok(vec![
                StyleOp::LetterSpacing(Some(0.0)),
                StyleOp::ThemeLetterSpacing(None),
            ]),
            "inherit" | "unset" | "revert" => Ok(Vec::new()),
            _ => Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `letter-spacing: {kw}`"),
            )),
        };
    }
    let spec = parse_length_spec(trimmed)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `letter-spacing`"))?;
    let px = ctx
        .resolve_length_px(spec)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `letter-spacing`"))?;
    Ok(vec![
        StyleOp::LetterSpacing(Some(px)),
        StyleOp::ThemeLetterSpacing(None),
    ])
}

fn compile_font_kerning(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `font-kerning`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `font-kerning`"))?;
    let spec = match kw.to_ascii_lowercase().as_str() {
        "auto" | "initial" => FontKerningSpec::Auto,
        "normal" => FontKerningSpec::Normal,
        "none" => FontKerningSpec::None,
        "inherit" | "unset" | "revert" => return Ok(Vec::new()),
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `font-kerning: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::FontKerning(Some(spec))])
}

fn compile_font_feature_settings(
    values: &[ComponentValue],
    span: Span,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && let Some(kw) = cv.as_ident()
    {
        return match kw.to_ascii_lowercase().as_str() {
            "normal" | "initial" => Ok(vec![StyleOp::FontFeatures(Some(Vec::new()))]),
            "inherit" | "unset" | "revert" => Ok(Vec::new()),
            _ => Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `font-feature-settings`",
            )),
        };
    }
    let groups = split_comma_groups(trimmed);
    if groups.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `font-feature-settings`",
        ));
    }
    let mut out = Vec::with_capacity(groups.len());
    for g in groups {
        let items = split_whitespace_items(g);
        let Some(first) = items.first() else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `font-feature-settings` item",
            ));
        };
        let tag = parse_ot_tag(first).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid OpenType feature tag")
        })?;
        let val = match items.get(1) {
            None => 1u32,
            Some([cv]) => match cv.as_token() {
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("on") => 1,
                Some(Token::Ident(s)) if s.eq_ignore_ascii_case("off") => 0,
                Some(Token::Number {
                    value,
                    int: Some(_),
                    ..
                }) if *value >= 0.0 => *value as u32,
                _ => {
                    return Err(CssDiag::error(
                        span,
                        DiagCode::InvalidValue,
                        "invalid OpenType feature value",
                    ));
                }
            },
            _ => {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    "invalid `font-feature-settings` item",
                ));
            }
        };
        if items.len() > 2 {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "unexpected tokens in `font-feature-settings`",
            ));
        }
        out.push(FontFeatureSetting::new(tag, val));
    }
    Ok(vec![StyleOp::FontFeatures(Some(out))])
}

fn compile_font_variation_settings(
    values: &[ComponentValue],
    span: Span,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && let Some(kw) = cv.as_ident()
    {
        return match kw.to_ascii_lowercase().as_str() {
            "normal" | "initial" => Ok(vec![
                StyleOp::UnsupportedFontVariation(false),
                StyleOp::FontVariationSettings(Some(Vec::new())),
            ]),
            "inherit" | "unset" | "revert" => Ok(Vec::new()),
            _ => Ok(vec![StyleOp::UnsupportedFontVariation(true)]),
        };
    }
    let groups = split_comma_groups(trimmed);
    if groups.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `font-variation-settings`",
        ));
    }
    let mut axes = Vec::with_capacity(groups.len());
    for g in groups {
        let items = split_whitespace_items(g);
        if items.len() != 2 {
            return Ok(vec![StyleOp::UnsupportedFontVariation(true)]);
        }
        let Some(tag) = parse_ot_tag(items[0]) else {
            return Ok(vec![StyleOp::UnsupportedFontVariation(true)]);
        };
        let [
            ComponentValue::Token(SpannedToken {
                token: Token::Number { value, .. },
                ..
            }),
        ] = items[1]
        else {
            return Ok(vec![StyleOp::UnsupportedFontVariation(true)]);
        };
        if !value.is_finite() {
            return Ok(vec![StyleOp::UnsupportedFontVariation(true)]);
        }
        axes.push(FontVariationSetting::new(tag, *value as f32));
    }
    let wght = axes
        .iter()
        .rev()
        .find(|a| a.tag == *b"wght")
        .map(|a| a.value);
    let mut ops = vec![
        StyleOp::UnsupportedFontVariation(false),
        StyleOp::FontVariationSettings(Some(axes)),
    ];
    if let Some(w) = wght {
        ops.push(StyleOp::FontWeight(Some(
            w.round().clamp(1.0, 1000.0) as u16
        )));
    }
    Ok(ops)
}

fn parse_ot_tag(values: &[ComponentValue]) -> Option<[u8; 4]> {
    let [
        ComponentValue::Token(SpannedToken {
            token: Token::String(s),
            ..
        }),
    ] = values
    else {
        return None;
    };
    if s.len() != 4 || !s.is_ascii() {
        return None;
    }
    let mut tag = [0u8; 4];
    tag.copy_from_slice(s.as_bytes());
    Some(tag)
}

fn compile_text_align(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `text-align`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `text-align`"))?;
    let align = match kw.to_ascii_lowercase().as_str() {
        "start" => TextAlignSpec::Start,
        "end" => TextAlignSpec::End,
        "left" => TextAlignSpec::Left,
        "right" => TextAlignSpec::Right,
        "center" => TextAlignSpec::Center,
        "justify" => TextAlignSpec::Justify,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `text-align: {kw}`"),
            ));
        }
    };
    Ok(vec![
        StyleOp::TextAlign(align),
        StyleOp::TextJustifyLines(Some(align == TextAlignSpec::Justify)),
    ])
}

fn compile_text_overflow(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `text-overflow`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `text-overflow`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "ellipsis" => Ok(vec![StyleOp::TextOverflowEllipsis(true)]),
        "clip" => Ok(vec![StyleOp::TextOverflowEllipsis(false)]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid `text-overflow: {kw}`"),
        )),
    }
}

fn compile_white_space(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `white-space`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `white-space`"))?;
    let spec = match kw.to_ascii_lowercase().as_str() {
        "normal" | "wrap" => WhiteSpaceSpec::Normal,
        "nowrap" => WhiteSpaceSpec::Nowrap,
        "pre" => WhiteSpaceSpec::Pre,
        "pre-wrap" => WhiteSpaceSpec::PreWrap,
        "pre-line" => WhiteSpaceSpec::PreLine,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `white-space: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::WhiteSpace(spec)])
}

fn compile_word_break(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `word-break`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `word-break`"))?;
    let spec = match kw.to_ascii_lowercase().as_str() {
        "normal" | "initial" | "keep-all" => WordBreakSpec::Normal,
        "break-all" => WordBreakSpec::BreakAll,
        "break-word" => WordBreakSpec::BreakWord,
        "inherit" | "unset" | "revert" => return Ok(Vec::new()),
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `word-break: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::WordBreak(Some(spec))])
}

fn compile_overflow_wrap(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `overflow-wrap`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `overflow-wrap`"))?;
    let spec = match kw.to_ascii_lowercase().as_str() {
        "normal" => OverflowWrapSpec::Normal,
        "anywhere" | "break-word" => OverflowWrapSpec::Anywhere,
        "inherit" | "unset" | "revert" => return Ok(Vec::new()),
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `overflow-wrap: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::OverflowWrap(Some(spec))])
}

fn compile_line_break(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `line-break`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `line-break`"))?;
    let spec = match kw.to_ascii_lowercase().as_str() {
        "auto" | "initial" => LineBreakSpec::Auto,
        "normal" => LineBreakSpec::Normal,
        "anywhere" => LineBreakSpec::Anywhere,
        "strict" => LineBreakSpec::Strict,
        "loose" => LineBreakSpec::Loose,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `line-break: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::LineBreak(Some(spec))])
}

fn compile_hyphens(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `hyphens`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `hyphens`"))?;
    let spec = match kw.to_ascii_lowercase().as_str() {
        "none" => HyphensSpec::None,
        "manual" | "initial" => HyphensSpec::Manual,
        "auto" => HyphensSpec::Auto,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `hyphens: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::Hyphens(Some(spec))])
}

fn compile_dominant_baseline(
    values: &[ComponentValue],
    span: Span,
) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `dominant-baseline`",
        ));
    };
    let kw = cv.as_ident().ok_or_else(|| {
        CssDiag::error(span, DiagCode::InvalidValue, "invalid `dominant-baseline`")
    })?;
    let spec = match kw.to_ascii_lowercase().as_str() {
        "auto" | "initial" => DominantBaselineSpec::Auto,
        "central" => DominantBaselineSpec::Central,
        "alphabetic" => DominantBaselineSpec::Alphabetic,
        "ideographic" => DominantBaselineSpec::Ideographic,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `dominant-baseline: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::DominantBaseline(Some(spec))])
}

fn compile_vertical_align(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && let Some(kw) = cv.as_ident()
    {
        let va = match kw.to_ascii_lowercase().as_str() {
            "baseline" | "initial" | "unset" => VerticalAlignSpec::Baseline,
            "sub" => VerticalAlignSpec::Sub,
            "super" => VerticalAlignSpec::Super,
            "text-top" => VerticalAlignSpec::TextTop,
            "text-bottom" => VerticalAlignSpec::TextBottom,
            "middle" => VerticalAlignSpec::Middle,
            "top" => VerticalAlignSpec::Top,
            "bottom" => VerticalAlignSpec::Bottom,
            _ => {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    format!("invalid `vertical-align: {kw}`"),
                ));
            }
        };
        return Ok(vec![StyleOp::VerticalAlign(va)]);
    }
    let spec = parse_length_spec(trimmed)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `vertical-align`"))?;
    let va = match spec {
        LengthSpec::Px(0.0) => VerticalAlignSpec::Baseline,
        other => VerticalAlignSpec::Offset(other),
    };
    Ok(vec![StyleOp::VerticalAlign(va)])
}

fn compile_text_orientation(
    values: &[ComponentValue],
    span: Span,
) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `text-orientation`",
        ));
    };
    let kw = cv.as_ident().ok_or_else(|| {
        CssDiag::error(span, DiagCode::InvalidValue, "invalid `text-orientation`")
    })?;
    let next = match kw.to_ascii_lowercase().as_str() {
        "mixed" | "initial" => Some(TextOrientationSpec::Mixed),
        "upright" => Some(TextOrientationSpec::Upright),
        "sideways" | "sideways-right" => Some(TextOrientationSpec::Sideways),
        "inherit" | "unset" => None,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `text-orientation: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::TextOrientation(next)])
}

fn compile_text_spacing_trim(
    values: &[ComponentValue],
    span: Span,
) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `text-spacing-trim`",
        ));
    };
    let kw = cv.as_ident().ok_or_else(|| {
        CssDiag::error(span, DiagCode::InvalidValue, "invalid `text-spacing-trim`")
    })?;
    let spec = match kw.to_ascii_lowercase().as_str() {
        "space-all" => TextSpacingTrimSpec::SpaceAll,
        "normal" | "space-first" => TextSpacingTrimSpec::Normal,
        "trim-both" => TextSpacingTrimSpec::TrimBoth,
        "auto" => TextSpacingTrimSpec::Auto,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `text-spacing-trim: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::TextSpacingTrim(Some(spec))])
}

fn compile_text_autospace(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `text-autospace`",
        ));
    }
    if items.len() == 1
        && let [cv] = items[0]
        && let Some(kw) = cv.as_ident()
    {
        return match kw.to_ascii_lowercase().as_str() {
            "no-autospace" => Ok(vec![StyleOp::TextAutospace(Some(
                TextAutospaceSpec::NoAutospace,
            ))]),
            "normal" | "auto" => Ok(vec![StyleOp::TextAutospace(Some(
                TextAutospaceSpec::Normal,
            ))]),
            "ideograph-alpha" | "ideograph-numeric" => Ok(vec![StyleOp::TextAutospace(Some(
                TextAutospaceSpec::Normal,
            ))]),
            _ => Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `text-autospace: {kw}`"),
            )),
        };
    }
    let all_known = items.iter().all(|it| {
        matches!(it, [cv] if cv.as_ident().is_some_and(|kw| {
            kw.eq_ignore_ascii_case("ideograph-alpha")
                || kw.eq_ignore_ascii_case("ideograph-numeric")
        }))
    });
    if all_known {
        Ok(vec![StyleOp::TextAutospace(Some(
            TextAutospaceSpec::Normal,
        ))])
    } else {
        Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `text-autospace`",
        ))
    }
}

fn compile_text_justify(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `text-justify`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `text-justify`"))?;
    let spec = match kw.to_ascii_lowercase().as_str() {
        "auto" => TextJustifySpec::Auto,
        "none" => TextJustifySpec::None,
        "inter-word" => TextJustifySpec::InterWord,
        "inter-character" | "distribute" => TextJustifySpec::InterCharacter,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `text-justify: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::TextJustify(Some(spec))])
}

fn compile_text_wrap_style(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `text-wrap-style`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `text-wrap-style`"))?;
    let spec = match kw.to_ascii_lowercase().as_str() {
        "auto" | "stable" => TextWrapStyleSpec::Auto,
        "pretty" => TextWrapStyleSpec::Pretty,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `text-wrap-style: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::TextWrapStyle(Some(spec))])
}

fn compile_line_clamp(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `line-clamp`",
        ));
    };
    match cv.as_token() {
        Some(Token::Ident(s)) if s.eq_ignore_ascii_case("none") => {
            Ok(vec![StyleOp::LineClamp(None)])
        }
        Some(Token::Number {
            value,
            int: Some(_),
            ..
        }) if *value == 0.0 => Ok(vec![StyleOp::LineClamp(None)]),
        Some(Token::Number {
            value,
            int: Some(_),
            ..
        }) if *value >= 1.0 && *value <= f64::from(u16::MAX) => Ok(vec![
            StyleOp::LineClamp(Some(*value as u16)),
            StyleOp::TextOverflowEllipsis(true),
            StyleOp::OverflowX(OverflowSpec::Hidden),
            StyleOp::OverflowY(OverflowSpec::Hidden),
        ]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `line-clamp` value",
        )),
    }
}

fn compile_text_decoration(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `text-decoration`",
        ));
    }
    if items.len() == 1
        && matches!(items[0], [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("none")))
    {
        return Ok(vec![StyleOp::TextDecoration(Some(
            TextDecorationLine::default(),
        ))]);
    }
    let mut deco = TextDecorationLine::default();
    let mut saw_line = false;
    for it in items {
        let [cv] = it else { continue };
        let Some(kw) = cv.as_ident() else { continue };
        match kw.to_ascii_lowercase().as_str() {
            "underline" => {
                deco.underline = true;
                saw_line = true;
            }
            "line-through" => {
                deco.line_through = true;
                saw_line = true;
            }
            "overline" | "blink" | "spelling-error" | "grammar-error" | "solid" | "double"
            | "dotted" | "dashed" | "wavy" => {}
            _ => {}
        }
    }
    if saw_line {
        Ok(vec![StyleOp::TextDecoration(Some(deco))])
    } else {
        Ok(Vec::new())
    }
}

fn css_global_reset(values: &[ComponentValue]) -> Option<bool> {
    let [cv] = trim_ws(values) else { return None };
    match cv.as_ident()?.to_ascii_lowercase().as_str() {
        "inherit" | "unset" => Some(false),
        "initial" => Some(true),
        _ => None,
    }
}

fn compile_text_emphasis_shorthand(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    if let Some(is_initial) = css_global_reset(values) {
        return Ok(vec![
            StyleOp::TextEmphasisStyle(is_initial.then_some(TextEmphasisStyle::None)),
            StyleOp::TextEmphasisColor(is_initial.then_some(TextEmphasisColor::CurrentColor)),
        ]);
    }
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `text-emphasis`",
        ));
    }
    let mut marks: Vec<ComponentValue> = Vec::new();
    let mut color: Option<TextEmphasisColor> = None;
    for item in items {
        if color.is_none()
            && let Some(c) = parse_emphasis_color(item, ctx)
        {
            color = Some(c);
        } else {
            if !marks.is_empty() {
                marks.push(ComponentValue::Token(SpannedToken::new(
                    Token::Whitespace,
                    Span::EMPTY,
                )));
            }
            marks.extend_from_slice(item);
        }
    }
    let style = if marks.is_empty() {
        TextEmphasisStyle::None
    } else {
        parse_emphasis_style(&marks).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `text-emphasis`")
        })?
    };
    Ok(vec![
        StyleOp::TextEmphasisStyle(Some(style)),
        StyleOp::TextEmphasisColor(Some(color.unwrap_or_default())),
    ])
}

fn compile_text_emphasis_style(
    values: &[ComponentValue],
    span: Span,
) -> Result<Vec<StyleOp>, CssDiag> {
    if let Some(is_initial) = css_global_reset(values) {
        return Ok(vec![StyleOp::TextEmphasisStyle(
            is_initial.then_some(TextEmphasisStyle::None),
        )]);
    }
    let style = parse_emphasis_style(values).ok_or_else(|| {
        CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `text-emphasis-style`",
        )
    })?;
    Ok(vec![StyleOp::TextEmphasisStyle(Some(style))])
}

fn parse_emphasis_style(values: &[ComponentValue]) -> Option<TextEmphasisStyle> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed {
        if let Some(Token::String(s)) = cv.as_token() {
            return (!s.is_empty()).then(|| TextEmphasisStyle::String(s.clone().into()));
        }
        if cv
            .as_ident()
            .is_some_and(|s| s.eq_ignore_ascii_case("none"))
        {
            return Some(TextEmphasisStyle::None);
        }
    }
    let items = split_whitespace_items(trimmed);
    let mut open: Option<bool> = None;
    let mut shape: Option<TextEmphasisShape> = None;
    for it in items {
        let [cv] = it else { return None };
        let kw = cv.as_ident()?.to_ascii_lowercase();
        match kw.as_str() {
            "filled" if open.is_none() => open = Some(false),
            "open" if open.is_none() => open = Some(true),
            "dot" if shape.is_none() => shape = Some(TextEmphasisShape::Dot),
            "circle" if shape.is_none() => shape = Some(TextEmphasisShape::Circle),
            "double-circle" if shape.is_none() => shape = Some(TextEmphasisShape::DoubleCircle),
            "triangle" if shape.is_none() => shape = Some(TextEmphasisShape::Triangle),
            "sesame" if shape.is_none() => shape = Some(TextEmphasisShape::Sesame),
            _ => return None,
        }
    }
    (open.is_some() || shape.is_some()).then_some(TextEmphasisStyle::Mark {
        shape,
        open: open.unwrap_or(false),
    })
}

fn compile_text_emphasis_position(
    values: &[ComponentValue],
    span: Span,
) -> Result<Vec<StyleOp>, CssDiag> {
    if let Some(is_initial) = css_global_reset(values) {
        return Ok(vec![StyleOp::TextEmphasisPosition(
            is_initial.then_some(TextEmphasisPosition::OVER),
        )]);
    }
    let items = split_whitespace_items(values);
    let mut under: Option<bool> = None;
    let mut left: Option<bool> = None;
    for it in items {
        let [cv] = it else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `text-emphasis-position`",
            ));
        };
        let Some(kw) = cv.as_ident() else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `text-emphasis-position`",
            ));
        };
        match kw.to_ascii_lowercase().as_str() {
            "over" if under.is_none() => under = Some(false),
            "under" if under.is_none() => under = Some(true),
            "right" if left.is_none() => left = Some(false),
            "left" if left.is_none() => left = Some(true),
            _ => {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    "invalid `text-emphasis-position`",
                ));
            }
        }
    }
    let under = under.ok_or_else(|| {
        CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "missing `over`/`under` in `text-emphasis-position`",
        )
    })?;
    Ok(vec![StyleOp::TextEmphasisPosition(Some(
        TextEmphasisPosition {
            under,
            left: left.unwrap_or(false),
        },
    ))])
}

fn compile_text_emphasis_color(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    if let Some(is_initial) = css_global_reset(values) {
        return Ok(vec![StyleOp::TextEmphasisColor(
            is_initial.then_some(TextEmphasisColor::CurrentColor),
        )]);
    }
    let color = parse_emphasis_color(values, ctx).ok_or_else(|| {
        CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `text-emphasis-color`",
        )
    })?;
    Ok(vec![StyleOp::TextEmphasisColor(Some(color))])
}

fn parse_emphasis_color(
    values: &[ComponentValue],
    ctx: &CompileContext,
) -> Option<TextEmphasisColor> {
    match parse_color(values, ctx.color_opts)? {
        ParsedColor::CurrentColor => Some(TextEmphasisColor::CurrentColor),
        ParsedColor::Paint(pc) => Some(TextEmphasisColor::Color(pc)),
    }
}

fn compile_text_shadow(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `text-shadow`",
        ));
    }
    if let [cv] = trimmed
        && cv
            .as_ident()
            .is_some_and(|s| s.eq_ignore_ascii_case("none"))
    {
        return Ok(vec![StyleOp::TextShadows(Vec::new())]);
    }
    let groups = split_comma_groups(trimmed);
    let mut shadows = Vec::new();
    for g in groups {
        let shadow = parse_one_text_shadow(g, ctx).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `text-shadow` layer")
        })?;
        shadows.push(shadow);
        if shadows.len() >= nana_ui_core::MAX_TEXT_SHADOWS {
            break;
        }
    }
    Ok(vec![StyleOp::TextShadows(shadows)])
}

fn parse_one_text_shadow(
    values: &[ComponentValue],
    ctx: &CompileContext,
) -> Option<TextShadowSpec> {
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return None;
    }
    let mut lengths = Vec::new();
    let mut color: Option<PaintColor> = None;
    for item in items {
        if let Some(ParsedColor::Paint(pc)) = parse_color(item, ctx.color_opts) {
            if color.is_some() {
                return None;
            }
            color = Some(pc);
            continue;
        }
        let spec = parse_length_spec(item)?;
        let px = ctx.resolve_length_px(spec)?;
        lengths.push(px);
    }
    if lengths.len() < 2 || lengths.len() > 3 {
        return None;
    }
    let blur_radius = match lengths.get(2) {
        Some(&b) if b >= 0.0 => b,
        Some(_) => return None,
        None => 0.0,
    };
    Some(TextShadowSpec {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_radius,
        color: color.unwrap_or(PaintColor::srgb([0.0, 0.0, 0.0, 0.5])),
    })
}

fn compile_text_stroke_shorthand(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `-webkit-text-stroke`",
        ));
    }
    let mut width: Option<f32> = None;
    let mut color: Option<PaintColor> = None;
    for item in items {
        if width.is_none()
            && let Some(w) = parse_stroke_width(item, ctx)
        {
            width = Some(w);
        } else if let Some(parsed) = parse_color(item, ctx.color_opts) {
            color = parsed.into_paint();
        } else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `-webkit-text-stroke`",
            ));
        }
    }
    let stroke = width
        .filter(|w| *w > 0.0)
        .map(|w| TextStrokeSpec { width: w, color });
    state.text_stroke = stroke;
    Ok(vec![StyleOp::TextStroke(stroke)])
}

fn compile_text_stroke_width(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let w = parse_stroke_width(values, ctx).ok_or_else(|| {
        CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `-webkit-text-stroke-width`",
        )
    })?;
    let mut stroke = state.text_stroke.unwrap_or(TextStrokeSpec {
        width: 0.0,
        color: None,
    });
    stroke.width = w;
    state.text_stroke = Some(stroke);
    Ok(vec![StyleOp::TextStroke(Some(stroke))])
}

fn compile_text_stroke_color(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let parsed = parse_color(values, ctx.color_opts).ok_or_else(|| {
        CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `-webkit-text-stroke-color`",
        )
    })?;
    let mut stroke = state.text_stroke.unwrap_or(TextStrokeSpec {
        width: 0.0,
        color: None,
    });
    stroke.color = parsed.into_paint();
    state.text_stroke = Some(stroke);
    Ok(vec![StyleOp::TextStroke(Some(stroke))])
}

fn parse_stroke_width(values: &[ComponentValue], ctx: &CompileContext) -> Option<f32> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && let Some(kw) = cv.as_ident()
    {
        return match kw.to_ascii_lowercase().as_str() {
            "thin" => Some(1.0),
            "medium" => Some(3.0),
            "thick" => Some(5.0),
            _ => None,
        };
    }
    let spec = parse_length_spec(trimmed)?;
    ctx.resolve_length_px(spec).filter(|v| *v >= 0.0)
}

fn compile_paint_order(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `paint-order`",
        ));
    }
    let mut order = Vec::with_capacity(items.len());
    for it in items {
        let [cv] = it else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `paint-order`",
            ));
        };
        let Some(kw) = cv.as_ident() else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `paint-order`",
            ));
        };
        order.push(kw.to_ascii_lowercase());
    }
    let stroke = order.iter().position(|t| t == "stroke");
    let fill = order.iter().position(|t| t == "fill");
    let stroke_first = match (stroke, fill) {
        (Some(s), Some(f)) => s < f,
        (Some(_), None) => true,
        _ => false,
    };
    Ok(vec![StyleOp::PaintOrderStrokeFirst(stroke_first)])
}

fn is_inherit_or_unset(cv: &ComponentValue) -> bool {
    cv.as_ident()
        .is_some_and(|s| s.eq_ignore_ascii_case("inherit") || s.eq_ignore_ascii_case("unset"))
}
