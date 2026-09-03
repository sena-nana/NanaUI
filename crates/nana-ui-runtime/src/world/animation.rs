//! Runtime animation sampling.

use super::*;

impl UiWorld {
    /// Sample only due timelines. Built-in paint transitions publish their
    /// values and targeted invalidation here; other consumers receive samples.
    pub fn advance_animations(&mut self, now: Duration) -> AnimationFrame {
        self.animation_now = now;
        let mut component_updates = Vec::new();
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
            let (sample, next_deadline) = {
                let animation = self
                    .animations
                    .get_mut(&id)
                    .expect("due animation must remain active");
                let sample = animation
                    .sample(now)
                    .expect("due animation must produce a sample");
                let next_deadline = (!sample.finished && animation.has_follow_up_deadline())
                    .then_some(animation.next_deadline);
                (sample, next_deadline)
            };
            if sample.finished {
                self.animations.remove(&id);
            } else if let Some(next_deadline) = next_deadline {
                self.animation_deadlines.insert((next_deadline, id));
            }
            if crate::component_animation_id(
                crate::component_animation_kinds::SWITCH,
                sample.target,
            ) == Some(sample.id)
                && let Some(from) = self.switch_transitions.get(&sample.target).copied()
                && let Some(mut visual @ StandardVisual::Switch { .. }) =
                    self.standard_visual(sample.target)
            {
                if let StandardVisual::Switch {
                    checked,
                    thumb_progress,
                    ..
                } = &mut visual
                {
                    *thumb_progress = from + (f32::from(*checked) - from) * sample.progress;
                }
                self.nodes.set_visual(sample.target, Some(visual));
                self.mark(sample.target, DirtyMask::RENDER);
                component_updates.push(sample.target);
                if sample.finished {
                    self.switch_transitions.remove(&sample.target);
                }
            }
            if crate::component_animation_id(crate::component_animation_kinds::HOVER, sample.target)
                == Some(sample.id)
            {
                self.mark_hover_paint(sample.target);
                if sample.finished {
                    self.hover_transitions.remove(&sample.target);
                }
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
}

impl UiWorld {
    pub fn next_animation_deadline(&self) -> Option<Duration> {
        self.animation_deadlines
            .first()
            .map(|(deadline, _)| *deadline)
    }
}

impl UiWorld {
    pub(super) fn start_component_animation(
        &mut self,
        target: StableNodeId,
        kind: u64,
        duration: Duration,
        easing: crate::Easing,
    ) {
        let Some(id) = crate::component_animation_id(kind, target) else {
            return;
        };
        let spec = AnimationSpec::new(
            id,
            target,
            self.animation_now,
            duration,
            crate::framework::COMPONENT_FRAME_INTERVAL,
            easing,
        );
        let active = ActiveAnimation::new(spec);
        let deadline = active.next_deadline;
        if let Some(previous) = self.animations.insert(id, active) {
            self.animation_deadlines
                .remove(&(previous.next_deadline, id));
        }
        self.animation_deadlines.insert((deadline, id));
    }
}
