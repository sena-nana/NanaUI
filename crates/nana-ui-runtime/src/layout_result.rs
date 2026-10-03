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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutResultSource {
    RuntimeLayout,
    CompatibilityWrite,
}

impl Default for LayoutResultSource {
    fn default() -> Self {
        Self::RuntimeLayout
    }
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
}

fn inset(box_: LayoutBox, top: f32, right: f32, bottom: f32, left: f32) -> LayoutBox {
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
