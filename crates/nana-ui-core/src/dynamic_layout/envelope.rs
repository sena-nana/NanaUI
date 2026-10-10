//! What a box can give up, summarized for its parent.

use super::{
    AdjustmentKind, AdjustmentSegment, ExecutionClass, LayoutUnits, MAX_ENVELOPE_SEGMENTS,
    SegmentList, TotalCost,
};

/// A box's elasticity, independent of the size it is offered: segments in
/// consumption order and the capacity each class holds. A parent reads only
/// this, never the box's children. It carries no preferred size of its own:
/// the box's intrinsic metrics are that authority, and
/// [`AdjustmentEnvelope`] pairs the two when a solver asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EnvelopeShape {
    segments: SegmentList,
    class_capacity: [LayoutUnits; 4],
    /// What taking all of each class costs: the sum of cost times capacity.
    class_cost: [TotalCost; 4],
    /// Bumped only when the segments themselves change.
    pub generation: u64,
}

impl EnvelopeShape {
    /// No elasticity.
    pub const RIGID: Self = Self {
        segments: SegmentList::EMPTY,
        class_capacity: [LayoutUnits::ZERO; 4],
        class_cost: [TotalCost::ZERO; 4],
        generation: 0,
    };

    /// A shape from any number of segments, normalized.
    pub fn from_segments(segments: impl IntoIterator<Item = AdjustmentSegment>) -> Self {
        let mut scratch: Vec<AdjustmentSegment> = segments.into_iter().collect();
        Self::normalized(&mut scratch)
    }

    /// Sort into consumption order; merge neighbours of one class and cost;
    /// drop what cannot be taken (no capacity, or forbidden); past
    /// [`MAX_ENVELOPE_SEGMENTS`], merge the dearest two of a class, keeping
    /// the dearer cost, so a merged shape never prices capacity below what
    /// it cost.
    pub fn normalized(segments: &mut Vec<AdjustmentSegment>) -> Self {
        segments.retain(|segment| {
            segment.capacity.is_positive() && !segment.marginal_cost.is_forbidden()
        });
        segments.sort_by_key(AdjustmentSegment::order_key);
        let mut merged: Vec<AdjustmentSegment> = Vec::with_capacity(segments.len());
        for segment in segments.drain(..) {
            match merged.last_mut() {
                Some(last)
                    if last.execution_class == segment.execution_class
                        && last.marginal_cost == segment.marginal_cost =>
                {
                    merge_into(last, &segment);
                }
                _ => merged.push(segment),
            }
        }
        while merged.len() > MAX_ENVELOPE_SEGMENTS {
            // The dearest pair that shares a class; failing that, the last two.
            let at = (1..merged.len())
                .rev()
                .find(|at| merged[*at - 1].execution_class == merged[*at].execution_class)
                .unwrap_or(merged.len() - 1);
            let dearer = merged.remove(at);
            let into = &mut merged[at - 1];
            into.execution_class = into.execution_class.max(dearer.execution_class);
            merge_into(into, &dearer);
        }
        let mut shape = Self::RIGID;
        for segment in merged {
            let class = segment.execution_class.index();
            shape.class_capacity[class] += segment.capacity;
            shape.class_cost[class].add(segment.marginal_cost, segment.capacity);
            let pushed = shape.segments.try_push(segment);
            debug_assert!(pushed);
        }
        shape
    }

    pub fn segments(&self) -> &[AdjustmentSegment] {
        self.segments.as_slice()
    }

    pub fn is_rigid(&self) -> bool {
        self.segments.is_empty()
    }

    /// Capacity in one class.
    pub fn class_capacity(&self, class: ExecutionClass) -> LayoutUnits {
        self.class_capacity[class.index()]
    }

    /// What taking all of one class costs.
    pub fn class_cost(&self, class: ExecutionClass) -> TotalCost {
        self.class_cost[class.index()]
    }

    /// Capacity in every class up to and including `class`.
    pub fn capacity_through(&self, class: ExecutionClass) -> LayoutUnits {
        self.class_capacity[..=class.index()].iter().copied().sum()
    }

    pub fn total_capacity(&self) -> LayoutUnits {
        self.class_capacity.iter().copied().sum()
    }

    /// Whether two shapes hold the same segments, whatever their generations.
    pub fn same_facts(&self, other: &Self) -> bool {
        self.segments == other.segments
    }

    /// The same capacity as one segment per class, priced at its dearest:
    /// for a context that only wants how far a box can shrink (a grid track).
    pub fn bounds_only(&self) -> Self {
        Self::from_segments(ExecutionClass::ALL.into_iter().filter_map(|class| {
            let capacity = self.class_capacity(class);
            let cost = self
                .segments()
                .iter()
                .filter(|segment| segment.execution_class == class)
                .map(|segment| segment.marginal_cost)
                .max()?;
            Some(AdjustmentSegment::new(
                capacity,
                cost,
                AdjustmentKind::Aggregate,
                class,
            ))
        }))
    }
}

fn merge_into(into: &mut AdjustmentSegment, from: &AdjustmentSegment) {
    into.capacity += from.capacity;
    into.marginal_cost = into.marginal_cost.max(from.marginal_cost);
    if into.kind != from.kind {
        into.kind = AdjustmentKind::Aggregate;
    }
    into.sources = into.sources.saturating_add(from.sources);
    into.coarse |= from.coarse;
}

/// A shape with the sizes it adjusts from, as a solver reads it: preferred,
/// the least it can be, the most.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdjustmentEnvelope {
    pub preferred: LayoutUnits,
    pub minimum: LayoutUnits,
    pub maximum: Option<LayoutUnits>,
    pub shape: EnvelopeShape,
}

impl AdjustmentEnvelope {
    /// `shape` over a preferred size, its minimum the preferred size less
    /// every class `allowed` opens, never below `floor`.
    pub fn new(
        preferred: LayoutUnits,
        floor: LayoutUnits,
        maximum: Option<LayoutUnits>,
        shape: EnvelopeShape,
        allowed: ExecutionClass,
    ) -> Self {
        let minimum = (preferred - shape.capacity_through(allowed)).max(floor);
        Self {
            preferred,
            minimum: minimum.min(preferred),
            maximum,
            shape,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynamic_layout::LayoutCost;

    fn segment(px: f32, cost: u32, class: ExecutionClass) -> AdjustmentSegment {
        AdjustmentSegment::new(
            LayoutUnits::from_px(px),
            LayoutCost::Finite(cost),
            AdjustmentKind::Padding,
            class,
        )
    }

    #[test]
    fn a_shape_sorts_merges_and_drops_what_cannot_be_taken() {
        let shape = EnvelopeShape::from_segments([
            segment(2.0, 20, ExecutionClass::LocalBox),
            segment(1.0, 10, ExecutionClass::PlacementOnly),
            segment(3.0, 20, ExecutionClass::LocalBox),
            segment(0.0, 5, ExecutionClass::PlacementOnly),
            AdjustmentSegment::new(
                LayoutUnits::from_px(9.0),
                LayoutCost::Forbidden,
                AdjustmentKind::Gap,
                ExecutionClass::PlacementOnly,
            ),
        ]);
        assert_eq!(shape.segments().len(), 2);
        assert_eq!(shape.segments()[0].marginal_cost, LayoutCost::Finite(10));
        assert_eq!(shape.segments()[1].capacity, LayoutUnits::from_px(5.0));
        assert_eq!(shape.segments()[1].sources, 2);
        assert_eq!(
            shape.class_capacity(ExecutionClass::LocalBox),
            LayoutUnits::from_px(5.0)
        );
        assert_eq!(shape.total_capacity(), LayoutUnits::from_px(6.0));
    }

    /// A thousand distinct segments still fit in eight, priced no lower than
    /// they were, with no capacity lost.
    #[test]
    fn a_shape_stays_bounded_and_never_cheapens_capacity() {
        let shape = EnvelopeShape::from_segments(
            (0..1000).map(|at| segment(1.0, at, ExecutionClass::ALL[at as usize % 3])),
        );
        assert!(shape.segments().len() <= MAX_ENVELOPE_SEGMENTS);
        assert_eq!(shape.total_capacity(), LayoutUnits::from_px(1000.0));
        let mut cost_of_capacity = 0u64;
        for segment in shape.segments() {
            cost_of_capacity +=
                u64::from(segment.marginal_cost.finite().unwrap()) * segment.capacity.0 as u64;
        }
        let original: u64 = (0..1000u64).map(|cost| cost * 64).sum();
        assert!(cost_of_capacity >= original);
        let ordered = shape
            .segments()
            .windows(2)
            .all(|pair| pair[0].order_key() <= pair[1].order_key());
        assert!(ordered);
    }
}
