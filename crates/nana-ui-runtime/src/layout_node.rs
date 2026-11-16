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
}

impl FormattingContextKind {
    pub(crate) const fn cache_id(self) -> crate::FormattingContextId {
        crate::FormattingContextId::new(match self {
            Self::Flex => 0,
            Self::Grid => 1,
            Self::Inline => 2,
            Self::Overlay => 3,
            Self::Layers => 4,
        })
    }
    /// The context a style alone selects, before layout has run. Layout
    /// records the context it actually ran; this is the provisional answer
    /// for a node it has not reached yet.
    #[must_use]
    pub fn from_display(style: &LayoutStyle) -> Self {
        match style.display {
            Some(DisplaySpec::Layers) => Self::Layers,
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
        for parent in [Ctx::Flex, Ctx::Grid, Ctx::Layers, Ctx::Inline] {
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
