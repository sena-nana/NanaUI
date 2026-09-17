//! Shared playback longhands. [`crate::motion::MotionTrack`] and Runtime
//! `AnimationSpec` both use these types so delay / iteration / direction /
//! fill / pause are one contract.

use std::time::Duration;

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
    pub(crate) fn start_progress(self) -> f32 {
        match self {
            Self::Normal | Self::Alternate => 0.0,
            Self::Reverse | Self::AlternateReverse => 1.0,
        }
    }

    pub(crate) fn map_progress(self, iteration_index: u32, linear: f32) -> f32 {
        let reverse = match self {
            Self::Normal => false,
            Self::Reverse => true,
            Self::Alternate => !iteration_index.is_multiple_of(2),
            Self::AlternateReverse => iteration_index.is_multiple_of(2),
        };
        if reverse { 1.0 - linear } else { linear }
    }

    pub(crate) fn end_progress(self, completed_iterations: u32) -> f32 {
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
    pub fn applies_backwards(self) -> bool {
        matches!(self, Self::Backwards | Self::Both)
    }

    pub fn applies_forwards(self) -> bool {
        matches!(self, Self::Forwards | Self::Both)
    }
}

/// CSS `animation-play-state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationPlayState {
    #[default]
    Running,
    Paused,
}

/// Playback longhands shared by Motion IR and Runtime `AnimationSpec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnimationPlayback {
    pub iteration_count: AnimationIteration,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
    /// Absolute clock the track is frozen at while [`Self::play_state`] is
    /// [`AnimationPlayState::Paused`]. Missing means "paused but not rewound":
    /// evaluation uses the caller timestamp, never `timing.start`.
    pub paused_at: Option<Duration>,
}

impl AnimationPlayback {
    pub const fn running(
        iteration_count: AnimationIteration,
        direction: AnimationDirection,
        fill_mode: AnimationFillMode,
    ) -> Self {
        Self {
            iteration_count,
            direction,
            fill_mode,
            play_state: AnimationPlayState::Running,
            paused_at: None,
        }
    }

    /// Same freeze rule as [`crate::motion::MotionTrack::evaluation_clock`].
    /// Paused: `hold_at` → [`Self::paused_at`] → `now`. Never rewinds to start.
    pub fn evaluation_clock(self, now: Duration, hold_at: Option<Duration>) -> Duration {
        if self.play_state == AnimationPlayState::Paused {
            hold_at.or(self.paused_at).unwrap_or(now)
        } else {
            now
        }
    }
}

/// Host-clock progress after delay / iteration / direction / fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedProgress {
    pub linear: f32,
    pub finished: bool,
    /// Whether fill-mode applies an animated value at this timestamp.
    pub applies: bool,
}

/// Absolute-clock timing for one Motion track. Runtime `AnimationSpec` is
/// this struct plus a node target: its `start` already includes CSS delay
/// (`delay` is then `ZERO`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionTiming {
    /// Clock when delay begins counting.
    pub start: Duration,
    /// Hold before the first iteration. Folded into `start` by CSS → Spec.
    pub delay: Duration,
    /// Length of one iteration. Ignored by spring / decay (they use settle).
    pub duration: Duration,
    /// CPU sparse-sample cadence. Compositor evaluation ignores this field.
    pub frame_interval: Duration,
}

impl MotionTiming {
    pub const fn new(start: Duration, duration: Duration, frame_interval: Duration) -> Self {
        Self {
            start,
            delay: Duration::ZERO,
            duration,
            frame_interval,
        }
    }

    pub const fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn effective_start(self) -> Option<Duration> {
        self.start.checked_add(self.delay)
    }

    pub fn end(self, playback: AnimationPlayback) -> Option<Duration> {
        let start = self.effective_start()?;
        match playback.iteration_count {
            AnimationIteration::Infinite => None,
            AnimationIteration::Count(0) => Some(start),
            AnimationIteration::Count(count) => {
                start.checked_add(self.duration.saturating_mul(count))
            }
        }
    }

    /// A finite zero-length run ends at its effective start, like CSS
    /// `transition-duration: 0`; an endless one never ends and is invalid.
    pub fn is_valid(self, playback: AnimationPlayback) -> bool {
        if self.frame_interval.is_zero() {
            return false;
        }
        match playback.iteration_count {
            AnimationIteration::Count(0) => false,
            AnimationIteration::Infinite => !self.duration.is_zero(),
            AnimationIteration::Count(_) => self.end(playback).is_some(),
        }
    }

    /// Linear iteration progress after delay / fill. Spring / decay use
    /// elapsed time instead of this mapping.
    pub fn timed_progress(self, playback: AnimationPlayback, now: Duration) -> TimedProgress {
        let Some(start) = self.effective_start() else {
            return TimedProgress {
                linear: playback.direction.end_progress(0),
                finished: true,
                applies: playback.fill_mode.applies_forwards(),
            };
        };
        if now < start {
            let hold = playback.fill_mode.applies_backwards();
            return TimedProgress {
                linear: if hold {
                    playback.direction.start_progress()
                } else {
                    0.0
                },
                finished: false,
                applies: hold,
            };
        }
        let elapsed = now.saturating_sub(start);
        match playback.iteration_count {
            AnimationIteration::Infinite => {
                let duration = self.duration.as_secs_f32();
                if duration <= 0.0 {
                    return TimedProgress {
                        linear: playback.direction.end_progress(1),
                        finished: false,
                        applies: true,
                    };
                }
                let t = elapsed.as_secs_f32() / duration;
                let iteration_index = t.floor() as u32;
                let linear = (t - iteration_index as f32).clamp(0.0, 1.0);
                TimedProgress {
                    linear: playback.direction.map_progress(iteration_index, linear),
                    finished: false,
                    applies: true,
                }
            }
            AnimationIteration::Count(count) => {
                let Some(end) = self.end(playback) else {
                    return TimedProgress {
                        linear: playback.direction.end_progress(count),
                        finished: true,
                        applies: playback.fill_mode.applies_forwards(),
                    };
                };
                if now > end {
                    let hold = playback.fill_mode.applies_forwards();
                    return TimedProgress {
                        linear: if hold {
                            playback.direction.end_progress(count)
                        } else {
                            0.0
                        },
                        finished: true,
                        applies: hold,
                    };
                }
                if now == end {
                    return TimedProgress {
                        linear: playback.direction.end_progress(count),
                        finished: true,
                        applies: true,
                    };
                }
                let duration = self.duration.as_secs_f32();
                if duration <= 0.0 {
                    return TimedProgress {
                        linear: playback.direction.end_progress(count),
                        finished: true,
                        applies: playback.fill_mode.applies_forwards(),
                    };
                }
                let t = elapsed.as_secs_f32() / duration;
                let iteration_index = (t.floor() as u32).min(count.saturating_sub(1));
                let linear = (t - iteration_index as f32).clamp(0.0, 1.0);
                TimedProgress {
                    linear: playback.direction.map_progress(iteration_index, linear),
                    finished: false,
                    applies: true,
                }
            }
        }
    }
}
