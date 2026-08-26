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

    /// Snap onto the step grid, then clamp.
    pub fn snap(self, value: f64) -> f64 {
        let step = self.effective_step();
        let origin = self.minimum.filter(|minimum| minimum.is_finite());
        let base = origin.unwrap_or(0.0);
        let value = if value.is_finite() { value } else { base };
        let snapped = base + ((value - base) / step).round() * step;
        self.clamp(round_to(snapped, self.precision))
    }

    /// Move `value` by `steps` grid positions. Zero steps still snaps, so an
    /// out-of-grid value settles the first time the control is nudged.
    pub fn step_by(self, value: f64, steps: i32) -> f64 {
        let step = self.effective_step();
        let base = if value.is_finite() {
            value
        } else {
            self.minimum.unwrap_or(0.0)
        };
        self.snap(base + f64::from(steps) * step)
    }

    /// Whether stepping up can still change the value.
    pub fn can_increment(self, value: f64) -> bool {
        self.maximum
            .filter(|maximum| maximum.is_finite())
            .is_none_or(|maximum| self.clamp(value) < maximum)
    }

    /// Whether stepping down can still change the value.
    pub fn can_decrement(self, value: f64) -> bool {
        self.minimum
            .filter(|minimum| minimum.is_finite())
            .is_none_or(|minimum| self.clamp(value) > minimum)
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
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        trimmed
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(|value| self.snap(value))
    }
}

fn round_to(value: f64, precision: u8) -> f64 {
    let scale = 10f64.powi(i32::from(precision));
    (value * scale).round() / scale
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
