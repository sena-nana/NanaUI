use std::time::Duration;

use crate::StableNodeId;

/// Stable identity for one logical animation. Starting the same ID again
/// atomically replaces its active timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnimationId(u64);

impl AnimationId {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Easing {
    #[default]
    Linear,
    EaseOutCubic,
    EaseInOutCubic,
}

impl Easing {
    fn sample(self, progress: f32) -> f32 {
        match self {
            Self::Linear => progress,
            Self::EaseOutCubic => 1.0 - (1.0 - progress).powi(3),
            Self::EaseInOutCubic if progress < 0.5 => 4.0 * progress.powi(3),
            Self::EaseInOutCubic => 1.0 - (-2.0 * progress + 2.0).powi(3) / 2.0,
        }
    }
}

/// Backend-neutral animation timing. The host owns the monotonic clock and
/// passes timestamps from the same epoch to [`crate::UiWorld::advance_animations`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimationSpec {
    pub id: AnimationId,
    pub target: StableNodeId,
    pub start: Duration,
    pub duration: Duration,
    pub frame_interval: Duration,
    pub easing: Easing,
}

impl AnimationSpec {
    pub(crate) fn end(self) -> Option<Duration> {
        self.start.checked_add(self.duration)
    }

    pub(crate) fn is_valid(self) -> bool {
        !self.duration.is_zero() && !self.frame_interval.is_zero() && self.end().is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationSample {
    pub id: AnimationId,
    pub target: StableNodeId,
    /// Eased progress in the inclusive range `0.0..=1.0`.
    pub progress: f32,
    pub finished: bool,
}

/// Samples due at the supplied timestamp and the next time the host should
/// wake the UI runtime. An empty static UI has no deadline.
/// `animation_deadlines_scanned` / `animations_considered` count deadline-index
/// entries examined and animation records looked up this call.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationFrame {
    pub samples: Vec<AnimationSample>,
    /// Framework-owned component lifecycle updates applied at this wake.
    /// These are already committed to retained state; applications must not
    /// re-apply them.
    pub component_updates: Vec<StableNodeId>,
    pub next_deadline: Option<Duration>,
    pub animation_deadlines_scanned: usize,
    pub animations_considered: usize,
}

impl AnimationFrame {
    pub fn has_updates(&self) -> bool {
        !self.samples.is_empty() || !self.component_updates.is_empty()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveAnimation {
    pub(crate) spec: AnimationSpec,
    pub(crate) next_deadline: Duration,
}

impl ActiveAnimation {
    pub(crate) fn new(spec: AnimationSpec) -> Self {
        Self {
            next_deadline: spec.start,
            spec,
        }
    }

    pub(crate) fn sample(&mut self, now: Duration) -> Option<AnimationSample> {
        if now < self.next_deadline {
            return None;
        }
        let end = self
            .spec
            .end()
            .expect("validated animation must have an end");
        let finished = now >= end;
        let linear = if finished {
            1.0
        } else {
            now.saturating_sub(self.spec.start).as_secs_f32() / self.spec.duration.as_secs_f32()
        };
        if !finished {
            self.next_deadline = now
                .checked_add(self.spec.frame_interval)
                .unwrap_or(end)
                .min(end);
        }
        Some(AnimationSample {
            id: self.spec.id,
            target: self.spec.target,
            progress: self.spec.easing.sample(linear.clamp(0.0, 1.0)),
            finished,
        })
    }
}
