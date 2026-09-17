//! CPU reference evaluator. Same [`crate::motion::MotionTrack`] + timestamp
//! yields a deterministic sample. GPU Workstream D must implement the same
//! function of `(track, now)` — this module is the semantic entry, not a
//! second timeline.

use std::time::Duration;

use super::{
    DecayParams, SpringParams,
    easing::Easing,
    ir::{Keyframe, MotionCurve, MotionSample, MotionTo, MotionTrack, MotionValue},
    playback::{AnimationPlayState, AnimationPlayback, MotionTiming, TimedProgress},
    property::AnimationClass,
};

/// Eased unit progress plus fill application. Runtime `AnimationSpec` uses this
/// so fill is not implemented by scheduler side-channels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProgressSample {
    pub progress: f32,
    pub finished: bool,
    pub applies: bool,
}

/// Evaluate `track` at `now`. Invalid tracks may still produce a sample;
/// callers that need a contract check [`MotionTrack::is_valid`] first.
/// `hold_at` freezes a paused track without depending on a previous sample.
pub fn evaluate_track(track: &MotionTrack, now: Duration) -> MotionSample {
    evaluate_track_at(track, now, None)
}

pub fn evaluate_track_at(
    track: &MotionTrack,
    now: Duration,
    hold_at: Option<Duration>,
) -> MotionSample {
    let clock = track.evaluation_clock(now, hold_at);
    match track.curve {
        MotionCurve::Spring(params) => evaluate_spring(track, clock, params),
        MotionCurve::Decay(params) => evaluate_decay(track, clock, params),
        MotionCurve::Easing(_) | MotionCurve::Steps { .. } => evaluate_timed(track, clock),
    }
}

/// Timed progress after playback (iteration / alternate / reverse / fill).
/// Uses the same pause clock as [`evaluate_track`]: `paused_at` then `now`.
/// Runtime `AnimationSpec` passes the host timestamp; it is not a second path.
pub fn evaluate_progress(
    timing: MotionTiming,
    playback: AnimationPlayback,
    curve: MotionCurve,
    now: Duration,
) -> ProgressSample {
    let clock = playback.evaluation_clock(now, None);
    let TimedProgress {
        linear,
        finished,
        applies,
    } = timing.timed_progress(playback, clock);
    ProgressSample {
        progress: curve.sample_progress(linear.clamp(0.0, 1.0)),
        finished,
        applies,
    }
}

fn evaluate_timed(track: &MotionTrack, now: Duration) -> MotionSample {
    let phase = track.timing.timed_progress(track.playback, now);
    let linear = phase.linear.clamp(0.0, 1.0);
    let progress = track.curve.sample_progress(linear);
    let value = if !phase.applies {
        if phase.finished {
            track.rest_value()
        } else {
            track.from
        }
    } else {
        interpolate_source(track, linear, progress, phase.finished)
    };
    let velocity = if phase.applies {
        timed_velocity(track, now)
    } else {
        track.from.zero_velocity()
    };
    MotionSample {
        id: track.id,
        target: track.target,
        property: track.property,
        value,
        progress,
        velocity,
        finished: phase.finished,
        applies: phase.applies,
    }
}

fn interpolate_source(track: &MotionTrack, linear: f32, eased: f32, finished: bool) -> MotionValue {
    if track.property.animation_class() == AnimationClass::Discrete {
        return snap_discrete(track, finished);
    }
    match &track.to {
        MotionTo::Value(to) => track.from.lerp(*to, eased),
        MotionTo::Keyframes(stops) => interpolate_keyframes(track.from, stops, linear, track.curve),
    }
}

fn snap_discrete(track: &MotionTrack, at_end: bool) -> MotionValue {
    if at_end {
        track.rest_value()
    } else {
        track.from
    }
}

fn interpolate_keyframes(
    from: MotionValue,
    stops: &[Keyframe],
    linear: f32,
    curve: MotionCurve,
) -> MotionValue {
    if stops.is_empty() {
        return from;
    }
    let p = linear.clamp(0.0, 1.0);
    let idx = stops.partition_point(|stop| stop.offset < p);
    if idx < stops.len() && (stops[idx].offset - p).abs() <= f32::EPSILON {
        return stops[idx].value;
    }
    if idx == 0 {
        let first = stops[0];
        if first.offset <= 0.0 {
            return first.value;
        }
        let local = (p / first.offset).clamp(0.0, 1.0);
        // The implicit keyframe at offset 0 is `from`, and it declares no
        // easing of its own, so this interval runs on the track's curve.
        return from.lerp(first.value, ease_local(curve, None, local));
    }
    if idx >= stops.len() {
        return stops[stops.len() - 1].value;
    }
    let prev = stops[idx - 1];
    let next = stops[idx];
    let span = (next.offset - prev.offset).max(f32::EPSILON);
    let local = ((p - prev.offset) / span).clamp(0.0, 1.0);
    // CSS Animations and the Web Animations API give a keyframe's timing
    // function to the interval that *starts* at it, not the one that ends
    // there. Reading `next.easing` shifts every per-stop easing one interval
    // late and applies the last stop's, which governs nothing.
    prev.value
        .lerp(next.value, ease_local(curve, prev.easing, local))
}

fn ease_local(curve: MotionCurve, stop: Option<Easing>, local: f32) -> f32 {
    if let Some(easing) = stop {
        return easing.sample(local);
    }
    curve.sample_progress(local)
}

fn timed_velocity(track: &MotionTrack, now: Duration) -> MotionValue {
    let dt = Duration::from_micros(1_000);
    let earlier = now.saturating_sub(dt);
    if earlier == now {
        return track.from.zero_velocity();
    }
    let now_phase = track.timing.timed_progress(track.playback, now);
    let prev_phase = track.timing.timed_progress(track.playback, earlier);
    let value = interpolate_source(
        track,
        now_phase.linear.clamp(0.0, 1.0),
        track
            .curve
            .sample_progress(now_phase.linear.clamp(0.0, 1.0)),
        now_phase.finished,
    );
    let prev = interpolate_source(
        track,
        prev_phase.linear.clamp(0.0, 1.0),
        track
            .curve
            .sample_progress(prev_phase.linear.clamp(0.0, 1.0)),
        prev_phase.finished,
    );
    (prev - value).scale(-1.0 / dt.as_secs_f32())
}

const SETTLE_POSITION: f32 = 0.001;
const SETTLE_VELOCITY: f32 = 0.01;
/// Longest settle time either physics curve will report.
///
/// The spring search stops doubling at 120 s; decay is analytic and would
/// otherwise hand `Duration::from_secs_f32` whatever a huge time constant
/// produces.
const MAX_SETTLE_SECONDS: f32 = 128.0;

fn evaluate_spring(track: &MotionTrack, now: Duration, params: SpringParams) -> MotionSample {
    let Some(start) = track.timing.effective_start() else {
        return settled_sample(track, track.rest_value());
    };
    if now < start {
        let applies = track.playback.fill_mode.applies_backwards();
        return MotionSample {
            id: track.id,
            target: track.target,
            property: track.property,
            value: track.from,
            progress: 0.0,
            velocity: track.velocity,
            finished: false,
            applies,
        };
    }
    let t = now.saturating_sub(start).as_secs_f32();
    let rest = track.rest_value();
    if track.property.animation_class() == AnimationClass::Discrete {
        return discrete_physics_sample(
            track,
            now,
            start,
            spring_settle_duration(track.from, rest, track.velocity, params),
        );
    }
    let (value, velocity) = spring_state(track.from, rest, track.velocity, params, t);
    let finished = is_settled(value, rest, velocity);
    MotionSample {
        id: track.id,
        target: track.target,
        property: track.property,
        value,
        progress: if finished { 1.0 } else { 0.0 },
        velocity,
        finished,
        applies: true,
    }
}

fn evaluate_decay(track: &MotionTrack, now: Duration, params: DecayParams) -> MotionSample {
    let Some(start) = track.timing.effective_start() else {
        return settled_sample(track, decay_rest(track.from, track.velocity, params));
    };
    if now < start {
        let applies = track.playback.fill_mode.applies_backwards();
        return MotionSample {
            id: track.id,
            target: track.target,
            property: track.property,
            value: track.from,
            progress: 0.0,
            velocity: track.velocity,
            finished: false,
            applies,
        };
    }
    let t = now.saturating_sub(start).as_secs_f32();
    if track.property.animation_class() == AnimationClass::Discrete {
        return discrete_physics_sample(
            track,
            now,
            start,
            decay_settle_duration(track.velocity, params),
        );
    }
    let (value, velocity) = decay_state(track.from, track.velocity, params, t);
    let rest = decay_rest(track.from, track.velocity, params);
    let finished = is_settled(value, rest, velocity);
    MotionSample {
        id: track.id,
        target: track.target,
        property: track.property,
        value,
        progress: if finished { 1.0 } else { 0.0 },
        velocity,
        finished,
        applies: true,
    }
}

fn settled_sample(track: &MotionTrack, value: MotionValue) -> MotionSample {
    MotionSample {
        id: track.id,
        target: track.target,
        property: track.property,
        value,
        progress: 1.0,
        velocity: value.zero_velocity(),
        finished: true,
        applies: track.playback.fill_mode.applies_forwards(),
    }
}

fn discrete_physics_sample(
    track: &MotionTrack,
    now: Duration,
    start: Duration,
    settle: Option<Duration>,
) -> MotionSample {
    let at_end = settle.is_some_and(|end| now.saturating_sub(start) >= end);
    MotionSample {
        id: track.id,
        target: track.target,
        property: track.property,
        value: snap_discrete(track, at_end),
        progress: if at_end { 1.0 } else { 0.0 },
        velocity: if at_end {
            track.rest_value().zero_velocity()
        } else {
            track.velocity
        },
        finished: at_end,
        applies: true,
    }
}

fn is_settled(value: MotionValue, rest: MotionValue, velocity: MotionValue) -> bool {
    max_abs(value - rest) <= SETTLE_POSITION && max_abs(velocity) <= SETTLE_VELOCITY
}

/// Discrete tags have no interpolable metric (`max_abs` is 0). Settle search
/// uses the integer tag as a scalar so Discrete+physics is not instantly done.
fn physics_numeric(value: MotionValue) -> MotionValue {
    match value {
        MotionValue::Discrete(tag) => MotionValue::Scalar(tag as f32),
        other => other,
    }
}

fn max_abs(value: MotionValue) -> f32 {
    match value {
        MotionValue::Scalar(v) => v.abs(),
        MotionValue::Color(c) => c.iter().fold(0.0_f32, |m, x| m.max(x.abs())),
        MotionValue::Transform(p) => [p.a, p.b, p.c, p.d, p.e, p.f]
            .into_iter()
            .fold(0.0_f32, |m, x| m.max(x.abs())),
        MotionValue::Discrete(_) => 0.0,
    }
}

fn spring_state(
    from: MotionValue,
    rest: MotionValue,
    velocity: MotionValue,
    params: SpringParams,
    t: f32,
) -> (MotionValue, MotionValue) {
    match (from, rest, velocity) {
        (MotionValue::Scalar(x0), MotionValue::Scalar(target), MotionValue::Scalar(v0)) => {
            let (x, v) = damped_harmonic(x0 - target, v0, params, t);
            (MotionValue::Scalar(target + x), MotionValue::Scalar(v))
        }
        (MotionValue::Color(a), MotionValue::Color(b), MotionValue::Color(v)) => {
            let mut out = [0.0; 4];
            let mut vel = [0.0; 4];
            for i in 0..4 {
                let (x, dv) = damped_harmonic(a[i] - b[i], v[i], params, t);
                out[i] = b[i] + x;
                vel[i] = dv;
            }
            (MotionValue::Color(out), MotionValue::Color(vel))
        }
        (MotionValue::Transform(a), MotionValue::Transform(b), MotionValue::Transform(v)) => {
            let channels = [
                (a.a, b.a, v.a),
                (a.b, b.b, v.b),
                (a.c, b.c, v.c),
                (a.d, b.d, v.d),
                (a.e, b.e, v.e),
                (a.f, b.f, v.f),
            ];
            let mut pos = [0.0_f32; 6];
            let mut vel = [0.0_f32; 6];
            for (i, (x0, target, v0)) in channels.into_iter().enumerate() {
                let (x, dv) = damped_harmonic(x0 - target, v0, params, t);
                pos[i] = target + x;
                vel[i] = dv;
            }
            (
                MotionValue::Transform(crate::PaintTransform {
                    a: pos[0],
                    b: pos[1],
                    c: pos[2],
                    d: pos[3],
                    e: pos[4],
                    f: pos[5],
                }),
                MotionValue::Transform(crate::PaintTransform {
                    a: vel[0],
                    b: vel[1],
                    c: vel[2],
                    d: vel[3],
                    e: vel[4],
                    f: vel[5],
                }),
            )
        }
        _ => {
            if t <= 0.0 {
                (from, velocity)
            } else {
                (rest, rest.zero_velocity())
            }
        }
    }
}

fn decay_state(
    from: MotionValue,
    velocity: MotionValue,
    params: DecayParams,
    t: f32,
) -> (MotionValue, MotionValue) {
    fn scalar(x0: f32, v0: f32, tau: f32, t: f32) -> (f32, f32) {
        if tau <= 0.0 || !tau.is_finite() {
            return (x0, 0.0);
        }
        let t = t.max(0.0);
        let decay = (-t / tau).exp();
        (x0 + v0 * tau * (1.0 - decay), v0 * decay)
    }
    let tau = params.time_constant;
    match (from, velocity) {
        (MotionValue::Scalar(x0), MotionValue::Scalar(v0)) => {
            let (x, v) = scalar(x0, v0, tau, t);
            (MotionValue::Scalar(x), MotionValue::Scalar(v))
        }
        (MotionValue::Color(a), MotionValue::Color(v)) => {
            let mut out = [0.0; 4];
            let mut vel = [0.0; 4];
            for i in 0..4 {
                let (x, dv) = scalar(a[i], v[i], tau, t);
                out[i] = x;
                vel[i] = dv;
            }
            (MotionValue::Color(out), MotionValue::Color(vel))
        }
        (MotionValue::Transform(a), MotionValue::Transform(v)) => {
            let channels = [
                (a.a, v.a),
                (a.b, v.b),
                (a.c, v.c),
                (a.d, v.d),
                (a.e, v.e),
                (a.f, v.f),
            ];
            let mut pos = [0.0_f32; 6];
            let mut vel = [0.0_f32; 6];
            for (i, (x0, v0)) in channels.into_iter().enumerate() {
                let (x, dv) = scalar(x0, v0, tau, t);
                pos[i] = x;
                vel[i] = dv;
            }
            (
                MotionValue::Transform(crate::PaintTransform {
                    a: pos[0],
                    b: pos[1],
                    c: pos[2],
                    d: pos[3],
                    e: pos[4],
                    f: pos[5],
                }),
                MotionValue::Transform(crate::PaintTransform {
                    a: vel[0],
                    b: vel[1],
                    c: vel[2],
                    d: vel[3],
                    e: vel[4],
                    f: vel[5],
                }),
            )
        }
        _ => (from, velocity.zero_velocity()),
    }
}

fn decay_rest(from: MotionValue, velocity: MotionValue, params: DecayParams) -> MotionValue {
    decay_state(from, velocity, params, 1_000.0).0
}

/// Closed-form damped harmonic oscillator for displacement from rest.
/// `x(t), v(t)` from `x0, v0` — random-accessible in `t`.
pub fn damped_harmonic(x0: f32, v0: f32, params: SpringParams, t: f32) -> (f32, f32) {
    if t <= 0.0 {
        return (x0, v0);
    }
    if !(params.mass > 0.0
        && params.stiffness > 0.0
        && params.mass.is_finite()
        && params.stiffness.is_finite())
    {
        return (0.0, 0.0);
    }
    let omega0 = (params.stiffness / params.mass).sqrt();
    if !omega0.is_finite() || omega0 == 0.0 {
        return (0.0, 0.0);
    }
    let zeta = params.damping / (2.0 * (params.stiffness * params.mass).sqrt());
    if !zeta.is_finite() {
        return (0.0, 0.0);
    }
    if zeta < 1.0 - 1e-5 {
        let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
        if omega_d == 0.0 || !omega_d.is_finite() {
            return critically_damped(x0, v0, omega0, t);
        }
        let a = x0;
        let b = (v0 + zeta * omega0 * x0) / omega_d;
        let exp = (-zeta * omega0 * t).exp();
        let (sin, cos) = (omega_d * t).sin_cos();
        let x = exp * (a * cos + b * sin);
        let v = -zeta * omega0 * x + exp * (-a * omega_d * sin + b * omega_d * cos);
        (x, v)
    } else if zeta > 1.0 + 1e-5 {
        let disc = (zeta * zeta - 1.0).sqrt();
        let r1 = -omega0 * (zeta - disc);
        let r2 = -omega0 * (zeta + disc);
        if (r1 - r2).abs() <= f32::EPSILON {
            return critically_damped(x0, v0, omega0, t);
        }
        let a = (v0 - r2 * x0) / (r1 - r2);
        let b = x0 - a;
        let e1 = (r1 * t).exp();
        let e2 = (r2 * t).exp();
        (a * e1 + b * e2, a * r1 * e1 + b * r2 * e2)
    } else {
        critically_damped(x0, v0, omega0, t)
    }
}

fn critically_damped(x0: f32, v0: f32, omega0: f32, t: f32) -> (f32, f32) {
    let a = x0;
    let b = v0 + omega0 * x0;
    let exp = (-omega0 * t).exp();
    let x = (a + b * t) * exp;
    let v = b * exp + (a + b * t) * (-omega0) * exp;
    (x, v)
}

pub(super) fn spring_settle_duration(
    from: MotionValue,
    rest: MotionValue,
    velocity: MotionValue,
    params: SpringParams,
) -> Option<Duration> {
    let from = physics_numeric(from);
    let rest = physics_numeric(rest);
    let velocity = physics_numeric(velocity);
    // Bound the search; underdamped springs settle exponentially.
    let mut lo = 0.0_f32;
    let mut hi = 16.0_f32;
    for _ in 0..20 {
        let (value, vel) = spring_state(from, rest, velocity, params, hi);
        if is_settled(value, rest, vel) {
            break;
        }
        hi *= 2.0;
        if hi > 120.0 {
            break;
        }
    }
    for _ in 0..32 {
        let mid = 0.5 * (lo + hi);
        let (value, vel) = spring_state(from, rest, velocity, params, mid);
        if is_settled(value, rest, vel) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(Duration::from_secs_f32(hi.max(0.0)))
}

pub(super) fn decay_settle_duration(
    velocity: MotionValue,
    params: DecayParams,
) -> Option<Duration> {
    if params.time_constant <= 0.0 {
        return Some(Duration::ZERO);
    }
    // |v0| e^{-t/τ} <= SETTLE_VELOCITY
    let v0 = max_abs(physics_numeric(velocity));
    if v0 <= SETTLE_VELOCITY {
        return Some(Duration::ZERO);
    }
    let t = params.time_constant * (v0 / SETTLE_VELOCITY).ln();
    // `from_secs_f32` panics on a non-finite or out-of-range value, and an
    // infinite velocity is one division by a zero timestep away — a fling
    // whose delta was measured over no time at all. No deadline is the honest
    // answer; a panic in the frame loop is not.
    if !t.is_finite() || t > MAX_SETTLE_SECONDS {
        return None;
    }
    Some(Duration::from_secs_f32(t.max(0.0)))
}

/// Compile a retarget: new `from` / `velocity` are the presentation sample at
/// `now`. Logical rest becomes `to`. Same track id.
pub fn retarget_track(track: &MotionTrack, now: Duration, to: MotionValue) -> MotionTrack {
    let sample = evaluate_track(track, now);
    let mut next = track.clone();
    next.from = sample.value;
    next.to = MotionTo::Value(to);
    next.velocity = sample.velocity;
    next.timing.start = now;
    next.timing.delay = Duration::ZERO;
    next.playback.play_state = AnimationPlayState::Running;
    next.playback.paused_at = None;
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{
        AnimatableProperty, AnimationClass, AnimationDirection, AnimationFillMode,
        AnimationIteration, AnimationPlayback, MotionTargetId, MotionTiming, MotionTrackId,
        StepJump,
    };

    fn track(curve: MotionCurve) -> MotionTrack {
        MotionTrack::transition(
            MotionTrackId::new(1).unwrap(),
            MotionTargetId::new(1).unwrap(),
            AnimatableProperty::Opacity,
            MotionValue::Scalar(0.0),
            MotionValue::Scalar(1.0),
            MotionTiming::new(
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
            ),
            curve,
            AnimationPlayback::default(),
        )
    }

    /// An overshooting curve is the whole reason `cubic-bezier` accepts y
    /// control points outside 0..1; a clamped interpolation deletes it and
    /// leaves `progress` and `value` describing different animations.
    #[test]
    fn an_overshooting_curve_carries_the_overshoot_into_the_value() {
        let mut track = track(MotionCurve::Easing(Easing::CubicBezier([
            0.34, 1.56, 0.64, 1.0,
        ])));
        track.from = MotionValue::Scalar(0.0);
        track.to = MotionTo::Value(MotionValue::Scalar(100.0));
        let sample = evaluate_track(&track, Duration::from_millis(60));
        assert!(
            sample.progress > 1.0,
            "ease-out-back overshoots by t = 0.6: {}",
            sample.progress
        );
        let MotionValue::Scalar(value) = sample.value else {
            panic!("a scalar track samples a scalar");
        };
        assert!(
            (value - sample.progress * 100.0).abs() < 1e-3,
            "value {value} does not follow progress {}",
            sample.progress
        );
        assert!(value > 100.0, "the overshoot never reached the value");

        // The end of the same animation still lands exactly on `to`.
        let end = evaluate_track(&track, Duration::from_millis(100));
        assert_eq!(end.value, MotionValue::Scalar(100.0));
    }

    /// A colour is the one value that does not extrapolate: a channel past its
    /// own range is not a colour, so it clamps the way a browser clamps an
    /// interpolated colour to the gamut.
    #[test]
    fn an_overshooting_curve_still_leaves_a_colour_in_range() {
        let mut track = track(MotionCurve::Easing(Easing::CubicBezier([
            0.34, 1.56, 0.64, 1.0,
        ])));
        track.from = MotionValue::Color([0.0, 0.0, 0.0, 0.0]);
        track.to = MotionTo::Value(MotionValue::Color([1.0, 0.5, 0.25, 1.0]));
        let sample = evaluate_track(&track, Duration::from_millis(60));
        let MotionValue::Color(channels) = sample.value else {
            panic!("a colour track samples a colour");
        };
        for channel in channels {
            assert!(
                (0.0..=1.0).contains(&channel),
                "channel {channel} left the gamut: {channels:?}"
            );
        }
    }

    /// CSS gives a keyframe's timing function to the interval that *starts* at
    /// it. Reading the ending stop's shifts every per-stop easing one interval
    /// late and applies the last stop's, which governs nothing at all.
    #[test]
    fn a_keyframe_easing_governs_the_interval_that_starts_at_it() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.from = MotionValue::Scalar(0.0);
        track.to = MotionTo::Keyframes(vec![
            Keyframe {
                offset: 0.5,
                value: MotionValue::Scalar(10.0),
                easing: Some(Easing::EaseOutCubic),
            },
            Keyframe {
                offset: 1.0,
                value: MotionValue::Scalar(20.0),
                easing: None,
            },
        ]);

        // Half-way through the second interval, eased by the stop that opens
        // it: ease-out-cubic(0.5) = 0.875 -> 10 + 10 * 0.875.
        let sample = evaluate_track(&track, Duration::from_millis(75));
        let MotionValue::Scalar(value) = sample.value else {
            panic!("a scalar track samples a scalar");
        };
        let expected = 10.0 + 10.0 * Easing::EaseOutCubic.sample(0.5);
        assert!(
            (value - expected).abs() < 1e-4,
            "{value} is not {expected}: the easing landed on the wrong interval"
        );

        // The first interval runs on the track's curve, because the implicit
        // keyframe at offset 0 declares none.
        let first = evaluate_track(&track, Duration::from_millis(25));
        assert_eq!(first.value, MotionValue::Scalar(5.0));
    }

    /// A fling whose velocity was measured over no time at all arrives as
    /// `inf`, and `Duration::from_secs_f32` panics on it.
    #[test]
    fn a_non_finite_decay_velocity_has_no_deadline_instead_of_a_panic() {
        for velocity in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            let settle = decay_settle_duration(
                MotionValue::Scalar(velocity),
                DecayParams { time_constant: 0.3 },
            );
            assert_eq!(settle, None, "velocity {velocity} produced a deadline");
        }
        let sane = decay_settle_duration(
            MotionValue::Scalar(400.0),
            DecayParams { time_constant: 0.3 },
        );
        assert!(sane.is_some_and(|settle| settle > Duration::ZERO));
    }

    #[test]
    fn cubic_bezier_midpoint_is_deterministic() {
        let track = track(MotionCurve::Easing(Easing::CubicBezier([
            0.2, 0.8, 0.2, 1.0,
        ])));
        let a = evaluate_track(&track, Duration::from_millis(50));
        let b = evaluate_track(&track, Duration::from_millis(50));
        assert_eq!(a, b);
        let expected = Easing::MENU_POP.sample(0.5);
        assert!((a.progress - expected).abs() < 1e-6);
    }

    #[test]
    fn steps_jump_end_holds_until_the_next_notch() {
        let track = track(MotionCurve::Steps {
            count: 2,
            jump: StepJump::End,
        });
        let early = evaluate_track(&track, Duration::from_millis(49));
        let mid = evaluate_track(&track, Duration::from_millis(50));
        let end = evaluate_track(&track, Duration::from_millis(100));
        assert_eq!(early.progress, 0.0);
        assert_eq!(mid.progress, 0.5);
        assert_eq!(end.progress, 1.0);
        assert!(end.finished);
    }

    #[test]
    fn keyframes_lerp_between_stops() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.to = MotionTo::Keyframes(vec![
            Keyframe {
                offset: 0.0,
                value: MotionValue::Scalar(0.0),
                easing: None,
            },
            Keyframe {
                offset: 0.5,
                value: MotionValue::Scalar(10.0),
                easing: None,
            },
            Keyframe {
                offset: 1.0,
                value: MotionValue::Scalar(20.0),
                easing: None,
            },
        ]);
        let mid = evaluate_track(&track, Duration::from_millis(25));
        match mid.value {
            MotionValue::Scalar(v) => assert!((v - 5.0).abs() < 1e-5),
            other => panic!("expected scalar, got {other:?}"),
        }
        let three_quarter = evaluate_track(&track, Duration::from_millis(75));
        match three_quarter.value {
            MotionValue::Scalar(v) => assert!((v - 15.0).abs() < 1e-5),
            other => panic!("expected scalar, got {other:?}"),
        }
    }

    fn scalar(sample: &MotionSample) -> f32 {
        match sample.value {
            MotionValue::Scalar(v) => v,
            other => panic!("expected scalar, got {other:?}"),
        }
    }

    fn spring_track(params: SpringParams, from: f32, rest: f32, velocity: f32) -> MotionTrack {
        let mut track = track(MotionCurve::Spring(params));
        track.from = MotionValue::Scalar(from);
        track.to = MotionTo::Value(MotionValue::Scalar(rest));
        track.velocity = MotionValue::Scalar(velocity);
        track
    }

    /// Textbook mass-spring in (k, c, m) form. This is not
    /// [`super::damped_harmonic`]: it uses σ / ωn² / discriminant, not ζ.
    fn textbook_mass_spring(x0: f64, v0: f64, k: f64, c: f64, m: f64, t: f64) -> (f64, f64) {
        if t <= 0.0 {
            return (x0, v0);
        }
        if !(m > 0.0 && k > 0.0 && m.is_finite() && k.is_finite()) {
            return (0.0, 0.0);
        }
        let sigma = c / (2.0 * m);
        let wn2 = k / m;
        let disc = sigma * sigma - wn2;
        if disc < -1e-12 {
            let wd = (-disc).sqrt();
            let a = x0;
            let b = (v0 + sigma * x0) / wd;
            let decay = (-sigma * t).exp();
            let angle = wd * t;
            let (sin, cos) = (angle.sin(), angle.cos());
            let x = decay * (a * cos + b * sin);
            let v = -sigma * x + decay * (-a * wd * sin + b * wd * cos);
            (x, v)
        } else if disc > 1e-12 {
            let root = disc.sqrt();
            let r1 = -sigma + root;
            let r2 = -sigma - root;
            let a = (v0 - r2 * x0) / (r1 - r2);
            let b = x0 - a;
            let e1 = (r1 * t).exp();
            let e2 = (r2 * t).exp();
            (a * e1 + b * e2, a * r1 * e1 + b * r2 * e2)
        } else {
            let a = x0;
            let b = v0 + sigma * x0;
            let decay = (-sigma * t).exp();
            (
                (a + b * t) * decay,
                b * decay + (a + b * t) * (-sigma) * decay,
            )
        }
    }

    fn spring_ode_residual_f64(x0: f64, v0: f64, k: f64, c: f64, m: f64, t: f64, h: f64) -> f64 {
        let xm = textbook_mass_spring(x0, v0, k, c, m, (t - h).max(0.0)).0;
        let x = textbook_mass_spring(x0, v0, k, c, m, t).0;
        let xp = textbook_mass_spring(x0, v0, k, c, m, t + h).0;
        let xdot = (xp - xm) / (2.0 * h);
        let xddot = (xp - 2.0 * x + xm) / (h * h);
        m * xddot + c * xdot + k * x
    }

    #[test]
    fn spring_under_critical_and_over_damped_satisfy_the_ode() {
        let cases = [
            SpringParams::new(170.0, 10.0, 1.0),
            SpringParams::new(100.0, 20.0, 1.0),
            SpringParams::new(80.0, 40.0, 1.0),
        ];
        for params in cases {
            let zeta = params.damping / (2.0 * (params.stiffness * params.mass).sqrt());
            let track = spring_track(params, 10.0, 0.0, 4.0);
            let at_start = evaluate_track(&track, Duration::ZERO);
            assert!((scalar(&at_start) - 10.0).abs() < 1e-6);
            match at_start.velocity {
                MotionValue::Scalar(v) => assert!((v - 4.0).abs() < 1e-6),
                other => panic!("{other:?}"),
            }
            let k = f64::from(params.stiffness);
            let c = f64::from(params.damping);
            let m = f64::from(params.mass);
            let residual = spring_ode_residual_f64(10.0, 4.0, k, c, m, 0.12, 1e-4);
            assert!(residual.abs() < 5e-3, "ζ={zeta} residual {residual}");
            for t in [0.05_f64, 0.12, 0.4] {
                let now = Duration::from_secs_f64(t);
                let sample = evaluate_track(&track, now);
                let eval_t = f64::from(now.as_secs_f32());
                let (x, v) = textbook_mass_spring(10.0, 4.0, k, c, m, eval_t);
                assert!(
                    (f64::from(scalar(&sample)) - x).abs() < 2e-4,
                    "ζ={zeta} t={t} pos"
                );
                match sample.velocity {
                    MotionValue::Scalar(got) => {
                        assert!((f64::from(got) - v).abs() < 2e-3, "ζ={zeta} t={t} vel");
                    }
                    other => panic!("{other:?}"),
                }
            }
            let late = evaluate_track(&track, Duration::from_secs(8));
            assert!((scalar(&late) - 0.0).abs() < 0.02);
            assert!(late.finished);
        }
    }

    #[test]
    fn spring_illegal_parameters_stay_finite() {
        for params in [
            SpringParams::new(0.0, 1.0, 1.0),
            SpringParams::new(-4.0, 1.0, 1.0),
            SpringParams::new(10.0, f32::NAN, 1.0),
            SpringParams::new(10.0, 1.0, 0.0),
        ] {
            let track = spring_track(params, 3.0, 1.0, 2.0);
            let sample = evaluate_track(&track, Duration::from_millis(40));
            match (sample.value, sample.velocity) {
                (MotionValue::Scalar(x), MotionValue::Scalar(v)) => {
                    assert!(x.is_finite(), "{x}");
                    assert!(v.is_finite(), "{v}");
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn spring_at_rest_stays_at_target() {
        let mut track = track(MotionCurve::Spring(SpringParams::new(170.0, 26.0, 1.0)));
        track.from = MotionValue::Scalar(1.0);
        track.to = MotionTo::Value(MotionValue::Scalar(1.0));
        track.velocity = MotionValue::Scalar(0.0);
        let sample = evaluate_track(&track, Duration::from_secs(2));
        match sample.value {
            MotionValue::Scalar(v) => assert!((v - 1.0).abs() < SETTLE_POSITION),
            other => panic!("{other:?}"),
        }
        assert!(sample.finished);
    }

    #[test]
    fn decay_random_access_is_closed_form() {
        let mut track = track(MotionCurve::Decay(DecayParams::new(0.2)));
        track.from = MotionValue::Scalar(0.0);
        track.velocity = MotionValue::Scalar(100.0);
        let t = 0.5_f32;
        let sample = evaluate_track(&track, Duration::from_secs_f32(t));
        let tau = 0.2_f32;
        let expected = 100.0 * tau * (1.0 - (-t / tau).exp());
        match sample.value {
            MotionValue::Scalar(v) => assert!((v - expected).abs() < 1e-4),
            other => panic!("{other:?}"),
        }
        let again = evaluate_track(&track, Duration::from_secs_f32(t));
        assert_eq!(sample, again);
    }

    #[test]
    fn minimize_resume_does_not_need_to_catch_up_frames() {
        let params = SpringParams::new(120.0, 12.0, 1.0);
        let track = spring_track(params, 0.0, 8.0, 4.0);
        let after_gap = evaluate_track(&track, Duration::from_millis(2000));
        let again = evaluate_track(&track, Duration::from_millis(2000));
        assert_eq!(after_gap, again);
        assert!((scalar(&after_gap) - 8.0).abs() < 0.05);
        let at_200ms = Duration::from_millis(200);
        let (x, _) = textbook_mass_spring(
            -8.0,
            4.0,
            f64::from(params.stiffness),
            f64::from(params.damping),
            f64::from(params.mass),
            f64::from(at_200ms.as_secs_f32()),
        );
        let sample = evaluate_track(&track, at_200ms);
        assert!((f64::from(scalar(&sample)) - (8.0 + x)).abs() < 2e-4);
    }

    #[test]
    fn retarget_continues_from_current_presentation() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.from = MotionValue::Scalar(0.0);
        track.to = MotionTo::Value(MotionValue::Scalar(10.0));
        let mid = evaluate_track(&track, Duration::from_millis(50));
        match mid.value {
            MotionValue::Scalar(v) => assert!((v - 5.0).abs() < 1e-5),
            other => panic!("{other:?}"),
        }
        let next = retarget_track(&track, Duration::from_millis(50), MotionValue::Scalar(0.0));
        match next.from {
            MotionValue::Scalar(v) => assert!((v - 5.0).abs() < 1e-5),
            other => panic!("{other:?}"),
        }
        let after = evaluate_track(&next, Duration::from_millis(100));
        match after.value {
            MotionValue::Scalar(v) => assert!((v - 2.5).abs() < 1e-4),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn delay_holds_start_progress_until_effective_start() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.timing.delay = Duration::from_millis(40);
        let held = evaluate_track(&track, Duration::from_millis(20));
        assert_eq!(held.progress, 0.0);
        assert!(!held.finished);
        let mid = evaluate_track(&track, Duration::from_millis(90));
        assert!((mid.progress - 0.5).abs() < 1e-5);
    }

    #[test]
    fn reverse_and_fill_match_runtime_playback_contract() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.playback.direction = AnimationDirection::Reverse;
        track.playback.fill_mode = AnimationFillMode::Backwards;
        track.timing.start = Duration::from_millis(50);
        let held = evaluate_track(&track, Duration::from_millis(10));
        assert!((held.progress - 1.0).abs() < f32::EPSILON);
        assert!(!held.finished);
        track.playback.iteration_count = AnimationIteration::Count(2);
        track.playback.direction = AnimationDirection::Alternate;
        track.timing.start = Duration::ZERO;
        let second = evaluate_track(&track, Duration::from_millis(125));
        assert!((second.progress - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn paused_track_uses_hold_clock() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.playback.play_state = AnimationPlayState::Paused;
        let held = evaluate_track_at(&track, Duration::from_millis(50), Some(Duration::ZERO));
        assert_eq!(held.progress, 0.0);
        assert!(!held.finished);
    }

    #[test]
    fn pause_at_midpoint_freezes_without_an_external_clock() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.pause_at(Duration::from_millis(50));
        let frozen = evaluate_track(&track, Duration::from_millis(90));
        assert!((frozen.progress - 0.5).abs() < 1e-5);
        assert!(!frozen.finished);
        track.resume();
        let resumed = evaluate_track(&track, Duration::from_millis(90));
        assert!((resumed.progress - 0.9).abs() < 1e-5);
    }

    #[test]
    fn evaluate_progress_uses_the_same_pause_clock_as_evaluate_track() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.pause_at(Duration::from_millis(50));
        let now = Duration::from_millis(90);
        let progress = evaluate_progress(track.timing, track.playback, track.curve, now);
        let motion = evaluate_track(&track, now);
        assert!((progress.progress - 0.5).abs() < 1e-5);
        assert_eq!(progress.progress, motion.progress);
        assert_eq!(progress.finished, motion.finished);
        assert_eq!(progress.applies, motion.applies);
        track.playback.paused_at = Some(Duration::from_millis(25));
        let moved = evaluate_progress(track.timing, track.playback, track.curve, now);
        assert!((moved.progress - 0.25).abs() < 1e-5);
        assert_eq!(moved.progress, evaluate_track(&track, now).progress);
    }

    #[test]
    fn fill_none_reverse_does_not_apply_before_start_or_after_end() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.playback.direction = AnimationDirection::Reverse;
        track.playback.fill_mode = AnimationFillMode::None;
        track.timing.start = Duration::from_millis(50);
        let delay = evaluate_track(&track, Duration::from_millis(10));
        assert!(!delay.applies);
        assert!(!delay.finished);
        assert_eq!(scalar(&delay), 0.0);
        let after = evaluate_track(&track, Duration::from_millis(200));
        assert!(!after.applies);
        assert!(after.finished);
        assert_eq!(scalar(&after), 1.0);
    }

    #[test]
    fn fill_backwards_and_forwards_apply_at_delay_and_after() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.playback.direction = AnimationDirection::Reverse;
        track.playback.fill_mode = AnimationFillMode::Both;
        track.timing.start = Duration::from_millis(50);
        let delay = evaluate_track(&track, Duration::from_millis(10));
        assert!(delay.applies);
        assert!((delay.progress - 1.0).abs() < f32::EPSILON);
        assert_eq!(scalar(&delay), 1.0);
        let after = evaluate_track(&track, Duration::from_millis(200));
        assert!(after.applies);
        assert!(after.finished);
        assert_eq!(after.progress, 0.0);
        assert_eq!(scalar(&after), 0.0);
    }

    #[test]
    fn fill_modes_none_backwards_forwards_at_any_timestamp_including_reverse() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.playback.direction = AnimationDirection::Reverse;
        track.timing.start = Duration::from_millis(50);
        let delay = Duration::from_millis(10);
        let mid = Duration::from_millis(100);
        let after = Duration::from_millis(200);

        track.playback.fill_mode = AnimationFillMode::None;
        assert!(!evaluate_track(&track, delay).applies);
        assert!(evaluate_track(&track, mid).applies);
        assert!(!evaluate_track(&track, after).applies);

        track.playback.fill_mode = AnimationFillMode::Backwards;
        let held = evaluate_track(&track, delay);
        assert!(held.applies);
        assert!((held.progress - 1.0).abs() < f32::EPSILON);
        assert!(evaluate_track(&track, mid).applies);
        assert!(!evaluate_track(&track, after).applies);

        track.playback.fill_mode = AnimationFillMode::Forwards;
        assert!(!evaluate_track(&track, delay).applies);
        assert!(evaluate_track(&track, mid).applies);
        let done = evaluate_track(&track, after);
        assert!(done.applies);
        assert!(done.finished);
        assert!((done.progress - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn discrete_display_snaps_and_does_not_lerp_scalars() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.property = AnimatableProperty::Display;
        track.from = MotionValue::Scalar(0.0);
        track.to = MotionTo::Value(MotionValue::Scalar(1.0));
        let mid = evaluate_track(&track, Duration::from_millis(50));
        assert_eq!(mid.execution_class(), AnimationClass::Discrete);
        assert_eq!(scalar(&mid), 0.0);
        let end = evaluate_track(&track, Duration::from_millis(100));
        assert_eq!(scalar(&end), 1.0);
        track.from = MotionValue::Discrete(2);
        track.to = MotionTo::Value(MotionValue::Discrete(7));
        track.velocity = MotionValue::Discrete(2);
        assert!(track.is_valid());
        let held = evaluate_track(&track, Duration::from_millis(40));
        match held.value {
            MotionValue::Discrete(tag) => assert_eq!(tag, 2),
            other => panic!("{other:?}"),
        }
        let done = evaluate_track(&track, Duration::from_millis(100));
        match done.value {
            MotionValue::Discrete(tag) => assert_eq!(tag, 7),
            other => panic!("{other:?}"),
        }
    }

    fn discrete_tag(sample: &MotionSample) -> u32 {
        match sample.value {
            MotionValue::Discrete(tag) => tag,
            other => panic!("expected discrete, got {other:?}"),
        }
    }

    fn display_track(direction: AnimationDirection, iterations: AnimationIteration) -> MotionTrack {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.property = AnimatableProperty::Display;
        track.from = MotionValue::Discrete(2);
        track.to = MotionTo::Value(MotionValue::Discrete(7));
        track.velocity = MotionValue::Discrete(2);
        track.playback.direction = direction;
        track.playback.iteration_count = iterations;
        assert!(track.is_valid());
        track
    }

    #[test]
    fn discrete_reverse_snaps_only_when_the_run_finishes() {
        let track = display_track(AnimationDirection::Reverse, AnimationIteration::ONCE);
        let start = evaluate_track(&track, Duration::ZERO);
        assert!(!start.finished);
        assert!((start.progress - 1.0).abs() < f32::EPSILON);
        assert_eq!(discrete_tag(&start), 2);
        let mid = evaluate_track(&track, Duration::from_millis(50));
        assert!(!mid.finished);
        assert_eq!(discrete_tag(&mid), 2);
        let end = evaluate_track(&track, Duration::from_millis(100));
        assert!(end.finished);
        assert_eq!(discrete_tag(&end), 7);
    }

    #[test]
    fn discrete_alternate_holds_from_until_the_run_finishes() {
        let track = display_track(AnimationDirection::Alternate, AnimationIteration::Count(2));
        let first_end = evaluate_track(&track, Duration::from_millis(100));
        assert!(!first_end.finished);
        assert!((first_end.progress - 1.0).abs() < f32::EPSILON);
        assert_eq!(discrete_tag(&first_end), 2);
        let second_mid = evaluate_track(&track, Duration::from_millis(150));
        assert!(!second_mid.finished);
        assert_eq!(discrete_tag(&second_mid), 2);
        let done = evaluate_track(&track, Duration::from_millis(200));
        assert!(done.finished);
        assert_eq!(discrete_tag(&done), 7);
    }

    #[test]
    fn discrete_spring_holds_from_until_settle() {
        let params = SpringParams::new(170.0, 26.0, 1.0);
        let mut track = display_track(AnimationDirection::Normal, AnimationIteration::ONCE);
        track.curve = MotionCurve::Spring(params);
        track.velocity = MotionValue::Discrete(0);
        assert!(track.is_valid());
        let settle =
            super::spring_settle_duration(track.from, track.rest_value(), track.velocity, params)
                .expect("settle");
        assert!(
            settle > Duration::from_millis(1),
            "settle {settle:?} must outlast ε so t>0 snap would fail"
        );
        let early = evaluate_track(&track, Duration::from_millis(1));
        assert!(!early.finished);
        assert_eq!(discrete_tag(&early), 2);
        let done = evaluate_track(&track, settle + Duration::from_millis(1));
        assert!(done.finished);
        assert_eq!(discrete_tag(&done), 7);
    }

    #[test]
    fn discrete_decay_holds_from_until_settle() {
        let params = DecayParams::new(0.2);
        let mut track = display_track(AnimationDirection::Normal, AnimationIteration::ONCE);
        track.curve = MotionCurve::Decay(params);
        track.velocity = MotionValue::Discrete(80);
        assert!(track.is_valid());
        let settle = super::decay_settle_duration(track.velocity, params).expect("settle");
        assert!(
            settle > Duration::from_millis(1),
            "settle {settle:?} must outlast ε so t>0 snap would fail"
        );
        let early = evaluate_track(&track, Duration::from_millis(1));
        assert!(!early.finished);
        assert_eq!(discrete_tag(&early), 2);
        let done = evaluate_track(&track, settle + Duration::from_millis(1));
        assert!(done.finished);
        assert_eq!(discrete_tag(&done), 7);
    }

    #[test]
    fn keyframes_empty_exact_stop_and_unsorted_contract() {
        let mut track = track(MotionCurve::Easing(Easing::Linear));
        track.to = MotionTo::Keyframes(Vec::new());
        assert_eq!(
            scalar(&evaluate_track(&track, Duration::from_millis(40))),
            0.0
        );

        track.to = MotionTo::Keyframes(vec![
            Keyframe {
                offset: 0.0,
                value: MotionValue::Scalar(0.0),
                easing: None,
            },
            Keyframe {
                offset: 0.4,
                value: MotionValue::Scalar(4.0),
                easing: None,
            },
            Keyframe {
                offset: 1.0,
                value: MotionValue::Scalar(10.0),
                easing: None,
            },
        ]);
        assert!(track.is_valid());
        assert_eq!(
            scalar(&evaluate_track(&track, Duration::from_millis(40))),
            4.0
        );

        track.to = MotionTo::Keyframes(vec![
            Keyframe {
                offset: 0.9,
                value: MotionValue::Scalar(9.0),
                easing: None,
            },
            Keyframe {
                offset: 0.1,
                value: MotionValue::Scalar(1.0),
                easing: None,
            },
        ]);
        assert!(!track.is_valid());
    }

    #[test]
    fn retarget_spring_keeps_presentation_and_velocity() {
        let params = SpringParams::new(140.0, 18.0, 1.0);
        let track = spring_track(params, 0.0, 10.0, 0.0);
        let now = Duration::from_millis(80);
        let mid = evaluate_track(&track, now);
        let current = scalar(&mid);
        assert!(current > 0.0 && current < 10.0);
        let next = retarget_track(&track, now, MotionValue::Scalar(0.0));
        match next.from {
            MotionValue::Scalar(v) => assert!((v - current).abs() < 1e-5),
            other => panic!("{other:?}"),
        }
        match (mid.velocity, next.velocity) {
            (MotionValue::Scalar(a), MotionValue::Scalar(b)) => {
                assert!((a - b).abs() < 1e-5);
                assert!(a.abs() > 0.0);
            }
            other => panic!("{other:?}"),
        }
        let later = evaluate_track(&next, now + Duration::from_secs(6));
        assert!((scalar(&later) - 0.0).abs() < 0.05);
        assert!(later.finished);
    }

    #[test]
    fn same_timestamp_is_bit_identical() {
        let mut track = track(MotionCurve::Spring(SpringParams::new(90.0, 20.0, 1.0)));
        track.from = MotionValue::Scalar(3.0);
        track.velocity = MotionValue::Scalar(-1.5);
        let now = Duration::from_millis(333);
        assert_eq!(evaluate_track(&track, now), evaluate_track(&track, now));
    }
}
