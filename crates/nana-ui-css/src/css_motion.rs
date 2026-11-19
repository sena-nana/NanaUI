//! CSS motion values: time lists, timing functions and the `transition`
//! shorthand, as strings and Style Model easings. Used by the `.vue`
//! compiler, which compiles `transition` into implicit animations.

use std::time::Duration;

use nana_ui_core::{
    AnimationDirection, AnimationFillMode, AnimationIteration, AnimationPlayState, Easing,
    MotionCurve, StepJump,
};

use crate::css_interactive::MotionDeclarations;

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
    if durations
        .iter()
        .all(|duration| parse_css_time_token(duration).is_none_or(|ms| ms <= 0.0))
        && delays
            .iter()
            .all(|delay| parse_css_time_token(delay).is_none_or(|ms| ms <= 0.0))
    {
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

pub fn motion_curve_from_css_token(token: &str) -> MotionCurve {
    let lower = token.trim().to_ascii_lowercase();
    match lower.as_str() {
        "step-start" => MotionCurve::Steps {
            count: 1,
            jump: StepJump::Start,
        },
        "step-end" => MotionCurve::Steps {
            count: 1,
            jump: StepJump::End,
        },
        _ => parse_steps_fn(&lower)
            .unwrap_or_else(|| MotionCurve::Easing(easing_from_css_keyword(&lower))),
    }
}

fn parse_steps_fn(lower: &str) -> Option<MotionCurve> {
    let inner = lower.strip_prefix("steps(")?.strip_suffix(')')?.trim();
    let mut parts = inner.split(',').map(str::trim);
    let count: u32 = parts.next()?.parse().ok()?;
    if count == 0 {
        return None;
    }
    let jump = match parts.next() {
        None | Some("end" | "jump-end") => StepJump::End,
        Some("start" | "jump-start") => StepJump::Start,
        Some("jump-none") => StepJump::None,
        Some("jump-both") => StepJump::Both,
        _ => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some(MotionCurve::Steps { count, jump })
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedAnimation {
    pub name: String,
    pub duration_ms: f32,
    pub delay_ms: f32,
    pub curve: MotionCurve,
    pub iterations: AnimationIteration,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
}

impl Default for ParsedAnimation {
    fn default() -> Self {
        Self {
            name: String::new(),
            duration_ms: 0.0,
            delay_ms: 0.0,
            curve: motion_curve_from_css_token("ease"),
            iterations: AnimationIteration::ONCE,
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
        }
    }
}

fn parse_animation_iteration(token: &str) -> Option<AnimationIteration> {
    let lower = token.trim().to_ascii_lowercase();
    if lower == "infinite" {
        return Some(AnimationIteration::Infinite);
    }
    let count = lower.parse::<f32>().ok()?;
    if !count.is_finite() || count < 0.0 {
        return None;
    }
    Some(AnimationIteration::Count(count.round() as u32))
}

fn parse_animation_direction(token: &str) -> Option<AnimationDirection> {
    match token.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(AnimationDirection::Normal),
        "reverse" => Some(AnimationDirection::Reverse),
        "alternate" => Some(AnimationDirection::Alternate),
        "alternate-reverse" => Some(AnimationDirection::AlternateReverse),
        _ => None,
    }
}

fn parse_animation_fill_mode(token: &str) -> Option<AnimationFillMode> {
    match token.trim().to_ascii_lowercase().as_str() {
        "none" => Some(AnimationFillMode::None),
        "forwards" => Some(AnimationFillMode::Forwards),
        "backwards" => Some(AnimationFillMode::Backwards),
        "both" => Some(AnimationFillMode::Both),
        _ => None,
    }
}

fn parse_animation_play_state(token: &str) -> Option<AnimationPlayState> {
    match token.trim().to_ascii_lowercase().as_str() {
        "running" => Some(AnimationPlayState::Running),
        "paused" => Some(AnimationPlayState::Paused),
        _ => None,
    }
}

fn parse_animation_shorthand_item(raw: &str) -> ParsedAnimation {
    let mut anim = ParsedAnimation::default();
    let mut saw_duration = false;
    let mut saw_fill = false;
    for token in split_css_tokens(raw) {
        let lower = token.to_ascii_lowercase();
        if let Some(ms) = parse_css_time_token(&lower) {
            if !saw_duration {
                anim.duration_ms = ms;
                saw_duration = true;
            } else {
                anim.delay_ms = ms;
            }
            continue;
        }
        if is_css_timing_function(&lower) {
            anim.curve = motion_curve_from_css_token(&lower);
            continue;
        }
        if let Some(iters) = parse_animation_iteration(&lower) {
            anim.iterations = iters;
            continue;
        }
        if let Some(dir) = parse_animation_direction(&lower) {
            anim.direction = dir;
            continue;
        }
        if let Some(play) = parse_animation_play_state(&lower) {
            anim.play_state = play;
            continue;
        }
        if lower == "none" && !saw_fill && !anim.name.is_empty() {
            anim.fill_mode = AnimationFillMode::None;
            saw_fill = true;
            continue;
        }
        if lower != "none"
            && let Some(fill) = parse_animation_fill_mode(&lower)
        {
            anim.fill_mode = fill;
            saw_fill = true;
            continue;
        }
        if anim.name.is_empty() {
            anim.name = token;
        }
    }
    anim
}

pub fn parse_animation_declarations(motion: &MotionDeclarations) -> Vec<ParsedAnimation> {
    let names_raw = motion
        .animation_name
        .as_deref()
        .map(split_css_comma_list)
        .unwrap_or_default();
    let durations = motion
        .animation_duration
        .as_deref()
        .map(split_css_comma_list)
        .unwrap_or_default();
    let timings = motion
        .animation_timing_function
        .as_deref()
        .map(split_css_comma_list)
        .unwrap_or_default();
    let delays = motion
        .animation_delay
        .as_deref()
        .map(split_css_comma_list)
        .unwrap_or_default();
    let iterations = motion
        .animation_iteration_count
        .as_deref()
        .map(split_css_comma_list)
        .unwrap_or_default();
    let directions = motion
        .animation_direction
        .as_deref()
        .map(split_css_comma_list)
        .unwrap_or_default();
    let fills = motion
        .animation_fill_mode
        .as_deref()
        .map(split_css_comma_list)
        .unwrap_or_default();
    let play_states = motion
        .animation_play_state
        .as_deref()
        .map(split_css_comma_list)
        .unwrap_or_default();

    let mut items: Vec<ParsedAnimation> = if let Some(shorthand) = motion.animation.as_deref() {
        let parts = split_css_comma_list(shorthand);
        if parts.is_empty() {
            vec![parse_animation_shorthand_item(shorthand)]
        } else {
            parts
                .iter()
                .map(|part| parse_animation_shorthand_item(part))
                .collect()
        }
    } else if !names_raw.is_empty() {
        names_raw
            .iter()
            .map(|name| ParsedAnimation {
                name: name.clone(),
                ..Default::default()
            })
            .collect()
    } else {
        return Vec::new();
    };

    for (idx, item) in items.iter_mut().enumerate() {
        if !names_raw.is_empty() {
            item.name = css_list_at(&names_raw, idx).to_string();
        }
        if !durations.is_empty()
            && let Some(ms) = parse_css_time_token(css_list_at(&durations, idx))
        {
            item.duration_ms = ms;
        }
        if !timings.is_empty() {
            item.curve = motion_curve_from_css_token(css_list_at(&timings, idx));
        }
        if !delays.is_empty()
            && let Some(ms) = parse_css_time_token(css_list_at(&delays, idx))
        {
            item.delay_ms = ms;
        }
        if !iterations.is_empty()
            && let Some(iter) = parse_animation_iteration(css_list_at(&iterations, idx))
        {
            item.iterations = iter;
        }
        if !directions.is_empty()
            && let Some(dir) = parse_animation_direction(css_list_at(&directions, idx))
        {
            item.direction = dir;
        }
        if !fills.is_empty()
            && let Some(fill) = parse_animation_fill_mode(css_list_at(&fills, idx))
        {
            item.fill_mode = fill;
        }
        if !play_states.is_empty()
            && let Some(play) = parse_animation_play_state(css_list_at(&play_states, idx))
        {
            item.play_state = play;
        }
    }

    items.retain(|item| !item.name.is_empty() && !item.name.eq_ignore_ascii_case("none"));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_zero_duration_transition_delay_and_animation_specs() {
        let shorthand = parse_transition_shorthand("opacity 0s 100ms").expect("delay-only");
        assert_eq!(shorthand.property, "opacity");
        assert_eq!(shorthand.duration, "0s");
        assert_eq!(shorthand.delay, "100ms");

        assert_eq!(
            motion_curve_from_css_token("steps(4, start)"),
            MotionCurve::Steps {
                count: 4,
                jump: StepJump::Start,
            }
        );
        assert_eq!(
            motion_curve_from_css_token("cubic-bezier(0.1, 0.2, 0.3, 0.9)"),
            MotionCurve::Easing(Easing::CubicBezier([0.1, 0.2, 0.3, 0.9]))
        );

        let motion = MotionDeclarations {
            animation: Some(
                "pulse 250ms 40ms steps(4, start) infinite alternate both paused".into(),
            ),
            ..Default::default()
        };
        let anims = parse_animation_declarations(&motion);
        assert_eq!(anims.len(), 1);
        assert_eq!(anims[0].name, "pulse");
        assert_eq!(anims[0].duration_ms, 250.0);
        assert_eq!(anims[0].delay_ms, 40.0);
        assert_eq!(
            anims[0].curve,
            MotionCurve::Steps {
                count: 4,
                jump: StepJump::Start,
            }
        );
        assert_eq!(anims[0].iterations, AnimationIteration::Infinite);
        assert_eq!(anims[0].direction, AnimationDirection::Alternate);
        assert_eq!(anims[0].fill_mode, AnimationFillMode::Both);
        assert_eq!(anims[0].play_state, AnimationPlayState::Paused);
    }
}
