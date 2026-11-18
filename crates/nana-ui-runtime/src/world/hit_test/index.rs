//! The hit index: entries, their bounds tree and the structural edits that
//! keep it (insert, reindex, refresh bounds, remove, replace).

use super::*;

pub(in crate::world) fn sort_hit_children(node: &mut HitEntry) {
    // Children were attached last-to-first; (z, order) restores document order
    // within a stacking level so a reverse walk is front-to-back.
    node.children
        .sort_by_key(|child| (child.z_index, child.order));
    for child in &mut node.children {
        sort_hit_children(child);
    }
}

/// Flat, document-local hit projection. Runtime remains the topology authority;
/// entries are directly addressed and scrolling changes one inherited offset.
#[derive(Default)]
pub(in crate::world) struct HitIndex {
    pub(in crate::world) roots: Vec<Option<StableNodeId>>,
    pub(in crate::world) entries: hashbrown::HashMap<StableNodeId, IndexedHit>,
    pub(super) root_bounds: BoundsTree,
    pub(super) viewport_roots: HashSet<StableNodeId>,
    /// Entries hanging from their clip parent rather than their parent
    /// (#201). Ids whose entry has since gone or stopped escaping only cost a
    /// paint-order ranking they did not need.
    pub(super) escaped: HashSet<StableNodeId>,
    /// Entries that read geometry from beyond their own offset
    /// ([`HitEntry::pinned`]): a move of a box above one builds it again.
    /// Ids whose entry has since gone or stopped being pinned cost a check.
    pub(super) pinned: HashSet<StableNodeId>,
    /// The open levels and pending children of a [`Self::visit_hits`] walk,
    /// kept between queries so a pointer move allocates nothing once they
    /// have grown to the tree's depth.
    pub(super) visit_scratch: std::cell::RefCell<HitVisitScratch>,
    /// Whether a scroll has shifted an entry since the index was built.
    /// Until one has, every [`IndexedHit::shift`] is +0, and so is any sum
    /// of them: the walks up for one ([`Self::inherited_shift`]) answer
    /// without walking.
    pub(super) shifted: bool,
}

/// See [`HitIndex::visit_scratch`].
#[derive(Default)]
pub(super) struct HitVisitScratch {
    pub(super) open: Vec<OpenHit>,
    /// Every open level's pending children, each level's in a range of it.
    pub(super) children: Vec<StableNodeId>,
}

/// Where a build starts: a node, the transform its hit parent hands its
/// children, and that parent (`None` for a root of the index), whose
/// origin the node's offset is measured from.
pub(super) type HitSeed = (StableNodeId, [f32; 6], Option<StableNodeId>);

// Preorder construction data; parents precede their children.
pub(super) struct BuiltHit {
    pub(super) entry: HitEntry,
    pub(super) parent: Option<usize>,
}

pub(in crate::world) struct IndexedHit {
    pub(in crate::world) entry: HitEntry,
    pub(in crate::world) parent: Option<StableNodeId>,
    pub(in crate::world) children: Vec<Option<StableNodeId>>,
    pub(super) shift: [f32; 2],
    pub(super) bounds: Bounds,
    pub(super) child_bounds: BoundsTree,
    pub(super) sibling_slot: usize,
}

pub(super) fn hit_bounds(entry: &HitEntry) -> Bounds {
    if !entry.hittable && entry.menu.is_none() {
        return Bounds::Inactive;
    }
    if entry.persp != [0.0, 0.0] {
        return Bounds::Unknown;
    }
    let [a, bm, c, d, e, f] = entry.transform;
    if entry.menu.is_none() && a == 1.0 && bm == 0.0 && c == 0.0 && d == 1.0 && e == 0.0 && f == 0.0
    {
        let bounds = entry.layout;
        if bounds.x.is_finite()
            && bounds.y.is_finite()
            && bounds.width.is_finite()
            && bounds.height.is_finite()
        {
            return Bounds::Known(bounds);
        }
        return Bounds::Unknown;
    }
    let b = entry
        .menu
        .map_or(entry.layout, |menu| union_bounds(entry.layout, menu));
    let points = [
        (b.x, b.y),
        (b.x + b.width, b.y),
        (b.x, b.y + b.height),
        (b.x + b.width, b.y + b.height),
    ];
    let mut left = f32::INFINITY;
    let mut top = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    let mut bottom = f32::NEG_INFINITY;
    for (x, y) in points {
        let tx = a * x + c * y + e;
        let ty = bm * x + d * y + f;
        left = left.min(tx);
        top = top.min(ty);
        right = right.max(tx);
        bottom = bottom.max(ty);
    }
    [left, top, right, bottom]
        .iter()
        .all(|v| v.is_finite())
        .then_some(LayoutBox {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
        .into()
}

/// What an entry covers in its parent's children's space: its own hit and
/// its children's, `shift` (its scroll) away -- and, when it hands them its
/// own layout coordinates, the box around their corners through its
/// transform -- all [`HitEntry::origin`] away. A cold build and every
/// refresh of a kept entry use this one formula, so they agree to the bit.
fn entry_bounds(entry: &HitEntry, children: Option<Bounds>, shift: [f32; 2]) -> Bounds {
    let own = hit_bounds(entry);
    let merged = match children {
        Some(children) if entry.local_children => {
            own.merge(turned_bounds(children.translated(shift), entry.transform))
        }
        Some(children) => own.merge(children.translated(shift)),
        None => own,
    };
    merged.translated(entry.origin)
}

/// `bounds` in an entry's layout coordinates as the box around their
/// corners in the space its (perspective-free) `transform` maps them to.
fn turned_bounds(bounds: Bounds, transform: [f32; 6]) -> Bounds {
    match bounds {
        Bounds::Known(bounds) => project_transformed_box(bounds, transform, [0.0, 0.0]).into(),
        other => other,
    }
}

/// What a box that hands its children its own layout coordinates hands
/// them: its scroll, nothing of its transform.
pub(super) fn scroll_seed(scroll: [f32; 2]) -> [f32; 6] {
    [1.0, 0.0, 0.0, 1.0, -scroll[0], -scroll[1]]
}

/// `offset` through the linear part of `transform`: where a box `offset`
/// away in the coordinates `transform` maps stands in the space it maps to.
pub(super) fn linear_offset([a, b, c, d, _, _]: [f32; 6], [x, y]: [f32; 2]) -> [f32; 2] {
    if a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 {
        return [x, y];
    }
    [a * x + c * y, b * x + d * y]
}

pub(super) fn union_bounds(a: LayoutBox, b: LayoutBox) -> LayoutBox {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    LayoutBox {
        x,
        y,
        width: (a.x + a.width).max(b.x + b.width) - x,
        height: (a.y + a.height).max(b.y + b.height) - y,
    }
}

impl HitIndex {
    #[cfg(test)]
    pub(super) fn from_forest(forest: Vec<HitEntry>) -> Self {
        let mut index = Self::default();
        for (slot, entry) in forest.into_iter().enumerate() {
            index.roots.push(Some(entry.id));
            index.insert_tree(entry, None, [0.0, 0.0], slot);
        }
        index.reindex_children(None);
        index
    }

    pub(super) fn from_entries(built: Vec<BuiltHit>) -> Self {
        let n = built.len();
        let ids = built.iter().map(|node| node.entry.id).collect::<Vec<_>>();
        // One child table for the whole forest. Visit order already pops the
        // first sibling first, so a group stays in sibling order until a
        // z-index reorders that range.
        let mut child_counts = vec![0u32; n];
        let mut root_count = 0usize;
        let mut reorder = false;
        for node in &built {
            reorder |= node.entry.z_index != 0;
            if let Some(parent) = node.parent {
                child_counts[parent] += 1;
            } else {
                root_count += 1;
            }
        }
        let mut offsets = vec![0u32; n + 1];
        for index in 0..n {
            offsets[index + 1] = offsets[index] + child_counts[index];
        }
        let mut cursor = offsets.clone();
        let mut child_index = vec![0usize; offsets[n] as usize];
        let mut roots = Vec::with_capacity(root_count);
        for (index, node) in built.iter().enumerate() {
            if let Some(parent) = node.parent {
                let slot = cursor[parent];
                cursor[parent] += 1;
                child_index[slot as usize] = index;
            } else {
                roots.push(index);
            }
        }
        if reorder {
            let by_paint = |index: &usize| {
                let entry = &built[*index].entry;
                (entry.z_index, entry.order)
            };
            for index in 0..n {
                let start = offsets[index] as usize;
                let end = offsets[index + 1] as usize;
                if end - start > 1 {
                    child_index[start..end].sort_by_key(by_paint);
                }
            }
            if roots.len() > 1 {
                roots.sort_by_key(by_paint);
            }
        }
        let mut bounds = vec![Bounds::Empty; n];
        let mut trees = Vec::with_capacity(n);
        trees.resize_with(n, BoundsTree::default);
        for index in (0..n).rev() {
            let start = offsets[index] as usize;
            let end = offsets[index + 1] as usize;
            if start == end {
                bounds[index] = entry_bounds(&built[index].entry, None, [0.0, 0.0]);
                continue;
            }
            let tree = BoundsTree::new(
                child_index[start..end]
                    .iter()
                    .copied()
                    .map(|child| bounds[child]),
            );
            bounds[index] = entry_bounds(&built[index].entry, Some(tree.bounds()), [0.0, 0.0]);
            trees[index] = tree;
        }
        let root_bounds = BoundsTree::new(roots.iter().copied().map(|child| bounds[child]));
        let mut sibling_slot = vec![0usize; n];
        for index in 0..n {
            let start = offsets[index] as usize;
            let end = offsets[index + 1] as usize;
            for (slot, &child) in child_index[start..end].iter().enumerate() {
                sibling_slot[child] = slot;
            }
        }
        for (slot, &child) in roots.iter().enumerate() {
            sibling_slot[child] = slot;
        }
        let mut entries = hashbrown::HashMap::with_capacity(n);
        let mut escaped = HashSet::new();
        let mut pinned = HashSet::new();
        for (index, node) in built.into_iter().enumerate() {
            if node.entry.escaped {
                escaped.insert(node.entry.id);
            }
            if node.entry.pinned {
                pinned.insert(node.entry.id);
            }
            let id = node.entry.id;
            let parent = node.parent.map(|position| ids[position]);
            let start = offsets[index] as usize;
            let end = offsets[index + 1] as usize;
            let children = child_index[start..end]
                .iter()
                .map(|&child| Some(ids[child]))
                .collect::<Vec<_>>();
            entries.insert(
                id,
                IndexedHit {
                    bounds: bounds[index],
                    entry: node.entry,
                    parent,
                    children,
                    shift: [0.0, 0.0],
                    child_bounds: std::mem::take(&mut trees[index]),
                    sibling_slot: sibling_slot[index],
                },
            );
        }
        Self {
            roots: roots.into_iter().map(|index| Some(ids[index])).collect(),
            entries,
            root_bounds,
            viewport_roots: HashSet::new(),
            escaped,
            pinned,
            visit_scratch: Default::default(),
            shifted: false,
        }
    }

    /// Indexes `entry` and its subtree. The walk keeps its open levels on the
    /// heap, so a subtree as deep as the tree allows indexes on any stack.
    pub(super) fn insert_tree(
        &mut self,
        entry: HitEntry,
        parent: Option<StableNodeId>,
        offset: [f32; 2],
        sibling_slot: usize,
    ) {
        struct Open {
            entry: HitEntry,
            parent: Option<StableNodeId>,
            sibling_slot: usize,
            bounds: Bounds,
            child_ids: Vec<Option<StableNodeId>>,
            children: std::iter::Enumerate<std::vec::IntoIter<HitEntry>>,
            /// What its children's transforms take off: nothing below an
            /// entry that hands them its own layout coordinates.
            child_offset: [f32; 2],
        }
        let open = |index: &mut Self,
                    mut entry: HitEntry,
                    parent: Option<StableNodeId>,
                    sibling_slot: usize,
                    offset: [f32; 2]| {
            if entry.escaped {
                index.escaped.insert(entry.id);
            }
            if entry.pinned {
                index.pinned.insert(entry.id);
            }
            let children = std::mem::take(&mut entry.children);
            entry.transform[4] -= offset[0];
            entry.transform[5] -= offset[1];
            for (_, transform) in entry
                .self_clips
                .iter_mut()
                .chain(entry.child_clips.iter_mut())
            {
                transform[4] -= offset[0];
                transform[5] -= offset[1];
            }
            Open {
                bounds: Bounds::Empty,
                child_ids: children.iter().map(|child| Some(child.id)).collect(),
                children: children.into_iter().enumerate(),
                child_offset: if entry.local_children {
                    [0.0, 0.0]
                } else {
                    offset
                },
                entry,
                parent,
                sibling_slot,
            }
        };
        let mut levels = vec![open(self, entry, parent, sibling_slot, offset)];
        while let Some(level) = levels.last_mut() {
            if let Some((slot, child)) = level.children.next() {
                let parent = Some(level.entry.id);
                let offset = level.child_offset;
                let child = open(self, child, parent, slot, offset);
                levels.push(child);
                continue;
            }
            let level = levels.pop().unwrap();
            let id = level.entry.id;
            let child_bounds = BoundsTree::new(
                level
                    .child_ids
                    .iter()
                    .map(|id| self.entries[&id.unwrap()].bounds),
            );
            let bounds = entry_bounds(
                &level.entry,
                (!level.child_ids.is_empty()).then(|| child_bounds.bounds()),
                [0.0, 0.0],
            );
            if let Some(parent) = levels.last_mut() {
                parent.bounds = parent.bounds.merge(bounds);
            }
            self.entries.insert(
                id,
                IndexedHit {
                    entry: level.entry,
                    parent: level.parent,
                    children: level.child_ids,
                    shift: [0.0, 0.0],
                    bounds,
                    child_bounds,
                    sibling_slot: level.sibling_slot,
                },
            );
        }
    }

    // Only structural sibling changes rebuild a range index. Geometry and
    // scrolling update one leaf per ancestor, without scanning siblings.
    fn reindex_children(&mut self, parent: Option<StableNodeId>) {
        let mut children = match parent {
            Some(id) => std::mem::take(&mut self.entries.get_mut(&id).unwrap().children),
            None => std::mem::take(&mut self.roots),
        };
        // Deletions leave empty bound slots, so the stable sibling positions
        // survive until the next structural insertion/reorder compacts them.
        children.retain(|id| {
            id.is_some_and(|id| {
                self.entries
                    .get(&id)
                    .is_some_and(|node| node.parent == parent)
            })
        });
        let bounds = BoundsTree::new(children.iter().map(|id| self.entries[&id.unwrap()].bounds));
        for (slot, id) in children.iter().enumerate() {
            self.entries.get_mut(&id.unwrap()).unwrap().sibling_slot = slot;
        }
        if let Some(parent) = parent {
            let node = self.entries.get_mut(&parent).unwrap();
            node.child_bounds = bounds;
            node.children = children;
            self.refresh_bounds(parent);
        } else {
            self.root_bounds = bounds;
            self.roots = children;
        }
    }

    /// [`Self::refresh_bounds`] for many entries: the deepest first, so each
    /// entry and each ancestor is refreshed once, after everything under it.
    pub(super) fn refresh_bounds_of(&mut self, ids: &[StableNodeId]) {
        // Levels from the top. A deep walk remembers the depths it passes
        // above its first few steps, so the entries of a deep chain or
        // under one each walk a few steps, not the whole chain.
        const DIRECT: usize = 32;
        let mut depths = crate::NodeMap::<usize>::default();
        let mut passed = Vec::new();
        let mut depth = |index: &Self, id: StableNodeId| {
            passed.clear();
            let mut steps = 0;
            let mut cursor = Some(id);
            while let Some(at) = cursor {
                #[cfg(test)]
                crate::world::page_origins::steps::step(
                    crate::world::page_origins::steps::Walk::HitDepth,
                );
                if steps >= DIRECT {
                    if let Some(&known) = depths.get(&at) {
                        steps += known;
                        break;
                    }
                    passed.push((at, steps));
                }
                steps += 1;
                cursor = index.entries.get(&at).and_then(|node| node.parent);
            }
            for &(at, below) in &passed {
                depths.insert(at, steps - below);
            }
            #[cfg(any(test, feature = "layout-verify"))]
            assert_eq!(
                steps,
                std::iter::successors(Some(id), |id| index.entries.get(id)?.parent).count(),
                "{id:?}: a remembered depth is the walked one"
            );
            steps
        };
        let mut open: std::collections::BinaryHeap<(usize, StableNodeId)> = ids
            .iter()
            .filter(|&&id| self.entries.contains_key(&id))
            .map(|&id| (depth(self, id), id))
            .collect();
        let mut queued: crate::NodeSet = ids.iter().copied().collect();
        while let Some((level, id)) = open.pop() {
            let Some(node) = self.entries.get_mut(&id) else {
                continue;
            };
            node.bounds = entry_bounds(
                &node.entry,
                (!node.children.is_empty()).then(|| node.child_bounds.bounds()),
                node.shift,
            );
            let (parent, slot, bounds) = (node.parent, node.sibling_slot, node.bounds);
            match parent {
                Some(parent) => {
                    self.entries
                        .get_mut(&parent)
                        .unwrap()
                        .child_bounds
                        .set(slot, bounds);
                    if queued.insert(parent) {
                        open.push((level - 1, parent));
                    }
                }
                None => self.root_bounds.set(slot, bounds),
            }
        }
    }

    pub(super) fn refresh_bounds(&mut self, mut id: StableNodeId) {
        loop {
            let Some(node) = self.entries.get_mut(&id) else {
                return;
            };
            node.bounds = entry_bounds(
                &node.entry,
                (!node.children.is_empty()).then(|| node.child_bounds.bounds()),
                node.shift,
            );
            let (parent, slot, bounds) = (node.parent, node.sibling_slot, node.bounds);
            if let Some(parent) = parent {
                self.entries
                    .get_mut(&parent)
                    .unwrap()
                    .child_bounds
                    .set(slot, bounds);
                id = parent;
            } else {
                self.root_bounds.set(slot, bounds);
                return;
            }
        }
    }

    fn remove_descendants(&mut self, children: Vec<Option<StableNodeId>>, parent: StableNodeId) {
        let mut pending: Vec<_> = children
            .into_iter()
            .flatten()
            .map(|child| (child, parent))
            .collect();
        while let Some((id, parent)) = pending.pop() {
            // A preceding patch can already have moved this entry elsewhere.
            // Old sibling slots must never delete the new owner's projection.
            if self
                .entries
                .get(&id)
                .is_none_or(|node| node.parent != Some(parent))
            {
                continue;
            }
            self.viewport_roots.remove(&id);
            self.escaped.remove(&id);
            self.pinned.remove(&id);
            let node = self.entries.remove(&id).unwrap();
            pending.extend(node.children.into_iter().flatten().map(|child| (child, id)));
        }
    }

    pub(super) fn remove(&mut self, id: StableNodeId) {
        self.viewport_roots.remove(&id);
        self.escaped.remove(&id);
        self.pinned.remove(&id);
        let Some(node) = self.entries.remove(&id) else {
            return;
        };
        let parent = node.parent;
        let slot = node.sibling_slot;
        self.remove_descendants(node.children, id);
        if let Some(parent) = parent {
            let node = self.entries.get_mut(&parent).unwrap();
            node.children[slot] = None;
            node.child_bounds.clear(slot);
            if node.child_bounds.is_empty() {
                node.children = Vec::new();
                node.child_bounds = BoundsTree::default();
            }
            self.refresh_bounds(parent);
        } else {
            self.roots[slot] = None;
            self.root_bounds.clear(slot);
            if self.root_bounds.is_empty() {
                self.roots = Vec::new();
                self.root_bounds = BoundsTree::default();
            }
        }
    }

    pub(super) fn replace(
        &mut self,
        root: StableNodeId,
        parent: Option<StableNodeId>,
        entry: Option<HitEntry>,
    ) {
        let offset = self.child_shift(parent);
        // A paint/geometry patch normally preserves this root's sibling slot.
        // Do not remove, search or sort its unrelated siblings in that case.
        if let Some(next) = entry.as_ref()
            && self.entries.get(&root).is_some_and(|old| {
                old.parent == parent
                    && old.entry.z_index == next.z_index
                    && old.entry.order == next.order
            })
        {
            self.escaped.remove(&root);
            self.pinned.remove(&root);
            let old = self.entries.remove(&root).unwrap();
            self.remove_descendants(old.children, root);
            self.insert_tree(entry.unwrap(), parent, offset, old.sibling_slot);
            self.refresh_bounds(root);
            return;
        }
        self.remove(root);
        let Some(entry) = entry else {
            return;
        };
        {
            let id = entry.id;
            self.insert_tree(entry, parent, offset, 0);
            let mut children = if let Some(parent) = parent {
                std::mem::take(&mut self.entries.get_mut(&parent).unwrap().children)
            } else {
                std::mem::take(&mut self.roots)
            };
            children.retain(|id| {
                id.is_some_and(|id| {
                    self.entries
                        .get(&id)
                        .is_some_and(|node| node.parent == parent)
                })
            });
            children.push(Some(id));
            children.sort_by_key(|id| {
                let n = &self.entries[&id.unwrap()].entry;
                (n.z_index, n.order)
            });
            if let Some(parent) = parent {
                self.entries.get_mut(&parent).unwrap().children = children;
            } else {
                self.roots = children;
            }
        }
        self.reindex_children(parent);
    }
}

#[cfg(test)]
pub(in crate::world) fn find_hit_transform(index: &HitIndex, id: StableNodeId) -> Option<[f32; 6]> {
    let mut transform = index.entries.get(&id)?.entry.transform;
    let shift = index.inherited_shift(id);
    transform[4] += shift[0];
    transform[5] += shift[1];
    Some(transform)
}

/// Entries in `entry`'s tree, counted on a heap stack: the tree is as deep
/// as the world's.
pub(in crate::world) fn count_hit_entries(entry: &HitEntry) -> usize {
    let mut count = 0;
    let mut open = vec![entry];
    while let Some(entry) = open.pop() {
        count += 1;
        open.extend(&entry.children);
    }
    count
}

pub(in crate::world) fn retain_hit_tree(index: &mut HitIndex, id: StableNodeId) {
    index.remove(id);
}
