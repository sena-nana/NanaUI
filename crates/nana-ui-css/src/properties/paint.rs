//! Paint, background, gradient, border, outline, box-shadow, mask, clip-path,
//! filter, backdrop-filter, and blend-mode CSS properties parsed directly from
//! `&[ComponentValue]` to `Result<Vec<StyleOp>, CssDiag>`.
//!
//! Confirmed fixes vs legacy `css_paint.rs`:
//! - `normalize_gradient_stops` preserves explicit first/last positions (`red 20%, blue 80%`)
//!   and NEVER divides `px` stops by hardcoded `100.0`!
//! - `background-position` / `object-position` use `syntax::values::parse_position`,
//!   handling `right 10px`, `center 20px`, 3/4-value edge offsets cleanly!
//! - `border-radius` splits via `ComponentValue` items so `calc(...)` works cleanly!
//! - `parse_color(&[])` / empty strings are rejected cleanly!

use nana_ui_core::{
    BackdropFilter, BackgroundImage, BackgroundImageFit, BackgroundRepeat, BorderImageSlice,
    BorderImageSpec, BorderStyle, BoxShadowSpec, ClipCircle, ClipEllipse, ClipInset, ClipPath,
    ClipPoint, ClipShapeRadius, ColorFilter, CssGradient, FilterDropShadow, GradientStop,
    LengthSpec, LinearGradient, MAX_BACKGROUND_LAYERS, MaskImage, MixBlendMode, OutlineStyle,
    PaintColor, RadialGradient, StyleOp,
};

use crate::properties::theme::clear_theme_ops_for_property;
use crate::properties::{CompileContext, DeclState};
use crate::syntax::diag::{CssDiag, DiagCode};
use crate::syntax::parser::{ComponentValue, split_comma_groups, split_whitespace_items, trim_ws};
use crate::syntax::span::Span;
use crate::syntax::tokenizer::{SpannedToken, Token};
use crate::syntax::values::{
    ColorInterpolationMethod, ColorParseOptions, HueInterpolationMethod, ParsedColor,
    interpolate_paint_colors, normalize_gradient_stops, parse_angle_deg, parse_color,
    parse_gradient_stop, parse_length_spec, parse_position,
};

const MAX_GRADIENT_STOPS: usize = 8;

pub(crate) fn parse_paint_property(
    prop: &str,
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
) -> Option<Result<Vec<StyleOp>, CssDiag>> {
    match prop {
        "background" => Some(compile_background_shorthand(values, span, ctx, state)),
        "background-color" | "fill" => Some(compile_background_color(values, span, ctx)),
        "background-image" => Some(compile_background_image(values, span, ctx, state)),
        "background-size" => Some(compile_background_size(values, span, state)),
        "background-position" => Some(compile_background_position(values, span, state)),
        "background-repeat" => Some(compile_background_repeat(values, span, state)),
        "object-fit" => Some(compile_object_fit(values, span)),
        "object-position" => Some(compile_object_position(values, span)),
        "border" => Some(compile_border_shorthand(values, span, ctx, None)),
        "border-top" => Some(compile_border_shorthand(values, span, ctx, Some(0))),
        "border-right" => Some(compile_border_shorthand(values, span, ctx, Some(1))),
        "border-bottom" => Some(compile_border_shorthand(values, span, ctx, Some(2))),
        "border-left" => Some(compile_border_shorthand(values, span, ctx, Some(3))),
        "border-width" => Some(compile_border_width_shorthand(values, span, ctx)),
        "border-top-width" => Some(compile_border_side_width(values, span, ctx, 0)),
        "border-right-width" => Some(compile_border_side_width(values, span, ctx, 1)),
        "border-bottom-width" => Some(compile_border_side_width(values, span, ctx, 2)),
        "border-left-width" => Some(compile_border_side_width(values, span, ctx, 3)),
        "border-color" => Some(compile_border_color_shorthand(values, span, ctx)),
        "border-top-color" => Some(compile_border_side_color(values, span, ctx, 0)),
        "border-right-color" => Some(compile_border_side_color(values, span, ctx, 1)),
        "border-bottom-color" => Some(compile_border_side_color(values, span, ctx, 2)),
        "border-left-color" => Some(compile_border_side_color(values, span, ctx, 3)),
        "border-style" => Some(compile_border_style_shorthand(values, span)),
        "border-top-style" => Some(compile_border_side_style(values, span, 0)),
        "border-right-style" => Some(compile_border_side_style(values, span, 1)),
        "border-bottom-style" => Some(compile_border_side_style(values, span, 2)),
        "border-left-style" => Some(compile_border_side_style(values, span, 3)),
        "border-radius" => Some(compile_border_radius(values, span, state)),
        "border-top-left-radius" => Some(compile_border_corner_radius(values, span, 0, state)),
        "border-top-right-radius" => Some(compile_border_corner_radius(values, span, 1, state)),
        "border-bottom-right-radius" => Some(compile_border_corner_radius(values, span, 2, state)),
        "border-bottom-left-radius" => Some(compile_border_corner_radius(values, span, 3, state)),
        "border-image" => Some(compile_border_image_shorthand(values, span, ctx, state)),
        "border-image-source" => Some(compile_border_image_source(values, span, ctx, state)),
        "border-image-slice" => Some(compile_border_image_slice(values, span, state)),
        "border-image-width" | "border-image-outset" | "border-image-repeat" => {
            Some(compile_border_image_extra(prop, values, span, state))
        }
        "stroke" => Some(compile_svg_stroke(values, span, ctx)),
        "stroke-width" => Some(compile_svg_stroke_width(values, span, ctx)),
        "outline" => Some(compile_outline_shorthand(values, span, ctx)),
        "outline-width" => Some(compile_outline_width(values, span, ctx)),
        "outline-style" => Some(compile_outline_style(values, span)),
        "outline-color" => Some(compile_outline_color(values, span, ctx)),
        "box-shadow" => Some(compile_box_shadow(values, span, ctx)),
        "mask-image" | "-webkit-mask-image" => Some(compile_mask_image(values, span, ctx)),
        "clip-path" => Some(compile_clip_path(values, span)),
        "filter" => Some(compile_filter(values, span, ctx)),
        "backdrop-filter" | "-webkit-backdrop-filter" => {
            Some(compile_backdrop_filter(values, span, ctx))
        }
        "mix-blend-mode" => Some(compile_mix_blend_mode(values, span)),
        _ => None,
    }
}

// ============================================================================
// Background & Object Fit/Position
// ============================================================================

fn compile_background_color(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `background-color`",
        ));
    }
    if let [cv] = trimmed
        && cv.as_ident().is_some_and(|s| {
            s.eq_ignore_ascii_case("none") || s.eq_ignore_ascii_case("transparent")
        })
    {
        return Ok(vec![StyleOp::Background(None)]);
    }
    let pc = parse_color(trimmed, ctx.color_opts)
        .and_then(ParsedColor::into_paint)
        .ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `background-color`")
        })?;
    Ok(vec![StyleOp::Background(Some(pc))])
}

fn compile_background_shorthand(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `background`",
        ));
    }
    if let [cv] = trimmed
        && cv.as_ident().is_some_and(|s| {
            s.eq_ignore_ascii_case("none") || s.eq_ignore_ascii_case("transparent")
        })
    {
        state.bg_layers.clear();
        state.bg_size_list.clear();
        state.bg_size_lengths.clear();
        state.bg_position_list.clear();
        state.bg_repeat_list.clear();
        return Ok(vec![
            StyleOp::Background(None),
            StyleOp::BackgroundLayers(Vec::new()),
            StyleOp::BackgroundSizeList(Vec::new()),
            StyleOp::BackgroundSizeLengths(Vec::new()),
            StyleOp::BackgroundPositionList(Vec::new()),
            StyleOp::BackgroundRepeatList(Vec::new()),
        ]);
    }

    // Fast path: single color value (`background: #fff`, `background: rgba(...)`, `background: var(--nana-surface)`).
    if let Some(pc) = parse_color(trimmed, ctx.color_opts).and_then(ParsedColor::into_paint) {
        state.bg_layers.clear();
        state.bg_size_list.clear();
        state.bg_size_lengths.clear();
        state.bg_position_list.clear();
        state.bg_repeat_list.clear();
        return Ok(vec![
            StyleOp::Background(Some(pc)),
            StyleOp::BackgroundLayers(Vec::new()),
            StyleOp::BackgroundSizeList(Vec::new()),
            StyleOp::BackgroundSizeLengths(Vec::new()),
            StyleOp::BackgroundPositionList(Vec::new()),
            StyleOp::BackgroundRepeatList(Vec::new()),
        ]);
    }

    let groups = split_comma_groups(trimmed);
    let mut color: Option<PaintColor> = None;
    let mut layers: Vec<BackgroundImage> = Vec::new();
    for (idx, group) in groups.iter().enumerate() {
        let is_last = idx + 1 == groups.len();
        let (layer_img, layer_color) =
            parse_background_layer(group, is_last, ctx).ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid `background` layer")
            })?;
        if let Some(c) = layer_color {
            color = Some(c);
        }
        if let Some(img) = layer_img
            && layers.len() < MAX_BACKGROUND_LAYERS
        {
            layers.push(img);
        }
    }

    state.bg_layers = layers.clone();
    state.bg_size_list.clear();
    state.bg_size_lengths.clear();
    state.bg_position_list.clear();
    state.bg_repeat_list.clear();

    let mut ops = vec![
        StyleOp::BackgroundSizeList(Vec::new()),
        StyleOp::BackgroundSizeLengths(Vec::new()),
        StyleOp::BackgroundPositionList(Vec::new()),
        StyleOp::BackgroundRepeatList(Vec::new()),
        StyleOp::BackgroundLayers(layers),
    ];
    if let Some(pc) = color {
        ops.push(StyleOp::Background(Some(pc)));
    }
    Ok(ops)
}

fn parse_background_layer(
    group: &[ComponentValue],
    allow_color: bool,
    ctx: &CompileContext,
) -> Option<(Option<BackgroundImage>, Option<PaintColor>)> {
    let items = split_whitespace_items(group);
    if items.is_empty() {
        return None;
    }

    let mut image: Option<BackgroundImage> = None;
    let mut color: Option<PaintColor> = None;
    let mut repeat_toks: Vec<String> = Vec::new();
    let mut pos_cvs: Vec<ComponentValue> = Vec::new();
    let mut size_cvs: Vec<ComponentValue> = Vec::new();
    let mut after_slash = false;
    let mut saw_any = false;

    for item in items {
        if matches!(item, [cv] if matches!(cv.as_token(), Some(Token::Delim('/')))) {
            if after_slash {
                return None;
            }
            after_slash = true;
            continue;
        }
        if !after_slash
            && image.is_none()
            && let Some(img) = parse_background_image_item(item, ctx)
        {
            image = Some(img);
            saw_any = true;
            continue;
        }
        if !after_slash
            && allow_color
            && color.is_none()
            && let Some(pc) = parse_color(item, ctx.color_opts).and_then(ParsedColor::into_paint)
        {
            color = Some(pc);
            saw_any = true;
            continue;
        }
        if let [cv] = item
            && let Some(id) = cv.as_ident()
            && is_repeat_kw(id)
        {
            after_slash = false;
            repeat_toks.push(id.to_ascii_lowercase());
            saw_any = true;
            continue;
        }
        if after_slash {
            if !size_cvs.is_empty() {
                size_cvs.push(ws_cv());
            }
            size_cvs.extend_from_slice(item);
            saw_any = true;
        } else {
            if !pos_cvs.is_empty() {
                pos_cvs.push(ws_cv());
            }
            pos_cvs.extend_from_slice(item);
            saw_any = true;
        }
    }

    if !saw_any {
        return None;
    }

    if let Some(BackgroundImage::Url {
        fit,
        size_width,
        size_height,
        position,
        repeat,
        ..
    }) = image.as_mut()
    {
        if !repeat_toks.is_empty()
            && let Some(r) = parse_repeat_keywords(&repeat_toks)
        {
            *repeat = r;
        }
        if !pos_cvs.is_empty()
            && let Some(p) = parse_position(&pos_cvs)
        {
            *position = p;
        }
        if !size_cvs.is_empty()
            && let Some((f, w, h)) = parse_bg_size_item(&size_cvs)
        {
            *fit = f;
            *size_width = w;
            *size_height = h;
        }
    }

    Some((image, color))
}

fn compile_background_image(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `background-image`",
        ));
    }
    if let [cv] = trimmed
        && cv
            .as_ident()
            .is_some_and(|s| s.eq_ignore_ascii_case("none"))
    {
        state.bg_layers.clear();
        return Ok(vec![StyleOp::BackgroundLayers(Vec::new())]);
    }
    let groups = split_comma_groups(trimmed);
    let mut layers = Vec::new();
    for g in groups {
        let img = parse_background_image_item(g, ctx).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `background-image`")
        })?;
        layers.push(img);
        if layers.len() >= MAX_BACKGROUND_LAYERS {
            break;
        }
    }
    zip_background_longhands(&mut layers, state);
    state.bg_layers = layers.clone();
    Ok(vec![StyleOp::BackgroundLayers(layers)])
}

fn parse_background_image_item(
    values: &[ComponentValue],
    ctx: &CompileContext,
) -> Option<BackgroundImage> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    if let Some(url) = cv.as_url()
        && !url.is_empty()
    {
        return Some(BackgroundImage::url(url));
    }
    if let ComponentValue::Function { name, args, .. } = cv {
        match name.to_ascii_lowercase().as_str() {
            "linear-gradient" => parse_linear_gradient(args, false, ctx.color_opts)
                .map(|g| BackgroundImage::Gradient(CssGradient::Linear(g))),
            "repeating-linear-gradient" => parse_linear_gradient(args, true, ctx.color_opts)
                .map(|g| BackgroundImage::Gradient(CssGradient::Linear(g))),
            "radial-gradient" => parse_radial_gradient(args, ctx.color_opts)
                .map(|g| BackgroundImage::Gradient(CssGradient::Radial(g))),
            _ => None,
        }
    } else {
        None
    }
}

fn compile_background_size(
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let groups = split_comma_groups(values);
    if groups.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `background-size`",
        ));
    }
    let mut fits = Vec::with_capacity(groups.len());
    let mut lengths = Vec::with_capacity(groups.len());
    for g in groups {
        let (f, w, h) = parse_bg_size_item(g).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `background-size`")
        })?;
        fits.push(f);
        lengths.push((w, h));
    }
    state.bg_size_list = fits.clone();
    state.bg_size_lengths = lengths.clone();
    let mut ops = vec![
        StyleOp::BackgroundSizeList(fits),
        StyleOp::BackgroundSizeLengths(lengths),
    ];
    if !state.bg_layers.is_empty() {
        let mut layers = state.bg_layers.clone();
        zip_background_longhands(&mut layers, state);
        state.bg_layers = layers.clone();
        ops.push(StyleOp::BackgroundLayers(layers));
    }
    Ok(ops)
}

fn parse_bg_size_item(
    values: &[ComponentValue],
) -> Option<(BackgroundImageFit, Option<LengthSpec>, Option<LengthSpec>)> {
    let items = split_whitespace_items(values);
    match items.as_slice() {
        [one] => {
            if let [cv] = *one
                && let Some(kw) = cv.as_ident()
            {
                return match kw.to_ascii_lowercase().as_str() {
                    "contain" => Some((BackgroundImageFit::Contain, None, None)),
                    "cover" => Some((BackgroundImageFit::Cover, None, None)),
                    "auto" => Some((BackgroundImageFit::Auto, None, None)),
                    _ => None,
                };
            }
            if let [cv] = *one
                && let Some(Token::Percentage(p)) = cv.as_token()
                && (*p - 100.0).abs() < 1e-6
            {
                return Some((BackgroundImageFit::Stretch, None, None));
            }
            let w = parse_length_spec(one)?;
            Some((BackgroundImageFit::Length, Some(w), None))
        }
        [a, b] => {
            let a_auto =
                matches!(*a, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("auto")));
            let b_auto =
                matches!(*b, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("auto")));
            if a_auto && b_auto {
                return Some((BackgroundImageFit::Auto, None, None));
            }
            let a_100 = matches!(*a, [cv] if matches!(cv.as_token(), Some(Token::Percentage(p)) if (*p - 100.0).abs() < 1e-6));
            let b_100 = matches!(*b, [cv] if matches!(cv.as_token(), Some(Token::Percentage(p)) if (*p - 100.0).abs() < 1e-6));
            if a_100 && b_100 {
                return Some((BackgroundImageFit::Stretch, None, None));
            }
            let w = if a_auto {
                LengthSpec::Auto
            } else {
                parse_length_spec(a)?
            };
            let h = if b_auto {
                LengthSpec::Auto
            } else {
                parse_length_spec(b)?
            };
            Some((BackgroundImageFit::Length, Some(w), Some(h)))
        }
        _ => None,
    }
}

fn compile_background_position(
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let groups = split_comma_groups(values);
    if groups.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `background-position`",
        ));
    }
    let mut positions = Vec::with_capacity(groups.len());
    for g in groups {
        let p = parse_position(g).ok_or_else(|| {
            CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `background-position`",
            )
        })?;
        positions.push(p);
    }
    state.bg_position_list = positions.clone();
    let mut ops = vec![StyleOp::BackgroundPositionList(positions)];
    if !state.bg_layers.is_empty() {
        let mut layers = state.bg_layers.clone();
        zip_background_longhands(&mut layers, state);
        state.bg_layers = layers.clone();
        ops.push(StyleOp::BackgroundLayers(layers));
    }
    Ok(ops)
}

fn compile_background_repeat(
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let groups = split_comma_groups(values);
    if groups.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `background-repeat`",
        ));
    }
    let mut repeats = Vec::with_capacity(groups.len());
    for g in groups {
        let items = split_whitespace_items(g);
        let mut kws = Vec::with_capacity(items.len());
        for it in items {
            let [cv] = it else {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    "invalid `background-repeat`",
                ));
            };
            let Some(id) = cv.as_ident() else {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    "invalid `background-repeat`",
                ));
            };
            kws.push(id.to_ascii_lowercase());
        }
        let r = parse_repeat_keywords(&kws).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `background-repeat`")
        })?;
        repeats.push(r);
    }
    state.bg_repeat_list = repeats.clone();
    let mut ops = vec![StyleOp::BackgroundRepeatList(repeats)];
    if !state.bg_layers.is_empty() {
        let mut layers = state.bg_layers.clone();
        zip_background_longhands(&mut layers, state);
        state.bg_layers = layers.clone();
        ops.push(StyleOp::BackgroundLayers(layers));
    }
    Ok(ops)
}

fn is_repeat_kw(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "repeat" | "no-repeat" | "repeat-x" | "repeat-y" | "round" | "space"
    )
}

fn parse_repeat_keywords(kws: &[String]) -> Option<BackgroundRepeat> {
    if kws.iter().any(|k| k == "space") {
        return Some(BackgroundRepeat::Unsupported);
    }
    match kws
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["repeat-x"] => Some(BackgroundRepeat::RepeatX),
        ["repeat-y"] => Some(BackgroundRepeat::RepeatY),
        ["no-repeat"] | ["no-repeat", "no-repeat"] => Some(BackgroundRepeat::NoRepeat),
        ["repeat"] | ["repeat", "repeat"] => Some(BackgroundRepeat::Repeat),
        ["round"] | ["round", "round"] => Some(BackgroundRepeat::Round),
        ["round", "no-repeat"] => Some(BackgroundRepeat::RoundX),
        ["no-repeat", "round"] => Some(BackgroundRepeat::RoundY),
        ["round", "repeat"] | ["repeat", "round"] => Some(BackgroundRepeat::Unsupported),
        ["repeat", "no-repeat"] => Some(BackgroundRepeat::RepeatX),
        ["no-repeat", "repeat"] => Some(BackgroundRepeat::RepeatY),
        _ => None,
    }
}

fn zip_background_longhands(layers: &mut [BackgroundImage], state: &DeclState) {
    for (idx, layer) in layers.iter_mut().enumerate() {
        let BackgroundImage::Url {
            fit,
            size_width,
            size_height,
            position,
            repeat,
            ..
        } = layer
        else {
            continue;
        };
        if !state.bg_size_list.is_empty() {
            *fit = state.bg_size_list[idx % state.bg_size_list.len()];
        }
        if !state.bg_size_lengths.is_empty() {
            let (w, h) = state.bg_size_lengths[idx % state.bg_size_lengths.len()];
            *size_width = w;
            *size_height = h;
        }
        if !state.bg_position_list.is_empty() {
            *position = state.bg_position_list[idx % state.bg_position_list.len()];
        }
        if !state.bg_repeat_list.is_empty() {
            *repeat = state.bg_repeat_list[idx % state.bg_repeat_list.len()];
        }
    }
}

fn compile_object_fit(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `object-fit`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `object-fit`"))?;
    let fit = match kw.to_ascii_lowercase().as_str() {
        "fill" => BackgroundImageFit::Stretch,
        "contain" => BackgroundImageFit::Contain,
        "cover" => BackgroundImageFit::Cover,
        "none" => BackgroundImageFit::Auto,
        "scale-down" => BackgroundImageFit::ScaleDown,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("invalid `object-fit: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::ObjectFit(Some(fit))])
}

fn compile_object_position(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let pos = parse_position(values)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `object-position`"))?;
    Ok(vec![StyleOp::ObjectPosition(Some(pos))])
}

// ============================================================================
// Gradients (using syntax::values::normalize_gradient_stops!)
// ============================================================================

/// Parse `linear-gradient(...)` / `repeating-linear-gradient(...)` args.
pub fn parse_linear_gradient(
    args: &[ComponentValue],
    repeating: bool,
    opts: ColorParseOptions,
) -> Option<LinearGradient> {
    let groups = split_comma_groups(args);
    if groups.len() < 2 {
        return None;
    }
    let (angle_deg, interp, stop_groups) =
        if let Some((angle, interp)) = parse_linear_header(groups[0]) {
            (angle, interp, &groups[1..])
        } else {
            (180.0, None, &groups[..])
        };

    let mut parsed_stops = Vec::with_capacity(stop_groups.len());
    for g in stop_groups {
        parsed_stops.push(parse_gradient_stop(g, opts)?);
    }

    let mut stops = normalize_gradient_stops(&parsed_stops, None, |pc| {
        pc.into_paint()
            .unwrap_or(PaintColor::srgb([0.0, 0.0, 0.0, 1.0]))
    })?;

    if repeating {
        stops = expand_repeating_stops(stops)?;
    } else {
        stops.truncate(MAX_GRADIENT_STOPS);
    }

    if let Some(method) = interp {
        stops = apply_gradient_interpolation(stops, method);
    }
    (stops.len() >= 2).then_some(LinearGradient { angle_deg, stops })
}

fn parse_linear_header(head: &[ComponentValue]) -> Option<(f32, Option<ColorInterpolationMethod>)> {
    let items = split_whitespace_items(head);
    if items.is_empty() {
        return None;
    }
    let (rest, interp) = strip_color_interp_clause(&items)?;
    if rest.is_empty() {
        return interp.map(|m| (180.0, Some(m)));
    }
    if rest.len() == 1
        && let Some(deg) = parse_angle_deg(rest[0])
    {
        return Some((deg, interp));
    }
    // `to <side-or-corner>`
    if let [first, tail @ ..] = rest.as_slice()
        && matches!(*first, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("to")))
    {
        let mut dirs = Vec::with_capacity(tail.len());
        for t in tail {
            let [cv] = *t else { return None };
            dirs.push(cv.as_ident()?.to_ascii_lowercase());
        }
        let angle = match dirs
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["top"] => 0.0,
            ["right"] => 90.0,
            ["bottom"] => 180.0,
            ["left"] => 270.0,
            ["top", "right"] | ["right", "top"] => 45.0,
            ["bottom", "right"] | ["right", "bottom"] => 135.0,
            ["bottom", "left"] | ["left", "bottom"] => 225.0,
            ["top", "left"] | ["left", "top"] => 315.0,
            _ => return None,
        };
        return Some((angle, interp));
    }
    None
}

/// Parse `radial-gradient(...)` args.
pub fn parse_radial_gradient(
    args: &[ComponentValue],
    opts: ColorParseOptions,
) -> Option<RadialGradient> {
    let groups = split_comma_groups(args);
    if groups.len() < 2 {
        return None;
    }
    let (circle, center, interp, stop_groups) =
        if let Some((c, ctr, interp)) = parse_radial_header(groups[0]) {
            (c, ctr, interp, &groups[1..])
        } else {
            (
                true,
                [LengthSpec::Percent(50.0), LengthSpec::Percent(50.0)],
                None,
                &groups[..],
            )
        };

    let mut parsed_stops = Vec::with_capacity(stop_groups.len());
    for g in stop_groups {
        parsed_stops.push(parse_gradient_stop(g, opts)?);
    }

    let mut stops = normalize_gradient_stops(&parsed_stops, None, |pc| {
        pc.into_paint()
            .unwrap_or(PaintColor::srgb([0.0, 0.0, 0.0, 1.0]))
    })?;
    stops.truncate(MAX_GRADIENT_STOPS);
    if let Some(method) = interp {
        stops = apply_gradient_interpolation(stops, method);
    }
    (stops.len() >= 2).then_some(RadialGradient {
        circle,
        center,
        stops,
    })
}

fn parse_radial_header(
    head: &[ComponentValue],
) -> Option<(bool, [LengthSpec; 2], Option<ColorInterpolationMethod>)> {
    let items = split_whitespace_items(head);
    if items.is_empty() {
        return None;
    }
    let (rest, interp) = strip_color_interp_clause(&items)?;
    let mut circle = true;
    let mut saw_header_kw = interp.is_some();
    let mut at_idx: Option<usize> = None;

    for (i, item) in rest.iter().enumerate() {
        if let [cv] = *item
            && let Some(id) = cv.as_ident()
        {
            match id.to_ascii_lowercase().as_str() {
                "circle" => {
                    circle = true;
                    saw_header_kw = true;
                }
                "ellipse" => {
                    circle = false;
                    saw_header_kw = true;
                }
                "closest-side" | "farthest-side" | "closest-corner" | "farthest-corner" => {
                    saw_header_kw = true;
                }
                "at" => {
                    at_idx = Some(i);
                    saw_header_kw = true;
                    break;
                }
                _ => {}
            }
        }
    }

    if !saw_header_kw {
        return None;
    }

    let center = if let Some(idx) = at_idx {
        let mut pos_cvs = Vec::new();
        for it in &rest[idx + 1..] {
            if !pos_cvs.is_empty() {
                pos_cvs.push(ws_cv());
            }
            pos_cvs.extend_from_slice(it);
        }
        let pos = parse_position(&pos_cvs)?;
        [pos.x, pos.y]
    } else {
        [LengthSpec::Percent(50.0), LengthSpec::Percent(50.0)]
    };

    Some((circle, center, interp))
}

fn strip_color_interp_clause<'a>(
    items: &[&'a [ComponentValue]],
) -> Option<(Vec<&'a [ComponentValue]>, Option<ColorInterpolationMethod>)> {
    let Some(in_idx) = items.iter().position(
        |it| matches!(*it, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("in"))),
    ) else {
        return Some((items.to_vec(), None));
    };
    let space = match items.get(in_idx + 1)? {
        [cv] => cv.as_ident()?.to_ascii_lowercase(),
        _ => return None,
    };
    let (end, hue) = if items.get(in_idx + 3).is_some_and(
        |it| matches!(*it, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("hue"))),
    ) {
        let dir = match items[in_idx + 2] {
            [cv] => cv.as_ident()?.to_ascii_lowercase(),
            _ => return None,
        };
        let hm = match dir.as_str() {
            "shorter" => HueInterpolationMethod::Shorter,
            "longer" => HueInterpolationMethod::Longer,
            "increasing" => HueInterpolationMethod::Increasing,
            "decreasing" => HueInterpolationMethod::Decreasing,
            _ => return None,
        };
        (in_idx + 4, Some(hm))
    } else {
        (in_idx + 2, None)
    };
    let method = match space.as_str() {
        "srgb" if hue.is_none() => ColorInterpolationMethod::Srgb,
        "srgb-linear" if hue.is_none() => ColorInterpolationMethod::SrgbLinear,
        "oklch" => ColorInterpolationMethod::Oklch(hue.unwrap_or(HueInterpolationMethod::Shorter)),
        "hsv" => ColorInterpolationMethod::Hsv(hue.unwrap_or(HueInterpolationMethod::Shorter)),
        _ => return None,
    };
    let mut rest = Vec::with_capacity(items.len().saturating_sub(end - in_idx));
    rest.extend_from_slice(&items[..in_idx]);
    rest.extend_from_slice(&items[end..]);
    Some((rest, Some(method)))
}

fn expand_repeating_stops(stops: Vec<GradientStop>) -> Option<Vec<GradientStop>> {
    if stops.len() < 2 {
        return None;
    }
    let start = stops[0].position;
    let end = stops[stops.len() - 1].position;
    let period = end - start;
    if period <= 1e-5 {
        return None;
    }
    if start.abs() <= 1e-5 && (end - 1.0).abs() <= 1e-5 {
        let mut out = stops;
        out.truncate(MAX_GRADIENT_STOPS);
        return Some(out);
    }
    let n_min = ((0.0 - start) / period).ceil() as i32;
    let n_max = ((1.0 - start) / period).ceil() as i32;
    let mut out: Vec<GradientStop> = Vec::new();
    for n in n_min..=n_max {
        for stop in &stops {
            let pos = stop.position + n as f32 * period;
            if pos < -1e-4 || pos > 1.0 + 1e-4 {
                continue;
            }
            let pos = pos.clamp(0.0, 1.0);
            if let Some(last) = out.last()
                && (last.position - pos).abs() < 1e-5
                && last.color == stop.color
            {
                continue;
            }
            out.push(GradientStop {
                position: pos,
                color: stop.color,
            });
            if out.len() >= MAX_GRADIENT_STOPS {
                break;
            }
        }
        if out.len() >= MAX_GRADIENT_STOPS {
            break;
        }
    }
    if out.len() < 2 {
        return None;
    }
    Some(out)
}

fn apply_gradient_interpolation(
    stops: Vec<GradientStop>,
    method: ColorInterpolationMethod,
) -> Vec<GradientStop> {
    match method {
        ColorInterpolationMethod::Srgb => stops
            .into_iter()
            .map(|s| GradientStop {
                position: s.position,
                color: PaintColor::srgb(s.color.to_srgb()),
            })
            .collect(),
        ColorInterpolationMethod::SrgbLinear => stops
            .into_iter()
            .map(|s| {
                let (channels, alpha) = s.color.to_linear_sc_rgb();
                GradientStop {
                    position: s.position,
                    color: PaintColor::LinearScRgb { channels, alpha },
                }
            })
            .collect(),
        ColorInterpolationMethod::Oklch(_) | ColorInterpolationMethod::Hsv(_) => {
            if stops.len() < 2 {
                return stops;
            }
            if stops.len() >= MAX_GRADIENT_STOPS {
                return stops
                    .into_iter()
                    .map(|s| GradientStop {
                        position: s.position,
                        color: interpolate_paint_colors(method, s.color, s.color, 0.0, 1.0),
                    })
                    .collect();
            }
            let budget = MAX_GRADIENT_STOPS - stops.len();
            let seg_count = stops.len() - 1;
            let lengths: Vec<f32> = stops
                .windows(2)
                .map(|w| (w[1].position - w[0].position).max(0.0))
                .collect();
            let total_len: f32 = lengths.iter().sum();
            let mut extra = vec![0usize; seg_count];
            if total_len > 1.0e-5 {
                let mut assigned = 0usize;
                for (i, &len) in lengths.iter().enumerate() {
                    let share = ((len / total_len) * budget as f32).floor() as usize;
                    extra[i] = share;
                    assigned += share;
                }
                for (i, &len) in lengths.iter().enumerate() {
                    if assigned >= budget {
                        break;
                    }
                    if len > 1.0e-5 {
                        extra[i] += 1;
                        assigned += 1;
                    }
                }
            }
            let mut out = Vec::with_capacity(MAX_GRADIENT_STOPS);
            for (seg_idx, w) in stops.windows(2).enumerate() {
                out.push(GradientStop {
                    position: w[0].position,
                    color: interpolate_paint_colors(method, w[0].color, w[1].color, 0.0, 1.0),
                });
                let subdivisions = extra[seg_idx] + 1;
                for step in 1..subdivisions {
                    let u = step as f32 / subdivisions as f32;
                    let pos = w[0].position + (w[1].position - w[0].position) * u;
                    out.push(GradientStop {
                        position: pos,
                        color: interpolate_paint_colors(method, w[0].color, w[1].color, u, 1.0),
                    });
                }
            }
            if let Some(last) = stops.last() {
                out.push(GradientStop {
                    position: last.position,
                    color: interpolate_paint_colors(method, last.color, last.color, 1.0, 1.0),
                });
            }
            out.truncate(MAX_GRADIENT_STOPS);
            out
        }
    }
}

// ============================================================================
// Borders & Border Radius & Border Image
// ============================================================================

fn parse_border_width_val(values: &[ComponentValue], ctx: &CompileContext) -> Option<f32> {
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

fn parse_border_style_val(values: &[ComponentValue]) -> Option<BorderStyle> {
    let [cv] = trim_ws(values) else { return None };
    match cv.as_ident()?.to_ascii_lowercase().as_str() {
        "none" | "hidden" => Some(BorderStyle::None),
        "solid" => Some(BorderStyle::Solid),
        "dashed" => Some(BorderStyle::Dashed),
        "dotted" => Some(BorderStyle::Dotted),
        "double" | "groove" | "ridge" | "inset" | "outset" => Some(BorderStyle::Unsupported),
        _ => None,
    }
}

fn expand_four<T: Copy>(items: &[T]) -> Option<[T; 4]> {
    Some(match items {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return None,
    })
}

fn compile_border_width_shorthand(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    let mut parsed = Vec::with_capacity(items.len());
    for it in items {
        parsed.push(parse_border_width_val(it, ctx).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `border-width`")
        })?);
    }
    let [t, r, b, l] = expand_four(&parsed)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `border-width`"))?;
    let mut ops = clear_theme_ops_for_property("border-width");
    ops.extend([
        StyleOp::BorderWidth(Some(t.max(r).max(b).max(l))),
        StyleOp::BorderTopWidth(Some(t)),
        StyleOp::BorderRightWidth(Some(r)),
        StyleOp::BorderBottomWidth(Some(b)),
        StyleOp::BorderLeftWidth(Some(l)),
    ]);
    Ok(ops)
}

fn compile_border_side_width(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    side: usize,
) -> Result<Vec<StyleOp>, CssDiag> {
    let w = parse_border_width_val(values, ctx)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid border side width"))?;
    Ok(match side {
        0 => vec![
            StyleOp::BorderTopWidth(Some(w)),
            StyleOp::ThemeBorderTopWidth(None),
        ],
        1 => vec![
            StyleOp::BorderRightWidth(Some(w)),
            StyleOp::ThemeBorderRightWidth(None),
        ],
        2 => vec![
            StyleOp::BorderBottomWidth(Some(w)),
            StyleOp::ThemeBorderBottomWidth(None),
        ],
        _ => vec![
            StyleOp::BorderLeftWidth(Some(w)),
            StyleOp::ThemeBorderLeftWidth(None),
        ],
    })
}

fn compile_border_color_shorthand(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    let mut parsed = Vec::with_capacity(items.len());
    for it in items {
        let pc = parse_color(it, ctx.color_opts)
            .and_then(ParsedColor::into_paint)
            .ok_or_else(|| {
                CssDiag::error(span, DiagCode::InvalidValue, "invalid `border-color`")
            })?;
        parsed.push(pc);
    }
    let [t, r, b, l] = expand_four(&parsed)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `border-color`"))?;
    Ok(vec![
        StyleOp::BorderColor(Some(t)),
        StyleOp::BorderTopColor(Some(t)),
        StyleOp::BorderRightColor(Some(r)),
        StyleOp::BorderBottomColor(Some(b)),
        StyleOp::BorderLeftColor(Some(l)),
    ])
}

fn compile_border_side_color(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    side: usize,
) -> Result<Vec<StyleOp>, CssDiag> {
    let pc = parse_color(values, ctx.color_opts)
        .and_then(ParsedColor::into_paint)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid border side color"))?;
    Ok(vec![match side {
        0 => StyleOp::BorderTopColor(Some(pc)),
        1 => StyleOp::BorderRightColor(Some(pc)),
        2 => StyleOp::BorderBottomColor(Some(pc)),
        _ => StyleOp::BorderLeftColor(Some(pc)),
    }])
}

fn compile_border_style_shorthand(
    values: &[ComponentValue],
    span: Span,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    let mut parsed = Vec::with_capacity(items.len());
    for it in items {
        parsed.push(parse_border_style_val(it).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `border-style`")
        })?);
    }
    let [t, r, b, l] = expand_four(&parsed)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `border-style`"))?;
    Ok(vec![
        StyleOp::BorderStyle(Some(t)),
        StyleOp::BorderTopStyle(Some(t)),
        StyleOp::BorderRightStyle(Some(r)),
        StyleOp::BorderBottomStyle(Some(b)),
        StyleOp::BorderLeftStyle(Some(l)),
    ])
}

fn compile_border_side_style(
    values: &[ComponentValue],
    span: Span,
    side: usize,
) -> Result<Vec<StyleOp>, CssDiag> {
    let st = parse_border_style_val(values)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid border side style"))?;
    Ok(vec![match side {
        0 => StyleOp::BorderTopStyle(Some(st)),
        1 => StyleOp::BorderRightStyle(Some(st)),
        2 => StyleOp::BorderBottomStyle(Some(st)),
        _ => StyleOp::BorderLeftStyle(Some(st)),
    }])
}

fn compile_border_shorthand(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    side: Option<usize>,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() || items.len() > 3 {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `border`",
        ));
    }
    if items.len() == 1
        && matches!(items[0], [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("none")))
    {
        let mut ops = Vec::new();
        push_border_side_style(&mut ops, side, BorderStyle::None);
        push_border_side_width(&mut ops, side, 0.0);
        push_border_side_color(&mut ops, side, None);
        return Ok(ops);
    }
    let mut width: Option<f32> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<PaintColor> = None;
    for it in items {
        if style.is_none()
            && let Some(s) = parse_border_style_val(it)
        {
            style = Some(s);
        } else if width.is_none()
            && let Some(w) = parse_border_width_val(it, ctx)
        {
            width = Some(w);
        } else if color.is_none()
            && let Some(pc) = parse_color(it, ctx.color_opts).and_then(ParsedColor::into_paint)
        {
            color = Some(pc);
        } else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid token in `border` shorthand",
            ));
        }
    }
    let mut ops = Vec::new();
    if let Some(w) = width {
        push_border_side_width(&mut ops, side, w);
    }
    if let Some(s) = style {
        push_border_side_style(&mut ops, side, s);
    }
    if let Some(pc) = color {
        push_border_side_color(&mut ops, side, Some(pc));
    }
    Ok(ops)
}

fn push_border_side_width(ops: &mut Vec<StyleOp>, side: Option<usize>, w: f32) {
    match side {
        None => {
            ops.extend(clear_theme_ops_for_property("border-width"));
            ops.push(StyleOp::BorderWidth(Some(w)));
            ops.push(StyleOp::BorderTopWidth(Some(w)));
            ops.push(StyleOp::BorderRightWidth(Some(w)));
            ops.push(StyleOp::BorderBottomWidth(Some(w)));
            ops.push(StyleOp::BorderLeftWidth(Some(w)));
        }
        Some(0) => {
            ops.push(StyleOp::BorderTopWidth(Some(w)));
            ops.push(StyleOp::ThemeBorderTopWidth(None));
        }
        Some(1) => {
            ops.push(StyleOp::BorderRightWidth(Some(w)));
            ops.push(StyleOp::ThemeBorderRightWidth(None));
        }
        Some(2) => {
            ops.push(StyleOp::BorderBottomWidth(Some(w)));
            ops.push(StyleOp::ThemeBorderBottomWidth(None));
        }
        _ => {
            ops.push(StyleOp::BorderLeftWidth(Some(w)));
            ops.push(StyleOp::ThemeBorderLeftWidth(None));
        }
    }
}

fn push_border_side_style(ops: &mut Vec<StyleOp>, side: Option<usize>, s: BorderStyle) {
    match side {
        None => {
            ops.push(StyleOp::BorderStyle(Some(s)));
            ops.push(StyleOp::BorderTopStyle(Some(s)));
            ops.push(StyleOp::BorderRightStyle(Some(s)));
            ops.push(StyleOp::BorderBottomStyle(Some(s)));
            ops.push(StyleOp::BorderLeftStyle(Some(s)));
        }
        Some(0) => ops.push(StyleOp::BorderTopStyle(Some(s))),
        Some(1) => ops.push(StyleOp::BorderRightStyle(Some(s))),
        Some(2) => ops.push(StyleOp::BorderBottomStyle(Some(s))),
        _ => ops.push(StyleOp::BorderLeftStyle(Some(s))),
    }
}

fn push_border_side_color(ops: &mut Vec<StyleOp>, side: Option<usize>, c: Option<PaintColor>) {
    match side {
        None => {
            ops.push(StyleOp::BorderColor(c));
            ops.push(StyleOp::BorderTopColor(c));
            ops.push(StyleOp::BorderRightColor(c));
            ops.push(StyleOp::BorderBottomColor(c));
            ops.push(StyleOp::BorderLeftColor(c));
        }
        Some(0) => ops.push(StyleOp::BorderTopColor(c)),
        Some(1) => ops.push(StyleOp::BorderRightColor(c)),
        Some(2) => ops.push(StyleOp::BorderBottomColor(c)),
        _ => ops.push(StyleOp::BorderLeftColor(c)),
    }
}

fn compile_border_radius(
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    // Support optional `/ <vertical>` by taking the horizontal radii before `/`.
    let horiz = if let Some(slash) = trimmed
        .iter()
        .position(|cv| matches!(cv.as_token(), Some(Token::Delim('/'))))
    {
        trim_ws(&trimmed[..slash])
    } else {
        trimmed
    };
    let items = split_whitespace_items(horiz);
    let mut parsed = Vec::with_capacity(items.len());
    for it in items {
        let len = parse_length_spec(it).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `border-radius`")
        })?;
        if is_negative_length(len) {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "negative `border-radius`",
            ));
        }
        parsed.push(len);
    }
    let corners = expand_four(&parsed)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `border-radius`"))?;
    state.border_radii = Some(corners);
    let mut ops = vec![
        StyleOp::BorderRadii(Some(corners)),
        StyleOp::ThemeBorderRadius(None),
    ];
    if let [LengthSpec::Px(r), b, c, d] = corners
        && b == LengthSpec::Px(r)
        && c == LengthSpec::Px(r)
        && d == LengthSpec::Px(r)
    {
        ops.push(StyleOp::BorderRadius(Some(r)));
    }
    Ok(ops)
}

fn compile_border_corner_radius(
    values: &[ComponentValue],
    span: Span,
    corner: usize,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() || items.len() > 2 {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid corner border-radius",
        ));
    }
    let len = parse_length_spec(items[0]).ok_or_else(|| {
        CssDiag::error(span, DiagCode::InvalidValue, "invalid corner border-radius")
    })?;
    if is_negative_length(len) {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "negative corner border-radius",
        ));
    }
    let mut corners = state.border_radii.unwrap_or([LengthSpec::Px(0.0); 4]);
    corners[corner] = len;
    state.border_radii = Some(corners);
    Ok(vec![
        StyleOp::BorderRadii(Some(corners)),
        StyleOp::ThemeBorderRadius(None),
    ])
}

fn is_negative_length(spec: LengthSpec) -> bool {
    match spec {
        LengthSpec::Px(v)
        | LengthSpec::Percent(v)
        | LengthSpec::Em(v)
        | LengthSpec::Rem(v)
        | LengthSpec::Viewport { value: v, .. } => v < 0.0,
        _ => false,
    }
}

// ============================================================================
// Border Image
// ============================================================================

fn compile_border_image_shorthand(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `border-image`",
        ));
    }
    if let [cv] = trimmed
        && cv
            .as_ident()
            .is_some_and(|s| s.eq_ignore_ascii_case("none"))
    {
        state.border_image = None;
        state.unsupported_border_image = false;
        return Ok(vec![
            StyleOp::BorderImage(None),
            StyleOp::UnsupportedBorderImage(false),
        ]);
    }
    let items = split_whitespace_items(trimmed);
    let mut source: Option<BackgroundImage> = None;
    let mut tail_items: Vec<&[ComponentValue]> = Vec::new();
    for it in items {
        if source.is_none()
            && let Some(img) = parse_background_image_item(it, ctx)
        {
            source = Some(img);
            continue;
        }
        tail_items.push(it);
    }
    let Some(source) = source else {
        state.border_image = None;
        state.unsupported_border_image = true;
        return Ok(vec![
            StyleOp::BorderImage(None),
            StyleOp::UnsupportedBorderImage(true),
        ]);
    };
    let Some((slice, fill)) = parse_border_image_slice_items(&tail_items) else {
        state.border_image = None;
        state.unsupported_border_image = true;
        return Ok(vec![
            StyleOp::BorderImage(None),
            StyleOp::UnsupportedBorderImage(true),
        ]);
    };
    let spec = BorderImageSpec {
        source,
        slice,
        fill,
    };
    if !spec.paints_linear_or_url() {
        state.border_image = None;
        state.unsupported_border_image = true;
        return Ok(vec![
            StyleOp::BorderImage(None),
            StyleOp::UnsupportedBorderImage(true),
        ]);
    }
    state.border_image = Some(spec.clone());
    state.unsupported_border_image = false;
    Ok(vec![
        StyleOp::BorderImage(Some(spec)),
        StyleOp::UnsupportedBorderImage(false),
    ])
}

fn compile_border_image_source(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `border-image-source`",
        ));
    }
    if let [cv] = trimmed
        && cv
            .as_ident()
            .is_some_and(|s| s.eq_ignore_ascii_case("none"))
    {
        state.border_image = None;
        return Ok(vec![StyleOp::BorderImage(None)]);
    }
    let Some(source) = parse_background_image_item(trimmed, ctx) else {
        state.border_image = None;
        state.unsupported_border_image = true;
        return Ok(vec![
            StyleOp::BorderImage(None),
            StyleOp::UnsupportedBorderImage(true),
        ]);
    };
    let mut spec = state
        .border_image
        .clone()
        .unwrap_or_else(|| BorderImageSpec::from_source(source.clone()));
    spec.source = source;
    if !spec.paints_linear_or_url() {
        state.border_image = None;
        state.unsupported_border_image = true;
        return Ok(vec![
            StyleOp::BorderImage(None),
            StyleOp::UnsupportedBorderImage(true),
        ]);
    }
    if state.unsupported_border_image {
        return Ok(Vec::new());
    }
    state.border_image = Some(spec.clone());
    Ok(vec![StyleOp::BorderImage(Some(spec))])
}

fn compile_border_image_slice(
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `border-image-slice`",
        ));
    }
    let Some((slice, fill)) = parse_border_image_slice_items(&items) else {
        state.border_image = None;
        state.unsupported_border_image = true;
        return Ok(vec![
            StyleOp::BorderImage(None),
            StyleOp::UnsupportedBorderImage(true),
        ]);
    };
    if state.unsupported_border_image {
        return Ok(Vec::new());
    }
    let mut spec = state
        .border_image
        .clone()
        .unwrap_or_else(|| BorderImageSpec::from_source(BackgroundImage::url("")));
    spec.slice = slice;
    spec.fill = fill;
    state.border_image = Some(spec.clone());
    Ok(vec![StyleOp::BorderImage(Some(spec))])
}

fn compile_border_image_extra(
    prop: &str,
    values: &[ComponentValue],
    span: Span,
    state: &mut DeclState,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("empty `{prop}`"),
        ));
    }
    let is_default = match prop {
        "border-image-width" => items.iter().all(|it| {
            matches!(*it, [cv] if matches!(cv.as_token(), Some(Token::Number { value, .. }) if (*value - 1.0).abs() < 1e-6))
        }),
        "border-image-outset" => items.iter().all(|it| {
            matches!(*it, [cv] if matches!(cv.as_token(),
                Some(Token::Number { value, .. }) if *value == 0.0
            ) || matches!(cv.as_token(),
                Some(Token::Dimension { value, unit }) if *value == 0.0 && unit.eq_ignore_ascii_case("px")
            ))
        }),
        "border-image-repeat" => items.iter().all(|it| {
            matches!(*it, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("stretch")))
        }),
        _ => false,
    };
    if !is_default {
        state.border_image = None;
        state.unsupported_border_image = true;
        return Ok(vec![
            StyleOp::BorderImage(None),
            StyleOp::UnsupportedBorderImage(true),
        ]);
    }
    Ok(Vec::new())
}

fn parse_border_image_slice_items(
    items: &[&[ComponentValue]],
) -> Option<([BorderImageSlice; 4], bool)> {
    let mut nums = Vec::new();
    let mut fill = false;
    for it in items {
        let [cv] = *it else { return None };
        match cv.as_token()? {
            Token::Delim('/') => return None,
            Token::Ident(s) if s.eq_ignore_ascii_case("fill") => fill = true,
            Token::Ident(s) if s.eq_ignore_ascii_case("stretch") => {}
            Token::Ident(_) => return None,
            Token::Percentage(p) if p.is_finite() && *p >= 0.0 => {
                nums.push(BorderImageSlice::Percent(*p as f32));
            }
            Token::Number { value, .. } if value.is_finite() && *value >= 0.0 => {
                nums.push(BorderImageSlice::Number(*value as f32));
            }
            _ => return None,
        }
    }
    let slice = if nums.is_empty() {
        [BorderImageSlice::Percent(100.0); 4]
    } else {
        expand_four(&nums)?
    };
    Some((slice, fill))
}

// ============================================================================
// SVG Stroke, Outline, Box Shadow, Mask, Clip Path, Filter, Backdrop Filter
// ============================================================================

fn compile_svg_stroke(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let pc = parse_color(values, ctx.color_opts)
        .and_then(ParsedColor::into_paint)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `stroke`"))?;
    Ok(vec![StyleOp::BorderColor(Some(pc))])
}

fn compile_svg_stroke_width(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let spec = parse_length_spec(values)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `stroke-width`"))?;
    let px = ctx
        .resolve_length_px(spec)
        .filter(|v| *v >= 0.0)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `stroke-width`"))?;
    Ok(vec![StyleOp::BorderWidth(Some(px))])
}

fn compile_outline_shorthand(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() || items.len() > 3 {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `outline`",
        ));
    }
    if items.len() == 1
        && matches!(items[0], [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("none")))
    {
        return Ok(vec![
            StyleOp::OutlineWidth(0.0),
            StyleOp::OutlineStyle(OutlineStyle::None),
            StyleOp::OutlineColor(None),
        ]);
    }
    let mut width: Option<f32> = None;
    let mut style: Option<OutlineStyle> = None;
    let mut color: Option<PaintColor> = None;
    for it in items {
        if style.is_none()
            && let [cv] = it
            && let Some(kw) = cv.as_ident()
        {
            match kw.to_ascii_lowercase().as_str() {
                "none" | "hidden" => {
                    style = Some(OutlineStyle::None);
                    continue;
                }
                "solid" => {
                    style = Some(OutlineStyle::Solid);
                    continue;
                }
                _ => {}
            }
        }
        if width.is_none()
            && let Some(w) = parse_border_width_val(it, ctx)
        {
            width = Some(w);
        } else if color.is_none()
            && let Some(pc) = parse_color(it, ctx.color_opts).and_then(ParsedColor::into_paint)
        {
            color = Some(pc);
        } else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `outline` token",
            ));
        }
    }
    let mut ops = Vec::new();
    if let Some(w) = width {
        ops.push(StyleOp::OutlineWidth(w));
    }
    let final_style = style.or_else(|| width.filter(|w| *w > 0.0).map(|_| OutlineStyle::Solid));
    if let Some(s) = final_style {
        ops.push(StyleOp::OutlineStyle(s));
    }
    if let Some(pc) = color {
        ops.push(StyleOp::OutlineColor(Some(pc)));
    }
    Ok(ops)
}

fn compile_outline_width(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if matches!(trimmed, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("none"))) {
        return Ok(vec![StyleOp::OutlineWidth(0.0)]);
    }
    let w = parse_border_width_val(trimmed, ctx)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `outline-width`"))?;
    Ok(vec![StyleOp::OutlineWidth(w)])
}

fn compile_outline_style(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `outline-style`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `outline-style`"))?;
    match kw.to_ascii_lowercase().as_str() {
        "none" | "hidden" => Ok(vec![StyleOp::OutlineStyle(OutlineStyle::None)]),
        "solid" => Ok(vec![StyleOp::OutlineStyle(OutlineStyle::Solid)]),
        _ => Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            format!("unsupported `outline-style: {kw}`"),
        )),
    }
}

fn compile_outline_color(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let pc = parse_color(values, ctx.color_opts)
        .and_then(ParsedColor::into_paint)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `outline-color`"))?;
    Ok(vec![StyleOp::OutlineColor(Some(pc))])
}

fn compile_box_shadow(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `box-shadow`",
        ));
    }
    if matches!(trimmed, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("none"))) {
        return Ok(vec![StyleOp::BoxShadows(Vec::new())]);
    }
    let groups = split_comma_groups(trimmed);
    let mut layers = Vec::new();
    for g in groups {
        let layer = parse_one_box_shadow(g, ctx).ok_or_else(|| {
            CssDiag::error(span, DiagCode::InvalidValue, "invalid `box-shadow` layer")
        })?;
        layers.push(layer);
        if layers.len() >= nana_ui_core::MAX_BOX_SHADOWS {
            break;
        }
    }
    Ok(vec![StyleOp::BoxShadows(layers)])
}

fn parse_one_box_shadow(values: &[ComponentValue], ctx: &CompileContext) -> Option<BoxShadowSpec> {
    let items = split_whitespace_items(values);
    let mut lengths = Vec::new();
    let mut color: Option<PaintColor> = None;
    let mut inset = false;

    for it in items {
        if matches!(it, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("inset"))) {
            if inset {
                return None;
            }
            inset = true;
            continue;
        }
        if let Some(ParsedColor::Paint(pc)) = parse_color(it, ctx.color_opts) {
            if color.is_some() {
                return None;
            }
            color = Some(pc);
            continue;
        }
        let spec = parse_length_spec(it)?;
        let px = ctx.resolve_length_px(spec)?;
        lengths.push(px);
    }

    if lengths.len() < 2 || lengths.len() > 4 {
        return None;
    }
    let blur_radius = match lengths.get(2) {
        Some(&b) if b >= 0.0 => b,
        Some(_) => return None,
        None => 0.0,
    };
    let spread_radius = lengths.get(3).copied().unwrap_or(0.0);
    Some(BoxShadowSpec {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_radius,
        spread_radius,
        color: color.unwrap_or(PaintColor::srgb([0.0, 0.0, 0.0, 1.0])),
        inset,
    })
}

fn compile_mask_image(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `mask-image`",
        ));
    }
    if matches!(trimmed, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("none"))) {
        return Ok(vec![StyleOp::Mask(None)]);
    }
    let img = parse_background_image_item(trimmed, ctx)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `mask-image`"))?;
    let mask = match img {
        BackgroundImage::Gradient(g) => MaskImage::Gradient(g),
        BackgroundImage::Url { url, .. } => MaskImage::Url(url),
    };
    Ok(vec![StyleOp::Mask(Some(mask))])
}

fn compile_clip_path(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `clip-path`",
        ));
    }
    if matches!(trimmed, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("none"))) {
        return Ok(vec![StyleOp::ClipPath(None)]);
    }
    let cp = parse_clip_path_fn(trimmed)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `clip-path`"))?;
    Ok(vec![StyleOp::ClipPath(Some(cp))])
}

fn parse_clip_path_fn(values: &[ComponentValue]) -> Option<ClipPath> {
    let [ComponentValue::Function { name, args, .. }] = values else {
        return None;
    };
    match name.to_ascii_lowercase().as_str() {
        "inset" => parse_clip_inset(args),
        "polygon" => parse_clip_polygon(args),
        "circle" => parse_clip_circle(args),
        "ellipse" => parse_clip_ellipse(args),
        _ => None,
    }
}

fn parse_clip_inset(args: &[ComponentValue]) -> Option<ClipPath> {
    let items = split_whitespace_items(args);
    let round_pos = items.iter().position(
        |it| matches!(*it, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("round"))),
    );
    let (len_items, round) = if let Some(idx) = round_pos {
        let r = items.get(idx + 1).and_then(|it| parse_length_spec(it))?;
        (&items[..idx], Some(r))
    } else {
        (items.as_slice(), None)
    };
    let mut parsed = Vec::with_capacity(len_items.len());
    for it in len_items {
        parsed.push(parse_length_spec(it)?);
    }
    let [top, right, bottom, left] = expand_four(&parsed)?;
    Some(ClipPath::Inset(ClipInset {
        top,
        right,
        bottom,
        left,
        round,
    }))
}

fn parse_clip_polygon(args: &[ComponentValue]) -> Option<ClipPath> {
    let groups = split_comma_groups(args);
    let mut points = Vec::new();
    for (idx, g) in groups.iter().enumerate() {
        if idx == 0
            && matches!(*g, [cv] if cv.as_ident().is_some_and(|s| {
                s.eq_ignore_ascii_case("nonzero") || s.eq_ignore_ascii_case("evenodd")
            }))
        {
            continue;
        }
        let items = split_whitespace_items(g);
        if items.len() != 2 {
            return None;
        }
        let x = parse_length_spec(items[0])?;
        let y = parse_length_spec(items[1])?;
        points.push(ClipPoint { x, y });
    }
    (points.len() >= 3).then_some(ClipPath::Polygon(points))
}

fn parse_clip_circle(args: &[ComponentValue]) -> Option<ClipPath> {
    let items = split_whitespace_items(args);
    let (radii, [cx, cy]) = split_shape_at(&items)?;
    let radius = match radii {
        [] => ClipShapeRadius::ClosestSide,
        [one] => parse_shape_radius(one)?,
        _ => return None,
    };
    Some(ClipPath::Circle(ClipCircle { radius, cx, cy }))
}

fn parse_clip_ellipse(args: &[ComponentValue]) -> Option<ClipPath> {
    let items = split_whitespace_items(args);
    let (radii, [cx, cy]) = split_shape_at(&items)?;
    let (rx, ry) = match radii {
        [] => (ClipShapeRadius::ClosestSide, ClipShapeRadius::ClosestSide),
        [one] => {
            let r = parse_shape_radius(one)?;
            (r, r)
        }
        [a, b] => (parse_shape_radius(a)?, parse_shape_radius(b)?),
        _ => return None,
    };
    Some(ClipPath::Ellipse(ClipEllipse { rx, ry, cx, cy }))
}

fn split_shape_at<'a>(
    items: &'a [&'a [ComponentValue]],
) -> Option<(&'a [&'a [ComponentValue]], [LengthSpec; 2])> {
    if let Some(at_idx) = items.iter().position(
        |it| matches!(*it, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("at"))),
    ) {
        let mut pos_cvs = Vec::new();
        for it in &items[at_idx + 1..] {
            if !pos_cvs.is_empty() {
                pos_cvs.push(ws_cv());
            }
            pos_cvs.extend_from_slice(it);
        }
        let pos = parse_position(&pos_cvs)?;
        Some((&items[..at_idx], [pos.x, pos.y]))
    } else {
        Some((
            items,
            [LengthSpec::Percent(50.0), LengthSpec::Percent(50.0)],
        ))
    }
}

fn parse_shape_radius(values: &[ComponentValue]) -> Option<ClipShapeRadius> {
    if let [cv] = trim_ws(values)
        && let Some(kw) = cv.as_ident()
    {
        return match kw.to_ascii_lowercase().as_str() {
            "closest-side" => Some(ClipShapeRadius::ClosestSide),
            "farthest-side" => Some(ClipShapeRadius::FarthestSide),
            _ => None,
        };
    }
    parse_length_spec(values).map(ClipShapeRadius::Length)
}

fn compile_filter(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `filter`",
        ));
    }
    if matches!(trimmed, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("none"))) {
        return Ok(vec![StyleOp::Filter(None)]);
    }
    let filter = parse_color_filter_values(trimmed, ctx)
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `filter`"))?;
    Ok(vec![StyleOp::Filter(
        (!filter.is_identity()).then_some(filter),
    )])
}

/// Parse CSS `filter` functions directly from `&[ComponentValue]`.
pub fn parse_color_filter_values(
    values: &[ComponentValue],
    ctx: &CompileContext,
) -> Option<ColorFilter> {
    let mut filter = ColorFilter::default();
    let mut saw_any = false;
    let mut saw_saturate = false;
    let mut saw_grayscale = false;

    for cv in values {
        if cv.is_whitespace() {
            continue;
        }
        let ComponentValue::Function { name, args, .. } = cv else {
            return None;
        };
        saw_any = true;
        match name.to_ascii_lowercase().as_str() {
            "brightness" => {
                filter.brightness = if trim_ws(args).is_empty() {
                    1.0
                } else {
                    parse_filter_scalar(args)?
                };
            }
            "saturate" => {
                if saw_grayscale {
                    return None;
                }
                saw_saturate = true;
                filter.saturate = if trim_ws(args).is_empty() {
                    1.0
                } else {
                    parse_filter_scalar(args)?
                };
            }
            "contrast" => {
                filter.contrast = if trim_ws(args).is_empty() {
                    1.0
                } else {
                    parse_filter_scalar(args)?
                };
            }
            "grayscale" => {
                if saw_saturate {
                    return None;
                }
                saw_grayscale = true;
                let amount = if trim_ws(args).is_empty() {
                    1.0
                } else {
                    parse_filter_scalar(args)?.clamp(0.0, 1.0)
                };
                filter.saturate *= 1.0 - amount;
            }
            "hue-rotate" => {
                filter.hue_rotate_deg = if trim_ws(args).is_empty() {
                    0.0
                } else {
                    crate::syntax::values::parse_angle_deg_or_zero(args)?
                };
            }
            "invert" => {
                filter.invert = if trim_ws(args).is_empty() {
                    1.0
                } else {
                    parse_filter_scalar(args)?.clamp(0.0, 1.0)
                };
            }
            "opacity" => {
                filter.opacity = if trim_ws(args).is_empty() {
                    1.0
                } else {
                    parse_filter_scalar(args)?.clamp(0.0, 1.0)
                };
            }
            "blur" => {
                let px = if trim_ws(args).is_empty() {
                    0.0
                } else {
                    parse_blur_px(args, ctx)?
                };
                filter.blur_radius = px.clamp(0.0, ColorFilter::MAX_WIDE_BLUR_RADIUS);
            }
            "drop-shadow" => {
                if filter.drop_shadow.is_some() {
                    return None;
                }
                let mut shadow = parse_drop_shadow_values(args, ctx)?;
                shadow.blur_radius = shadow
                    .blur_radius
                    .clamp(0.0, ColorFilter::MAX_WIDE_BLUR_RADIUS);
                filter.drop_shadow = Some(shadow);
            }
            _ => return None,
        }
    }
    saw_any.then_some(filter)
}

fn compile_backdrop_filter(
    values: &[ComponentValue],
    span: Span,
    ctx: &CompileContext,
) -> Result<Vec<StyleOp>, CssDiag> {
    let trimmed = trim_ws(values);
    if trimmed.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty `backdrop-filter`",
        ));
    }
    if matches!(trimmed, [cv] if cv.as_ident().is_some_and(|s| s.eq_ignore_ascii_case("none"))) {
        return Ok(vec![StyleOp::BackdropFilter(None)]);
    }
    let mut filter = BackdropFilter::default();
    let mut saw_any = false;
    for cv in trimmed {
        if cv.is_whitespace() {
            continue;
        }
        let ComponentValue::Function { name, args, .. } = cv else {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                "invalid `backdrop-filter`",
            ));
        };
        saw_any = true;
        match name.to_ascii_lowercase().as_str() {
            "blur" => {
                let px = if trim_ws(args).is_empty() {
                    0.0
                } else {
                    parse_blur_px(args, ctx).ok_or_else(|| {
                        CssDiag::error(span, DiagCode::InvalidValue, "invalid `blur()` radius")
                    })?
                };
                filter.blur_radius = px.clamp(0.0, BackdropFilter::MAX_BLUR_RADIUS);
            }
            "saturate" => {
                filter.saturate = if trim_ws(args).is_empty() {
                    1.0
                } else {
                    parse_filter_scalar(args).ok_or_else(|| {
                        CssDiag::error(span, DiagCode::InvalidValue, "invalid `saturate()` amount")
                    })?
                };
            }
            _ => {}
        }
    }
    if !saw_any {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `backdrop-filter`",
        ));
    }
    Ok(vec![StyleOp::BackdropFilter(
        filter.is_active().then_some(filter),
    )])
}

fn parse_filter_scalar(values: &[ComponentValue]) -> Option<f32> {
    let [cv] = trim_ws(values) else { return None };
    match cv.as_token()? {
        Token::Number { value, .. } if value.is_finite() && *value >= 0.0 => Some(*value as f32),
        Token::Percentage(p) if p.is_finite() && *p >= 0.0 => Some(*p as f32 / 100.0),
        _ => None,
    }
}

fn parse_blur_px(values: &[ComponentValue], ctx: &CompileContext) -> Option<f32> {
    let spec = parse_length_spec(values)?;
    ctx.resolve_length_px(spec).filter(|v| *v >= 0.0)
}

fn parse_drop_shadow_values(
    values: &[ComponentValue],
    ctx: &CompileContext,
) -> Option<FilterDropShadow> {
    let items = split_whitespace_items(values);
    let mut lengths = Vec::new();
    let mut color: Option<PaintColor> = None;
    for it in items {
        if let Some(ParsedColor::Paint(pc)) = parse_color(it, ctx.color_opts) {
            if color.is_some() {
                return None;
            }
            color = Some(pc);
            continue;
        }
        let spec = parse_length_spec(it)?;
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
    Some(FilterDropShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_radius,
        color: color.unwrap_or(PaintColor::srgb([0.0, 0.0, 0.0, 1.0])),
    })
}

fn compile_mix_blend_mode(values: &[ComponentValue], span: Span) -> Result<Vec<StyleOp>, CssDiag> {
    let [cv] = trim_ws(values) else {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "invalid `mix-blend-mode`",
        ));
    };
    let kw = cv
        .as_ident()
        .ok_or_else(|| CssDiag::error(span, DiagCode::InvalidValue, "invalid `mix-blend-mode`"))?;
    let mode = match kw.to_ascii_lowercase().as_str() {
        "normal" => MixBlendMode::Normal,
        "multiply" => MixBlendMode::Multiply,
        "screen" => MixBlendMode::Screen,
        "overlay" => MixBlendMode::Overlay,
        "darken" => MixBlendMode::Darken,
        "lighten" => MixBlendMode::Lighten,
        "color-dodge" => MixBlendMode::ColorDodge,
        "color-burn" => MixBlendMode::ColorBurn,
        "hard-light" => MixBlendMode::HardLight,
        "soft-light" => MixBlendMode::SoftLight,
        "difference" => MixBlendMode::Difference,
        "exclusion" => MixBlendMode::Exclusion,
        "hue" => MixBlendMode::Hue,
        "saturation" => MixBlendMode::Saturation,
        "color" => MixBlendMode::Color,
        "luminosity" => MixBlendMode::Luminosity,
        _ => {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("unsupported `mix-blend-mode: {kw}`"),
            ));
        }
    };
    Ok(vec![StyleOp::MixBlend(mode)])
}

#[inline]
fn ws_cv() -> ComponentValue {
    ComponentValue::Token(SpannedToken::new(Token::Whitespace, Span::EMPTY))
}
