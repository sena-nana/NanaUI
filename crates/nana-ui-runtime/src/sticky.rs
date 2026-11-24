//! Shared sticky placement projection. Layout boxes stay in normal flow.

use nana_ui_core::{DisplaySpec, FontSizeContext, LayoutStyle, PositionSpec};

use crate::{LayoutBox, NodeMap, NodeSet, ScrollOffset, StableNodeId};

/// Dependency index for sticky branches. A scroll visits its own dependents,
/// never all sticky nodes in every document. Topology changes rebind only the
/// moved subtree's registered branches.
#[derive(Debug, Clone, Default)]
pub struct StickyDependencies {
    ancestors: NodeMap<Vec<StableNodeId>>,
    dependents: NodeMap<NodeSet>,
}

impl StickyDependencies {
    pub fn has_dependents(&self, id: StableNodeId) -> bool {
        self.dependents.contains_key(&id)
    }
    pub fn remove(&mut self, id: StableNodeId) {
        // Most worlds hold no sticky box: no lookup to hash for.
        if self.ancestors.is_empty() {
            return;
        }
        if let Some(parents) = self.ancestors.remove(&id) {
            for parent in parents {
                if let Some(nodes) = self.dependents.get_mut(&parent) {
                    nodes.remove(&id);
                    if nodes.is_empty() {
                        self.dependents.remove(&parent);
                    }
                }
            }
        }
    }

    pub fn bind(&mut self, id: StableNodeId, ancestors: impl Iterator<Item = StableNodeId>) {
        self.remove(id);
        let ancestors: Vec<_> = ancestors.collect();
        for &parent in &ancestors {
            self.dependents.entry(parent).or_default().insert(id);
        }
        self.ancestors.insert(id, ancestors);
    }

    pub fn affected(&self, root: StableNodeId) -> Vec<StableNodeId> {
        if self.dependents.is_empty() {
            return Vec::new();
        }
        self.dependents
            .get(&root)
            .into_iter()
            .flat_map(|nodes| nodes.iter().copied())
            .collect()
    }
}

/// The three layout boxes of a sticky ancestor's result in its own
/// coordinates: its padding box, content box, and scrollable extent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StickyBoxes {
    pub padding_box: LayoutBox,
    pub content_box: LayoutBox,
    pub scroll_extent: LayoutBox,
}

/// One ancestor, nearest first, from the authoritative retained layout.
/// Runtime input and Scene paint feed the same projection with these facts.
pub struct StickyAncestor<'a> {
    pub style: &'a LayoutStyle,
    pub bounds: LayoutBox,
    /// The ancestor's sticky boxes in its own coordinates, its border box at
    /// the origin: placed at `bounds`' origin here.
    pub boxes: Option<StickyBoxes>,
    pub scroll: ScrollOffset,
    pub font_size: f32,
    pub writing: nana_ui_core::WritingContext,
}

/// Local translation before the node's own transform. Only sticky nodes walk
/// their ancestors; no measure, layout write, or component state is involved.
/// Nested sticky placements are evaluated from the scrollport inward in one
/// pass, rather than recursively re-evaluating every ancestor's ancestors.
pub fn sticky_translation<'a>(
    style: &LayoutStyle,
    bounds: LayoutBox,
    font_size: f32,
    node_writing: nana_ui_core::WritingContext,
    ancestors: impl Iterator<Item = StickyAncestor<'a>>,
) -> [f32; 2] {
    if style.position != PositionSpec::Sticky {
        return [0.0; 2];
    }
    let mut chain = Vec::new();
    for ancestor in ancestors {
        let boundary = ancestor.style.overflow_x.clips()
            || ancestor.style.overflow_y.clips()
            || ancestor.style.position == PositionSpec::Fixed;
        chain.push(ancestor);
        if boundary {
            break;
        }
    }
    let Some(root) = chain.pop() else {
        return [0.0; 2];
    };
    let viewport = root
        .boxes
        .map_or(root.bounds, |boxes| at_bounds(&root, boxes.padding_box));
    let mut containing = sticky_containing_box(&root);
    let mut writing = root.writing;
    let mut scroll = [root.scroll.x, root.scroll.y];
    let mut inherited = [0.0; 2];
    for ancestor in chain.into_iter().rev() {
        if ancestor.style.position == PositionSpec::Sticky {
            let delta = project_sticky(
                ancestor.style,
                ancestor.bounds,
                ancestor.font_size,
                ancestor.writing,
                containing,
                viewport,
                [scroll[0] - inherited[0], scroll[1] - inherited[1]],
                writing,
            );
            inherited[0] += delta[0];
            inherited[1] += delta[1];
        }
        scroll[0] += ancestor.scroll.x;
        scroll[1] += ancestor.scroll.y;
        if ancestor.style.display != Some(DisplaySpec::Inline) {
            containing = sticky_containing_box(&ancestor);
            writing = ancestor.writing;
        }
    }
    project_sticky(
        style,
        bounds,
        font_size,
        node_writing,
        containing,
        viewport,
        [scroll[0] - inherited[0], scroll[1] - inherited[1]],
        writing,
    )
}

fn sticky_containing_box(ancestor: &StickyAncestor<'_>) -> LayoutBox {
    ancestor.boxes.map_or(ancestor.bounds, |boxes| {
        at_bounds(
            ancestor,
            if ancestor.style.overflow_x.clips() || ancestor.style.overflow_y.clips() {
                boxes.scroll_extent
            } else {
                boxes.content_box
            },
        )
    })
}

/// `local`, a box of `ancestor`'s result, where the ancestor's box stands.
fn at_bounds(ancestor: &StickyAncestor<'_>, local: LayoutBox) -> LayoutBox {
    crate::layout_result::place_box([ancestor.bounds.x, ancestor.bounds.y], local)
}

fn project_sticky(
    style: &LayoutStyle,
    bounds: LayoutBox,
    font_size: f32,
    node_writing: nana_ui_core::WritingContext,
    containing: LayoutBox,
    viewport: LayoutBox,
    scroll: [f32; 2],
    writing: nana_ui_core::WritingContext,
) -> [f32; 2] {
    let fonts = FontSizeContext::new(nana_ui_core::FontSizeContext::DEFAULT_ROOT_PX, font_size);
    let (margin, offsets) = style.resolved_sticky_edges_in(
        node_writing,
        Some(writing.inline_size(containing.width, containing.height)),
        fonts,
    );
    let inset = |spec, base| LayoutStyle::resolve_inset_fonts(spec, base, fonts);
    let x = sticky_axis(
        bounds.x,
        bounds.width,
        scroll[0],
        viewport.x,
        viewport.width,
        containing.x,
        containing.width,
        margin.left,
        margin.right,
        inset(offsets[3], viewport.width),
        inset(offsets[1], viewport.width),
        writing.physical_axis_reversed(nana_ui_core::FlexDirection::Row),
    );
    let y = sticky_axis(
        bounds.y,
        bounds.height,
        scroll[1],
        viewport.y,
        viewport.height,
        containing.y,
        containing.height,
        margin.top,
        margin.bottom,
        inset(offsets[0], viewport.height),
        inset(offsets[2], viewport.height),
        writing.physical_axis_reversed(nana_ui_core::FlexDirection::Column),
    );
    [x, y]
}

fn sticky_axis(
    origin: f32,
    size: f32,
    scroll: f32,
    viewport: f32,
    extent: f32,
    containing: f32,
    containing_extent: f32,
    margin_start: f32,
    margin_end: f32,
    start: Option<f32>,
    end: Option<f32>,
    reversed: bool,
) -> f32 {
    if start.is_none() && end.is_none() {
        return 0.0;
    }
    let mut first = start.unwrap_or(0.0);
    let mut last = end.unwrap_or(0.0);
    // Oversized boxes reduce the end-edge inset rather than oscillating
    // between incompatible clamps (CSS Positioned Layout, sticky view rect).
    let deficit = (size - (extent - first - last)).max(0.0);
    if reversed {
        first -= deficit;
    } else {
        last -= deficit;
    }
    let mut used = origin;
    if end.is_some() {
        used = used.min(viewport + extent - last + scroll - size);
    }
    if start.is_some() {
        used = used.max(viewport + first + scroll);
    }
    let min = containing + margin_start.min(origin - containing);
    let max = containing + containing_extent
        - size
        - margin_end.min(containing + containing_extent - origin - size);
    used.clamp(min, max) - origin
}
