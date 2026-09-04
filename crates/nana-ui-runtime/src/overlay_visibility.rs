//! Time-based overlay show/hide policy. Hosts supply clocks and lock flags;
//! the policy does not own windows, media, or pointer routing.

use std::time::{Duration, Instant};

/// Idle auto-hide after this long while the overlay is not locked.
pub const OVERLAY_IDLE: Duration = Duration::from_secs(3);

/// Pointer / menu / drag locks that keep chrome visible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverlayLocks {
    pub focused: bool,
    pub dragging: bool,
    pub menu_open: bool,
}

impl OverlayLocks {
    pub fn held(self) -> bool {
        self.focused || self.dragging || self.menu_open
    }
}

/// Timing knobs. Zero hover dwell reveals immediately; zero startup skips the
/// initial forced-visible window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayVisibilityConfig {
    pub idle: Duration,
    pub hover_dwell: Duration,
    pub startup: Duration,
}

impl Default for OverlayVisibilityConfig {
    fn default() -> Self {
        Self {
            idle: OVERLAY_IDLE,
            hover_dwell: Duration::ZERO,
            startup: Duration::ZERO,
        }
    }
}

/// Auto-hide machine for media chrome and stage HUDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayVisibility {
    config: OverlayVisibilityConfig,
    visible: bool,
    active: bool,
    held: bool,
    deadline: Option<Instant>,
    startup_until: Option<Instant>,
    hot_since: Option<Instant>,
}

impl Default for OverlayVisibility {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl OverlayVisibility {
    pub fn new(now: Instant) -> Self {
        Self::with_config(now, OverlayVisibilityConfig::default())
    }

    pub fn with_config(now: Instant, config: OverlayVisibilityConfig) -> Self {
        let startup_until = (!config.startup.is_zero()).then(|| now + config.startup);
        Self {
            config,
            visible: true,
            active: false,
            held: false,
            deadline: None,
            startup_until,
            hot_since: None,
        }
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn wakeup(&self) -> Option<Instant> {
        let mut next = self.deadline;
        if let Some(startup) = self.startup_until {
            next = Some(next.map_or(startup, |deadline| deadline.min(startup)));
        }
        if let Some(hot) = self.hot_since {
            let dwell = hot + self.config.hover_dwell;
            next = Some(next.map_or(dwell, |deadline| deadline.min(dwell)));
        }
        next
    }

    /// `active` means the surface is in a mode that may hide chrome (playing
    /// media, idle stage). Loading / paused / empty pass false and stay up.
    pub fn synchronize(&mut self, now: Instant, active: bool, locks: OverlayLocks) -> bool {
        let old = self.visible;
        let held = locks.held();
        if self.startup_until.is_some_and(|until| now >= until) {
            self.startup_until = None;
        }
        if !active || held || self.startup_until.is_some() {
            self.visible = true;
            self.deadline = None;
        } else if !self.active || self.held || (self.visible && self.deadline.is_none()) {
            self.deadline = Some(now + self.config.idle);
        }
        self.active = active;
        self.held = held;
        old != self.visible
    }

    /// Immediate reveal (pointer / keyboard activity).
    pub fn activity(&mut self, now: Instant) -> bool {
        let changed = !self.visible;
        self.visible = true;
        self.deadline = (self.active && !self.held).then_some(now + self.config.idle);
        changed
    }

    /// Hover dwell. `hot` true starts the dwell clock; false cancels it.
    pub fn set_pointer_inside(&mut self, now: Instant, hot: bool) -> bool {
        if hot {
            if self.config.hover_dwell.is_zero() {
                self.hot_since = None;
                return self.activity(now);
            }
            if self.hot_since.is_none() {
                self.hot_since = Some(now);
            }
            if now.saturating_duration_since(self.hot_since.unwrap()) >= self.config.hover_dwell {
                self.hot_since = None;
                return self.activity(now);
            }
            false
        } else {
            self.hot_since = None;
            false
        }
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        if self.startup_until.is_some_and(|until| now >= until) {
            self.startup_until = None;
        }
        if let Some(hot) = self.hot_since {
            if now.saturating_duration_since(hot) >= self.config.hover_dwell {
                self.hot_since = None;
                return self.activity(now);
            }
        }
        if self.deadline.is_some_and(|deadline| deadline <= now) {
            self.deadline = None;
            let changed = self.visible;
            self.visible = false;
            return changed;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn stays_visible_when_inactive_or_locked() {
        let mut vis = OverlayVisibility::new(t0());
        let now = t0();
        assert!(!vis.synchronize(now, false, OverlayLocks::default()));
        assert!(vis.visible());
        vis.synchronize(now, true, OverlayLocks::default());
        vis.tick(now + OVERLAY_IDLE);
        assert!(!vis.visible());
        vis.synchronize(
            now + OVERLAY_IDLE,
            true,
            OverlayLocks {
                dragging: true,
                ..OverlayLocks::default()
            },
        );
        assert!(vis.visible());
    }

    #[test]
    fn activity_resets_idle_deadline() {
        let mut vis = OverlayVisibility::new(t0());
        let now = t0();
        vis.synchronize(now, true, OverlayLocks::default());
        vis.tick(now + Duration::from_secs(2));
        assert!(vis.visible());
        vis.activity(now + Duration::from_secs(2));
        vis.tick(now + Duration::from_secs(4));
        assert!(vis.visible());
        vis.tick(now + Duration::from_secs(2) + OVERLAY_IDLE);
        assert!(!vis.visible());
    }

    #[test]
    fn hover_dwell_delays_reveal() {
        let now = t0();
        let mut vis = OverlayVisibility::with_config(
            now,
            OverlayVisibilityConfig {
                idle: OVERLAY_IDLE,
                hover_dwell: Duration::from_millis(150),
                startup: Duration::ZERO,
            },
        );
        vis.synchronize(now, true, OverlayLocks::default());
        vis.tick(now + OVERLAY_IDLE);
        assert!(!vis.visible());
        assert!(!vis.set_pointer_inside(now + OVERLAY_IDLE, true));
        assert!(!vis.visible());
        vis.tick(now + OVERLAY_IDLE + Duration::from_millis(150));
        assert!(vis.visible());
    }
}
