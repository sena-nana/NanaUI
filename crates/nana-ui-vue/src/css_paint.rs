//! CSS paint property parsers (`linear-gradient`, `url()`, `clip-path`, `filter`).

use nana_ui_core::box_layout::{
    BackdropFilter, BackgroundImage, BackgroundImageFit, BackgroundPosition, BackgroundRepeat,
    BorderImageSlice, BorderImageSpec, ClipCircle, ClipEllipse, ClipInset, ClipPath, ClipPoint,
    ClipShapeRadius, ColorFilter, CssGradient, FontFeatureSetting, GradientStop, LengthSpec,
    LinearGradient, MAX_BACKGROUND_LAYERS, MaskImage, MixBlendMode, OutlineStyle, OverflowSpec,
    PointerEventsSpec, RadialGradient, TextDecorationLine,
};

use crate::css_map::{
    CssLayoutParse, parse_css_length_px, resolve_paint_color, split_css_space_tokens,
};

const MAX_GRADIENT_STOPS: usize = 8;

/// Apply paint longhands/shorthands onto [`LayoutStyle`](nana_ui_core::LayoutStyle).
pub fn apply_css_paint_property(style: &mut nana_ui_core::LayoutStyle, name: &str, val: &str) {
    match name {
        "background" => apply_background_shorthand(style, val),
        "background-image" => apply_background_image(style, val),
        "background-size" => apply_background_size(style, val),
        "background-position" => apply_background_position(style, val),
        "background-repeat" => apply_background_repeat(style, val),
        "object-fit" => apply_object_fit(style, val),
        "object-position" => apply_object_position(style, val),
        "background-color" | "fill" => {
            let v = val.trim();
            if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("transparent") {
                style.background = None;
            } else if let Some(c) = resolve_paint_color(val) {
                style.background = Some(c);
            }
        }
        "mask-image" | "-webkit-mask-image" => {
            style.paint.mask = parse_mask_image(val);
        }
        "clip-path" => {
            style.paint.clip_path = parse_clip_path(val);
        }
        "filter" => {
            style.paint.filter = parse_color_filter(val);
        }
        "backdrop-filter" | "-webkit-backdrop-filter" => {
            style.paint.backdrop_filter = parse_backdrop_filter(val);
        }
        "text-shadow" => {
            style.paint.text_shadow = crate::css_map::parse_text_shadow(val);
        }
        "box-shadow" => {
            if let Some(layers) = crate::css_map::parse_box_shadows(val) {
                style.paint.box_shadows = layers;
            }
        }
        "outline" => apply_outline_shorthand(style, val),
        "outline-width" => {
            if val.trim().eq_ignore_ascii_case("none") {
                style.paint.outline.width = 0.0;
            } else if let Some(px) = parse_css_length_px(val, None) {
                style.paint.outline.width = px.max(0.0);
            }
        }
        "outline-color" => {
            if let Some(c) = resolve_paint_color(val) {
                style.paint.outline.color = Some(c);
            }
        }
        "outline-style" => apply_outline_style(style, val),
        "mix-blend-mode" => {
            if let Some(mode) = MixBlendMode::parse(val) {
                style.paint.mix_blend = mode;
            }
        }
        "line-clamp" | "-webkit-line-clamp" => apply_line_clamp(style, val),
        "text-decoration" | "text-decoration-line" => apply_text_decoration_line(style, val),
        "font-feature-settings" => apply_font_feature_settings(style, val),
        "font-variation-settings" => apply_font_variation_settings(style, val),
        "pointer-events" => {
            let v = val.trim();
            if v.eq_ignore_ascii_case("inherit") || v.eq_ignore_ascii_case("unset") {
                style.pointer_events = None;
            } else if let Some(spec) = PointerEventsSpec::parse(v) {
                style.pointer_events = Some(spec);
            }
        }
        "border-image" => apply_border_image_shorthand(style, val),
        "border-image-source" => apply_border_image_source(style, val),
        "border-image-slice" => apply_border_image_slice(style, val),
        "border-image-width" | "border-image-outset" | "border-image-repeat" => {
            apply_border_image_extra(style, name, val);
        }
        _ => {}
    }
}

fn apply_border_image_shorthand(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let trimmed = val.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        clear_border_image(style, false);
        return;
    }
    match parse_border_image_shorthand(trimmed) {
        Some(spec) => {
            style.paint.border_image = Some(spec);
            style.paint.unsupported_border_image = false;
        }
        None => clear_border_image(style, true),
    }
}

fn apply_border_image_source(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let trimmed = val.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        style.paint.border_image = None;
        return;
    }
    match parse_border_image_source(trimmed) {
        BorderImageSourceParse::Supported(source, rest) if rest.trim().is_empty() => {
            let mut spec = style
                .paint
                .border_image
                .take()
                .unwrap_or_else(|| BorderImageSpec::from_source(source.clone()));
            spec.source = source;
            if spec.paints_linear_or_url() {
                set_border_image(style, spec);
            } else {
                mark_border_image_unsupported(style);
            }
        }
        _ => mark_border_image_unsupported(style),
    }
}

fn apply_border_image_slice(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let trimmed = val.trim();
    if trimmed.is_empty() {
        return;
    }
    let Some((slice, fill)) = parse_border_image_tail(trimmed) else {
        mark_border_image_unsupported(style);
        return;
    };
    let mut spec = style
        .paint
        .border_image
        .take()
        .unwrap_or_else(|| BorderImageSpec::from_source(BackgroundImage::url("")));
    spec.slice = slice;
    spec.fill = fill;
    set_border_image(style, spec);
}

fn apply_border_image_extra(style: &mut nana_ui_core::LayoutStyle, name: &str, val: &str) {
    let trimmed = val.trim();
    if trimmed.is_empty() {
        return;
    }
    let default = match name {
        "border-image-width" => is_default_border_image_width(trimmed),
        "border-image-outset" => is_default_border_image_outset(trimmed),
        "border-image-repeat" => is_default_border_image_repeat(trimmed),
        _ => false,
    };
    if !default {
        mark_border_image_unsupported(style);
    }
}

fn set_border_image(style: &mut nana_ui_core::LayoutStyle, spec: BorderImageSpec) {
    if style.paint.unsupported_border_image {
        return;
    }
    style.paint.border_image = Some(spec);
}

fn mark_border_image_unsupported(style: &mut nana_ui_core::LayoutStyle) {
    style.paint.border_image = None;
    style.paint.unsupported_border_image = true;
}

fn clear_border_image(style: &mut nana_ui_core::LayoutStyle, unsupported: bool) {
    style.paint.border_image = None;
    style.paint.unsupported_border_image = unsupported;
}

enum BorderImageSourceParse {
    Supported(BackgroundImage, String),
    Unsupported,
    Missing,
}

fn parse_border_image_shorthand(input: &str) -> Option<BorderImageSpec> {
    match parse_border_image_source(input) {
        BorderImageSourceParse::Supported(source, rest) => {
            let (slice, fill) = parse_border_image_tail(&rest)?;
            let spec = BorderImageSpec {
                source,
                slice,
                fill,
            };
            spec.paints_linear_or_url().then_some(spec)
        }
        BorderImageSourceParse::Unsupported | BorderImageSourceParse::Missing => None,
    }
}

fn parse_border_image_source(input: &str) -> BorderImageSourceParse {
    let trimmed = input.trim();
    if let Some(url) = extract_first_url(trimmed) {
        return BorderImageSourceParse::Supported(
            BackgroundImage::url(url),
            strip_first_function(trimmed, "url"),
        );
    }
    if let Some(grad) = parse_linear_gradient(trimmed) {
        return BorderImageSourceParse::Supported(
            BackgroundImage::Gradient(CssGradient::Linear(grad)),
            strip_first_linear_gradient_fn(trimmed),
        );
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("radial-gradient(")
        || lower.contains("repeating-linear-gradient(")
        || lower.contains("repeating-radial-gradient(")
        || lower.contains("conic-gradient(")
    {
        return BorderImageSourceParse::Unsupported;
    }
    BorderImageSourceParse::Missing
}

fn parse_border_image_tail(rest: &str) -> Option<([BorderImageSlice; 4], bool)> {
    if rest.contains('/') {
        return None;
    }
    let mut nums = Vec::new();
    let mut fill = false;
    for token in rest.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower == "fill" {
            fill = true;
            continue;
        }
        if lower == "stretch" {
            continue;
        }
        if matches!(lower.as_str(), "repeat" | "round" | "space") {
            return None;
        }
        nums.push(parse_border_image_slice_token(token)?);
    }
    if nums.len() > 4 {
        return None;
    }
    Some((expand_border_image_slice(&nums), fill))
}

fn parse_border_image_slice_token(token: &str) -> Option<BorderImageSlice> {
    if let Some(percent) = token.strip_suffix('%') {
        let value = percent.parse::<f32>().ok()?;
        return value
            .is_finite()
            .then_some(BorderImageSlice::Percent(value.max(0.0)));
    }
    if token.chars().any(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }
    let value = token.parse::<f32>().ok()?;
    value
        .is_finite()
        .then_some(BorderImageSlice::Number(value.max(0.0)))
}

fn expand_border_image_slice(nums: &[BorderImageSlice]) -> [BorderImageSlice; 4] {
    let hundred = BorderImageSlice::Percent(100.0);
    match nums {
        [] => [hundred; 4],
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d, ..] => [*a, *b, *c, *d],
    }
}

fn is_default_border_image_width(val: &str) -> bool {
    let tokens: Vec<&str> = val.split_whitespace().collect();
    !tokens.is_empty() && tokens.iter().all(|token| *token == "1")
}

fn is_default_border_image_outset(val: &str) -> bool {
    let tokens: Vec<&str> = val.split_whitespace().collect();
    !tokens.is_empty()
        && tokens
            .iter()
            .all(|token| *token == "0" || token.eq_ignore_ascii_case("0px"))
}

fn is_default_border_image_repeat(val: &str) -> bool {
    let tokens: Vec<&str> = val.split_whitespace().collect();
    !tokens.is_empty()
        && tokens
            .iter()
            .all(|token| token.eq_ignore_ascii_case("stretch"))
}

/// Bind `<img src>` onto the replaced-content paint slot (above CSS backgrounds).
pub fn apply_img_replaced_content(style: &mut nana_ui_core::LayoutStyle, src: &str) {
    let trimmed = src.trim();
    if trimmed.is_empty() {
        style.paint.content_image = None;
        return;
    }
    let fit = style
        .paint
        .object_fit
        .unwrap_or(BackgroundImageFit::Stretch);
    let position = style
        .paint
        .object_position
        .unwrap_or_else(BackgroundPosition::center);
    style.paint.content_image = Some(BackgroundImage::Url {
        url: trimmed.to_string(),
        fit,
        size_width: None,
        size_height: None,
        position,
        repeat: BackgroundRepeat::NoRepeat,
    });
}

/// Bind `<video poster>` only when there is no HostTexture slot.
///
/// `data-nana-video` / Runtime [`nana_ui_runtime::Video`] samples
/// `"nana.host-texture"` (`video:{id}`). A slotted surface must not also
/// paint `poster` as `content_image`. Decode and playback stay with the host.
pub fn apply_video_poster(
    style: &mut nana_ui_core::LayoutStyle,
    poster: &str,
    has_host_texture_slot: bool,
) {
    if has_host_texture_slot {
        style.paint.content_image = None;
        style.paint.skipped_replaced = None;
        return;
    }
    let trimmed = poster.trim();
    if trimmed.is_empty() {
        style.paint.content_image = None;
        style.paint.skipped_replaced = Some("video".into());
        return;
    }
    style.paint.skipped_replaced = None;
    apply_img_replaced_content(style, trimmed);
}

/// `<iframe>` is not a browser. Do not fetch `src`.
pub fn apply_iframe_skip(style: &mut nana_ui_core::LayoutStyle) {
    style.paint.content_image = None;
    style.paint.skipped_replaced = Some("iframe".into());
}

/// `<canvas>` is not a CSS 2D bitmap. Pixels exist only on a HostTexture slot
/// (`data-nana-canvas` / `data-nana-gpu` → `"nana.host-texture"`). Bare canvas
/// is an empty box; do not bind `src` or a pixmap onto `content_image`.
pub fn apply_canvas_skip(style: &mut nana_ui_core::LayoutStyle, has_host_texture_slot: bool) {
    style.paint.content_image = None;
    style.paint.skipped_replaced = if has_host_texture_slot {
        None
    } else {
        Some("canvas".into())
    };
}

fn apply_background_shorthand(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    // CSS `background` resets size/position/repeat longhands. Do not zip
    // leftover lists onto a later `background-image`.
    reset_background_placement_lists(style);
    let trimmed = val.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        style.background = None;
        clear_background_images(style);
        return;
    }
    let parts = split_top_level_commas(trimmed);
    if parts.is_empty() {
        return;
    }
    let mut color = None;
    let mut layers = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let last = index + 1 == parts.len();
        let parsed = parse_background_layer(part, last);
        if let Some(c) = parsed.color {
            color = Some(c);
        }
        if let Some(image) = parsed.image {
            layers.push(image);
        }
    }
    if let Some(c) = color {
        style.background = Some(c);
    }
    if layers.is_empty() {
        if color.is_some() {
            clear_background_images(style);
        } else if let Some(c) = resolve_paint_color(trimmed) {
            style.background = Some(c);
            clear_background_images(style);
        }
        return;
    }
    assign_background_layers(style, layers);
}

fn apply_background_image(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let trimmed = val.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        clear_background_images(style);
        return;
    }
    let mut layers: Vec<BackgroundImage> = split_top_level_commas(trimmed)
        .into_iter()
        .filter_map(|part| parse_background_image_value(&part))
        .take(MAX_BACKGROUND_LAYERS)
        .collect();
    if layers.is_empty() {
        return;
    }
    zip_background_longhands(&mut layers, style);
    assign_background_layers(style, layers);
}

fn apply_background_size(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let (fits, lengths) = parse_background_size_list(val);
    if fits.is_empty() {
        return;
    }
    style.paint.background_size_list = fits;
    style.paint.background_size_lengths = lengths;
    apply_size_to_existing_layers(style);
}

fn apply_background_position(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let list = parse_background_position_list(val);
    if list.is_empty() {
        return;
    }
    style.paint.background_position_list = list;
    apply_position_to_existing_layers(style);
}

fn apply_background_repeat(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let list = parse_background_repeat_list(val);
    if list.is_empty() {
        return;
    }
    style.paint.background_repeat_list = list;
    apply_repeat_to_existing_layers(style);
}

fn apply_object_fit(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let Some(fit) = parse_object_fit(val) else {
        return;
    };
    style.paint.object_fit = Some(fit);
    if let Some(BackgroundImage::Url { fit: slot, .. }) = style.paint.content_image.as_mut() {
        *slot = fit;
    }
}

fn apply_object_position(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let Some(position) = parse_background_position_value(val) else {
        return;
    };
    style.paint.object_position = Some(position);
    if let Some(BackgroundImage::Url { position: slot, .. }) = style.paint.content_image.as_mut() {
        *slot = position;
    }
}

fn reset_background_placement_lists(style: &mut nana_ui_core::LayoutStyle) {
    style.paint.background_size_list.clear();
    style.paint.background_size_lengths.clear();
    style.paint.background_position_list.clear();
    style.paint.background_repeat_list.clear();
}

fn clear_background_images(style: &mut nana_ui_core::LayoutStyle) {
    style.paint.background_image = None;
    style.paint.background_layers.clear();
}

fn assign_background_layers(
    style: &mut nana_ui_core::LayoutStyle,
    mut layers: Vec<BackgroundImage>,
) {
    style.paint.background_image = layers.first().cloned();
    if layers.len() > 1 {
        style.paint.background_layers = layers.split_off(1);
    } else {
        style.paint.background_layers.clear();
    }
}

fn zip_background_longhands(layers: &mut [BackgroundImage], style: &nana_ui_core::LayoutStyle) {
    for (index, layer) in layers.iter_mut().enumerate() {
        apply_slot_size(
            layer,
            cycle(&style.paint.background_size_list, index),
            cycle(&style.paint.background_size_lengths, index),
        );
        apply_slot_position(layer, cycle(&style.paint.background_position_list, index));
        apply_slot_repeat(layer, cycle(&style.paint.background_repeat_list, index));
    }
}

fn apply_size_to_existing_layers(style: &mut nana_ui_core::LayoutStyle) {
    let fits = style.paint.background_size_list.clone();
    let lengths = style.paint.background_size_lengths.clone();
    if let Some(layer) = style.paint.background_image.as_mut() {
        apply_slot_size(layer, cycle(&fits, 0), cycle(&lengths, 0));
    }
    for (index, layer) in style.paint.background_layers.iter_mut().enumerate() {
        apply_slot_size(layer, cycle(&fits, index + 1), cycle(&lengths, index + 1));
    }
}

fn apply_position_to_existing_layers(style: &mut nana_ui_core::LayoutStyle) {
    let list = style.paint.background_position_list.clone();
    if let Some(layer) = style.paint.background_image.as_mut() {
        apply_slot_position(layer, cycle(&list, 0));
    }
    for (index, layer) in style.paint.background_layers.iter_mut().enumerate() {
        apply_slot_position(layer, cycle(&list, index + 1));
    }
}

fn apply_repeat_to_existing_layers(style: &mut nana_ui_core::LayoutStyle) {
    let list = style.paint.background_repeat_list.clone();
    if let Some(layer) = style.paint.background_image.as_mut() {
        apply_slot_repeat(layer, cycle(&list, 0));
    }
    for (index, layer) in style.paint.background_layers.iter_mut().enumerate() {
        apply_slot_repeat(layer, cycle(&list, index + 1));
    }
}

fn cycle<T: Copy>(list: &[T], index: usize) -> Option<T> {
    if list.is_empty() {
        None
    } else {
        Some(list[index % list.len()])
    }
}

fn apply_slot_size(
    layer: &mut BackgroundImage,
    fit: Option<BackgroundImageFit>,
    lengths: Option<(Option<LengthSpec>, Option<LengthSpec>)>,
) {
    let Some(BackgroundImage::Url {
        fit: slot,
        size_width,
        size_height,
        ..
    }) = layer_url_mut(layer)
    else {
        return;
    };
    if let Some(fit) = fit {
        *slot = fit;
    }
    if let Some((width, height)) = lengths {
        *size_width = width;
        *size_height = height;
    }
}

fn apply_slot_position(layer: &mut BackgroundImage, position: Option<BackgroundPosition>) {
    let Some(BackgroundImage::Url { position: slot, .. }) = layer_url_mut(layer) else {
        return;
    };
    if let Some(position) = position {
        *slot = position;
    }
}

fn apply_slot_repeat(layer: &mut BackgroundImage, repeat: Option<BackgroundRepeat>) {
    let Some(BackgroundImage::Url { repeat: slot, .. }) = layer_url_mut(layer) else {
        return;
    };
    if let Some(repeat) = repeat {
        *slot = repeat;
    }
}

fn layer_url_mut(layer: &mut BackgroundImage) -> Option<&mut BackgroundImage> {
    matches!(layer, BackgroundImage::Url { .. }).then_some(layer)
}

fn parse_background_image_value(input: &str) -> Option<BackgroundImage> {
    let trimmed = input.trim();
    if let Some(grad) = parse_linear_gradient(trimmed) {
        return Some(BackgroundImage::Gradient(CssGradient::Linear(grad)));
    }
    if let Some(grad) = parse_radial_gradient(trimmed) {
        return Some(BackgroundImage::Gradient(CssGradient::Radial(grad)));
    }
    parse_css_url(trimmed).map(BackgroundImage::url)
}

struct ParsedBackgroundLayer {
    image: Option<BackgroundImage>,
    color: Option<[f32; 4]>,
}

fn parse_background_layer(input: &str, allow_color: bool) -> ParsedBackgroundLayer {
    let trimmed = input.trim();
    let mut rest = trimmed.to_string();
    let mut image = None;
    if let Some(url) = extract_first_url(&rest) {
        image = Some(BackgroundImage::url(url.as_str()));
        rest = strip_first_function(&rest, "url");
    } else if let Some(grad) = parse_linear_gradient(trimmed) {
        image = Some(BackgroundImage::Gradient(CssGradient::Linear(grad)));
        rest = strip_first_linear_gradient_fn(&rest);
    } else if let Some(grad) = parse_radial_gradient(trimmed) {
        image = Some(BackgroundImage::Gradient(CssGradient::Radial(grad)));
        rest = strip_first_function(&rest, "radial-gradient");
    }
    let mut color = None;
    if allow_color {
        for token in rest.split_whitespace() {
            if let Some(c) = resolve_paint_color(token) {
                color = Some(c);
                break;
            }
        }
        if color.is_none()
            && let Some(c) = resolve_paint_color(&rest)
        {
            color = Some(c);
        }
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
        if let Some(parsed_repeat) = parse_repeat_from_tokens(&rest) {
            *repeat = parsed_repeat;
        }
        if let Some((parsed_fit, w, h)) = parse_size_after_slash(&rest) {
            *fit = parsed_fit;
            *size_width = w;
            *size_height = h;
        }
        if let Some(parsed_pos) = parse_position_from_layer_rest(&rest) {
            *position = parsed_pos;
        }
    }
    ParsedBackgroundLayer { image, color }
}

pub fn parse_linear_gradient(input: &str) -> Option<LinearGradient> {
    let trimmed = input.trim();
    let repeating = strip_function(trimmed, "repeating-linear-gradient").is_some();
    let inner = if repeating {
        strip_function(trimmed, "repeating-linear-gradient")?
    } else {
        strip_function(trimmed, "linear-gradient")?
    };
    let (angle_deg, stops_src) = split_gradient_header(inner)?;
    let stops = parse_gradient_stops(stops_src, repeating)?;
    if stops.len() < 2 {
        return None;
    }
    Some(LinearGradient { angle_deg, stops })
}

fn strip_first_linear_gradient_fn(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    if lower.contains("repeating-linear-gradient(") {
        strip_first_function(input, "repeating-linear-gradient")
    } else {
        strip_first_function(input, "linear-gradient")
    }
}

fn parse_mask_image(input: &str) -> Option<MaskImage> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return None;
    }
    if let Some(grad) = parse_linear_gradient(trimmed) {
        return Some(MaskImage::Gradient(CssGradient::Linear(grad)));
    }
    if let Some(grad) = parse_radial_gradient(trimmed) {
        return Some(MaskImage::Gradient(CssGradient::Radial(grad)));
    }
    parse_css_url(trimmed).map(MaskImage::Url)
}

pub fn parse_radial_gradient(input: &str) -> Option<RadialGradient> {
    let inner = strip_function(input, "radial-gradient")?;
    let (circle, center, stops_src) = split_radial_header(inner)?;
    let stops = parse_gradient_stops(stops_src, false)?;
    if stops.len() < 2 {
        return None;
    }
    Some(RadialGradient {
        circle,
        center,
        stops,
    })
}

fn default_radial_center() -> [LengthSpec; 2] {
    [LengthSpec::Percent(50.0), LengthSpec::Percent(50.0)]
}

fn split_radial_header(input: &str) -> Option<(bool, [LengthSpec; 2], &str)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let comma = trimmed.find(',')?;
    let head = trimmed[..comma].trim();
    let tail = trimmed[comma + 1..].trim();
    let lower = head.to_ascii_lowercase();
    let circle = !lower.starts_with("ellipse");
    let center = if let Some(at_idx) = lower.find(" at ") {
        parse_radial_center(&head[at_idx + 4..])?
    } else if split_color_and_rest(head).is_some() {
        return Some((circle, default_radial_center(), trimmed));
    } else {
        default_radial_center()
    };
    Some((circle, center, tail))
}

fn parse_radial_center(input: &str) -> Option<[LengthSpec; 2]> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("center") {
        return Some(default_radial_center());
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() == 2 {
        let x = parse_radial_center_axis(parts[0], true)?;
        let y = parse_radial_center_axis(parts[1], false)?;
        return Some([x, y]);
    }
    if parts.len() == 1 {
        let x = parse_radial_center_axis(parts[0], true)?;
        return Some([x, LengthSpec::Percent(50.0)]);
    }
    None
}

fn parse_radial_center_axis(token: &str, horizontal: bool) -> Option<LengthSpec> {
    let lower = token.to_ascii_lowercase();
    match lower.as_str() {
        "left" if horizontal => Some(LengthSpec::Percent(0.0)),
        "right" if horizontal => Some(LengthSpec::Percent(100.0)),
        "top" if !horizontal => Some(LengthSpec::Percent(0.0)),
        "bottom" if !horizontal => Some(LengthSpec::Percent(100.0)),
        "center" => Some(LengthSpec::Percent(50.0)),
        _ => {
            if lower.ends_with('%') {
                return lower
                    .trim_end_matches('%')
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .map(LengthSpec::Percent);
            }
            let spec = LengthSpec::parse(token)?;
            match spec {
                LengthSpec::Fill
                | LengthSpec::Shrink
                | LengthSpec::Auto
                | LengthSpec::MinContent
                | LengthSpec::MaxContent
                | LengthSpec::FitContent => None,
                other => Some(other),
            }
        }
    }
}

pub fn parse_clip_path(input: &str) -> Option<ClipPath> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return None;
    }
    if let Some(inner) = strip_function(trimmed, "inset") {
        return parse_clip_inset(inner);
    }
    if let Some(inner) = strip_function(trimmed, "polygon") {
        return parse_clip_polygon(inner);
    }
    if let Some(inner) = strip_function(trimmed, "circle") {
        return parse_clip_circle(inner);
    }
    if let Some(inner) = strip_function(trimmed, "ellipse") {
        return parse_clip_ellipse(inner);
    }
    None
}

pub fn parse_color_filter(input: &str) -> Option<ColorFilter> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return None;
    }
    let mut filter = ColorFilter::default();
    let mut saw_saturate = false;
    let mut saw_grayscale = false;
    for token in split_filter_tokens(trimmed) {
        if let Some(value) = strip_complete_function(&token, "brightness") {
            filter.brightness = parse_filter_scalar(value).unwrap_or(1.0);
        } else if let Some(value) = strip_complete_function(&token, "saturate") {
            if saw_grayscale {
                return None;
            }
            saw_saturate = true;
            filter.saturate = parse_filter_scalar(value).unwrap_or(1.0);
        } else if let Some(value) = strip_complete_function(&token, "contrast") {
            filter.contrast = parse_filter_scalar(value).unwrap_or(1.0);
        } else if let Some(value) = strip_complete_function(&token, "grayscale") {
            if saw_saturate {
                return None;
            }
            saw_grayscale = true;
            let amount = parse_filter_scalar(value).unwrap_or(1.0).clamp(0.0, 1.0);
            filter.saturate *= 1.0 - amount;
        } else if let Some(value) = strip_complete_function(&token, "hue-rotate") {
            filter.hue_rotate_deg = parse_hue_rotate(value).unwrap_or(0.0);
        } else if let Some(value) = strip_complete_function(&token, "invert") {
            filter.invert = parse_filter_scalar(value).unwrap_or(1.0).clamp(0.0, 1.0);
        } else if let Some(value) = strip_complete_function(&token, "opacity") {
            filter.opacity = parse_filter_scalar(value).unwrap_or(1.0).clamp(0.0, 1.0);
        } else if let Some(value) = strip_complete_function(&token, "blur") {
            let px = parse_blur_radius(value).unwrap_or(0.0);
            filter.blur_radius = px.clamp(0.0, ColorFilter::MAX_BLUR_RADIUS);
        } else if let Some(value) = strip_complete_function(&token, "drop-shadow") {
            if filter.drop_shadow.is_some() {
                return None;
            }
            let mut shadow = crate::css_map::parse_drop_shadow(value)?;
            shadow.blur_radius = shadow.blur_radius.clamp(0.0, ColorFilter::MAX_BLUR_RADIUS);
            filter.drop_shadow = Some(shadow);
        } else {
            // Unknown function: fail closed (do not apply the known subset).
            return None;
        }
    }
    if filter.is_identity() {
        None
    } else {
        Some(filter)
    }
}

/// Parse `backdrop-filter: blur(Npx) saturate(M)`; unknown functions are skipped.
pub fn parse_backdrop_filter(input: &str) -> Option<BackdropFilter> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return None;
    }
    let mut filter = BackdropFilter::default();
    for token in split_filter_tokens(trimmed) {
        if let Some(value) = strip_function(&token, "blur") {
            let px = parse_blur_radius(value).unwrap_or(0.0);
            filter.blur_radius = px.clamp(0.0, BackdropFilter::MAX_BLUR_RADIUS);
        } else if let Some(value) = strip_function(&token, "saturate") {
            filter.saturate = parse_filter_scalar(value).unwrap_or(1.0);
        }
    }
    if filter.is_active() {
        Some(filter)
    } else {
        None
    }
}

fn parse_blur_radius(input: &str) -> Option<f32> {
    let trimmed = input.trim();
    if trimmed.ends_with("px") {
        trimmed.trim_end_matches("px").trim().parse::<f32>().ok()
    } else {
        trimmed.parse::<f32>().ok()
    }
}

fn parse_filter_scalar(input: &str) -> Option<f32> {
    let trimmed = input.trim();
    if trimmed.ends_with('%') {
        trimmed
            .trim_end_matches('%')
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| v / 100.0)
    } else {
        trimmed.parse::<f32>().ok()
    }
}

fn parse_hue_rotate(input: &str) -> Option<f32> {
    let trimmed = input.trim().to_ascii_lowercase();
    if let Some(n) = trimmed.strip_suffix("deg") {
        return n.trim().parse().ok();
    }
    if let Some(n) = trimmed.strip_suffix("turn") {
        return n.trim().parse::<f32>().ok().map(|turns| turns * 360.0);
    }
    if let Some(n) = trimmed.strip_suffix("rad") {
        return n.trim().parse::<f32>().ok().map(|rad| rad.to_degrees());
    }
    trimmed.parse().ok()
}

fn apply_outline_style(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    match val.trim().to_ascii_lowercase().as_str() {
        "none" | "hidden" => style.paint.outline.style = OutlineStyle::None,
        "solid" => style.paint.outline.style = OutlineStyle::Solid,
        _ => {}
    }
}

fn apply_outline_shorthand(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let trimmed = val.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        style.paint.outline = Default::default();
        return;
    }
    let mut saw_style = false;
    for part in trimmed.split_whitespace() {
        let lower = part.to_ascii_lowercase();
        if lower == "none" || lower == "hidden" {
            style.paint.outline.style = OutlineStyle::None;
            saw_style = true;
        } else if lower == "solid" {
            style.paint.outline.style = OutlineStyle::Solid;
            saw_style = true;
        } else if let Some(px) = parse_css_length_px(part, None) {
            style.paint.outline.width = px.max(0.0);
        } else if let Some(c) = resolve_paint_color(part) {
            style.paint.outline.color = Some(c);
        }
    }
    if !saw_style && style.paint.outline.width > 0.0 {
        style.paint.outline.style = OutlineStyle::Solid;
    }
}

fn apply_text_decoration_line(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let trimmed = val.trim();
    if trimmed.is_empty() {
        return;
    }
    if trimmed.eq_ignore_ascii_case("none") {
        style.text_decoration = Some(TextDecorationLine::default());
        return;
    }
    let mut deco = TextDecorationLine::default();
    let mut saw = false;
    for part in trimmed.split_whitespace() {
        match part.to_ascii_lowercase().as_str() {
            "underline" => {
                deco.underline = true;
                saw = true;
            }
            "line-through" => {
                deco.line_through = true;
                saw = true;
            }
            "overline" | "blink" | "spelling-error" | "grammar-error" => {
                // cosmic-text can overline; Scene stroke path only does
                // underline / line-through. Unknown lines fail closed.
            }
            "solid" | "double" | "dotted" | "dashed" | "wavy" => {}
            _ => {}
        }
    }
    if saw {
        style.text_decoration = Some(deco);
    }
}

fn apply_font_feature_settings(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let trimmed = val.trim();
    if trimmed.eq_ignore_ascii_case("normal") {
        style.font_features = Some(Vec::new());
        return;
    }
    let Some(features) = parse_font_feature_settings(trimmed) else {
        return;
    };
    style.font_features = Some(features);
}

fn parse_font_feature_settings(raw: &str) -> Option<Vec<FontFeatureSetting>> {
    let mut out = Vec::new();
    for chunk in split_comma_list(raw) {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        let (tag, rest) = parse_feature_tag(chunk)?;
        let value = match rest.trim() {
            "" | "on" => 1,
            "off" => 0,
            other => other.parse::<u32>().ok()?,
        };
        out.push(FontFeatureSetting { tag, value });
    }
    Some(out)
}

fn parse_feature_tag(raw: &str) -> Option<([u8; 4], &str)> {
    let s = raw.trim();
    let quote = s.as_bytes().first().copied()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let rest = &s[1..];
    let end = rest.find(quote as char)?;
    let tag = rest[..end].as_bytes();
    if tag.len() != 4 || !tag.iter().all(|b| b.is_ascii()) {
        return None;
    }
    let mut out = [0u8; 4];
    out.copy_from_slice(tag);
    Some((out, rest[end + 1..].trim()))
}

fn split_comma_list(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&input[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    if start <= input.len() {
        parts.push(&input[start..]);
    }
    parts
}

fn apply_font_variation_settings(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    crate::css_map::apply_font_variation_settings(style, val);
}

fn apply_line_clamp(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let trimmed = val.trim();
    if trimmed.eq_ignore_ascii_case("none") || trimmed == "0" {
        style.line_clamp = None;
        return;
    }
    let Ok(n) = trimmed.parse::<u16>() else {
        return;
    };
    if n == 0 {
        style.line_clamp = None;
        return;
    }
    style.line_clamp = Some(n);
    style.text_overflow_ellipsis = true;
    style.white_space_nowrap = false;
    style.overflow_x = OverflowSpec::Hidden;
    style.overflow_y = OverflowSpec::Hidden;
}

/// Split a CSS `filter` / `backdrop-filter` list into function tokens.
///
/// A closing `)` at depth 0 starts the next function (`brightness(0.5)sepia()`
/// is two tokens). Top-level separators match `split_css_space_tokens`:
/// space, tab, and newline.
fn split_filter_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = idx + ch.len_utf8();
                    push_filter_token(&mut tokens, &input[start..end]);
                    start = end;
                }
            }
            ' ' | '\t' | '\n' if depth == 0 => {
                push_filter_token(&mut tokens, &input[start..idx]);
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    push_filter_token(&mut tokens, &input[start..]);
    tokens
}

fn push_filter_token(tokens: &mut Vec<String>, raw: &str) {
    let token = raw.trim();
    if !token.is_empty() {
        tokens.push(token.to_string());
    }
}

/// Like [`strip_function`], but leftover after the matching `)` is rejected
/// instead of dropped (`brightness(0.5)sepia()` is not a brightness token).
fn strip_complete_function<'a>(input: &'a str, name: &str) -> Option<&'a str> {
    let trimmed = input.trim();
    let (head, after_open) = trimmed.split_once('(')?;
    if !head.eq_ignore_ascii_case(name) {
        return None;
    }
    let mut depth = 1i32;
    for (idx, ch) in after_open.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    if !after_open[idx + ch.len_utf8()..].trim().is_empty() {
                        return None;
                    }
                    return Some(after_open[..idx].trim());
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_clip_inset(input: &str) -> Option<ClipPath> {
    let (lengths, round) = if let Some((lengths, round)) = split_inset_round(input) {
        (lengths, round)
    } else {
        (input.to_string(), None)
    };
    let parts: Vec<&str> = lengths.split_whitespace().collect();
    let parsed: Vec<LengthSpec> = parts
        .iter()
        .filter_map(|part| LengthSpec::parse(part))
        .collect();
    if parsed.is_empty() {
        return None;
    }
    let [top, right, bottom, left] = match parsed.len() {
        1 => [parsed[0]; 4],
        2 => [parsed[0], parsed[1], parsed[0], parsed[1]],
        3 => [parsed[0], parsed[1], parsed[2], parsed[1]],
        _ => [parsed[0], parsed[1], parsed[2], parsed[3]],
    };
    Some(ClipPath::Inset(ClipInset {
        top,
        right,
        bottom,
        left,
        round,
    }))
}

fn split_inset_round(input: &str) -> Option<(String, Option<LengthSpec>)> {
    let lower = input.to_ascii_lowercase();
    let round_idx = lower.find(" round ")?;
    let lengths = input[..round_idx].trim().to_string();
    let round_src = input[round_idx + " round ".len()..].trim();
    let round = LengthSpec::parse(round_src.split_whitespace().next()?);
    Some((lengths, round))
}

fn parse_clip_polygon(input: &str) -> Option<ClipPath> {
    let parts = split_top_level_commas(input);
    let mut points = Vec::new();
    for pair in parts {
        let coords: Vec<&str> = pair.split_whitespace().collect();
        if coords.len() != 2 {
            continue;
        }
        let x = LengthSpec::parse(coords[0])?;
        let y = LengthSpec::parse(coords[1])?;
        points.push(ClipPoint { x, y });
    }
    if points.len() < 3 {
        return None;
    }
    Some(ClipPath::Polygon(points))
}

fn parse_clip_circle(input: &str) -> Option<ClipPath> {
    let (radius_src, position) = split_basic_shape_at(input);
    let radius = if radius_src.is_empty() {
        ClipShapeRadius::ClosestSide
    } else {
        parse_shape_radius(&radius_src)?
    };
    let [cx, cy] = position;
    Some(ClipPath::Circle(ClipCircle { radius, cx, cy }))
}

fn parse_clip_ellipse(input: &str) -> Option<ClipPath> {
    let (radius_src, position) = split_basic_shape_at(input);
    let [cx, cy] = position;
    let (rx, ry) = if radius_src.is_empty() {
        (ClipShapeRadius::ClosestSide, ClipShapeRadius::ClosestSide)
    } else {
        let parts: Vec<&str> = radius_src.split_whitespace().collect();
        match parts.as_slice() {
            [one] => {
                let r = parse_shape_radius(one)?;
                (r, r)
            }
            [a, b] => (parse_shape_radius(a)?, parse_shape_radius(b)?),
            _ => return None,
        }
    };
    Some(ClipPath::Ellipse(ClipEllipse { rx, ry, cx, cy }))
}

fn split_basic_shape_at(input: &str) -> (String, [LengthSpec; 2]) {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();
    let (radii, pos_src) = if lower.starts_with("at ") {
        (String::new(), trimmed[3..].trim())
    } else if let Some(idx) = lower.find(" at ") {
        (trimmed[..idx].trim().to_string(), trimmed[idx + 4..].trim())
    } else {
        return (
            trimmed.to_string(),
            [LengthSpec::Percent(50.0), LengthSpec::Percent(50.0)],
        );
    };
    let pos = parse_radial_center(pos_src)
        .unwrap_or([LengthSpec::Percent(50.0), LengthSpec::Percent(50.0)]);
    (radii, pos)
}

fn parse_shape_radius(input: &str) -> Option<ClipShapeRadius> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "closest-side" => Some(ClipShapeRadius::ClosestSide),
        "farthest-side" => Some(ClipShapeRadius::FarthestSide),
        _ => LengthSpec::parse(trimmed).map(ClipShapeRadius::Length),
    }
}

fn split_gradient_header(input: &str) -> Option<(f32, &str)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(idx) = trimmed.find(',') {
        let head = trimmed[..idx].trim();
        let tail = trimmed[idx + 1..].trim();
        if is_gradient_angle_token(head) {
            let angle_deg = parse_gradient_angle(head)?;
            return Some((angle_deg, tail));
        }
        if split_color_and_rest(head).is_some() {
            return Some((180.0, trimmed));
        }
        return None;
    }
    if split_color_and_rest(trimmed).is_some() {
        return Some((180.0, trimmed));
    }
    None
}

fn is_gradient_angle_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    lower.ends_with("deg")
        || lower.ends_with("turn")
        || lower.starts_with("to ")
        || token.trim().parse::<f32>().ok().is_some()
}

fn parse_gradient_angle(token: &str) -> Option<f32> {
    let lower = token.to_ascii_lowercase();
    if lower.ends_with("deg") {
        return lower.trim_end_matches("deg").trim().parse().ok();
    }
    if lower.ends_with("turn") {
        return lower
            .trim_end_matches("turn")
            .trim()
            .parse::<f32>()
            .ok()
            .map(|turns| turns * 360.0);
    }
    if lower.starts_with("to ") {
        return Some(match lower.as_str() {
            "to top" => 0.0,
            "to right" => 90.0,
            "to bottom" => 180.0,
            "to left" => 270.0,
            "to top right" | "to right top" => 45.0,
            "to bottom right" | "to right bottom" => 135.0,
            "to bottom left" | "to left bottom" => 225.0,
            "to top left" | "to left top" => 315.0,
            _ => 180.0,
        });
    }
    token.parse::<f32>().ok().or_else(|| {
        if resolve_paint_color(token).is_some() {
            Some(180.0)
        } else {
            None
        }
    })
}

fn parse_gradient_stops(input: &str, repeating: bool) -> Option<Vec<GradientStop>> {
    let parts = split_top_level_commas(input);
    let mut raw = Vec::new();
    for part in &parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        raw.push(parse_color_stop_item(trimmed)?);
    }
    let mut stops = Vec::new();
    let last_index = raw.len().saturating_sub(1);
    for (index, item) in raw.iter().enumerate() {
        if item.positions.is_empty() {
            let position = if stops.is_empty() {
                0.0
            } else if index == last_index {
                1.0
            } else {
                index as f32 / (raw.len().saturating_sub(1) as f32)
            };
            stops.push(GradientStop {
                position,
                color: item.color,
            });
        } else {
            for &position in &item.positions {
                stops.push(GradientStop {
                    position,
                    color: item.color,
                });
            }
        }
        if stops.len() >= MAX_GRADIENT_STOPS && !repeating {
            break;
        }
    }
    if repeating {
        stops = expand_repeating_linear_stops(stops)?;
    } else {
        normalize_gradient_stops(&mut stops);
        stops.truncate(MAX_GRADIENT_STOPS);
    }
    if stops.len() < 2 { None } else { Some(stops) }
}

struct ColorStopItem {
    color: [f32; 4],
    positions: Vec<f32>,
}

fn parse_color_stop_item(input: &str) -> Option<ColorStopItem> {
    let (color, rest) = split_color_and_rest(input)?;
    let mut positions = Vec::new();
    for token in rest.split_whitespace() {
        if token.is_empty() {
            continue;
        }
        positions.push(parse_stop_position(token)?);
        if positions.len() > 2 {
            return None;
        }
    }
    Some(ColorStopItem { color, positions })
}

fn split_color_and_rest(input: &str) -> Option<([f32; 4], &str)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    for name in ["rgba", "hsla", "rgb", "hsl"] {
        if let Some(end) = leading_function_end(&lower, name) {
            let color = resolve_paint_color(&trimmed[..end])?;
            return Some((color, trimmed[end..].trim()));
        }
    }
    if let Some((head, rest)) = split_first_token(trimmed)
        && let Some(color) = resolve_paint_color(head)
    {
        return Some((color, rest));
    }
    resolve_paint_color(trimmed).map(|color| (color, ""))
}

fn leading_function_end(lower: &str, name: &str) -> Option<usize> {
    let head = format!("{name}(");
    if !lower.starts_with(&head) {
        return None;
    }
    let mut depth = 0i32;
    let mut started = false;
    for (idx, ch) in lower.char_indices() {
        match ch {
            '(' => {
                depth += 1;
                started = true;
            }
            ')' if started => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

fn split_first_token(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim();
    let idx = trimmed.find(char::is_whitespace)?;
    Some((trimmed[..idx].trim(), trimmed[idx..].trim()))
}

fn expand_repeating_linear_stops(stops: Vec<GradientStop>) -> Option<Vec<GradientStop>> {
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
            if pos < -1e-4 {
                continue;
            }
            if pos > 1.0 + 1e-4 {
                continue;
            }
            let pos = pos.clamp(0.0, 1.0);
            if let Some(last) = out.last()
                && (last.position - pos).abs() < 1e-5
                && last.color[0] == stop.color[0]
                && last.color[1] == stop.color[1]
                && last.color[2] == stop.color[2]
                && last.color[3] == stop.color[3]
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
    if out[0].position > 1e-4 || out.last().is_some_and(|stop| stop.position < 1.0 - 1e-4) {
        return None;
    }
    Some(out)
}

fn parse_stop_position(input: &str) -> Option<f32> {
    let trimmed = input.trim();
    if trimmed.ends_with('%') {
        trimmed
            .trim_end_matches('%')
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| (v / 100.0).clamp(0.0, 1.0))
    } else if let Some(px) = parse_css_length_px(trimmed, None) {
        Some((px / 100.0).clamp(0.0, 1.0))
    } else {
        None
    }
}

fn normalize_gradient_stops(stops: &mut Vec<GradientStop>) {
    if stops.is_empty() {
        return;
    }
    if stops.first().is_some_and(|stop| stop.position > 0.0) {
        stops[0].position = 0.0;
    }
    if stops.last().is_some_and(|stop| stop.position < 1.0) {
        if let Some(last) = stops.last_mut() {
            last.position = 1.0;
        }
    }
    stops.sort_by(|a, b| {
        a.position
            .partial_cmp(&b.position)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn parse_css_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let inner = strip_function(trimmed, "url")?;
    let unquoted = inner
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(inner);
    let url = unquoted.trim();
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

fn extract_first_url(input: &str) -> Option<String> {
    let lower = input.to_ascii_lowercase();
    let start = lower.find("url(")?;
    parse_css_url(&input[start..])
}

fn strip_first_function(input: &str, name: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let head = format!("{name}(");
    let Some(start) = lower.find(&head) else {
        return input.to_string();
    };
    if let Some(inner) = strip_function(&input[start..], name) {
        let end = start + head.len() + inner.len() + 1;
        let mut out = String::new();
        out.push_str(&input[..start]);
        if end < input.len() {
            out.push_str(&input[end..]);
        }
        return out;
    }
    input.to_string()
}

fn parse_background_size_list(
    input: &str,
) -> (
    Vec<BackgroundImageFit>,
    Vec<(Option<LengthSpec>, Option<LengthSpec>)>,
) {
    let mut fits = Vec::new();
    let mut lengths = Vec::new();
    for part in split_top_level_commas(input) {
        let (fit, width, height) = parse_background_size_value(&part);
        fits.push(fit);
        lengths.push((width, height));
    }
    (fits, lengths)
}

fn parse_background_size_value(
    input: &str,
) -> (BackgroundImageFit, Option<LengthSpec>, Option<LengthSpec>) {
    let lower = input.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return (BackgroundImageFit::Auto, None, None);
    }
    if lower == "contain" {
        return (BackgroundImageFit::Contain, None, None);
    }
    if lower == "cover" {
        return (BackgroundImageFit::Cover, None, None);
    }
    if lower == "100% 100%" || lower == "100%" {
        return (BackgroundImageFit::Stretch, None, None);
    }
    if lower == "auto" || lower == "auto auto" {
        return (BackgroundImageFit::Auto, None, None);
    }
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.len() >= 2 {
        let width = parse_size_length(tokens[0]);
        let height = parse_size_length(tokens[1]);
        if width.is_some() || height.is_some() {
            return (BackgroundImageFit::Length, width, height);
        }
    } else if tokens.len() == 1
        && let Some(width) = parse_size_length(tokens[0])
    {
        return (BackgroundImageFit::Length, Some(width), None);
    }
    (BackgroundImageFit::Auto, None, None)
}

fn parse_size_length(input: &str) -> Option<LengthSpec> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("auto") {
        return Some(LengthSpec::Auto);
    }
    LengthSpec::parse(trimmed)
}

fn parse_size_after_slash(
    input: &str,
) -> Option<(BackgroundImageFit, Option<LengthSpec>, Option<LengthSpec>)> {
    let slash = input.find('/')?;
    let size_src = input[slash + 1..].trim();
    let size_src = size_src
        .split_whitespace()
        .take_while(|token| !is_repeat_token(token))
        .collect::<Vec<_>>()
        .join(" ");
    if size_src.is_empty() {
        return None;
    }
    Some(parse_background_size_value(&size_src))
}

fn parse_background_position_list(input: &str) -> Vec<BackgroundPosition> {
    split_top_level_commas(input)
        .into_iter()
        .filter_map(|part| parse_background_position_value(&part))
        .collect()
}

fn parse_background_position_value(input: &str) -> Option<BackgroundPosition> {
    let tokens: Vec<String> = split_css_space_tokens(input)
        .into_iter()
        .take_while(|token| token.as_str() != "/" && !is_repeat_token(token))
        .collect();
    if tokens.is_empty() {
        return None;
    }
    match tokens.as_slice() {
        [one] => Some(position_from_one(one)),
        [a, b] => Some(position_from_two(a, b)),
        [a, b, c] => position_from_three(a, b, c),
        [a, b, c, d] => position_from_four(a, b, c, d),
        _ => None,
    }
}

fn parse_position_from_layer_rest(input: &str) -> Option<BackgroundPosition> {
    let before_slash = input.split('/').next().unwrap_or(input);
    parse_background_position_value(before_slash)
}

fn position_from_one(token: &str) -> BackgroundPosition {
    let lower = token.to_ascii_lowercase();
    match lower.as_str() {
        "left" => BackgroundPosition {
            x: LengthSpec::Percent(0.0),
            y: LengthSpec::Percent(50.0),
        },
        "right" => BackgroundPosition {
            x: LengthSpec::Percent(100.0),
            y: LengthSpec::Percent(50.0),
        },
        "top" => BackgroundPosition {
            x: LengthSpec::Percent(50.0),
            y: LengthSpec::Percent(0.0),
        },
        "bottom" => BackgroundPosition {
            x: LengthSpec::Percent(50.0),
            y: LengthSpec::Percent(100.0),
        },
        "center" => BackgroundPosition::center(),
        _ => BackgroundPosition {
            x: parse_size_length(token).unwrap_or(LengthSpec::Percent(0.0)),
            y: LengthSpec::Percent(50.0),
        },
    }
}

fn position_from_two(a: &str, b: &str) -> BackgroundPosition {
    let a_lower = a.to_ascii_lowercase();
    let b_lower = b.to_ascii_lowercase();
    if is_x_keyword(&a_lower) && is_y_keyword(&b_lower) {
        return BackgroundPosition {
            x: keyword_x(&a_lower),
            y: keyword_y(&b_lower),
        };
    }
    if is_y_keyword(&a_lower) && is_x_keyword(&b_lower) {
        return BackgroundPosition {
            x: keyword_x(&b_lower),
            y: keyword_y(&a_lower),
        };
    }
    BackgroundPosition {
        x: parse_size_length(a).unwrap_or(LengthSpec::Percent(0.0)),
        y: parse_size_length(b).unwrap_or(LengthSpec::Percent(0.0)),
    }
}

fn is_x_keyword(token: &str) -> bool {
    matches!(token, "left" | "right" | "center")
}

fn is_y_keyword(token: &str) -> bool {
    matches!(token, "top" | "bottom" | "center")
}

fn keyword_x(token: &str) -> LengthSpec {
    match token {
        "left" => LengthSpec::Percent(0.0),
        "right" => LengthSpec::Percent(100.0),
        _ => LengthSpec::Percent(50.0),
    }
}

fn keyword_y(token: &str) -> LengthSpec {
    match token {
        "top" => LengthSpec::Percent(0.0),
        "bottom" => LengthSpec::Percent(100.0),
        _ => LengthSpec::Percent(50.0),
    }
}

fn is_h_edge(token: &str) -> bool {
    matches!(token, "left" | "right")
}

fn is_v_edge(token: &str) -> bool {
    matches!(token, "top" | "bottom")
}

fn is_h_or_center(token: &str) -> bool {
    is_h_edge(token) || token == "center"
}

fn is_v_or_center(token: &str) -> bool {
    is_v_edge(token) || token == "center"
}

fn is_position_keyword(token: &str) -> bool {
    is_h_or_center(token) || is_v_edge(token)
}

fn invert_from_end(spec: LengthSpec) -> Option<LengthSpec> {
    match spec {
        LengthSpec::Px(v) => Some(LengthSpec::CalcPercentOffset {
            percent: 100.0,
            offset_px: -v,
        }),
        LengthSpec::Percent(p) => Some(LengthSpec::Percent(100.0 - p)),
        LengthSpec::CalcPercentOffset { percent, offset_px } => {
            Some(LengthSpec::CalcPercentOffset {
                percent: 100.0 - percent,
                offset_px: -offset_px,
            })
        }
        _ => None,
    }
}

fn offset_from_edge(edge: &str, offset: &str) -> Option<LengthSpec> {
    let spec = parse_size_length(offset)?;
    if matches!(spec, LengthSpec::Auto) {
        return None;
    }
    match edge {
        "left" | "top" => Some(spec),
        "right" | "bottom" => invert_from_end(spec),
        _ => None,
    }
}

fn position_from_three(a: &str, b: &str, c: &str) -> Option<BackgroundPosition> {
    let a = a.to_ascii_lowercase();
    let b = b.to_ascii_lowercase();
    let c = c.to_ascii_lowercase();
    let a = a.as_str();
    let b = b.as_str();
    let c = c.as_str();
    if is_h_edge(a) && is_v_or_center(c) && !is_position_keyword(b) {
        return Some(BackgroundPosition {
            x: offset_from_edge(a, b)?,
            y: keyword_y(c),
        });
    }
    if is_v_edge(a) && is_h_or_center(c) && !is_position_keyword(b) {
        return Some(BackgroundPosition {
            x: keyword_x(c),
            y: offset_from_edge(a, b)?,
        });
    }
    if is_h_or_center(a) && is_v_edge(b) && !is_position_keyword(c) {
        return Some(BackgroundPosition {
            x: keyword_x(a),
            y: offset_from_edge(b, c)?,
        });
    }
    if is_v_or_center(a) && is_h_edge(b) && !is_position_keyword(c) {
        return Some(BackgroundPosition {
            x: offset_from_edge(b, c)?,
            y: keyword_y(a),
        });
    }
    None
}

fn position_from_four(a: &str, b: &str, c: &str, d: &str) -> Option<BackgroundPosition> {
    let a = a.to_ascii_lowercase();
    let b = b.to_ascii_lowercase();
    let c = c.to_ascii_lowercase();
    let d = d.to_ascii_lowercase();
    let a = a.as_str();
    let b = b.as_str();
    let c = c.as_str();
    let d = d.as_str();
    if is_h_edge(a) && is_v_edge(c) && !is_position_keyword(b) && !is_position_keyword(d) {
        return Some(BackgroundPosition {
            x: offset_from_edge(a, b)?,
            y: offset_from_edge(c, d)?,
        });
    }
    if is_v_edge(a) && is_h_edge(c) && !is_position_keyword(b) && !is_position_keyword(d) {
        return Some(BackgroundPosition {
            x: offset_from_edge(c, d)?,
            y: offset_from_edge(a, b)?,
        });
    }
    None
}

fn parse_background_repeat_list(input: &str) -> Vec<BackgroundRepeat> {
    split_top_level_commas(input)
        .into_iter()
        .filter_map(|part| parse_repeat_from_tokens(&part))
        .collect()
}

fn parse_repeat_from_tokens(input: &str) -> Option<BackgroundRepeat> {
    let tokens: Vec<&str> = input
        .split_whitespace()
        .filter(|token| is_repeat_token(token))
        .collect();
    if tokens
        .iter()
        .any(|token| token.eq_ignore_ascii_case("space"))
    {
        return Some(BackgroundRepeat::Unsupported);
    }
    match tokens.as_slice() {
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

fn is_repeat_token(token: &str) -> bool {
    matches!(
        token.to_ascii_lowercase().as_str(),
        "repeat" | "no-repeat" | "repeat-x" | "repeat-y" | "space" | "round"
    )
}

fn parse_object_fit(input: &str) -> Option<BackgroundImageFit> {
    Some(match input.trim().to_ascii_lowercase().as_str() {
        "contain" => BackgroundImageFit::Contain,
        "cover" => BackgroundImageFit::Cover,
        "fill" => BackgroundImageFit::Stretch,
        "none" => BackgroundImageFit::Auto,
        "scale-down" => BackgroundImageFit::ScaleDown,
        _ => return None,
    })
}

fn strip_function<'a>(input: &'a str, name: &str) -> Option<&'a str> {
    let trimmed = input.trim();
    let head = format!("{name}(");
    if !trimmed.to_ascii_lowercase().starts_with(&head) {
        return None;
    }
    let mut depth = 0i32;
    let mut started = false;
    let mut end = None;
    for (idx, ch) in trimmed.char_indices() {
        match ch {
            '(' => {
                depth += 1;
                started = true;
            }
            ')' if started => {
                depth -= 1;
                if depth == 0 {
                    end = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    Some(trimmed[head.len()..end].trim())
}

fn split_top_level_commas(input: &str) -> Vec<String> {
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
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css_map::LayoutStyleCss;
    use nana_ui_core::LayoutStyle;

    #[test]
    fn linear_gradient_color_list_without_angle() {
        let grad = parse_linear_gradient("linear-gradient(white, transparent)").unwrap();
        assert!((grad.angle_deg - 180.0).abs() < 0.01);
        assert_eq!(grad.stops.len(), 2);
        assert!((grad.stops[0].color[3] - 1.0).abs() < 0.01);
        assert!(grad.stops[1].color[3] < 0.01);
    }

    #[test]
    fn linear_gradient_white_to_transparent_parses() {
        let grad = parse_linear_gradient("linear-gradient(180deg, white, transparent)").unwrap();
        assert!((grad.angle_deg - 180.0).abs() < 0.01);
        assert_eq!(grad.stops.len(), 2);
        assert!((grad.stops[0].color[3] - 1.0).abs() < 0.01);
        assert!(grad.stops[1].color[3] < 0.01);
    }

    #[test]
    fn linear_gradient_dual_color_stop_positions() {
        let grad = parse_linear_gradient("linear-gradient(#ffffff 0 97%, transparent)").unwrap();
        assert_eq!(grad.stops.len(), 3);
        assert!((grad.stops[0].position - 0.0).abs() < 0.01);
        assert!((grad.stops[1].position - 0.97).abs() < 0.01);
        assert!((grad.stops[2].position - 1.0).abs() < 0.01);
        assert!(grad.stops[0].color[0] > 0.9 && grad.stops[1].color[0] > 0.9);
        assert!(grad.stops[2].color[3] < 0.01);
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "background: linear-gradient(#fff 0 97%, transparent)",
            None,
            None,
        );
        assert!(layout.paint.background_image.is_some());
    }

    #[test]
    fn repeating_linear_gradient_extends_stops() {
        let grad = parse_linear_gradient("repeating-linear-gradient(red, blue 25%)").unwrap();
        assert!(grad.stops.len() >= 4);
        assert!((grad.stops[0].position - 0.0).abs() < 0.01);
        assert!((grad.stops.last().unwrap().position - 1.0).abs() < 0.01);
        let same_as_linear = parse_linear_gradient("repeating-linear-gradient(red, blue)").unwrap();
        assert_eq!(same_as_linear.stops.len(), 2);
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "background-image: repeating-linear-gradient(90deg, #000, transparent 25%)",
            None,
            None,
        );
        assert!(
            layout.paint.background_image.is_some(),
            "repeating-linear-gradient must not drop the layer"
        );
        assert!(parse_linear_gradient("repeating-linear-gradient(red 10%, red 10%)").is_none());
        assert!(
            parse_linear_gradient("repeating-linear-gradient(red, blue 10%)").is_none(),
            "period that cannot tile [0,1] in 8 stops must fail closed, not stretch last to 1"
        );
        let mut overflow = LayoutStyle::default();
        overflow.apply_css_text(
            "background-image: repeating-linear-gradient(red, blue 10%)",
            None,
            None,
        );
        assert!(
            overflow.paint.background_image.is_none(),
            "overflowing repeating-linear-gradient must drop the layer"
        );
    }

    #[test]
    fn background_shorthand_gradient_not_dropped() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "background: linear-gradient(to bottom, #ffffff, transparent)",
            None,
            None,
        );
        assert!(layout.paint.background_image.is_some());
        assert!(layout.background.is_none());
    }

    #[test]
    fn mask_image_parses_fade() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "-webkit-mask-image: linear-gradient(180deg, black, transparent)",
            None,
            None,
        );
        assert!(layout.paint.mask.is_some());
    }

    #[test]
    fn clip_path_inset_and_polygon() {
        let inset = parse_clip_path("inset(10px 20px 30px 40px round 5px)").unwrap();
        assert!(matches!(inset, ClipPath::Inset(_)));
        let poly = parse_clip_path("polygon(0% 0%, 100% 0%, 50% 100%)").unwrap();
        assert!(matches!(poly, ClipPath::Polygon(_)));
        let circle = parse_clip_path("circle(50% at 25% 25%)").unwrap();
        match circle {
            ClipPath::Circle(c) => {
                assert!(
                    matches!(c.radius, ClipShapeRadius::Length(LengthSpec::Percent(p)) if (p - 50.0).abs() < 0.01)
                );
                assert_eq!(c.cx, LengthSpec::Percent(25.0));
                assert_eq!(c.cy, LengthSpec::Percent(25.0));
            }
            other => panic!("expected circle, got {other:?}"),
        }
        assert!(matches!(
            parse_clip_path("circle()").unwrap(),
            ClipPath::Circle(_)
        ));
        let ellipse = parse_clip_path("ellipse(40% 20% at 10px 20px)").unwrap();
        assert!(matches!(ellipse, ClipPath::Ellipse(_)));
        assert!(parse_clip_path("path('M0 0')").is_none());
    }

    #[test]
    fn mask_image_url_reuses_background_url_parse() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("mask-image: url(\"hero.png\")", None, None);
        match layout.paint.mask {
            Some(MaskImage::Url(ref url)) => assert_eq!(url, "hero.png"),
            other => panic!("expected mask url, got {other:?}"),
        }
        layout.apply_css_text("mask-image: none", None, None);
        assert!(layout.paint.mask.is_none());
    }

    #[test]
    fn filter_brightness_saturate_contrast() {
        let filter = parse_color_filter("brightness(0.5) saturate(0) contrast(1.2)").unwrap();
        assert!((filter.brightness - 0.5).abs() < 0.01);
        assert!(filter.saturate.abs() < 0.01);
        assert!((filter.contrast - 1.2).abs() < 0.01);
    }

    #[test]
    fn filter_grayscale_encodes_as_saturate() {
        let full = parse_color_filter("grayscale()").unwrap();
        assert!(full.saturate.abs() < 0.01);
        let half = parse_color_filter("grayscale(50%)").unwrap();
        assert!((half.saturate - 0.5).abs() < 0.01);
        let with_brightness = parse_color_filter("brightness(0.8) grayscale(1)").unwrap();
        assert!((with_brightness.brightness - 0.8).abs() < 0.01);
        assert!(with_brightness.saturate.abs() < 0.01);
        assert!(parse_color_filter("grayscale(0)").is_none());
        assert!(parse_color_filter("grayscale() sepia()").is_none());
        assert!(parse_color_filter("grayscale(1) saturate(2)").is_none());
        assert!(parse_color_filter("saturate(2) grayscale(50%)").is_none());
        assert!(parse_color_filter("grayscale(1)saturate(2)").is_none());
        assert!(parse_color_filter("brightness(0.8) saturate(1.2) grayscale(1)").is_none());
    }

    #[test]
    fn backdrop_filter_blur_and_saturate() {
        let filter = parse_backdrop_filter("blur(12px) saturate(1.5)").unwrap();
        assert!((filter.blur_radius - 12.0).abs() < 0.01);
        assert!((filter.saturate - 1.5).abs() < 0.01);
    }

    #[test]
    fn backdrop_filter_ignores_unknown_functions() {
        let filter = parse_backdrop_filter("blur(8px) drop-shadow(0 0 4px black)").unwrap();
        assert!((filter.blur_radius - 8.0).abs() < 0.01);
    }

    #[test]
    fn backdrop_filter_clamps_huge_blur() {
        let filter = parse_backdrop_filter("blur(200px)").unwrap();
        assert!((filter.blur_radius - BackdropFilter::MAX_BLUR_RADIUS).abs() < 0.01);
    }

    #[test]
    fn backdrop_filter_css_property_applies() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("backdrop-filter: blur(10px)", None, None);
        assert!(layout.paint.backdrop_filter.is_some());
    }

    #[test]
    fn background_image_url_accepts_http_file_and_relative() {
        for css in [
            "url(\"http://example.com/a.png\")",
            "url(\"https://example.com/a.png\")",
            "url('file:///tmp/test.png')",
            "url(test.png)",
            "url(\"./assets/icon.png\")",
            "url(\"icons/mark.svg\")",
        ] {
            let image = parse_background_image_value(css).unwrap();
            match image {
                BackgroundImage::Url { ref url, .. } => assert!(!url.is_empty(), "{css}"),
                other => panic!("expected url background for {css}, got {other:?}"),
            }
        }
    }

    #[test]
    fn radial_gradient_circle_at_center_parses() {
        let grad = parse_radial_gradient("radial-gradient(circle at center, red, blue)").unwrap();
        assert!(grad.circle);
        assert_eq!(grad.center[0], LengthSpec::Percent(50.0));
        assert_eq!(grad.center[1], LengthSpec::Percent(50.0));
        assert_eq!(grad.stops.len(), 2);
    }

    #[test]
    fn mask_radial_px_center_uses_length_not_div100() {
        let grad =
            parse_radial_gradient("radial-gradient(circle at 10px 20px, black, transparent)")
                .unwrap();
        assert_eq!(grad.center[0], LengthSpec::Px(10.0));
        assert_eq!(grad.center[1], LengthSpec::Px(20.0));
        let used = grad.resolved_center(200.0, 100.0).unwrap();
        assert!((used[0] - 0.05).abs() < 1e-5, "got {}", used[0]);
        assert!((used[1] - 0.20).abs() < 1e-5, "got {}", used[1]);
        assert!(grad.resolved_center(0.0, 100.0).is_none());
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "mask-image: radial-gradient(circle at 10px 20px, black, transparent)",
            None,
            None,
        );
        match layout.paint.mask {
            Some(MaskImage::Gradient(CssGradient::Radial(ref radial))) => {
                assert_eq!(radial.center[0], LengthSpec::Px(10.0));
                assert_eq!(radial.center[1], LengthSpec::Px(20.0));
            }
            other => panic!("expected radial mask, got {other:?}"),
        }
    }

    #[test]
    fn background_image_url_rejects_empty() {
        assert!(parse_background_image_value("url(\"\")").is_none());
    }

    #[test]
    fn background_image_relative_url_applies_via_css() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("background-image: url(\"hero.png\")", None, None);
        match layout.paint.background_image {
            Some(BackgroundImage::Url { ref url, .. }) => assert_eq!(url, "hero.png"),
            other => panic!("expected relative url, got {other:?}"),
        }
    }

    #[test]
    fn background_image_data_url() {
        let image =
            parse_background_image_value("url(\"data:image/png;base64,iVBORw0KGgo=\")").unwrap();
        match image {
            BackgroundImage::Url { ref url, fit, .. } => {
                assert!(url.starts_with("data:image/png;base64,"));
                assert_eq!(fit, BackgroundImageFit::Auto);
            }
            other => panic!("expected data url background, got {other:?}"),
        }
    }

    #[test]
    fn background_image_two_layers_keep_css_order() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "background-image: url(\"a.png\"), url(\"b.png\")",
            None,
            None,
        );
        match layout.paint.background_image {
            Some(BackgroundImage::Url { ref url, .. }) => assert_eq!(url, "a.png"),
            other => panic!("expected first url, got {other:?}"),
        }
        assert_eq!(layout.paint.background_layers.len(), 1);
        match &layout.paint.background_layers[0] {
            BackgroundImage::Url { url, .. } => assert_eq!(url, "b.png"),
            other => panic!("expected second url, got {other:?}"),
        }
    }

    #[test]
    fn background_size_auto_and_px_and_repeat_position() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "background-image: url(\"tile.png\"); background-size: 32px 16px; background-repeat: repeat-x; background-position: 10px 20px",
            None,
            None,
        );
        match layout.paint.background_image {
            Some(BackgroundImage::Url {
                fit,
                size_width,
                size_height,
                position,
                repeat,
                ..
            }) => {
                assert_eq!(fit, BackgroundImageFit::Length);
                assert_eq!(size_width, Some(LengthSpec::Px(32.0)));
                assert_eq!(size_height, Some(LengthSpec::Px(16.0)));
                assert_eq!(position.x, LengthSpec::Px(10.0));
                assert_eq!(position.y, LengthSpec::Px(20.0));
                assert_eq!(repeat, BackgroundRepeat::RepeatX);
            }
            other => panic!("expected url placement, got {other:?}"),
        }
        let mut auto = LayoutStyle::default();
        auto.apply_css_text(
            "background-image: url(a.png); background-size: auto",
            None,
            None,
        );
        match auto.paint.background_image {
            Some(BackgroundImage::Url { fit, .. }) => assert_eq!(fit, BackgroundImageFit::Auto),
            other => panic!("expected auto size, got {other:?}"),
        }
    }

    #[test]
    fn background_position_three_value_with_calc_is_not_xy_pair() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "background-image: url(a.png); background-position: right calc(8px) center",
            None,
            None,
        );
        match layout.paint.background_image {
            Some(BackgroundImage::Url { position, .. }) => {
                assert_eq!(
                    position.x,
                    LengthSpec::CalcPercentOffset {
                        percent: 100.0,
                        offset_px: -8.0,
                    }
                );
                assert_eq!(position.y, LengthSpec::Percent(50.0));
            }
            other => panic!("expected positioned url, got {other:?}"),
        }
        assert_eq!(
            layout.paint.background_position_list,
            vec![BackgroundPosition {
                x: LengthSpec::CalcPercentOffset {
                    percent: 100.0,
                    offset_px: -8.0,
                },
                y: LengthSpec::Percent(50.0),
            }]
        );
    }

    #[test]
    fn background_position_four_value_and_fail_closed_five() {
        let mut ok = LayoutStyle::default();
        ok.apply_css_text(
            "background-image: url(a.png); background-position: left 10px top 20px",
            None,
            None,
        );
        match ok.paint.background_image {
            Some(BackgroundImage::Url { position, .. }) => {
                assert_eq!(position.x, LengthSpec::Px(10.0));
                assert_eq!(position.y, LengthSpec::Px(20.0));
            }
            other => panic!("expected 4-value position, got {other:?}"),
        }

        let mut stale = LayoutStyle::default();
        stale.apply_css_text(
            "background-image: url(a.png); background-position: 4px 8px",
            None,
            None,
        );
        stale.apply_css_text("background-position: left 1px top 2px extra", None, None);
        match stale.paint.background_image {
            Some(BackgroundImage::Url { position, .. }) => {
                assert_eq!(
                    position.x,
                    LengthSpec::Px(4.0),
                    "must not fake a 5-token xy"
                );
                assert_eq!(position.y, LengthSpec::Px(8.0));
            }
            other => panic!("expected previous layer position, got {other:?}"),
        }
        assert_eq!(
            stale.paint.background_position_list,
            vec![BackgroundPosition {
                x: LengthSpec::Px(4.0),
                y: LengthSpec::Px(8.0),
            }]
        );
    }

    #[test]
    fn object_fit_contain_and_img_src_bind() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("object-fit: contain; object-position: left top", None, None);
        apply_img_replaced_content(&mut layout, "photo.png");
        match layout.paint.content_image {
            Some(BackgroundImage::Url {
                ref url,
                fit,
                position,
                repeat,
                ..
            }) => {
                assert_eq!(url, "photo.png");
                assert_eq!(fit, BackgroundImageFit::Contain);
                assert_eq!(position.x, LengthSpec::Percent(0.0));
                assert_eq!(position.y, LengthSpec::Percent(0.0));
                assert_eq!(repeat, BackgroundRepeat::NoRepeat);
            }
            other => panic!("expected img content, got {other:?}"),
        }
    }

    #[test]
    fn object_fit_scale_down_is_not_silent_contain() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("object-fit: scale-down", None, None);
        assert_eq!(layout.paint.object_fit, Some(BackgroundImageFit::ScaleDown));
        apply_img_replaced_content(&mut layout, "photo.png");
        match layout.paint.content_image {
            Some(BackgroundImage::Url { fit, .. }) => {
                assert_eq!(fit, BackgroundImageFit::ScaleDown);
            }
            other => panic!("expected scale-down img, got {other:?}"),
        }
    }

    #[test]
    fn video_poster_and_iframe_skip_fail_closed() {
        let mut poster = LayoutStyle::default();
        apply_video_poster(&mut poster, "still.png", false);
        match poster.paint.content_image {
            Some(BackgroundImage::Url { ref url, .. }) => assert_eq!(url, "still.png"),
            other => panic!("expected poster, got {other:?}"),
        }
        assert!(poster.paint.skipped_replaced.is_none());

        let mut video = LayoutStyle::default();
        apply_video_poster(&mut video, "", false);
        assert!(video.paint.content_image.is_none());
        assert_eq!(video.paint.skipped_replaced.as_deref(), Some("video"));

        let mut slotted = LayoutStyle::default();
        apply_video_poster(&mut slotted, "still.png", true);
        assert!(
            slotted.paint.content_image.is_none(),
            "HostTexture video must not also paint poster"
        );
        assert!(slotted.paint.skipped_replaced.is_none());

        let mut iframe = LayoutStyle::default();
        apply_iframe_skip(&mut iframe);
        assert!(iframe.paint.content_image.is_none());
        assert_eq!(iframe.paint.skipped_replaced.as_deref(), Some("iframe"));
    }

    #[test]
    fn canvas_without_slot_is_not_a_2d_bitmap() {
        let mut bare = LayoutStyle::default();
        bare.paint.content_image = Some(BackgroundImage::url_with_fit(
            "frame.png",
            BackgroundImageFit::Stretch,
        ));
        apply_canvas_skip(&mut bare, false);
        assert!(
            bare.paint.content_image.is_none(),
            "bare <canvas> must not keep a pixmap on content_image"
        );
        assert_eq!(bare.paint.skipped_replaced.as_deref(), Some("canvas"));

        let mut slotted = LayoutStyle::default();
        slotted.paint.content_image = Some(BackgroundImage::url_with_fit(
            "frame.png",
            BackgroundImageFit::Stretch,
        ));
        apply_canvas_skip(&mut slotted, true);
        assert!(
            slotted.paint.content_image.is_none(),
            "HostTexture canvas still must not pretend to be content_image"
        );
        assert!(slotted.paint.skipped_replaced.is_none());
    }

    #[test]
    fn unspecified_url_and_size_defaults_to_css_repeat_and_auto() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "background-image: url(\"tile.png\"); background-size: 32px",
            None,
            None,
        );
        match layout.paint.background_image {
            Some(BackgroundImage::Url {
                fit,
                size_width,
                repeat,
                ..
            }) => {
                assert_eq!(fit, BackgroundImageFit::Length);
                assert_eq!(size_width, Some(LengthSpec::Px(32.0)));
                assert_eq!(
                    repeat,
                    BackgroundRepeat::Repeat,
                    "CSS initial background-repeat is repeat"
                );
            }
            other => panic!("expected sized url, got {other:?}"),
        }
    }

    #[test]
    fn background_repeat_space_is_not_silent_repeat() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "background-image: url(\"tile.png\"); background-repeat: space",
            None,
            None,
        );
        match layout.paint.background_image {
            Some(BackgroundImage::Url { repeat, .. }) => {
                assert_eq!(repeat, BackgroundRepeat::Unsupported);
            }
            other => panic!("expected space url layer, got {other:?}"),
        }
        layout.apply_css_text("background-repeat: round", None, None);
        match layout.paint.background_image {
            Some(BackgroundImage::Url { repeat, .. }) => {
                assert_eq!(repeat, BackgroundRepeat::Round);
            }
            other => panic!("expected round url layer, got {other:?}"),
        }
    }

    #[test]
    fn background_shorthand_resets_placement_lists() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "background-repeat: no-repeat; background-size: cover; background-position: 10px 10px",
            None,
            None,
        );
        assert!(!layout.paint.background_repeat_list.is_empty());
        layout.apply_css_text("background: url(\"a.png\")", None, None);
        assert!(
            layout.paint.background_repeat_list.is_empty()
                && layout.paint.background_size_list.is_empty()
                && layout.paint.background_position_list.is_empty(),
            "shorthand must reset leftover longhand lists"
        );
        match &layout.paint.background_image {
            Some(BackgroundImage::Url { repeat, fit, .. }) => {
                assert_eq!(*repeat, BackgroundRepeat::Repeat);
                assert_eq!(*fit, BackgroundImageFit::Auto);
            }
            other => panic!("expected shorthand url, got {other:?}"),
        }
        layout.apply_css_text("background-image: url(\"b.png\")", None, None);
        match layout.paint.background_image {
            Some(BackgroundImage::Url { repeat, fit, .. }) => {
                assert_eq!(
                    repeat,
                    BackgroundRepeat::Repeat,
                    "later background-image must not zip pre-shorthand no-repeat"
                );
                assert_eq!(fit, BackgroundImageFit::Auto);
            }
            other => panic!("expected reset url, got {other:?}"),
        }
    }

    #[test]
    fn background_shorthand_color_and_two_url_layers() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "background: url(\"fg.png\") center / contain no-repeat, url(\"bg.png\") repeat, #112233",
            None,
            None,
        );
        assert!(layout.background.is_some());
        match layout.paint.background_image {
            Some(BackgroundImage::Url {
                ref url,
                fit,
                repeat,
                ..
            }) => {
                assert_eq!(url, "fg.png");
                assert_eq!(fit, BackgroundImageFit::Contain);
                assert_eq!(repeat, BackgroundRepeat::NoRepeat);
            }
            other => panic!("expected fg url, got {other:?}"),
        }
        match layout.paint.background_layers.first() {
            Some(BackgroundImage::Url { url, repeat, .. }) => {
                assert_eq!(url, "bg.png");
                assert_eq!(*repeat, BackgroundRepeat::Repeat);
            }
            other => panic!("expected bg url layer, got {other:?}"),
        }
    }

    #[test]
    fn filter_hue_rotate_and_element_blur() {
        let filter = parse_color_filter("hue-rotate(90deg) blur(8px)").unwrap();
        assert!((filter.hue_rotate_deg - 90.0).abs() < 0.01);
        assert!((filter.blur_radius - 8.0).abs() < 0.01);
        let turn = parse_color_filter("hue-rotate(0.25turn)").unwrap();
        assert!((turn.hue_rotate_deg - 90.0).abs() < 0.01);
    }

    #[test]
    fn filter_clamps_element_blur() {
        let filter = parse_color_filter("blur(200px)").unwrap();
        assert!((filter.blur_radius - ColorFilter::MAX_BLUR_RADIUS).abs() < 0.01);
    }

    #[test]
    fn filter_drop_shadow_parses_offset_blur_and_color() {
        let filter = parse_color_filter("drop-shadow(4px 6px 8px rgba(0, 0, 0, 0.5))").unwrap();
        let shadow = filter.drop_shadow.expect("drop-shadow");
        assert!((shadow.offset_x - 4.0).abs() < 0.01);
        assert!((shadow.offset_y - 6.0).abs() < 0.01);
        assert!((shadow.blur_radius - 8.0).abs() < 0.01);
        assert!((shadow.color[3] - 0.5).abs() < 0.01);
        let colored = parse_color_filter("drop-shadow(red 2px 3px)").unwrap();
        let red = colored.drop_shadow.expect("color-first");
        assert!((red.offset_x - 2.0).abs() < 0.01);
        assert!((red.color[0] - 1.0).abs() < 0.01);
        let combined = parse_color_filter("brightness(0.5) drop-shadow(0 4px 4px black)").unwrap();
        assert!((combined.brightness - 0.5).abs() < 0.01);
        assert!(combined.drop_shadow.is_some());
        let glued = parse_color_filter("brightness(0.5)drop-shadow(0 4px 4px black)").unwrap();
        assert!((glued.brightness - 0.5).abs() < 0.01);
        assert!(glued.drop_shadow.is_some());
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("filter: drop-shadow(2px 4px 4px red)", None, None);
        assert!(layout.paint.filter.unwrap().drop_shadow.is_some());
    }

    #[test]
    fn filter_drop_shadow_clamps_blur_and_rejects_spread() {
        let filter = parse_color_filter("drop-shadow(0 0 200px black)").unwrap();
        assert!(
            (filter.drop_shadow.unwrap().blur_radius - ColorFilter::MAX_BLUR_RADIUS).abs() < 0.01
        );
        assert!(parse_color_filter("drop-shadow(0 0 4px 2px black)").is_none());
        assert!(parse_color_filter("drop-shadow(inset 0 0 4px black)").is_none());
        assert!(
            parse_color_filter("drop-shadow(0 0 4px black) drop-shadow(1px 1px black)").is_none()
        );
    }

    #[test]
    fn filter_unknown_functions_fail_closed() {
        assert!(parse_color_filter("sepia()").is_none());
        assert!(parse_color_filter("brightness(0.5) sepia()").is_none());
        assert!(parse_color_filter("brightness(0.5)sepia()").is_none());
        assert!(parse_color_filter("brightness(0.5)\tsepia()").is_none());
        assert!(parse_color_filter("brightness(0.5)\nsepia()").is_none());
        assert!(parse_color_filter("url(#svg-filter)").is_none());
        assert!(parse_color_filter("brightness(0.5) url(#svg-filter)").is_none());
        assert!(parse_color_filter("brightness(0.5)url(#svg-filter)").is_none());
        assert!(parse_color_filter("drop-shadow(0 0 4px black) sepia()").is_none());
        assert!(parse_color_filter("blur(200px) sepia()").is_none());
    }

    #[test]
    fn filter_invert_and_opacity_use_existing_slots() {
        let invert = parse_color_filter("invert()").unwrap();
        assert!((invert.invert - 1.0).abs() < 0.01);
        let half = parse_color_filter("invert(50%) brightness(0.8)").unwrap();
        assert!((half.invert - 0.5).abs() < 0.01);
        assert!((half.brightness - 0.8).abs() < 0.01);
        let fade = parse_color_filter("opacity(0.25)").unwrap();
        assert!((fade.opacity - 0.25).abs() < 0.01);
        assert!(parse_color_filter("opacity()").is_none());
        assert!(parse_color_filter("invert(0)").is_none());
        assert!(parse_color_filter("grayscale() saturate(2)").is_none());
    }

    #[test]
    fn filter_empty_none_and_identity_are_unused() {
        assert!(parse_color_filter("").is_none());
        assert!(parse_color_filter("   ").is_none());
        assert!(parse_color_filter("none").is_none());
        assert!(parse_color_filter("NONE").is_none());
        assert!(parse_color_filter("brightness(1)").is_none());
        assert!(parse_color_filter("brightness(1) saturate(1) contrast(1)").is_none());
        assert!(parse_color_filter("hue-rotate(0deg) blur(0)").is_none());
    }

    #[test]
    fn outline_is_paint_only_stroke() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("outline: 2px solid red", None, None);
        assert!(layout.paint.outline.is_active());
        assert!((layout.paint.outline.width - 2.0).abs() < 0.01);
        assert!(layout.border_width.is_none());
        layout.apply_css_text("outline-style: none", None, None);
        assert!(!layout.paint.outline.is_active());
    }

    #[test]
    fn mix_blend_mode_subset_fails_closed() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("mix-blend-mode: multiply", None, None);
        assert_eq!(layout.paint.mix_blend, MixBlendMode::Multiply);
        layout.apply_css_text("mix-blend-mode: overlay", None, None);
        assert_eq!(
            layout.paint.mix_blend,
            MixBlendMode::Multiply,
            "unknown modes must fail closed"
        );
        layout.apply_css_text("mix-blend-mode: screen", None, None);
        assert_eq!(layout.paint.mix_blend, MixBlendMode::Screen);
        layout.apply_css_text("mix-blend-mode: normal", None, None);
        assert!(layout.paint.mix_blend.is_normal());
    }

    #[test]
    fn line_clamp_enables_ellipsis_and_wrap() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("-webkit-line-clamp: 2", None, None);
        assert_eq!(layout.line_clamp, Some(2));
        assert!(layout.uses_text_ellipsis());
        assert!(!layout.white_space_nowrap);
        assert_eq!(layout.overflow_x, OverflowSpec::Hidden);
        assert_eq!(layout.overflow_y, OverflowSpec::Hidden);
        layout.apply_css_text("line-clamp: none", None, None);
        assert!(layout.line_clamp.is_none());
    }

    #[test]
    fn text_decoration_underline_and_line_through() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("text-decoration: underline", None, None);
        let deco = layout.text_decoration.expect("underline");
        assert!(deco.underline);
        assert!(!deco.line_through);
        layout.apply_css_text("text-decoration: underline line-through", None, None);
        let deco = layout.text_decoration.expect("both");
        assert!(deco.underline && deco.line_through);
        layout.apply_css_text("text-decoration: none", None, None);
        assert_eq!(layout.text_decoration, Some(TextDecorationLine::default()));
        layout.apply_css_text("text-decoration: overline", None, None);
        assert_eq!(
            layout.text_decoration,
            Some(TextDecorationLine::default()),
            "overline fails closed (Scene stroke is underline / line-through only)"
        );
    }

    #[test]
    fn font_feature_settings_parse_and_variation_fails_closed() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("font-feature-settings: \"liga\" 1, 'kern' off", None, None);
        let features = layout.font_features.as_ref().expect("features");
        assert_eq!(features.len(), 2);
        assert_eq!(features[0].tag, *b"liga");
        assert_eq!(features[0].value, 1);
        assert_eq!(features[1].tag, *b"kern");
        assert_eq!(features[1].value, 0);
        layout.apply_css_text("font-feature-settings: normal", None, None);
        assert_eq!(layout.font_features.as_deref(), Some(&[][..]));

        layout.apply_css_text("font-variation-settings: \"wght\" 700", None, None);
        assert!(!layout.unsupported_font_variation);
        assert_eq!(layout.font_weight, Some(700));
        layout.apply_css_text("font-variation-settings: \"BEVL\" 1", None, None);
        assert!(layout.unsupported_font_variation);
        assert_eq!(
            layout.font_weight,
            Some(700),
            "BEVL must not remap onto font-weight / wght"
        );
        layout.apply_css_text(
            "font-variation-settings: \"wght\" 400, \"wdth\" 100",
            None,
            None,
        );
        assert!(layout.unsupported_font_variation);
        assert_eq!(layout.font_weight, Some(400));
        layout.apply_css_text("font-variation-settings: normal", None, None);
        assert!(!layout.unsupported_font_variation);
    }

    #[test]
    fn pointer_events_none_and_auto() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("pointer-events: none", None, None);
        assert_eq!(layout.pointer_events, Some(PointerEventsSpec::None));
        layout.apply_css_text("pointer-events: auto", None, None);
        assert_eq!(layout.pointer_events, Some(PointerEventsSpec::Auto));
        layout.apply_css_text("pointer-events: inherit", None, None);
        assert_eq!(layout.pointer_events, None);
        layout.apply_css_text("pointer-events: none", None, None);
        layout.apply_css_text("pointer-events: visiblePainted", None, None);
        assert_eq!(
            layout.pointer_events,
            Some(PointerEventsSpec::None),
            "unknown values fail closed and keep the last specified value"
        );
    }

    #[test]
    fn border_image_url_slice_fill_is_supported() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("border-image: url(\"frame.png\") 30 fill", None, None);
        assert!(!layout.paint.unsupported_border_image);
        let spec = layout.paint.border_image.as_ref().expect("border-image");
        assert_eq!(spec.source.url_str(), Some("frame.png"));
        assert!(spec.fill);
        assert_eq!(spec.slice, [BorderImageSlice::Number(30.0); 4]);
        assert!(layout.paint.background_image.is_none());
        layout.apply_css_text("border-image: none", None, None);
        assert!(layout.paint.border_image.is_none());
        assert!(!layout.paint.unsupported_border_image);
    }

    #[test]
    fn border_image_linear_gradient_slice_parses() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "border-image: linear-gradient(red, blue) 20% fill",
            None,
            None,
        );
        let spec = layout.paint.border_image.as_ref().expect("gradient");
        assert!(matches!(
            spec.source,
            BackgroundImage::Gradient(CssGradient::Linear(_))
        ));
        assert_eq!(spec.slice, [BorderImageSlice::Percent(20.0); 4]);
        assert!(spec.fill);
        assert!(!layout.paint.unsupported_border_image);
    }

    #[test]
    fn border_image_repeat_and_radial_fail_closed() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("border-image: url(\"frame.png\") 30 round", None, None);
        assert!(layout.paint.unsupported_border_image);
        assert!(layout.paint.border_image.is_none());
        layout.apply_css_text("border-image: none", None, None);
        layout.apply_css_text(
            "border-image: radial-gradient(circle, red, blue) 30 fill",
            None,
            None,
        );
        assert!(layout.paint.unsupported_border_image);
        assert!(layout.paint.border_image.is_none());
    }

    #[test]
    fn border_image_repeat_before_source_stays_unsupported() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "border-image-repeat: round; border-image-source: url(\"frame.png\"); border-image-slice: 30 fill",
            None,
            None,
        );
        assert!(layout.paint.unsupported_border_image);
        assert!(
            layout.paint.border_image.is_none(),
            "later source/slice must not install a stretch 9-slice"
        );
    }

    #[test]
    fn border_image_width_before_source_stays_unsupported() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "border-image-width: 2; border-image-source: url(\"frame.png\"); border-image-slice: 30 fill",
            None,
            None,
        );
        assert!(layout.paint.unsupported_border_image);
        assert!(layout.paint.border_image.is_none());
    }

    #[test]
    fn border_image_outset_before_source_stays_unsupported() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "border-image-outset: 4px; border-image-source: url(\"frame.png\"); border-image-slice: 30 fill",
            None,
            None,
        );
        assert!(layout.paint.unsupported_border_image);
        assert!(layout.paint.border_image.is_none());
    }

    #[test]
    fn box_shadow_inset_and_multiple_layers() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "box-shadow: inset 2px 2px 4px black, 0 4px 8px rgba(0,0,0,0.5), 0 1px 0 red, 0 2px 0 blue, 0 3px 0 green",
            None,
            None,
        );
        assert_eq!(layout.paint.box_shadows.len(), 4);
        assert!(layout.paint.box_shadows[0].inset);
        assert!(!layout.paint.box_shadows[1].inset);
    }
}
