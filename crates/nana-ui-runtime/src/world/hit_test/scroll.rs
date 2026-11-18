//! Scroll offsets applied to the index as shifts, without a rebuild.

use super::*;

impl UiWorld {
    /// Pre-compose a scroll translation onto descendant hit entries of
    /// `scroller`. The scroller chrome stays un-scrolled — rebuild applies
    /// scroll only when walking children. Equivalent to a rebuild because
    /// scroll changes nothing else about the entries (membership, order,
    /// z-index, and clips are scroll-invariant: the scroller's own clip never
    /// includes its scroll offset).
    pub fn update_hit_test_scroll(
        &mut self,
        document: DocumentId,
        scroller: StableNodeId,
        delta: [f32; 2],
    ) {
        // Entries the scroll rewrites, clips with them, are what the hit
        // index's build sentinel adds while this runs.
        let built = |world: &Self| {
            world
                .counters
                .last_counters
                .hit_test_nodes_rebuilt
                .unwrap_or(0)
        };
        let before = built(self);
        self.shift_hit_test_scroll(document, scroller, delta);
        let rewritten = built(self).saturating_sub(before);
        if rewritten != 0 {
            self.bump_last_counters(|counters| counters.scroll_clip_updates += rewritten);
        }
    }

    fn shift_hit_test_scroll(
        &mut self,
        document: DocumentId,
        scroller: StableNodeId,
        delta: [f32; 2],
    ) {
        let Some(index) = self.hit_test_index.get_mut(&document) else {
            return;
        };
        let Some(node) = index.entries.get_mut(&scroller) else {
            return;
        };
        if node.entry.persp != [0.0, 0.0] {
            self.rebuild_hit_test(document);
            return;
        }
        // In the space its children are held in: its own layout coordinates
        // when it hands them those.
        let [a, b, c, d, _, _] = if node.entry.local_children {
            IDENTITY_AFFINE
        } else {
            node.entry.transform
        };
        node.shift[0] += a * delta[0] + c * delta[1];
        node.shift[1] += b * delta[0] + d * delta[1];
        node.entry.scroll[0] -= delta[0];
        node.entry.scroll[1] -= delta[1];
        index.shifted = true;
        index.refresh_bounds(scroller);
        // Ordinary descendants share one lazy scroll shift. Sticky boxes
        // additionally change their offset: each is built again against
        // current layout. Its offset is part of its origin, and what it
        // holds is in its own box's coordinates, so the sticky entry alone
        // is replaced in place ([`Self::rebuilt_hit_entry_in_place`]), all
        // of them in one pass whose bounds refresh once; a branch holding a
        // pinned entry, or one that cannot be (under a perspective the
        // offset is in its transform), is patched whole, outermost first.
        let mut sticky = self.sticky_nodes.affected(scroller);
        sticky.compact_sort_by_key(|id| {
            std::iter::successors(self.parent_id(*id), |id| self.parent_id(*id)).count()
        });
        let pinned = self.pinned_hit_entries_under(document, &sticky);
        let mut skips = ShiftSkips::default();
        let (mut rebuilt, mut patched) = (Vec::new(), Vec::new());
        for &id in &sticky {
            let entry = (!pinned
                .iter()
                .any(|&pin| self.is_descendant_or_self(pin, id)))
            .then(|| self.rebuilt_hit_entry_in_place(document, id, true, &mut skips))
            .flatten();
            match entry {
                Some(entry) => rebuilt.push((id, entry)),
                None => patched.push(id),
            }
        }
        let _replaced = self.replace_hit_entries_in_place(document, rebuilt);
        for id in patched {
            if !self.patch_hit_subtree(document, id) {
                self.rebuild_hit_test(document);
                return;
            }
        }
        // Once every patch is in: one may build again what an entry
        // replaced in place holds.
        #[cfg(any(test, feature = "layout-verify"))]
        self.check_moved_hit_entries(document, &_replaced);
        self.settle_non_scroll_hit_dirty(sticky);
    }
}

impl UiWorld {
    /// Whether every input-dirty node is a recorded scroller whose entry
    /// nothing but its scroll changed. Ordinary scroll dirties only that
    /// container; descendant dirtiness means additional input geometry
    /// changed (for example a frozen row's transform), and a scroller in
    /// `non_scroll_hit_dirty` had its own box, transform or membership
    /// changed too, which a scroll shift does not carry.
    /// When true, the frame driver
    /// can patch the hit index in place instead of rebuilding the document.
    pub fn hit_test_work_is_scroll_only(
        &self,
        input: &[StableNodeId],
        updates: &[(StableNodeId, [f32; 2])],
    ) -> bool {
        !updates.is_empty()
            && input.iter().all(|node| {
                updates.iter().any(|(scroller, _)| *scroller == *node)
                    && !self.non_scroll_hit_dirty.contains(node)
            })
    }

    /// Forget the non-scroll input changes of `ids`, whose entries were just
    /// built again from current geometry. One marked again since its work
    /// was taken keeps its record: that change is the next frame's.
    pub(super) fn settle_non_scroll_hit_dirty(
        &mut self,
        ids: impl IntoIterator<Item = StableNodeId>,
    ) {
        if self.non_scroll_hit_dirty.is_empty() {
            return;
        }
        for id in ids {
            if !self.input_pending(id) {
                self.non_scroll_hit_dirty.remove(&id);
            }
        }
    }

    /// Whether `id` carries an INPUT mark not yet taken as system work.
    pub(super) fn input_pending(&self, id: StableNodeId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|record| record.dirty.intersects(DirtyMask::INPUT))
    }
}

impl UiWorld {
    /// Drain the scroll deltas recorded since the last drain.
    pub fn take_scroll_hit_updates(&mut self) -> Vec<(StableNodeId, [f32; 2])> {
        std::mem::take(&mut self.scroll_hit_updates)
    }
}
