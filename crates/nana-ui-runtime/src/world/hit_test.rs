//! Incremental hit index and transformed pointer queries.

use super::*;

mod bounds;
use bounds::{Bounds, BoundsTree};

#[derive(Debug, Clone)]
pub(super) enum HitPathClip {
    Rect(LayoutBox),
    RoundedRect { rect: LayoutBox, radius: f32 },
    Polygon(Vec<[f32; 2]>),
    Ellipse { rect: LayoutBox },
}

impl HitPathClip {
    fn contains(&self, transform: [f32; 6], persp: [f32; 2], x: f32, y: f32) -> bool {
        let Some((x, y)) = transformed_point(transform, persp, x, y) else {
            return false;
        };
        match self {
            Self::Rect(bounds) => bounds.contains(x, y),
            Self::RoundedRect { rect, radius } => rounded_rect_contains(*rect, *radius, x, y),
            Self::Ellipse { rect } => {
                let rx = rect.width * 0.5;
                let ry = rect.height * 0.5;
                if rx <= 0.0 || ry <= 0.0 {
                    return false;
                }
                let dx = (x - (rect.x + rx)) / rx;
                let dy = (y - (rect.y + ry)) / ry;
                dx * dx + dy * dy <= 1.0
            }
            Self::Polygon(points) => point_in_polygon(points, x, y),
        }
    }
}

fn rounded_rect_contains(rect: LayoutBox, radius: f32, x: f32, y: f32) -> bool {
    if !rect.contains(x, y) {
        return false;
    }
    let radius = radius.min(rect.width * 0.5).min(rect.height * 0.5);
    if radius <= 0.0 {
        return true;
    }
    let nearest_x = if x < rect.x + radius {
        rect.x + radius
    } else if x > rect.x + rect.width - radius {
        rect.x + rect.width - radius
    } else {
        x
    };
    let nearest_y = if y < rect.y + radius {
        rect.y + radius
    } else if y > rect.y + rect.height - radius {
        rect.y + rect.height - radius
    } else {
        y
    };
    (x - nearest_x).powi(2) + (y - nearest_y).powi(2) <= radius * radius
}

fn path_clips_contain(
    clips: &[HitPathClip],
    transform: [f32; 6],
    persp: [f32; 2],
    x: f32,
    y: f32,
) -> bool {
    for clip in clips {
        if !clip.contains(transform, persp, x, y) {
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::INPUT_CLIP_REJECTIONS);
            return false;
        }
    }
    true
}

fn point_in_polygon(points: &[[f32; 2]], x: f32, y: f32) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = points[points.len() - 1];
    for &current in points {
        if (current[1] > y) != (previous[1] > y) {
            let at_x = (previous[0] - current[0]) * (y - current[1]) / (previous[1] - current[1])
                + current[0];
            if x < at_x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

fn path_clips_for(style: &nana_ui_core::LayoutStyle, layout: LayoutBox) -> Vec<HitPathClip> {
    let Some(path) = style.paint.clip_path.as_ref() else {
        return Vec::new();
    };
    match path {
        nana_ui_core::ClipPath::Inset(inset) => {
            let [top, right, bottom, left] = inset.resolve_offsets(layout.width, layout.height);
            let rect = LayoutBox {
                x: left,
                y: top,
                width: (layout.width - left - right).max(0.0),
                height: (layout.height - top - bottom).max(0.0),
            };
            let radius = inset.resolve_round(layout.width, layout.height);
            if radius > 0.0 {
                vec![HitPathClip::RoundedRect { rect, radius }]
            } else {
                vec![HitPathClip::Rect(rect)]
            }
        }
        nana_ui_core::ClipPath::Polygon(_) => path
            .resolve_polygon_points(layout.width, layout.height)
            .map_or_else(Vec::new, |points| vec![HitPathClip::Polygon(points)]),
        nana_ui_core::ClipPath::Circle(_) | nana_ui_core::ClipPath::Ellipse(_) => path
            .resolve_ellipse_rect(layout.width, layout.height)
            .map_or_else(Vec::new, |[x, y, width, height]| {
                vec![HitPathClip::Ellipse {
                    rect: LayoutBox {
                        x,
                        y,
                        width,
                        height,
                    },
                }]
            }),
    }
}

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
    /// Entries hanging from their clip parent rather than their parent
    /// (#201). Ids whose entry has since gone or stopped escaping only cost a
    /// paint-order ranking they did not need.
    escaped: HashSet<StableNodeId>,
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

/// Empty strip between trigger and overlay surface on the axis that separates them.
fn overlay_connector_box(trigger: LayoutBox, surface: LayoutBox) -> Option<LayoutBox> {
    const MIN: f32 = 0.5;
    let x = trigger.x.min(surface.x);
    let y = trigger.y.min(surface.y);
    let right = (trigger.x + trigger.width).max(surface.x + surface.width);
    let bottom = (trigger.y + trigger.height).max(surface.y + surface.height);
    let trigger_right = trigger.x + trigger.width;
    let trigger_bottom = trigger.y + trigger.height;
    let surface_right = surface.x + surface.width;
    let surface_bottom = surface.y + surface.height;
    let below = surface.y - trigger_bottom;
    let above = trigger.y - surface_bottom;
    let rightward = surface.x - trigger_right;
    let leftward = trigger.x - surface_right;
    [
        (
            below,
            LayoutBox {
                x,
                y: trigger_bottom,
                width: right - x,
                height: below,
            },
        ),
        (
            above,
            LayoutBox {
                x,
                y: surface_bottom,
                width: right - x,
                height: above,
            },
        ),
        (
            rightward,
            LayoutBox {
                x: trigger_right,
                y,
                width: rightward,
                height: bottom - y,
            },
        ),
        (
            leftward,
            LayoutBox {
                x: surface_right,
                y,
                width: leftward,
                height: bottom - y,
            },
        ),
    ]
    .into_iter()
    .filter(|(gap, _)| *gap > MIN)
    .reduce(|best, next| if next.0 > best.0 { next } else { best })
    .map(|(_, box_)| box_)
}

impl HitIndex {
    fn viewport_hit_at(&self, x: f32, y: f32) -> bool {
        (!self.viewport_roots.is_empty()
            && self.root_bounds.visit(x, y, &mut |slot| {
                self.roots[slot].is_some_and(|id| self.viewport_roots.contains(&id))
            }))
            || self.escaped_hit_at(x, y)
    }

    /// Whether an entry that hangs from its clip parent covers `(x, y)`. Its
    /// place among its new siblings is not its paint order, so such a point
    /// is ranked by paint order as a viewport root's is.
    fn escaped_hit_at(&self, x: f32, y: f32) -> bool {
        self.escaped.iter().any(|id| {
            self.entries.get(id).is_some_and(|indexed| {
                indexed.entry.escaped && {
                    let shift = self.inherited_shift(*id);
                    indexed.bounds.contains(x - shift[0], y - shift[1])
                }
            })
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
        let mut bounds = built
            .iter()
            .map(|node| hit_bounds(&node.entry))
            .collect::<Vec<_>>();
        let mut trees = Vec::with_capacity(n);
        trees.resize_with(n, BoundsTree::default);
        for index in (0..n).rev() {
            let start = offsets[index] as usize;
            let end = offsets[index + 1] as usize;
            if start == end {
                continue;
            }
            let tree = BoundsTree::new(
                child_index[start..end]
                    .iter()
                    .copied()
                    .map(|child| bounds[child]),
            );
            bounds[index] = bounds[index].merge(tree.bounds());
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
        for (index, node) in built.into_iter().enumerate() {
            if node.entry.escaped {
                escaped.insert(node.entry.id);
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
        if entry.escaped {
            self.escaped.insert(id);
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
            if self
                .entries
                .get(&id)
                .is_none_or(|node| node.parent != Some(parent))
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
        if !path_clips_contain(&node.path_clips, node.transform, node.persp, x, y) {
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
        if node.hittable
            && transformed_contains(node.layout, node.transform, node.persp, x, y)
            && painter_accepts(node, node.layout, node.transform, node.persp, x, y)
        {
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
    // Match scene composition and WGSL f32/FMA rounding at the singular edge.
    if !ni.is_finite() || ni.abs() < 1e-6 {
        return (IDENTITY_AFFINE, [0.0, 0.0]);
    }
    let inv = 1.0 / ni;
    (
        [na * inv, nb * inv, nc * inv, nd * inv, ne * inv, nf * inv],
        [ng * inv, nh * inv],
    )
}

/// Whether a painted node's painter takes a point its box already contains.
fn painter_accepts(
    node: &HitEntry,
    layout: LayoutBox,
    transform: [f32; 6],
    persp: [f32; 2],
    x: f32,
    y: f32,
) -> bool {
    let Some(hit) = node.painter.as_deref() else {
        return true;
    };
    let Some((px, py)) = transformed_point(transform, persp, x, y) else {
        return false;
    };
    let local = [px - layout.x, py - layout.y];
    let size = [layout.width, layout.height];
    if let Some(answer) = hit.painter.painter().hit_test(local, size) {
        return answer;
    }
    hit.recording.as_ref().is_none_or(|recording| {
        recording
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(local, size)
    })
}

impl UiWorld {
    fn painter_hit(&self, id: StableNodeId, layout: LayoutBox) -> Option<Arc<PainterHit>> {
        let painter = self.node_painter(id)?;
        let recording = painter
            .painter()
            .hit_painted_outline()
            .then(|| self.shared_recording(id, painter, layout))
            .flatten();
        Some(Arc::new(PainterHit {
            painter: painter.clone(),
            recording,
        }))
    }
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
    // A small but finite determinant is still a valid presentation transform.
    // Reject only a true singularity; using an absolute epsilon here made
    // visible nodes with large logical extents and tiny scales unhittable.
    if !det.is_finite() || det == 0.0 {
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
    // For an affine transform `ii` is exactly the 2x2 determinant.  A small
    // non-zero determinant remains invertible; rejecting it with an absolute
    // epsilon makes large logical surfaces with tiny presentation scales
    // visibly hittable but logically unreachable.  Perspective transforms
    // still reject a zero projective denominator below.
    if !ii.is_finite() || ii == 0.0 {
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

pub(super) fn project_transformed_box(
    bounds: LayoutBox,
    [a, by, c, d, e, f]: [f32; 6],
    [g, h]: [f32; 2],
) -> Option<LayoutBox> {
    let mut x = f32::INFINITY;
    let mut y = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    let mut bottom = f32::NEG_INFINITY;
    for (px, py) in [
        (bounds.x, bounds.y),
        (bounds.x + bounds.width, bounds.y),
        (bounds.x, bounds.y + bounds.height),
        (bounds.x + bounds.width, bounds.y + bounds.height),
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

impl UiWorld {
    pub(super) fn layout_projection_transform(
        &self,
        target: StableNodeId,
    ) -> Option<([f32; 6], [f32; 2])> {
        self.layout_projection_transform_from(target, true)
    }

    /// [`Self::layout_projection_transform`], or with `own` false only what
    /// lies above `target`: its ancestors' transforms and scroll offsets,
    /// not its own transform. A box placed beside `target` (a popover's
    /// hanging surface) follows where `target` sits, not how it animates.
    pub(super) fn layout_projection_transform_from(
        &self,
        target: StableNodeId,
        own: bool,
    ) -> Option<([f32; 6], [f32; 2])> {
        if !self.is_mounted(target) {
            return None;
        }
        // Each ancestor, and whether its scroll reaches `target`: not when
        // `target` (or a box on the way) escapes past it (#201).
        let mut chain = Vec::new();
        let mut escaping = None;
        let mut cursor = if own {
            Some(target)
        } else {
            escaping = self.clip_escape(target);
            self.nodes.get(target)?.hierarchy.parent
        };
        let mut transform = (IDENTITY_AFFINE, [0.0, 0.0]);
        while let Some(id) = cursor {
            let node = self.nodes.get(id)?;
            let passed_over = escaping.is_some_and(|anchor| anchor != Some(id));
            if !passed_over {
                escaping = self.clip_escape(id);
            }
            chain.push((id, passed_over));
            if self.hit_motion_layout(id).position == PositionSpec::Fixed {
                transform = self.hanging_branch_transform(id);
                break;
            }
            cursor = node.hierarchy.parent;
        }
        let mut blocks_3d = false;
        for (id, passed_over) in chain.into_iter().rev() {
            let node = self.nodes.get(id)?;
            let style = self.hit_motion_layout(id);
            let b = self.component_layout_box(id)?;
            let local = self.input_local_scene_transform(id, &style, b, blocks_3d);
            transform = then_hit(transform, self.with_sticky_translation(id, local));
            blocks_3d |= style.fails_closed_3d_context();
            if id != target && !passed_over {
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
        Some(transform)
    }

    /// Current viewport geometry for layout-time anchors. The hit index is
    /// published only after the frame settles, so it cannot serve this query.
    /// Compositor transform overlays follow presentation; layout boxes stay
    /// logical.
    pub(crate) fn viewport_layout_box(&self, target: StableNodeId) -> Option<LayoutBox> {
        self.presentation_input_bounds(target)
    }

    /// Map a window point into this node's untransformed layout coordinates.
    /// Uses the current hit projection when it exists, otherwise the same
    /// ancestor scroll/transform walk as [`Self::viewport_layout_box`].
    pub fn pointer_layout_position(
        &self,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<(f32, f32)> {
        if !self.is_mounted(target) {
            return None;
        }
        if let Some(document) = self.document_of(target)
            && !self.has_compositor_transform_overlay_for(document)
            && let Some(index) = self.hit_test_index.get(&document)
            && let Some(indexed) = index.entries.get(&target)
        {
            let shift = index.inherited_shift(target);
            return transformed_point(
                indexed.entry.transform,
                indexed.entry.persp,
                x - shift[0],
                y - shift[1],
            );
        }
        let (transform, persp) = self.layout_projection_transform(target)?;
        transformed_point(transform, persp, x, y)
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
        if self.has_compositor_transform_overlay_for(document) {
            let ([a, b, c, d, e, f], [g, h]) = self.layout_projection_transform(target)?;
            let w = g * x + h * y + 1.0;
            if !w.is_finite() || w.abs() < 1e-8 {
                return None;
            }
            let result = ((a * x + c * y + e) / w, (b * x + d * y + f) / w);
            return (result.0.is_finite() && result.1.is_finite()).then_some(result);
        }
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

/// Paint order of one hit candidate: its stacking groups from the root down,
/// each a z-index and the length of the sibling-order path that reaches it.
///
/// Every group's path is a prefix of the one path, so the key is that path
/// plus the group cut points. It is compared as the list of `(z, path
/// prefix)` pairs it stands for, lexicographically. Kept as reusable buffers
/// so picking the topmost of several candidates allocates nothing once warm.
#[derive(Debug, Default, Clone)]
pub(super) struct HitPaintKey {
    groups: Vec<(i32, usize)>,
    path: Vec<usize>,
    chain: Vec<StableNodeId>,
}

impl HitPaintKey {
    fn clear(&mut self) {
        self.groups.clear();
        self.path.clear();
        self.chain.clear();
    }
}

impl PartialEq for HitPaintKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for HitPaintKey {}

impl PartialOrd for HitPaintKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HitPaintKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        for (&(z, len), &(other_z, other_len)) in self.groups.iter().zip(&other.groups) {
            let order = z
                .cmp(&other_z)
                .then_with(|| self.path[..len].cmp(&other.path[..other_len]));
            if order.is_ne() {
                return order;
            }
        }
        self.groups.len().cmp(&other.groups.len())
    }
}

/// Scratch for [`UiWorld::hit_test`]: the candidates at the point and the
/// best and current paint keys, reused across queries.
#[derive(Debug, Default)]
pub(super) struct HitScratch {
    candidates: Vec<StableNodeId>,
    best: HitPaintKey,
    current: HitPaintKey,
}

impl UiWorld {
    // Fixed branches are separate spatial roots. Paint already stops the
    // stacking-group prefix at `position: fixed` so menus leave a parent
    // isolation group; hit order must use the same cut, or a later sibling
    // card steals clicks from an open Popover.
    fn hit_paint_key(
        &self,
        index: &HitIndex,
        target: StableNodeId,
        x: f32,
        y: f32,
        key: &mut HitPaintKey,
    ) {
        key.clear();
        if self.triggered_overlays.contains(&target) && self.hanging_surface_contains(target, x, y)
        {
            // The surface is a root-level layer at its content's level, below
            // every item of that content (each has a longer path there).
            key.groups.push((crate::popover::MENU_OVERLAY_Z_INDEX, 0));
            return;
        }
        let menu_hit = self.menu_region_contains(index, target, x, y);
        // An open select menu is a root-level layer at the menu's level, as
        // the scene paints it, ordered by where its field sits.
        let select_menu = menu_hit && self.select_menus.contains(&target);
        let mut cursor = Some(target);
        while let Some(id) = cursor {
            key.chain.push(id);
            if self.hit_motion_layout(id).position == PositionSpec::Fixed {
                break;
            }
            cursor = self.parent_id(id);
        }
        let mut inherited_z = 0;
        for &id in key.chain.iter().rev() {
            let node = self.record(id);
            let style = self.hit_motion_layout(id);
            key.path
                .push(index.entries.get(&id).map_or(0, |entry| entry.entry.order));
            if select_menu {
                continue;
            }
            inherited_z = style.z_index.unwrap_or(inherited_z);
            let children = node
                .hierarchy
                .children
                .iter()
                .any(|child| self.is_mounted(*child));
            let opacity = self
                .sampled_compositor_opacity(id)
                .unwrap_or_else(|| style.opacity.unwrap_or(1.0))
                .clamp(0.0, 1.0);
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
            // A scroll view whose bars can show groups what it holds, so its
            // bars paint over it: hit order cuts there too.
            let scrollbar_chrome = matches!(
                self.nodes.visual(id),
                Some(StandardVisual::Scrollbar { visibility, .. })
                    if *visibility != nana_ui_core::ScrollbarVisibility::Hidden
            );
            let group = (children
                && (style.creates_paint_stacking_context()
                    || (opacity > 0.0 && opacity < 1.0)
                    || scrollbar_chrome))
                || filter_group
                || !style.paint.mix_blend.is_normal();
            if group || id == target {
                let z = if id == target && !group && menu_hit {
                    inherited_z.max(1000)
                } else {
                    inherited_z
                };
                key.groups.push((z, key.path.len()));
            }
        }
        if select_menu {
            let z = self.hit_motion_layout(target).z_index.unwrap_or(0);
            key.groups
                .push((z.max(crate::popover::MENU_OVERLAY_Z_INDEX), key.path.len()));
        }
    }

    /// Whether `target`'s menu region (an open select menu, a hover card's
    /// connector strip) is at the window point `(x, y)`.
    fn menu_region_contains(&self, index: &HitIndex, target: StableNodeId, x: f32, y: f32) -> bool {
        let shift = index.inherited_shift(target);
        let has_overlay = self
            .document_of(target)
            .is_some_and(|document| self.has_compositor_transform_overlay_for(document));
        index.entries.get(&target).is_some_and(|node| {
            node.entry.menu.is_some_and(|menu| {
                if has_overlay {
                    self.layout_projection_transform(target)
                        .is_some_and(|(transform, persp)| {
                            transformed_contains(menu, transform, persp, x, y)
                        })
                } else {
                    transformed_contains(
                        menu,
                        node.entry.transform,
                        node.entry.persp,
                        x - shift[0],
                        y - shift[1],
                    )
                }
            })
        })
    }

    /// Whether an open select menu of `index`'s document is at `(x, y)`, so
    /// the hit there is ranked by paint order rather than taken as the tree
    /// walk first meets one.
    fn select_menu_at(&self, index: &HitIndex, document: DocumentId, x: f32, y: f32) -> bool {
        !self.select_menus.is_empty()
            && self.select_menus.iter().any(|id| {
                self.document_of(*id) == Some(document)
                    && self.menu_region_contains(index, *id, x, y)
            })
    }

    fn visit_presentation_roots(
        &self,
        index: &HitIndex,
        x: f32,
        y: f32,
        emit: &mut impl FnMut(StableNodeId) -> bool,
    ) -> bool {
        for slot in (0..index.roots.len()).rev() {
            let Some(id) = index.roots[slot] else {
                continue;
            };
            let base = (IDENTITY_AFFINE, [0.0, 0.0]);
            let base = if index
                .entries
                .get(&id)
                .is_some_and(|root| root.entry.escaped)
            {
                self.clip_bridge_transform(id, None, base, true)
            } else {
                base
            };
            if self.visit_presentation_hits(index, id, base, false, x, y, emit) {
                return true;
            }
        }
        false
    }

    fn visit_presentation_hits(
        &self,
        index: &HitIndex,
        id: StableNodeId,
        parent_hit: ([f32; 6], [f32; 2]),
        parent_blocks_3d: bool,
        x: f32,
        y: f32,
        emit: &mut impl FnMut(StableNodeId) -> bool,
    ) -> bool {
        let Some(indexed) = index.entries.get(&id) else {
            return false;
        };
        if self.motion_blocks_input(id) || self.inline_box_omitted(id) {
            return false;
        }
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        let style = self.hit_motion_layout(id);
        let (parent_hit, parent_blocks_3d) = if style.position == PositionSpec::Fixed {
            (self.hanging_branch_transform(id), false)
        } else {
            (parent_hit, parent_blocks_3d)
        };
        let Some(layout) = self.component_layout_box(id) else {
            return false;
        };
        let local = self.input_local_scene_transform(id, &style, layout, parent_blocks_3d);
        let (transform, persp) = then_hit(parent_hit, self.with_sticky_translation(id, local));
        let node = &indexed.entry;
        if !node
            .self_clips
            .iter()
            .all(|(bounds, _)| transformed_contains(*bounds, parent_hit.0, parent_hit.1, x, y))
        {
            return false;
        }
        if !path_clips_contain(&node.path_clips, transform, persp, x, y) {
            return false;
        }
        let menu_hit = node
            .menu
            .is_some_and(|menu| transformed_contains(menu, transform, persp, x, y));
        let children_ok = node
            .child_clips
            .iter()
            .all(|(bounds, _)| transformed_contains(*bounds, transform, persp, x, y));
        let child_transform = then_hit(
            (transform, persp),
            (
                [
                    1.0,
                    0.0,
                    0.0,
                    1.0,
                    -record.scroll_offset.x,
                    -record.scroll_offset.y,
                ],
                [0.0, 0.0],
            ),
        );
        let child_blocks_3d = parent_blocks_3d || style.fails_closed_3d_context();
        let menu_z = node.z_index.max(1000);
        let mut emitted_menu = !menu_hit;
        if children_ok {
            for slot in (0..indexed.children.len()).rev() {
                let Some(child) = indexed.children[slot] else {
                    continue;
                };
                if !emitted_menu
                    && index
                        .entries
                        .get(&child)
                        .is_some_and(|child| child.entry.z_index <= menu_z)
                {
                    emitted_menu = true;
                    if emit(id) {
                        return true;
                    }
                }
                // An escaped child takes the transforms of the ancestors it
                // passes over, but none of their scroll.
                let parent_hit = if index
                    .entries
                    .get(&child)
                    .is_some_and(|child| child.entry.escaped)
                {
                    self.clip_bridge_transform(child, Some(id), child_transform, true)
                } else {
                    child_transform
                };
                if self.visit_presentation_hits(
                    index,
                    child,
                    parent_hit,
                    child_blocks_3d,
                    x,
                    y,
                    emit,
                ) {
                    return true;
                }
            }
        }
        if !emitted_menu && emit(id) {
            return true;
        }
        if node.hittable
            && transformed_contains(layout, transform, persp, x, y)
            && painter_accepts(node, layout, transform, persp, x, y)
        {
            return emit(id);
        }
        false
    }

    /// Every hit at `(x, y)`, topmost first.
    pub fn hit_test_candidates(&self, document: DocumentId, x: f32, y: f32) -> Vec<StableNodeId> {
        self.hit_test_queries.set(self.hit_test_queries.get() + 1);
        let mut candidates = Vec::new();
        let Some(forest) = self.collect_hit_candidates(document, x, y, &mut candidates) else {
            return candidates;
        };
        if forest.viewport_hit_at(x, y)
            || self.select_menu_at(forest, document, x, y)
            || self.has_compositor_transform_overlay_for(document)
            || self.hanging_surfaces_at(document, x, y).next().is_some()
        {
            candidates.sort_by_cached_key(|id| {
                let mut key = HitPaintKey::default();
                self.hit_paint_key(forest, *id, x, y, &mut key);
                std::cmp::Reverse(key)
            });
        }
        candidates
    }

    /// Hits at `(x, y)` in visit order, motion-blocked ones dropped. `None`
    /// when the document has no hit index.
    fn collect_hit_candidates(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
        candidates: &mut Vec<StableNodeId>,
    ) -> Option<&HitIndex> {
        candidates.clear();
        let forest = self.hit_test_index.get(&document)?;
        if self.has_compositor_transform_overlay_for(document) {
            self.visit_presentation_roots(forest, x, y, &mut |id| {
                candidates.push(id);
                false
            });
        } else {
            forest.visit_roots(x, y, &mut |id| {
                candidates.push(id);
                false
            });
        }
        // An open menu's surface is not in its trigger's branch of the
        // index: it hangs above the page, out of the trigger's clips.
        for id in self.hanging_surfaces_at(document, x, y) {
            if !candidates.contains(&id) {
                candidates.push(id);
            }
        }
        // 缓存只提供几何；交互资格始终由当前树决定，覆盖帧间关闭和节点释放。
        candidates.retain(|id| {
            !self.motion_blocks_input(*id) && self.validate_pointer_target(document, *id).is_ok()
        });
        Some(forest)
    }

    /// Open triggered menus (Popover, ActionMenu, HoverCard) of `document`
    /// whose surface is at `(x, y)`. The surface paints in the root stacking
    /// context at its content's level, so it takes the pointer there too:
    /// over the page, under its own items.
    fn hanging_surfaces_at(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> impl Iterator<Item = StableNodeId> + '_ {
        self.triggered_overlays.iter().copied().filter(move |id| {
            self.document_of(*id) == Some(document)
                && self.record(*id).interaction.pointer_events
                && self.hanging_surface_contains(*id, x, y)
        })
    }

    pub(crate) fn hanging_surface_contains(&self, id: StableNodeId, x: f32, y: f32) -> bool {
        crate::popover::hanging_surface(self, id).is_some_and(|surface| {
            match self.hanging_surface_transform(id) {
                Some((transform, persp)) => transformed_contains(surface, transform, persp, x, y),
                None => surface.contains(x, y),
            }
        })
    }

    /// Hit-test queries answered since the world was created: one per
    /// [`Self::hit_test`] or [`Self::hit_test_candidates`] call. Input routing
    /// is budgeted against it (at most one per uncaptured pointer event).
    pub fn hit_test_queries(&self) -> u64 {
        self.hit_test_queries.get()
    }
}

impl UiWorld {
    /// Topmost hit at `(x, y)`.
    ///
    /// Walks the same order as [`Self::hit_test_candidates`] but returns at the
    /// first hit, so pointer dispatch on every move does not collect and then
    /// discard the full candidate list.
    pub fn hit_test(&self, document: DocumentId, x: f32, y: f32) -> Option<StableNodeId> {
        self.hit_test_queries.set(self.hit_test_queries.get() + 1);
        if !self.closing_surfaces.is_empty()
            || self.hit_test_index.get(&document).is_some_and(|index| {
                index.viewport_hit_at(x, y) || self.select_menu_at(index, document, x, y)
            })
            || self.hanging_surfaces_at(document, x, y).next().is_some()
        {
            return self.topmost_candidate(document, x, y);
        }
        let forest = self.hit_test_index.get(&document)?;
        let mut found = None;
        if self.has_compositor_transform_overlay_for(document) {
            self.visit_presentation_roots(forest, x, y, &mut |id| {
                if self.validate_pointer_target(document, id).is_err() {
                    return false;
                }
                found = Some(id);
                true
            });
        } else {
            forest.visit_roots(x, y, &mut |id| {
                if self.validate_pointer_target(document, id).is_err() {
                    return false;
                }
                found = Some(id);
                true
            });
        }
        found
    }

    /// The first of [`Self::hit_test_candidates`] without collecting or
    /// sorting them: the highest paint key wins, the earliest visited on a
    /// tie (the stable sort's first). Reuses the world's scratch, so a pointer
    /// moving inside a scroll viewport allocates nothing once warm.
    fn topmost_candidate(&self, document: DocumentId, x: f32, y: f32) -> Option<StableNodeId> {
        let Ok(mut scratch) = self.hit_scratch.try_borrow_mut() else {
            // Only a hit query made from inside another one lands here.
            return self.hit_test_candidates(document, x, y).into_iter().next();
        };
        let HitScratch {
            candidates,
            best,
            current,
        } = &mut *scratch;
        let forest = self.collect_hit_candidates(document, x, y, candidates)?;
        if !forest.viewport_hit_at(x, y)
            && !self.select_menu_at(forest, document, x, y)
            && !self.has_compositor_transform_overlay_for(document)
            && self.hanging_surfaces_at(document, x, y).next().is_none()
        {
            return candidates.first().copied();
        }
        let mut top = None;
        for &id in candidates.iter() {
            self.hit_paint_key(forest, id, x, y, current);
            if top.is_none() || *current > *best {
                std::mem::swap(best, current);
                top = Some(id);
            }
        }
        top
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
        // Entries the scroll rewrites, clips with them, are what the hit
        // index's build sentinel adds while this runs.
        let built = |world: &Self| world.last_counters.hit_test_nodes_rebuilt.unwrap_or(0);
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
        let [a, b, c, d, _, _] = node.entry.transform;
        node.shift[0] += a * delta[0] + c * delta[1];
        node.shift[1] += b * delta[0] + d * delta[1];
        index.refresh_bounds(scroller);
        // Ordinary descendants share one lazy scroll shift. Sticky branches
        // additionally change their local placement, so rebase just those
        // branches against current layout instead of translating them twice.
        let mut sticky = self.sticky_nodes.affected(scroller);
        sticky.sort_by_key(|id| {
            std::iter::successors(self.parent_id(*id), |id| self.parent_id(*id)).count()
        });
        for id in sticky {
            if !self.patch_hit_subtree(document, id) {
                self.rebuild_hit_test(document);
                break;
            }
        }
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
    /// Overlay children receive `position: fixed` only from [`Self::effective_layout_style`].
    pub(super) fn hit_motion_layout(&self, id: StableNodeId) -> Arc<nana_ui_core::LayoutStyle> {
        if self.layout_length_tracks.is_empty()
            && self.overlay_host_nodes.is_empty()
            && self.z_index_nodes == 0
            && self.detached_mounted.is_empty()
            && !self.nodes.has_visuals()
        {
            let record = self.record(id);
            if record.mount == crate::MountState::Mounted
                && !record.resolved_layout.omits_box()
                && !record.resolved_layout.has_logical_box_edges()
            {
                return Arc::clone(&record.resolved_layout);
            }
        }
        self.motion_layout(id, &self.effective_layout_style(id))
    }

    /// Gap between a HoverCard trigger and its overlay; that strip is outside the subtree.
    fn overlay_connector_hit(&self, id: StableNodeId) -> Option<LayoutBox> {
        let parent = self.parent_id(id)?;
        match self.nodes.visual(parent)? {
            StandardVisual::MenuSurface {
                kind: crate::MenuSurfaceKind::HoverCard,
                open: true,
                overlay: Some(overlay),
                ..
            } if self.record(parent).hierarchy.children.first().copied() == Some(id) => {
                {
                    // The card is viewport-fixed; the trigger is wherever the
                    // scroll offsets above it put its layout box.
                    let trigger = self.component_layout_box(parent)?;
                    overlay_connector_box(
                        self.project_input_bounds(parent, trigger)
                            .unwrap_or(trigger),
                        crate::popover::overlay_surface_from_items(self, parent, Some(overlay)),
                    )
                }
            }
            _ => None,
        }
    }

    /// Build hit entries for `seeds` and their visible descendants. Each seed
    /// carries the accumulated transform of its parent, so a scoped patch can
    /// resume from an existing entry instead of walking from the document root.
    ///
    /// `order` is assigned per sibling group from `Hierarchy` position. It is
    /// only ever compared between siblings, so a spliced subtree stays sortable
    /// against untouched siblings without renumbering the document.
    fn build_hit_entries(&self, seeds: Vec<(StableNodeId, [f32; 6])>) -> Vec<BuiltHit> {
        let (mut built, mut deferred) = self.build_hit_entries_deferring(seeds);
        // An escaped box whose clip parent is no node hangs as a root of its
        // own, from the transforms above it but none of their scroll.
        while let Some(id) = deferred.pop() {
            if self.layout_clip_parent(id).is_some() {
                continue;
            }
            let (transform, _) =
                self.clip_bridge_transform(id, None, (IDENTITY_AFFINE, [0.0, 0.0]), false);
            let (more, later) = self.build_hit_entries_deferring(vec![(id, transform)]);
            let base = built.len();
            built.extend(more.into_iter().map(|mut node| {
                node.parent = node.parent.map(|parent| parent + base);
                node
            }));
            if let Some(root) = built.get_mut(base) {
                root.entry.escaped = true;
                root.entry.order = self
                    .parent_id(id)
                    .and_then(|parent| self.child_index(parent, id))
                    .unwrap_or_default();
            }
            deferred.extend(later);
        }
        built
    }

    /// [`Self::build_hit_entries`], leaving out the escaped boxes whose clip
    /// parent this build does not reach (it was seeded below it): those are
    /// returned for the caller to hang where they belong.
    fn build_hit_entries_deferring(
        &self,
        seeds: Vec<(StableNodeId, [f32; 6])>,
    ) -> (Vec<BuiltHit>, Vec<StableNodeId>) {
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
        // What each built entry hands its children: an escaped box resumes
        // from its clip parent's.
        let mut child_states: Vec<(([f32; 6], [f32; 2]), bool)> = Vec::new();
        let mut deferred = Vec::new();
        let mut memo = AncestorMemo::default();
        let visuals = self.nodes.has_visuals();
        let plain_layout = self.layout_length_tracks.is_empty()
            && self.overlay_host_nodes.is_empty()
            && self.z_index_nodes == 0
            && self.detached_mounted.is_empty()
            && !visuals;
        let retained_boxes = self.layout_results.is_empty();
        while let Some((id, parent_hit, parent, position, parent_used_pe, parent_blocks_3d)) =
            stack.pop()
        {
            if self.motion_blocks_input(id) || self.inline_box_omitted(id) {
                continue;
            }
            let (
                visible,
                specified_pointer,
                computed_pointer,
                retained_layout,
                resolved_layout,
                default_box,
                scroll,
                children,
                interaction,
                painted,
            ) = {
                let record = self.record(id);
                let computed = record.resolved.0.as_ref();
                let has_box = computed.box_visible
                    && (computed.visible || !record.hierarchy.children.is_empty());
                if !has_box {
                    continue;
                }
                let default_box = plain_layout
                    && record.mount == crate::MountState::Mounted
                    && crate::components::layout_style_is_default(&record.resolved_layout);
                let plain_style = !default_box
                    && plain_layout
                    && record.mount == crate::MountState::Mounted
                    && !record.resolved_layout.omits_box()
                    && !record.resolved_layout.has_logical_box_edges();
                (
                    computed.visible,
                    record.style.layout.pointer_events,
                    computed.pointer_events,
                    record.layout,
                    plain_style.then(|| Arc::clone(&record.resolved_layout)),
                    default_box,
                    record.scroll_offset,
                    Arc::clone(&record.hierarchy.children),
                    record.interaction,
                    record.style.painter.is_some() || !self.painter_overrides.is_empty(),
                )
            };
            let Some(layout) = (if retained_boxes {
                Some(retained_layout)
            } else {
                self.component_layout_box(id)
            }) else {
                continue;
            };
            // A mounted default box under an identity ancestor has no transform,
            // clip, scroll, or overlay. Skip the style arc and the geometry
            // helpers; descendant seeds stay on the same identity transform.
            if default_box
                && retained_boxes
                && scroll == crate::ScrollOffset::default()
                && parent_hit.0 == IDENTITY_AFFINE
                && parent_hit.1 == [0.0, 0.0]
                && !parent_blocks_3d
                && !painted
                && self.confirm_modals == 0
            {
                let used_pe = PointerEventsSpec::inherit_from(specified_pointer, parent_used_pe);
                let hittable = visible
                    && interaction.pointer_events
                    && used_pe.hittable()
                    && computed_pointer.hittable();
                let index = built.len();
                built.push(BuiltHit {
                    entry: HitEntry {
                        id,
                        source_children: Arc::clone(&children),
                        layout,
                        transform: IDENTITY_AFFINE,
                        persp: [0.0, 0.0],
                        self_clips: Vec::new(),
                        child_clips: Vec::new(),
                        path_clips: Vec::new(),
                        z_index: 0,
                        order: position,
                        hittable,
                        menu: None,
                        painter: None,
                        escaped: false,
                        children: Vec::new(),
                    },
                    parent,
                });
                child_states.push((parent_hit, false));
                stack.extend(children.iter().enumerate().rev().map(|(position, child)| {
                    (*child, parent_hit, Some(index), position, used_pe, false)
                }));
                continue;
            }
            let motion_layout = if let Some(style) = resolved_layout {
                style
            } else {
                self.hit_motion_layout(id)
            };
            let node_style = motion_layout.as_ref();
            // This is a projection root, not a Runtime reparent. Fixed layout
            // is viewport-relative; ancestors still control lifecycle and
            // inherited pointer-events, but cannot clip or scroll this branch.
            //
            // An absolute box that escapes overflow containers hangs from its
            // clip parent instead (#201): their clips and scroll do not reach
            // it, the transforms between still do.
            let (parent_hit, parent, parent_blocks_3d, escaped) =
                if node_style.position == PositionSpec::Fixed {
                    ((IDENTITY_AFFINE, [0.0, 0.0]), None, false, false)
                } else if let Some(tree_parent) = parent
                    && node_style.position == PositionSpec::Absolute
                    && let Some(anchor) = self.clip_escape(id)
                {
                    let mut cursor = Some(tree_parent);
                    while let Some(at) = cursor
                        && Some(built[at].entry.id) != anchor
                    {
                        cursor = built[at].parent;
                    }
                    let Some(at) = cursor else {
                        deferred.push(id);
                        continue;
                    };
                    let (base, blocks_3d) = child_states[at];
                    (
                        self.clip_bridge_transform(id, anchor, base, false),
                        Some(at),
                        blocks_3d,
                        true,
                    )
                } else {
                    (parent_hit, parent, parent_blocks_3d, false)
                };
            let local = if parent_blocks_3d && node_style.transform_3d.is_some() {
                (IDENTITY_AFFINE, [0.0, 0.0])
            } else {
                node_style
                    .world_scene_transform(layout.x, layout.y, layout.width, layout.height)
                    .unwrap_or((IDENTITY_AFFINE, [0.0, 0.0]))
            };
            let (transform, persp) = then_hit(parent_hit, self.with_sticky_translation(id, local));
            let child_transform = then_hit(
                (transform, persp),
                ([1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y], [0.0, 0.0]),
            );
            let child_blocks_3d = parent_blocks_3d || node_style.fails_closed_3d_context();
            // The committed value, as pointer dispatch validates it: a commit
            // that turns it `none` withdraws entries before styles resolve.
            let used_pe = PointerEventsSpec::inherit_from(specified_pointer, parent_used_pe);
            let mut self_clips = Vec::new();
            let mut child_clips = Vec::new();
            let path_clips = path_clips_for(node_style, layout);
            if let Some((x, y, w, h)) =
                node_style.overflow_clip_box(layout.x, layout.y, layout.width, layout.height)
            {
                let overlay_menu = visuals
                    && matches!(
                        self.nodes.visual(id),
                        Some(StandardVisual::MenuSurface {
                            open: true,
                            overlay: Some(_),
                            ..
                        })
                    );
                if !overlay_menu {
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
            }
            if visuals && self.clip_visuals != 0 {
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
            let confirm_busy = self
                .confirm_action_effect(id)
                .is_some_and(|effect| effect.0);
            let hittable = visible
                && interaction.pointer_events
                && used_pe.hittable()
                && computed_pointer.hittable()
                && !confirm_busy;
            let menu = (visuals && hittable)
                .then(|| {
                    self.component_geometry(id)
                        .and_then(|geometry| match geometry {
                            crate::ComponentGeometry::Select {
                                menu: Some(menu), ..
                            } => Some(menu.surface),
                            _ => None,
                        })
                        .or_else(|| self.overlay_connector_hit(id))
                })
                .flatten();
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
                    path_clips,
                    z_index: self.stacking_z_index_memo(id, &mut memo),
                    order: position,
                    hittable,
                    menu,
                    painter: if painted {
                        self.painter_hit(id, layout)
                    } else {
                        None
                    },
                    escaped,
                    children: Vec::new(),
                },
                parent,
            });
            child_states.push((child_transform, child_blocks_3d));
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
        (built, deferred)
    }

    #[cfg(test)]
    pub(super) fn build_hit_forest(&self, seeds: Vec<(StableNodeId, [f32; 6])>) -> Vec<HitEntry> {
        self.build_hit_forest_deferring(seeds).0
    }

    /// [`Self::build_hit_forest`], and the escaped boxes it left for the
    /// caller to hang from a clip parent outside the seeded branch.
    fn build_hit_forest_deferring(
        &self,
        seeds: Vec<(StableNodeId, [f32; 6])>,
    ) -> (Vec<HitEntry>, Vec<StableNodeId>) {
        let (built, deferred) = self.build_hit_entries_deferring(seeds);
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
        (entries.into_iter().flatten().collect::<Vec<_>>(), deferred)
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
    fn patch_hit_branch(
        &mut self,
        document: DocumentId,
        root: StableNodeId,
    ) -> Option<Vec<StableNodeId>> {
        if self.hit_motion_layout(root).position == PositionSpec::Fixed {
            let (mut forest, deferred) =
                self.build_hit_forest_deferring(vec![(root, IDENTITY_AFFINE)]);
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
                self.build_hit_forest_deferring(vec![(root, IDENTITY_AFFINE)]);
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
        let Some(parent_transform) = self
            .hit_test_index
            .get(&document)
            .and_then(|forest| find_hit_transform(forest, parent))
        else {
            // Parent is absent from the index. That is correct only when the
            // parent omits its layout subtree; otherwise the index is stale.
            return (!self.node_has_hit_box(parent)).then(Vec::new);
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
        let position = position?;
        let (mut rebuilt, deferred) =
            self.build_hit_forest_deferring(vec![(root, child_transform)]);
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
        let Some(anchor_transform) = self
            .hit_test_index
            .get(&document)
            .and_then(|forest| find_hit_transform(forest, anchor))
        else {
            return (!self.node_has_hit_box(anchor)).then(Vec::new);
        };
        let scroll = self.record(anchor).scroll_offset;
        let base = then_affine(anchor_transform, [1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y]);
        let (transform, _) =
            self.clip_bridge_transform(root, Some(anchor), (base, [0.0, 0.0]), false);
        let position = self
            .parent_id(root)
            .and_then(|parent| self.child_index(parent, root))?;
        let (mut rebuilt, deferred) = self.build_hit_forest_deferring(vec![(root, transform)]);
        if rebuilt.len() > 1 {
            return None;
        }
        let entry = rebuilt.pop().map(|mut entry| {
            entry.order = position;
            entry.escaped = true;
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
        #[cfg(debug_assertions)]
        self.audit_hit_eligibility("scoped rebuild");
        true
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
    pub(super) fn withdraw_pointer_events_none(&mut self, root: StableNodeId) {
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

    /// A commit moved `root` under a parked parent. Parking a subtree takes
    /// it out of the index; moving one into a parked branch must too.
    pub(super) fn withdraw_parked_hits(&mut self, root: StableNodeId) {
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
    pub(super) fn withdraw_pointer_interaction(&mut self, id: StableNodeId) {
        let document = self.record(id).document;
        self.withdraw_hit_entry(document, id);
    }
}

/// World size up to which commits and hit-index builds re-check the index
/// against the pointer authority under `debug_assertions`. Unit-test worlds
/// are a handful of nodes; product worlds are thousands, and auditing those
/// on every commit would slow debug builds without testing anything the small
/// worlds do not.
#[cfg(debug_assertions)]
const HIT_AUDIT_LIMIT: usize = 512;

impl UiWorld {
    /// Assert that the retained index offers no pointer target the committed
    /// pointer authority ([`Self::validate_pointer_target`]) rejects.
    ///
    /// The index keeps its geometry until the frame rebuilds it, so a commit
    /// that takes a node out of pointer dispatch must withdraw the entry at
    /// once, or a hit between that commit and the next frame lands on it.
    /// Which mutations do that is a judgement no still-frame test checks;
    /// running this after every commit and every build makes the whole suite
    /// a test of it, at O(index × depth), so it is bounded to small worlds.
    #[cfg(debug_assertions)]
    pub(super) fn audit_hit_eligibility(&self, site: &str) {
        if self.nodes.len() > HIT_AUDIT_LIMIT {
            return;
        }
        for (&document, index) in &self.hit_test_index {
            for (&id, indexed) in &index.entries {
                let entry = &indexed.entry;
                if !entry.hittable && entry.menu.is_none() {
                    continue;
                }
                if let Err(error) = self.validate_pointer_target(document, id) {
                    let why = self.nodes.get(id).map(|node| {
                        (
                            self.is_mounted(id),
                            node.interaction.pointer_events,
                            self.used_pointer_events(id),
                            node.resolved.0.pointer_events,
                        )
                    });
                    panic!(
                        "{site}: retained hit index offers {id:?}, which the pointer authority \
                         rejects: {error:?} (mounted, interaction, used, computed) = {why:?}"
                    );
                }
            }
        }
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
        #[cfg(debug_assertions)]
        self.audit_hit_eligibility("rebuild");
    }
}

#[cfg(test)]
#[path = "hit_test/build_tests.rs"]
mod build_tests;

#[cfg(test)]
#[path = "hit_test/presentation_tests.rs"]
mod presentation_tests;

#[cfg(test)]
mod connector_tests {
    use super::{overlay_connector_box, transformed_point};
    use crate::LayoutBox;

    fn box_at(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
        LayoutBox {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn connector_picks_the_separating_axis() {
        let below =
            overlay_connector_box(box_at(10.0, 0.0, 20.0, 10.0), box_at(0.0, 16.0, 40.0, 8.0))
                .unwrap();
        assert_eq!(
            (below.x, below.y, below.width, below.height),
            (0.0, 10.0, 40.0, 6.0)
        );
        let above =
            overlay_connector_box(box_at(10.0, 20.0, 20.0, 10.0), box_at(0.0, 0.0, 40.0, 14.0))
                .unwrap();
        assert_eq!((above.y, above.height), (14.0, 6.0));
        let right =
            overlay_connector_box(box_at(0.0, 0.0, 20.0, 10.0), box_at(26.0, 0.0, 40.0, 10.0))
                .unwrap();
        assert_eq!((right.x, right.width), (20.0, 6.0));
        assert_eq!(
            overlay_connector_box(box_at(0.0, 0.0, 20.0, 10.0), box_at(4.0, 2.0, 40.0, 20.0)),
            None
        );
        let diagonal =
            overlay_connector_box(box_at(0.0, 0.0, 10.0, 10.0), box_at(14.0, 20.0, 10.0, 10.0))
                .unwrap();
        assert_eq!((diagonal.y, diagonal.height), (10.0, 10.0));
        let tied =
            overlay_connector_box(box_at(0.0, 0.0, 10.0, 10.0), box_at(20.0, 20.0, 10.0, 10.0))
                .unwrap();
        assert_eq!((tied.y, tied.height), (10.0, 10.0));
    }

    #[test]
    fn tiny_affine_scale_remains_hittable() {
        let Some((x, y)) = transformed_point(
            [1.0e-4, 0.0, 0.0, 1.0e-4, 0.0, 0.0],
            [0.0, 0.0],
            1.0e-4,
            1.0e-4,
        ) else {
            panic!("tiny affine transforms must remain invertible");
        };
        assert!((x - 1.0).abs() < 1.0e-5);
        assert!((y - 1.0).abs() < 1.0e-5);
    }
}
