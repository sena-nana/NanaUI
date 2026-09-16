//! Runtime animation sampling and presentation overlay lifecycle.

use super::*;
use crate::{AnimationEvent, AnimationEventKind, MotionInterrupt, retarget_track};
use nana_ui_core::motion::MotionTargetId;
use std::collections::HashSet;

impl UiWorld {
    /// Sample only due timelines. Built-in paint transitions publish their
    /// values and targeted invalidation here; other consumers receive samples.
    /// Compositor overlays are not written into UiWorld logical style.
    pub fn advance_animations(&mut self, now: Duration) -> AnimationFrame {
        self.animation_now = now;
        self.begin_motion_frame();
        let mut component_updates = Vec::new();
        let mut events = std::mem::take(&mut self.pending_animation_events);
        let mut animation_deadlines_scanned = 0usize;
        let due = self
            .animation_deadlines
            .range(..=(now, AnimationId::new(u64::MAX).expect("max ID is nonzero")))
            .inspect(|_| {
                animation_deadlines_scanned = animation_deadlines_scanned.saturating_add(1)
            })
            .copied()
            .collect::<Vec<_>>();
        let mut samples = Vec::with_capacity(due.len());
        let mut animations_considered = 0usize;
        for (deadline, id) in due {
            self.animation_deadlines.remove(&(deadline, id));
            animations_considered = animations_considered.saturating_add(1);
            let (sample, next_deadline, fill_forwards) = {
                let animation = self
                    .animations
                    .get_mut(&id)
                    .expect("due animation must remain active");
                let fill_forwards = animation.spec.playback.fill_mode.applies_forwards();
                let sample = animation
                    .sample(now)
                    .expect("due animation must produce a sample");
                let next_deadline = (!sample.finished && animation.has_follow_up_deadline())
                    .then_some(animation.next_deadline);
                (sample, next_deadline, fill_forwards)
            };
            if sample.finished {
                self.animations.remove(&id);
                if !fill_forwards {
                    self.unbind_presentation(id.track_id());
                }
                events.push(AnimationEvent {
                    id: sample.id,
                    target: sample.target,
                    kind: AnimationEventKind::Finished,
                });
            } else if let Some(next_deadline) = next_deadline {
                self.index_animation_deadline(next_deadline, id);
            }
            if crate::component_animation_id(
                crate::component_animation_kinds::SWITCH,
                sample.target,
            ) == Some(sample.id)
                && let Some(mut visual @ StandardVisual::Switch { .. }) =
                    self.standard_visual(sample.target)
            {
                if let StandardVisual::Switch { thumb_progress, .. } = &mut visual {
                    *thumb_progress = match sample.value {
                        crate::MotionValue::Scalar(value) => value.clamp(0.0, 1.0),
                        _ => sample.progress,
                    };
                }
                self.nodes.set_visual(sample.target, Some(visual));
                self.mark(sample.target, DirtyMask::RENDER);
                self.account_animation_dirty(DirtyMask::RENDER);
                component_updates.push(sample.target);
            }
            if crate::component_animation_id(crate::component_animation_kinds::HOVER, sample.target)
                == Some(sample.id)
            {
                if let Some(transition) = self.hover_transitions.get_mut(&sample.target) {
                    transition.progress = sample.progress;
                }
                self.mark_hover_paint(sample.target);
                self.account_animation_dirty(DirtyMask::STYLE | DirtyMask::RENDER);
                if sample.finished {
                    self.hover_transitions.remove(&sample.target);
                }
                component_updates.push(sample.target);
            }
            if crate::component_animation_id(
                crate::component_animation_kinds::SPINNER,
                sample.target,
            ) == Some(sample.id)
                && let Some(mut visual @ StandardVisual::Spinner { .. }) =
                    self.standard_visual(sample.target)
            {
                if let StandardVisual::Spinner { phase, .. } = &mut visual {
                    *phase = sample.progress;
                }
                self.nodes.set_visual(sample.target, Some(visual));
                self.mark(sample.target, DirtyMask::RENDER);
                self.account_animation_dirty(DirtyMask::RENDER);
                component_updates.push(sample.target);
            }
            if crate::component_animation_id(
                crate::component_animation_kinds::SURFACE,
                sample.target,
            ) == Some(sample.id)
                || crate::component_animation_id(
                    crate::component_animation_kinds::SURFACE_POP,
                    sample.target,
                ) == Some(sample.id)
            {
                self.advance_surface_motion(&sample);
                if sample.finished {
                    component_updates.push(sample.target);
                }
            }
            if crate::component_animation_id(
                crate::component_animation_kinds::LOADING,
                sample.target,
            ) == Some(sample.id)
            {
                self.advance_loading_phase(&sample);
                component_updates.push(sample.target);
            }
            if crate::component_animation_id(
                crate::component_animation_kinds::SIDEBAR,
                sample.target,
            ) == Some(sample.id)
            {
                self.account_animation_dirty(DirtyMask::LAYOUT);
                component_updates.push(sample.target);
            }
            if crate::component_animation_id(
                crate::component_animation_kinds::WORKSPACE,
                sample.target,
            ) == Some(sample.id)
            {
                self.account_animation_dirty(DirtyMask::LAYOUT);
                component_updates.push(sample.target);
            }
            if self.apply_layout_class_sample(&sample) {
                component_updates.push(sample.target);
            }
            samples.push(sample);
        }
        if !component_updates.is_empty() {
            self.generation = self.generation.wrapping_add(1);
        }
        AnimationFrame {
            samples,
            component_updates,
            events,
            next_deadline: self.next_animation_deadline(),
            animation_deadlines_scanned,
            animations_considered,
        }
    }
}

impl UiWorld {
    /// Whether `id` currently owns an active animation timeline. Component
    /// projections query this to avoid re-starting a running timeline.
    pub fn animation_is_active(&self, id: AnimationId) -> bool {
        self.animations.contains_key(&id)
    }

    pub(crate) fn animation_timing_start(&self, id: AnimationId) -> Option<Duration> {
        self.animations
            .get(&id)
            .map(|animation| animation.spec.timing.start)
    }

    /// Consume mutation-scoped lifecycle events from the last commit.
    /// Deadline completions still arrive on the `advance_animations` that
    /// hits their completion deadline.
    pub fn take_animation_events(&mut self) -> Vec<AnimationEvent> {
        std::mem::take(&mut self.pending_animation_events)
    }

    /// Close the previous commit's unobserved event batch. Called at the
    /// start of the next successful apply so leftover Cancelled from park
    /// cannot make a later idle advance look like work.
    pub(super) fn close_prior_animation_event_frame(&mut self) {
        self.pending_animation_events.clear();
    }
}

impl UiWorld {
    pub fn next_animation_deadline(&self) -> Option<Duration> {
        self.animation_deadlines
            .first()
            .map(|(deadline, _)| *deadline)
    }
}

impl UiWorld {
    pub(super) fn start_component_track(
        &mut self,
        target: StableNodeId,
        kind: u64,
        duration: Duration,
        easing: crate::Easing,
        property: crate::AnimatableProperty,
        from: crate::MotionValue,
        to: crate::MotionValue,
        interrupt: crate::MotionInterrupt,
        playback: Option<crate::AnimationPlayback>,
    ) {
        let Some(id) = crate::component_animation_id(kind, target) else {
            return;
        };
        let mut spec = AnimationSpec::new(
            id,
            target,
            self.animation_now,
            duration,
            crate::framework::COMPONENT_FRAME_INTERVAL,
            easing,
        )
        .with_property(property)
        .with_range(from, crate::MotionTo::Value(to))
        .with_interrupt(interrupt);
        if let Some(playback) = playback {
            spec = spec.with_playback(playback);
        }
        self.install_animation(spec);
    }

    pub(super) fn install_animation(&mut self, spec: AnimationSpec) {
        let spec = self.apply_retarget(spec);
        let id = spec.id;
        let active = crate::animation::ActiveAnimation::new(spec.clone());
        let deadline = active.next_deadline;
        if let Some(previous) = self.animations.insert(id, active) {
            self.animation_deadlines
                .remove(&(previous.next_deadline, id));
        }
        self.index_animation_deadline(deadline, id);
        self.install_presentation_overlay(&spec);
    }

    /// `Duration::MAX` means no CPU wake (a paused hold, or a compositor
    /// overlay without completion, which `compositor_needs_tick` presents), so
    /// it stays out of the index instead of reporting a deadline ~584 billion
    /// years away.
    fn index_animation_deadline(&mut self, deadline: Duration, id: AnimationId) {
        if deadline != Duration::MAX {
            self.animation_deadlines.insert((deadline, id));
        }
    }

    pub(super) fn cancel_animation(&mut self, id: AnimationId) -> bool {
        let Some(animation) = self.animations.remove(&id) else {
            self.unbind_presentation(id.track_id());
            return false;
        };
        self.animation_deadlines
            .remove(&(animation.next_deadline, id));
        self.unbind_presentation(id.track_id());
        self.pending_animation_events.push(AnimationEvent {
            id,
            target: animation.spec.target,
            kind: AnimationEventKind::Cancelled,
        });
        true
    }

    pub(super) fn finish_animation(&mut self, id: AnimationId) -> bool {
        let Some(animation) = self.animations.remove(&id) else {
            return false;
        };
        self.animation_deadlines
            .remove(&(animation.next_deadline, id));
        let now = self.animation_now;
        let mut spec = animation.spec;
        snap_spec_to_completion(&mut spec, now);
        let fill_forwards = spec.playback.fill_mode.applies_forwards();
        if fill_forwards && spec.uses_presentation_overlay() {
            self.install_presentation_overlay(&spec);
        } else {
            self.unbind_presentation(id.track_id());
        }
        self.pending_animation_events.push(AnimationEvent {
            id,
            target: spec.target,
            kind: AnimationEventKind::Finished,
        });
        true
    }

    pub(super) fn reverse_animation(&mut self, id: AnimationId) -> bool {
        let Some(animation) = self.animations.get(&id) else {
            return false;
        };
        let Some(track) = animation.spec.to_motion_track() else {
            return false;
        };
        let now = self.animation_now.max(animation.spec.timing.start);
        let destination = track.from;
        let next = retarget_track(&track, now, destination);
        let mut spec = animation.spec.clone();
        apply_track_to_spec(&mut spec, &next);
        spec.interrupt = MotionInterrupt::Retarget;
        self.install_animation(spec);
        true
    }

    pub(super) fn pause_animation(&mut self, id: AnimationId) -> bool {
        let now = self.animation_now;
        let Some(deadline) = self
            .animations
            .get(&id)
            .map(|animation| animation.next_deadline)
        else {
            return false;
        };
        self.animation_deadlines.remove(&(deadline, id));
        let Some(animation) = self.animations.get_mut(&id) else {
            return false;
        };
        animation.spec.playback.play_state = crate::AnimationPlayState::Paused;
        animation.spec.playback.paused_at = Some(now);
        animation.next_deadline = Duration::MAX;
        if let Some(overlay) = self.presentation.get_mut(id.track_id()) {
            overlay.track.pause_at(now);
            let _ = self.motion_descriptors.bind(&overlay.track);
        }
        true
    }

    pub(super) fn resume_animation(&mut self, id: AnimationId) -> bool {
        let Some(animation) = self.animations.get(&id) else {
            return false;
        };
        let previous = animation.next_deadline;
        let compositor = animation.spec.uses_completion_deadline_only();
        let interval = animation.spec.timing.frame_interval;
        let end = animation.spec.end();
        let completion = animation
            .spec
            .to_motion_track()
            .and_then(|track| nana_ui_core::motion::track_completion_deadline(&track));
        let now = self.animation_now;
        let next = if compositor {
            completion
                .filter(|deadline| *deadline > now)
                .unwrap_or(Duration::MAX)
        } else {
            let step = now.checked_add(interval).unwrap_or(now);
            match end {
                Some(end) => step.min(end),
                None => step,
            }
        };
        self.animation_deadlines.remove(&(previous, id));
        let Some(animation) = self.animations.get_mut(&id) else {
            return false;
        };
        animation.spec.playback.play_state = crate::AnimationPlayState::Running;
        animation.spec.playback.paused_at = None;
        animation.next_deadline = next;
        if let Some(overlay) = self.presentation.get_mut(id.track_id()) {
            overlay.track.resume();
            let _ = self.motion_descriptors.bind(&overlay.track);
        }
        self.index_animation_deadline(next, id);
        true
    }

    fn apply_retarget(&self, mut spec: AnimationSpec) -> AnimationSpec {
        if spec.interrupt != MotionInterrupt::Retarget {
            return spec;
        }
        let now = spec.timing.start.max(self.animation_now);
        let Some(rest) = spec.to_motion_track().map(|track| track.rest_value()) else {
            return spec;
        };
        if let Some(previous) = self
            .animations
            .get(&spec.id)
            .and_then(|active| active.spec.to_motion_track())
        {
            let next = retarget_track(&previous, now, rest);
            apply_track_to_spec(&mut spec, &next);
            return spec;
        }
        if let Some(target) = MotionTargetId::new(spec.target.get())
            && let Some(sample) = self.presentation.sample(target, spec.property, now)
            && let Some(value) = sample.applied_value()
        {
            spec.from = value;
            spec.velocity = sample.velocity;
            spec.timing.start = now;
            spec.timing.delay = Duration::ZERO;
            return spec;
        }
        if spec.from == rest
            && let Some(logical) = self.logical_motion_value(spec.target, spec.property)
        {
            spec.from = logical;
            spec.velocity = logical.zero_velocity();
            spec.timing.start = now;
            spec.timing.delay = Duration::ZERO;
        }
        spec
    }

    fn install_presentation_overlay(&mut self, spec: &AnimationSpec) {
        if !spec.uses_presentation_overlay() {
            self.unbind_presentation(spec.id.track_id());
            return;
        }
        let Some(track) = spec.to_motion_track() else {
            return;
        };
        let logical = self
            .logical_motion_value(spec.target, spec.property)
            .unwrap_or(track.rest_value());
        let _ = self.motion_descriptors.bind(&track);
        self.presentation.insert(track, logical);
    }

    fn unbind_presentation(&mut self, id: nana_ui_core::motion::MotionTrackId) {
        self.presentation.remove(id);
        self.motion_descriptors.cancel(id);
    }
}

fn apply_track_to_spec(spec: &mut AnimationSpec, track: &nana_ui_core::motion::MotionTrack) {
    spec.from = track.from;
    spec.to = track.to.clone();
    spec.velocity = track.velocity;
    spec.timing = track.timing;
    spec.playback = track.playback;
    spec.curve = track.curve;
    spec.property = track.property;
}

fn snap_spec_to_completion(spec: &mut AnimationSpec, _now: Duration) {
    let rest = spec
        .to_motion_track()
        .map(|track| track.rest_value())
        .unwrap_or(spec.from);
    spec.from = rest;
    spec.to = crate::MotionTo::Value(rest);
    spec.timing.delay = Duration::ZERO;
    spec.timing.start = Duration::ZERO;
    spec.timing.duration = Duration::from_nanos(1);
}

impl UiWorld {
    pub(super) fn cancel_animations_for_removed(&mut self, id: StableNodeId) {
        let cancelled = self
            .animations
            .iter()
            .filter_map(|(&animation_id, animation)| {
                (animation.spec.target == id).then_some((animation_id, animation.next_deadline))
            })
            .collect::<Vec<_>>();
        for (animation_id, deadline) in cancelled {
            self.animations.remove(&animation_id);
            self.animation_deadlines.remove(&(deadline, animation_id));
            self.pending_animation_events.push(AnimationEvent {
                id: animation_id,
                target: id,
                kind: AnimationEventKind::Cancelled,
            });
        }
        if let Some(target) = MotionTargetId::new(id.get()) {
            self.presentation.remove_target(target);
            self.motion_descriptors.cancel_target(target);
        }
    }

    pub(super) fn drop_non_overlay_animations_for_parked(
        &mut self,
        parked: &HashSet<StableNodeId>,
    ) {
        let cancelled = self
            .animations
            .iter()
            .filter_map(|(&animation_id, animation)| {
                let target = animation.spec.target;
                if !parked.contains(&target) {
                    return None;
                }
                let infinite_component = crate::component_animation_id(
                    crate::component_animation_kinds::SKELETON,
                    target,
                ) == Some(animation_id)
                    || crate::component_animation_id(
                        crate::component_animation_kinds::SPINNER,
                        target,
                    ) == Some(animation_id)
                    || crate::component_animation_id(
                        crate::component_animation_kinds::LOADING,
                        target,
                    ) == Some(animation_id);
                (!animation.spec.uses_presentation_overlay() || infinite_component).then_some((
                    animation_id,
                    animation.next_deadline,
                    target,
                ))
            })
            .collect::<Vec<_>>();
        for (animation_id, deadline, target) in cancelled {
            self.animations.remove(&animation_id);
            self.animation_deadlines.remove(&(deadline, animation_id));
            self.unbind_presentation(animation_id.track_id());
            self.pending_animation_events.push(AnimationEvent {
                id: animation_id,
                target,
                kind: AnimationEventKind::Cancelled,
            });
        }
    }
}
