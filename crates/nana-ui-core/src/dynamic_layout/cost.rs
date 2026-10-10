//! What adjusting a layout costs in how it looks.

use serde::{Deserialize, Serialize};

use super::LayoutUnits;

/// The visual price of one unit of an adjustment, or a hard no.
///
/// It says how much worse a result looks -- a gap closed up, a padding
/// pinched, punctuation compressed -- never how long it takes to compute;
/// [`super::ExecutionClass`] says that. `Forbidden` is its own value, not a
/// large number: it sorts after every finite cost and adding to it stays
/// forbidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LayoutCost {
    Finite(u32),
    Forbidden,
}

impl Default for LayoutCost {
    fn default() -> Self {
        Self::ZERO
    }
}

impl LayoutCost {
    pub const ZERO: Self = Self::Finite(0);

    pub const fn is_forbidden(self) -> bool {
        matches!(self, Self::Forbidden)
    }

    pub const fn finite(self) -> Option<u32> {
        match self {
            Self::Finite(cost) => Some(cost),
            Self::Forbidden => None,
        }
    }

    /// Finite plus finite saturates at the largest finite cost; anything
    /// with `Forbidden` is forbidden.
    pub const fn saturating_add(self, other: Self) -> Self {
        match (self, other) {
            (Self::Finite(a), Self::Finite(b)) => Self::Finite(a.saturating_add(b)),
            _ => Self::Forbidden,
        }
    }
}

/// The total price of a result: the sum of each segment's marginal cost
/// times the units taken from it, and whether anything forbidden was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TotalCost {
    pub forbidden: bool,
    pub finite: u64,
}

impl TotalCost {
    pub const ZERO: Self = Self {
        forbidden: false,
        finite: 0,
    };

    /// Taking `units` at `cost` each.
    pub fn add(&mut self, cost: LayoutCost, units: LayoutUnits) {
        if !units.is_positive() {
            return;
        }
        match cost {
            LayoutCost::Finite(cost) => {
                self.finite = self.finite.saturating_add(u64::from(cost) * units.0 as u64);
            }
            LayoutCost::Forbidden => self.forbidden = true,
        }
    }

    pub fn plus(mut self, other: Self) -> Self {
        self.forbidden |= other.forbidden;
        self.finite = self.finite.saturating_add(other.finite);
        self
    }
}

/// The default visual prices, per pixel taken. Cheapest first: closing up
/// the gaps between items reads as the same layout tighter; pinching a
/// control's padding reads as a denser control; closing the gap inside one
/// (an icon against its label) starts to read as crowded.
pub mod costs {
    use super::LayoutCost;

    pub const PLACEMENT_GAP: LayoutCost = LayoutCost::Finite(10);
    pub const PADDING: LayoutCost = LayoutCost::Finite(20);
    pub const CONTENT_GAP: LayoutCost = LayoutCost::Finite(30);
    /// CJK punctuation compressed toward its ink (Issue #211).
    pub const PUNCTUATION: LayoutCost = LayoutCost::Finite(15);
    /// The space automatically put between ideographs and Latin text.
    pub const AUTOSPACE: LayoutCost = LayoutCost::Finite(12);
    /// The gap an inline object keeps against its neighbours.
    pub const EDGE_GAP: LayoutCost = LayoutCost::Finite(18);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_sorts_last_and_stays_forbidden() {
        assert!(LayoutCost::Finite(u32::MAX) < LayoutCost::Forbidden);
        assert_eq!(
            LayoutCost::Finite(u32::MAX).saturating_add(LayoutCost::Finite(1)),
            LayoutCost::Finite(u32::MAX)
        );
        assert!(
            LayoutCost::Finite(1)
                .saturating_add(LayoutCost::Forbidden)
                .is_forbidden()
        );
        let mut total = TotalCost::ZERO;
        total.add(LayoutCost::Finite(3), LayoutUnits(4));
        total.add(LayoutCost::Forbidden, LayoutUnits::ZERO);
        assert_eq!(
            total,
            TotalCost {
                forbidden: false,
                finite: 12
            }
        );
        total.add(LayoutCost::Forbidden, LayoutUnits(1));
        assert!(total.forbidden);
        assert!(
            TotalCost {
                forbidden: false,
                finite: u64::MAX
            } < total
        );
    }
}
