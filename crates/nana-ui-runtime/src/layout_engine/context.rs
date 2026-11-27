//! Static formatting-context dispatch (Issues #197 and #199).
//!
//! Measure and placement both ask this one function which context a
//! container establishes, so the two passes cannot pick different
//! algorithms for the same node. The Flex, Grid, Inline and Overlay modules
//! keep their specialized algorithms and retained plans; Layers is the grid
//! algorithm over one cell that every child shares; Free has no flow at all,
//! its children are placed one by one from their explicit rects. Child membership is
//! classified by the same Participation contract used by LayoutNode queries.

use super::*;
use crate::FormattingContextKind;

/// The context a container runs for its in-flow children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EstablishedContext {
    pub kind: FormattingContextKind,
    /// A grid with explicit tracks, areas or placed items: the 2D track
    /// solver. A grid without any is a single implicit column, which the
    /// line packer places identically.
    pub grid_2d: bool,
}

impl EstablishedContext {
    pub(super) fn inline(self) -> bool {
        self.kind == FormattingContextKind::Inline
    }
}

/// The context node `id` with `style` establishes over its in-flow children
/// `flow`. A modal frame establishes Overlay whatever its display: its slot
/// contract places the children.
pub(super) fn establish_context(
    id: StableNodeId,
    style: &LayoutStyle,
    flow: &[StableNodeId],
    nodes: &LayoutInputMap<'_>,
) -> EstablishedContext {
    let kind = match style.display {
        _ if nodes.world.places_modal_slots(id) => FormattingContextKind::Overlay,
        Some(DisplaySpec::Layers) => FormattingContextKind::Layers,
        // Its children are all placed by their explicit rects, beside the
        // (empty) flow: the flex path below runs over no item.
        Some(DisplaySpec::Free) => FormattingContextKind::Free,
        Some(display) if display.is_grid_container() => FormattingContextKind::Grid,
        Some(display) if display.is_flex_container() => FormattingContextKind::Flex,
        _ if flow
            .iter()
            .any(|id| nodes.style_ref(*id).is_some_and(|s| s.is_inline_level())) =>
        {
            FormattingContextKind::Inline
        }
        _ => FormattingContextKind::Flex,
    };
    EstablishedContext {
        kind,
        grid_2d: matches!(
            kind,
            FormattingContextKind::Grid | FormattingContextKind::Layers
        ) && uses_2d_grid(style),
    }
}
