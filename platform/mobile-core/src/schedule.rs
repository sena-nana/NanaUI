//! When the shell should present next.

use std::time::Instant;

/// What the document asks of the shell's display link.
///
/// `Idle` pauses it (`CADisplayLink.isPaused`, no Choreographer callback):
/// nothing is animating and nothing changed. `At` is the next animation,
/// caret or timer deadline, `Now` a frame at the next vsync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDemand {
    Idle,
    At(Instant),
    Now,
}

impl FrameDemand {
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Now, _) | (_, Self::Now) => Self::Now,
            (Self::At(left), Self::At(right)) => Self::At(left.min(right)),
            (at @ Self::At(_), Self::Idle) | (Self::Idle, at @ Self::At(_)) => at,
            (Self::Idle, Self::Idle) => Self::Idle,
        }
    }

    /// `At` a deadline that has passed is a frame now.
    pub fn resolve(self, now: Instant) -> Self {
        match self {
            Self::At(deadline) if deadline <= now => Self::Now,
            other => other,
        }
    }

    pub fn from_deadline(deadline: Option<Instant>) -> Self {
        deadline.map_or(Self::Idle, Self::At)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn merge_keeps_the_most_urgent() {
        let now = Instant::now();
        let soon = FrameDemand::At(now + Duration::from_millis(5));
        let later = FrameDemand::At(now + Duration::from_millis(50));
        assert_eq!(soon.merge(later), soon);
        assert_eq!(later.merge(FrameDemand::Idle), later);
        assert_eq!(FrameDemand::Idle.merge(FrameDemand::Now), FrameDemand::Now);
        assert_eq!(
            FrameDemand::Idle.merge(FrameDemand::Idle),
            FrameDemand::Idle
        );
    }

    #[test]
    fn a_passed_deadline_resolves_to_now() {
        let now = Instant::now();
        assert_eq!(FrameDemand::At(now).resolve(now), FrameDemand::Now);
        let later = FrameDemand::At(now + Duration::from_millis(1));
        assert_eq!(later.resolve(now), later);
    }
}
