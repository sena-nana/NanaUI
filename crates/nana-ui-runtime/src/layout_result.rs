//! Canonical, render/input-facing output of layout (Issue #203).
//!
//! `LayoutBox` remains the small compatibility projection used by older
//! callers.  New consumers should retain a `LayoutResult`: it packages all
//! box geometry and the generation that produced it, so Scene, hit testing,
//! accessibility and scroll code cannot accidentally derive different
//! placements from component state.

use std::sync::Arc;

use crate::{LayoutBox, ScrollOffset, StableNodeId};

/// The kind of fragment emitted by a formatting context.  Fragments describe
/// layout output only; renderer primitives are deliberately not represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LayoutFragmentKind {
    TextLine,
    TextRun,
    InlineAtomic,
    FlexChildPlacement,
    GridChildPlacement,
    ChildPlacement,
    ComponentContent,
    Overlay,
    ScrollViewport,
}

/// A typed piece of layout output. `node` is present for a child/inline
/// fragment and absent for a text line or a component-owned part.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutFragment {
    pub kind: LayoutFragmentKind,
    pub node: Option<StableNodeId>,
    pub bounds: LayoutBox,
    pub index: usize,
    pub first_baseline: Option<f32>,
    pub last_baseline: Option<f32>,
}

impl LayoutFragment {
    pub const fn new(kind: LayoutFragmentKind, bounds: LayoutBox) -> Self {
        Self {
            kind,
            node: None,
            bounds,
            index: 0,
            first_baseline: None,
            last_baseline: None,
        }
    }

    pub const fn for_node(kind: LayoutFragmentKind, node: StableNodeId, bounds: LayoutBox) -> Self {
        Self {
            kind,
            node: Some(node),
            bounds,
            index: 0,
            first_baseline: None,
            last_baseline: None,
        }
    }
}

/// A named semantic part of a component's layout.  This is the bridge for
/// component content/overlay/viewport geometry; paint data stays in the
/// component's visual payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LayoutPartKind {
    ComponentContent,
    Overlay,
    ScrollViewport,
    TextContent,
    ChildPlacement,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutPart {
    pub kind: LayoutPartKind,
    pub node: Option<StableNodeId>,
    pub bounds: LayoutBox,
}

impl LayoutPart {
    pub const fn new(kind: LayoutPartKind, bounds: LayoutBox) -> Self {
        Self {
            kind,
            node: None,
            bounds,
        }
    }
}

/// Placement of one direct child in the parent's formatting context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutChildPlacement {
    pub node: StableNodeId,
    pub bounds: LayoutBox,
    pub index: usize,
}

/// Origin of a result, useful when diagnosing a stale or compatibility path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LayoutResultSource {
    #[default]
    RuntimeLayout,
    CompatibilityWrite,
}

/// Immutable geometry snapshot for one retained node.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutResult {
    /// The node's authoritative border box. This is the value projected by
    /// [`crate::UiWorld::layout_box`].
    pub bounds: LayoutBox,
    pub border_box: LayoutBox,
    pub padding_box: LayoutBox,
    pub content_box: LayoutBox,
    pub first_baseline: Option<f32>,
    pub last_baseline: Option<f32>,
    /// Union of the box and laid-out descendants, in document coordinates.
    pub overflow: LayoutBox,
    /// Scrollable extent in document coordinates and current offset. Paint-only
    /// scroll changes keep the result generation stable; a changed extent
    /// creates a new result.
    pub scroll_extent: LayoutBox,
    pub scroll_offset: ScrollOffset,
    pub child_placements: Arc<[LayoutChildPlacement]>,
    pub fragments: Arc<[LayoutFragment]>,
    pub parts: Arc<[LayoutPart]>,
    /// Nearest layout clip and containing block. The references are retained
    /// node identities, never renderer primitives.
    pub clip: Option<StableNodeId>,
    pub containing_block: Option<StableNodeId>,
    pub dependencies: Arc<[StableNodeId]>,
    pub dependency_generation: u64,
    pub generation: u64,
    pub source: LayoutResultSource,
}

impl LayoutResult {
    pub fn from_box(
        bounds: LayoutBox,
        padding: nana_ui_core::PaddingSpec,
        border: nana_ui_core::PaddingSpec,
    ) -> Self {
        let padding_box = inset(bounds, border.top, border.right, border.bottom, border.left);
        let content_box = inset(
            padding_box,
            padding.top,
            padding.right,
            padding.bottom,
            padding.left,
        );
        Self {
            bounds,
            border_box: bounds,
            padding_box,
            content_box,
            first_baseline: None,
            last_baseline: None,
            overflow: bounds,
            scroll_extent: content_box,
            scroll_offset: ScrollOffset::default(),
            child_placements: Arc::from([]),
            fragments: Arc::from([]),
            parts: Arc::from([]),
            clip: None,
            containing_block: None,
            dependencies: Arc::from([]),
            dependency_generation: 0,
            generation: 0,
            source: LayoutResultSource::CompatibilityWrite,
        }
    }

    /// Compare all layout output while ignoring the revision/source stamp.
    pub fn geometry_eq(&self, other: &Self) -> bool {
        self.bounds == other.bounds
            && self.border_box == other.border_box
            && self.padding_box == other.padding_box
            && self.content_box == other.content_box
            && self.first_baseline == other.first_baseline
            && self.last_baseline == other.last_baseline
            && self.overflow == other.overflow
            && self.scroll_extent == other.scroll_extent
            && self.child_placements == other.child_placements
            && self.fragments == other.fragments
            && self.parts == other.parts
            && self.clip == other.clip
            && self.containing_block == other.containing_block
            && self.dependencies == other.dependencies
    }

    /// Classify the externally visible result change for dependency-aware
    /// propagation.  The revision/source stamps and paint-only scroll offset
    /// are intentionally ignored: scrolling moves the retained projection but
    /// does not change the node's intrinsic or formatting metrics.
    pub fn metric_delta(&self, other: &Self) -> nana_ui_core::LayoutMetricDelta {
        use nana_ui_core::LayoutMetricDelta as Delta;

        let mut delta = Delta::NONE;
        if self.content_box.width != other.content_box.width {
            delta = delta.union(Delta::INTRINSIC_INLINE);
        }
        if self.content_box.height != other.content_box.height {
            delta = delta.union(Delta::INTRINSIC_BLOCK);
        }
        if self.first_baseline != other.first_baseline || self.last_baseline != other.last_baseline
        {
            delta = delta.union(Delta::BASELINE);
        }
        // Position is a separate exported metric. Comparing whole boxes here
        // would classify an otherwise placement-only move as USED_SIZE and
        // make the parent remeasure unnecessarily.
        if self.bounds.width != other.bounds.width
            || self.bounds.height != other.bounds.height
            || self.border_box.width != other.border_box.width
            || self.border_box.height != other.border_box.height
            || self.padding_box.width != other.padding_box.width
            || self.padding_box.height != other.padding_box.height
        {
            delta = delta.union(Delta::USED_SIZE);
        }
        if self.bounds.x != other.bounds.x
            || self.bounds.y != other.bounds.y
            || self.border_box.x != other.border_box.x
            || self.border_box.y != other.border_box.y
            || self.padding_box.x != other.padding_box.x
            || self.padding_box.y != other.padding_box.y
            || self.content_box.x != other.content_box.x
            || self.content_box.y != other.content_box.y
        {
            delta = delta.union(Delta::PLACEMENT);
        }
        // A container can keep its own box while changing the placement of a
        // direct child, fragment, or semantic part. Those nested geometries
        // are part of the retained result and must not be mistaken for a
        // stable boundary. Size changes also affect the container's exported
        // child metrics, while an origin-only change remains placement-only.
        let mut nested_placement = false;
        let mut nested_used_size = false;
        for (left, right) in self
            .child_placements
            .iter()
            .zip(other.child_placements.iter())
        {
            if left.bounds.x != right.bounds.x || left.bounds.y != right.bounds.y {
                nested_placement = true;
            }
            if left.bounds.width != right.bounds.width || left.bounds.height != right.bounds.height
            {
                nested_used_size = true;
            }
        }
        for (left, right) in self.fragments.iter().zip(other.fragments.iter()) {
            if left.bounds.x != right.bounds.x || left.bounds.y != right.bounds.y {
                nested_placement = true;
            }
            if left.bounds.width != right.bounds.width || left.bounds.height != right.bounds.height
            {
                nested_used_size = true;
            }
            if left.first_baseline != right.first_baseline
                || left.last_baseline != right.last_baseline
            {
                delta = delta.union(Delta::BASELINE);
            }
        }
        for (left, right) in self.parts.iter().zip(other.parts.iter()) {
            if left.bounds.x != right.bounds.x || left.bounds.y != right.bounds.y {
                nested_placement = true;
            }
            if left.bounds.width != right.bounds.width || left.bounds.height != right.bounds.height
            {
                nested_used_size = true;
            }
        }
        if nested_placement {
            delta = delta.union(Delta::PLACEMENT);
        }
        if nested_used_size {
            delta = delta.union(Delta::USED_SIZE);
        }
        if self.overflow != other.overflow {
            delta = delta.union(Delta::OVERFLOW);
        }
        if self.scroll_extent != other.scroll_extent {
            delta = delta.union(Delta::SCROLL_EXTENT);
        }
        let child_topology_changed = self.child_placements.len() != other.child_placements.len()
            || self
                .child_placements
                .iter()
                .zip(other.child_placements.iter())
                .any(|(left, right)| left.node != right.node || left.index != right.index);
        let fragment_topology_changed = self.fragments.len() != other.fragments.len()
            || self
                .fragments
                .iter()
                .zip(other.fragments.iter())
                .any(|(left, right)| {
                    left.kind != right.kind || left.node != right.node || left.index != right.index
                });
        let part_topology_changed = self.parts.len() != other.parts.len()
            || self
                .parts
                .iter()
                .zip(other.parts.iter())
                .any(|(left, right)| left.kind != right.kind || left.node != right.node);
        if self.clip != other.clip
            || self.containing_block != other.containing_block
            || self.dependencies != other.dependencies
            || child_topology_changed
            || fragment_topology_changed
            || part_topology_changed
        {
            delta = delta.union(Delta::TOPOLOGY);
        }
        delta
    }
}

pub(crate) fn inset(box_: LayoutBox, top: f32, right: f32, bottom: f32, left: f32) -> LayoutBox {
    let top = top.max(0.0);
    let right = right.max(0.0);
    let bottom = bottom.max(0.0);
    let left = left.max(0.0);
    LayoutBox {
        x: box_.x + left,
        y: box_.y + top,
        width: (box_.width - left - right).max(0.0),
        height: (box_.height - top - bottom).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_core::LayoutMetricDelta;

    fn result() -> LayoutResult {
        LayoutResult::from_box(
            LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 40.0,
            },
            nana_ui_core::PaddingSpec::default(),
            nana_ui_core::PaddingSpec::default(),
        )
    }

    #[test]
    fn placement_change_does_not_request_measure() {
        let mut moved = result();
        moved.bounds.x += 3.0;
        assert_eq!(result().metric_delta(&moved), LayoutMetricDelta::PLACEMENT);

        let mut inset_moved = result();
        inset_moved.padding_box.x += 2.0;
        assert_eq!(
            result().metric_delta(&inset_moved),
            LayoutMetricDelta::PLACEMENT
        );

        let mut child_moved = result();
        child_moved.child_placements = Arc::from([LayoutChildPlacement {
            node: StableNodeId::new(7).unwrap(),
            bounds: LayoutBox {
                x: 4.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            index: 0,
        }]);
        let mut child_moved_again = child_moved.clone();
        child_moved_again.child_placements = Arc::from([LayoutChildPlacement {
            node: StableNodeId::new(7).unwrap(),
            bounds: LayoutBox {
                x: 8.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            index: 0,
        }]);
        assert_eq!(
            child_moved.metric_delta(&child_moved_again),
            LayoutMetricDelta::PLACEMENT
        );
    }

    #[test]
    fn baseline_change_is_exported_and_scroll_offset_is_paint_only() {
        let mut baseline = result();
        baseline.first_baseline = Some(12.0);
        assert_eq!(
            result().metric_delta(&baseline),
            LayoutMetricDelta::BASELINE
        );

        let mut scrolled = result();
        scrolled.scroll_offset.x = 2.0;
        assert_eq!(result().metric_delta(&scrolled), LayoutMetricDelta::NONE);
    }
}
