//! Incremental hit index and transformed pointer queries.

use super::*;

pub(super) fn sort_hit_children(node: &mut HitEntry) {
    // Children were attached last-to-first; (z, order) restores document order
    // within a stacking level so a reverse walk is front-to-back.
    node.children
        .sort_by_key(|child| (child.z_index, child.order));
    for child in &mut node.children {
        sort_hit_children(child);
    }
}

/// Accumulated transform of `id`'s existing entry, used as the splice point for
/// a scoped rebuild so the patch never walks up to the document root.
pub(super) fn find_hit_transform(forest: &[HitEntry], id: StableNodeId) -> Option<[f32; 6]> {
    for entry in forest {
        if entry.id == id {
            return Some(entry.transform);
        }
        if let Some(transform) = find_hit_transform(&entry.children, id) {
            return Some(transform);
        }
    }
    None
}

pub(super) fn find_hit_entry_mut(
    forest: &mut [HitEntry],
    id: StableNodeId,
) -> Option<&mut HitEntry> {
    for entry in forest {
        if entry.id == id {
            return Some(entry);
        }
        if let Some(found) = find_hit_entry_mut(&mut entry.children, id) {
            return Some(found);
        }
    }
    None
}

pub(super) fn count_hit_entries(entry: &HitEntry) -> usize {
    1 + entry.children.iter().map(count_hit_entries).sum::<usize>()
}

pub(super) fn retain_hit_tree(nodes: &mut Vec<HitEntry>, id: StableNodeId) {
    nodes.retain_mut(|node| {
        if node.id == id {
            false
        } else {
            retain_hit_tree(&mut node.children, id);
            true
        }
    });
}

pub(super) fn patch_hit_scroll(
    nodes: &mut [HitEntry],
    scroller: StableNodeId,
    subtree: &HashSet<StableNodeId>,
    delta: [f32; 2],
) {
    for node in nodes {
        if node.id != scroller && subtree.contains(&node.id) {
            let [a, b, c, d, e, f] = node.transform;
            node.transform = [
                a,
                b,
                c,
                d,
                a * delta[0] + c * delta[1] + e,
                b * delta[0] + d * delta[1] + f,
            ];
        }
        patch_hit_scroll(&mut node.children, scroller, subtree, delta);
    }
}

/// First candidate `collect_hit_candidates` would push for this subtree.
///
/// Mirrors that traversal exactly and returns at the first would-be push. Kept
/// beside it so the two orders stay in step; the pair is pinned by
/// `hit_test_matches_the_first_collected_candidate`.
pub(super) fn first_hit_candidate(node: &HitEntry, x: f32, y: f32) -> Option<StableNodeId> {
    if !node
        .self_clips
        .iter()
        .all(|(bounds, transform)| transformed_contains(*bounds, *transform, [0.0, 0.0], x, y))
    {
        return None;
    }
    let menu_hit = node
        .menu
        .is_some_and(|menu| transformed_contains(menu, node.transform, node.persp, x, y));
    let children_ok = node
        .child_clips
        .iter()
        .all(|(bounds, transform)| transformed_contains(*bounds, *transform, [0.0, 0.0], x, y));
    let menu_z = node.z_index.max(1_000);
    if children_ok {
        for child in node.children.iter().rev() {
            if menu_hit && child.z_index <= menu_z {
                return Some(node.id);
            }
            if let Some(found) = first_hit_candidate(child, x, y) {
                return Some(found);
            }
        }
    }
    if menu_hit {
        return Some(node.id);
    }
    if node.hittable && transformed_contains(node.layout, node.transform, node.persp, x, y) {
        return Some(node.id);
    }
    None
}

pub(super) fn collect_hit_candidates(node: &HitEntry, x: f32, y: f32, out: &mut Vec<StableNodeId>) {
    if !node
        .self_clips
        .iter()
        .all(|(bounds, transform)| transformed_contains(*bounds, *transform, [0.0, 0.0], x, y))
    {
        return;
    }
    let menu_hit = node
        .menu
        .is_some_and(|menu| transformed_contains(menu, node.transform, node.persp, x, y));
    let children_ok = node
        .child_clips
        .iter()
        .all(|(bounds, transform)| transformed_contains(*bounds, *transform, [0.0, 0.0], x, y));
    let menu_z = node.z_index.max(1_000);
    let mut emitted_menu = !menu_hit;
    if children_ok {
        for child in node.children.iter().rev() {
            if !emitted_menu && child.z_index <= menu_z {
                out.push(node.id);
                emitted_menu = true;
            }
            collect_hit_candidates(child, x, y, out);
        }
    }
    if !emitted_menu {
        out.push(node.id);
    }
    if node.hittable && transformed_contains(node.layout, node.transform, node.persp, x, y) {
        out.push(node.id);
    }
}

pub(super) fn then_affine([a, b, c, d, e, f]: [f32; 6], rhs: [f32; 6]) -> [f32; 6] {
    then_hit(([a, b, c, d, e, f], [0.0, 0.0]), (rhs, [0.0, 0.0])).0
}

pub(super) fn then_hit(
    (left, [lg, lh]): ([f32; 6], [f32; 2]),
    (right, [rg, rh]): ([f32; 6], [f32; 2]),
) -> ([f32; 6], [f32; 2]) {
    let [a, b, c, d, e, f] = left;
    let [ra, rb, rc, rd, re, rf] = right;
    let na = a * ra + c * rb + e * rg;
    let nb = b * ra + d * rb + f * rg;
    let nc = a * rc + c * rd + e * rh;
    let nd = b * rc + d * rd + f * rh;
    let ne = a * re + c * rf + e;
    let nf = b * re + d * rf + f;
    let ng = lg * ra + lh * rb + rg;
    let nh = lg * rc + lh * rd + rh;
    let ni = lg * re + lh * rf + 1.0;
    if !ni.is_finite() || ni.abs() < 1e-8 {
        return (IDENTITY_AFFINE, [0.0, 0.0]);
    }
    let inv = 1.0 / ni;
    (
        [na * inv, nb * inv, nc * inv, nd * inv, ne * inv, nf * inv],
        [ng * inv, nh * inv],
    )
}

pub(super) fn transformed_contains(
    bounds: LayoutBox,
    [a, b, c, d, e, f]: [f32; 6],
    [g, h]: [f32; 2],
    x: f32,
    y: f32,
) -> bool {
    let det = a * (d - f * h) - c * (b - f * g) + e * (b * h - d * g);
    if !det.is_finite() || det.abs() <= f32::EPSILON {
        return false;
    }
    let inv = 1.0 / det;
    let ia = (d - f * h) * inv;
    let ic = (-c + e * h) * inv;
    let ie = (c * f - e * d) * inv;
    let ib = (-b + f * g) * inv;
    let id = (a - e * g) * inv;
    let if_ = (e * b - a * f) * inv;
    let ig = (b * h - d * g) * inv;
    let ih = (c * g - a * h) * inv;
    let ii = (a * d - c * b) * inv;
    if !ii.is_finite() || ii.abs() < 1e-8 {
        return false;
    }
    let w = ig * x + ih * y + ii;
    if !w.is_finite() || w.abs() < 1e-8 {
        return false;
    }
    let local_x = (ia * x + ic * y + ie) / w;
    let local_y = (ib * x + id * y + if_) / w;
    bounds.contains(local_x, local_y)
}

impl UiWorld {
    pub fn hit_test_candidates(&self, document: DocumentId, x: f32, y: f32) -> Vec<StableNodeId> {
        let Some(forest) = self.hit_test_index.get(&document) else {
            return Vec::new();
        };
        let mut candidates = Vec::new();
        for node in forest.iter().rev() {
            collect_hit_candidates(node, x, y, &mut candidates);
        }
        candidates
    }
}

impl UiWorld {
    /// Topmost hit at `(x, y)`.
    ///
    /// Walks the same order as [`Self::hit_test_candidates`] but returns at the
    /// first hit, so pointer dispatch on every move does not collect and then
    /// discard the full candidate list.
    pub fn hit_test(&self, document: DocumentId, x: f32, y: f32) -> Option<StableNodeId> {
        let forest = self.hit_test_index.get(&document)?;
        forest
            .iter()
            .rev()
            .find_map(|node| first_hit_candidate(node, x, y))
    }
}

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
        let mut subtree = vec![scroller];
        let mut index = 0;
        while index < subtree.len() {
            let id = subtree[index];
            index += 1;
            subtree.extend(self.record(id).hierarchy.children.iter().copied());
        }
        let subtree = subtree
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        let Some(entries) = self.hit_test_index.get_mut(&document) else {
            return;
        };
        patch_hit_scroll(entries, scroller, &subtree, delta);
    }
}

impl UiWorld {
    /// Whether every input-dirty node is explained by recorded scroll deltas
    /// (it is a scroller or descends from one). When true, the frame driver
    /// can patch the hit index in place instead of rebuilding the document.
    pub fn hit_test_work_is_scroll_only(
        &self,
        input: &[StableNodeId],
        updates: &[(StableNodeId, [f32; 2])],
    ) -> bool {
        !updates.is_empty()
            && input.iter().all(|node| {
                updates.iter().any(|(scroller, _)| {
                    *scroller == *node || {
                        let mut cursor = self.parent_id(*node);
                        while let Some(ancestor) = cursor {
                            if ancestor == *scroller {
                                return true;
                            }
                            cursor = self.parent_id(ancestor);
                        }
                        false
                    }
                })
            })
    }
}

impl UiWorld {
    /// Drain the scroll deltas recorded since the last drain.
    pub fn take_scroll_hit_updates(&mut self) -> Vec<(StableNodeId, [f32; 2])> {
        std::mem::take(&mut self.scroll_hit_updates)
    }
}

impl UiWorld {
    /// Build hit entries for `seeds` and their visible descendants. Each seed
    /// carries the accumulated transform of its parent, so a scoped patch can
    /// resume from an existing entry instead of walking from the document root.
    ///
    /// `order` is assigned per sibling group from `Hierarchy` position. It is
    /// only ever compared between siblings, so a spliced subtree stays sortable
    /// against untouched siblings without renumbering the document.
    pub(super) fn build_hit_forest(&self, seeds: Vec<(StableNodeId, [f32; 6])>) -> Vec<HitEntry> {
        struct Built {
            entry: HitEntry,
            parent: Option<usize>,
        }
        let mut stack = seeds
            .into_iter()
            .enumerate()
            .rev()
            .map(|(position, (id, transform))| {
                (
                    id,
                    (transform, [0.0f32, 0.0]),
                    None::<usize>,
                    position,
                    self.parent_used_pointer_events(id),
                    false,
                )
            })
            .collect::<Vec<_>>();
        let mut built: Vec<Built> = Vec::new();
        let mut memo = AncestorMemo::default();
        while let Some((id, parent_hit, parent, position, parent_used_pe, parent_blocks_3d)) =
            stack.pop()
        {
            let style = self.record(id).resolved.0.as_ref();
            let layout = self.record(id).layout;
            let node_style = self.record(id).style.layout.as_ref();
            let local = if parent_blocks_3d && node_style.transform_3d.is_some() {
                (IDENTITY_AFFINE, [0.0, 0.0])
            } else {
                node_style
                    .world_scene_transform(layout.x, layout.y, layout.width, layout.height)
                    .unwrap_or((IDENTITY_AFFINE, [0.0, 0.0]))
            };
            let (transform, persp) = then_hit(parent_hit, local);
            let scroll = self.record(id).scroll_offset;
            let child_transform = then_hit(
                (transform, persp),
                ([1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y], [0.0, 0.0]),
            );
            let child_blocks_3d = parent_blocks_3d || node_style.fails_closed_3d_context();
            let children = Arc::clone(&self.record(id).hierarchy.children);
            let used_pe =
                PointerEventsSpec::inherit_from(node_style.pointer_events, parent_used_pe);
            if !style.visible {
                // `visibility:hidden` skips this entry but descendants may be
                // `visibility:visible` and still need the accumulated transform.
                stack.extend(children.iter().enumerate().rev().map(|(position, child)| {
                    (
                        *child,
                        child_transform,
                        parent,
                        position,
                        used_pe,
                        child_blocks_3d,
                    )
                }));
                continue;
            }
            let mut self_clips = Vec::new();
            let mut child_clips = Vec::new();
            if let Some((x, y, w, h)) =
                node_style.overflow_clip_box(layout.x, layout.y, layout.width, layout.height)
            {
                child_clips.push((
                    LayoutBox {
                        x,
                        y,
                        width: w,
                        height: h,
                    },
                    transform,
                ));
            }
            if self.clip_visuals != 0 {
                if matches!(
                    self.nodes.visual(id),
                    Some(StandardVisual::EmptyState { .. })
                ) {
                    child_clips.push((layout, transform));
                }
                if let Some(crate::ComponentGeometry::ModalFrame { surface, .. }) =
                    self.component_geometry(id)
                {
                    child_clips.push((surface, transform));
                }
                if let Some(parent_id) = self.parent_id(id)
                    && let Some(StandardVisual::ModalFrame { slots, .. }) =
                        self.nodes.visual(parent_id)
                    && slots.body == Some(id)
                    && let Some(crate::ComponentGeometry::ModalFrame { body, .. }) =
                        self.component_geometry(parent_id)
                {
                    self_clips.push((body, parent_hit.0));
                }
            }
            let interaction = self.record(id).interaction;
            let confirm_busy = self
                .confirm_action_effect(id)
                .is_some_and(|effect| effect.0);
            let hittable = interaction.pointer_events
                && used_pe.hittable()
                && style.pointer_events.hittable()
                && !confirm_busy;
            let menu = hittable
                .then(|| self.component_geometry(id))
                .flatten()
                .and_then(|geometry| match geometry {
                    crate::ComponentGeometry::Select {
                        menu: Some(menu), ..
                    } => Some(menu.surface),
                    _ => None,
                });
            let index = built.len();
            built.push(Built {
                entry: HitEntry {
                    id,
                    layout,
                    transform,
                    persp,
                    self_clips,
                    child_clips,
                    z_index: self.stacking_z_index_memo(id, &mut memo),
                    order: position,
                    hittable,
                    menu,
                    children: Vec::new(),
                },
                parent,
            });
            // Sibling position is the sort key. Invisible siblings are skipped
            // and leave gaps, which is harmless because only relative order
            // between surviving siblings is ever compared.
            stack.extend(children.iter().enumerate().rev().map(|(position, child)| {
                (
                    *child,
                    child_transform,
                    Some(index),
                    position,
                    used_pe,
                    child_blocks_3d,
                )
            }));
        }
        let n = built.len();
        let mut parent_of = Vec::with_capacity(n);
        let mut entries = Vec::with_capacity(n);
        for node in built {
            parent_of.push(node.parent);
            entries.push(Some(node.entry));
        }
        for i in (0..n).rev() {
            if let Some(parent) = parent_of[i] {
                let child = entries[i].take().expect("child hit node");
                entries[parent]
                    .as_mut()
                    .expect("parent hit node")
                    .children
                    .push(child);
            }
        }
        entries.into_iter().flatten().collect::<Vec<_>>()
    }
}

impl UiWorld {
    /// Every build path funnels through here, so recording once keeps the
    /// sentinel honest for both a scoped patch and a full rebuild.
    pub(super) fn note_hit_nodes_built(&mut self, forest: &[HitEntry]) {
        let built = forest.iter().map(count_hit_entries).sum::<usize>();
        self.bump_last_counters(|counters| counters.record_hit_test_rebuild(built));
    }
}

impl UiWorld {
    /// Whether `id` would contribute an entry to the hit index.
    pub(super) fn node_hit_visible(&self, id: StableNodeId) -> bool {
        self.contains(id) && self.record(id).resolved.0.visible
    }
}

impl UiWorld {
    /// Replace `root`'s entry (and its descendants) in place. Reuses the parent
    /// entry's accumulated transform so no walk from the document root is
    /// needed. Returns `false` when the splice point is missing and the caller
    /// must rebuild the document.
    pub(super) fn patch_hit_subtree(&mut self, document: DocumentId, root: StableNodeId) -> bool {
        let parent = self.parent_id(root);
        let Some(parent) = parent else {
            // Document roots: membership is owned by `live_document_roots`, so a
            // root entering or leaving the set is a structural change.
            let roots = self.document_roots(document);
            let present = self
                .hit_test_index
                .get(&document)
                .is_some_and(|forest| forest.iter().any(|entry| entry.id == root));
            let expected = roots.contains(&root) && self.node_hit_visible(root);
            if present != expected {
                return false;
            }
            if !expected {
                return true;
            }
            let position = roots.iter().position(|id| *id == root).unwrap_or_default();
            let mut rebuilt = self.build_hit_forest(vec![(root, IDENTITY_AFFINE)]);
            let Some(mut entry) = rebuilt.pop() else {
                return false;
            };
            entry.order = position;
            sort_hit_children(&mut entry);
            self.note_hit_nodes_built(std::slice::from_ref(&entry));
            let Some(forest) = self.hit_test_index.get_mut(&document) else {
                return false;
            };
            if let Some(slot) = forest.iter_mut().find(|slot| slot.id == root) {
                *slot = entry;
            } else {
                forest.push(entry);
            }
            forest.sort_by_key(|entry| (entry.z_index, entry.order));
            return true;
        };

        // The parent's own entry supplies the inherited transform. Its scroll is
        // read live because scroll never invalidates the parent entry itself.
        let Some(parent_transform) = self
            .hit_test_index
            .get(&document)
            .and_then(|forest| find_hit_transform(forest, parent))
        else {
            // Parent is absent from the index. That is correct only when the
            // parent is itself not hit-visible; otherwise the index is stale.
            return !self.node_hit_visible(parent);
        };
        let scroll = self.record(parent).scroll_offset;
        let child_transform =
            then_affine(parent_transform, [1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y]);
        let siblings = Arc::clone(&self.record(parent).hierarchy.children);
        let position = siblings.iter().position(|id| *id == root);
        let Some(position) = position else {
            return false;
        };
        let mut rebuilt = self.build_hit_forest(vec![(root, child_transform)]);
        let entry = rebuilt.pop().map(|mut entry| {
            entry.order = position;
            sort_hit_children(&mut entry);
            entry
        });
        if let Some(entry) = entry.as_ref() {
            self.note_hit_nodes_built(std::slice::from_ref(entry));
        }
        let Some(forest) = self.hit_test_index.get_mut(&document) else {
            return false;
        };
        let Some(parent_entry) = find_hit_entry_mut(forest, parent) else {
            return false;
        };
        match entry {
            Some(entry) => {
                if let Some(slot) = parent_entry
                    .children
                    .iter_mut()
                    .find(|slot| slot.id == root)
                {
                    *slot = entry;
                } else {
                    parent_entry.children.push(entry);
                }
            }
            // The subtree turned invisible: drop it from the parent.
            None => parent_entry.children.retain(|slot| slot.id != root),
        }
        parent_entry
            .children
            .sort_by_key(|child| (child.z_index, child.order));
        true
    }
}

impl UiWorld {
    /// Reduce `dirty` to the shallowest nodes that cover it, dropping any node
    /// that already has a dirty ancestor. `None` means a dirty node belongs to
    /// another document and the caller should not attempt a scoped patch.
    pub(super) fn minimal_hit_patch_roots(
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
        if dirty.is_empty() {
            return true;
        }
        if !self.hit_test_index.contains_key(&document) {
            return false;
        }
        let Some(roots) = self.minimal_hit_patch_roots(document, dirty) else {
            return false;
        };
        for root in roots {
            if !self.patch_hit_subtree(document, root) {
                return false;
            }
        }
        true
    }
}

impl UiWorld {
    /// Rebuild one document's event-time hit-test tree after scheduled input
    /// or layout work. Pointer dispatch walks that tree in z/order with clip
    /// early-out instead of flattening and sorting every query.
    pub fn rebuild_hit_test(&mut self, document: DocumentId) {
        let roots = self.document_roots(document);
        let seeds = roots
            .iter()
            .copied()
            .map(|id| (id, IDENTITY_AFFINE))
            .collect::<Vec<_>>();
        let mut forest = self.build_hit_forest(seeds);
        for root in &mut forest {
            sort_hit_children(root);
        }
        forest.sort_by_key(|entry| (entry.z_index, entry.order));
        self.note_hit_nodes_built(&forest);
        self.hit_test_index.insert(document, forest);
    }
}
