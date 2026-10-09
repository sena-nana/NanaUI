//! Responsive policy (Issue #265): a node takes a layout variant by the size
//! of a container box, from bounded size buckets.
//!
//! A rule is layout intent. The variant its bucket picks is a patch over
//! the node's own layout, resolved where design intent is: the node's
//! resolved layout is its authored layout, its design intent applied, then
//! its variant. A bucket change is therefore a style change to the node,
//! classified and seeded like one, and laid out by the one incremental
//! layout. No query engine, renderer or callback writes a box.
//!
//! Rules are indexed by their container. A container whose content box moved
//! on the axis its rules read evaluates those rules and no other; a rule
//! that lands in the bucket it was in changes nothing.

use std::fmt;
use std::sync::Arc;

use nana_ui_core::LayoutStyle;

use crate::StableNodeId;

/// The most breakpoints a rule takes: buckets are bounded.
pub const MAX_RESPONSIVE_BREAKPOINTS: usize = 16;

/// The box whose size a rule reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResponsiveContainer {
    /// The node's parent: the space the node is given.
    Parent,
    /// A named box: a dock panel, a page.
    Node(StableNodeId),
}

/// The axis of the container's content box a rule reads, in the container's
/// writing mode: inline is its width in horizontal writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResponsiveAxis {
    Inline,
    Block,
}

/// A layout a rule applies in one bucket: a patch over the node's own
/// layout. Two variants are the same variant when they are one patch.
#[derive(Clone)]
pub(crate) struct LayoutVariant(Arc<dyn Fn(&mut LayoutStyle) + Send + Sync>);

impl LayoutVariant {
    pub(crate) fn apply(&self, layout: &mut LayoutStyle) {
        (self.0)(layout);
    }
}

impl fmt::Debug for LayoutVariant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LayoutVariant(..)")
    }
}

impl PartialEq for LayoutVariant {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A node's responsive policy: the container size it follows, where that
/// size's buckets break, and the variant the node takes in each.
///
/// Bucket 0 lies below the first breakpoint; bucket `i` runs from
/// breakpoint `i - 1` up to the next. A bucket with no variant keeps the
/// node's own layout.
///
/// ```
/// # use nana_ui_runtime::{ResponsiveAxis, ResponsiveContainer, ResponsiveRule};
/// # use nana_ui_core::FlexDirection;
/// // A toolbar stacks its items below 480 px of the space it is given.
/// let compact = ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
///     .below(480.0, |layout| layout.direction = Some(FlexDirection::Column));
/// assert_eq!(compact.bucket_for(479.0), 0);
/// assert_eq!(compact.bucket_for(480.0), 1);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ResponsiveRule {
    container: ResponsiveContainer,
    axis: ResponsiveAxis,
    breakpoints: Vec<f32>,
    variants: Vec<Option<LayoutVariant>>,
}

impl ResponsiveRule {
    /// A rule on `axis` of `container` with one bucket: the node's own
    /// layout at every size.
    pub fn new(container: ResponsiveContainer, axis: ResponsiveAxis) -> Self {
        Self {
            container,
            axis,
            breakpoints: Vec::new(),
            variants: vec![None],
        }
    }

    /// Below `extent`, apply `patch`; from `extent` up, the buckets above.
    /// Sets the first breakpoint, so call it once, before
    /// [`Self::at_least`].
    pub fn below(
        mut self,
        extent: f32,
        patch: impl Fn(&mut LayoutStyle) + Send + Sync + 'static,
    ) -> Self {
        self.variants[0] = Some(LayoutVariant(Arc::new(patch)));
        self.breakpoints.insert(0, extent);
        self.variants.insert(1, None);
        self
    }

    /// From `extent` up to the next breakpoint, apply `patch`. Breakpoints
    /// ascend: `extent` lies above every one already set.
    pub fn at_least(
        mut self,
        extent: f32,
        patch: impl Fn(&mut LayoutStyle) + Send + Sync + 'static,
    ) -> Self {
        self.breakpoints.push(extent);
        self.variants.push(Some(LayoutVariant(Arc::new(patch))));
        self
    }

    /// From `extent` up to the next breakpoint, keep the node's own layout.
    pub fn own_from(mut self, extent: f32) -> Self {
        self.breakpoints.push(extent);
        self.variants.push(None);
        self
    }

    pub fn container(&self) -> ResponsiveContainer {
        self.container
    }

    pub fn axis(&self) -> ResponsiveAxis {
        self.axis
    }

    /// The bucket a container extent falls in.
    pub fn bucket_for(&self, extent: f32) -> usize {
        self.breakpoints
            .partition_point(|breakpoint| *breakpoint <= extent)
    }

    /// The variant of `bucket`, if it has one.
    pub(crate) fn variant(&self, bucket: usize) -> Option<&LayoutVariant> {
        self.variants.get(bucket).and_then(Option::as_ref)
    }

    /// Finite, strictly ascending breakpoints, at most
    /// [`MAX_RESPONSIVE_BREAKPOINTS`] of them.
    pub(crate) fn is_valid(&self) -> bool {
        self.breakpoints.len() <= MAX_RESPONSIVE_BREAKPOINTS
            && self
                .breakpoints
                .iter()
                .all(|breakpoint| breakpoint.is_finite())
            && self.breakpoints.windows(2).all(|pair| pair[0] < pair[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_break_at_each_breakpoint_from_below() {
        let rule = ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
            .below(480.0, |_| {})
            .at_least(960.0, |_| {});
        assert!(rule.is_valid());
        assert_eq!(rule.bucket_for(0.0), 0);
        assert_eq!(rule.bucket_for(479.9), 0);
        assert_eq!(rule.bucket_for(480.0), 1);
        assert_eq!(rule.bucket_for(959.0), 1);
        assert_eq!(rule.bucket_for(960.0), 2);
        assert!(rule.variant(0).is_some());
        assert!(rule.variant(1).is_none());
        assert!(rule.variant(2).is_some());
    }

    #[test]
    fn breakpoints_must_ascend_and_stay_bounded() {
        let descending = ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
            .own_from(960.0)
            .own_from(480.0);
        assert!(!descending.is_valid());
        let nan = ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Block)
            .own_from(f32::NAN);
        assert!(!nan.is_valid());
        let mut many = ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline);
        for step in 0..=MAX_RESPONSIVE_BREAKPOINTS {
            many = many.own_from(step as f32 * 10.0);
        }
        assert!(!many.is_valid());
    }
}
