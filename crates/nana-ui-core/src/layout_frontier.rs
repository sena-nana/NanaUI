//! Compact contracts for dependency-aware incremental layout.
//!
//! The runtime owns the frontier builder and the retained layout plans.  The
//! types in this module deliberately contain only fixed-size masks and enums:
//! a node must not carry a copy of all of its ancestors or descendants.  CSS,
//! Rust layout and component projection can therefore describe the same
//! invalidation without depending on one another's implementation.

use crate::layout_authority::LayoutFieldMask;
use std::ops::{BitOr, BitOrAssign};

/// What a layout node consumes or exports at a dependency boundary.
///
/// These flags describe an edge, rather than a particular node.  For example,
/// a child that consumes its parent's inline constraint can be reached by a
/// top-down width change; a parent that exports intrinsic inline size can be
/// reached by a bottom-up text mutation.  Keeping this as a mask makes union
/// and intersection constant time and keeps per-node metadata bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LayoutDependencyFootprint(u16);

impl LayoutDependencyFootprint {
    pub const NONE: Self = Self(0);
    pub const CONSUMES_PARENT_INLINE_CONSTRAINT: Self = Self(1 << 0);
    pub const CONSUMES_PARENT_BLOCK_CONSTRAINT: Self = Self(1 << 1);
    pub const EXPORTS_INTRINSIC_INLINE: Self = Self(1 << 2);
    pub const EXPORTS_INTRINSIC_BLOCK: Self = Self(1 << 3);
    pub const EXPORTS_BASELINE: Self = Self(1 << 4);
    pub const DEPENDS_ON_CHILD_METRICS: Self = Self(1 << 5);
    pub const DEPENDS_ON_SIBLING_PREFIX: Self = Self(1 << 6);
    pub const DEPENDS_ON_CONTAINING_BLOCK: Self = Self(1 << 7);
    pub const DEPENDS_ON_WRITING_CONTEXT: Self = Self(1 << 8);
    pub const CONTEXT_LOCAL_COUPLING: Self = Self(1 << 9);
    pub const ALL: Self = Self((1 << 10) - 1);

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// A footprint for a child whose used size is a direct function of its
    /// parent constraints.  This is useful for simple block/flex leaves.
    pub const fn parent_constraints() -> Self {
        Self::CONSUMES_PARENT_INLINE_CONSTRAINT.union(Self::CONSUMES_PARENT_BLOCK_CONSTRAINT)
    }

    /// A footprint for a content-sized container that exports its children's
    /// intrinsic metrics and has to observe those metrics itself.
    pub const fn intrinsic_container() -> Self {
        Self::EXPORTS_INTRINSIC_INLINE
            .union(Self::EXPORTS_INTRINSIC_BLOCK)
            .union(Self::DEPENDS_ON_CHILD_METRICS)
    }
}

impl BitOr for LayoutDependencyFootprint {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl BitOrAssign for LayoutDependencyFootprint {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// The stage(s) made stale by an invalidation.  A single seed may require
/// both measuring and placement, hence this is a mask rather than an enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InvalidationKind(u8);

impl InvalidationKind {
    pub const NONE: Self = Self(0);
    pub const MEASURE: Self = Self(1 << 0);
    pub const PLACEMENT: Self = Self(1 << 1);
    pub const CONTEXT_REFLOW: Self = Self(1 << 2);
    pub const WRITING_CONTEXT: Self = Self(1 << 3);
    pub const SCROLL_OVERFLOW: Self = Self(1 << 4);
    pub const TOPOLOGY: Self = Self(1 << 5);
    pub const ALL: Self = Self((1 << 6) - 1);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl BitOr for InvalidationKind {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl BitOrAssign for InvalidationKind {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Why a frontier seed was produced.  The reason is diagnostic data; the
/// typed kind and changed fields are what the scheduler uses for propagation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InvalidationReason(u16);

impl InvalidationReason {
    pub const NONE: Self = Self(0);
    pub const STYLE: Self = Self(1 << 0);
    pub const TEXT: Self = Self(1 << 1);
    pub const FONT: Self = Self(1 << 2);
    pub const PARENT_CONSTRAINT: Self = Self(1 << 3);
    pub const CONTAINING_BLOCK: Self = Self(1 << 4);
    pub const SIBLING: Self = Self(1 << 5);
    pub const CONTEXT: Self = Self(1 << 6);
    pub const WRITING: Self = Self(1 << 7);
    pub const SCROLL: Self = Self(1 << 8);
    pub const RESOURCE: Self = Self(1 << 9);
    pub const LOCALE: Self = Self(1 << 10);
    pub const VIEWPORT: Self = Self(1 << 11);
    pub const TOPOLOGY: Self = Self(1 << 12);
    pub const UNKNOWN: Self = Self(1 << 13);
    pub const ALL: Self = Self((1 << 14) - 1);
    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOr for InvalidationReason {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl BitOrAssign for InvalidationReason {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// The authority that emitted an invalidation.  Keeping source separate from
/// [`InvalidationReason`] lets diagnostics say both "text" and "nana-text".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum LayoutInvalidationSource {
    Author,
    Component,
    Runtime,
    Text,
    Font,
    Resource,
    Locale,
    Viewport,
    Structure,
    Scroll,
    #[default]
    Unknown,
}

/// Typed footprint emitted by the layout mutation/resolution authority.
///
/// `changed_inputs` reuses [`LayoutFieldMask`] so the same property
/// classification used by the layout resolver is visible to the scheduler.
/// `affected_axes` describes dependency edges that may need to be followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutInvalidation {
    pub source: LayoutInvalidationSource,
    pub reason: InvalidationReason,
    pub kind: InvalidationKind,
    pub changed_inputs: LayoutFieldMask,
    pub affected_axes: LayoutDependencyFootprint,
}

impl LayoutInvalidation {
    pub const fn new(
        source: LayoutInvalidationSource,
        reason: InvalidationReason,
        kind: InvalidationKind,
        changed_inputs: LayoutFieldMask,
        affected_axes: LayoutDependencyFootprint,
    ) -> Self {
        Self {
            source,
            reason,
            kind,
            changed_inputs,
            affected_axes,
        }
    }

    pub const fn none() -> Self {
        Self::new(
            LayoutInvalidationSource::Unknown,
            InvalidationReason::NONE,
            InvalidationKind::NONE,
            LayoutFieldMask::NONE,
            LayoutDependencyFootprint::NONE,
        )
    }

    pub const fn is_empty(self) -> bool {
        self.kind.is_empty() && self.changed_inputs.bits() == 0 && self.affected_axes.is_empty()
    }

    /// Merge seeds for the same node in one frame. Reasons are retained as a
    /// union so diagnostics can explain every cause without allocating a list.
    /// A disagreement in source becomes `Unknown` without dropping any work.
    pub fn merge(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let source = if self.source == other.source {
            self.source
        } else {
            LayoutInvalidationSource::Unknown
        };
        Self {
            source,
            reason: self.reason.union(other.reason),
            kind: self.kind.union(other.kind),
            changed_inputs: self.changed_inputs.union(other.changed_inputs),
            affected_axes: self.affected_axes.union(other.affected_axes),
        }
    }

    pub const fn with_kind(mut self, kind: InvalidationKind) -> Self {
        self.kind = self.kind.union(kind);
        self
    }
}

/// Which exported layout facts changed after a local recompute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LayoutMetricDelta(u16);

impl LayoutMetricDelta {
    pub const NONE: Self = Self(0);
    pub const INTRINSIC_INLINE: Self = Self(1 << 0);
    pub const INTRINSIC_BLOCK: Self = Self(1 << 1);
    pub const BASELINE: Self = Self(1 << 2);
    pub const USED_SIZE: Self = Self(1 << 3);
    pub const PLACEMENT: Self = Self(1 << 4);
    pub const OVERFLOW: Self = Self(1 << 5);
    pub const WRITING_CONTEXT: Self = Self(1 << 6);
    pub const TOPOLOGY: Self = Self(1 << 7);
    pub const SCROLL_EXTENT: Self = Self(1 << 8);
    pub const ALL: Self = Self((1 << 9) - 1);

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn is_none(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether an ancestor may need a fresh intrinsic measure. Placement-only
    /// changes deliberately return false: moving a child does not change its
    /// exported size or baseline.
    pub const fn propagates_measure(self) -> bool {
        self.intersects(
            Self::INTRINSIC_INLINE
                .union(Self::INTRINSIC_BLOCK)
                .union(Self::BASELINE)
                .union(Self::USED_SIZE)
                .union(Self::WRITING_CONTEXT)
                .union(Self::TOPOLOGY),
        )
    }

    /// Whether an ancestor/context needs placement or context solving.
    pub const fn propagates_placement(self) -> bool {
        self.intersects(
            Self::INTRINSIC_INLINE
                .union(Self::INTRINSIC_BLOCK)
                .union(Self::BASELINE)
                .union(Self::USED_SIZE)
                .union(Self::PLACEMENT)
                .union(Self::OVERFLOW)
                .union(Self::WRITING_CONTEXT)
                .union(Self::TOPOLOGY)
                .union(Self::SCROLL_EXTENT),
        )
    }

    pub const fn is_placement_only(self) -> bool {
        self.0 == Self::PLACEMENT.0
    }

    /// Dependency classes exported by this result change. This lets a
    /// retained scheduler choose only consumers of the changed metric when it
    /// has a dependency index available.
    pub const fn affected_footprint(self) -> LayoutDependencyFootprint {
        let mut footprint = LayoutDependencyFootprint::NONE;
        if self.intersects(Self::INTRINSIC_INLINE.union(Self::USED_SIZE)) {
            footprint = footprint
                .union(LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE)
                .union(LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS);
        }
        if self.intersects(Self::INTRINSIC_BLOCK.union(Self::USED_SIZE)) {
            footprint = footprint
                .union(LayoutDependencyFootprint::EXPORTS_INTRINSIC_BLOCK)
                .union(LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS);
        }
        if self.intersects(Self::BASELINE) {
            footprint = footprint
                .union(LayoutDependencyFootprint::EXPORTS_BASELINE)
                .union(LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS);
        }
        if self.intersects(Self::OVERFLOW.union(Self::SCROLL_EXTENT)) {
            footprint = footprint.union(LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS);
        }
        if self.intersects(Self::WRITING_CONTEXT) {
            footprint = footprint.union(LayoutDependencyFootprint::DEPENDS_ON_WRITING_CONTEXT);
        }
        if self.intersects(Self::PLACEMENT) {
            footprint = footprint.union(LayoutDependencyFootprint::CONTEXT_LOCAL_COUPLING);
        }
        if self.intersects(Self::TOPOLOGY) {
            footprint = footprint.union(LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS);
        }
        footprint
    }
}

impl BitOr for LayoutMetricDelta {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl BitOrAssign for LayoutMetricDelta {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footprint_and_kind_masks_are_constant_size_and_composable() {
        let footprint = LayoutDependencyFootprint::parent_constraints()
            .union(LayoutDependencyFootprint::DEPENDS_ON_CONTAINING_BLOCK);
        assert!(footprint.contains(LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT));
        assert!(footprint.intersects(LayoutDependencyFootprint::DEPENDS_ON_CONTAINING_BLOCK));
        assert!(
            !footprint
                .without(LayoutDependencyFootprint::parent_constraints())
                .contains(LayoutDependencyFootprint::CONSUMES_PARENT_BLOCK_CONSTRAINT)
        );

        let kinds = InvalidationKind::MEASURE.union(InvalidationKind::PLACEMENT);
        assert!(kinds.contains(InvalidationKind::MEASURE));
        assert!(kinds.intersects(InvalidationKind::PLACEMENT));
    }

    #[test]
    fn merged_seeds_union_work_and_keep_all_reasons() {
        let first = LayoutInvalidation::new(
            LayoutInvalidationSource::Text,
            InvalidationReason::TEXT,
            InvalidationKind::MEASURE,
            LayoutFieldMask::TYPOGRAPHY,
            LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE,
        );
        let second = LayoutInvalidation::new(
            LayoutInvalidationSource::Author,
            InvalidationReason::STYLE,
            InvalidationKind::PLACEMENT,
            LayoutFieldMask::SPACING,
            LayoutDependencyFootprint::DEPENDS_ON_SIBLING_PREFIX,
        );
        let merged = first.merge(second);
        assert_eq!(merged.source, LayoutInvalidationSource::Unknown);
        assert!(merged.reason.contains(InvalidationReason::TEXT));
        assert!(merged.reason.contains(InvalidationReason::STYLE));
        assert!(merged.kind.contains(InvalidationKind::MEASURE));
        assert!(merged.kind.contains(InvalidationKind::PLACEMENT));
        assert!(merged.changed_inputs.contains(LayoutFieldMask::TYPOGRAPHY));
        assert!(merged.changed_inputs.contains(LayoutFieldMask::SPACING));
    }

    #[test]
    fn metric_delta_stops_after_a_placement_only_change() {
        assert!(!LayoutMetricDelta::NONE.propagates_measure());
        assert!(!LayoutMetricDelta::PLACEMENT.propagates_measure());
        assert!(LayoutMetricDelta::PLACEMENT.is_placement_only());
        assert!(
            !(LayoutMetricDelta::PLACEMENT.union(LayoutMetricDelta::OVERFLOW)).is_placement_only()
        );
        assert!(LayoutMetricDelta::INTRINSIC_INLINE.propagates_measure());
        assert!(LayoutMetricDelta::INTRINSIC_BLOCK.propagates_placement());
        assert!(!LayoutMetricDelta::NONE.propagates_placement());
        assert!(
            LayoutMetricDelta::PLACEMENT
                .affected_footprint()
                .contains(LayoutDependencyFootprint::CONTEXT_LOCAL_COUPLING)
        );
    }

    #[test]
    fn empty_invalidation_is_the_identity_for_seed_merge() {
        let seed = LayoutInvalidation::new(
            LayoutInvalidationSource::Text,
            InvalidationReason::TEXT,
            InvalidationKind::MEASURE,
            LayoutFieldMask::INTRINSIC,
            LayoutDependencyFootprint::EXPORTS_INTRINSIC_BLOCK,
        );
        assert_eq!(LayoutInvalidation::none().merge(seed), seed);
        assert_eq!(seed.merge(LayoutInvalidation::none()), seed);
    }
}
