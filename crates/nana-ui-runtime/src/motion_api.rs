//! Rust L3 declarative motion frontend. Compiles to [`AnimationSpec`] /
//! [`MotionTrack`]; not a second timeline.

use std::{hash::Hasher, time::Duration};

use nana_ui_core::{PaintTransform, Spring};

use crate::{
    AnimatableProperty, AnimationDirection, AnimationFillMode, AnimationId, AnimationIteration,
    AnimationPlayState, AnimationPlayback, AnimationSpec, Easing, MotionCurve, MotionGraph,
    MotionInterrupt, MotionTiming, MotionTo, MotionTrack, MotionValue, MutationQueue, StableNodeId,
};

/// User-authored tracks share one hashed namespace so they never collide with
/// built-in component kind tags.
const USER_MOTION: u64 = 64;

/// One node's L3 motion handle. `transition` / `motion` compile to the same
/// [`AnimationSpec`] path CSS and built-ins already use.
pub struct NodeMotion<'a> {
    queue: &'a mut MutationQueue,
    id: StableNodeId,
    now: Duration,
}

impl<'a> NodeMotion<'a> {
    pub fn transition(self) -> TransitionBuilder<'a> {
        TransitionBuilder {
            queue: self.queue,
            target: self.id,
            now: self.now,
            opacity: None,
            transform: None,
            width: None,
            height: None,
            duration: nana_ui_core::motion::HOVER_COLOR,
            easing: Easing::EaseOutCubic,
            delay: Duration::ZERO,
            started: false,
        }
    }

    pub fn motion(self, spring: Spring) -> SpringBuilder<'a> {
        let property = spring.implied_property();
        SpringBuilder {
            queue: self.queue,
            target: self.id,
            now: self.now,
            spring,
            property,
            started: false,
        }
    }

    pub fn timeline(self, graph: MotionGraph) -> TimelineBuilder<'a> {
        TimelineBuilder {
            queue: self.queue,
            target: self.id,
            now: self.now,
            graph,
            started: false,
        }
    }

    /// Explicit FLIP: layout is already Last. Presentation translate plays to identity.
    pub fn flip(
        self,
        first: nana_ui_core::FlipRect,
        last: nana_ui_core::FlipRect,
    ) -> FlipBuilder<'a> {
        FlipBuilder {
            queue: self.queue,
            target: self.id,
            now: self.now,
            first,
            last,
            duration: nana_ui_core::motion::OVERLAY_FADE,
            easing: Easing::EaseOutCubic,
            animate_size: false,
            started: false,
        }
    }
}

/// `node.transition().opacity(1.0).transform(t).duration(d).ease(e)`.
pub struct TransitionBuilder<'a> {
    queue: &'a mut MutationQueue,
    target: StableNodeId,
    now: Duration,
    opacity: Option<f32>,
    transform: Option<PaintTransform>,
    width: Option<f32>,
    height: Option<f32>,
    duration: Duration,
    easing: Easing,
    delay: Duration,
    started: bool,
}

impl TransitionBuilder<'_> {
    pub fn opacity(mut self, value: f32) -> Self {
        self.opacity = Some(value);
        self
    }

    pub fn transform(mut self, value: PaintTransform) -> Self {
        self.transform = Some(value);
        self
    }

    pub fn width(mut self, value: f32) -> Self {
        self.width = Some(value);
        self
    }

    pub fn height(mut self, value: f32) -> Self {
        self.height = Some(value);
        self
    }

    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    pub fn ease(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn start(mut self) {
        self.commit();
    }

    fn commit(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        let mut timing = MotionTiming::new(
            self.now,
            self.duration,
            crate::framework::COMPONENT_FRAME_INTERVAL,
        );
        timing.delay = self.delay;
        let curve = MotionCurve::Easing(self.easing);
        let playback = AnimationPlayback {
            iteration_count: AnimationIteration::ONCE,
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::Forwards,
            play_state: AnimationPlayState::Running,
            paused_at: None,
        };
        if let Some(opacity) = self.opacity {
            self.queue.start_animation(user_spec(
                self.target,
                AnimatableProperty::Opacity,
                MotionValue::Scalar(opacity),
                timing,
                curve,
                playback,
            ));
        }
        if let Some(transform) = self.transform {
            self.queue.start_animation(user_spec(
                self.target,
                AnimatableProperty::Transform,
                MotionValue::Transform(transform),
                timing,
                curve,
                playback,
            ));
        }
        if let Some(width) = self.width {
            self.queue.start_animation(user_spec(
                self.target,
                AnimatableProperty::Width,
                MotionValue::Scalar(width),
                timing,
                curve,
                playback,
            ));
        }
        if let Some(height) = self.height {
            self.queue.start_animation(user_spec(
                self.target,
                AnimatableProperty::Height,
                MotionValue::Scalar(height),
                timing,
                curve,
                playback,
            ));
        }
    }
}

impl Drop for TransitionBuilder<'_> {
    fn drop(&mut self) {
        self.commit();
    }
}

/// `node.motion(Spring::to(1.0).stiffness(..).damping(..)).opacity()`.
pub struct SpringBuilder<'a> {
    queue: &'a mut MutationQueue,
    target: StableNodeId,
    now: Duration,
    spring: Spring,
    property: Option<AnimatableProperty>,
    started: bool,
}

impl SpringBuilder<'_> {
    pub fn opacity(mut self) -> Self {
        self.property = Some(AnimatableProperty::Opacity);
        self
    }

    pub fn transform(mut self) -> Self {
        self.property = Some(AnimatableProperty::Transform);
        self
    }

    pub fn start(mut self) {
        self.commit();
    }

    fn commit(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        let Some(property) = self.property else {
            return;
        };
        let rest = self.spring.rest_value();
        self.queue.start_animation(user_spec(
            self.target,
            property,
            rest,
            MotionTiming::new(
                self.now,
                Duration::from_millis(1),
                crate::framework::COMPONENT_FRAME_INTERVAL,
            ),
            MotionCurve::Spring(self.spring.params()),
            AnimationPlayback {
                iteration_count: AnimationIteration::ONCE,
                direction: AnimationDirection::Normal,
                fill_mode: AnimationFillMode::Forwards,
                play_state: AnimationPlayState::Running,
                paused_at: None,
            },
        ));
    }
}

impl Drop for SpringBuilder<'_> {
    fn drop(&mut self) {
        self.commit();
    }
}

/// `Timeline::parallel` / `sequence` compiled through the same spec path.
pub struct TimelineBuilder<'a> {
    queue: &'a mut MutationQueue,
    target: StableNodeId,
    now: Duration,
    graph: MotionGraph,
    started: bool,
}

impl TimelineBuilder<'_> {
    pub fn start(mut self) {
        self.commit();
    }

    fn commit(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        let graph = std::mem::replace(&mut self.graph, MotionGraph::parallel(Vec::new()));
        for mut track in graph.compile() {
            track.timing.start = track.timing.start.saturating_add(self.now);
            if let Some(target) = nana_ui_core::motion::MotionTargetId::new(self.target.get()) {
                track.target = target;
            }
            // Keyed on the track as well as the property: a sequence that
            // fades opacity in and then out compiles to two tracks on one
            // property, and one id for both would let the second overwrite the
            // first in the same mutation batch — only the last stage would
            // ever play. The caller's own `MotionTrackId`s are stable across
            // rebuilds of the same timeline, so re-running one still replaces
            // it rather than stacking a second copy.
            let Some(id) = user_timeline_animation_id(self.target, track.property, track.id) else {
                continue;
            };
            let Some(target) = StableNodeId::new(track.target.get()) else {
                continue;
            };
            self.queue.start_animation(
                AnimationSpec::from_track(id, target, &track)
                    .with_interrupt(MotionInterrupt::Retarget),
            );
        }
    }
}

impl Drop for TimelineBuilder<'_> {
    fn drop(&mut self) {
        self.commit();
    }
}

/// `node.flip(first, last).duration(d).ease(e)`. Layout stays Last.
pub struct FlipBuilder<'a> {
    queue: &'a mut MutationQueue,
    target: StableNodeId,
    now: Duration,
    first: nana_ui_core::FlipRect,
    last: nana_ui_core::FlipRect,
    duration: Duration,
    easing: Easing,
    animate_size: bool,
    started: bool,
}

impl FlipBuilder<'_> {
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    pub fn ease(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    /// Shared-element size: real Layout-class width/height, never a scale.
    pub fn animate_size(mut self) -> Self {
        self.animate_size = true;
        self
    }

    pub fn start(mut self) {
        self.commit();
    }

    fn commit(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.queue.start_layout_flip(
            self.target,
            self.first,
            self.last,
            self.now,
            self.duration,
            self.easing,
        );
        if self.animate_size && self.first.size_differs(self.last) {
            let mut timing = MotionTiming::new(
                self.now,
                self.duration,
                crate::framework::COMPONENT_FRAME_INTERVAL,
            );
            timing.delay = Duration::ZERO;
            let curve = MotionCurve::Easing(self.easing);
            let playback = AnimationPlayback {
                iteration_count: AnimationIteration::ONCE,
                direction: AnimationDirection::Normal,
                fill_mode: AnimationFillMode::Forwards,
                play_state: AnimationPlayState::Running,
                paused_at: None,
            };
            if (self.first.width - self.last.width).abs() > 1e-4 {
                let mut spec = user_spec(
                    self.target,
                    AnimatableProperty::Width,
                    MotionValue::Scalar(self.last.width),
                    timing,
                    curve,
                    playback,
                );
                spec.from = MotionValue::Scalar(self.first.width);
                spec.interrupt = MotionInterrupt::Replace;
                self.queue.start_animation(spec);
            }
            if (self.first.height - self.last.height).abs() > 1e-4 {
                let mut spec = user_spec(
                    self.target,
                    AnimatableProperty::Height,
                    MotionValue::Scalar(self.last.height),
                    timing,
                    curve,
                    playback,
                );
                spec.from = MotionValue::Scalar(self.first.height);
                spec.interrupt = MotionInterrupt::Replace;
                self.queue.start_animation(spec);
            }
        }
    }
}

impl Drop for FlipBuilder<'_> {
    fn drop(&mut self) {
        self.commit();
    }
}

impl MutationQueue {
    /// L3 node handle. `now` is the host monotonic clock already used by
    /// [`AnimationSpec`] timing.
    pub fn node(&mut self, id: StableNodeId, now: Duration) -> NodeMotion<'_> {
        NodeMotion {
            queue: self,
            id,
            now,
        }
    }
}

impl AnimationSpec {
    pub fn from_track(id: AnimationId, target: StableNodeId, track: &MotionTrack) -> Self {
        Self {
            id,
            target,
            timing: track.timing,
            playback: track.playback,
            curve: track.curve,
            property: track.property,
            from: track.from,
            to: track.to.clone(),
            velocity: track.velocity,
            interrupt: MotionInterrupt::Replace,
        }
    }
}

fn user_animation_id(target: StableNodeId, property: AnimatableProperty) -> Option<AnimationId> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write_u64(USER_MOTION);
    hasher.write_u64(target.get());
    hasher.write_u8(property_tag(property));
    AnimationId::new(hasher.finish())
}

/// One id per (node, property, track) for a compiled timeline.
fn user_timeline_animation_id(
    target: StableNodeId,
    property: AnimatableProperty,
    track: nana_ui_core::motion::MotionTrackId,
) -> Option<AnimationId> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write_u64(USER_MOTION);
    hasher.write_u64(target.get());
    hasher.write_u8(property_tag(property));
    hasher.write_u64(track.get());
    AnimationId::new(hasher.finish())
}

fn property_tag(property: AnimatableProperty) -> u8 {
    match property {
        AnimatableProperty::Transform => 1,
        AnimatableProperty::Opacity => 2,
        AnimatableProperty::Clip => 3,
        AnimatableProperty::Color => 4,
        AnimatableProperty::Background => 5,
        AnimatableProperty::Blur => 6,
        AnimatableProperty::Filter => 7,
        AnimatableProperty::Shadow => 8,
        AnimatableProperty::ShaderParameter => 9,
        AnimatableProperty::Width => 10,
        AnimatableProperty::Height => 11,
        AnimatableProperty::Padding => 12,
        AnimatableProperty::Margin => 13,
        AnimatableProperty::FontSize => 14,
        AnimatableProperty::FontAxis => 15,
        AnimatableProperty::Display => 16,
        AnimatableProperty::Progress => 17,
    }
}

fn user_spec(
    target: StableNodeId,
    property: AnimatableProperty,
    to: MotionValue,
    timing: MotionTiming,
    curve: MotionCurve,
    playback: AnimationPlayback,
) -> AnimationSpec {
    let id = user_animation_id(target, property)
        .unwrap_or_else(|| AnimationId::new(target.get()).expect("node ids are nonzero"));
    AnimationSpec {
        id,
        target,
        timing,
        playback,
        curve,
        property,
        from: to,
        to: MotionTo::Value(to),
        velocity: to.zero_velocity(),
        interrupt: MotionInterrupt::Retarget,
    }
}
