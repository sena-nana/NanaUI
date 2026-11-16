//! What under a moved root projects again: the pins below it and the
//! boxes that keep no relative bounds.

use super::*;

impl UiWorld {
    /// What under `roots` -- boxes layout moved that hand their subtrees
    /// offsets ([`Self::moved_root_hands_offsets`]) -- projects again
    /// although nothing marked it, found from the pinned boxes alone. A box
    /// on a path from such a root that nothing pins is placed by its offset,
    /// hands its children offsets and cuts nothing: it, and every box under
    /// it that nothing pins, kept its bounds relative to its parent to the
    /// bit ([`Self::offset_placed`]). So each pin under a root with no box
    /// that changes that between them is where
    /// [`Self::reach_moved_subtree`]'s walk would first find anything; the
    /// walk starts there instead. A pinned box on the way that still hands
    /// offsets (a scale, a sticky offset, a box measured from a block that
    /// moved with the root) is passed over: what it holds is placed from it.
    /// A pin below one that hands its children something else (a rotation,
    /// a 3D context, a clip or a modal frame), or below one that stayed
    /// where it was (measured from a frame above the root), is the upper
    /// one's walk to reach, or nothing's; a pin with another moved box
    /// between it and the root is that box's. O(pins × depth), not
    /// O(boxes moved).
    pub(super) fn reach_moved_pins(
        &self,
        roots: &[StableNodeId],
        layout_moved: &[StableNodeId],
        affected: &mut Vec<StableNodeId>,
        pending: &mut Vec<StableNodeId>,
        memo: &mut ProjectionMemo,
    ) {
        let mut open = Vec::new();
        // The blocks boxes on the way up are measured from, until the walk
        // passes them: one it does not pass before the root stayed put.
        let mut frames = Vec::new();
        for pin in self.nodes.space_pins() {
            let Some(record) = self.nodes.get(pin) else {
                continue;
            };
            // The nearest box layout moved above the pin, unless a box on
            // the way stayed where it was, or hands what it holds anything
            // but offsets.
            frames.clear();
            let mut at = record.hierarchy.parent;
            let root = loop {
                let Some(id) = at else {
                    break None;
                };
                frames.retain(|&block| block != id);
                if layout_moved.binary_search(&id).is_ok() {
                    break frames.is_empty().then_some(id);
                }
                let Some(above) = self.nodes.get(id) else {
                    break None;
                };
                match above.layout_frame() {
                    crate::LayoutFrame::Parent => {}
                    crate::LayoutFrame::Block(block) => frames.push(block),
                    crate::LayoutFrame::Page => break None,
                }
                // A pin on the way placed from its viewport box and not
                // upright there walks what it holds itself, this pin among
                // them: passing it only finds this pin twice.
                if above.space.intersects(NodeSpace::CLIPS | NodeSpace::MODAL)
                    || !above.space.hands_offsets()
                {
                    break None;
                }
                at = above.hierarchy.parent;
            };
            let Some(root) = root.filter(|root| roots.binary_search(root).is_ok()) else {
                continue;
            };
            open.push((pin, true));
            self.reach_moved_subtree(
                root,
                &mut open,
                layout_moved,
                false,
                affected,
                pending,
                memo,
            );
        }
    }

    /// The pinned boxes ([`NodeSpace::pins_accessibility`]) under `roots`
    /// -- boxes whose transform changed and gave what they hold the same
    /// space ([`NodeSpace::keeps_space`]), holding their own boxes upright
    /// in theirs ([`Self::moved_root_hands_offsets`]) -- into `pending`,
    /// whose subtrees project again. Everything else under such a root is
    /// placed by its offset, in a space the new transform did not change,
    /// and keeps its bounds to the bit. Unlike a move, nothing under a turn
    /// stays where it stood: a box measured from a block above the root, or
    /// escaping a clip above it, is still carried by its transform, so the
    /// nearest pin below the root is found whatever frame it is measured
    /// from; a pin below another pin is that one's subtree. O(pins × depth).
    pub(super) fn reach_turned_pins(
        &self,
        roots: &[StableNodeId],
        pending: &mut Vec<StableNodeId>,
    ) {
        for pin in self.nodes.space_pins() {
            let Some(record) = self.nodes.get(pin) else {
                continue;
            };
            let mut at = record.hierarchy.parent;
            let reached = loop {
                let Some(id) = at else {
                    break false;
                };
                if roots.binary_search(&id).is_ok() {
                    break true;
                }
                let Some(above) = self.nodes.get(id) else {
                    break false;
                };
                if above.space.pins_accessibility() {
                    break false;
                }
                at = above.hierarchy.parent;
            };
            if reached {
                pending.push(pin);
            }
        }
    }

    /// What under `root`, a box layout moved, projects again although nothing
    /// marked it, into `affected`, and the boxes whose whole subtree does
    /// into `pending`, walking from the boxes in `open` (each with whether
    /// its parent's accessible box is its layout box) or, when `open` is
    /// empty, from `root`'s children. Everything under `root` moved with it
    /// and kept its offset from its parent but for:
    ///
    /// - a box measured from a frame that did not move (a `fixed` box, an
    ///   absolute one whose containing block is above `root`): it stayed on
    ///   the page, so its offset from its parent changed;
    /// - a box that clips what it holds, which is where the viewport-space
    ///   clips of its subtree come from (and a clip visual anywhere): its
    ///   subtree projects again, as a non-carrying seed's does;
    /// - a box not placed by its offset ([`Self::offset_placed`]: its own
    ///   transform or sticky offset, a rotation or a 3D context above, a
    ///   virtual or modal visual): what moved it does not move it by the
    ///   same offset to the bit ([`Self::keeps_relative_bounds`]).
    ///
    /// The walk stops at a box layout moved itself (in `layout_moved`): it
    /// is walked as a root of its own. Below a box that hands its children
    /// offsets it goes on only when `descend_identity` is set: what is there
    /// that can project again is pinned, and found from the pins
    /// ([`Self::reach_moved_pins`]); below one that does not, it checks each
    /// box. The walk reads each box's record and nothing else on the common
    /// path -- no box, no memo.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reach_moved_subtree(
        &self,
        root: StableNodeId,
        open: &mut Vec<(StableNodeId, bool)>,
        layout_moved: &[StableNodeId],
        descend_identity: bool,
        affected: &mut Vec<StableNodeId>,
        pending: &mut Vec<StableNodeId>,
        memo: &mut ProjectionMemo,
    ) {
        let Some(record) = self.nodes.get(root) else {
            open.clear();
            return;
        };
        let special = |record: &NodeRecord| record.space.contains(NodeSpace::VIRTUAL);
        // Whether `block` moved with `root`: it is `root` or under it.
        let inside = |block: StableNodeId| {
            std::iter::successors(Some(block), |&at| self.parent_id(at)).any(|at| at == root)
        };
        if open.is_empty() {
            // Whether `root` holds its box upright in its space: then the
            // bounds of what it holds come from their offsets. Motion
            // overlays can transform any box, so they leave only the exact
            // check; a clip visual sends every box to `pending` below.
            let parent_offsets = self.presentation.is_empty()
                && self.layout_length_tracks.is_empty()
                && !self.is_modal_frame(root)
                && self
                    .accessible_space(root, memo)
                    .is_some_and(|space| space.unit.is_some());
            open.extend(
                record
                    .hierarchy
                    .children
                    .iter()
                    .map(|&child| (child, parent_offsets)),
            );
        }
        while let Some((id, parent_offsets)) = open.pop() {
            let Some(record) = self.nodes.get(id) else {
                continue;
            };
            let follows = match record.layout_frame() {
                crate::LayoutFrame::Parent => true,
                crate::LayoutFrame::Block(block) => inside(block),
                crate::LayoutFrame::Page => false,
            };
            if !follows {
                affected.push(id);
                // What it holds is placed from it: by offset, where it
                // holds its own box upright in its space; otherwise from
                // viewport boxes less its own, which stand where they stood
                // but are summed through a chain the move changed, and do
                // not keep their difference to the bit.
                if !(record.space.hands_offsets()
                    && self
                        .accessible_space(id, memo)
                        .is_some_and(|space| space.unit.is_some()))
                {
                    pending.push(id);
                }
                continue;
            }
            if layout_moved.binary_search(&id).is_ok() {
                continue;
            }
            if self.clip_visuals != 0 || record.resolved_layout.clips_overflow() {
                pending.push(id);
                continue;
            }
            let keeps = if parent_offsets && record.space.places_by_offset() && !special(record) {
                true
            } else {
                self.keeps_relative_bounds(id, memo)
            };
            if !keeps {
                affected.push(id);
            }
            // A box placed by its offset under one that holds its own box
            // upright holds its own upright too, however it turns; one
            // placed from its viewport box, only when that is upright.
            let handed = parent_offsets
                && record.space.hands_offsets()
                && !record.space.contains(NodeSpace::MODAL)
                && (record.space.places_by_offset()
                    || self
                        .accessible_space(id, memo)
                        .is_some_and(|space| space.unit.is_some()));
            if handed && !descend_identity {
                continue;
            }
            open.extend(
                record
                    .hierarchy
                    .children
                    .iter()
                    .map(|&child| (child, handed)),
            );
        }
    }

    /// Whether `id`'s accessible bounds come from its offset
    /// ([`Self::offset_placed`]), or are its layout box and its parent's
    /// are too, so that a box that kept its offset from its parent keeps
    /// them, to the bit. Virtual nodes are placed from its geometry, and a
    /// clip visual cuts the boxes under it: both project again.
    pub(super) fn keeps_relative_bounds(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> bool {
        if self.clip_visuals != 0 {
            return false;
        }
        if self
            .nodes
            .get(id)
            .is_some_and(|record| record.space.contains(NodeSpace::VIRTUAL))
        {
            return false;
        }
        if let Some(record) = self.nodes.get(id)
            && self.offset_placed(id, record, memo)
        {
            return true;
        }
        // Placed from the viewport in a framed space, or handing a frame of
        // its own from there: its bounds are page positions through maps,
        // which a move changes in the last bit.
        let parent = self.nodes.get(id).and_then(|node| node.hierarchy.parent);
        if [Some(id), parent].into_iter().flatten().any(|node| {
            self.accessible_space(node, memo)
                .is_some_and(|space| space.framed)
        }) {
            return false;
        }
        // Under a parent that scrolls, its bounds are its viewport box less
        // the parent's, in the space the parent's scroll moves: a scroll
        // changes them where its viewport box stays (a stuck `sticky` box).
        // That includes a scroll back to the origin, which leaves no offset
        // behind to tell by: a scroll container is enough.
        if parent.is_some_and(|parent| {
            self.nodes.get(parent).is_some_and(|record| {
                record.resolved_layout.clips_overflow()
                    || self.nodes.scroll_offset(parent) != ScrollOffset::default()
            })
        }) {
            return false;
        }
        [Some(id), parent].into_iter().flatten().all(|node| {
            !self.is_modal_frame(node)
                && (self.axis_aligned(node, memo)
                    || self.visible_accessibility_bounds(node, memo)
                        == self.component_layout_box(node))
        })
    }
}
