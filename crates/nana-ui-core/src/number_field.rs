//! Backend-neutral numeric field value rules.
//!
//! A numeric spinner has to agree with itself about three things: what a value
//! is allowed to be, how a keystroke or a stepper press moves it, and how it
//! reads back as text. All three live here so the Runtime control and any host
//! that wants to pre-validate share one answer.

use serde::{Deserialize, Serialize};

/// Bounds, granularity, and display precision of a numeric field.
///
/// An absent bound means unbounded on that side. `step` is also the snapping
/// grid: a committed value is pulled to the nearest multiple of `step` measured
/// from `minimum`, or from zero when there is no minimum.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NumberFieldSpec {
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub step: f64,
    /// Decimal places used when formatting. Parsing accepts any precision.
    pub precision: u8,
}

impl Default for NumberFieldSpec {
    fn default() -> Self {
        Self {
            minimum: None,
            maximum: None,
            step: 1.0,
            precision: 0,
        }
    }
}

impl NumberFieldSpec {
    /// Effective step. Non-finite or non-positive requests fall back to 1.
    pub fn effective_step(self) -> f64 {
        if self.step.is_finite() && self.step > 0.0 {
            self.step
        } else {
            1.0
        }
    }

    /// Pull a value inside the bounds. Non-finite input resolves to the lower
    /// bound, or zero when the field is unbounded below.
    pub fn clamp(self, value: f64) -> f64 {
        let mut value = if value.is_finite() {
            value
        } else {
            self.minimum.unwrap_or(0.0)
        };
        if let Some(minimum) = self.minimum.filter(|minimum| minimum.is_finite()) {
            value = value.max(minimum);
        }
        if let Some(maximum) = self.maximum.filter(|maximum| maximum.is_finite()) {
            value = value.min(maximum);
        }
        value
    }

    /// The step the field moves by: its step, but never finer than one
    /// unit of its precision, which could not display the difference (a step
    /// of 0.05 at one decimal place moves by 0.1).
    pub fn display_step(self) -> f64 {
        let unit = 10f64.powi(-i32::from(self.precision));
        self.effective_step().max(unit)
    }

    /// Snap onto the field's grid, within its bounds.
    ///
    /// The grid starts at the minimum rounded up to the precision (or zero
    /// when unbounded below) and moves by [`Self::display_step`], so every
    /// grid point is a number the field displays exactly and no two points
    /// display alike. A bound off the grid (a maximum of 10.3 on a
    /// whole-number grid) holds at the last point inside it (10). Snapping
    /// is idempotent, and what `format` shows `parse` reads back.
    pub fn snap(self, value: f64) -> f64 {
        let origin = self.grid_origin();
        let value = if value.is_finite() { value } else { origin };
        let mut snapped = self.grid_point(((value - origin) / self.display_step()).round());
        if let Some(maximum) = self.grid_maximum().filter(|maximum| snapped > *maximum) {
            snapped = maximum;
        }
        if self.minimum.is_some_and(f64::is_finite) && snapped < origin {
            snapped = origin;
        }
        // The raw bounds hold for a range narrower than one step, and `+ 0.0`
        // turns the -0.0 a minimum just below zero rounds up to into 0.0,
        // which `format` would otherwise show as "-0.0".
        self.clamp(snapped) + 0.0
    }

    /// Where the grid starts: the minimum rounded up to the precision, so
    /// the first point is one the field displays and lies inside the bound.
    fn grid_origin(self) -> f64 {
        let Some(minimum) = self.minimum.filter(|minimum| minimum.is_finite()) else {
            return 0.0;
        };
        let scale = 10f64.powi(i32::from(self.precision));
        let scaled = minimum * scale;
        if !scaled.is_finite() {
            return minimum;
        }
        // A hair of tolerance keeps a minimum already at the precision (0.3)
        // from rounding up past itself through float noise.
        (scaled - 1e-9).ceil() / scale
    }

    /// Grid point `k`, rounded to the precision to shed float noise.
    fn grid_point(self, k: f64) -> f64 {
        round_to(self.grid_origin() + k * self.display_step(), self.precision)
    }

    /// The last grid point inside the maximum, or `None` when unbounded
    /// above. Rounding shifts a point by less than a step, so the point below
    /// the estimate is the only other candidate.
    fn grid_maximum(self) -> Option<f64> {
        let maximum = self.maximum.filter(|maximum| maximum.is_finite())?;
        let last = ((maximum - self.grid_origin()) / self.display_step() + 1e-9).floor();
        let point = self.grid_point(last);
        Some(if point > maximum {
            self.grid_point(last - 1.0)
        } else {
            point
        })
    }

    /// Move `value` by `steps` grid positions. Zero steps still snaps, so an
    /// out-of-grid value settles the first time the control is nudged.
    pub fn step_by(self, value: f64, steps: i32) -> f64 {
        let base = if value.is_finite() {
            value
        } else {
            self.minimum.unwrap_or(0.0)
        };
        self.snap(base + f64::from(steps) * self.display_step())
    }

    /// Whether stepping up can still change the value.
    pub fn can_increment(self, value: f64) -> bool {
        self.step_by(value, 1) > self.snap(value)
    }

    /// Whether stepping down can still change the value.
    pub fn can_decrement(self, value: f64) -> bool {
        self.step_by(value, -1) < self.snap(value)
    }

    /// Render a value at this field's precision.
    pub fn format(self, value: f64) -> String {
        format!(
            "{:.precision$}",
            self.clamp(value),
            precision = usize::from(self.precision)
        )
    }

    /// Read a draft string. Returns `None` when the text is not a number, so
    /// the caller can keep the last committed value instead of guessing.
    pub fn parse(self, text: &str) -> Option<f64> {
        Self::parse_unsnapped(text).map(|value| self.snap(value))
    }

    /// The number a draft spells, before bounds, precision or the step grid
    /// apply. Blank or non-finite text is `None`.
    pub fn parse_unsnapped(text: &str) -> Option<f64> {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
    }
}

/// Round to `precision` decimals. A value too large to scale is already
/// coarser than any decimal place and is returned as it is.
fn round_to(value: f64, precision: u8) -> f64 {
    let scale = 10f64.powi(i32::from(precision));
    let scaled = value * scale;
    if !scaled.is_finite() {
        return value;
    }
    scaled.round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> NumberFieldSpec {
        NumberFieldSpec {
            minimum: Some(0.0),
            maximum: Some(10.0),
            step: 0.5,
            precision: 1,
        }
    }

    #[test]
    fn clamping_holds_the_declared_bounds() {
        assert_eq!(spec().clamp(-4.0), 0.0);
        assert_eq!(spec().clamp(40.0), 10.0);
        assert_eq!(spec().clamp(4.0), 4.0);
        assert_eq!(spec().clamp(f64::NAN), 0.0);
        assert_eq!(NumberFieldSpec::default().clamp(f64::NEG_INFINITY), 0.0);
    }

    #[test]
    fn snapping_pulls_onto_the_step_grid_from_the_minimum() {
        assert_eq!(spec().snap(1.3), 1.5);
        assert_eq!(spec().snap(1.2), 1.0);
        let offset = NumberFieldSpec {
            minimum: Some(0.2),
            maximum: Some(2.2),
            step: 1.0,
            precision: 1,
        };
        assert_eq!(offset.snap(1.0), 1.2);
        assert_eq!(offset.snap(0.0), 0.2);
    }

    #[test]
    fn stepping_stops_at_the_bounds_instead_of_wrapping() {
        assert_eq!(spec().step_by(9.5, 1), 10.0);
        assert_eq!(spec().step_by(10.0, 1), 10.0);
        assert_eq!(spec().step_by(0.0, -1), 0.0);
        assert_eq!(spec().step_by(0.5, -1), 0.0);
        assert_eq!(spec().step_by(4.0, 4), 6.0);
    }

    #[test]
    fn an_off_grid_maximum_holds_at_the_last_grid_point() {
        let off_grid = NumberFieldSpec {
            minimum: Some(0.0),
            maximum: Some(10.3),
            step: 1.0,
            precision: 0,
        };
        assert_eq!(off_grid.snap(11.0), 10.0);
        assert_eq!(off_grid.snap(10.3), 10.0);
        for value in [-3.0, 0.4, 9.6, 10.2, 10.3, 11.0, 400.0] {
            let once = off_grid.snap(value);
            assert_eq!(off_grid.snap(once), once, "{value}");
        }
        assert_eq!(off_grid.step_by(10.0, 1), 10.0);
        assert!(!off_grid.can_increment(10.0));
        assert!(off_grid.can_increment(9.0));
        // A maximum on the grid keeps its last point despite float division.
        let tenths = NumberFieldSpec {
            minimum: Some(0.0),
            maximum: Some(0.3),
            step: 0.1,
            precision: 1,
        };
        assert_eq!(tenths.snap(0.3), 0.3);
        assert_eq!(tenths.snap(0.9), 0.3);
    }

    #[test]
    fn a_minimum_the_precision_cannot_display_still_snaps_idempotently() {
        for minimum in [0.05, -0.25] {
            let spec = NumberFieldSpec {
                minimum: Some(minimum),
                maximum: Some(2.0),
                step: 0.1,
                precision: 1,
            };
            for tenth in -60..30 {
                let once = spec.snap(f64::from(tenth) / 10.0);
                assert_eq!(spec.snap(once), once, "{minimum}: {tenth}");
                // What the field shows reads back as what it holds.
                assert_eq!(spec.parse(&spec.format(once)), Some(once), "{minimum}");
            }
        }
    }

    #[test]
    fn snapping_is_idempotent_bounded_and_displayable_across_grids() {
        for minimum in [None, Some(0.0), Some(0.05), Some(-0.25), Some(1.0)] {
            for maximum in [None, Some(10.0), Some(10.3), Some(3.3)] {
                for step in [0.1, 0.25, 0.5, 1.0, 3.0] {
                    for precision in [0, 1, 2] {
                        let spec = NumberFieldSpec {
                            minimum,
                            maximum,
                            step,
                            precision,
                        };
                        for index in -60..60 {
                            let value = f64::from(index) * 0.37;
                            let once = spec.snap(value);
                            let case = format!("{spec:?} {value}");
                            assert_eq!(spec.snap(once), once, "{case}");
                            assert_eq!(spec.clamp(once), once, "{case}");
                            assert_eq!(spec.parse(&spec.format(once)), Some(once), "{case}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn extreme_bounds_snap_promptly_and_stay_finite() {
        for (minimum, maximum, step, precision) in [
            (0.0, f64::MAX, 0.1, 1),
            (f64::MIN, f64::MAX, 1.0, 2),
            (-1e20, 1e20, 0.3, 2),
            (0.0, 1e17, 0.3, 0),
        ] {
            let spec = NumberFieldSpec {
                minimum: Some(minimum),
                maximum: Some(maximum),
                step,
                precision,
            };
            for value in [0.0, 5.0, -5.0, 1e30, -1e30] {
                let snapped = spec.snap(value);
                assert!(snapped.is_finite(), "{spec:?} {value}");
                assert_eq!(spec.clamp(snapped), snapped, "{spec:?} {value}");
            }
        }
    }

    #[test]
    fn snapping_never_yields_negative_zero() {
        for minimum in [-0.25, -0.35] {
            let spec = NumberFieldSpec {
                minimum: Some(minimum),
                maximum: Some(2.0),
                step: 0.1,
                precision: 1,
            };
            for tenth in -5..20 {
                let snapped = spec.snap(f64::from(tenth) / 10.0);
                assert!(
                    snapped != 0.0 || snapped.is_sign_positive(),
                    "{minimum}: {tenth}"
                );
                assert!(!spec.format(snapped).starts_with("-0.0"), "{minimum}");
            }
        }
    }

    #[test]
    fn every_value_the_display_can_show_inside_the_bounds_is_reachable() {
        let spec = |minimum, maximum, step, precision| NumberFieldSpec {
            minimum: Some(minimum),
            maximum: Some(maximum),
            step,
            precision,
        };
        // A minimum off the precision: the first displayable point stays.
        let half = spec(0.5, 10.0, 1.0, 0);
        assert_eq!((half.snap(1.0), half.snap(0.5)), (1.0, 1.0));
        assert!(half.can_decrement(2.0));
        assert_eq!(half.step_by(2.0, -1), 1.0);
        assert_eq!(spec(-0.25, 10.0, 1.0, 0).snap(0.0), 0.0);
        assert_eq!(spec(0.05, 2.0, 0.1, 1).snap(0.1), 0.1);
        // The top of the range stays reachable.
        assert_eq!(spec(0.33, 5.4, 0.1, 1).snap(5.4), 5.4);
        assert_eq!(spec(0.05, 5.12, 1.0, 0).snap(5.0), 5.0);
    }

    #[test]
    fn a_step_finer_than_the_precision_still_walks_the_whole_range() {
        let spec = NumberFieldSpec {
            minimum: Some(0.0),
            maximum: Some(1.0),
            step: 0.05,
            precision: 1,
        };
        let mut value = 0.0;
        for _ in 0..10 {
            let next = spec.step_by(value, 1);
            assert!(next > value, "stuck at {value}");
            value = next;
        }
        assert_eq!(value, 1.0);
        assert!(!spec.can_increment(1.0));
    }

    #[test]
    fn a_zero_step_falls_back_to_one_rather_than_freezing() {
        let broken = NumberFieldSpec {
            step: 0.0,
            ..NumberFieldSpec::default()
        };
        assert_eq!(broken.effective_step(), 1.0);
        assert_eq!(broken.step_by(3.0, 2), 5.0);
    }

    #[test]
    fn bound_reports_drive_stepper_availability() {
        assert!(spec().can_increment(9.5));
        assert!(!spec().can_increment(10.0));
        assert!(spec().can_decrement(0.5));
        assert!(!spec().can_decrement(0.0));
        let open = NumberFieldSpec::default();
        assert!(open.can_increment(1e9));
        assert!(open.can_decrement(-1e9));
    }

    #[test]
    fn formatting_and_parsing_round_trip_at_the_declared_precision() {
        assert_eq!(spec().format(2.0), "2.0");
        assert_eq!(spec().format(40.0), "10.0");
        assert_eq!(spec().parse("2.5"), Some(2.5));
        assert_eq!(spec().parse("  3 "), Some(3.0));
        assert_eq!(spec().parse("2.3"), Some(2.5));
        assert_eq!(spec().parse("40"), Some(10.0));
        assert_eq!(spec().parse(""), None);
        assert_eq!(spec().parse("-"), None);
        assert_eq!(spec().parse("abc"), None);
        assert_eq!(spec().parse("inf"), None);
    }
}
