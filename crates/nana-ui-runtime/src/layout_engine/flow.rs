//! Shared Runtime layout flow algorithms.

use super::*;
use crate::{FormattingContextKind as Context, ParticipationKind as Participation};

/// In-flow children, plus whether the answer depended on anything below
/// the direct children.
///
/// The cached plans (`ContainerPlan`, `MeasurePlan`) re-check only the children
/// the change closure reaches, and they identify those children by id in the
/// container's own child list. That identification is only sound while the flow
/// list is a subsequence of the direct children whose membership is decided by
/// each child's own style. Two cases break it, and both are reported here:
///
/// - `display: contents` splices a child's OWN children into the flow list, so
///   the list is a function of a grandchild list the plan never recorded.
/// - an inline-level child under a block parent is unboxed or not depending on
///   whether its subtree contains a block, so a change arbitrarily deep can
///   move the child in or out of the flow list while its style stays put.
///
/// Reporting the condition rather than checking for it afterwards keeps this
/// off the per-sibling path: the collection already visits every child, and a
/// membership test over the produced list would be quadratic.
pub(super) fn collect_flow_children_reporting(
    children: &[StableNodeId],
    nodes: &mut LayoutInputMap<'_>,
    parent_style: &LayoutStyle,
) -> Result<(Vec<StableNodeId>, bool), UiWorldError> {
    let mut out = nodes.scratch.ids();
    let mut descendant_dependent = false;
    collect_flow_children_into(
        children,
        nodes,
        parent_style,
        &mut out,
        &mut descendant_dependent,
    )?;
    Ok((out, descendant_dependent))
}

pub(super) fn collect_flow_children_into(
    children: &[StableNodeId],
    nodes: &mut LayoutInputMap<'_>,
    parent_style: &LayoutStyle,
    out: &mut Vec<StableNodeId>,
    descendant_dependent: &mut bool,
) -> Result<(), UiWorldError> {
    for child in children.iter().copied() {
        let Some(style) = nodes.style_ref(child) else {
            continue;
        };
        let participation = nodes.participation(child, &style, Context::from_display(parent_style));
        if participation == Participation::NoBox {
            continue;
        }
        if matches!(
            participation,
            Participation::Absolute | Participation::Fixed | Participation::FreeItem
        ) {
            continue;
        }
        // `inline-flex`/`inline-grid` are atomic boxes. Only a non-atomic
        // `inline` under a non-flex/grid container hoists a block descendant.
        let inline_span = participation == Participation::InlineSpan
            || (parent_style.display.is_none()
                && nodes.participation(child, &style, Context::Inline)
                    == Participation::InlineSpan);
        if inline_span {
            // Whichever way the test goes, the answer came from the child's
            // subtree, so a plan over this container cannot be re-checked from
            // the direct child list alone.
            *descendant_dependent = true;
            if inline_contains_block(child, nodes)? {
                let nested = match nodes.get(child)? {
                    Some(node) => (*node.children).clone(),
                    None => continue,
                };
                collect_flow_children_into(
                    &nested,
                    nodes,
                    parent_style,
                    out,
                    descendant_dependent,
                )?;
                continue;
            }
        }
        out.push(child);
    }
    Ok(())
}

pub(super) fn inline_contains_block(
    id: StableNodeId,
    nodes: &mut LayoutInputMap<'_>,
) -> Result<bool, UiWorldError> {
    let Some(node) = nodes.get(id)? else {
        return Ok(false);
    };
    let children: &Arc<Vec<StableNodeId>> = node.children;
    for &grandchild in children.iter() {
        let Some(style) = nodes.style_ref(grandchild) else {
            continue;
        };
        let participation = nodes.participation(grandchild, &style, Context::Inline);
        if matches!(
            participation,
            Participation::NoBox | Participation::Absolute | Participation::Fixed
        ) {
            continue;
        }
        if participation == Participation::InlineSpan {
            if inline_contains_block(grandchild, nodes)? {
                return Ok(true);
            }
            continue;
        }
        if participation == Participation::FlexItem {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The children placed beside the flow, one by one: positioned ones, and
/// in a `display: free` container every child with a box (its explicit rect
/// places it).
pub(super) fn collect_positioned_children(
    children: &[StableNodeId],
    nodes: &mut LayoutInputMap<'_>,
    parent_style: &LayoutStyle,
) -> Result<Vec<StableNodeId>, UiWorldError> {
    let mut out = Vec::new();
    collect_positioned_children_into(children, nodes, parent_style, &mut out)?;
    Ok(out)
}

pub(super) fn collect_positioned_children_into(
    children: &[StableNodeId],
    nodes: &mut LayoutInputMap<'_>,
    parent_style: &LayoutStyle,
    out: &mut Vec<StableNodeId>,
) -> Result<(), UiWorldError> {
    let parent = if parent_style.display == Some(DisplaySpec::Free) {
        Context::Free
    } else {
        Context::Flex
    };
    for child in children.iter().copied() {
        let Some(style) = nodes.style_ref(child) else {
            continue;
        };
        let participation = nodes.participation(child, &style, parent);
        if matches!(
            participation,
            Participation::Absolute | Participation::Fixed | Participation::FreeItem
        ) {
            out.push(child);
        }
    }
    Ok(())
}

/// The strut a line box of an ordinary block starts from: its ascent and
/// descent about the baseline ([`super::inline::line_strut`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LineStrut {
    pub(super) ascent: f32,
    pub(super) descent: f32,
    pub(super) font_ascent: f32,
    pub(super) font_descent: f32,
    pub(super) x_height: f32,
    pub(super) font_px: f32,
}

impl From<LineStrut> for crate::inline_content::ParentLineStrut {
    fn from(strut: LineStrut) -> Self {
        Self {
            font_ascent: strut.font_ascent,
            font_descent: strut.font_descent,
            x_height: strut.x_height,
            font_px: strut.font_px,
        }
    }
}

/// One wrap line: child indices plus the shortened IFC line box.
#[derive(Debug, Clone)]
pub(super) struct LineBoxSlot {
    pub(super) indices: Vec<usize>,
    pub(super) main_start: f32,
    pub(super) main_available: f32,
    /// Content-relative cross start (IFC only).
    pub(super) cross_y: f32,
    pub(super) pin_cross: bool,
    /// A line of an ordinary block's inline-level items: they sit on one
    /// baseline (CSS 2.1 §10.8.1), not at the line's top.
    pub(super) baseline_aligned: bool,
}
