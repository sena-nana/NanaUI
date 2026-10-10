//! Formatting-context selection (Issue #197).
//!
//! Measure and placement both ask this one function which context a
//! container establishes, so the two passes cannot pick different
//! algorithms for the same node. The algorithms themselves stay where they
//! are; this only names the choice.

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

/// The context `style` establishes over its in-flow children `flow`. A modal
/// frame's slot placement is decided before flow and recorded by placement.
pub(super) fn establish_context(
    style: &LayoutStyle,
    flow: &[StableNodeId],
    nodes: &LayoutInputMap<'_>,
) -> EstablishedContext {
    let kind = match style.display {
        Some(display) if display.is_grid_container() => FormattingContextKind::Grid,
        Some(display) if display.is_flex_container() => FormattingContextKind::Flex,
        _ if flow
            .iter()
            .any(|id| nodes.style(*id).is_some_and(|s| s.is_inline_level())) =>
        {
            FormattingContextKind::Inline
        }
        _ => FormattingContextKind::Flow,
    };
    EstablishedContext {
        kind,
        grid_2d: kind == FormattingContextKind::Grid && uses_2d_grid(style, flow, nodes),
    }
}
