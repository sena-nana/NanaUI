//! Rebuilding moved entries in place, and the scoped rebuild that chooses
//! between that and a patch.

use super::*;
use crate::world::page_origins::DIRECT_DEPTH;

impl UiWorld {
    /// `id`'s entry built again on its own, when it hands its children what
    /// the one held does: the same transform and scroll, no perspective or
    /// 3D context, over the same children, from its tree parent. Such an
    /// entry hands its children nothing a change of it could alter, so they
    /// keep theirs; anything else patches the subtree. An entry that hands
    /// its children its own layout coordinates ([`HitEntry::local_children`])
    /// hands them its scroll alone, so its transform may change too.
    ///
    /// It is built as [`Self::patch_hit_branch`] would build it -- from the
    /// parent's live transform and scroll, brought back into the space the
    /// index holds it in by the scroll shifts above it -- so under a scroll
    /// that space is not the live one, and the transform is compared there.
    ///
    /// `skips` carries the unshifted entries above deep parents from one
    /// entry to the next ([`HitIndex::inherited_shift_skipping`]).
    ///
    /// `scrolled`: nothing but a scroll changed since the entry was held
    /// (a sticky box a scroll moves), so nothing above it changed in that
    /// space either. The transform built against the live scroll and
    /// brought back by the shifts is then the held one up to their
    /// rounding: the held one stays, with the clips taken with it, so that
    /// what the entry holds keeps the space it was built in.
    pub(super) fn rebuilt_hit_entry_in_place(
        &self,
        document: DocumentId,
        id: StableNodeId,
        scrolled: bool,
        skips: &mut ShiftSkips,
    ) -> Option<HitEntry> {
        let index = self.hit_index.hit_test_index.get(&document)?;
        let hit = index.entries.get(&id)?;
        let held = &hit.entry;
        // A build seeded at a box does not look for the clip parent it
        // escapes to (#201): the branch patch hangs it there.
        // Under a scale, rotation or skew an entry's bounds are the box
        // around turned corners, merged with its children's: built again
        // over children kept from another build they come out an ulp from a
        // fresh one. Such branches patch whole, as they did.
        // An entry that hands its children its own layout coordinates hands
        // them nothing of its transform, which may then be anything 2D.
        if held.escaped
            || held.persp != [0.0, 0.0]
            || (!held.local_children && held.transform[..4] != [1.0, 0.0, 0.0, 1.0])
            || hit.parent != self.parent_id(id)
            || self.clip_escape(id).is_some()
        {
            return None;
        }
        // What the parent hands its children, live: its entry's transform
        // with the scroll shifts above it (or, when it hands them its own
        // coordinates, nothing of it), then its own scroll; and the shift
        // the index holds them at.
        let (seed, offset) = match hit.parent {
            Some(parent) => {
                if index.entries.get(&parent)?.entry.persp != [0.0, 0.0] {
                    return None;
                }
                let record = self.record(parent);
                // A shallow walk up costs less than remembering it.
                let skips = (record.depth > DIRECT_DEPTH).then_some(&mut *skips);
                index.live_child_seed(parent, self.nodes.scroll_offset(parent), skips)?
            }
            None => (IDENTITY_AFFINE, [0.0, 0.0]),
        };
        // A 3D context changes what the children are built from without
        // showing in this entry; the seed does not carry one either.
        if self.record(id).space.depth() {
            return None;
        }
        let (mut built, deferred) =
            self.build_hit_entries_reaching(vec![(id, seed, hit.parent)], false);
        if !deferred.is_empty() || built.len() != 1 {
            return None;
        }
        let mut fresh = built.pop()?.entry;
        // Into the space the index holds the entry in, as `insert_tree`
        // takes a patched branch there.
        fresh.transform[4] -= offset[0];
        fresh.transform[5] -= offset[1];
        for (_, transform) in fresh
            .self_clips
            .iter_mut()
            .chain(fresh.child_clips.iter_mut())
        {
            transform[4] -= offset[0];
            transform[5] -= offset[1];
        }
        if scrolled && !held.local_children && fresh.transform[..4] == held.transform[..4] {
            let back = [
                held.transform[4] - fresh.transform[4],
                held.transform[5] - fresh.transform[5],
            ];
            if back != [0.0, 0.0] {
                fresh.transform = held.transform;
                for (_, transform) in fresh
                    .self_clips
                    .iter_mut()
                    .chain(fresh.child_clips.iter_mut())
                {
                    transform[4] += back[0];
                    transform[5] += back[1];
                }
            }
        }
        // A box that turns fixed hangs from the viewport, and one whose
        // stacking moved takes another place among its siblings: both are
        // structural, for the subtree patch.
        if fresh.escaped
            || fresh.persp != [0.0, 0.0]
            || fresh.local_children != held.local_children
            || (!held.local_children && fresh.transform != held.transform)
            || fresh.scroll != held.scroll
            || !Arc::ptr_eq(&fresh.source_children, &held.source_children)
            || fresh.z_index != held.z_index
            || self
                .layout_style(id)
                .is_none_or(|style| style.position == PositionSpec::Fixed)
        {
            return None;
        }
        fresh.order = held.order;
        Some(fresh)
    }

    /// Put the entries built again on their own (see
    /// [`Self::rebuilt_hit_entry_in_place`]) in place over their children, and
    /// refresh the bounds above them once each. An entry that only moved
    /// takes a new origin this way; what hangs below it keeps its own.
    pub(super) fn replace_hit_entries_in_place(
        &mut self,
        document: DocumentId,
        rebuilt: Vec<(StableNodeId, HitEntry)>,
    ) -> Vec<StableNodeId> {
        if rebuilt.is_empty() {
            return Vec::new();
        }
        let Some(index) = self.hit_index.hit_test_index.get_mut(&document) else {
            return Vec::new();
        };
        let mut touched: Vec<StableNodeId> = Vec::with_capacity(rebuilt.len());
        let mut moved = 0usize;
        let built = rebuilt.len();
        for (id, mut entry) in rebuilt {
            if let Some(hit) = index.entries.get_mut(&id) {
                if entry.origin != hit.entry.origin {
                    moved += 1;
                }
                if entry.pinned {
                    index.pinned.insert(id);
                }
                entry.children = std::mem::take(&mut hit.entry.children);
                hit.entry = entry;
                touched.push(id);
            }
        }
        index.refresh_bounds_of(&touched);
        self.bump_last_counters(|counters| counters.hit_test_entries_moved += moved);
        self.record_hit_test_nodes_built(built);
        touched
    }

    /// The pinned entries ([`HitEntry::pinned`]) below a node of `dirty`
    /// and not in it: they read geometry a change above them can move,
    /// so they are built again although nothing marked them.
    pub(super) fn pinned_hit_entries_under(
        &mut self,
        document: DocumentId,
        dirty: &[StableNodeId],
    ) -> Vec<StableNodeId> {
        let Some(index) = self.hit_index.hit_test_index.get_mut(&document) else {
            return Vec::new();
        };
        let HitIndex {
            pinned, entries, ..
        } = index;
        pinned.retain(|id| entries.get(id).is_some_and(|hit| hit.entry.pinned));
        if pinned.is_empty() {
            return Vec::new();
        }
        let dirty: crate::NodeSet = dirty.iter().copied().collect();
        let index = &self.hit_index.hit_test_index[&document];
        index
            .pinned
            .iter()
            .copied()
            .filter(|&id| {
                !dirty.contains(&id)
                    && self.contains(id)
                    && std::iter::successors(self.parent_id(id), |&at| self.parent_id(at))
                        .any(|at| dirty.contains(&at))
            })
            .collect()
    }

    /// Rebuild only the subtrees covering `dirty` instead of the whole document.
    ///
    /// Returns `false` when the change cannot be expressed as a local splice and
    /// the caller must fall back to [`Self::rebuild_hit_test`]. Structural cases
    /// that escalate: no existing index for the document, a dirty node whose
    /// parent chain is not represented in the index while still being live, and
    /// a dirty document root that changed root membership.
    pub fn rebuild_hit_test_scoped(
        &mut self,
        document: DocumentId,
        dirty: &[StableNodeId],
    ) -> bool {
        // Styles stay as they are while the index is patched: the pointer
        // events each deep node uses are remembered for the whole rebuild.
        let previous = self
            .hit_index
            .pointer_events_memo
            .replace(Some(NodeMap::default()));
        let rebuilt = self.rebuild_hit_test_scoped_remembering(document, dirty);
        self.hit_index.pointer_events_memo.replace(previous);
        rebuilt
    }

    fn rebuild_hit_test_scoped_remembering(
        &mut self,
        document: DocumentId,
        dirty: &[StableNodeId],
    ) -> bool {
        if dirty.is_empty() {
            return true;
        }
        if !self.hit_index.hit_test_index.contains_key(&document) {
            return false;
        }
        // An entry that hands its children what it handed them is built
        // again on its own, in place; the rest patch the subtrees over them. Entries below a dirty one keep theirs,
        // being in their own boxes' coordinates, but for the pinned.
        let mut rest = self.pinned_hit_entries_under(document, dirty);
        let mut rebuilt = Vec::new();
        let mut skips = ShiftSkips::default();
        for &id in dirty {
            if let Some(entry) = self.rebuilt_hit_entry_in_place(document, id, false, &mut skips) {
                rebuilt.push((id, entry));
            } else {
                rest.push(id);
            }
        }
        let _replaced = self.replace_hit_entries_in_place(document, rebuilt);
        let Some(roots) = self.minimal_hit_patch_roots(document, &rest) else {
            return false;
        };
        for root in roots {
            if !self.patch_hit_subtree(document, root) {
                return false;
            }
        }
        // Each entry of `dirty` and the pinned ones was built again, in
        // place or under a patched root.
        self.settle_non_scroll_hit_dirty(dirty.iter().chain(&rest).copied());
        // Once every patch is in: an entry replaced early sits over others
        // the patches after it build again.
        // The checks walk every entry in full; the walks the rebuild took
        // are what `deep_walk_tests` counts.
        // The eligibility audit exists under `debug_assertions` alone, as at
        // its other sites: a release test build checks the moved entries.
        #[cfg(test)]
        crate::world::page_origins::steps::uncounted(|| {
            self.check_moved_hit_entries(document, &_replaced);
            #[cfg(debug_assertions)]
            self.audit_hit_eligibility("scoped rebuild");
        });
        #[cfg(all(not(test), feature = "layout-verify"))]
        self.check_moved_hit_entries(document, &_replaced);
        #[cfg(all(not(test), debug_assertions))]
        self.audit_hit_eligibility("scoped rebuild");
        true
    }
}
