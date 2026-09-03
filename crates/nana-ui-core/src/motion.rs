//! Backend-neutral motion contracts: easing curves and shared duration tokens.

use std::time::Duration;

/// Timing function for one animation timeline.
///
/// [`Easing::CubicBezier`] follows CSS `cubic-bezier(x1, y1, x2, y2)`
/// semantics with the endpoints pinned to `(0, 0)` and `(1, 1)`. Control-point
/// x coordinates must stay in `0.0..=1.0`; every constructor in this
/// repository guarantees that, which keeps `x(t)` monotonic so sampling can
/// solve for the curve parameter directly.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Easing {
    #[default]
    Linear,
    EaseOutCubic,
    EaseInOutCubic,
    CubicBezier([f32; 4]),
}

impl Easing {
    /// LiliaUI menu pop-in curve: `cubic-bezier(0.2, 0.8, 0.2, 1)`.
    pub const MENU_POP: Self = Self::CubicBezier([0.2, 0.8, 0.2, 1.0]);

    /// Maps linear `progress` in `0.0..=1.0` onto the eased curve. For
    /// [`Easing::CubicBezier`], progress at or outside the unit range returns
    /// the pinned endpoint values.
    pub fn sample(self, progress: f32) -> f32 {
        match self {
            Self::Linear => progress,
            Self::EaseOutCubic => 1.0 - (1.0 - progress).powi(3),
            Self::EaseInOutCubic if progress < 0.5 => 4.0 * progress.powi(3),
            Self::EaseInOutCubic => 1.0 - (-2.0 * progress + 2.0).powi(3) / 2.0,
            Self::CubicBezier(points) => sample_cubic_bezier(points, progress),
        }
    }
}

/// Shared motion durations aligned with the LiliaUI motion spec. Surfaces wire
/// these in per interaction; the constants only centralize the values.
pub const HOVER_COLOR: Duration = Duration::from_millis(120);
/// Overlay fade-in/out duration.
pub const OVERLAY_FADE: Duration = Duration::from_millis(140);
/// Menu opacity transition duration.
pub const MENU_OPACITY: Duration = Duration::from_millis(160);
/// Menu pop-in scale/translate duration.
pub const MENU_POP: Duration = Duration::from_millis(180);
/// Sidebar collapse/expand duration.
pub const SIDEBAR_COLLAPSE: Duration = Duration::from_millis(260);
/// Skeleton pulse cycle duration.
pub const SKELETON_PULSE: Duration = Duration::from_millis(1400);

/// Bernstein-form cubic bezier over one axis, endpoints pinned to 0 and 1.
fn bezier_axis(t: f32, p1: f32, p2: f32) -> f32 {
    3.0 * (1.0 - t).powi(2) * t * p1 + 3.0 * (1.0 - t) * t.powi(2) * p2 + t.powi(3)
}

/// Solves `x(t) = progress` by bisection (x monotonic for control x in
/// `0.0..=1.0`) and returns `y(t)`.
fn sample_cubic_bezier([x1, y1, x2, y2]: [f32; 4], progress: f32) -> f32 {
    if progress <= 0.0 {
        return 0.0;
    }
    if progress >= 1.0 {
        return 1.0;
    }
    let mut low: f32 = 0.0;
    let mut high: f32 = 1.0;
    for _ in 0..64 {
        let mid = 0.5 * (low + high);
        if bezier_axis(mid, x1, x2) < progress {
            low = mid;
        } else {
            high = mid;
        }
    }
    bezier_axis(0.5 * (low + high), y1, y2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cubic_bezier_endpoints_are_exact() {
        assert_eq!(Easing::MENU_POP.sample(0.0), 0.0);
        assert_eq!(Easing::MENU_POP.sample(1.0), 1.0);
        assert_eq!(Easing::CubicBezier([0.0, 0.0, 1.0, 1.0]).sample(0.0), 0.0);
        assert_eq!(Easing::CubicBezier([0.0, 0.0, 1.0, 1.0]).sample(1.0), 1.0);
    }

    #[test]
    fn menu_pop_curve_is_monotonically_nondecreasing() {
        let mut previous = 0.0;
        for step in 0..=64 {
            let progress = step as f32 / 64.0;
            let value = Easing::MENU_POP.sample(progress);
            assert!(
                value >= previous,
                "MENU_POP regressed at progress {progress}: {value} < {previous}"
            );
            previous = value;
        }
    }

    #[test]
    fn menu_pop_midpoint_matches_reference_value() {
        // cubic-bezier(0.2, 0.8, 0.2, 1) at progress 0.5: solving x(t) = 0.5
        // gives t ~= 0.72446, so y(t) ~= 0.94608 (independent reference).
        let value = Easing::MENU_POP.sample(0.5);
        assert!(
            (value - 0.946_08).abs() < 0.01,
            "MENU_POP midpoint drifted: {value}"
        );
    }

    #[test]
    fn ease_in_out_cubic_keeps_quarter_point_values() {
        assert_eq!(Easing::EaseInOutCubic.sample(0.25), 0.0625);
        assert_eq!(Easing::EaseInOutCubic.sample(0.75), 0.9375);
    }
}
