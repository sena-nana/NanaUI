//! Runtime animation sampling.

use super::*;

impl UiWorld {
    /// Sample only active animations that are due at `now`. This method does
    /// not mark render state dirty: consumers apply sampled values through the
    /// normal atomic mutation boundary.
    pub fn advance_animations(&mut self, now: Duration) -> AnimationFrame {
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
            samples.push(sample);
        }
        AnimationFrame {
            samples,
            component_updates: Vec::new(),
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
