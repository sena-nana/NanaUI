//! CSS paint property parsers (`linear-gradient`, `url()`, `clip-path`, `filter`).

use nana_ui_core::box_layout::{
    BackdropFilter, BackgroundImage, BackgroundImageFit, ClipInset, ClipPath, ClipPoint,
    ColorFilter, CssGradient, GradientStop, LengthSpec, LinearGradient, RadialGradient,
};

use crate::css_map::{CssLayoutParse, parse_css_length_px, resolve_paint_color};

const MAX_GRADIENT_STOPS: usize = 8;

/// Apply paint longhands/shorthands onto [`LayoutStyle`](nana_ui_core::LayoutStyle).
pub fn apply_css_paint_property(style: &mut nana_ui_core::LayoutStyle, name: &str, val: &str) {
    match name {
        "background" => apply_background_shorthand(style, val),
        "background-image" => apply_background_image(style, val),
        "background-size" => apply_background_size(style, val),
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
        _ => {}
    }
}

fn apply_background_shorthand(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let trimmed = val.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        style.background = None;
        style.paint.background_image = None;
        return;
    }
    if let Some(image) = parse_background_image_value(trimmed) {
        style.paint.background_image = Some(image);
        return;
    }
    if let Some(c) = resolve_paint_color(trimmed) {
        style.background = Some(c);
        style.paint.background_image = None;
    }
}

fn apply_background_image(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let trimmed = val.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        style.paint.background_image = None;
        return;
    }
    if let Some(image) = parse_background_image_value(trimmed) {
        style.paint.background_image = Some(image);
    }
}

fn apply_background_size(style: &mut nana_ui_core::LayoutStyle, val: &str) {
    let fit = parse_background_size_fit(val);
    if let Some(BackgroundImage::Url { fit: slot, .. }) = style.paint.background_image.as_mut() {
        *slot = fit;
    }
}

fn parse_background_image_value(input: &str) -> Option<BackgroundImage> {
    let trimmed = input.trim();
    if let Some(grad) = parse_linear_gradient(trimmed) {
        return Some(BackgroundImage::Gradient(CssGradient::Linear(grad)));
    }
    if let Some(grad) = parse_radial_gradient(trimmed) {
        return Some(BackgroundImage::Gradient(CssGradient::Radial(grad)));
    }
    parse_css_url(trimmed).map(|url| BackgroundImage::Url {
        url,
        fit: BackgroundImageFit::Cover,
    })
}

pub fn parse_linear_gradient(input: &str) -> Option<LinearGradient> {
    let inner = strip_function(input, "linear-gradient")?;
    let (angle_deg, stops_src) = split_gradient_header(inner)?;
    let stops = parse_gradient_stops(stops_src)?;
    if stops.is_empty() {
        return None;
    }
    Some(LinearGradient { angle_deg, stops })
}

fn parse_mask_image(input: &str) -> Option<CssGradient> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return None;
    }
    parse_linear_gradient(trimmed)
        .map(CssGradient::Linear)
        .or_else(|| parse_radial_gradient(trimmed).map(CssGradient::Radial))
}

pub fn parse_radial_gradient(input: &str) -> Option<RadialGradient> {
    let inner = strip_function(input, "radial-gradient")?;
    let (circle, center, stops_src) = split_radial_header(inner)?;
    let stops = parse_gradient_stops(stops_src)?;
    if stops.is_empty() {
        return None;
    }
    Some(RadialGradient {
        circle,
        center,
        stops,
    })
}

fn split_radial_header(input: &str) -> Option<(bool, [f32; 2], &str)> {
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
    } else if resolve_paint_color(head).is_some() {
        return Some((circle, [0.5, 0.5], trimmed));
    } else {
        [0.5, 0.5]
    };
    Some((circle, center, tail))
}

fn parse_radial_center(input: &str) -> Option<[f32; 2]> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("center") {
        return Some([0.5, 0.5]);
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() == 2 {
        let x = parse_radial_center_axis(parts[0], true)?;
        let y = parse_radial_center_axis(parts[1], false)?;
        return Some([x, y]);
    }
    if parts.len() == 1 {
        let x = parse_radial_center_axis(parts[0], true)?;
        return Some([x, 0.5]);
    }
    None
}

fn parse_radial_center_axis(token: &str, horizontal: bool) -> Option<f32> {
    let lower = token.to_ascii_lowercase();
    match lower.as_str() {
        "left" if horizontal => Some(0.0),
        "right" if horizontal => Some(1.0),
        "top" if !horizontal => Some(0.0),
        "bottom" if !horizontal => Some(1.0),
        "center" => Some(0.5),
        _ => {
            if lower.ends_with('%') {
                lower
                    .trim_end_matches('%')
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .map(|v| (v / 100.0).clamp(0.0, 1.0))
            } else if let Some(px) = parse_css_length_px(token, None) {
                Some((px / 100.0).clamp(0.0, 1.0))
            } else {
                None
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
    None
}

pub fn parse_color_filter(input: &str) -> Option<ColorFilter> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return None;
    }
    let mut filter = ColorFilter::default();
    for token in split_filter_tokens(trimmed) {
        if let Some(value) = strip_function(&token, "brightness") {
            filter.brightness = parse_filter_scalar(value).unwrap_or(1.0);
        } else if let Some(value) = strip_function(&token, "saturate") {
            filter.saturate = parse_filter_scalar(value).unwrap_or(1.0);
        } else if let Some(value) = strip_function(&token, "contrast") {
            filter.contrast = parse_filter_scalar(value).unwrap_or(1.0);
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

fn split_filter_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ' ' if depth == 0 => {
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
        if resolve_paint_color(head).is_some() {
            return Some((180.0, trimmed));
        }
        return None;
    }
    if resolve_paint_color(trimmed).is_some() {
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

fn parse_gradient_stops(input: &str) -> Option<Vec<GradientStop>> {
    let parts = split_top_level_commas(input);
    let mut stops = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (color_src, position) = if let Some(space) = find_stop_position(trimmed) {
            (
                trimmed[..space].trim(),
                parse_stop_position(&trimmed[space..])?,
            )
        } else {
            let position = if stops.is_empty() {
                0.0
            } else if index == parts.len() - 1 {
                1.0
            } else {
                index as f32 / (parts.len().saturating_sub(1) as f32)
            };
            (trimmed, position)
        };
        let color = resolve_paint_color(color_src)?;
        stops.push(GradientStop { position, color });
        if stops.len() >= MAX_GRADIENT_STOPS {
            break;
        }
    }
    normalize_gradient_stops(&mut stops);
    if stops.is_empty() { None } else { Some(stops) }
}

fn find_stop_position(input: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (idx, ch) in input.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => depth -= 1,
            ' ' if depth == 0 => return Some(idx),
            _ => {}
        }
    }
    None
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
        Some(px / 100.0)
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

fn parse_background_size_fit(input: &str) -> BackgroundImageFit {
    let lower = input.trim().to_ascii_lowercase();
    if lower == "contain" {
        BackgroundImageFit::Contain
    } else if lower == "100% 100%" || lower == "100%" {
        BackgroundImageFit::Stretch
    } else {
        BackgroundImageFit::Cover
    }
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
    }

    #[test]
    fn filter_brightness_saturate_contrast() {
        let filter = parse_color_filter("brightness(0.5) saturate(0) contrast(1.2)").unwrap();
        assert!((filter.brightness - 0.5).abs() < 0.01);
        assert!(filter.saturate.abs() < 0.01);
        assert!((filter.contrast - 1.2).abs() < 0.01);
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
        assert!((grad.center[0] - 0.5).abs() < 0.01);
        assert!((grad.center[1] - 0.5).abs() < 0.01);
        assert_eq!(grad.stops.len(), 2);
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
            BackgroundImage::Url { ref url, fit } => {
                assert!(url.starts_with("data:image/png;base64,"));
                assert_eq!(fit, BackgroundImageFit::Cover);
            }
            other => panic!("expected data url background, got {other:?}"),
        }
    }
}
