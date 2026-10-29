//! The clip chain (#201): which ancestors' clips and scroll reach a box.
//!
//! One rule decides it, and [`crate::LayoutResult::clip_parent`] /
//! [`crate::LayoutResult::clip`] publish it. A box takes clips and scroll
//! from its parent, except an absolute box whose containing block lies
//! outside overflow containers between them: CSS has an `overflow`
//! container clip and scroll a descendant only when it is the descendant's
//! containing block or an ancestor of it (CSS Overflow 3, CSS 2.1
//! §11.1.1), so that box hangs from the parent of the outermost container
//! it escapes. A `fixed` box hangs from nothing. Scene extraction, hit
//! testing and accessibility all follow this link, so a box is clipped and
//! scrolled the same way in paint, pointer input and the accessible tree.

use super::*;

impl UiWorld {
    /// `id`'s position as containing blocks and the clip chain read it: an
    /// item a triggered menu places is `fixed`, as its effective style says.
    pub(super) fn clip_position(&self, id: StableNodeId) -> PositionSpec {
        if self.nodes.has_visuals() && self.parent_triggered_overlay(id).is_some() {
            return PositionSpec::Fixed;
        }
        self.nodes.get(id).map_or(PositionSpec::Static, |record| {
            record.resolved_layout.position
        })
    }

    fn clips_overflow(&self, id: StableNodeId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|record| record.resolved_layout.clips_overflow())
    }

    /// The node `id` takes clips and scroll from, computed: see
    /// [`crate::LayoutResult::clip_parent`].
    pub(super) fn clip_parent_of(&self, id: StableNodeId) -> Option<StableNodeId> {
        let position = self.clip_position(id);
        if position == PositionSpec::Fixed {
            return None;
        }
        let parent = self.parent_id(id)?;
        if position != PositionSpec::Absolute {
            return Some(parent);
        }
        // The ancestors strictly between the box and its containing block
        // (the nearest positioned ancestor, or the document for the initial
        // block) are static; the outermost of them that clips is the last
        // container the box escapes.
        let mut outermost = None;
        let mut ancestor = Some(parent);
        while let Some(id) = ancestor {
            if self.is_document_node(id) || self.clip_position(id).establishes_containing_block() {
                break;
            }
            if self.clips_overflow(id) {
                outermost = Some(id);
            }
            ancestor = self.parent_id(id);
        }
        match outermost {
            Some(container) => self.parent_id(container),
            None => Some(parent),
        }
    }

    /// The nearest ancestor whose overflow clip reaches `id`, computed: see
    /// [`crate::LayoutResult::clip`].
    pub(super) fn overflow_clip_of(&self, id: StableNodeId) -> Option<StableNodeId> {
        let mut cursor = self.clip_parent_of(id);
        while let Some(id) = cursor {
            if self.clips_overflow(id) {
                return Some(id);
            }
            cursor = self.clip_parent_of(id);
        }
        None
    }

    /// The node `id` takes clips and scroll from: what its published
    /// [`crate::LayoutResult::clip_parent`] says, or the same rule before a
    /// result is published.
    pub fn layout_clip_parent(&self, id: StableNodeId) -> Option<StableNodeId> {
        match self.layout_result(id) {
            Some(result) => result.clip_parent,
            None => self.clip_parent_of(id),
        }
    }

    /// `id`'s clip chain, nearest first: every ancestor whose overflow clip
    /// and scroll reach it, read from the published results.
    pub fn layout_clip_chain(&self, id: StableNodeId) -> impl Iterator<Item = StableNodeId> + '_ {
        let clip = |id| match self.layout_result(id) {
            Some(result) => result.clip,
            None => self.overflow_clip_of(id),
        };
        std::iter::successors(clip(id), move |id| clip(*id))
    }

    /// The nodes whose clips and scroll reach `id`, nearest first: its clip
    /// parent, that node's clip parent, and so on, read from the published
    /// results. It ends at a `fixed` box, which hangs from nothing. A walk
    /// that cuts what `id` shows by the boxes above it (an open menu's room,
    /// a component's chrome clip) follows this, not the tree, so it agrees
    /// with paint and hit testing about which boxes those are.
    pub(crate) fn clip_ancestors(
        &self,
        id: StableNodeId,
    ) -> impl Iterator<Item = StableNodeId> + '_ {
        std::iter::successors(self.layout_clip_parent(id), move |node| {
            self.layout_clip_parent(*node)
        })
    }

    /// Where `id` hangs when it escapes overflow containers on the way to
    /// its containing block: `Some(anchor)`, the clip parent it takes in
    /// place of its parent (`Some(None)`: no node, the box is a root of its
    /// own). `None` when it hangs from its parent or is `fixed`.
    pub(crate) fn clip_escape(&self, id: StableNodeId) -> Option<Option<StableNodeId>> {
        if self.clip_position(id) != PositionSpec::Absolute {
            return None;
        }
        let anchor = self.layout_clip_parent(id);
        (anchor != self.parent_id(id)).then_some(anchor)
    }

    /// The transforms of the ancestors an escaped box passes over, from just
    /// below `anchor` down to `id`'s parent, composed onto `base` (the
    /// anchor's transform for its children): what `id` inherits in place of
    /// its parent's child transform. `presented` reads compositor overlays
    /// as hit testing under them does.
    pub(super) fn clip_bridge_transform(
        &self,
        id: StableNodeId,
        anchor: Option<StableNodeId>,
        base: ([f32; 6], [f32; 2]),
        presented: bool,
    ) -> ([f32; 6], [f32; 2]) {
        let mut between = Vec::new();
        let mut cursor = self.parent_id(id);
        while let Some(node) = cursor {
            if Some(node) == anchor {
                break;
            }
            between.push(node);
            cursor = self.parent_id(node);
        }
        let mut transform = base;
        for node in between.into_iter().rev() {
            let Some(bounds) = self.component_layout_box(node) else {
                continue;
            };
            let style = self.hit_motion_layout(node);
            let local = if presented {
                self.input_local_scene_transform(node, &style, bounds, false)
            } else {
                style
                    .world_scene_transform(bounds.x, bounds.y, bounds.width, bounds.height)
                    .unwrap_or((IDENTITY_AFFINE, [0.0, 0.0]))
            };
            transform = then_hit(transform, self.with_sticky_translation(node, local));
        }
        transform
    }
}
