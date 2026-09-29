//! CSS motion values: time lists, timing functions and the `transition`
//! shorthand, as strings and Style Model easings. Shared by the Vue path,
//! which compiles them into animation tracks at run time, and the `.vue`
//! compiler, which compiles `transition` into implicit animations.

use std::time::Duration;

use nana_ui_core::Easing;

pub struct TransitionShorthand {
    pub property: String,
    pub duration: String,
    pub timing_function: String,
    pub delay: String,
}

pub fn parse_transition_shorthand(raw: &str) -> Option<TransitionShorthand> {
    let mut items = split_css_comma_list(raw);
    if items.is_empty() {
        items = vec![raw.to_string()];
    }
    let mut properties = Vec::new();
    let mut durations = Vec::new();
    let mut timings = Vec::new();
    let mut delays = Vec::new();
    for item in items {
        // CSS defaults a missing `transition-duration` to `0s` **per item**.
        // Failing the item — and with `?`, the whole list — meant
        // `transition: opacity, transform 200ms` lost the transform transition
        // as well as the opacity one.
        let parsed = parse_transition_item(&item);
        properties.push(parsed.property);
        durations.push(parsed.duration);
        timings.push(parsed.timing_function);
        delays.push(parsed.delay);
    }
    if durations.iter().all(|duration| duration == "0s") {
        return None;
    }
    Some(TransitionShorthand {
        property: properties.join(", "),
        duration: durations.join(", "),
        timing_function: timings.join(", "),
        delay: delays.join(", "),
    })
}

pub fn parse_transition_item(raw: &str) -> TransitionShorthand {
    let mut property = String::new();
    let mut duration = String::new();
    let mut timing_function = String::new();
    let mut delay = String::new();
    for token in split_css_tokens(raw) {
        let lower = token.to_ascii_lowercase();
        if is_css_time_token(&lower) {
            if duration.is_empty() {
                duration = token;
            } else {
                delay = token;
            }
            continue;
        }
        if is_css_timing_function(&lower) {
            timing_function = token;
            continue;
        }
        if property.is_empty() {
            property = token;
        }
    }
    TransitionShorthand {
        property: if property.is_empty() {
            "all".into()
        } else {
            property
        },
        duration: if duration.is_empty() {
            "0s".into()
        } else {
            duration
        },
        timing_function: if timing_function.is_empty() {
            "ease".into()
        } else {
            timing_function
        },
        delay: if delay.is_empty() { "0s".into() } else { delay },
    }
}

pub fn is_css_timing_function(token: &str) -> bool {
    matches!(
        token,
        "linear"
            | "ease"
            | "ease-in"
            | "ease-out"
            | "ease-in-out"
            | "ease-in-out-cubic"
            | "step-start"
            | "step-end"
    ) || token.starts_with("cubic-bezier(")
        || token.starts_with("steps(")
}

/// A `<time>`: a number with an `s` or `ms` unit. The suffix alone is not
/// one — `font-variation-settings` and an animation called `pulses` end in `s`
/// too, and taking them for a duration loses the property or the name.
pub fn is_css_time_token(token: &str) -> bool {
    parse_css_time_token(token).is_some()
}

pub fn split_css_tokens(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in raw.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            _ if ch.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

pub fn split_css_comma_list(raw: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in raw.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                let item = current.trim().to_string();
                if !item.is_empty() {
                    items.push(item);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let item = current.trim().to_string();
    if !item.is_empty() {
        items.push(item);
    }
    items
}

pub fn css_list_at(list: &[String], index: usize) -> &str {
    if list.is_empty() {
        return "";
    }
    list.get(index)
        .map(String::as_str)
        .unwrap_or(list[list.len() - 1].as_str())
}

pub fn parse_css_time_token(trimmed: &str) -> Option<f32> {
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(value) = trimmed.strip_suffix("ms") {
        return value.trim().parse::<f32>().ok().map(|v| v.max(0.0));
    }
    if let Some(value) = trimmed.strip_suffix('s') {
        return value
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| (v * 1000.0).max(0.0));
    }
    None
}

/// A parsed CSS time in milliseconds as a [`Duration`], to the nanosecond.
/// `from_secs_f32(ms / 1000.0)` makes `200ms` 200.000003ms, so a CSS track
/// would end a frame later than the same track authored in Rust.
pub fn css_ms_duration(ms: f32) -> Duration {
    Duration::from_nanos((f64::from(ms.max(0.0)) * 1_000_000.0).round() as u64)
}

pub fn parse_css_time_ms(raw: &str) -> Option<f32> {
    let parts = split_css_comma_list(raw);
    if parts.is_empty() {
        return parse_css_time_token(raw);
    }
    parts
        .iter()
        .filter_map(|part| parse_css_time_token(part))
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
}

pub fn first_timing_token(raw: &str) -> String {
    split_css_comma_list(raw)
        .into_iter()
        .next()
        .unwrap_or_else(|| raw.trim().to_string())
}

pub fn easing_from_css_keyword(name: &str) -> Easing {
    match name {
        "linear" => Easing::Linear,
        "ease" => Easing::CubicBezier([0.25, 0.1, 0.25, 1.0]),
        "ease-in" => Easing::CubicBezier([0.42, 0.0, 1.0, 1.0]),
        "ease-out" => Easing::CubicBezier([0.0, 0.0, 0.58, 1.0]),
        "ease-in-out-cubic" => Easing::EaseInOutCubic,
        "ease-in-out" => Easing::CubicBezier([0.42, 0.0, 0.58, 1.0]),
        "ease-out-cubic" => Easing::EaseOutCubic,
        other => parse_cubic_bezier(other)
            .map(Easing::CubicBezier)
            .unwrap_or(Easing::EaseOutCubic),
    }
}

pub fn parse_cubic_bezier(name: &str) -> Option<[f32; 4]> {
    let inner = name
        .strip_prefix("cubic-bezier(")?
        .trim_end_matches(')')
        .trim();
    let mut values = [0.0f32; 4];
    let mut count = 0usize;
    for part in inner.split(',') {
        let value: f32 = part.trim().parse().ok()?;
        if count >= 4 {
            return None;
        }
        if matches!(count, 0 | 2) && !(0.0..=1.0).contains(&value) {
            return None;
        }
        values[count] = value;
        count += 1;
    }
    (count == 4).then_some(values)
}

pub fn parse_transition_properties(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty() && !token.eq_ignore_ascii_case("none"))
        .map(str::to_string)
        .collect()
}
