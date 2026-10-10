//! One piece of capacity a box can give up.

use serde::{Deserialize, Serialize};

use super::{LayoutCost, LayoutUnits};

/// How much work taking a segment needs, cheapest first. A solver opens a
/// class only when the cheaper ones cannot cover the deficit; within a class
/// it takes the cheapest [`LayoutCost`] first. The class never changes what
/// a result costs, only whether a solver looks at it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[repr(u8)]
pub enum ExecutionClass {
    /// Only where things are placed moves: a gap between items.
    #[default]
    PlacementOnly = 0,
    /// One box resolves a smaller extent inside itself: its padding, the
    /// gap between its parts. Its content keeps its size.
    LocalBox = 1,
    /// A box lays its content out again: text breaks differently.
    LocalReflow = 2,
    /// A box swaps to another arrangement. Explicit opt-in only.
    Structural = 3,
}

impl ExecutionClass {
    pub const ALL: [Self; 4] = [
        Self::PlacementOnly,
        Self::LocalBox,
        Self::LocalReflow,
        Self::Structural,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// What a segment gives up, for diagnostics and for a context to ignore
/// what it does not understand.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[non_exhaustive]
pub enum AdjustmentKind {
    /// The gap between items in a line.
    #[default]
    Gap,
    /// A box's own padding.
    Padding,
    /// A gap inside one box: an icon against its label.
    ContentGap,
    /// Capacity merged from several segments or several children.
    Aggregate,
    /// Text letter or word spacing.
    TextSpacing,
    /// CJK punctuation compressed toward its ink.
    Punctuation,
    /// Space a justified line stretched.
    Justification,
    /// The space put between ideographs and Latin text.
    Autospace,
    /// The gap an inline object keeps against its neighbours.
    EdgeGap,
    /// Capacity inside an inline object.
    ObjectInternal,
    /// A discrete arrangement switch.
    Discrete,
}

/// Capacity a box can give up at one marginal cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct AdjustmentSegment {
    pub capacity: LayoutUnits,
    pub marginal_cost: LayoutCost,
    pub kind: AdjustmentKind,
    pub execution_class: ExecutionClass,
    /// How many original segments were merged into this one.
    pub sources: u16,
    /// A summary its participant can refine on request
    /// ([`super::ParticipantSource`]).
    pub coarse: bool,
}

impl AdjustmentSegment {
    pub fn new(
        capacity: LayoutUnits,
        marginal_cost: LayoutCost,
        kind: AdjustmentKind,
        execution_class: ExecutionClass,
    ) -> Self {
        Self {
            capacity,
            marginal_cost,
            kind,
            execution_class,
            sources: 1,
            coarse: false,
        }
    }

    /// The order segments are kept and consumed in: class, then cost, then
    /// kind.
    pub(super) fn order_key(&self) -> (ExecutionClass, LayoutCost, AdjustmentKind) {
        (self.execution_class, self.marginal_cost, self.kind)
    }
}

/// The most segments one envelope holds. A box with more opportunities than
/// this merges its dearest ones (see [`super::EnvelopeShape::normalized`]):
/// what a parent reads never grows with the size of a child's subtree.
pub const MAX_ENVELOPE_SEGMENTS: usize = 8;

/// Up to [`MAX_ENVELOPE_SEGMENTS`] segments, inline: no heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct SegmentList {
    items: [AdjustmentSegment; MAX_ENVELOPE_SEGMENTS],
    len: u8,
}

impl SegmentList {
    pub(super) const EMPTY: Self = Self {
        items: [AdjustmentSegment {
            capacity: LayoutUnits::ZERO,
            marginal_cost: LayoutCost::ZERO,
            kind: AdjustmentKind::Gap,
            execution_class: ExecutionClass::PlacementOnly,
            sources: 0,
            coarse: false,
        }; MAX_ENVELOPE_SEGMENTS],
        len: 0,
    };

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_slice(&self) -> &[AdjustmentSegment] {
        &self.items[..self.len()]
    }

    /// Whether `segment` fit.
    pub(super) fn try_push(&mut self, segment: AdjustmentSegment) -> bool {
        if self.len() == MAX_ENVELOPE_SEGMENTS {
            return false;
        }
        self.items[self.len()] = segment;
        self.len += 1;
        true
    }

    pub fn iter(&self) -> std::slice::Iter<'_, AdjustmentSegment> {
        self.as_slice().iter()
    }
}
