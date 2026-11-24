//! Runtime animation sampling and presentation overlay lifecycle.

use super::*;
use crate::{
    AnimationDirection, AnimationEvent, AnimationEventKind, AnimationFillMode, MotionInterrupt,
    retarget_track,
};
use nana_ui_core::motion::MotionTargetId;
use std::collections::HashSet;

impl UiWorld {
    /// Sample only due timelines. Built-in paint transitions publish their
    /// values and targeted invalidation here; other consumers receive samples.
    /// Compositor overlays are not written into UiWorld logical style.
    ///
    /// `now` is host time: the document's presentation clock maps it onto
    /// `animation_now` (the identity unless the document was paused, seeked
    /// or re-timed), and every sample is taken at that document time.
    pub fn advance_animations(&mut self, now: Duration) -> AnimationFrame {
        let now = self.motion.presentation_clock.observe(now);
        self.motion.animation_now = now;
        self.begin_motion_frame();
        let mut component_updates = Vec::new();
        let mut events = std::mem::take(&mut self.motion.pending_animation_events);
        let mut animation_deadlines_scanned = 0usize;
        let due = self
            .motion
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
            self.motion.animation_deadlines.remove(&(deadline, id));
            animations_considered = animations_considered.saturating_add(1);
            let (sample, next_deadline, fill_forwards) = {
                let animation = self
                    .motion
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
            self.settle_animation_sample(
                id,
                sample,
                next_deadline,
                fill_forwards,
                &mut events,
                &mut component_updates,
                &mut samples,
            );
        }
        if !component_updates.is_empty() {
            self.invalidation.generation = self.invalidation.generation.wrapping_add(1);
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
    /// Set the document's presentation time to `to` (Issue #300). One value
    /// serves both evaluation paths: `animation_now`, which the compositor and
    /// [`Self::sync_presentation_clock`] read, and every running track, which
    /// is sampled at `to` whatever its next deadline was. Layout-class tracks
    /// write their absolute value at `to`, so a seek can go back. Compositor
    /// tracks are not written into the world; they read the new time.
    ///
    /// A paused document holds `to`; a playing one continues from it at the
    /// next host time. Runs the seek carries past their end finish as they
    /// would on a frame: their `Finished` waits for the next advance, so it
    /// is delivered once, through the same path. A finished run is not brought
    /// back by seeking before its end.
    pub fn seek_presentation(&mut self, to: Duration) -> AnimationFrame {
        self.motion.presentation_clock.seek(to);
        self.resample_all_at(to)
    }

    /// Advance the document's time by `interval` and sample there, as
    /// [`Self::seek_presentation`] does; `interval` becomes the frame
    /// interval until the document plays again.
    pub fn step_presentation(&mut self, interval: Duration) -> AnimationFrame {
        self.motion.presentation_clock.note_step(interval);
        let to = self.motion.animation_now.saturating_add(interval);
        self.seek_presentation(to)
    }

    /// Set the time to `to` as one frame of `interval`, as a caller drawing
    /// the document at its own step does (Issue #304): samples as
    /// [`Self::seek_presentation`] does, and `interval` is the frame interval
    /// until the document plays again, whether `to` lies ahead or behind.
    pub fn seek_presentation_frame(&mut self, to: Duration, interval: Duration) -> AnimationFrame {
        self.motion.presentation_clock.note_step(interval);
        self.seek_presentation(to)
    }

    fn resample_all_at(&mut self, to: Duration) -> AnimationFrame {
        self.motion.animation_now = to;
        self.begin_motion_frame();
        let mut ids = self.motion.animations.keys().copied().collect::<Vec<_>>();
        ids.sort_unstable();
        let mut samples = Vec::with_capacity(ids.len());
        let mut component_updates = Vec::new();
        let mut events = Vec::new();
        let mut animations_considered = 0usize;
        for id in ids {
            let Some(animation) = self.motion.animations.get_mut(&id) else {
                continue;
            };
            let previous = animation.next_deadline;
            let fill_forwards = animation.spec.playback.fill_mode.applies_forwards();
            let Some(sample) = animation.resample(to) else {
                continue;
            };
            let next_deadline = (!sample.finished && animation.has_follow_up_deadline())
                .then_some(animation.next_deadline);
            self.motion.animation_deadlines.remove(&(previous, id));
            animations_considered = animations_considered.saturating_add(1);
            self.settle_animation_sample(
                id,
                sample,
                next_deadline,
                fill_forwards,
                &mut events,
                &mut component_updates,
                &mut samples,
            );
        }
        self.motion.pending_animation_events.extend(events);
        if !component_updates.is_empty() {
            self.invalidation.generation = self.invalidation.generation.wrapping_add(1);
        }
        AnimationFrame {
            samples,
            component_updates,
            events: Vec::new(),
            next_deadline: self.next_animation_deadline(),
            animation_deadlines_scanned: 0,
            animations_considered,
        }
    }

    /// The running tracks on `node`, by id: what a panel lists and edits
    /// before [`crate::MutationQueue::replace_animation`] writes one back.
    pub fn animation_tracks(&self, node: StableNodeId) -> Vec<AnimationSpec> {
        let mut tracks = self
            .motion
            .animations
            .values()
            .filter(|animation| animation.spec.target == node)
            .map(|animation| animation.spec.clone())
            .collect::<Vec<_>>();
        tracks.sort_unstable_by_key(|spec| spec.id);
        tracks
    }

    /// Replace a running track whole, under the same id (Issue #325). The
    /// new spec is taken as written: no retarget from the old run's value and
    /// no new run, so no `Cancelled` and no second id. It is sampled at the
    /// document's current time on the next advance, so a paused document
    /// shows the edited curve at the frame it holds rather than replaying
    /// from 0.
    pub(super) fn replace_animation(&mut self, spec: AnimationSpec) -> bool {
        let id = spec.id;
        let Some(previous) = self.motion.animations.get(&id) else {
            return false;
        };
        let previous_target = previous.spec.target;
        let previous_property = previous.spec.property;
        let previous_deadline = previous.next_deadline;
        self.motion
            .animation_deadlines
            .remove(&(previous_deadline, id));
        // Due at its (effective) start: one already reached is sampled by
        // the next advance, at the document's time then.
        let active = crate::animation::ActiveAnimation::new(spec.clone());
        let deadline = active.next_deadline;
        self.motion.animations.insert(id, active);
        self.index_animation_deadline(deadline, id);
        self.install_presentation_overlay(&spec);
        if is_layout_length(previous_property)
            && (previous_target != spec.target || previous_property != spec.property)
        {
            self.clear_settled_length_slot_if_unused(previous_target, previous_property);
        }
        true
    }
}

impl UiWorld {
    /// Apply one sample of `id`'s run: retire it when it finished, re-index
    /// its next deadline otherwise, and publish what the built-in consumers
    /// read from it. Shared by deadline-driven advances and by a seek, which
    /// samples every run at the new time.
    #[allow(clippy::too_many_arguments)]
    fn settle_animation_sample(
        &mut self,
        id: AnimationId,
        sample: crate::AnimationSample,
        next_deadline: Option<Duration>,
        fill_forwards: bool,
        events: &mut Vec<AnimationEvent>,
        component_updates: &mut Vec<StableNodeId>,
        samples: &mut Vec<crate::AnimationSample>,
    ) {
        if sample.finished {
            self.motion.animations.remove(&id);
            if !fill_forwards && !is_layout_length(sample.property) {
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
        if let Some(kind) = crate::animation::component_animation_kind(sample.id, sample.target) {
            self.settle_component_sample(kind, &sample, component_updates);
        }
        if self.apply_layout_class_sample(&sample) {
            component_updates.push(sample.target);
        }
        if matches!(
            sample.property,
            crate::AnimatableProperty::Background | crate::AnimatableProperty::Color
        ) && self.contains(sample.target)
        {
            let dirty = DirtyMask::STYLE | DirtyMask::RENDER;
            if sample.property == crate::AnimatableProperty::Color {
                self.mark_subtree(sample.target, dirty);
            } else {
                self.mark(sample.target, dirty);
            }
            self.account_animation_dirty(dirty);
            component_updates.push(sample.target);
        }
        samples.push(sample);
    }

    fn settle_component_sample(
        &mut self,
        kind: u64,
        sample: &crate::AnimationSample,
        component_updates: &mut Vec<StableNodeId>,
    ) {
        let scalar_01 = || match sample.value {
            crate::MotionValue::Scalar(value) => value.clamp(0.0, 1.0),
            _ => sample.progress,
        };
        match kind {
            crate::component_animation_kinds::SWITCH => {
                if let Some(mut visual) = self
                    .widget_visual_as::<crate::SwitchVisual>(sample.target)
                    .cloned()
                {
                    visual.thumb_progress = scalar_01();
                    self.nodes.set_widget_visual(
                        sample.target,
                        Some(crate::WidgetVisualHandle::new(visual)),
                    );
                    self.mark(sample.target, DirtyMask::RENDER);
                    self.account_animation_dirty(DirtyMask::RENDER);
                    component_updates.push(sample.target);
                }
            }
            k @ (crate::component_animation_kinds::SCROLLBAR_FADE
            | crate::component_animation_kinds::SCROLLBAR_EXPAND_X
            | crate::component_animation_kinds::SCROLLBAR_EXPAND_Y) => {
                if let Some(mut visual) = self
                    .widget_visual_as::<crate::ScrollbarVisual>(sample.target)
                    .copied()
                {
                    let value = scalar_01();
                    match k {
                        crate::component_animation_kinds::SCROLLBAR_FADE => {
                            visual.presence = value;
                        }
                        crate::component_animation_kinds::SCROLLBAR_EXPAND_X => {
                            visual.expand[0] = value;
                        }
                        _ => visual.expand[1] = value,
                    }
                    self.nodes.set_widget_visual(
                        sample.target,
                        Some(crate::WidgetVisualHandle::new(visual)),
                    );
                    self.mark(sample.target, DirtyMask::RENDER);
                    self.account_animation_dirty(DirtyMask::RENDER);
                    component_updates.push(sample.target);
                }
            }
            crate::component_animation_kinds::HOVER => {
                if let Some(transition) = self.motion.hover_transitions.get_mut(&sample.target) {
                    transition.progress = sample.progress;
                }
                self.mark_hover_paint(sample.target);
                self.account_animation_dirty(DirtyMask::STYLE | DirtyMask::RENDER);
                if sample.finished {
                    self.motion.hover_transitions.remove(&sample.target);
                }
                component_updates.push(sample.target);
            }
            crate::component_animation_kinds::SPINNER => {
                if let Some(mut visual) = self
                    .widget_visual_as::<crate::SpinnerVisual>(sample.target)
                    .cloned()
                {
                    visual.phase = sample.progress;
                    self.nodes.set_widget_visual(
                        sample.target,
                        Some(crate::WidgetVisualHandle::new(visual)),
                    );
                    self.mark(sample.target, DirtyMask::RENDER);
                    self.account_animation_dirty(DirtyMask::RENDER);
                    component_updates.push(sample.target);
                }
            }
            // An indeterminate bar's sweep moves its segment and nothing
            // else: the bar's own projection carries the phase on.
            crate::component_animation_kinds::PROGRESS => {
                if let Some(mut visual) = self
                    .widget_visual_as::<crate::ProgressVisual>(sample.target)
                    .filter(|v| matches!(v.mode, crate::ProgressMode::Sweep { .. }))
                    .cloned()
                {
                    visual.mode = crate::ProgressMode::Sweep {
                        phase: sample.progress,
                    };
                    self.nodes.set_widget_visual(
                        sample.target,
                        Some(crate::WidgetVisualHandle::new(visual)),
                    );
                    self.mark(sample.target, DirtyMask::RENDER);
                    self.account_animation_dirty(DirtyMask::RENDER);
                    component_updates.push(sample.target);
                }
            }
            crate::component_animation_kinds::SURFACE_SCRIM => {
                self.update_scrim_presence(sample.target, scalar_01());
                self.mark(sample.target, DirtyMask::RENDER);
                self.account_animation_dirty(DirtyMask::RENDER);
                component_updates.push(sample.target);
                self.advance_surface_motion(sample);
            }
            crate::component_animation_kinds::SURFACE
            | crate::component_animation_kinds::SURFACE_POP => {
                self.advance_surface_motion(sample);
                if sample.finished {
                    component_updates.push(sample.target);
                }
            }
            crate::component_animation_kinds::LOADING => {
                self.advance_loading_phase(sample);
                component_updates.push(sample.target);
            }
            crate::component_animation_kinds::SIDEBAR
            | crate::component_animation_kinds::WORKSPACE => {
                self.account_animation_dirty(DirtyMask::LAYOUT);
                component_updates.push(sample.target);
            }
            _ => {}
        }
    }
}

impl UiWorld {
    /// Whether `id` currently owns an active animation timeline. Component
    /// projections query this to avoid re-starting a running timeline.
    pub fn animation_is_active(&self, id: AnimationId) -> bool {
        self.motion.animations.contains_key(&id)
    }

    /// Whether `id` finished but still holds its property, because it fills
    /// forwards. It holds until the same id runs again or its owner stops it
    /// ([`crate::MutationQueue::stop_animation`]).
    pub fn animation_is_held(&self, id: AnimationId) -> bool {
        !self.motion.animations.contains_key(&id)
            && self.motion.presentation.get(id.track_id()).is_some()
    }

    /// Running, or finished and still holding: stopping it changes something.
    pub fn animation_is_live(&self, id: AnimationId) -> bool {
        self.motion.animations.contains_key(&id)
            || self.motion.presentation.get(id.track_id()).is_some()
    }

    pub(crate) fn animation_timing_start(&self, id: AnimationId) -> Option<Duration> {
        self.motion
            .animations
            .get(&id)
            .map(|animation| animation.spec.timing.start)
    }

    /// Consume the lifecycle events commits produced since they were last
    /// delivered (a run cancelled, finished early, replaced, or removed with
    /// its node). Each event is delivered once: here, or by the next
    /// [`Self::advance_animations`], which also reports the completions its
    /// deadlines reach.
    pub fn take_animation_events(&mut self) -> Vec<AnimationEvent> {
        std::mem::take(&mut self.motion.pending_animation_events)
    }

    /// Whether commits left events no advance has delivered yet.
    pub fn has_pending_animation_events(&self) -> bool {
        !self.motion.pending_animation_events.is_empty()
    }
}

impl UiWorld {
    /// The next time the host should call [`Self::advance_animations`]: the
    /// earliest sample or completion deadline, or the current animation time
    /// while commits left events to deliver, so a cancel or a replacement
    /// reaches its waiter even when nothing else is due.
    pub fn next_animation_deadline(&self) -> Option<Duration> {
        let due = self
            .motion
            .animation_deadlines
            .first()
            .map(|(deadline, _)| *deadline);
        // Built-in component timelines (a spinner's turn, a hover fade) have
        // no waiter outside the framework: their events ride along with the
        // next advance instead of waking the host for themselves.
        if !self
            .motion
            .pending_animation_events
            .iter()
            .any(|event| !crate::animation::is_component_animation(event.id, event.target))
        {
            return due;
        }
        Some(due.map_or(self.motion.animation_now, |due| {
            due.min(self.motion.animation_now)
        }))
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
            self.motion.animation_now,
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
        let previous_layout = self
            .motion
            .animations
            .get(&id)
            .filter(|active| is_layout_length(active.spec.property))
            .map(|active| (active.spec.target, active.spec.property));
        let active = crate::animation::ActiveAnimation::new(spec.clone());
        let deadline = active.next_deadline;
        if let Some(previous) = self.motion.animations.insert(id, active) {
            self.motion
                .animation_deadlines
                .remove(&(previous.next_deadline, id));
            // Every run that starts ends with exactly one event. A run
            // replaced in flight is cancelled (CSS `transitioncancel`); one
            // that already reached its end but was not reaped yet finished.
            let kind = if previous.spec.to_motion_track().is_some_and(|track| {
                crate::evaluate_track(&track, self.motion.animation_now).finished
            }) {
                AnimationEventKind::Finished
            } else {
                AnimationEventKind::Cancelled
            };
            self.motion.pending_animation_events.push(AnimationEvent {
                id,
                target: previous.spec.target,
                kind,
            });
        }
        self.index_animation_deadline(deadline, id);
        self.install_presentation_overlay(&spec);
        if let Some((previous_target, previous_property)) = previous_layout
            && (previous_target != spec.target || previous_property != spec.property)
        {
            self.clear_settled_length_slot_if_unused(previous_target, previous_property);
        }
    }

    /// `Duration::MAX` means no CPU wake (a paused hold, or a compositor
    /// overlay without completion, which `compositor_needs_tick` presents), so
    /// it stays out of the index instead of reporting a deadline ~584 billion
    /// years away.
    fn index_animation_deadline(&mut self, deadline: Duration, id: AnimationId) {
        if deadline != Duration::MAX {
            self.motion.animation_deadlines.insert((deadline, id));
        }
    }

    pub(super) fn cancel_animation(&mut self, id: AnimationId) -> bool {
        let Some(animation) = self.motion.animations.remove(&id) else {
            // A finished run's hold, if it left one.
            self.unbind_presentation(id.track_id());
            return false;
        };
        self.motion
            .animation_deadlines
            .remove(&(animation.next_deadline, id));
        self.unbind_presentation(id.track_id());
        self.motion.pending_animation_events.push(AnimationEvent {
            id,
            target: animation.spec.target,
            kind: AnimationEventKind::Cancelled,
        });
        true
    }

    pub(super) fn finish_animation(&mut self, id: AnimationId) -> bool {
        let Some(animation) = self.motion.animations.remove(&id) else {
            return false;
        };
        self.motion
            .animation_deadlines
            .remove(&(animation.next_deadline, id));
        let now = self.motion.animation_now;
        let mut spec = animation.spec;
        let fill_forwards = spec.playback.fill_mode.applies_forwards();
        snap_spec_to_completion(&mut spec, now);
        if (fill_forwards || is_layout_length(spec.property)) && spec.has_overlay() {
            self.install_presentation_overlay(&spec);
        } else {
            self.unbind_presentation(id.track_id());
        }
        self.motion.pending_animation_events.push(AnimationEvent {
            id,
            target: spec.target,
            kind: AnimationEventKind::Finished,
        });
        true
    }

    pub(super) fn reverse_animation(&mut self, id: AnimationId) -> bool {
        let Some(animation) = self.motion.animations.get(&id) else {
            return false;
        };
        let Some(track) = animation.spec.to_motion_track() else {
            return false;
        };
        let now = self.motion.animation_now.max(animation.spec.timing.start);
        let destination = track.from;
        let next = retarget_track(&track, now, destination);
        let mut spec = animation.spec.clone();
        apply_track_to_spec(&mut spec, &next);
        spec.interrupt = MotionInterrupt::Retarget;
        self.install_animation(spec);
        true
    }

    pub(super) fn pause_animation(&mut self, id: AnimationId) -> bool {
        let now = self.motion.animation_now;
        let Some(deadline) = self
            .motion
            .animations
            .get(&id)
            .map(|animation| animation.next_deadline)
        else {
            return false;
        };
        self.motion.animation_deadlines.remove(&(deadline, id));
        let Some(animation) = self.motion.animations.get_mut(&id) else {
            return false;
        };
        animation.spec.playback.play_state = crate::AnimationPlayState::Paused;
        animation.spec.playback.paused_at = Some(now);
        animation.next_deadline = Duration::MAX;
        if let Some(overlay) = self.motion.presentation.get_mut(id.track_id()) {
            overlay.track.pause_at(now);
            let _ = self.motion.motion_descriptors.bind(&overlay.track);
        }
        true
    }

    pub(super) fn resume_animation(&mut self, id: AnimationId) -> bool {
        let Some(animation) = self.motion.animations.get(&id) else {
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
        let now = self.motion.animation_now;
        let next = if compositor {
            match completion {
                Some(deadline) if deadline > now => deadline,
                // Past its estimated end without having settled: look again
                // shortly rather than never, or its finish would be lost.
                Some(_) => now.checked_add(interval).unwrap_or(Duration::MAX),
                None => Duration::MAX,
            }
        } else {
            let step = now.checked_add(interval).unwrap_or(now);
            match end {
                Some(end) => step.min(end),
                None => step,
            }
        };
        self.motion.animation_deadlines.remove(&(previous, id));
        let Some(animation) = self.motion.animations.get_mut(&id) else {
            return false;
        };
        animation.spec.playback.play_state = crate::AnimationPlayState::Running;
        animation.spec.playback.paused_at = None;
        animation.next_deadline = next;
        if let Some(overlay) = self.motion.presentation.get_mut(id.track_id()) {
            overlay.track.resume();
            let _ = self.motion.motion_descriptors.bind(&overlay.track);
        }
        self.index_animation_deadline(next, id);
        true
    }

    fn apply_retarget(&self, mut spec: AnimationSpec) -> AnimationSpec {
        if spec.interrupt != MotionInterrupt::Retarget {
            return spec;
        }
        let now = spec.timing.start.max(self.motion.animation_now);
        let Some(rest) = spec.to_motion_track().map(|track| track.rest_value()) else {
            return spec;
        };
        if let Some(previous) = self
            .motion.animations
            .get(&spec.id)
            .and_then(|active| active.spec.to_motion_track())
            // A run that has ended but not been reaped yet is not in flight:
            // what it presents now is found below.
            && let sample = crate::evaluate_track(&previous, now)
            && !sample.finished
        {
            // Only the starting state carries over; timing and curve stay the
            // new run's own, so a zero-length transition does not inherit the
            // old duration. An in-flight value does not freeze for the delay.
            spec.from = sample.value;
            spec.velocity = sample.velocity;
            spec.timing.start = now;
            spec.timing.delay = Duration::ZERO;
            return spec;
        }
        if let Some(target) = MotionTargetId::new(spec.target.get())
            && let Some(sample) = self.motion.presentation.sample(target, spec.property, now)
            && let Some(value) = sample.applied_value()
            // An axis a keyframe leaves out is absent: no value to start from.
            && !value.is_absent()
        {
            spec.from = value;
            spec.timing.start = now;
            if sample.finished && !spec.timing.delay.is_zero() && fills_back_to_from(&spec) {
                // A finished run's forwards hold: nothing is moving, so as
                // with a logical start value the run keeps its own delay.
                // It replaces the hold when it shares its id, so it fills
                // backwards: the held value, where it starts, shows until then.
                spec.velocity = value.zero_velocity();
                spec.playback.fill_mode = match spec.playback.fill_mode {
                    AnimationFillMode::None | AnimationFillMode::Backwards => {
                        AnimationFillMode::Backwards
                    }
                    AnimationFillMode::Forwards | AnimationFillMode::Both => {
                        AnimationFillMode::Both
                    }
                };
            } else {
                spec.velocity = sample.velocity;
                spec.timing.delay = Duration::ZERO;
            }
            return spec;
        }
        if spec.from == rest
            && let Some(logical) = self.logical_motion_value(spec.target, spec.property)
        {
            // Nothing is in flight here, only a start value to find, so the
            // run keeps its own delay: during it the logical value shows,
            // which is where the run starts.
            spec.from = logical;
            spec.velocity = logical.zero_velocity();
            spec.timing.start = now;
        }
        spec
    }

    fn install_presentation_overlay(&mut self, spec: &AnimationSpec) {
        if !spec.has_overlay() {
            self.unbind_presentation(spec.id.track_id());
            return;
        }
        let Some(mut track) = spec.to_motion_track() else {
            return;
        };
        if is_layout_length(track.property) {
            track.playback.fill_mode = match track.playback.fill_mode {
                AnimationFillMode::None => AnimationFillMode::Forwards,
                AnimationFillMode::Backwards => AnimationFillMode::Both,
                other => other,
            };
            self.motion
                .settled_length_overlays
                .entry(spec.target)
                .or_default();
        }
        let logical = self
            .logical_motion_value(spec.target, spec.property)
            .unwrap_or(track.rest_value());
        if spec.uses_presentation_overlay() {
            let _ = self.motion.motion_descriptors.bind(&track);
        }
        self.motion
            .presentation
            .insert_in_layer(track, logical, spec.layer);
        match spec.property {
            crate::AnimatableProperty::FontAxis(_) => {
                // A replaced run may have been holding the axis at another value.
                self.mark_font_axes_changed(spec.target);
            }
            crate::AnimatableProperty::Background => {
                self.mark(spec.target, DirtyMask::STYLE | DirtyMask::RENDER);
            }
            crate::AnimatableProperty::Color => {
                self.mark_subtree(spec.target, DirtyMask::STYLE | DirtyMask::RENDER);
            }
            _ => {}
        }
    }

    fn clear_settled_length_slot_if_unused(
        &mut self,
        target: StableNodeId,
        property: crate::AnimatableProperty,
    ) {
        if let Some(motion_target) = MotionTargetId::new(target.get())
            && self
                .motion
                .presentation
                .properties_of(motion_target)
                .any(|p| p == property)
        {
            return;
        }
        if let Some(overlay) = self.motion.settled_length_overlays.get_mut(&target) {
            overlay.set(property, None);
        }
        let has_any_length = MotionTargetId::new(target.get()).is_some_and(|motion_target| {
            self.motion
                .presentation
                .properties_of(motion_target)
                .any(is_layout_length)
        });
        if !has_any_length {
            self.motion.settled_length_overlays.remove(&target);
        }
    }

    fn unbind_presentation(&mut self, id: nana_ui_core::motion::MotionTrackId) {
        if let Some(overlay) = self.motion.presentation.remove(id)
            && let Some(target) = StableNodeId::new(overlay.track.target.get())
        {
            match overlay.track.property {
                crate::AnimatableProperty::FontAxis(_) => {
                    // The axis falls back to what the style says without it.
                    self.mark_font_axes_changed(target);
                }
                crate::AnimatableProperty::Background => {
                    self.mark(target, DirtyMask::STYLE | DirtyMask::RENDER);
                }
                crate::AnimatableProperty::Color => {
                    self.mark_subtree(target, DirtyMask::STYLE | DirtyMask::RENDER);
                }
                prop if is_layout_length(prop) => {
                    self.clear_settled_length_slot_if_unused(target, prop);
                    if self.contains(target) {
                        if prop != crate::AnimatableProperty::Margin {
                            self.nodes
                                .invalidate_text(target, crate::text_node::TextDirty::CONSTRAINT);
                        }
                        let change = match prop {
                            crate::AnimatableProperty::Width => {
                                nana_ui_core::LayoutStyleChange::WIDTH
                            }
                            crate::AnimatableProperty::Height => {
                                nana_ui_core::LayoutStyleChange::HEIGHT
                            }
                            _ => nana_ui_core::LayoutStyleChange::SPACING,
                        };
                        let invalidation = self.classify_layout_change(target, change);
                        self.record_layout_invalidation(target, invalidation);
                        self.mark_subtree(
                            target,
                            DirtyMask::INPUT | DirtyMask::ACCESSIBILITY | DirtyMask::RENDER,
                        );
                    }
                }
                _ => {}
            }
        }
        self.motion.motion_descriptors.cancel(id);
    }
}

impl UiWorld {
    /// A finished app or component run holds its value over the logical
    /// style only until the property is written again with another value:
    /// the later write is what the author means now. Writing the same value
    /// back — a component re-projecting its style, a cascade sync — keeps
    /// the hold. CSS animation holds are the cascade's to end
    /// (`animation-name`), as in CSS.
    pub(super) fn release_rewritten_holds(
        &mut self,
        id: StableNodeId,
        previous: &nana_ui_core::LayoutStyle,
        next: &nana_ui_core::LayoutStyle,
    ) {
        let Some(target) = MotionTargetId::new(id.get()) else {
            return;
        };
        let rewritten: Vec<crate::AnimatableProperty> = self
            .motion
            .presentation
            .properties_of(target)
            .filter(|property| match property {
                crate::AnimatableProperty::Opacity => previous.opacity != next.opacity,
                crate::AnimatableProperty::Background => {
                    previous.background != next.background
                        || previous.paint_colors.background != next.paint_colors.background
                }
                crate::AnimatableProperty::Color => {
                    previous.color != next.color
                        || previous.paint_colors.color != next.paint_colors.color
                }
                crate::AnimatableProperty::Transform => {
                    !previous.transforms.ptr_eq(&next.transforms)
                        && (previous.transforms.transform != next.transforms.transform
                            || previous.transforms.transform_3d != next.transforms.transform_3d)
                }
                crate::AnimatableProperty::FontAxis(tag) => {
                    let axis = |layout: &nana_ui_core::LayoutStyle| {
                        layout
                            .text_ext
                            .font_variation_settings
                            .as_deref()
                            .map(|axes| nana_ui_core::FontVariationSetting::axis_value(axes, *tag))
                    };
                    axis(previous) != axis(next)
                }
                crate::AnimatableProperty::Width => previous.width != next.width,
                crate::AnimatableProperty::Height => previous.height != next.height,
                crate::AnimatableProperty::Padding => previous.padding != next.padding,
                crate::AnimatableProperty::Margin => previous.margin != next.margin,
                _ => false,
            })
            .collect();
        if rewritten.is_empty() {
            return;
        }
        // The FLIP invert hold is the list move's, which ends it with its play
        // track; a style write in between does not.
        let flip = crate::component_animation_id(crate::component_animation_kinds::FLIP, id);
        for property in rewritten {
            let held: Vec<_> = self
                .motion
                .presentation
                .track_ids(target, property)
                .filter(|track| {
                    let animation = AnimationId::new(track.get()).expect("track ids are nonzero");
                    Some(animation) != flip
                        && !self.motion.animations.contains_key(&animation)
                        && self
                            .motion
                            .presentation
                            .get(*track)
                            .is_some_and(|overlay| overlay.layer == crate::MotionLayer::Runtime)
                })
                .collect();
            for track in held {
                self.unbind_presentation(track);
            }
        }
    }
}

fn is_layout_length(property: crate::AnimatableProperty) -> bool {
    matches!(
        property,
        crate::AnimatableProperty::Width
            | crate::AnimatableProperty::Height
            | crate::AnimatableProperty::Padding
            | crate::AnimatableProperty::Margin
    )
}

/// Whether filling `spec` backwards shows its `from`: its first iteration
/// plays forwards from it to one value.
fn fills_back_to_from(spec: &AnimationSpec) -> bool {
    matches!(spec.to, crate::MotionTo::Value(_))
        && matches!(
            spec.playback.direction,
            AnimationDirection::Normal | AnimationDirection::Alternate
        )
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
    // The value the run ends on, not its last keyframe: an alternating or
    // reversed run ends on another stop.
    let rest = spec
        .to_motion_track()
        .map(|track| {
            if track.curve.is_physics() {
                // A spring or decay settles within a tolerance of its rest;
                // finishing it is the rest itself.
                return track.rest_value();
            }
            nana_ui_core::motion::track_completion_deadline(&track)
                .map(|end| crate::evaluate_track(&track, end).value)
                .unwrap_or_else(|| track.rest_value())
        })
        .unwrap_or(spec.from);
    spec.from = rest;
    spec.to = crate::MotionTo::Value(rest);
    spec.timing.delay = Duration::ZERO;
    spec.timing.start = Duration::ZERO;
    spec.timing.duration = Duration::ZERO;
    // An endless zero-length run is invalid and would not rebind.
    spec.playback.iteration_count = crate::AnimationIteration::ONCE;
}

impl UiWorld {
    pub(super) fn cancel_animations_for_removed(&mut self, id: StableNodeId) {
        let cancelled = self
            .motion
            .animations
            .iter()
            .filter_map(|(&animation_id, animation)| {
                (animation.spec.target == id).then_some((animation_id, animation.next_deadline))
            })
            .collect::<Vec<_>>();
        self.motion.settled_length_overlays.remove(&id);
        for (animation_id, deadline) in cancelled {
            self.motion.animations.remove(&animation_id);
            self.motion
                .animation_deadlines
                .remove(&(deadline, animation_id));
            self.motion.pending_animation_events.push(AnimationEvent {
                id: animation_id,
                target: id,
                kind: AnimationEventKind::Cancelled,
            });
        }
        if let Some(target) = MotionTargetId::new(id.get()) {
            self.motion.presentation.remove_target(target);
            self.motion.motion_descriptors.cancel_target(target);
        }
    }

    pub(super) fn drop_non_overlay_animations_for_parked(
        &mut self,
        parked: &HashSet<StableNodeId>,
    ) {
        let cancelled = self
            .motion
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
            self.motion.animations.remove(&animation_id);
            self.motion
                .animation_deadlines
                .remove(&(deadline, animation_id));
            self.unbind_presentation(animation_id.track_id());
            self.motion.pending_animation_events.push(AnimationEvent {
                id: animation_id,
                target,
                kind: AnimationEventKind::Cancelled,
            });
        }
    }
}
