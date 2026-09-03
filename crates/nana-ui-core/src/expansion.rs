//! Backend-neutral expansion state with deterministic transition sampling.

use std::time::Duration;

use crate::motion::Easing;

#[derive(Debug, Clone, Copy)]
struct ExpansionTransition {
    started_at: Duration,
    from: f32,
    to: f32,
}

/// Owns a boolean expansion target and its visual transition without a host clock.
#[derive(Debug, Clone)]
pub struct ExpansionState {
    expanded: bool,
    duration: Duration,
    transition: Option<ExpansionTransition>,
}

impl ExpansionState {
    pub fn new(expanded: bool, duration: Duration) -> Self {
        Self {
            expanded,
            duration,
            transition: None,
        }
    }

    pub fn expanded(&self) -> bool {
        self.expanded
    }

    pub fn set_expanded(&mut self, expanded: bool, now: Duration) -> bool {
        if self.expanded == expanded {
            return false;
        }
        let from = self.value_at(now);
        self.expanded = expanded;
        self.transition = if self.duration.is_zero() {
            None
        } else {
            Some(ExpansionTransition {
                started_at: now,
                from,
                to: if expanded { 1.0 } else { 0.0 },
            })
        };
        true
    }

    pub fn toggle(&mut self, now: Duration) -> bool {
        self.set_expanded(!self.expanded, now)
    }

    pub fn is_animating_at(&self, now: Duration) -> bool {
        self.transition
            .is_some_and(|transition| now.saturating_sub(transition.started_at) < self.duration)
    }

    pub fn value_at(&self, now: Duration) -> f32 {
        let Some(transition) = self.transition else {
            return if self.expanded { 1.0 } else { 0.0 };
        };
        if self.duration.is_zero() {
            return transition.to;
        }
        let linear = (now.saturating_sub(transition.started_at).as_secs_f32()
            / self.duration.as_secs_f32())
        .clamp(0.0, 1.0);
        let progress = Easing::EaseInOutCubic.sample(linear);
        transition.from + (transition.to - transition.from) * progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_is_deterministic_and_reverses_from_the_sampled_value() {
        let mut state = ExpansionState::new(true, Duration::from_millis(160));
        assert!(state.set_expanded(false, Duration::from_millis(100)));
        let reversed_at = Duration::from_millis(180);
        let middle = state.value_at(reversed_at);
        // elapsed 恰为时长一半：EaseInOutCubic 中点必须落在 0.5，
        // 而 ease-out-cubic 会给出 0.875。
        assert_eq!(middle, 0.5);

        assert!(state.set_expanded(true, reversed_at));
        assert_eq!(state.value_at(reversed_at), middle);
        assert!(state.is_animating_at(Duration::from_millis(339)));
        assert!(!state.is_animating_at(Duration::from_millis(340)));
        assert_eq!(state.value_at(Duration::from_millis(340)), 1.0);
    }
}
