//! Pseudo-element declaration mappers:
//! - `::placeholder` -> `StyleOp::PlaceholderColor`, `StyleOp::PlaceholderOpacity`
//! - `::selection` -> `StyleOp::SelectionBackground`, `StyleOp::SelectionColor`
//! - `::-webkit-scrollbar*` -> `StyleOp::Scrollbar`

use nana_ui_core::{ScrollbarSkin, StyleOp};

use crate::properties::CompileContext;
use crate::syntax::diag::{CssDiag, DiagCode};
use crate::syntax::parser::{ComponentValue, split_whitespace_items, trim_ws};
use crate::syntax::selector::PseudoElement;
use crate::syntax::span::Span;
use crate::syntax::tokenizer::Token;
use crate::syntax::values::{parse_color, parse_length_spec};

/// Merge non-`None` / active fields of `overlay` onto `target`.
///
/// Used when combining multiple `::-webkit-scrollbar`, `::-webkit-scrollbar-thumb`,
/// and `::-webkit-scrollbar-track` rules targeting the same element into one
/// [`StyleOp::Scrollbar`] value.
pub fn merge_scrollbar_skin(target: &mut ScrollbarSkin, overlay: &ScrollbarSkin) {
    if overlay.thickness.is_some() {
        target.thickness = overlay.thickness;
    }
    if overlay.thumb_thickness.is_some() {
        target.thumb_thickness = overlay.thumb_thickness;
    }
    if overlay.thumb_active_thickness.is_some() {
        target.thumb_active_thickness = overlay.thumb_active_thickness;
    }
    if overlay.track_color.is_some() {
        target.track_color = overlay.track_color;
    }
    if overlay.track_hot_color.is_some() {
        target.track_hot_color = overlay.track_hot_color;
    }
    if overlay.thumb_color.is_some() {
        target.thumb_color = overlay.thumb_color;
    }
    if overlay.thumb_hot_color.is_some() {
        target.thumb_hot_color = overlay.thumb_hot_color;
    }
    if overlay.track_border_color.is_some() {
        target.track_border_color = overlay.track_border_color;
    }
    if overlay.thumb_border_color.is_some() {
        target.thumb_border_color = overlay.thumb_border_color;
    }
    if overlay.over_media {
        target.over_media = true;
    }
    if overlay.room.is_some() {
        target.room = overlay.room;
    }
}

/// Compile declarations targeting a paint-only [`PseudoElement`] (`::placeholder`,
/// `::selection`, `::-webkit-scrollbar*`).
///
/// Returns `Ok(ops)` for handled declarations or `Err(diag)` on invalid values.
pub(crate) fn compile_pseudo_property(
    pseudo: &PseudoElement,
    prop: &str,
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    scrollbar: &mut Option<ScrollbarSkin>,
) -> Result<Vec<StyleOp>, CssDiag> {
    match pseudo {
        PseudoElement::Placeholder => match prop {
            "color" => {
                let parsed = parse_color(values, ctx.color_opts).ok_or_else(|| {
                    CssDiag::error(
                        span,
                        DiagCode::InvalidValue,
                        "invalid `::placeholder` color",
                    )
                })?;
                let rgba = parsed.to_srgb_opt().ok_or_else(|| {
                    CssDiag::error(
                        span,
                        DiagCode::InvalidValue,
                        "`currentColor` is not supported for `::placeholder` color",
                    )
                })?;
                Ok(vec![StyleOp::PlaceholderColor(Some(rgba))])
            }
            "opacity" => {
                let [cv] = trim_ws(values) else {
                    return Err(CssDiag::error(
                        span,
                        DiagCode::InvalidValue,
                        "invalid `::placeholder` opacity",
                    ));
                };
                let v = match cv.as_token() {
                    Some(Token::Number { value, .. }) if value.is_finite() => {
                        (*value as f32).clamp(0.0, 1.0)
                    }
                    Some(Token::Percentage(p)) if p.is_finite() => {
                        (*p as f32 / 100.0).clamp(0.0, 1.0)
                    }
                    _ => {
                        return Err(CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid `::placeholder` opacity",
                        ));
                    }
                };
                Ok(vec![StyleOp::PlaceholderOpacity(Some(v))])
            }
            _ => Err(CssDiag::error(
                span,
                DiagCode::UnknownProperty,
                format!("unsupported property `{prop}` inside `::placeholder`"),
            )),
        },

        PseudoElement::Selection => match prop {
            "background" | "background-color" => {
                if is_none_or_transparent(values) {
                    return Ok(vec![StyleOp::SelectionBackground(None)]);
                }
                let pc = parse_color(values, ctx.color_opts)
                    .and_then(|c| c.into_paint())
                    .ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid `::selection` background color",
                        )
                    })?;
                Ok(vec![StyleOp::SelectionBackground(Some(pc))])
            }
            "color" => {
                let pc = parse_color(values, ctx.color_opts)
                    .and_then(|c| c.into_paint())
                    .ok_or_else(|| {
                        CssDiag::error(span, DiagCode::InvalidValue, "invalid `::selection` color")
                    })?;
                Ok(vec![StyleOp::SelectionColor(Some(pc))])
            }
            _ => Err(CssDiag::error(
                span,
                DiagCode::UnknownProperty,
                format!("unsupported property `{prop}` inside `::selection`"),
            )),
        },

        PseudoElement::WebkitScrollbar => {
            let mut skin = scrollbar.unwrap_or_default();
            match prop {
                "width" | "height" => {
                    let len = parse_length_spec(values).ok_or_else(|| {
                        CssDiag::error(span, DiagCode::InvalidValue, "invalid scrollbar thickness")
                    })?;
                    let px = ctx.resolve_length_px(len).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "scrollbar thickness must resolve to px",
                        )
                    })?;
                    if px < 0.0 {
                        return Err(CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "negative scrollbar thickness",
                        ));
                    }
                    skin.thickness = Some(px);
                    *scrollbar = Some(skin);
                    Ok(Vec::new())
                }
                "background" | "background-color" => {
                    let rgba = parse_srgb_color(values, ctx, span)?;
                    skin.track_color = Some(rgba);
                    *scrollbar = Some(skin);
                    Ok(Vec::new())
                }
                "border" | "border-color" => {
                    let rgba = parse_border_color_from_shorthand(values, ctx, span)?;
                    skin.track_border_color = Some(rgba);
                    *scrollbar = Some(skin);
                    Ok(Vec::new())
                }
                _ => Err(CssDiag::error(
                    span,
                    DiagCode::UnknownProperty,
                    format!("unsupported property `{prop}` inside `::-webkit-scrollbar`"),
                )),
            }
        }

        PseudoElement::WebkitScrollbarTrack => {
            let mut skin = scrollbar.unwrap_or_default();
            match prop {
                "background" | "background-color" => {
                    let rgba = parse_srgb_color(values, ctx, span)?;
                    skin.track_color = Some(rgba);
                    *scrollbar = Some(skin);
                    Ok(Vec::new())
                }
                "border" | "border-color" => {
                    let rgba = parse_border_color_from_shorthand(values, ctx, span)?;
                    skin.track_border_color = Some(rgba);
                    *scrollbar = Some(skin);
                    Ok(Vec::new())
                }
                _ => Err(CssDiag::error(
                    span,
                    DiagCode::UnknownProperty,
                    format!("unsupported property `{prop}` inside `::-webkit-scrollbar-track`"),
                )),
            }
        }

        PseudoElement::WebkitScrollbarThumb => {
            let mut skin = scrollbar.unwrap_or_default();
            match prop {
                "width" | "height" => {
                    let len = parse_length_spec(values).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid scrollbar thumb thickness",
                        )
                    })?;
                    let px = ctx.resolve_length_px(len).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "scrollbar thumb thickness must resolve to px",
                        )
                    })?;
                    if px < 0.0 {
                        return Err(CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "negative scrollbar thumb thickness",
                        ));
                    }
                    skin.thumb_thickness = Some(px);
                    *scrollbar = Some(skin);
                    Ok(Vec::new())
                }
                "background" | "background-color" => {
                    let rgba = parse_srgb_color(values, ctx, span)?;
                    skin.thumb_color = Some(rgba);
                    *scrollbar = Some(skin);
                    Ok(Vec::new())
                }
                "border" | "border-color" => {
                    let rgba = parse_border_color_from_shorthand(values, ctx, span)?;
                    skin.thumb_border_color = Some(rgba);
                    *scrollbar = Some(skin);
                    Ok(Vec::new())
                }
                "border-radius" => Ok(Vec::new()),
                _ => Err(CssDiag::error(
                    span,
                    DiagCode::UnknownProperty,
                    format!("unsupported property `{prop}` inside `::-webkit-scrollbar-thumb`"),
                )),
            }
        }

        PseudoElement::Before | PseudoElement::After | PseudoElement::Unknown(_) => {
            Err(CssDiag::error(
                span,
                DiagCode::UnsupportedSelector,
                format!("pseudo-element `{pseudo:?}` is not mapped onto StyleOp"),
            ))
        }
    }
}

fn is_none_or_transparent(values: &[ComponentValue]) -> bool {
    matches!(
        trim_ws(values),
        [cv] if cv.as_ident().is_some_and(|id| {
            id.eq_ignore_ascii_case("none") || id.eq_ignore_ascii_case("transparent")
        })
    )
}

fn parse_srgb_color(
    values: &[ComponentValue],
    ctx: &CompileContext,
    span: Span,
) -> Result<[f32; 4], CssDiag> {
    parse_color(values, ctx.color_opts)
        .and_then(|c| c.to_srgb_opt())
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid scrollbar color"))
}

fn parse_border_color_from_shorthand(
    values: &[ComponentValue],
    ctx: &CompileContext,
    span: Span,
) -> Result<[f32; 4], CssDiag> {
    if let Some(rgba) = parse_color(values, ctx.color_opts).and_then(|c| c.to_srgb_opt()) {
        return Ok(rgba);
    }
    for item in split_whitespace_items(values) {
        if let Some(rgba) = parse_color(item, ctx.color_opts).and_then(|c| c.to_srgb_opt()) {
            return Ok(rgba);
        }
    }
    Err(CssDiag::error(
        span,
        DiagCode::InvalidValue,
        "invalid scrollbar border color",
    ))
}
