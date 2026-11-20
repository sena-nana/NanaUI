//! Patching a subtree's branch of the index, and withdrawing entries that
//! leave pointer dispatch.

use super::*;

impl UiWorld {
    /// Replace `root`'s entry (and its descendants) in place. Reuses the parent
    /// entry's accumulated transform so no walk from the document root is
    /// needed. Returns `false` when the splice point is missing and the caller
    /// must rebuild the document.
    pub(in crate::world) fn patch_hit_subtree(
        &mut self,
        document: DocumentId,
        root: StableNodeId,
    ) -> bool {
        // A structural branch containing another viewport root spans multiple
        // projection trees. Rebuild only for that case; ordinary fixed-surface
        // edits and ancestor scrolling retain their normal incremental paths.
        if self.hit_test_index.get(&document).is_some_and(|index| {
            index.roots.iter().flatten().any(|id| {
                *id != root
                    && self.parent_id(*id).is_some()
                    && self.is_descendant_or_self(*id, root)
            })
        }) {
            return false;
        }
        // The escaped entries of this branch hang outside it; take them out,
        // so the rebuilt branch places each exactly once (#201).
        self.withdraw_escaped_entries_under(document, root);
        let Some(deferred) = self.patch_hit_branch(document, root) else {
            return false;
        };
        // Escaped boxes the branch reaches but does not hold hang from their
        // clip parent, outside it.
        deferred
            .into_iter()
            .all(|id| self.patch_hit_subtree(document, id))
    }

    /// Remove the entries of `root`'s branch that hang from a clip parent.
    fn withdraw_escaped_entries_under(&mut self, document: DocumentId, root: StableNodeId) {
        let Some(index) = self.hit_test_index.get(&document) else {
            return;
        };
        if index.escaped.is_empty() {
            return;
        }
        let stale: Vec<_> = index
            .escaped
            .iter()
            .copied()
            .filter(|id| {
                *id != root
                    && index.entries.contains_key(id)
                    && self.is_descendant_or_self(*id, root)
            })
            .collect();
        let index = self
            .hit_test_index
            .get_mut(&document)
            .expect("checked above");
        for id in stale {
            index.remove(id);
            index.escaped.remove(&id);
        }
    }

    /// [`Self::patch_hit_subtree`] for `root`'s own branch: `None` when the
    /// document must be rebuilt, otherwise the escaped boxes the branch
    /// reached and left to hang from their clip parent.
    pub(super) fn patch_hit_branch(
        &mut self,
        document: DocumentId,
        root: StableNodeId,
    ) -> Option<Vec<StableNodeId>> {
        if self.hit_motion_layout(root).position == PositionSpec::Fixed {
            let (mut forest, deferred) =
                self.build_hit_forest_deferring(vec![(root, IDENTITY_AFFINE, None)]);
            if forest.len() > 1 {
                return None;
            }
            let entry = forest.pop().map(|mut entry| {
                entry.order = self
                    .parent_id(root)
                    .and_then(|parent| {
                        self.record(parent)
                            .hierarchy
                            .children
                            .iter()
                            .position(|id| *id == root)
                    })
                    .unwrap_or_default();
                sort_hit_children(&mut entry);
                entry
            });
            if let Some(entry) = &entry {
                self.note_hit_nodes_built(std::slice::from_ref(entry));
            }
            let viewport_root = entry.is_some() && self.parent_id(root).is_some();
            let index = self
                .hit_test_index
                .get_mut(&document)
                .expect("existing index");
            index.replace(root, None, entry);
            if viewport_root {
                index.viewport_roots.insert(root);
            } else {
                index.viewport_roots.remove(&root);
            }
            return Some(deferred);
        }
        if let Some(anchor) = self.clip_escape(root) {
            return self.patch_escaped_hit(document, root, anchor?);
        }
        let parent = self.parent_id(root);
        let Some(parent) = parent else {
            // Document roots: membership is owned by `live_document_roots`, so a
            // root entering or leaving the set is a structural change.
            let roots = self.document_roots(document);
            let present = self.hit_test_index.get(&document).is_some_and(|forest| {
                forest
                    .entries
                    .get(&root)
                    .is_some_and(|entry| entry.parent.is_none())
            });
            let expected = roots.contains(&root) && self.node_has_hit_box(root);
            if present != expected {
                return None;
            }
            if !expected {
                return Some(Vec::new());
            }
            let position = roots.iter().position(|id| *id == root).unwrap_or_default();
            let (mut rebuilt, deferred) =
                self.build_hit_forest_deferring(vec![(root, IDENTITY_AFFINE, None)]);
            if rebuilt.len() > 1 {
                return None;
            }
            let mut entry = rebuilt.pop()?;
            entry.order = position;
            sort_hit_children(&mut entry);
            self.note_hit_nodes_built(std::slice::from_ref(&entry));
            let forest = self.hit_test_index.get_mut(&document)?;
            forest.replace(root, None, Some(entry));
            return Some(deferred);
        };

        // The parent's own entry supplies the inherited transform. Its scroll is
        // read live because scroll never invalidates the parent entry itself.
        let scroll = self.nodes.scroll_offset(parent);
        let Some((child_transform, _)) = self
            .hit_test_index
            .get(&document)
            .and_then(|forest| forest.live_child_seed(parent, scroll, None))
        else {
            // Parent is absent from the index. That is correct only when the
            // parent omits its layout subtree; otherwise the index is stale.
            return (!self.node_has_hit_box(parent)).then(Vec::new);
        };
        let siblings = &self.record(parent).hierarchy.children;
        let position = self
            .hit_test_index
            .get(&document)
            .and_then(|index| {
                let entry = index.entries.get(&root)?;
                let parent_entry = index.entries.get(&parent)?;
                (entry.parent == Some(parent)
                    && Arc::ptr_eq(siblings, &parent_entry.entry.source_children))
                .then_some(entry.entry.order)
            })
            .or_else(|| siblings.iter().position(|id| *id == root));
        let position = position?;
        let (mut rebuilt, deferred) =
            self.build_hit_forest_deferring(vec![(root, child_transform, Some(parent))]);
        if rebuilt.len() > 1 {
            return None;
        }
        let entry = rebuilt.pop().map(|mut entry| {
            entry.order = position;
            sort_hit_children(&mut entry);
            entry
        });
        if let Some(entry) = entry.as_ref() {
            self.note_hit_nodes_built(std::slice::from_ref(entry));
        }
        let forest = self.hit_test_index.get_mut(&document)?;
        forest.replace(root, Some(parent), entry);
        Some(deferred)
    }

    /// Patch an escaped box's branch where it hangs: under its clip parent
    /// `anchor`, from that entry's child transform and the transforms of the
    /// ancestors it passes over.
    fn patch_escaped_hit(
        &mut self,
        document: DocumentId,
        root: StableNodeId,
        anchor: StableNodeId,
    ) -> Option<Vec<StableNodeId>> {
        let scroll = self.nodes.scroll_offset(anchor);
        let Some((base, _)) = self
            .hit_test_index
            .get(&document)
            .and_then(|forest| forest.live_child_seed(anchor, scroll, None))
        else {
            return (!self.node_has_hit_box(anchor)).then(Vec::new);
        };
        let (transform, _) = self.clip_bridge_transform_from(
            root,
            Some(anchor),
            (base, [0.0, 0.0]),
            false,
            self.layout_origin(anchor).unwrap_or_default(),
        );
        let position = self
            .parent_id(root)
            .and_then(|parent| self.child_index(parent, root))?;
        let (mut rebuilt, deferred) =
            self.build_hit_forest_deferring(vec![(root, transform, Some(anchor))]);
        if rebuilt.len() > 1 {
            return None;
        }
        let entry = rebuilt.pop().map(|mut entry| {
            entry.order = position;
            entry.escaped = true;
            entry.pinned = true;
            sort_hit_children(&mut entry);
            entry
        });
        if let Some(entry) = entry.as_ref() {
            self.note_hit_nodes_built(std::slice::from_ref(entry));
        }
        let forest = self.hit_test_index.get_mut(&document)?;
        forest.replace(root, Some(anchor), entry);
        Some(deferred)
    }
}

impl UiWorld {
    /// Reduce `dirty` to the shallowest nodes that cover it, dropping any node
    /// that already has a dirty ancestor. `None` means a dirty node belongs to
    /// another document and the caller should not attempt a scoped patch.
    pub(in crate::world) fn minimal_hit_patch_roots(
        &self,
        document: DocumentId,
        dirty: &[StableNodeId],
    ) -> Option<Vec<StableNodeId>> {
        let mut pending = HashSet::new();
        for &id in dirty {
            if !self.contains(id) {
                // A despawned node was already spliced out by `retain_hit_tree`.
                continue;
            }
            if self.record(id).document != document {
                return None;
            }
            pending.insert(id);
        }
        let mut roots = Vec::new();
        for &id in &pending {
            let mut cursor = self.parent_id(id);
            let mut covered = false;
            while let Some(ancestor) = cursor {
                if pending.contains(&ancestor) {
                    covered = true;
                    break;
                }
                cursor = self.parent_id(ancestor);
            }
            if !covered {
                roots.push(id);
            }
        }
        roots.sort_unstable();
        Some(roots)
    }
}

impl UiWorld {
    /// Take `id`'s retained hit entry out of pointer dispatch now, ahead of
    /// the frame that rebuilds it. Only the flags change: the bounds it leaves
    /// behind still cover the entry, and a cover too large only visits a node
    /// that then emits nothing.
    fn withdraw_hit_entry(&mut self, document: DocumentId, id: StableNodeId) {
        if let Some(indexed) = self
            .hit_test_index
            .get_mut(&document)
            .and_then(|index| index.entries.get_mut(&id))
        {
            indexed.entry.hittable = false;
            indexed.entry.menu = None;
        }
    }

    /// A commit changed `root`'s `pointer-events`; the index keeps the value
    /// it was built with until the frame. When the used value is now `none`,
    /// withdraw `root` and every descendant that inherits it, so a hit before
    /// that frame falls through to what is below. A descendant that specifies
    /// its own value keeps it, and so does its subtree.
    ///
    /// Only withdraws: a subtree made hittable again waits for the frame,
    /// which resolves the computed style it also needs.
    ///
    /// Entries only hang under entries, so a root without one has nothing
    /// under it to withdraw either.
    pub(in crate::world) fn withdraw_pointer_events_none(&mut self, root: StableNodeId) {
        let document = self.record(root).document;
        if self
            .hit_test_index
            .get(&document)
            .is_none_or(|index| !index.entries.contains_key(&root))
            || self.used_pointer_events(root).hittable()
        {
            return;
        }
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            self.withdraw_hit_entry(document, id);
            stack.extend(
                self.record(id)
                    .hierarchy
                    .children
                    .iter()
                    .copied()
                    .filter(|child| self.record(*child).style.layout.pointer_events.is_none()),
            );
        }
    }

    /// A commit made `root`'s subtree inert; the index keeps the entries it
    /// was built with until the frame. Withdraw `root` and every descendant
    /// now -- a descendant cannot turn inertness back off -- so a hit before
    /// that frame falls through to what is below. Like
    /// [`Self::withdraw_pointer_events_none`], it only withdraws.
    pub(in crate::world) fn withdraw_inert_hits(&mut self, root: StableNodeId) {
        let document = self.record(root).document;
        if self
            .hit_test_index
            .get(&document)
            .is_none_or(|index| !index.entries.contains_key(&root))
        {
            return;
        }
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            self.withdraw_hit_entry(document, id);
            stack.extend(self.record(id).hierarchy.children.iter().copied());
        }
    }

    /// A commit moved `root` under a parked parent. Parking a subtree takes
    /// it out of the index; moving one into a parked branch must too.
    pub(in crate::world) fn withdraw_parked_hits(&mut self, root: StableNodeId) {
        let document = self.record(root).document;
        if self
            .hit_test_index
            .get(&document)
            .is_none_or(|index| !index.entries.contains_key(&root))
        {
            return;
        }
        let subtree = self.subtree_ids(root);
        let index = self
            .hit_test_index
            .get_mut(&document)
            .expect("checked above");
        for id in subtree {
            retain_hit_tree(index, id);
        }
    }

    /// A commit turned `id`'s own pointer interaction off. Not inherited, so
    /// only its entry is withdrawn.
    pub(in crate::world) fn withdraw_pointer_interaction(&mut self, id: StableNodeId) {
        let document = self.record(id).document;
        self.withdraw_hit_entry(document, id);
    }
}
