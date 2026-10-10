//! A container's envelope from its children's (Issue #209).
//!
//! A container summarizes what its children declare; its parent reads only
//! the summary. Along its line the children's capacities add up; across it
//! the box is as big as its biggest child, so shrinking it shrinks every
//! child taller than the target together. Either way the result is a shape
//! of at most [`super::MAX_ENVELOPE_SEGMENTS`] segments, however many
//! descendants fed it.

use super::{
    AdjustmentKind, AdjustmentSegment, EnvelopeShape, ExecutionClass, LayoutCost, LayoutUnits,
};

/// A sequential line: the container's own shape and each child's, side by
/// side. Capacities add; the merged shape keeps the cheapest first.
pub fn aggregate_sequential<'a>(
    own: &EnvelopeShape,
    children: impl IntoIterator<Item = &'a EnvelopeShape>,
) -> EnvelopeShape {
    let mut segments: Vec<AdjustmentSegment> = own.segments().to_vec();
    for child in children {
        segments.extend_from_slice(child.segments());
    }
    EnvelopeShape::normalized(&mut segments)
}

/// Children stacked across the line: the container is as big as its
/// biggest `preferred`, and shrinking it by `d` shrinks every child whose
/// preferred size passes the target. The marginal cost of the next unit is
/// the sum of what each such child pays for its next unit, so it only grows
/// as the target falls; the shape stops where a child runs out of capacity.
pub fn aggregate_parallel(children: &[(LayoutUnits, &EnvelopeShape)]) -> EnvelopeShape {
    let mut order: Vec<usize> = (0..children.len()).collect();
    // Tallest first; ties in child order.
    order.sort_by_key(|at| (std::cmp::Reverse(children[*at].0), *at));
    let Some(&first) = order.first() else {
        return EnvelopeShape::RIGID;
    };
    let top = children[first].0;
    let mut active = 0usize;
    let mut target_drop = LayoutUnits::ZERO;
    let mut segments = Vec::new();
    loop {
        // Children whose preferred size the target has reached join.
        while active < order.len() && top - children[order[active]].0 <= target_drop {
            active += 1;
        }
        // The next unit's price, and how far it holds.
        let mut cost = LayoutCost::ZERO;
        let mut class = ExecutionClass::PlacementOnly;
        let mut span: Option<LayoutUnits> = None;
        let mut exhausted = false;
        for &at in &order[..active] {
            let (preferred, shape) = children[at];
            // A child joins already shrunk by how far below it the target is.
            let consumed = target_drop - (top - preferred);
            match segment_at(shape, consumed) {
                Some((segment, left)) => {
                    cost = cost.saturating_add(segment.marginal_cost);
                    class = class.max(segment.execution_class);
                    span = Some(span.map_or(left, |span| span.min(left)));
                }
                None => exhausted = true,
            }
        }
        if exhausted || cost.is_forbidden() {
            break;
        }
        // The next child to join bounds the span too.
        if active < order.len() {
            let joins = top - children[order[active]].0 - target_drop;
            span = Some(span.map_or(joins, |span| span.min(joins)));
        }
        let Some(span) = span.filter(|span| span.is_positive()) else {
            break;
        };
        segments.push(AdjustmentSegment::new(
            span,
            cost,
            AdjustmentKind::Aggregate,
            class,
        ));
        target_drop += span;
        if segments.len() > 4 * super::MAX_ENVELOPE_SEGMENTS * children.len().max(1) {
            break;
        }
    }
    EnvelopeShape::normalized(&mut segments)
}

/// The segment a shape is in after giving up `consumed`, and what is left of
/// it; `None` once the shape is spent.
fn segment_at(
    shape: &EnvelopeShape,
    consumed: LayoutUnits,
) -> Option<(AdjustmentSegment, LayoutUnits)> {
    let mut start = LayoutUnits::ZERO;
    for segment in shape.segments() {
        let end = start + segment.capacity;
        if consumed < end {
            return Some((*segment, end - consumed));
        }
        start = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(px: f32) -> LayoutUnits {
        LayoutUnits::from_px(px)
    }

    fn shape(segments: &[(f32, u32)]) -> EnvelopeShape {
        EnvelopeShape::from_segments(segments.iter().map(|(capacity, cost)| {
            AdjustmentSegment::new(
                px(*capacity),
                LayoutCost::Finite(*cost),
                AdjustmentKind::Padding,
                ExecutionClass::LocalBox,
            )
        }))
    }

    #[test]
    fn a_line_adds_its_childrens_capacity_and_stays_bounded() {
        let own = shape(&[(4.0, 10)]);
        let children: Vec<_> = (0..100).map(|at| shape(&[(1.0, 20 + at % 13)])).collect();
        let line = aggregate_sequential(&own, &children);
        assert_eq!(line.total_capacity(), px(104.0));
        assert!(line.segments().len() <= crate::dynamic_layout::MAX_ENVELOPE_SEGMENTS);
        assert_eq!(line.segments()[0].marginal_cost, LayoutCost::Finite(10));
    }

    /// Two children across the line, 40 and 30 tall: the first 10 px only
    /// shrink the taller one; after that both pay.
    #[test]
    fn a_cross_axis_shrinks_every_child_past_the_target_together() {
        let tall = shape(&[(20.0, 10)]);
        let short = shape(&[(5.0, 20)]);
        let cross = aggregate_parallel(&[(px(40.0), &tall), (px(30.0), &short)]);
        let segments = cross.segments();
        assert_eq!(segments[0].capacity, px(10.0));
        assert_eq!(segments[0].marginal_cost, LayoutCost::Finite(10));
        assert_eq!(segments[1].capacity, px(5.0));
        assert_eq!(segments[1].marginal_cost, LayoutCost::Finite(30));
        // The short child is spent at 15: the box cannot go lower.
        assert_eq!(cross.total_capacity(), px(15.0));
        let rising = segments
            .windows(2)
            .all(|pair| pair[0].marginal_cost <= pair[1].marginal_cost);
        assert!(rising);
    }

    #[test]
    fn a_rigid_child_across_the_line_holds_the_box_once_reached() {
        let tall = shape(&[(20.0, 10)]);
        let rigid = EnvelopeShape::RIGID;
        let cross = aggregate_parallel(&[(px(40.0), &tall), (px(35.0), &rigid)]);
        assert_eq!(cross.total_capacity(), px(5.0));
    }
}
