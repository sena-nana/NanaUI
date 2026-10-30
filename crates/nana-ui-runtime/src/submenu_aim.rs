//! The pointer heading for an open submenu (the "safe triangle"). While the
//! pointer stays inside the triangle from where it left the submenu's row to
//! the submenu's near edge, the rows it crosses on the way do not take the
//! highlight. Resting inside the triangle ends the aim after a grace period,
//! so the row under a pointer that stopped short still takes over.
//!
//! Any surface that cascades menus to the side keeps one of these per open
//! submenu: [`crate::AppMenuBar`] today. A menu that drills in place, like
//! [`crate::ContextMenu`], has no submenu beside it to aim for.

use std::time::Duration;

use crate::LayoutBox;

/// How long a pointer may rest inside the triangle before the row under it
/// takes over.
pub(crate) const AIM_GRACE: Duration = Duration::from_millis(300);
/// The triangle reaches this far, px, past the submenu's top and bottom and
/// behind where the pointer left, so a hand that wobbles stays inside.
pub(crate) const AIM_TOLERANCE: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SubmenuAim {
    apex: (f32, f32),
    target: LayoutBox,
    /// The nearest the pointer has come to the target's near edge, px.
    closest: f32,
    until: Duration,
}

impl SubmenuAim {
    /// The pointer left the row that opened `target` at `from`.
    pub(crate) fn begin(from: (f32, f32), target: LayoutBox, now: Duration) -> Self {
        let mut aim = Self {
            apex: from,
            target,
            closest: f32::INFINITY,
            until: now + AIM_GRACE,
        };
        aim.closest = aim.distance(from);
        // behind where the pointer left, away from the target
        aim.apex.0 += if aim.edge() >= from.0 {
            -AIM_TOLERANCE
        } else {
            AIM_TOLERANCE
        };
        aim
    }

    /// The pointer moved to `at`: answers whether it is still heading for
    /// the target. A move that brings it nearer than ever re-arms the grace
    /// period, so a slow, steady approach is not cut short.
    pub(crate) fn track(&mut self, at: (f32, f32), now: Duration) -> bool {
        if now >= self.until || !self.contains(at) {
            return false;
        }
        let distance = self.distance(at);
        if distance < self.closest {
            self.closest = distance;
            self.until = now + AIM_GRACE;
        }
        true
    }

    /// When a pointer that rests from now on stops aiming.
    pub(crate) fn until(&self) -> Duration {
        self.until
    }

    /// The target's side facing the apex.
    fn edge(&self) -> f32 {
        if self.target.x >= self.apex.0 {
            self.target.x
        } else {
            self.target.x + self.target.width
        }
    }

    fn distance(&self, at: (f32, f32)) -> f32 {
        (self.edge() - at.0).abs()
    }

    fn contains(&self, at: (f32, f32)) -> bool {
        let edge = self.edge();
        let a = self.apex;
        let b = (edge, self.target.y - AIM_TOLERANCE);
        let c = (edge, self.target.y + self.target.height + AIM_TOLERANCE);
        let side = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| {
            (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
        };
        let (d1, d2, d3) = (side(a, b, at), side(b, c, at), side(c, a, at));
        let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(negative && positive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: LayoutBox = LayoutBox {
        x: 200.0,
        y: 40.0,
        width: 100.0,
        height: 120.0,
    };

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn heading_for_the_submenu_stays_inside_the_triangle() {
        let mut aim = SubmenuAim::begin((100.0, 50.0), TARGET, ms(0));
        assert!(aim.track((150.0, 70.0), ms(10)));
        assert!(
            !aim.track((110.0, 90.0), ms(20)),
            "falling away down the rows"
        );
    }

    #[test]
    fn a_wobble_where_the_pointer_left_stays_inside() {
        let mut aim = SubmenuAim::begin((100.0, 50.0), TARGET, ms(0));
        assert!(aim.track((100.0, 51.5), ms(8)), "straight down a hair");
        assert!(aim.track((99.0, 50.0), ms(16)), "back a pixel");
    }

    #[test]
    fn the_triangle_reaches_past_the_submenu_corners() {
        let mut aim = SubmenuAim::begin((100.0, 50.0), TARGET, ms(0));
        assert!(aim.track((199.0, 162.0), ms(10)), "past the bottom");
        let mut leftward = SubmenuAim::begin((400.0, 50.0), TARGET, ms(0));
        assert!(
            leftward.track((302.0, 38.0), ms(10)),
            "a submenu to the left"
        );
    }

    #[test]
    fn resting_runs_out_and_progress_rearms() {
        let mut aim = SubmenuAim::begin((100.0, 50.0), TARGET, ms(0));
        assert_eq!(aim.until(), AIM_GRACE);
        assert!(aim.track((120.0, 55.0), ms(200)));
        assert_eq!(aim.until(), ms(200) + AIM_GRACE, "nearer re-arms");
        assert!(aim.track((119.0, 55.0), ms(400)));
        assert_eq!(aim.until(), ms(200) + AIM_GRACE, "a step back does not");
        assert!(
            !aim.track((121.0, 55.0), ms(500)),
            "too late, inside or not"
        );
    }
}
