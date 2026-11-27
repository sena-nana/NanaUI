//! CSS Transitions and CSS Animations parsed directly from `&[ComponentValue]`,
//! preserving **exact declaration source order** between shorthands
//! (`transition`, `animation`) and longhands (`transition-duration`,
//! `animation-delay`, etc.).

use std::time::Duration;

use nana_ui_core::{
    AnimationDirection, AnimationFillMode, AnimationIteration, AnimationPlayState, Easing,
    MotionCurve, StepJump,
};

use crate::syntax::diag::{CssDiag, DiagCode};
use crate::syntax::parser::{ComponentValue, split_comma_groups, split_whitespace_items, trim_ws};
use crate::syntax::span::Span;
use crate::syntax::tokenizer::{SpannedToken, Token};

/// One resolved CSS transition item from a declaration block.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledTransition {
    pub property: String,
    pub duration_ms: f32,
    pub delay_ms: f32,
    pub curve: MotionCurve,
}

impl CompiledTransition {
    /// Convert `duration_ms` to [`Duration`] without `f32` rounding drift.
    pub fn duration(&self) -> Duration {
        css_ms_duration(self.duration_ms)
    }

    /// Convert `delay_ms` to [`Duration`] without `f32` rounding drift.
    pub fn delay(&self) -> Duration {
        css_ms_duration(self.delay_ms)
    }
}

/// One resolved CSS animation item from a declaration block.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledAnimation {
    pub name: String,
    pub duration_ms: f32,
    pub delay_ms: f32,
    pub curve: MotionCurve,
    pub iterations: AnimationIteration,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
}

impl Default for CompiledAnimation {
    fn default() -> Self {
        Self {
            name: String::new(),
            duration_ms: 0.0,
            delay_ms: 0.0,
            curve: default_ease_curve(),
            iterations: AnimationIteration::ONCE,
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
        }
    }
}

/// Convert milliseconds to a [`Duration`] with nanosecond precision.
pub fn css_ms_duration(ms: f32) -> Duration {
    Duration::from_nanos((f64::from(ms.max(0.0)) * 1_000_000.0).round() as u64)
}

#[inline]
pub fn default_ease_curve() -> MotionCurve {
    MotionCurve::Easing(Easing::CubicBezier([0.25, 0.1, 0.25, 1.0]))
}

/// Returns `true` for any CSS `transition*` or `animation*` property.
pub fn is_motion_property(name: &str) -> bool {
    matches!(
        name,
        "transition"
            | "transition-property"
            | "transition-duration"
            | "transition-timing-function"
            | "transition-delay"
            | "animation"
            | "animation-name"
            | "animation-duration"
            | "animation-timing-function"
            | "animation-delay"
            | "animation-iteration-count"
            | "animation-direction"
            | "animation-fill-mode"
            | "animation-play-state"
    )
}

/// Accumulates motion declarations in **exact declaration source order**.
///
/// Unlike legacy struct-based storage (which always applied longhands after
/// shorthands regardless of source order), `transition-duration: 500ms; transition: opacity 200ms`
/// correctly yields `200ms`, while `transition: opacity 200ms; transition-duration: 500ms`
/// yields `500ms`.
#[derive(Debug, Default)]
pub(crate) struct MotionBuilder {
    // Transition lists (updated in exact source order)
    t_property: Option<Vec<String>>,
    t_duration: Option<Vec<f32>>,
    t_timing: Option<Vec<MotionCurve>>,
    t_delay: Option<Vec<f32>>,

    // Animation lists (updated in exact source order)
    a_name: Option<Vec<String>>,
    a_duration: Option<Vec<f32>>,
    a_timing: Option<Vec<MotionCurve>>,
    a_delay: Option<Vec<f32>>,
    a_iterations: Option<Vec<AnimationIteration>>,
    a_direction: Option<Vec<AnimationDirection>>,
    a_fill: Option<Vec<AnimationFillMode>>,
    a_play: Option<Vec<AnimationPlayState>>,
}

impl MotionBuilder {
    pub fn apply(
        &mut self,
        prop: &str,
        values: &[ComponentValue],
        span: Span,
    ) -> Result<(), CssDiag> {
        let groups = split_comma_groups(values);
        if groups.is_empty() {
            return Err(CssDiag::error(
                span,
                DiagCode::InvalidValue,
                format!("empty value for `{prop}`"),
            ));
        }

        match prop {
            "transition" => {
                let mut props = Vec::with_capacity(groups.len());
                let mut durs = Vec::with_capacity(groups.len());
                let mut timings = Vec::with_capacity(groups.len());
                let mut delays = Vec::with_capacity(groups.len());
                for g in groups {
                    let (p, d, t, dl) = parse_transition_item(g, span)?;
                    props.push(p);
                    durs.push(d);
                    timings.push(t);
                    delays.push(dl);
                }
                self.t_property = Some(props);
                self.t_duration = Some(durs);
                self.t_timing = Some(timings);
                self.t_delay = Some(delays);
                Ok(())
            }
            "transition-property" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let [cv] = g else {
                        return Err(CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "expected property identifier in `transition-property`",
                        ));
                    };
                    let Some(id) = cv.as_ident() else {
                        return Err(CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "expected property identifier in `transition-property`",
                        ));
                    };
                    out.push(id.to_ascii_lowercase());
                }
                self.t_property = Some(out);
                Ok(())
            }
            "transition-duration" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let ms = parse_non_negative_time_ms(g).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid time in `transition-duration`",
                        )
                    })?;
                    out.push(ms);
                }
                self.t_duration = Some(out);
                Ok(())
            }
            "transition-delay" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let ms = parse_time_ms(g).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid time in `transition-delay`",
                        )
                    })?;
                    out.push(ms);
                }
                self.t_delay = Some(out);
                Ok(())
            }
            "transition-timing-function" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let curve = parse_timing_function(g).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid timing function in `transition-timing-function`",
                        )
                    })?;
                    out.push(curve);
                }
                self.t_timing = Some(out);
                Ok(())
            }
            "animation" => {
                let mut names = Vec::with_capacity(groups.len());
                let mut durs = Vec::with_capacity(groups.len());
                let mut timings = Vec::with_capacity(groups.len());
                let mut delays = Vec::with_capacity(groups.len());
                let mut iters = Vec::with_capacity(groups.len());
                let mut dirs = Vec::with_capacity(groups.len());
                let mut fills = Vec::with_capacity(groups.len());
                let mut plays = Vec::with_capacity(groups.len());
                for g in groups {
                    let anim = parse_animation_item(g, span)?;
                    names.push(anim.name);
                    durs.push(anim.duration_ms);
                    timings.push(anim.curve);
                    delays.push(anim.delay_ms);
                    iters.push(anim.iterations);
                    dirs.push(anim.direction);
                    fills.push(anim.fill_mode);
                    plays.push(anim.play_state);
                }
                self.a_name = Some(names);
                self.a_duration = Some(durs);
                self.a_timing = Some(timings);
                self.a_delay = Some(delays);
                self.a_iterations = Some(iters);
                self.a_direction = Some(dirs);
                self.a_fill = Some(fills);
                self.a_play = Some(plays);
                Ok(())
            }
            "animation-name" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let [cv] = g else {
                        return Err(CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid `animation-name`",
                        ));
                    };
                    let name = match cv.as_token() {
                        Some(Token::Ident(s) | Token::String(s)) => s.clone(),
                        _ => {
                            return Err(CssDiag::error(
                                span,
                                DiagCode::InvalidValue,
                                "invalid `animation-name`",
                            ));
                        }
                    };
                    out.push(name);
                }
                self.a_name = Some(out);
                Ok(())
            }
            "animation-duration" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let ms = parse_non_negative_time_ms(g).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid time in `animation-duration`",
                        )
                    })?;
                    out.push(ms);
                }
                self.a_duration = Some(out);
                Ok(())
            }
            "animation-delay" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let ms = parse_time_ms(g).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid time in `animation-delay`",
                        )
                    })?;
                    out.push(ms);
                }
                self.a_delay = Some(out);
                Ok(())
            }
            "animation-timing-function" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let curve = parse_timing_function(g).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid timing function in `animation-timing-function`",
                        )
                    })?;
                    out.push(curve);
                }
                self.a_timing = Some(out);
                Ok(())
            }
            "animation-iteration-count" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let iter = parse_iteration_count(g).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid `animation-iteration-count`",
                        )
                    })?;
                    out.push(iter);
                }
                self.a_iterations = Some(out);
                Ok(())
            }
            "animation-direction" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let dir = parse_direction(g).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid `animation-direction`",
                        )
                    })?;
                    out.push(dir);
                }
                self.a_direction = Some(out);
                Ok(())
            }
            "animation-fill-mode" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let fill = parse_fill_mode(g).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid `animation-fill-mode`",
                        )
                    })?;
                    out.push(fill);
                }
                self.a_fill = Some(out);
                Ok(())
            }
            "animation-play-state" => {
                let mut out = Vec::with_capacity(groups.len());
                for g in groups {
                    let play = parse_play_state(g).ok_or_else(|| {
                        CssDiag::error(
                            span,
                            DiagCode::InvalidValue,
                            "invalid `animation-play-state`",
                        )
                    })?;
                    out.push(play);
                }
                self.a_play = Some(out);
                Ok(())
            }
            _ => Err(CssDiag::error(
                span,
                DiagCode::UnknownProperty,
                format!("unknown motion property `{prop}`"),
            )),
        }
    }

    pub fn finish(self) -> (Vec<CompiledTransition>, Vec<CompiledAnimation>) {
        let transitions = self.finish_transitions();
        let animations = self.finish_animations();
        (transitions, animations)
    }

    fn finish_transitions(&self) -> Vec<CompiledTransition> {
        let props: Vec<String> = match &self.t_property {
            Some(p) => p.clone(),
            None if self.t_duration.is_some() || self.t_delay.is_some() => vec!["all".into()],
            None => return Vec::new(),
        };
        let durs = self.t_duration.as_deref().unwrap_or(&[]);
        let delays = self.t_delay.as_deref().unwrap_or(&[]);
        let timings = self.t_timing.as_deref().unwrap_or(&[]);

        let mut out = Vec::new();
        for (idx, prop) in props.iter().enumerate() {
            if prop.eq_ignore_ascii_case("none") {
                continue;
            }
            let duration_ms = cycle_copy(durs, idx).unwrap_or(0.0);
            let delay_ms = cycle_copy(delays, idx).unwrap_or(0.0);
            if duration_ms <= 0.0 && delay_ms <= 0.0 {
                continue;
            }
            let curve = cycle_copy(timings, idx).unwrap_or_else(default_ease_curve);
            out.push(CompiledTransition {
                property: prop.clone(),
                duration_ms,
                delay_ms,
                curve,
            });
        }
        out
    }

    fn finish_animations(&self) -> Vec<CompiledAnimation> {
        // Standalone `animation-play-state` without `animation-name` still emits
        // a play-state-only entry (`name: ""`) so `:hover { animation-play-state: paused }`
        // works identically to runtime `StyleSite::animations_for`.
        let names: Vec<String> = match &self.a_name {
            Some(n) => n.clone(),
            None => {
                if let Some(plays) = &self.a_play
                    && let Some(&first) = plays.first()
                {
                    return vec![CompiledAnimation {
                        name: String::new(),
                        play_state: first,
                        ..CompiledAnimation::default()
                    }];
                }
                return Vec::new();
            }
        };

        let durs = self.a_duration.as_deref().unwrap_or(&[]);
        let delays = self.a_delay.as_deref().unwrap_or(&[]);
        let timings = self.a_timing.as_deref().unwrap_or(&[]);
        let iters = self.a_iterations.as_deref().unwrap_or(&[]);
        let dirs = self.a_direction.as_deref().unwrap_or(&[]);
        let fills = self.a_fill.as_deref().unwrap_or(&[]);
        let plays = self.a_play.as_deref().unwrap_or(&[]);

        let mut out = Vec::new();
        for (idx, name) in names.iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            out.push(CompiledAnimation {
                name: name.clone(),
                duration_ms: cycle_copy(durs, idx).unwrap_or(0.0),
                delay_ms: cycle_copy(delays, idx).unwrap_or(0.0),
                curve: cycle_copy(timings, idx).unwrap_or_else(default_ease_curve),
                iterations: cycle_copy(iters, idx).unwrap_or(AnimationIteration::ONCE),
                direction: cycle_copy(dirs, idx).unwrap_or(AnimationDirection::Normal),
                fill_mode: cycle_copy(fills, idx).unwrap_or(AnimationFillMode::None),
                play_state: cycle_copy(plays, idx).unwrap_or(AnimationPlayState::Running),
            });
        }
        out
    }
}

fn cycle_copy<T: Copy>(slice: &[T], idx: usize) -> Option<T> {
    if slice.is_empty() {
        None
    } else {
        Some(slice[idx % slice.len()])
    }
}

fn parse_transition_item(
    values: &[ComponentValue],
    span: Span,
) -> Result<(String, f32, MotionCurve, f32), CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty transition item",
        ));
    }
    let mut property: Option<String> = None;
    let mut duration: Option<f32> = None;
    let mut delay: Option<f32> = None;
    let mut curve: Option<MotionCurve> = None;

    for item in items {
        if let Some(ms) = parse_time_ms(item) {
            if duration.is_none() {
                if ms < 0.0 {
                    return Err(CssDiag::error(
                        span,
                        DiagCode::InvalidValue,
                        "negative `transition-duration`",
                    ));
                }
                duration = Some(ms);
            } else if delay.is_none() {
                delay = Some(ms);
            } else {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    "too many time values in `transition`",
                ));
            }
            continue;
        }
        if curve.is_none()
            && let Some(c) = parse_timing_function(item)
        {
            curve = Some(c);
            continue;
        }
        if property.is_none()
            && let [cv] = item
            && let Some(id) = cv.as_ident()
        {
            property = Some(id.to_ascii_lowercase());
            continue;
        }
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "unexpected token in `transition` shorthand",
        ));
    }

    Ok((
        property.unwrap_or_else(|| "all".into()),
        duration.unwrap_or(0.0),
        curve.unwrap_or_else(default_ease_curve),
        delay.unwrap_or(0.0),
    ))
}

fn parse_animation_item(
    values: &[ComponentValue],
    span: Span,
) -> Result<CompiledAnimation, CssDiag> {
    let items = split_whitespace_items(values);
    if items.is_empty() {
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "empty animation item",
        ));
    }
    let mut anim = CompiledAnimation::default();
    let mut saw_duration = false;
    let mut saw_delay = false;
    let mut saw_timing = false;
    let mut saw_iter = false;
    let mut saw_dir = false;
    let mut saw_fill = false;
    let mut saw_play = false;
    let mut saw_name = false;

    for item in items {
        if let Some(ms) = parse_time_ms(item) {
            if !saw_duration {
                if ms < 0.0 {
                    return Err(CssDiag::error(
                        span,
                        DiagCode::InvalidValue,
                        "negative `animation-duration`",
                    ));
                }
                anim.duration_ms = ms;
                saw_duration = true;
            } else if !saw_delay {
                anim.delay_ms = ms;
                saw_delay = true;
            } else {
                return Err(CssDiag::error(
                    span,
                    DiagCode::InvalidValue,
                    "too many time values in `animation`",
                ));
            }
            continue;
        }
        if !saw_timing && let Some(c) = parse_timing_function(item) {
            anim.curve = c;
            saw_timing = true;
            continue;
        }
        if !saw_iter && let Some(it) = parse_iteration_count(item) {
            anim.iterations = it;
            saw_iter = true;
            continue;
        }
        if !saw_dir && let Some(d) = parse_direction(item) {
            anim.direction = d;
            saw_dir = true;
            continue;
        }
        if !saw_play && let Some(p) = parse_play_state(item) {
            anim.play_state = p;
            saw_play = true;
            continue;
        }
        // `none` is ambiguous between fill-mode and animation-name: if name is
        // already known and fill-mode isn't, bind fill-mode; otherwise if it's
        // another fill-mode keyword bind fill-mode.
        if let [cv] = item
            && let Some(id) = cv.as_ident()
        {
            if id.eq_ignore_ascii_case("none") && saw_name && !saw_fill {
                anim.fill_mode = AnimationFillMode::None;
                saw_fill = true;
                continue;
            }
            if !id.eq_ignore_ascii_case("none")
                && !saw_fill
                && let Some(f) = parse_fill_mode(item)
            {
                anim.fill_mode = f;
                saw_fill = true;
                continue;
            }
            if !saw_name {
                anim.name = id.to_string();
                saw_name = true;
                continue;
            }
        }
        if !saw_name
            && let [
                ComponentValue::Token(SpannedToken {
                    token: Token::String(s),
                    ..
                }),
            ] = item
        {
            anim.name = s.clone();
            saw_name = true;
            continue;
        }
        return Err(CssDiag::error(
            span,
            DiagCode::InvalidValue,
            "unexpected token in `animation` shorthand",
        ));
    }

    Ok(anim)
}

/// Parse a CSS `<time>` component value into milliseconds.
pub fn parse_time_ms(values: &[ComponentValue]) -> Option<f32> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    let Token::Dimension { value, unit } = cv.as_token()? else {
        return None;
    };
    if !value.is_finite() {
        return None;
    }
    match unit.to_ascii_lowercase().as_str() {
        "ms" => Some(*value as f32),
        "s" => Some((*value * 1000.0) as f32),
        _ => None,
    }
}

fn parse_non_negative_time_ms(values: &[ComponentValue]) -> Option<f32> {
    let ms = parse_time_ms(values)?;
    (ms >= 0.0).then_some(ms)
}

/// Parse a CSS `<easing-function>` (`linear`, `ease`, `ease-in`, `ease-out`,
/// `ease-in-out`, `cubic-bezier(...)`, `steps(...)`, `step-start`, `step-end`).
pub fn parse_timing_function(values: &[ComponentValue]) -> Option<MotionCurve> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    match cv {
        ComponentValue::Token(SpannedToken {
            token: Token::Ident(kw),
            ..
        }) => Some(match kw.to_ascii_lowercase().as_str() {
            "linear" => MotionCurve::Easing(Easing::Linear),
            "ease" => default_ease_curve(),
            "ease-in" => MotionCurve::Easing(Easing::CubicBezier([0.42, 0.0, 1.0, 1.0])),
            "ease-out" => MotionCurve::Easing(Easing::CubicBezier([0.0, 0.0, 0.58, 1.0])),
            "ease-in-out" => MotionCurve::Easing(Easing::CubicBezier([0.42, 0.0, 0.58, 1.0])),
            "ease-in-out-cubic" => MotionCurve::Easing(Easing::EaseInOutCubic),
            "ease-out-cubic" => MotionCurve::Easing(Easing::EaseOutCubic),
            "step-start" => MotionCurve::Steps {
                count: 1,
                jump: StepJump::Start,
            },
            "step-end" => MotionCurve::Steps {
                count: 1,
                jump: StepJump::End,
            },
            _ => return None,
        }),
        ComponentValue::Function { name, args, .. } => {
            if name.eq_ignore_ascii_case("cubic-bezier") {
                let groups = split_comma_groups(args);
                if groups.len() != 4 {
                    return None;
                }
                let mut vals = [0.0f32; 4];
                for (i, g) in groups.iter().enumerate() {
                    let [
                        ComponentValue::Token(SpannedToken {
                            token: Token::Number { value, .. },
                            ..
                        }),
                    ] = *g
                    else {
                        return None;
                    };
                    if !value.is_finite() {
                        return None;
                    }
                    let v = *value as f32;
                    if matches!(i, 0 | 2) && !(0.0..=1.0).contains(&v) {
                        return None;
                    }
                    vals[i] = v;
                }
                Some(MotionCurve::Easing(Easing::CubicBezier(vals)))
            } else if name.eq_ignore_ascii_case("steps") {
                let groups = split_comma_groups(args);
                if groups.is_empty() || groups.len() > 2 {
                    return None;
                }
                let [
                    ComponentValue::Token(SpannedToken {
                        token:
                            Token::Number {
                                value,
                                int: Some(_),
                                ..
                            },
                        ..
                    }),
                ] = groups[0]
                else {
                    return None;
                };
                if *value < 1.0 || !value.is_finite() {
                    return None;
                }
                let count = *value as u32;
                let jump = if let Some(second) = groups.get(1) {
                    let [cv] = *second else { return None };
                    match cv.as_ident()?.to_ascii_lowercase().as_str() {
                        "end" | "jump-end" => StepJump::End,
                        "start" | "jump-start" => StepJump::Start,
                        "jump-none" => {
                            if count < 2 {
                                return None;
                            }
                            StepJump::None
                        }
                        "jump-both" => StepJump::Both,
                        _ => return None,
                    }
                } else {
                    StepJump::End
                };
                Some(MotionCurve::Steps { count, jump })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn parse_iteration_count(values: &[ComponentValue]) -> Option<AnimationIteration> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    match cv.as_token()? {
        Token::Ident(s) if s.eq_ignore_ascii_case("infinite") => Some(AnimationIteration::Infinite),
        Token::Number { value, .. } if value.is_finite() && *value >= 0.0 => {
            Some(AnimationIteration::Count(value.round() as u32))
        }
        _ => None,
    }
}

fn parse_direction(values: &[ComponentValue]) -> Option<AnimationDirection> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    match cv.as_ident()?.to_ascii_lowercase().as_str() {
        "normal" => Some(AnimationDirection::Normal),
        "reverse" => Some(AnimationDirection::Reverse),
        "alternate" => Some(AnimationDirection::Alternate),
        "alternate-reverse" => Some(AnimationDirection::AlternateReverse),
        _ => None,
    }
}

fn parse_fill_mode(values: &[ComponentValue]) -> Option<AnimationFillMode> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    match cv.as_ident()?.to_ascii_lowercase().as_str() {
        "none" => Some(AnimationFillMode::None),
        "forwards" => Some(AnimationFillMode::Forwards),
        "backwards" => Some(AnimationFillMode::Backwards),
        "both" => Some(AnimationFillMode::Both),
        _ => None,
    }
}

fn parse_play_state(values: &[ComponentValue]) -> Option<AnimationPlayState> {
    let [cv] = trim_ws(values) else {
        return None;
    };
    match cv.as_ident()?.to_ascii_lowercase().as_str() {
        "running" => Some(AnimationPlayState::Running),
        "paused" => Some(AnimationPlayState::Paused),
        _ => None,
    }
}
