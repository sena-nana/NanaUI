//! The incremental accessibility transaction, and which moved roots hand
//! their subtrees offsets.

use super::*;

impl UiWorld {
    /// Project one complete incremental accessibility transaction, including
    /// tombstones for nodes removed from the retained world.
    pub fn project_accessibility_delta(&self, work: &SystemWork) -> AccessibilityDelta {
        // Scrolling and transforms leave LayoutBox unchanged and may carry no
        // ACCESSIBILITY dirty bit. Their hit-test subtrees nevertheless moved
        // in viewport space, so the native accessibility cache must see them.
        //
        // Scheduled-layout nodes are deliberately NOT seeds, for the same
        // reason `RuntimeDocument::apply_hit_test_work` refuses them: layout
        // invalidation propagates to ancestors, so any leaf resize puts the
        // document root in `work.layout_frontier_seeds`, and expanding a root seed walks the
        // whole document. The nodes whose box actually moved are already here
        // by a different route -- layout writeback commits `WriteLayout`,
        // which marks INPUT | RENDER | ACCESSIBILITY on exactly those nodes,
        // shifted descendants included.
        //
        // Bounds are relative to the parent's, so a box layout moved with its
        // parent keeps them, and nothing marks it: only the box layout wrote
        // with a new origin is listed (`layout_moved`, `accessibility_moved`).
        // Such a node is no subtree seed; of what it holds, only what did not
        // move with it, or does not keep its offset to the bit, projects
        // again (`reach_moved_subtree`).
        //
        // These sets exist to produce a sorted, de-duplicated id sequence. A
        // `BTreeSet` pays a tree insert and node allocation per id to do that;
        // sorting a flat vec once yields the identical sequence for far less.
        #[cfg(any(test, feature = "layout-verify"))]
        self.nodes.check_node_spaces();
        // A subtree mounted deep in the page sums its depth once.
        let _origins = self.remember_page_origins();
        let mut affected = work.accessibility.clone();
        // A node named by another one's text is projected again with it.
        let named = affected
            .iter()
            .filter_map(|id| self.labels.get(id))
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        affected.extend(named);
        let moved = |id: &StableNodeId| work.accessibility_moved.binary_search(id).is_ok();
        // A plain tree has no clips: its styles are all the default.
        let tree_plain =
            affected.len() + work.input_hit_test.len() >= 64 && self.accessibility_tree_is_plain();
        // A box that clips what it holds, or any box under a clip visual,
        // cuts boxes that need not move with it: its whole subtree projects
        // again -- unless layout left the box where and as big as it was (a
        // scroller whose content grew), so it cuts them as before.
        let reshaped = |id: &StableNodeId| work.layout_reshaped.binary_search(id).is_ok();
        let carries = |id: &StableNodeId| {
            moved(id)
                && (tree_plain
                    || (self.clip_visuals == 0 && (!self.clips_overflow(*id) || !reshaped(id))))
        };
        // A box whose transform changed and gave what it holds the same
        // space: it projects again, and of what it holds only the boxes
        // placed from the viewport (found from the pins), where it holds
        // its own box upright in its space, cuts nothing and nothing else
        // marked it. Any other projects its subtree again.
        let kept_turn = |id: &StableNodeId| work.transform_kept.binary_search(id).is_ok();
        // A box only its own scroll marked (layout neither moved nor
        // reshaped it, its transform is as it was): what it holds is placed
        // in its space moved by its scroll, so only it, and what does not
        // scroll with it to the bit, projects again.
        let scrolled = |id: &StableNodeId| {
            work.accessibility_scrolled.binary_search(id).is_ok()
                && !kept_turn(id)
                && work.layout_moved.binary_search(id).is_err()
                && work.layout_reshaped.binary_search(id).is_err()
                && !work.transform.contains(id)
        };
        let (mut moved_seeds, mut scroll_seeds, mut pending) = (Vec::new(), Vec::new(), Vec::new());
        for &id in &work.input_hit_test {
            if carries(&id) && !kept_turn(&id) {
                moved_seeds.push(id);
            } else if scrolled(&id) {
                scroll_seeds.push(id);
            } else {
                pending.push(id);
            }
        }
        let moved_seeds = moved_seeds;
        pending.extend(work.transform.iter().filter(|id| !kept_turn(id)));
        let mut memo = ProjectionMemo::default();
        let mut turn_roots = Vec::new();
        for &id in &work.transform_kept {
            if moved(&id)
                && work.layout_moved.binary_search(&id).is_err()
                && !self.clips_overflow(id)
                && self.moved_root_hands_offsets(id, &mut memo)
                && self.style_frames_its_space(id, &mut memo)
            {
                turn_roots.push(id);
            } else {
                pending.push(id);
            }
        }
        if !turn_roots.is_empty() {
            pending.retain(|id| turn_roots.binary_search(id).is_err());
            affected.extend(&turn_roots);
            self.reach_turned_pins(&turn_roots, &mut pending);
        }
        // A scrolled box projects again: its scroll and scroll axes changed.
        affected.extend(&scroll_seeds);
        // A plain tree places every box in flow, from its parent, and has
        // no clip, transform or scroll: what a moved box holds keeps its
        // bounds.
        let mut scroll_roots = Vec::new();
        if !tree_plain {
            // What a scroll carries keeps its bounds where its box stands
            // upright in its space (its children's offsets are then in its
            // space moved by its scroll), but for the pins under it: found
            // from those, as under a box layout moved -- the boxes a scroll
            // leaves where they stood (`fixed`, an absolute box escaping it),
            // `sticky` ones and what they hold, the clips under it. Any
            // other box's children are placed from their viewport boxes: its
            // subtree projects again.
            for &id in &scroll_seeds {
                if self.moved_root_hands_offsets(id, &mut memo) {
                    scroll_roots.push(id);
                } else {
                    pending.push(id);
                }
            }
            if !scroll_roots.is_empty() {
                scroll_roots.sort_unstable();
                // A scroller inside another one that scrolled moved on the
                // viewport, where the clips of what it holds are: its subtree
                // projects again.
                if scroll_roots.len() > 1 {
                    let nested = scroll_roots
                        .iter()
                        .copied()
                        .filter(|&id| {
                            std::iter::successors(self.parent_id(id), |&at| self.parent_id(at))
                                .any(|at| scroll_roots.binary_search(&at).is_ok())
                        })
                        .collect::<Vec<_>>();
                    scroll_roots.retain(|id| !nested.contains(id));
                    pending.extend(nested);
                }
                let (mut pinned, mut pinned_subtrees) = (Vec::new(), Vec::new());
                self.reach_moved_pins(
                    &scroll_roots,
                    &scroll_roots,
                    &mut pinned,
                    &mut pinned_subtrees,
                    &mut memo,
                );
                #[cfg(any(test, feature = "layout-verify"))]
                self.check_moved_pins(&scroll_roots, &scroll_roots, &pinned, &pinned_subtrees);
                affected.extend(pinned);
                pending.extend(pinned_subtrees);
            }
            // Moved boxes whose subtree places its boxes by their offsets
            // all the way down, but where something pins them: those are
            // found from the pins, not by walking what the box holds.
            let mut pin_roots = Vec::new();
            // Boxes resized where they stood whose own transform turns
            // about a centre the new size moved: their space moved, as a
            // moved box's does, while what they hold kept its offsets in
            // it. They are walked as moved boxes are, and moved boxes that
            // are walked box by box stop at them: those walks wait for the
            // list.
            let mut resized_roots = Vec::new();
            let mut walked = Vec::new();
            for &id in &moved_seeds {
                if work.layout_moved.binary_search(&id).is_ok() {
                    if self.moved_root_hands_offsets(id, &mut memo) {
                        pin_roots.push(id);
                    } else {
                        walked.push(id);
                    }
                } else if let Some(node) = self.nodes.get(id) {
                    let space = node.space;
                    // Its own transform turns about a centre the new size
                    // moved (a 2D transform, a `transform_3d` or a
                    // perspective): everything it holds moved on the
                    // viewport, as under a moved box.
                    let turns_about_its_size = space.transformed()
                        || space.intersects(NodeSpace::TRANSFORM_3D | NodeSpace::PERSPECTIVE);
                    if turns_about_its_size {
                        resized_roots.push(id);
                        if space.transformed() && self.moved_root_hands_offsets(id, &mut memo) {
                            // What it holds keeps its offsets in its space:
                            // only the pins under it can change.
                            pin_roots.push(id);
                        } else {
                            // Placed from viewport boxes (a motion overlay,
                            // a clip visual, a 3D context, a modal frame):
                            // what it holds is checked box by box, all the
                            // way down, not only its children.
                            walked.push(id);
                        }
                        continue;
                    }
                    // Resized where it stood: what it holds stays on the
                    // page, unless its box is turned or projected in the
                    // viewport, so its accessible box is the box around
                    // turned corners, whose origin the new size moves.
                    if !self.records_hand_offsets(id, node, &mut memo)
                        || self
                            .accessible_space(id, &mut memo)
                            .is_none_or(|space| space.unit.is_none())
                    {
                        affected.extend(
                            node.hierarchy
                                .children
                                .iter()
                                .copied()
                                .filter(|child| !moved(child)),
                        );
                    }
                }
            }
            let walk_roots = if resized_roots.is_empty() {
                std::borrow::Cow::Borrowed(&work.layout_moved)
            } else {
                let mut roots = work.layout_moved.clone();
                roots.extend(&resized_roots);
                roots.sort_unstable();
                roots.dedup();
                std::borrow::Cow::Owned(roots)
            };
            for id in walked {
                let mut open = Vec::new();
                self.reach_moved_subtree(
                    id,
                    &mut open,
                    &walk_roots,
                    false,
                    &mut affected,
                    &mut pending,
                    &mut memo,
                );
            }
            if !pin_roots.is_empty() {
                pin_roots.sort_unstable();
                let (mut pinned, mut pinned_subtrees) = (Vec::new(), Vec::new());
                self.reach_moved_pins(
                    &pin_roots,
                    &walk_roots,
                    &mut pinned,
                    &mut pinned_subtrees,
                    &mut memo,
                );
                #[cfg(any(test, feature = "layout-verify"))]
                self.check_moved_pins(&pin_roots, &walk_roots, &pinned, &pinned_subtrees);
                affected.extend(pinned);
                pending.extend(pinned_subtrees);
            }
        }
        // `visited` is membership-only, so it does not need ordering at all.
        let mut visited = hashbrown::HashSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            affected.push(id);
            if let Some(node) = self.nodes.get(id) {
                pending.extend(node.hierarchy.children.iter().copied());
            }
        }
        affected.sort_unstable();
        affected.dedup();
        // A moved box keeps the bounds it was handed only where they are its
        // layout box, and its parent's are too: what a transform or a scroll
        // above makes of a box is not moved with it to the bit.
        let kept = moved_seeds
            .iter()
            .copied()
            .filter(|id| affected.binary_search(id).is_err())
            .collect::<Vec<_>>();
        if !tree_plain && !kept.is_empty() {
            let reprojected = kept
                .iter()
                .copied()
                .filter(|&id| !self.keeps_relative_bounds(id, &mut memo))
                .collect::<Vec<_>>();
            if !reprojected.is_empty() {
                affected.extend(reprojected);
                affected.sort_unstable();
            }
        }
        #[cfg(any(test, feature = "layout-verify"))]
        {
            let mut seeds = moved_seeds.clone();
            seeds.extend(&turn_roots);
            seeds.extend(&scroll_roots);
            self.check_kept_accessibility(&seeds, &affected);
        }
        let mut removed = work.accessibility_removals.clone();
        let plain = affected.len() >= 64 && tree_plain;
        // Each projected node memoizes its map, its box and its chain's
        // answers for its children: sized once, the tables do not rehash
        // as a thousand moved rows fill them.
        if !plain {
            memo.reserve(affected.len());
        }
        let mut updated = Vec::with_capacity(affected.len());
        for id in affected {
            let entries = self.project_accessibility_entries(id, plain, true, &mut memo);
            if entries.is_empty() {
                if self.nodes.contains(id) {
                    removed.push(id);
                }
            } else {
                updated.extend(entries);
            }
        }
        removed.sort_unstable();
        removed.dedup();
        #[cfg(any(test, feature = "layout-verify"))]
        self.accessibility_shipped.record(&updated, &removed);
        AccessibilityDelta {
            generation: work.generation,
            updated,
            removed,
        }
    }

    /// Whether `root`, a box layout moved, holds its own box upright in its
    /// accessible space ([`AccessibleSpace::unit`]: every box from the page
    /// down to it is placed by its offset, however it turns, or stands
    /// upright in the viewport), and is no modal frame: then of what it
    /// holds only the pinned boxes ([`NodeSpace::pins_accessibility`]) and
    /// what is under those can project differently
    /// ([`Self::reach_moved_pins`]). Motion overlays can transform any box,
    /// and a clip visual cuts every box under it, so either leaves the walk.
    pub(super) fn moved_root_hands_offsets(
        &self,
        root: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> bool {
        if self.clip_visuals != 0
            || !self.presentation.is_empty()
            || !self.layout_length_tracks.is_empty()
            || self.is_modal_frame(root)
        {
            return false;
        }
        // Every box from the page down only translating and scaling by a
        // positive factor: the root's box is upright wherever its space
        // comes from. Read off the records, without the memo, for the
        // thousand rows a reflow moves.
        let upright_chain = self
            .nodes
            .get(root)
            .is_none_or(|record| self.records_upright(root, record, memo));
        #[cfg(any(test, feature = "layout-verify"))]
        if upright_chain {
            assert!(
                self.accessible_space(root, memo)
                    .is_some_and(|space| space.unit.is_some()),
                "{root:?}: an upright chain holds its box upright"
            );
        }
        upright_chain
            || self
                .accessible_space(root, memo)
                .is_some_and(|space| space.unit.is_some())
    }

    /// Whether `record`, `id`'s, and every record above it hand their
    /// children offsets ([`NodeSpace::hands_offsets`]).
    fn records_hand_offsets(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        memo: &mut ProjectionMemo,
    ) -> bool {
        let ProjectionMemo {
            records_offsets,
            chain,
            ..
        } = memo;
        self.records_all(id, record, records_offsets, chain, |record| {
            record.space.hands_offsets()
        })
    }

    /// Whether `record`, `id`'s, and every record above it hand their
    /// children offsets and stand upright ([`NodeSpace::upright`]).
    fn records_upright(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        memo: &mut ProjectionMemo,
    ) -> bool {
        let ProjectionMemo {
            records_upright,
            chain,
            ..
        } = memo;
        self.records_all(id, record, records_upright, chain, |record| {
            record.space.hands_offsets() && record.space.upright()
        })
    }

    /// Whether `holds` holds for `record`, `id`'s, and every record above
    /// it. A deep node remembers its answer in `known` for those below it,
    /// so the nodes of a deep chain each walk a step; a shallow one walks.
    fn records_all(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        known: &mut hashbrown::HashMap<StableNodeId, bool>,
        passed: &mut Vec<StableNodeId>,
        holds: impl Fn(&NodeRecord) -> bool,
    ) -> bool {
        let walked = || {
            std::iter::successors(Some(record), |record| {
                record
                    .hierarchy
                    .parent
                    .and_then(|parent| self.nodes.get(parent))
            })
            .all(|record| {
                #[cfg(test)]
                super::page_origins::steps::step(super::page_origins::steps::Walk::Records);
                holds(record)
            })
        };
        if record.depth <= super::page_origins::DIRECT_DEPTH {
            return walked();
        }
        passed.clear();
        let mut cursor = Some((id, record));
        let all = loop {
            let Some((at, record)) = cursor else {
                break true;
            };
            #[cfg(test)]
            super::page_origins::steps::step(super::page_origins::steps::Walk::Records);
            if let Some(&answer) = known.get(&at) {
                break answer;
            }
            passed.push(at);
            if !holds(record) {
                break false;
            }
            cursor = record
                .hierarchy
                .parent
                .and_then(|parent| Some((parent, self.nodes.get(parent)?)));
        };
        for at in passed.drain(..) {
            known.insert(at, all);
        }
        #[cfg(test)]
        let full = super::page_origins::steps::uncounted(walked);
        #[cfg(all(not(test), feature = "layout-verify"))]
        let full = walked();
        #[cfg(any(test, feature = "layout-verify"))]
        assert_eq!(all, full, "{id:?}: a remembered answer for its chain");
        all
    }

    /// Whether `id`'s space is what [`NodeSpace::keeps_space`] took it to be
    /// from its style alone: its own layout px where its own transform does
    /// more than move it, its parent's (or the viewport's) moved where it
    /// does not. A box whose parent's map is not one it can be placed in (a
    /// 3D context above) hands no frame however it scales: its space is the
    /// viewport's scaled by its factor, which a new factor changes.
    fn style_frames_its_space(&self, id: StableNodeId, memo: &mut ProjectionMemo) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        !record.space.frames_itself()
            || self
                .accessible_space(id, memo)
                .is_some_and(|space| space.framed && space.unit == Some([1.0, 1.0]))
    }

    pub(super) fn is_modal_frame(&self, id: StableNodeId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|record| record.space.contains(NodeSpace::MODAL))
    }
}
