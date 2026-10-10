//! Fixed-point layout lengths for the solver.

use serde::{Deserialize, Serialize};

/// A length in 1/64 px. Solving in integers makes every sum exact and
/// associative, so an answer does not depend on the order it was added in.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct LayoutUnits(pub i32);

impl LayoutUnits {
    pub const ZERO: Self = Self(0);
    /// Units per pixel.
    pub const PER_PX: i32 = 64;

    /// The nearest unit to `px`; anything not finite is zero.
    pub fn from_px(px: f32) -> Self {
        if !px.is_finite() {
            return Self::ZERO;
        }
        let units = (px * Self::PER_PX as f32).round();
        Self(units.clamp(i32::MIN as f32, i32::MAX as f32) as i32)
    }

    /// Exact in `f32` for any length a layout holds.
    pub fn to_px(self) -> f32 {
        self.0 as f32 / Self::PER_PX as f32
    }

    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// Never negative.
    pub const fn clamp_non_negative(self) -> Self {
        if self.0 < 0 { Self::ZERO } else { self }
    }
}

impl std::ops::Add for LayoutUnits {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        self.saturating_add(other)
    }
}

impl std::ops::Sub for LayoutUnits {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        self.saturating_sub(other)
    }
}

impl std::ops::AddAssign for LayoutUnits {
    fn add_assign(&mut self, other: Self) {
        *self = self.saturating_add(other);
    }
}

impl std::ops::SubAssign for LayoutUnits {
    fn sub_assign(&mut self, other: Self) {
        *self = self.saturating_sub(other);
    }
}

impl std::iter::Sum for LayoutUnits {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, Self::saturating_add)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_round_trip_pixels_and_reject_what_is_not_finite() {
        assert_eq!(LayoutUnits::from_px(1.5).0, 96);
        assert_eq!(LayoutUnits::from_px(1.5).to_px(), 1.5);
        assert_eq!(LayoutUnits::from_px(f32::NAN), LayoutUnits::ZERO);
        assert_eq!(LayoutUnits::from_px(-0.5).0, -32);
        assert_eq!(LayoutUnits(-5).clamp_non_negative(), LayoutUnits::ZERO);
    }
}
