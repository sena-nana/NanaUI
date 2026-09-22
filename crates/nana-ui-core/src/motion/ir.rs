//! Unified Motion IR. Authoring helpers (sequence / parallel) compile to
//! absolute-time [`MotionTrack`]s; evaluation always uses the host clock.

use std::time::Duration;

use crate::PaintTransform;

use super::{
    easing::Easing,
    playback::{AnimationPlayState, AnimationPlayback, MotionTiming},
    property::{AnimatableProperty, AnimationClass},
};

/// Stable identity for one logical track. Starting the same ID again
/// atomically replaces that track (Runtime `AnimationSpec` uses this as
/// `AnimationId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MotionTrackId(u64);

impl MotionTrackId {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Node the track writes presentation for. Runtime maps `StableNodeId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MotionTargetId(u64);

impl MotionTargetId {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Interpolable (or discrete) presentation value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionValue {
    Scalar(f32),
    Color([f32; 4]),
    Transform(PaintTransform),
    /// Topology / `display`. Snaps at the end of the timed run.
    Discrete(u32),
}

impl MotionValue {
    /// A font-axis keyframe for a list that does not name the axis: the
    /// axis is left out, so the face's default applies. Only a font-axis
    /// keyframe list carries it.
    pub const ABSENT: Self = Self::Scalar(f32::NAN);

    pub fn is_absent(self) -> bool {
        matches!(self, Self::Scalar(v) if v.is_nan())
    }

    pub fn zero_velocity(self) -> Self {
        match self {
            Self::Scalar(_) => Self::Scalar(0.0),
            Self::Color(_) => Self::Color([0.0; 4]),
            Self::Transform(_) => Self::Transform(PaintTransform {
                a: 0.0,
                b: 0.0,
                c: 0.0,
                d: 0.0,
                e: 0.0,
                f: 0.0,
            }),
            Self::Discrete(tag) => Self::Discrete(tag),
        }
    }

    /// Interpolates at `t`, which is the **eased** progress and may therefore
    /// leave `0..=1`.
    ///
    /// Clamping it here would silently delete the overshoot that is the whole
    /// point of a `cubic-bezier` whose control points reach past 1 (every
    /// "back" / anticipation curve), and would leave
    /// [`MotionSample::progress`] disagreeing with [`MotionSample::value`].
    /// Springs already overshoot, because the physics path does not come
    /// through here at all.
    ///
    /// Colour is the one exception: a channel is clamped to its own range
    /// after interpolating, the way a browser clamps an interpolated colour to
    /// the gamut rather than refusing to overshoot.
    pub fn lerp(self, to: Self, t: f32) -> Self {
        match (self, to) {
            (Self::Scalar(a), Self::Scalar(b)) => Self::Scalar(a + (b - a) * t),
            (Self::Color(a), Self::Color(b)) => Self::Color(std::array::from_fn(|i| {
                (a[i] + (b[i] - a[i]) * t).clamp(0.0, 1.0)
            })),
            (Self::Transform(a), Self::Transform(b)) => Self::Transform(PaintTransform {
                a: a.a + (b.a - a.a) * t,
                b: a.b + (b.b - a.b) * t,
                c: a.c + (b.c - a.c) * t,
                d: a.d + (b.d - a.d) * t,
                e: a.e + (b.e - a.e) * t,
                f: a.f + (b.f - a.f) * t,
            }),
            (from, to) => {
                if t >= 1.0 {
                    to
                } else {
                    from
                }
            }
        }
    }

    pub fn scale(self, k: f32) -> Self {
        match self {
            Self::Scalar(v) => Self::Scalar(v * k),
            Self::Color(c) => Self::Color(std::array::from_fn(|i| c[i] * k)),
            Self::Transform(p) => Self::Transform(PaintTransform {
                a: p.a * k,
                b: p.b * k,
                c: p.c * k,
                d: p.d * k,
                e: p.e * k,
                f: p.f * k,
            }),
            discrete @ Self::Discrete(_) => discrete,
        }
    }
}

impl std::ops::Add for MotionValue {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        match (self, other) {
            (Self::Scalar(a), Self::Scalar(b)) => Self::Scalar(a + b),
            (Self::Color(a), Self::Color(b)) => Self::Color(std::array::from_fn(|i| a[i] + b[i])),
            (Self::Transform(a), Self::Transform(b)) => Self::Transform(PaintTransform {
                a: a.a + b.a,
                b: a.b + b.b,
                c: a.c + b.c,
                d: a.d + b.d,
                e: a.e + b.e,
                f: a.f + b.f,
            }),
            (left, _) => left,
        }
    }
}

impl std::ops::Sub for MotionValue {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        self + other.scale(-1.0)
    }
}

/// Destination of a track: a single rest/target value or keyframe stops.
#[derive(Debug, Clone, PartialEq)]
pub enum MotionTo {
    Value(MotionValue),
    Keyframes(Vec<Keyframe>),
}

/// One keyframe stop. `offset` is in `0.0..=1.0`. Optional per-stop easing
/// applies inside that interval; otherwise the track's easing curve is used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keyframe {
    pub offset: f32,
    pub value: MotionValue,
    pub easing: Option<Easing>,
}

/// CSS `steps()` jump term.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepJump {
    Start,
    End,
    None,
    Both,
}

/// Curve / physics driver. Spring and decay are first-class: they evaluate
/// as `f(start, velocity, params, absolute_time)`, never by integrating the
/// previous frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionCurve {
    Easing(Easing),
    Steps { count: u32, jump: StepJump },
    Spring(SpringParams),
    Decay(DecayParams),
}

impl MotionCurve {
    pub fn sample_progress(self, linear: f32) -> f32 {
        let linear = linear.clamp(0.0, 1.0);
        match self {
            Self::Easing(easing) => easing.sample(linear),
            Self::Steps { count, jump } => sample_steps(linear, count, jump),
            Self::Spring(_) | Self::Decay(_) => linear,
        }
    }

    pub fn is_physics(self) -> bool {
        matches!(self, Self::Spring(_) | Self::Decay(_))
    }
}

/// Damped harmonic oscillator. Units: stiffness (N/m), damping (N·s/m),
/// mass (kg). Rest target is [`MotionTrack::to`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringParams {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
}

impl SpringParams {
    pub const fn new(stiffness: f32, damping: f32, mass: f32) -> Self {
        Self {
            stiffness,
            damping,
            mass,
        }
    }
}

/// Exponential inertial decay: `v(t) = v0 e^{-t/τ}`,
/// `x(t) = x0 + v0 τ (1 - e^{-t/τ})`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecayParams {
    /// Time constant τ in seconds. Must be > 0.
    pub time_constant: f32,
}

impl DecayParams {
    pub const fn new(time_constant: f32) -> Self {
        Self { time_constant }
    }
}

fn sample_steps(progress: f32, count: u32, jump: StepJump) -> f32 {
    if count == 0 {
        return progress;
    }
    let n = count as f32;
    let p = progress.clamp(0.0, 1.0);
    match jump {
        StepJump::End => {
            if p >= 1.0 {
                1.0
            } else {
                (p * n).floor() / n
            }
        }
        StepJump::Start => {
            // The CSS easing algorithm increments the current step for
            // `jump-start`, so input 0 is already 1/n — a `steps(4,
            // jump-start)` that rendered 0 on its first frame would have five
            // notches, not four. `ceil` also lands a step low exactly on a
            // boundary.
            (((p * n).floor() + 1.0) / n).min(1.0)
        }
        StepJump::None => {
            if count <= 1 {
                if p >= 1.0 { 1.0 } else { 0.0 }
            } else if p >= 1.0 {
                1.0
            } else {
                (p * (n - 1.0)).floor() / (n - 1.0)
            }
        }
        StepJump::Both => {
            if p >= 1.0 {
                1.0
            } else {
                ((p * n).floor() + 1.0) / (n + 1.0)
            }
        }
    }
}

/// How a same-id start replaces an in-flight track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MotionInterrupt {
    /// Drop the previous timeline (current Runtime `start_animation`).
    #[default]
    Replace,
    /// Continue from the evaluated presentation value and velocity at `now`.
    Retarget,
}

/// One property on one target. This is the only timeline record; sequence
/// and parallel only assign absolute [`MotionTiming::start`] values.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionTrack {
    pub id: MotionTrackId,
    pub target: MotionTargetId,
    pub property: AnimatableProperty,
    pub from: MotionValue,
    pub to: MotionTo,
    pub timing: MotionTiming,
    pub curve: MotionCurve,
    pub playback: AnimationPlayback,
    /// Initial velocity at `timing.effective_start`, same variant as `from`.
    pub velocity: MotionValue,
}

impl MotionTrack {
    #[allow(clippy::too_many_arguments)]
    pub fn transition(
        id: MotionTrackId,
        target: MotionTargetId,
        property: AnimatableProperty,
        from: MotionValue,
        to: MotionValue,
        timing: MotionTiming,
        curve: MotionCurve,
        playback: AnimationPlayback,
    ) -> Self {
        Self {
            id,
            target,
            property,
            from,
            to: MotionTo::Value(to),
            timing,
            curve,
            playback,
            velocity: from.zero_velocity(),
        }
    }

    /// Framework contract: class is derived from [`Self::property`], never stored.
    pub fn execution_class(&self) -> AnimationClass {
        self.property.animation_class()
    }

    pub fn pause_at(&mut self, at: Duration) {
        self.playback.play_state = AnimationPlayState::Paused;
        self.playback.paused_at = Some(at);
    }

    pub fn resume(&mut self) {
        self.playback.play_state = AnimationPlayState::Running;
        self.playback.paused_at = None;
    }

    pub fn rest_value(&self) -> MotionValue {
        match &self.to {
            MotionTo::Value(value) => *value,
            MotionTo::Keyframes(stops) => stops
                .iter()
                .max_by(|a, b| a.offset.total_cmp(&b.offset))
                .map(|stop| stop.value)
                .unwrap_or(self.from),
        }
    }

    /// Pause uses `hold_at`, then [`AnimationPlayback::paused_at`], then `now`.
    /// It never rewinds to `timing.start` when those are missing.
    pub fn evaluation_clock(&self, now: Duration, hold_at: Option<Duration>) -> Duration {
        self.playback.evaluation_clock(now, hold_at)
    }

    /// Endpoints are checked as the style would check them: the target and
    /// every keyframe must be a value the property accepts (an opacity in
    /// `0..=1`, a finite non-negative width…), so an out-of-range target fails
    /// here rather than being clamped into something the author did not
    /// write. The start only has to be finite: a run retargeted mid-flight
    /// starts wherever an overshooting curve had got to. Samples between the
    /// endpoints are not clamped either — overshoot is what such a curve is
    /// for — each consumer clamps to what it can show.
    pub fn is_valid(&self) -> bool {
        if !value_fits(self.property, self.from) || !start_fits(self.property, self.from) {
            return false;
        }
        if !self.velocity_fits() {
            return false;
        }
        match &self.to {
            MotionTo::Value(to) => {
                if !value_fits(self.property, *to) || !target_fits(self.property, *to, false) {
                    return false;
                }
            }
            MotionTo::Keyframes(stops) => {
                if !keyframes_monotonic(stops)
                    || stops.iter().any(|stop| {
                        !value_fits(self.property, stop.value)
                            || !target_fits(self.property, stop.value, true)
                    })
                {
                    return false;
                }
            }
        }
        match self.curve {
            MotionCurve::Spring(params) => {
                params.mass > 0.0 && params.stiffness > 0.0 && matches!(self.to, MotionTo::Value(_))
            }
            MotionCurve::Decay(params) => params.time_constant > 0.0,
            MotionCurve::Easing(_) | MotionCurve::Steps { .. } => {
                self.timing.is_valid(self.playback)
            }
        }
    }

    fn velocity_fits(&self) -> bool {
        match (self.property.animation_class(), self.velocity) {
            (AnimationClass::Discrete, MotionValue::Discrete(_)) => true,
            (AnimationClass::Discrete, _) => false,
            (_, MotionValue::Discrete(_)) => false,
            _ => {
                value_fits(self.property, self.velocity)
                    || matches!(self.velocity, MotionValue::Scalar(0.0))
            }
        }
    }
}

pub(super) fn value_fits(property: AnimatableProperty, value: MotionValue) -> bool {
    match (property, value) {
        (AnimatableProperty::Display, MotionValue::Discrete(_)) => true,
        (AnimatableProperty::Display, _) => false,
        (AnimatableProperty::Transform, MotionValue::Transform(_)) => true,
        (AnimatableProperty::Color | AnimatableProperty::Background, MotionValue::Color(_)) => true,
        (
            AnimatableProperty::Opacity
            | AnimatableProperty::Clip
            | AnimatableProperty::Blur
            | AnimatableProperty::Filter
            | AnimatableProperty::Shadow
            | AnimatableProperty::ShaderParameter
            | AnimatableProperty::Width
            | AnimatableProperty::Height
            | AnimatableProperty::Padding
            | AnimatableProperty::Margin
            | AnimatableProperty::FontSize
            | AnimatableProperty::FontAxis(_)
            | AnimatableProperty::Progress,
            MotionValue::Scalar(_),
        ) => true,
        _ => false,
    }
}

fn finite(value: MotionValue) -> bool {
    match value {
        MotionValue::Scalar(v) => v.is_finite(),
        MotionValue::Color(channels) => channels.iter().all(|c| c.is_finite()),
        MotionValue::Transform(t) => [t.a, t.b, t.c, t.d, t.e, t.f].iter().all(|v| v.is_finite()),
        MotionValue::Discrete(_) => true,
    }
}

fn absent_axis(property: AnimatableProperty, value: MotionValue) -> bool {
    matches!(property, AnimatableProperty::FontAxis(_)) && value.is_absent()
}

fn start_fits(property: AnimatableProperty, value: MotionValue) -> bool {
    finite(value) || absent_axis(property, value)
}

fn target_fits(property: AnimatableProperty, value: MotionValue, keyframe: bool) -> bool {
    if keyframe && absent_axis(property, value) {
        return true;
    }
    if !finite(value) {
        return false;
    }
    match (property, value) {
        (AnimatableProperty::Opacity, MotionValue::Scalar(v)) => (0.0..=1.0).contains(&v),
        (
            AnimatableProperty::Width
            | AnimatableProperty::Height
            | AnimatableProperty::Padding
            | AnimatableProperty::Blur,
            MotionValue::Scalar(v),
        ) => v >= 0.0,
        (AnimatableProperty::FontSize, MotionValue::Scalar(v)) => v > 0.0,
        (_, MotionValue::Color(channels)) => channels.iter().all(|c| (0.0..=1.0).contains(c)),
        _ => true,
    }
}

pub(super) fn keyframes_monotonic(stops: &[Keyframe]) -> bool {
    stops
        .windows(2)
        .all(|pair| pair[0].offset <= pair[1].offset)
}

/// Authored graph. Compiling yields the same host-clock tracks; it does not
/// create a second timeline authority.
#[derive(Debug, Clone, PartialEq)]
// An authoring tree, compiled once and dropped: boxing every track to shrink
// the combinator variants would cost an allocation per track for nothing.
#[allow(clippy::large_enum_variant)]
pub enum MotionGraph {
    Track(MotionTrack),
    Sequence(Vec<MotionGraph>),
    Parallel(Vec<MotionGraph>),
}

impl MotionGraph {
    pub fn track(track: MotionTrack) -> Self {
        Self::Track(track)
    }

    pub fn sequence(children: Vec<MotionGraph>) -> Self {
        Self::Sequence(children)
    }

    pub fn parallel(children: Vec<MotionGraph>) -> Self {
        Self::Parallel(children)
    }

    /// Assign absolute starts. Sequence children begin when the previous
    /// sibling's resolved run ends; parallel children share `origin`.
    pub fn compile(self) -> Vec<MotionTrack> {
        compile_graph(self, None)
    }
}

/// First-class spring destination. Compiles into a track as
/// `(from, velocity, params, absolute_time)` — not a second timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    to: MotionValue,
    stiffness: f32,
    damping: f32,
    mass: f32,
}

impl Spring {
    /// Rest target. Scalar destinations still need an explicit property
    /// (opacity / width / …) at the Runtime frontend.
    pub fn to(value: impl Into<MotionValue>) -> Self {
        Self {
            to: value.into(),
            stiffness: 170.0,
            damping: 26.0,
            mass: 1.0,
        }
    }

    pub fn stiffness(mut self, stiffness: f32) -> Self {
        self.stiffness = stiffness;
        self
    }

    pub fn damping(mut self, damping: f32) -> Self {
        self.damping = damping;
        self
    }

    pub fn mass(mut self, mass: f32) -> Self {
        self.mass = mass;
        self
    }

    pub fn rest_value(self) -> MotionValue {
        self.to
    }

    pub fn params(self) -> SpringParams {
        SpringParams::new(self.stiffness, self.damping, self.mass)
    }

    /// Property implied by the rest value, if unique. Scalars are ambiguous.
    pub fn implied_property(self) -> Option<AnimatableProperty> {
        match self.to {
            MotionValue::Transform(_) => Some(AnimatableProperty::Transform),
            MotionValue::Color(_) => Some(AnimatableProperty::Color),
            MotionValue::Discrete(_) => Some(AnimatableProperty::Display),
            MotionValue::Scalar(_) => None,
        }
    }
}

/// Authored timeline combinators. Sequence / parallel only assign absolute
/// start times; evaluation stays per-track at the host timestamp.
pub struct Timeline;

impl Timeline {
    pub fn parallel(children: impl IntoIterator<Item = MotionGraph>) -> MotionGraph {
        MotionGraph::parallel(children.into_iter().collect())
    }

    pub fn sequence(children: impl IntoIterator<Item = MotionGraph>) -> MotionGraph {
        MotionGraph::sequence(children.into_iter().collect())
    }
}

impl From<f32> for MotionValue {
    fn from(value: f32) -> Self {
        Self::Scalar(value)
    }
}

impl From<PaintTransform> for MotionValue {
    fn from(value: PaintTransform) -> Self {
        Self::Transform(value)
    }
}

impl From<[f32; 4]> for MotionValue {
    fn from(value: [f32; 4]) -> Self {
        Self::Color(value)
    }
}

/// Logical (UiWorld) vs transient presentation. Overlay storage is
/// [`crate::motion::PresentationStore`]; compositor-safe tracks must not be
/// modeled as per-frame UiWorld writes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationPair {
    pub logical: MotionValue,
    pub presentation: MotionValue,
}

/// Result of evaluating one track at one timestamp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSample {
    pub id: MotionTrackId,
    pub target: MotionTargetId,
    pub property: AnimatableProperty,
    pub value: MotionValue,
    /// Eased unit progress for timed curves; `1.0` when a spring/decay has settled.
    pub progress: f32,
    /// Time derivative of `value` (units per second). Used for retarget.
    pub velocity: MotionValue,
    pub finished: bool,
    /// False when fill-mode does not apply a presentation value at `now`.
    pub applies: bool,
}

impl MotionSample {
    pub fn execution_class(self) -> AnimationClass {
        self.property.animation_class()
    }

    /// Presentation value only when fill-mode applies at this timestamp.
    /// `value` may still hold from/rest when [`Self::applies`] is false.
    pub fn applied_value(self) -> Option<MotionValue> {
        self.applies.then_some(self.value)
    }
}

fn compile_graph(graph: MotionGraph, origin: Option<Duration>) -> Vec<MotionTrack> {
    match graph {
        MotionGraph::Track(mut track) => {
            if let Some(origin) = origin {
                track.timing.start = origin;
            }
            vec![track]
        }
        MotionGraph::Sequence(children) => {
            let mut cursor = origin;
            let mut out = Vec::new();
            for child in children {
                let compiled = compile_graph(child, cursor);
                cursor = compiled.iter().filter_map(track_end).max().or(cursor);
                out.extend(compiled);
            }
            out
        }
        MotionGraph::Parallel(children) => {
            let mut out = Vec::new();
            for child in children {
                out.extend(compile_graph(child, origin));
            }
            out
        }
    }
}

/// Absolute time when a finite track settles or completes its last iteration.
/// `None` for infinite playback. Scheduler completion deadlines use this;
/// compositor tracks must not require a per-frame CPU sample to learn `finished`.
pub fn track_completion_deadline(track: &MotionTrack) -> Option<Duration> {
    track_end(track)
}

pub(super) fn track_end(track: &MotionTrack) -> Option<Duration> {
    match track.curve {
        MotionCurve::Spring(params) => {
            let start = track.timing.effective_start()?;
            let settle = crate::motion::eval::spring_settle_duration(
                track.from,
                track.rest_value(),
                track.velocity,
                params,
            )?;
            start.checked_add(settle)
        }
        MotionCurve::Decay(params) => {
            let start = track.timing.effective_start()?;
            let settle = crate::motion::eval::decay_settle_duration(track.velocity, params)?;
            start.checked_add(settle)
        }
        MotionCurve::Easing(_) | MotionCurve::Steps { .. } => track.timing.end(track.playback),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::AnimationIteration;

    fn id(n: u64) -> MotionTrackId {
        MotionTrackId::new(n).unwrap()
    }

    fn target(n: u64) -> MotionTargetId {
        MotionTargetId::new(n).unwrap()
    }

    fn timed(track_id: u64, start_ms: u64, duration_ms: u64) -> MotionTrack {
        MotionTrack::transition(
            id(track_id),
            target(1),
            AnimatableProperty::Opacity,
            MotionValue::Scalar(0.0),
            MotionValue::Scalar(1.0),
            MotionTiming::new(
                Duration::from_millis(start_ms),
                Duration::from_millis(duration_ms),
                Duration::from_millis(16),
            ),
            MotionCurve::Easing(Easing::Linear),
            AnimationPlayback::default(),
        )
    }

    #[test]
    fn sequence_assigns_absolute_starts_without_a_second_clock() {
        let a = timed(1, 0, 100);
        let b = timed(2, 0, 50);
        let compiled =
            MotionGraph::sequence(vec![MotionGraph::track(a), MotionGraph::track(b)]).compile();
        assert_eq!(compiled[0].timing.start, Duration::from_millis(0));
        assert_eq!(compiled[1].timing.start, Duration::from_millis(100));
        assert_eq!(compiled[0].id, id(1));
        assert_eq!(compiled[1].id, id(2));
    }

    #[test]
    fn parallel_shares_origin() {
        let a = timed(1, 40, 100);
        let b = timed(2, 40, 50);
        let compiled =
            MotionGraph::parallel(vec![MotionGraph::track(a), MotionGraph::track(b)]).compile();
        assert_eq!(compiled[0].timing.start, Duration::from_millis(40));
        assert_eq!(compiled[1].timing.start, Duration::from_millis(40));
    }

    #[test]
    fn infinite_timed_track_has_no_end() {
        let mut track = timed(1, 0, 100);
        track.playback.iteration_count = AnimationIteration::INFINITE;
        assert_eq!(track_end(&track), None);
    }

    #[test]
    fn execution_class_follows_property_not_a_stored_override() {
        let mut track = timed(1, 0, 100);
        assert_eq!(track.execution_class(), AnimationClass::Compositor);
        track.property = AnimatableProperty::Width;
        assert_eq!(track.execution_class(), AnimationClass::Layout);
        track.property = AnimatableProperty::Display;
        assert_eq!(track.execution_class(), AnimationClass::Discrete);
    }

    #[test]
    fn is_valid_rejects_discrete_value_mismatch_and_unsorted_keyframes() {
        let mut display = timed(1, 0, 100);
        display.property = AnimatableProperty::Display;
        display.from = MotionValue::Scalar(0.0);
        display.to = MotionTo::Value(MotionValue::Scalar(1.0));
        assert!(!display.is_valid());
        display.from = MotionValue::Discrete(0);
        display.to = MotionTo::Value(MotionValue::Discrete(1));
        display.velocity = MotionValue::Discrete(0);
        assert!(display.is_valid());

        let mut keys = timed(2, 0, 100);
        keys.to = MotionTo::Keyframes(vec![
            Keyframe {
                offset: 0.8,
                value: MotionValue::Scalar(1.0),
                easing: None,
            },
            Keyframe {
                offset: 0.2,
                value: MotionValue::Scalar(0.0),
                easing: None,
            },
        ]);
        assert!(!keys.is_valid());
    }

    /// A target outside what the property accepts is refused, not clamped;
    /// a start past it (an overshoot being retargeted) is fine.
    #[test]
    fn targets_must_be_values_the_property_accepts() {
        let mut track = timed(1, 0, 100);
        track.to = MotionTo::Value(MotionValue::Scalar(3.0));
        assert!(!track.is_valid(), "opacity 3");
        track.to = MotionTo::Value(MotionValue::Scalar(f32::NAN));
        assert!(!track.is_valid(), "NaN target");
        track.to = MotionTo::Value(MotionValue::Scalar(0.5));
        track.from = MotionValue::Scalar(1.05);
        assert!(track.is_valid(), "an overshoot may be where a run starts");
        track.from = MotionValue::Scalar(f32::INFINITY);
        assert!(!track.is_valid());

        let mut axis = timed(2, 0, 100);
        axis.property = AnimatableProperty::FontAxis(*b"BEVL");
        axis.from = MotionValue::Scalar(f32::NAN);
        axis.to = MotionTo::Keyframes(vec![
            Keyframe {
                offset: 0.0,
                value: MotionValue::Scalar(f32::NAN),
                easing: None,
            },
            Keyframe {
                offset: 1.0,
                value: MotionValue::Scalar(500.0),
                easing: None,
            },
        ]);
        assert!(axis.is_valid(), "an absent axis in a keyframe list");
        axis.to = MotionTo::Value(MotionValue::Scalar(f32::NAN));
        assert!(!axis.is_valid(), "but not as a target of its own");
    }

    #[test]
    fn spring_to_builds_closed_form_params() {
        let spring = Spring::to(1.0).stiffness(200.0).damping(20.0).mass(1.0);
        assert_eq!(spring.rest_value(), MotionValue::Scalar(1.0));
        assert_eq!(spring.params().stiffness, 200.0);
        assert_eq!(spring.params().damping, 20.0);
        assert!(spring.implied_property().is_none());
        assert_eq!(
            Spring::to(PaintTransform::default()).implied_property(),
            Some(AnimatableProperty::Transform)
        );
    }

    #[test]
    fn timeline_parallel_shares_origin_sequence_advances() {
        let parallel = Timeline::parallel([
            MotionGraph::track(timed(1, 0, 100)),
            MotionGraph::track(timed(2, 0, 40)),
        ]);
        let compiled = parallel.compile();
        assert_eq!(compiled.len(), 2);
        assert_eq!(compiled[0].timing.start, compiled[1].timing.start);

        let sequence = Timeline::sequence([
            MotionGraph::track(timed(1, 0, 100)),
            MotionGraph::track(timed(2, 0, 40)),
        ]);
        let compiled = sequence.compile();
        assert_eq!(compiled[0].timing.start, Duration::ZERO);
        assert_eq!(compiled[1].timing.start, Duration::from_millis(100));
        assert!(matches!(compiled[0].to, MotionTo::Value(_)));
    }

    /// `steps(n, jump-start)` jumps on the way in: input 0 is already 1/n.
    /// Returning 0 there gives the curve n + 1 notches.
    #[test]
    fn steps_jump_start_takes_its_first_step_immediately() {
        assert_eq!(sample_steps(0.0, 4, StepJump::Start), 0.25);
        assert_eq!(sample_steps(0.1, 4, StepJump::Start), 0.25);
        assert_eq!(sample_steps(0.25, 4, StepJump::Start), 0.5);
        assert_eq!(sample_steps(0.99, 4, StepJump::Start), 1.0);
        assert_eq!(sample_steps(1.0, 4, StepJump::Start), 1.0);

        // The other three jumps are unchanged, including `both`, which already
        // stepped at 0.
        assert_eq!(sample_steps(0.0, 4, StepJump::End), 0.0);
        assert_eq!(sample_steps(0.0, 4, StepJump::Both), 0.2);
        assert_eq!(sample_steps(0.0, 4, StepJump::None), 0.0);
    }
}
