//! Incremental hit index and transformed pointer queries.

use super::*;

mod bounds;
use bounds::{Bounds, BoundsTree};

pub(super) fn sort_hit_children(node: &mut HitEntry) {
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
pub(super) struct HitIndex {
    pub(super) roots: Vec<Option<StableNodeId>>,
    pub(super) entries: hashbrown::HashMap<StableNodeId, IndexedHit>,
    root_bounds: BoundsTree,
    viewport_roots: HashSet<StableNodeId>,
}

// Preorder construction data; parents precede their children.
struct BuiltHit {
    entry: HitEntry,
    parent: Option<usize>,
}

pub(super) struct IndexedHit {
    pub(super) entry: HitEntry,
    pub(super) parent: Option<StableNodeId>,
    pub(super) children: Vec<Option<StableNodeId>>,
    shift: [f32; 2],
    bounds: Bounds,
    child_bounds: BoundsTree,
    sibling_slot: usize,
}

fn hit_bounds(entry: &HitEntry) -> Bounds {
    if !entry.hittable && entry.menu.is_none() {
        return Bounds::Inactive;
    }
    if entry.persp != [0.0, 0.0] {
        return Bounds::Unknown;
    }
    let b = entry
        .menu
        .map_or(entry.layout, |menu| union_bounds(entry.layout, menu));
    let [a, bm, c, d, e, f] = entry.transform;
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

fn union_bounds(a: LayoutBox, b: LayoutBox) -> LayoutBox {
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
    fn viewport_hit_at(&self, x: f32, y: f32) -> bool {
        !self.viewport_roots.is_empty()
            && self.root_bounds.visit(x, y, &mut |slot| {
                self.roots[slot].is_some_and(|id| self.viewport_roots.contains(&id))
            })
    }

    #[cfg(test)]
    fn from_forest(forest: Vec<HitEntry>) -> Self {
        let mut index = Self::default();
        for (slot, entry) in forest.into_iter().enumerate() {
            index.roots.push(Some(entry.id));
            index.insert_tree(entry, None, [0.0, 0.0], slot);
        }
        index.reindex_children(None);
        index
    }

    fn from_entries(built: Vec<BuiltHit>) -> Self {
        let ids = built.iter().map(|node| node.entry.id).collect::<Vec<_>>();
        let mut index = Self {
            entries: hashbrown::HashMap::with_capacity(built.len()),
            ..Default::default()
        };
        for node in built {
            let id = node.entry.id;
            let parent = node.parent.map(|position| ids[position]);
            index.entries.insert(
                id,
                IndexedHit {
                    bounds: hit_bounds(&node.entry),
                    entry: node.entry,
                    parent,
                    children: Vec::new(),
                    shift: [0.0, 0.0],
                    child_bounds: BoundsTree::default(),
                    sibling_slot: 0,
                },
            );
            if let Some(parent) = parent {
                index
                    .entries
                    .get_mut(&parent)
                    .expect("preorder parent")
                    .children
                    .push(Some(id));
            } else {
                index.roots.push(Some(id));
            }
        }
        // Child aggregates are ready before their parent. No temporary recursive
        // HitEntry tree or ancestor refits are needed during initial construction.
        for id in ids.into_iter().rev() {
            index.initialize_children(Some(id));
        }
        index.initialize_children(None);
        index
    }

    fn initialize_children(&mut self, parent: Option<StableNodeId>) {
        let mut children = match parent {
            Some(id) => std::mem::take(&mut self.entries.get_mut(&id).unwrap().children),
            None => std::mem::take(&mut self.roots),
        };
        children.sort_by_key(|id| {
            let entry = &self.entries[&id.unwrap()].entry;
            (entry.z_index, entry.order)
        });
        let bounds = BoundsTree::new(children.iter().map(|id| self.entries[&id.unwrap()].bounds));
        for (slot, id) in children.iter().enumerate() {
            self.entries.get_mut(&id.unwrap()).unwrap().sibling_slot = slot;
        }
        if let Some(parent) = parent {
            let node = self.entries.get_mut(&parent).unwrap();
            if !children.is_empty() {
                node.bounds = node.bounds.merge(bounds.bounds());
            }
            node.children = children;
            node.child_bounds = bounds;
        } else {
            self.roots = children;
            self.root_bounds = bounds;
        }
    }

    fn insert_tree(
        &mut self,
        mut entry: HitEntry,
        parent: Option<StableNodeId>,
        offset: [f32; 2],
        sibling_slot: usize,
    ) {
        let id = entry.id;
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
        let mut bounds = hit_bounds(&entry);
        let child_ids: Vec<_> = children.iter().map(|child| Some(child.id)).collect();
        for (slot, child) in children.into_iter().enumerate() {
            let child_id = child.id;
            self.insert_tree(child, Some(id), offset, slot);
            bounds = bounds.merge(self.entries[&child_id].bounds);
        }
        let child_bounds =
            BoundsTree::new(child_ids.iter().map(|id| self.entries[&id.unwrap()].bounds));
        self.entries.insert(
            id,
            IndexedHit {
                entry,
                parent,
                children: child_ids,
                shift: [0.0, 0.0],
                bounds,
                child_bounds,
                sibling_slot,
            },
        );
    }

    fn inherited_shift(&self, id: StableNodeId) -> [f32; 2] {
        let mut shift = [0.0, 0.0];
        let mut cursor = self.entries.get(&id).and_then(|n| n.parent);
        while let Some(id) = cursor {
            let Some(node) = self.entries.get(&id) else {
                break;
            };
            shift[0] += node.shift[0];
            shift[1] += node.shift[1];
            cursor = node.parent;
        }
        shift
    }

    fn child_shift(&self, parent: Option<StableNodeId>) -> [f32; 2] {
        parent.map_or([0.0, 0.0], |id| {
            let mut shift = self.inherited_shift(id);
            if let Some(node) = self.entries.get(&id) {
                shift[0] += node.shift[0];
                shift[1] += node.shift[1];
            }
            shift
        })
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

    fn refresh_bounds(&mut self, mut id: StableNodeId) {
        loop {
            let Some(node) = self.entries.get_mut(&id) else {
                return;
            };
            let own = hit_bounds(&node.entry);
            node.bounds = if node.children.is_empty() {
                own
            } else {
                own.merge(node.child_bounds.bounds().translated(node.shift))
            };
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

    fn visit_roots(&self, x: f32, y: f32, emit: &mut impl FnMut(StableNodeId) -> bool) -> bool {
        self.root_bounds.visit(x, y, &mut |slot| {
            self.visit_hits(self.roots[slot].expect("live root bounds"), x, y, emit)
        })
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
            if !self
                .entries
                .get(&id)
                .is_some_and(|node| node.parent == Some(parent))
            {
                continue;
            }
            let node = self.entries.remove(&id).unwrap();
            pending.extend(node.children.into_iter().flatten().map(|child| (child, id)));
        }
    }

    fn remove(&mut self, id: StableNodeId) {
        self.viewport_roots.remove(&id);
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

    fn replace(
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

    fn visit_hits(
        &self,
        id: StableNodeId,
        x: f32,
        y: f32,
        emit: &mut impl FnMut(StableNodeId) -> bool,
    ) -> bool {
        let Some(indexed) = self.entries.get(&id) else {
            return false;
        };
        if !indexed.bounds.contains(x, y) {
            return false;
        }
        let node = &indexed.entry;
        if !node
            .self_clips
            .iter()
            .all(|(bounds, transform)| transformed_contains(*bounds, *transform, [0.0, 0.0], x, y))
        {
            return false;
        }
        let menu_hit = node
            .menu
            .is_some_and(|menu| transformed_contains(menu, node.transform, node.persp, x, y));
        let children_ok = node
            .child_clips
            .iter()
            .all(|(bounds, transform)| transformed_contains(*bounds, *transform, [0.0, 0.0], x, y));
        let menu_z = node.z_index.max(1000);
        let mut emitted_menu = !menu_hit;
        if children_ok
            && indexed
                .child_bounds
                .visit(x - indexed.shift[0], y - indexed.shift[1], &mut |slot| {
                    let child = indexed.children[slot].expect("live child bounds");
                    if !emitted_menu && self.entries[&child].entry.z_index <= menu_z {
                        emitted_menu = true;
                        if emit(id) {
                            return true;
                        }
                    }
                    self.visit_hits(child, x - indexed.shift[0], y - indexed.shift[1], emit)
                })
        {
            return true;
        }

        if !emitted_menu && emit(id) {
            return true;
        }
        if node.hittable && transformed_contains(node.layout, node.transform, node.persp, x, y) {
            return emit(id);
        }
        false
    }
}

pub(super) fn find_hit_transform(index: &HitIndex, id: StableNodeId) -> Option<[f32; 6]> {
    let mut transform = index.entries.get(&id)?.entry.transform;
    let shift = index.inherited_shift(id);
    transform[4] += shift[0];
    transform[5] += shift[1];
    Some(transform)
}

pub(super) fn count_hit_entries(entry: &HitEntry) -> usize {
    1 + entry.children.iter().map(count_hit_entries).sum::<usize>()
}

pub(super) fn retain_hit_tree(index: &mut HitIndex, id: StableNodeId) {
    index.remove(id);
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
    transform: [f32; 6],
    persp: [f32; 2],
    x: f32,
    y: f32,
) -> bool {
    transformed_point(transform, persp, x, y).is_some_and(|(x, y)| bounds.contains(x, y))
}

fn transformed_point(
    [a, b, c, d, e, f]: [f32; 6],
    [g, h]: [f32; 2],
    x: f32,
    y: f32,
) -> Option<(f32, f32)> {
    let det = a * (d - f * h) - c * (b - f * g) + e * (b * h - d * g);
    if !det.is_finite() || det.abs() <= f32::EPSILON {
        return None;
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
        return None;
    }
    let w = ig * x + ih * y + ii;
    if !w.is_finite() || w.abs() < 1e-8 {
        return None;
    }
    let local_x = (ia * x + ic * y + ie) / w;
    let local_y = (ib * x + id * y + if_) / w;
    (local_x.is_finite() && local_y.is_finite()).then_some((local_x, local_y))
}

impl UiWorld {
    /// Current viewport geometry for layout-time anchors. The hit index is
    /// published only after the frame settles, so it cannot serve this query.
    pub(crate) fn viewport_layout_box(&self, target: StableNodeId) -> Option<LayoutBox> {
        if !self.is_mounted(target) {
            return None;
        }
        let mut chain = Vec::new();
        let mut cursor = Some(target);
        while let Some(id) = cursor {
            let node = self.nodes.get(id)?;
            chain.push(id);
            if node.style.layout.position == PositionSpec::Fixed {
                break;
            }
            cursor = node.hierarchy.parent;
        }
        let mut transform = (IDENTITY_AFFINE, [0.0, 0.0]);
        let mut blocks_3d = false;
        for id in chain.into_iter().rev() {
            let node = self.nodes.get(id)?;
            let style = self.motion_layout(id, &node.style.layout);
            let b = node.layout;
            let local = if blocks_3d && style.transform_3d.is_some() {
                (IDENTITY_AFFINE, [0.0, 0.0])
            } else {
                style
                    .world_scene_transform(b.x, b.y, b.width, b.height)
                    .unwrap_or((IDENTITY_AFFINE, [0.0, 0.0]))
            };
            transform = then_hit(transform, local);
            blocks_3d |= style.fails_closed_3d_context();
            if id != target {
                transform = then_hit(
                    transform,
                    (
                        [
                            1.0,
                            0.0,
                            0.0,
                            1.0,
                            -node.scroll_offset.x,
                            -node.scroll_offset.y,
                        ],
                        [0.0, 0.0],
                    ),
                );
            }
        }
        let b = self.layout_box(target)?;
        let ([a, by, c, d, e, f], [g, h]) = transform;
        let mut x = f32::INFINITY;
        let mut y = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        let mut bottom = f32::NEG_INFINITY;
        for (px, py) in [
            (b.x, b.y),
            (b.x + b.width, b.y),
            (b.x, b.y + b.height),
            (b.x + b.width, b.y + b.height),
        ] {
            let w = g * px + h * py + 1.0;
            if !w.is_finite() || w.abs() < 1e-8 {
                return None;
            }
            let tx = (a * px + c * py + e) / w;
            let ty = (by * px + d * py + f) / w;
            if !tx.is_finite() || !ty.is_finite() {
                return None;
            }
            x = x.min(tx);
            y = y.min(ty);
            right = right.max(tx);
            bottom = bottom.max(ty);
        }
        Some(LayoutBox {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }

    /// Map a window point into this node's untransformed layout coordinates.
    /// Uses the current hit projection, including inherited scroll offsets.
    pub fn pointer_layout_position(
        &self,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<(f32, f32)> {
        if !self.is_mounted(target) {
            return None;
        }
        let document = self.document_of(target)?;
        let index = self.hit_test_index.get(&document)?;
        let entry = &index.entries.get(&target)?.entry;
        let shift = index.inherited_shift(target);
        transformed_point(entry.transform, entry.persp, x - shift[0], y - shift[1])
    }

    /// Map layout geometry to window coordinates through the current hit projection.
    pub fn layout_pointer_position(
        &self,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<(f32, f32)> {
        if !self.is_mounted(target) {
            return None;
        }
        let document = self.document_of(target)?;
        let index = self.hit_test_index.get(&document)?;
        let entry = &index.entries.get(&target)?.entry;
        let [a, b, c, d, e, f] = entry.transform;
        let [g, h] = entry.persp;
        let w = g * x + h * y + 1.0;
        if !w.is_finite() || w.abs() < 1e-8 {
            return None;
        }
        let shift = index.inherited_shift(target);
        let result = (
            (a * x + c * y + e) / w + shift[0],
            (b * x + d * y + f) / w + shift[1],
        );
        (result.0.is_finite() && result.1.is_finite()).then_some(result)
    }
}

impl UiWorld {
    // Fixed branches share the document's paint order despite having separate
    // spatial roots. Structural paths compare exactly like preorder ordinals,
    // without walking or numbering every document node on pointer movement.
    fn hit_paint_key(
        &self,
        index: &HitIndex,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Vec<(i32, Vec<usize>)> {
        let shift = index.inherited_shift(target);
        let menu_hit = index.entries.get(&target).is_some_and(|node| {
            node.entry.menu.is_some_and(|menu| {
                transformed_contains(
                    menu,
                    node.entry.transform,
                    node.entry.persp,
                    x - shift[0],
                    y - shift[1],
                )
            })
        });
        let mut chain = Vec::new();
        let mut cursor = Some(target);
        while let Some(id) = cursor {
            chain.push(id);
            cursor = self.parent_id(id);
        }
        let mut path = Vec::new();
        let mut groups = Vec::new();
        let mut inherited_z = 0;
        for id in chain.into_iter().rev() {
            let node = self.record(id);
            let style = &node.style.layout;
            path.push(index.entries.get(&id).map_or(0, |entry| entry.entry.order));
            inherited_z = style.z_index.unwrap_or(inherited_z);
            let children = node
                .hierarchy
                .children
                .iter()
                .any(|child| self.is_mounted(*child));
            let opacity = style.opacity.unwrap_or(1.0).clamp(0.0, 1.0);
            let filter_group = style
                .paint
                .filter
                .filter(|filter| !filter.is_identity())
                .is_some_and(|filter| {
                    filter.blur_radius > 0.0
                        || filter.drop_shadow.is_some()
                        || children
                        || !node.text.value.is_empty()
                        || self.nodes.custom_render(id).is_some()
                });
            let group = (children
                && (style.creates_paint_stacking_context() || (opacity > 0.0 && opacity < 1.0)))
                || filter_group
                || !style.paint.mix_blend.is_normal();
            if group || id == target {
                let z = if id == target && !group && menu_hit {
                    inherited_z.max(1000)
                } else {
                    inherited_z
                };
                groups.push((z, path.clone()));
            }
        }
        groups
    }

    pub fn hit_test_candidates(&self, document: DocumentId, x: f32, y: f32) -> Vec<StableNodeId> {
        let Some(forest) = self.hit_test_index.get(&document) else {
            return Vec::new();
        };
        let mut candidates = Vec::new();
        forest.visit_roots(x, y, &mut |id| {
            candidates.push(id);
            false
        });
        candidates.retain(|id| !self.motion_blocks_input(*id));
        if forest.viewport_hit_at(x, y) {
            candidates
                .sort_by_cached_key(|id| std::cmp::Reverse(self.hit_paint_key(forest, *id, x, y)));
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
        if !self.closing_surfaces.is_empty()
            || self
                .hit_test_index
                .get(&document)
                .is_some_and(|index| index.viewport_hit_at(x, y))
        {
            return self.hit_test_candidates(document, x, y).into_iter().next();
        }
        let forest = self.hit_test_index.get(&document)?;
        let mut found = None;
        forest.visit_roots(x, y, &mut |id| {
            found = Some(id);
            true
        });
        found
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
        let [a, b, c, d, _, _] = node.entry.transform;
        node.shift[0] += a * delta[0] + c * delta[1];
        node.shift[1] += b * delta[0] + d * delta[1];
        index.refresh_bounds(scroller);
    }
}

impl UiWorld {
    /// Whether every input-dirty node is a recorded scroller. Ordinary scroll
    /// dirties only that container; descendant dirtiness means additional
    /// input geometry changed (for example a frozen row's transform).
    /// When true, the frame driver
    /// can patch the hit index in place instead of rebuilding the document.
    pub fn hit_test_work_is_scroll_only(
        &self,
        input: &[StableNodeId],
        updates: &[(StableNodeId, [f32; 2])],
    ) -> bool {
        !updates.is_empty()
            && input
                .iter()
                .all(|node| updates.iter().any(|(scroller, _)| *scroller == *node))
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
    fn build_hit_entries(&self, seeds: Vec<(StableNodeId, [f32; 6])>) -> Vec<BuiltHit> {
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
        let mut built: Vec<BuiltHit> = Vec::new();
        let mut memo = AncestorMemo::default();
        while let Some((id, parent_hit, parent, position, parent_used_pe, parent_blocks_3d)) =
            stack.pop()
        {
            if self.motion_blocks_input(id) {
                continue;
            }
            let style = self.record(id).resolved.0.as_ref();
            if !self.node_has_hit_box(id) {
                continue;
            }
            let layout = self.record(id).layout;
            let motion_layout = self.motion_layout(id, &self.record(id).style.layout);
            let node_style = motion_layout.as_ref();
            // This is a projection root, not a Runtime reparent. Fixed layout
            // is viewport-relative; ancestors still control lifecycle and
            // inherited pointer-events, but cannot clip or scroll this branch.
            let (parent_hit, parent, parent_blocks_3d) =
                if node_style.position == PositionSpec::Fixed {
                    ((IDENTITY_AFFINE, [0.0, 0.0]), None, false)
                } else {
                    (parent_hit, parent, parent_blocks_3d)
                };
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
            let hittable = style.visible
                && interaction.pointer_events
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
            built.push(BuiltHit {
                entry: HitEntry {
                    id,
                    source_children: Arc::clone(&children),
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
        built
    }

    pub(super) fn build_hit_forest(&self, seeds: Vec<(StableNodeId, [f32; 6])>) -> Vec<HitEntry> {
        let built = self.build_hit_entries(seeds);
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
    /// Hidden containers keep structural entries for descendant clip, scroll and
    /// sibling order. Hidden leaves and omitted layout subtrees need no entry.
    pub(super) fn node_has_hit_box(&self, id: StableNodeId) -> bool {
        self.nodes.get(id).is_some_and(|node| {
            node.resolved.0.box_visible
                && (node.resolved.0.visible || !node.hierarchy.children.is_empty())
        })
    }
}

impl UiWorld {
    /// Replace `root`'s entry (and its descendants) in place. Reuses the parent
    /// entry's accumulated transform so no walk from the document root is
    /// needed. Returns `false` when the splice point is missing and the caller
    /// must rebuild the document.
    pub(super) fn patch_hit_subtree(&mut self, document: DocumentId, root: StableNodeId) -> bool {
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
        if self.record(root).style.layout.position == PositionSpec::Fixed {
            let mut forest = self.build_hit_forest(vec![(root, IDENTITY_AFFINE)]);
            if forest.len() > 1 {
                return false;
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
            return true;
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
                return false;
            }
            if !expected {
                return true;
            }
            let position = roots.iter().position(|id| *id == root).unwrap_or_default();
            let mut rebuilt = self.build_hit_forest(vec![(root, IDENTITY_AFFINE)]);
            if rebuilt.len() > 1 {
                return false;
            }
            let Some(mut entry) = rebuilt.pop() else {
                return false;
            };
            entry.order = position;
            sort_hit_children(&mut entry);
            self.note_hit_nodes_built(std::slice::from_ref(&entry));
            let Some(forest) = self.hit_test_index.get_mut(&document) else {
                return false;
            };
            forest.replace(root, None, Some(entry));
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
            // parent omits its layout subtree; otherwise the index is stale.
            return !self.node_has_hit_box(parent);
        };
        let scroll = self.record(parent).scroll_offset;
        let child_transform =
            then_affine(parent_transform, [1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y]);
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
        let Some(position) = position else {
            return false;
        };
        let mut rebuilt = self.build_hit_forest(vec![(root, child_transform)]);
        if rebuilt.len() > 1 {
            return false;
        }
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
        forest.replace(root, Some(parent), entry);
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
        let entries = self.build_hit_entries(seeds);
        let viewport_roots = entries
            .iter()
            .filter(|node| node.parent.is_none() && self.parent_id(node.entry.id).is_some())
            .map(|node| node.entry.id)
            .collect();
        self.bump_last_counters(|counters| counters.record_hit_test_rebuild(entries.len()));
        let mut index = HitIndex::from_entries(entries);
        index.viewport_roots = viewport_roots;
        self.hit_test_index.insert(document, index);
    }
}

#[cfg(test)]
#[path = "hit_test/build_tests.rs"]
mod build_tests;
