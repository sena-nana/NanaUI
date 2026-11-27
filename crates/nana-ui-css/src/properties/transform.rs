//! CSS Transforms (`transform`, `transform-origin`, `transform-box`,
//! `perspective`, `perspective-origin`, `transform-style`, `backface-visibility`)
//! parsed directly from `&[ComponentValue]`.
//!
//! Fixes over the legacy string parser:
//! - Handles nested functions like `translateX(calc(10px + 5px))` cleanly via
//!   `ComponentValue::Function` trees (never truncates at the first `)`!).
//! - Implements `scaleZ(z)` properly (previously marked `used_3d` and then fell
//!   through to `_ => return None`).

use nana_ui_core::{
    BackfaceVisibility, LengthSpec, PaintMat4, PaintTransform, StyleOp, TransformBox,
    TransformOrigin,
};

use crate::properties::CompileContext;
use crate::syntax::diag::{CssDiag, DiagCode};
use crate::syntax::parser::{
    ComponentValue, serialize_component_values, split_comma_groups, split_whitespace_items, trim_ws,
};
use crate::syntax::span::Span;
use crate::syntax::tokenizer::Token;
use crate::syntax::values::{parse_angle_deg_or_zero, parse_length_spec};

/// Parsed CSS `transform` result: either a 2D affine matrix or a 4×4 3D matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParsedTransform {
    Affine(PaintTransform),
    Mat4(PaintMat4),
}

pub(crate) fn parse_transform_property(
    prop: &str,
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Option<Result<Vec<StyleOp>, CssDiag>> {
    match prop {
        "transform" => Some(compile_transform(values, span, ctx)),
        "transform-origin" => Some(compile_transform_origin(values, span, true)),
        "perspective-origin" => Some(compile_transform_origin(values, span, false)),
        "transform-box" => Some(compile_transform_box(values, span)),
        "perspective" => Some(compile_perspective(values, span, ctx)),
        "transform-style" => Some(compile_transform_style(values, span)),
        "backface-visibility" => Some(compile_backface_visibility(values, span)),
        _ => None,
    }
}

fn compile_transform(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `transform` value",
        ));
    }
    if let [cv] = trimmed
        && cv
            .as_ident()
            .is_some_and(|id| id.eq_ignore_ascii_case("none"))
    {
        return Ok(vec![
            StyleOp::Transform(None),
            StyleOp::Transform3d(None),
            StyleOp::UnsupportedTransform(None),
        ]);
    }
    match parse_transform_values(trimmed, ctx) {
        Some(ParsedTransform::Affine(affine)) => {
            let next = (!affine.is_identity()).then_some(affine);
            Ok(vec![
                StyleOp::Transform(next),
                StyleOp::Transform3d(None),
                StyleOp::UnsupportedTransform(None),
            ])
        }
        Some(ParsedTransform::Mat4(mat)) => Ok(vec![
            StyleOp::Transform(None),
            StyleOp::Transform3d(Some(mat)),
            StyleOp::UnsupportedTransform(None),
        ]),
        None => {
            // Preserve fail-closed unsupported_transform behavior on LayoutStyle
            // when a transform list contains an unknown/unrepresentable function.
            let raw = serialize_component_values(trimmed).trim().to_string();
            Ok(vec![
                StyleOp::Transform(None),
                StyleOp::Transform3d(None),
                StyleOp::UnsupportedTransform(Some(raw)),
            ])
        }
    }
}

/// Parse a CSS `transform` function list directly from `&[ComponentValue]`.
pub fn parse_transform_values(
    values: &[ComponentValue],
    ctx: &CompileContext,
) -> Option<ParsedTransform> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return None;
    }
    let mut result = PaintMat4::IDENTITY;
    let mut used_3d = false;
    let mut saw_any = false;

    for cv in trimmed {
        if cv.is_whitespace() {
            continue;
        }
        let ComponentValue::Function { name, args, .. } = cv else {
            return None;
        };
        saw_any = true;
        let lower = name.to_ascii_lowercase();
        used_3d |= matches!(
            lower.as_str(),
            "rotatex"
                | "rotatey"
                | "rotate3d"
                | "perspective"
                | "matrix3d"
                | "translatez"
                | "translate3d"
                | "scale3d"
                | "scalez"
        );
        let arg_groups = split_comma_groups(args);
        let next = match lower.as_str() {
            "translate" => match arg_groups.as_slice() {
                [x] => PaintMat4::translation(parse_tx_len(x, ctx)?, 0.0, 0.0),
                [x, y] => PaintMat4::translation(parse_tx_len(x, ctx)?, parse_tx_len(y, ctx)?, 0.0),
                _ => return None,
            },
            "translatex" => {
                let [x] = arg_groups.as_slice() else {
                    return None;
                };
                PaintMat4::translation(parse_tx_len(x, ctx)?, 0.0, 0.0)
            }
            "translatey" => {
                let [y] = arg_groups.as_slice() else {
                    return None;
                };
                PaintMat4::translation(0.0, parse_tx_len(y, ctx)?, 0.0)
            }
            "translatez" => {
                let [z] = arg_groups.as_slice() else {
                    return None;
                };
                PaintMat4::translation(0.0, 0.0, parse_tx_len(z, ctx)?)
            }
            "translate3d" => {
                let [x, y, z] = arg_groups.as_slice() else {
                    return None;
                };
                PaintMat4::translation(
                    parse_tx_len(x, ctx)?,
                    parse_tx_len(y, ctx)?,
                    parse_tx_len(z, ctx)?,
                )
            }
            "scale" => match arg_groups.as_slice() {
                [x] => {
                    let s = parse_scale_factor(x)?;
                    PaintMat4::scaling(s, s, 1.0)
                }
                [x, y] => PaintMat4::scaling(parse_scale_factor(x)?, parse_scale_factor(y)?, 1.0),
                _ => return None,
            },
            "scalex" => {
                let [x] = arg_groups.as_slice() else {
                    return None;
                };
                PaintMat4::scaling(parse_scale_factor(x)?, 1.0, 1.0)
            }
            "scaley" => {
                let [y] = arg_groups.as_slice() else {
                    return None;
                };
                PaintMat4::scaling(1.0, parse_scale_factor(y)?, 1.0)
            }
            "scalez" => {
                let [z] = arg_groups.as_slice() else {
                    return None;
                };
                PaintMat4::scaling(1.0, 1.0, parse_scale_factor(z)?)
            }
            "scale3d" => {
                let [x, y, z] = arg_groups.as_slice() else {
                    return None;
                };
                PaintMat4::scaling(
                    parse_scale_factor(x)?,
                    parse_scale_factor(y)?,
                    parse_scale_factor(z)?,
                )
            }
            "rotate" | "rotatez" => {
                let [a] = arg_groups.as_slice() else {
                    return None;
                };
                let rad = parse_angle_deg_or_zero(a)?.to_radians();
                PaintMat4::rotate_z(rad)
            }
            "rotatex" => {
                let [a] = arg_groups.as_slice() else {
                    return None;
                };
                PaintMat4::rotate_x(parse_angle_deg_or_zero(a)?.to_radians())
            }
            "rotatey" => {
                let [a] = arg_groups.as_slice() else {
                    return None;
                };
                PaintMat4::rotate_y(parse_angle_deg_or_zero(a)?.to_radians())
            }
            "rotate3d" => {
                let [x, y, z, a] = arg_groups.as_slice() else {
                    return None;
                };
                let vx = parse_number(x)?;
                let vy = parse_number(y)?;
                let vz = parse_number(z)?;
                let rad = parse_angle_deg_or_zero(a)?.to_radians();
                PaintMat4::rotate3d(vx, vy, vz, rad)?
            }
            "perspective" => {
                let [d] = arg_groups.as_slice() else {
                    return None;
                };
                let px = parse_tx_len(d, ctx)?;
                PaintMat4::perspective(px)?
            }
            "skew" => match arg_groups.as_slice() {
                [x] => {
                    let tx = parse_angle_deg_or_zero(x)?.to_radians().tan();
                    PaintMat4::from_affine(PaintTransform {
                        c: tx,
                        ..PaintTransform::default()
                    })
                }
                [x, y] => {
                    let tx = parse_angle_deg_or_zero(x)?.to_radians().tan();
                    let ty = parse_angle_deg_or_zero(y)?.to_radians().tan();
                    PaintMat4::from_affine(PaintTransform {
                        b: ty,
                        c: tx,
                        ..PaintTransform::default()
                    })
                }
                _ => return None,
            },
            "skewx" => {
                let [x] = arg_groups.as_slice() else {
                    return None;
                };
                let tx = parse_angle_deg_or_zero(x)?.to_radians().tan();
                PaintMat4::from_affine(PaintTransform {
                    c: tx,
                    ..PaintTransform::default()
                })
            }
            "skewy" => {
                let [y] = arg_groups.as_slice() else {
                    return None;
                };
                let ty = parse_angle_deg_or_zero(y)?.to_radians().tan();
                PaintMat4::from_affine(PaintTransform {
                    b: ty,
                    ..PaintTransform::default()
                })
            }
            "matrix" => {
                if arg_groups.len() != 6 {
                    return None;
                }
                let mut v = [0.0f32; 6];
                for (slot, g) in v.iter_mut().zip(&arg_groups) {
                    *slot = parse_number(g)?;
                }
                PaintMat4::from_affine(PaintTransform {
                    a: v[0],
                    b: v[1],
                    c: v[2],
                    d: v[3],
                    e: v[4],
                    f: v[5],
                })
            }
            "matrix3d" => {
                if arg_groups.len() != 16 {
                    return None;
                }
                let mut m = [0.0f32; 16];
                for (slot, g) in m.iter_mut().zip(&arg_groups) {
                    *slot = parse_number(g)?;
                }
                PaintMat4::from_matrix3d(m)?
            }
            _ => return None,
        };
        result = result.then(next);
    }

    if !saw_any || !result.m.iter().all(|v| v.is_finite()) {
        return None;
    }
    if !used_3d {
        let affine = result.as_affine()?;
        return [affine.a, affine.b, affine.c, affine.d, affine.e, affine.f]
            .into_iter()
            .all(f32::is_finite)
            .then_some(ParsedTransform::Affine(affine));
    }
    Some(ParsedTransform::Mat4(result))
}

fn parse_tx_len(values: &[ComponentValue], ctx: &CompileContext) -> Option<f32> {
    let spec = parse_length_spec(values)?;
    ctx.resolve_length_px(spec)
}

fn parse_number(values: &[ComponentValue]) -> Option<f32> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    match cv.as_token()? {
        Token::Number { value, .. } if value.is_finite() => Some(*value as f32),
        _ => None,
    }
}

fn parse_scale_factor(values: &[ComponentValue]) -> Option<f32> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    match cv.as_token()? {
        Token::Number { value, .. } if value.is_finite() => Some(*value as f32),
        Token::Percentage(p) if p.is_finite() => Some(*p as f32 / 100.0),
        _ => None,
    }
}

fn compile_transform_origin(
    values: &[ComponentValue],
    span: Span,
    allow_z: bool,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && cv.as_ident().is_some_and(|id| {
            id.eq_ignore_ascii_case("initial") || id.eq_ignore_ascii_case("unset")
        })
    {
        return Ok(vec![if allow_z {
            StyleOp::TransformOrigin(None)
        } else {
            StyleOp::PerspectiveOrigin(None)
        }]);
    }
    let origin = parse_origin_values(trimmed, allow_z).ok_or_else(|| {
        CssDiag::error(
            span,
            DiagCode::InvalidValue,
            if allow_z {
                "invalid `transform-origin`"
            } else {
                "invalid `perspective-origin`"
            },
        )
    })?;
    Ok(vec![if allow_z {
        StyleOp::TransformOrigin(Some(origin))
    } else {
        StyleOp::PerspectiveOrigin(Some(origin))
    }])
}

fn parse_origin_values(values: &[ComponentValue], allow_z: bool) -> Option<TransformOrigin> {
    let items = split_whitespace_items(values);
    match items.as_slice() {
        [one] => {
            let pos = crate::syntax::values::parse_position(one)?;
            Some(TransformOrigin { x: pos.x, y: pos.y })
        }
        [a, b] => {
            let mut joined = Vec::with_capacity(a.len() + 1 + b.len());
            joined.extend_from_slice(a);
            joined.push(ComponentValue::Token(crate::syntax::SpannedToken::new(
                Token::Whitespace,
                Span::EMPTY,
            )));
            joined.extend_from_slice(b);
            let pos = crate::syntax::values::parse_position(&joined)?;
            Some(TransformOrigin { x: pos.x, y: pos.y })
        }
        [a, b, z] if allow_z => {
            // Optional Z offset must be a length (never a percentage per CSS Transforms 2).
            let z_spec = parse_length_spec(z)?;
            if matches!(z_spec, LengthSpec::Percent(_)) {
                return None;
            }
            let mut joined = Vec::with_capacity(a.len() + 1 + b.len());
            joined.extend_from_slice(a);
            joined.push(ComponentValue::Token(crate::syntax::SpannedToken::new(
                Token::Whitespace,
                Span::EMPTY,
            )));
            joined.extend_from_slice(b);
            let pos = crate::syntax::values::parse_position(&joined)?;
            Some(TransformOrigin { x: pos.x, y: pos.y })
        }
        _ => None,
    }
}

fn compile_transform_box(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `transform-box`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `transform-box`"))?;
    let tb = match kw.to_ascii_lowercase().as_str() {
        "border-box" => TransformBox::BorderBox,
        "fill-box" => TransformBox::FillBox,
        "view-box" | "initial" | "unset" => TransformBox::ViewBox,
        "content-box" => TransformBox::ContentBox,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("unsupported `transform-box: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::TransformBox(tb)])
}

fn compile_perspective(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if let [cv] = trimmed
        && cv.as_ident().is_some_and(|id| {
            id.eq_ignore_ascii_case("none")
                || id.eq_ignore_ascii_case("initial")
                || id.eq_ignore_ascii_case("unset")
        })
    {
        return Ok(vec![StyleOp::CssPerspective(None)]);
    }
    let px = parse_tx_len(trimmed, ctx)
        .filter(|d| *d > 1e-5)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `perspective`"))?;
    Ok(vec![StyleOp::CssPerspective(Some(px))])
}

fn compile_transform_style(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `transform-style`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `transform-style`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "preserve-3d" => Ok(vec![StyleOp::Preserve3d(true)]),
        "flat" | "initial" | "unset" => Ok(vec![StyleOp::Preserve3d(false)]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("invalid `transform-style: {kw}`"),
        )),
    }
}

fn compile_backface_visibility(
    values: &[ComponentValue],
    span: Span,
) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `backface-visibility`",
        ));
    };
    let kw = cv.as_ident().ok_or_else(|| {
        CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `backface-visibility`",
        )
    })?;
    let bv = match kw.to_ascii_lowercase().as_str() {
        "visible" | "initial" | "unset" => BackfaceVisibility::Visible,
        "hidden" => BackfaceVisibility::Hidden,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `backface-visibility: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::BackfaceVisibility(bv)])
}
