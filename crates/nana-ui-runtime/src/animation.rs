use std::hash::Hasher;
use std::time::Duration;

use crate::StableNodeId;

pub use nana_ui_core::motion::Easing;

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

/// Component-kind tags for [`component_animation_id`]. Every component family
/// that starts its own timelines reserves one stable non-zero constant here,
/// so hashed IDs stay in per-component namespaces instead of colliding across
/// component types whose node IDs share one numeric space.
pub mod component_animation_kinds {
    /// Skeleton pulse timeline.
    pub const SKELETON: u64 = 1;
    pub const SWITCH: u64 = 2;
    pub const HOVER: u64 = 3;
}

/// Derives the animation ID for one component-owned timeline from the
/// component's kind tag and its node. Re-deriving the same pair yields the
/// same ID (restarting replaces that timeline); different component kinds on
/// identically numbered nodes never replace each other. Returns `None` in the
/// negligible case of a zero hash, which [`AnimationId`] rejects.
pub fn component_animation_id(kind_tag: u64, target: StableNodeId) -> Option<AnimationId> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write_u64(kind_tag);
    hasher.write_u64(target.get());
    AnimationId::new(hasher.finish())
}

/// CSS `animation-iteration-count`. Default is a single run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationIteration {
    Count(u32),
    Infinite,
}

impl AnimationIteration {
    pub const ONCE: Self = Self::Count(1);
    pub const INFINITE: Self = Self::Infinite;
}

impl Default for AnimationIteration {
    fn default() -> Self {
        Self::ONCE
    }
}

/// CSS `animation-direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationDirection {
    #[default]
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

impl AnimationDirection {
    fn start_progress(self) -> f32 {
        match self {
            Self::Normal | Self::Alternate => 0.0,
            Self::Reverse | Self::AlternateReverse => 1.0,
        }
    }

    fn map_progress(self, iteration_index: u32, linear: f32) -> f32 {
        let reverse = match self {
            Self::Normal => false,
            Self::Reverse => true,
            Self::Alternate => !iteration_index.is_multiple_of(2),
            Self::AlternateReverse => iteration_index.is_multiple_of(2),
        };
        if reverse { 1.0 - linear } else { linear }
    }

    fn end_progress(self, completed_iterations: u32) -> f32 {
        if completed_iterations == 0 {
            return self.start_progress();
        }
        self.map_progress(completed_iterations.saturating_sub(1), 1.0)
    }
}

/// CSS `animation-fill-mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationFillMode {
    #[default]
    None,
    Forwards,
    Backwards,
    Both,
}

impl AnimationFillMode {
    fn applies_backwards(self) -> bool {
        matches!(self, Self::Backwards | Self::Both)
    }
}

/// CSS `animation-play-state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationPlayState {
    #[default]
    Running,
    Paused,
}

/// CSS playback longhands. Kept beside [`AnimationSpec`] so existing six-field
/// spec literals (including CSS cascade builders) keep compiling; pass these
/// through [`crate::MutationQueue::start_animation_with_playback`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnimationPlayback {
    pub iteration_count: AnimationIteration,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
}

/// Backend-neutral animation timing. The host owns the monotonic clock and
/// passes timestamps from the same epoch to [`crate::UiWorld::advance_animations`].
///
/// Playback longhands have no field defaults (rustc 1.92/1.97 still treats
/// default field values as experimental). Use [`AnimationSpec::new`] for the
/// six-field one-shot API, or write all four playback fields on every literal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationSpec {
    pub id: AnimationId,
    pub target: StableNodeId,
    pub start: Duration,
    pub duration: Duration,
    pub frame_interval: Duration,
    pub easing: Easing,
    pub iteration_count: AnimationIteration,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
}

impl AnimationSpec {
    /// Six-field constructor. Playback is one-shot / normal / none / running.
    pub const fn new(
        id: AnimationId,
        target: StableNodeId,
        start: Duration,
        duration: Duration,
        frame_interval: Duration,
        easing: Easing,
    ) -> Self {
        Self {
            id,
            target,
            start,
            duration,
            frame_interval,
            easing,
            iteration_count: AnimationIteration::ONCE,
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
        }
    }

    pub fn with_playback(mut self, playback: AnimationPlayback) -> Self {
        self.iteration_count = playback.iteration_count;
        self.direction = playback.direction;
        self.fill_mode = playback.fill_mode;
        self.play_state = playback.play_state;
        self
    }

    pub(crate) fn end(self) -> Option<Duration> {
        match self.iteration_count {
            AnimationIteration::Infinite => None,
            AnimationIteration::Count(0) => Some(self.start),
            AnimationIteration::Count(count) => {
                self.start.checked_add(self.duration.saturating_mul(count))
            }
        }
    }

    pub(crate) fn is_valid(self) -> bool {
        if self.duration.is_zero() || self.frame_interval.is_zero() {
            return false;
        }
        match self.iteration_count {
            AnimationIteration::Count(0) => false,
            AnimationIteration::Infinite => true,
            AnimationIteration::Count(_) => self.end().is_some(),
        }
    }

    fn running(self) -> bool {
        self.play_state == AnimationPlayState::Running
    }

    fn local_linear(self, now: Duration) -> (f32, bool) {
        if now < self.start {
            return (self.direction.start_progress(), false);
        }
        let elapsed = now.saturating_sub(self.start);
        match self.iteration_count {
            AnimationIteration::Infinite => {
                let duration = self.duration.as_secs_f32();
                if duration <= 0.0 {
                    return (self.direction.end_progress(1), false);
                }
                let t = elapsed.as_secs_f32() / duration;
                let iteration_index = t.floor() as u32;
                let linear = (t - iteration_index as f32).clamp(0.0, 1.0);
                (self.direction.map_progress(iteration_index, linear), false)
            }
            AnimationIteration::Count(count) => {
                let Some(end) = self.end() else {
                    return (self.direction.end_progress(count), true);
                };
                if now >= end {
                    return (self.direction.end_progress(count), true);
                }
                let duration = self.duration.as_secs_f32();
                if duration <= 0.0 {
                    return (self.direction.end_progress(count), true);
                }
                let t = elapsed.as_secs_f32() / duration;
                let iteration_index = (t.floor() as u32).min(count.saturating_sub(1));
                let linear = (t - iteration_index as f32).clamp(0.0, 1.0);
                (self.direction.map_progress(iteration_index, linear), false)
            }
        }
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
    hold_at: Option<Duration>,
}

impl ActiveAnimation {
    pub(crate) fn new(spec: AnimationSpec) -> Self {
        let next_deadline = if spec.fill_mode.applies_backwards() {
            Duration::ZERO
        } else {
            spec.start
        };
        Self {
            hold_at: matches!(spec.play_state, AnimationPlayState::Paused).then_some(spec.start),
            spec,
            next_deadline,
        }
    }

    pub(crate) fn has_follow_up_deadline(&self) -> bool {
        self.spec.running() && self.hold_at.is_none()
    }

    pub(crate) fn sample(&mut self, now: Duration) -> Option<AnimationSample> {
        if now < self.next_deadline {
            return None;
        }
        let clock = self.hold_at.unwrap_or(now);
        let (linear, finished) = self.spec.local_linear(clock);
        if !finished && self.has_follow_up_deadline() {
            let step = now.checked_add(self.spec.frame_interval).unwrap_or(now);
            self.next_deadline = match self.spec.end() {
                Some(end) => step.min(end),
                None => step,
            };
        } else if !finished {
            // Paused hold: stay in the map, but do not wake again until replaced.
            self.next_deadline = Duration::MAX;
        }
        Some(AnimationSample {
            id: self.spec.id,
            target: self.spec.target,
            progress: self.spec.easing.sample(linear.clamp(0.0, 1.0)),
            finished,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StableNodeId;

    fn spec(start_ms: u64, duration_ms: u64) -> AnimationSpec {
        AnimationSpec::new(
            AnimationId::new(1).unwrap(),
            StableNodeId::new(1).unwrap(),
            Duration::from_millis(start_ms),
            Duration::from_millis(duration_ms),
            Duration::from_millis(10),
            Easing::Linear,
        )
    }

    fn progress_at(spec: AnimationSpec, now_ms: u64) -> AnimationSample {
        let mut active = ActiveAnimation::new(spec);
        active.next_deadline = Duration::ZERO;
        active
            .sample(Duration::from_millis(now_ms))
            .expect("sample")
    }

    #[test]
    fn default_animation_playback_is_one_shot_normal_running() {
        let spec = spec(100, 100);
        assert_eq!(spec.iteration_count, AnimationIteration::ONCE);
        assert_eq!(spec.direction, AnimationDirection::Normal);
        assert_eq!(spec.fill_mode, AnimationFillMode::None);
        assert_eq!(spec.play_state, AnimationPlayState::Running);
        assert_eq!(spec.end(), Some(Duration::from_millis(200)));
        let mid = progress_at(spec, 150);
        assert!((mid.progress - 0.5).abs() < f32::EPSILON);
        assert!(!mid.finished);
        let end = progress_at(spec, 200);
        assert_eq!(end.progress, 1.0);
        assert!(end.finished);
    }

    #[test]
    fn animation_iteration_count_repeats_before_finish() {
        let spec = AnimationSpec {
            iteration_count: AnimationIteration::Count(2),
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
            ..spec(0, 100)
        };
        let first = progress_at(spec, 50);
        assert!((first.progress - 0.5).abs() < f32::EPSILON);
        assert!(!first.finished);
        let second = progress_at(spec, 150);
        assert!((second.progress - 0.5).abs() < f32::EPSILON);
        assert!(!second.finished);
        let done = progress_at(spec, 200);
        assert_eq!(done.progress, 1.0);
        assert!(done.finished);
    }

    #[test]
    fn infinite_animation_never_finishes() {
        let spec = AnimationSpec {
            iteration_count: AnimationIteration::INFINITE,
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
            ..spec(0, 100)
        };
        assert_eq!(spec.end(), None);
        assert!(spec.is_valid());
        let late = progress_at(spec, 10_000);
        assert!(!late.finished);
        assert!(late.progress >= 0.0 && late.progress <= 1.0);
    }

    #[test]
    fn reverse_direction_runs_from_one_to_zero() {
        let spec = AnimationSpec {
            iteration_count: AnimationIteration::ONCE,
            direction: AnimationDirection::Reverse,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
            ..spec(0, 100)
        };
        assert!((progress_at(spec, 0).progress - 1.0).abs() < f32::EPSILON);
        assert!((progress_at(spec, 50).progress - 0.5).abs() < f32::EPSILON);
        let done = progress_at(spec, 100);
        assert_eq!(done.progress, 0.0);
        assert!(done.finished);
    }

    #[test]
    fn alternate_direction_flips_each_iteration() {
        let spec = AnimationSpec {
            iteration_count: AnimationIteration::Count(2),
            direction: AnimationDirection::Alternate,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
            ..spec(0, 100)
        };
        assert!((progress_at(spec, 25).progress - 0.25).abs() < f32::EPSILON);
        assert!((progress_at(spec, 125).progress - 0.75).abs() < f32::EPSILON);
        let done = progress_at(spec, 200);
        assert_eq!(done.progress, 0.0);
        assert!(done.finished);
    }

    #[test]
    fn fill_mode_backwards_holds_start_progress_during_delay() {
        let spec = AnimationSpec {
            iteration_count: AnimationIteration::ONCE,
            direction: AnimationDirection::Reverse,
            fill_mode: AnimationFillMode::Backwards,
            play_state: AnimationPlayState::Running,
            ..spec(50, 100)
        };
        let held = progress_at(spec, 10);
        assert_eq!(held.progress, 1.0);
        assert!(!held.finished);
    }

    #[test]
    fn fill_mode_forwards_keeps_terminal_progress() {
        let spec = AnimationSpec {
            iteration_count: AnimationIteration::ONCE,
            direction: AnimationDirection::Reverse,
            fill_mode: AnimationFillMode::Forwards,
            play_state: AnimationPlayState::Running,
            ..spec(0, 100)
        };
        let done = progress_at(spec, 150);
        assert_eq!(done.progress, 0.0);
        assert!(done.finished);
    }

    #[test]
    fn paused_animation_does_not_schedule_further_frames() {
        let spec = AnimationSpec {
            iteration_count: AnimationIteration::ONCE,
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Paused,
            ..spec(0, 100)
        };
        let mut active = ActiveAnimation::new(spec);
        let first = active
            .sample(Duration::from_millis(0))
            .expect("paused still emits the hold sample");
        assert_eq!(first.progress, 0.0);
        assert!(!first.finished);
        assert!(active.sample(Duration::from_millis(50)).is_none());
    }

    #[test]
    fn zero_iteration_or_duration_is_invalid() {
        assert!(
            !AnimationSpec {
                iteration_count: AnimationIteration::Count(0),
                direction: AnimationDirection::Normal,
                fill_mode: AnimationFillMode::None,
                play_state: AnimationPlayState::Running,
                ..spec(0, 100)
            }
            .is_valid()
        );
        assert!(
            !AnimationSpec {
                duration: Duration::ZERO,
                iteration_count: AnimationIteration::ONCE,
                direction: AnimationDirection::Normal,
                fill_mode: AnimationFillMode::None,
                play_state: AnimationPlayState::Running,
                ..spec(0, 100)
            }
            .is_valid()
        );
    }

    #[test]
    fn runtime_sidebar_and_workspace_motion_stay_on_owned_clocks() {
        assert_eq!(
            crate::SidebarSectionState::animation_duration(),
            nana_ui_core::motion::SIDEBAR_COLLAPSE
        );
        assert_eq!(
            nana_ui_core::WORKSPACE_REGION_TRANSITION_DURATION,
            nana_ui_core::motion::SIDEBAR_COLLAPSE
        );
    }

    #[test]
    fn component_animation_ids_are_stable_and_split_by_kind() {
        let node_a = StableNodeId::new(7).unwrap();
        let node_b = StableNodeId::new(8).unwrap();
        let skeleton = component_animation_id(component_animation_kinds::SKELETON, node_a);
        assert_eq!(
            skeleton,
            component_animation_id(component_animation_kinds::SKELETON, node_a),
            "re-deriving the same pair must address the same timeline"
        );
        assert_ne!(
            skeleton,
            component_animation_id(component_animation_kinds::SKELETON, node_b)
        );
        let other_kind = component_animation_id(component_animation_kinds::SKELETON + 1, node_a);
        assert_ne!(
            skeleton, other_kind,
            "another component kind on the same node needs its own namespace"
        );
        let skeleton = skeleton.unwrap();
        assert_ne!(
            Some(skeleton),
            AnimationId::new(node_a.get()),
            "hashed IDs must not land in the raw node-ID namespace"
        );
    }
}
