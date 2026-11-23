//! Canonical layout view of a retained node (Issue #197).
//!
//! Three things stay apart:
//!
//! - **identity**: the node's [`StableNodeId`] and its retained record. A
//!   layout context never creates, wraps or despawns it;
//! - **participation**: how the node takes part in its parent's formatting
//!   context ([`ParticipationKind`]). It is a view derived from the parent's
//!   context and the node's resolved layout intent, not a component;
//! - **established context**: how the node lays out its own children
//!   ([`FormattingContextKind`]). Layout records the context it actually ran.
//!
//! A context transition (Flex → Grid → Inline over the same children) changes
//! the established context, the children's participation and the layout
//! result. It does not change any child's identity, component, listeners or
//! state. Contexts only read the resolved layout intent (#166); they never
//! resolve style themselves.

use std::sync::Arc;

use nana_ui_core::{DisplaySpec, LayoutStyle, PositionSpec};

use crate::{LayoutFragmentKind, StableNodeId};

/// The algorithm a node runs to lay out its own children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FormattingContextKind {
    /// Flex container: `display: flex` / `inline-flex`, or unset container intent.
    Flex,
    /// Grid container: `display: grid` / `inline-grid`.
    Grid,
    /// A line-based inline context. A container whose in-flow children
    /// include inline-level boxes establishes it, and so does a text leaf for
    /// its own text: those glyph runs belong to nana-text, not to layout nodes.
    Inline,
    /// Modal slot placement: the children are placed by the overlay's slot
    /// contract, not by flow.
    Overlay,
    /// `display: layers`: every in-flow child shares one area, each aligned
    /// in it, later ones above earlier ones (SwiftUI `ZStack`, Compose `Box`).
    /// It runs the grid algorithm over a single `1fr` cell.
    Layers,
    /// `display: free`: every in-flow child is placed by its own
    /// [`ExplicitRect`], in the container's padding box. The children have no
    /// layout dependency on one another and do not size the container.
    Free,
}

impl FormattingContextKind {
    pub(crate) const fn cache_id(self) -> crate::FormattingContextId {
        crate::FormattingContextId::new(match self {
            Self::Flex => 0,
            Self::Grid => 1,
            Self::Inline => 2,
            Self::Overlay => 3,
            Self::Layers => 4,
            Self::Free => 5,
        })
    }
    /// The context a style alone selects, before layout has run. Layout
    /// records the context it actually ran; this is the provisional answer
    /// for a node it has not reached yet.
    #[must_use]
    pub fn from_display(style: &LayoutStyle) -> Self {
        match style.display {
            Some(DisplaySpec::Layers) => Self::Layers,
            Some(DisplaySpec::Free) => Self::Free,
            Some(display) if display.is_grid_container() => Self::Grid,
            Some(DisplaySpec::Inline) => Self::Inline,
            _ => Self::Flex,
        }
    }

    /// The fragment kind of a child placed by this context.
    #[must_use]
    pub const fn child_fragment_kind(self) -> LayoutFragmentKind {
        match self {
            Self::Flex | Self::Overlay => LayoutFragmentKind::FlexChildPlacement,
            Self::Grid | Self::Layers => LayoutFragmentKind::GridChildPlacement,
            Self::Inline => LayoutFragmentKind::InlineAtomic,
            Self::Free => LayoutFragmentKind::ChildPlacement,
        }
    }
}

/// How a node takes part in its parent's formatting context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParticipationKind {
    /// A document root: no parent context.
    Root,
    FlexItem,
    GridItem,
    /// An in-flow child of a `display: layers` container: one layer.
    LayerItem,
    /// A child of a `display: free` container: placed by its
    /// [`ExplicitRect`], depending on its parent alone.
    FreeItem,
    /// Inline-level text in an inline context: its glyph runs join the
    /// parent's lines directly.
    NativeText,
    /// An inline-level element with children of its own: its content joins
    /// the parent's lines (and is split around a block it contains).
    InlineSpan,
    /// Any other inline-level box (`inline-flex`, `inline-grid`, a control,
    /// replaced content) in an inline context: one unbreakable box on a line.
    AtomicInline,
    Absolute,
    Fixed,
    /// A child placed by an overlay's slot contract.
    OverlayItem,
    /// `display: none` or hidden: no box.
    NoBox,
}

impl ParticipationKind {
    /// The participation of a child with `child` intent and `content` in a
    /// `parent` context. This is the only mapping from a context to its
    /// children's participation; every context kind decides it here, so a new
    /// context does not compile until it does.
    #[must_use]
    pub fn classify(
        parent: FormattingContextKind,
        child: &LayoutStyle,
        content: LayoutContentKind,
    ) -> Self {
        if child.omits_box() {
            return Self::NoBox;
        }
        if parent != FormattingContextKind::Overlay {
            match child.position {
                PositionSpec::Absolute => return Self::Absolute,
                PositionSpec::Fixed => return Self::Fixed,
                PositionSpec::Static | PositionSpec::Relative | PositionSpec::Sticky => {}
            }
        }
        match parent {
            FormattingContextKind::Overlay => Self::OverlayItem,
            FormattingContextKind::Flex => Self::FlexItem,
            FormattingContextKind::Grid => Self::GridItem,
            FormattingContextKind::Layers => Self::LayerItem,
            FormattingContextKind::Free => Self::FreeItem,
            FormattingContextKind::Inline => {
                if child.display == Some(DisplaySpec::Inline) {
                    match content {
                        LayoutContentKind::Text => Self::NativeText,
                        LayoutContentKind::Children => Self::InlineSpan,
                        LayoutContentKind::Replaced | LayoutContentKind::Empty => {
                            Self::AtomicInline
                        }
                    }
                } else if child.is_inline_level() {
                    Self::AtomicInline
                } else {
                    Self::FlexItem
                }
            }
        }
    }
}

/// Whether `child` stays out of its parent's flow: positioned out of flow, or
/// a box placed by its explicit rect in a `display: free` parent. Either way
/// its siblings do not measure it and it does not size the parent. Every
/// "is this child in my flow" question that reads only the child's style asks
/// this instead, so a free item never couples to its siblings.
#[must_use]
pub(crate) fn leaves_parent_flow(parent: Option<&LayoutStyle>, child: &LayoutStyle) -> bool {
    child.position.is_out_of_flow() || is_free_item(parent, child)
}

/// A child `display: free` places by its explicit rect: one that generates a
/// box and is not itself `absolute` or `fixed`.
#[must_use]
pub(crate) fn is_free_item(parent: Option<&LayoutStyle>, child: &LayoutStyle) -> bool {
    parent.is_some_and(|parent| parent.display == Some(DisplaySpec::Free))
        && !child.omits_box()
        && !child.position.is_out_of_flow()
}

/// Where a child of a `display: free` container goes: logical pixels in its
/// parent's padding box, origin at the top left, y down.
///
/// The rect is authored data, not a layout style: writing it does not touch
/// `left`, `top`, `width` or `height`, and layout turns it into document
/// coordinates only when it places the child. It depends on the parent
/// alone: a sibling's rect never measures this one. A transform moves,
/// scales or rotates the painted box without changing this rect.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ExplicitRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ExplicitRect {
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Finite numbers and a size that is not negative.
    #[must_use]
    pub fn is_valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite())
            && self.width >= 0.0
            && self.height >= 0.0
    }

    /// Same size, bit for bit: only the origin differs.
    #[must_use]
    pub fn same_size(self, other: Self) -> bool {
        self.width.to_bits() == other.width.to_bits()
            && self.height.to_bits() == other.height.to_bits()
    }
}

/// What a node's own content is, independent of its component type: the
/// same contract covers Text, Image, Button, containers and custom render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LayoutContentKind {
    /// Child layout nodes.
    Children,
    /// Text of its own (a text node, or a leaf with a label).
    Text,
    /// Replaced content: an image, host texture or custom renderer.
    Replaced,
    Empty,
}

/// The layout view of one retained node, built per query. It is not stored,
/// not a component and not a second tree: every field is read from the
/// node's record, its parent's record and its layout result.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutNode {
    pub id: StableNodeId,
    /// The resolved layout intent (#166). Shared, read only.
    pub intent: Arc<LayoutStyle>,
    pub content: LayoutContentKind,
    /// The context this node lays its children (or its own text) out in.
    /// `None` for an empty leaf.
    pub established: Option<FormattingContextKind>,
    /// The parent's established context. `None` for a root, and for a child
    /// whose parent layout has not placed yet.
    pub parent_context: Option<FormattingContextKind>,
    /// `None` until the parent has been laid out: never guessed.
    pub participation: Option<ParticipationKind>,
    /// Bumped each time layout records a different established context.
    pub context_generation: u64,
    /// Generation of the node's published layout result.
    pub result_generation: Option<u64>,
}

#[cfg(test)]
mod tests {
    use nana_ui_core::LengthSpec;

    use super::*;

    use FormattingContextKind as Ctx;
    use LayoutContentKind as Content;
    use ParticipationKind as Part;

    fn display(display: DisplaySpec) -> LayoutStyle {
        LayoutStyle {
            display: Some(display),
            ..LayoutStyle::default()
        }
    }

    fn positioned(position: PositionSpec) -> LayoutStyle {
        LayoutStyle {
            position,
            offset_left: Some(LengthSpec::Px(0.0)),
            ..LayoutStyle::default()
        }
    }

    #[test]
    fn participation_is_decided_by_the_parent_context() {
        let default_box = LayoutStyle::default();
        let inline = display(DisplaySpec::Inline);
        let inline_flex = display(DisplaySpec::InlineFlex);
        let cases = [
            (Ctx::Flex, &default_box, Content::Children, Part::FlexItem),
            (Ctx::Flex, &inline, Content::Text, Part::FlexItem),
            (Ctx::Flex, &inline_flex, Content::Replaced, Part::FlexItem),
            (Ctx::Grid, &default_box, Content::Children, Part::GridItem),
            (Ctx::Grid, &inline, Content::Text, Part::GridItem),
            (Ctx::Grid, &inline_flex, Content::Replaced, Part::GridItem),
            (
                Ctx::Layers,
                &default_box,
                Content::Children,
                Part::LayerItem,
            ),
            (Ctx::Layers, &inline, Content::Text, Part::LayerItem),
            (
                Ctx::Layers,
                &inline_flex,
                Content::Replaced,
                Part::LayerItem,
            ),
            (Ctx::Free, &default_box, Content::Children, Part::FreeItem),
            (Ctx::Free, &inline, Content::Text, Part::FreeItem),
            (Ctx::Free, &inline_flex, Content::Replaced, Part::FreeItem),
            (Ctx::Inline, &inline, Content::Text, Part::NativeText),
            (Ctx::Inline, &inline, Content::Children, Part::InlineSpan),
            (Ctx::Inline, &inline, Content::Replaced, Part::AtomicInline),
            (
                Ctx::Inline,
                &inline_flex,
                Content::Children,
                Part::AtomicInline,
            ),
            (
                Ctx::Inline,
                &inline_flex,
                Content::Replaced,
                Part::AtomicInline,
            ),
            (Ctx::Inline, &default_box, Content::Text, Part::FlexItem),
            (
                Ctx::Overlay,
                &default_box,
                Content::Children,
                Part::OverlayItem,
            ),
            (
                Ctx::Overlay,
                &positioned(PositionSpec::Fixed),
                Content::Empty,
                Part::OverlayItem,
            ),
        ];
        for (parent, child, content, expected) in cases {
            assert_eq!(
                Part::classify(parent, child, content),
                expected,
                "{parent:?} / {:?} / {content:?}",
                child.display
            );
        }
    }

    #[test]
    fn out_of_flow_and_boxless_children_do_not_depend_on_the_context() {
        let none = display(DisplaySpec::None);
        let absolute = positioned(PositionSpec::Absolute);
        let fixed = positioned(PositionSpec::Fixed);
        for parent in [Ctx::Flex, Ctx::Grid, Ctx::Layers, Ctx::Free, Ctx::Inline] {
            assert_eq!(Part::classify(parent, &none, Content::Text), Part::NoBox);
            assert_eq!(
                Part::classify(parent, &absolute, Content::Empty),
                Part::Absolute
            );
            assert_eq!(
                Part::classify(parent, &fixed, Content::Replaced),
                Part::Fixed
            );
        }
        assert_eq!(
            Part::classify(Ctx::Overlay, &none, Content::Empty),
            Part::NoBox
        );
    }
}
