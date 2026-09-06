//! Layout-only content extents. Scroll offsets and paint effects do not change
//! the scrollable layout range. Child maxima support shrinking as well as growth.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Extent {
    pub right: f32,
    pub bottom: f32,
}

impl Extent {
    const EMPTY: Self = Self {
        right: f32::NEG_INFINITY,
        bottom: f32::NEG_INFINITY,
    };
    fn merge(self, other: Self) -> Self {
        Self {
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

#[derive(Default)]
struct MaxTree {
    values: Vec<Extent>,
    leaf: usize,
}

impl MaxTree {
    fn new(values: impl ExactSizeIterator<Item = Extent>) -> Self {
        if values.len() == 0 {
            return Self::default();
        }
        let leaf = values.len().next_power_of_two();
        let mut tree = vec![Extent::EMPTY; leaf * 2];
        for (index, value) in values.enumerate() {
            tree[leaf + index] = value;
        }
        for index in (1..leaf).rev() {
            tree[index] = tree[index * 2].merge(tree[index * 2 + 1]);
        }
        Self { values: tree, leaf }
    }
    fn total(&self) -> Extent {
        self.values.get(1).copied().unwrap_or(Extent::EMPTY)
    }
    fn set(&mut self, index: usize, value: Extent) -> usize {
        let mut at = self.leaf + index;
        self.values[at] = value;
        let mut work = 1;
        while at > 1 {
            at /= 2;
            self.values[at] = self.values[at * 2].merge(self.values[at * 2 + 1]);
            work += 1;
        }
        work
    }
}

struct Entry {
    children: Arc<Vec<StableNodeId>>,
    slots: HashMap<StableNodeId, usize>,
    maxima: MaxTree,
    extent: Extent,
}

#[derive(Default)]
pub(super) struct ContentBoundsIndex {
    entries: HashMap<StableNodeId, Entry>,
    dirty: HashSet<StableNodeId>,
    dirty_children: HashMap<StableNodeId, HashSet<StableNodeId>>,
    rebuild: HashSet<StableNodeId>,
    #[cfg(any(test, feature = "benchmark"))]
    refreshed: usize,
    #[cfg(any(test, feature = "benchmark"))]
    range_updates: usize,
}

impl ContentBoundsIndex {
    pub fn topology(
        &mut self,
        world: &UiWorld,
        child: StableNodeId,
        new_parent: Option<StableNodeId>,
    ) {
        if self.entries.is_empty() {
            return;
        }
        self.invalidate(world, child);
        for parent in world.parent_id(child).into_iter().chain(new_parent) {
            if !self.entries.contains_key(&parent) {
                continue;
            }
            self.invalidate(world, parent);
            self.rebuild.insert(parent);
            // A topology rebuild reads all current children. Keeping old dirty
            // edges here could retain IDs moved away and deleted before query.
            self.dirty_children.remove(&parent);
        }
    }

    pub fn invalidate(&mut self, world: &UiWorld, id: StableNodeId) {
        if self.entries.is_empty() {
            return;
        }
        let mut cursor = Some(id);
        while let Some(id) = cursor {
            // A missing entry is built from current authority on first query.
            // Insert already invalidates the nearest indexed parent topology.
            if !self.entries.contains_key(&id) {
                break;
            }
            let parent = world.parent_id(id);
            if let Some(parent) = parent
                && self.entries.contains_key(&parent)
            {
                self.dirty_children.entry(parent).or_default().insert(id);
            }
            if !self.dirty.insert(id) {
                break;
            }
            cursor = parent;
        }
    }

    pub fn remove(&mut self, id: StableNodeId, parent: Option<StableNodeId>) {
        self.entries.remove(&id);
        self.dirty.remove(&id);
        self.dirty_children.remove(&id);
        self.rebuild.remove(&id);
        if let Some(parent) = parent
            && let Some(children) = self.dirty_children.get_mut(&parent)
        {
            children.remove(&id);
        }
        if self.entries.is_empty() {
            self.dirty.clear();
            self.dirty_children.clear();
            self.rebuild.clear();
        }
    }

    pub fn content(&mut self, world: &UiWorld, id: StableNodeId) -> Extent {
        self.refresh(world, id);
        self.entries
            .get(&id)
            .map_or(Extent::EMPTY, |entry| entry.maxima.total())
    }

    fn refresh(&mut self, world: &UiWorld, id: StableNodeId) -> Extent {
        let dirty = self.dirty.remove(&id);
        if !dirty && let Some(entry) = self.entries.get(&id) {
            return entry.extent;
        }
        let Some(record) = world.nodes.get(id) else {
            return Extent::EMPTY;
        };
        #[cfg(any(test, feature = "benchmark"))]
        {
            self.refreshed += 1;
        }
        let children = Arc::clone(&record.hierarchy.children);
        let omitted = record.style.layout.omits_box();
        let layout = record.layout;
        let changed_children = self.dirty_children.remove(&id).unwrap_or_default();
        let rebuild = self.rebuild.remove(&id);
        let previous = self.entries.remove(&id);
        let mut entry = match previous {
            Some(entry) if !rebuild && Arc::ptr_eq(&entry.children, &children) => entry,
            _ => {
                let bounds = children
                    .iter()
                    .map(|child| self.refresh(world, *child))
                    .collect::<Vec<_>>();
                Entry {
                    slots: children
                        .iter()
                        .copied()
                        .enumerate()
                        .map(|(index, id)| (id, index))
                        .collect(),
                    maxima: MaxTree::new(bounds.into_iter()),
                    children,
                    extent: Extent::EMPTY,
                }
            }
        };
        for child in changed_children {
            if let Some(&slot) = entry.slots.get(&child) {
                let extent = self.refresh(world, child);
                let work = entry.maxima.set(slot, extent);
                #[cfg(any(test, feature = "benchmark"))]
                {
                    self.range_updates += work;
                }
                #[cfg(not(any(test, feature = "benchmark")))]
                let _ = work;
            }
        }
        entry.extent = if omitted {
            Extent::EMPTY
        } else {
            Extent {
                right: layout.x + layout.width,
                bottom: layout.y + layout.height,
            }
            .merge(entry.maxima.total())
        };
        let extent = entry.extent;
        self.entries.insert(id, entry);
        extent
    }
}

impl UiWorld {
    /// Diagnostic access to the canonical content index, with per-query work.
    #[cfg(feature = "benchmark")]
    pub fn benchmark_scroll_content_extent(
        &self,
        id: StableNodeId,
    ) -> ((f32, f32), (usize, usize)) {
        let mut index = self.scroll_content_bounds.borrow_mut();
        index.refreshed = 0;
        index.range_updates = 0;
        let extent = index.content(self, id);
        (
            (extent.right, extent.bottom),
            (index.refreshed, index.range_updates),
        )
    }

    pub(crate) fn scroll_content_extent(&self, id: StableNodeId) -> (f32, f32) {
        let extent = self.scroll_content_bounds.borrow_mut().content(self, id);
        (extent.right, extent.bottom)
    }

    pub(super) fn invalidate_scroll_content(&self, id: StableNodeId) {
        self.scroll_content_bounds.borrow_mut().invalidate(self, id);
    }

    pub(super) fn invalidate_scroll_topology(
        &self,
        child: StableNodeId,
        parent: Option<StableNodeId>,
    ) {
        self.scroll_content_bounds
            .borrow_mut()
            .topology(self, child, parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MutationQueue, NodeKind, NodeStyle};

    fn id(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }
    fn rectangle(y: f32) -> crate::LayoutBox {
        crate::LayoutBox {
            x: 0.0,
            y,
            width: 10.0,
            height: 10.0,
        }
    }
    fn fixture(count: u64) -> UiWorld {
        let document = crate::DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut mutations = MutationQueue::new();
        mutations.create(id(1), document, NodeKind::Document);
        for value in 2..count + 2 {
            mutations.create(id(value), document, NodeKind::Text);
            mutations.insert(id(1), id(value), None);
            mutations.write_layout(id(value), rectangle((value - 2) as f32 * 10.0));
        }
        world.commit(mutations).unwrap();
        world
    }

    #[test]
    fn ten_thousand_siblings_shrink_updates_one_ancestor_path() {
        let mut world = fixture(10_000);
        assert_eq!(world.scroll_content_extent(id(1)), (10.0, 100_000.0));
        {
            let mut index = world.scroll_content_bounds.borrow_mut();
            index.refreshed = 0;
            index.range_updates = 0;
        }
        let mut mutations = MutationQueue::new();
        mutations.write_layout(id(10_001), rectangle(0.0));
        world.commit(mutations).unwrap();
        assert_eq!(world.scroll_content_extent(id(1)), (10.0, 99_990.0));
        {
            let index = world.scroll_content_bounds.borrow();
            assert_eq!(index.refreshed, 2);
            assert!(index.range_updates <= 16);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_offset(id(1), crate::ScrollOffset { x: 0.0, y: 5.0 });
        world.commit(mutations).unwrap();
        assert_eq!(world.scroll_content_extent(id(1)), (10.0, 99_990.0));
        assert_eq!(world.scroll_content_bounds.borrow().refreshed, 2);
    }

    fn brute(world: &UiWorld, root: StableNodeId) -> (f32, f32) {
        let mut extent = Extent::EMPTY;
        let mut stack = world.node(root).unwrap().children;
        while let Some(id) = stack.pop() {
            if world.node_style(id).unwrap().layout.omits_box() {
                continue;
            }
            let node = world.node(id).unwrap();
            let layout = world.layout_box(id).unwrap();
            extent = extent.merge(Extent {
                right: layout.x + layout.width,
                bottom: layout.y + layout.height,
            });
            stack.extend(node.children);
        }
        (extent.right, extent.bottom)
    }

    #[test]
    fn cached_extents_match_traversal_after_visibility_reparent_and_deletion() {
        let mut world = fixture(4);
        let mut mutations = MutationQueue::new();
        mutations.insert(id(2), id(5), None);
        world.commit(mutations).unwrap();
        for root in [id(1), id(2), id(3)] {
            assert_eq!(world.scroll_content_extent(root), brute(&world, root));
        }
        let mut hidden = NodeStyle::default();
        Arc::make_mut(&mut hidden.layout).display = Some(nana_ui_core::DisplaySpec::None);
        let mut mutations = MutationQueue::new();
        mutations.set_style(id(2), hidden);
        mutations.write_layout(id(5), rectangle(90.0));
        world.commit(mutations).unwrap();
        assert_eq!(world.scroll_content_extent(id(1)), brute(&world, id(1)));
        let mut mutations = MutationQueue::new();
        mutations.set_style(id(2), NodeStyle::default());
        mutations.insert(id(3), id(5), None);
        mutations.write_layout(id(5), rectangle(120.0));
        world.commit(mutations).unwrap();
        for root in [id(1), id(2), id(3)] {
            assert_eq!(world.scroll_content_extent(root), brute(&world, root));
        }
        let mut mutations = MutationQueue::new();
        mutations.park_subtree(id(3));
        world.commit(mutations).unwrap();
        assert_eq!(world.scroll_content_extent(id(1)), brute(&world, id(1)));
        let mut mutations = MutationQueue::new();
        mutations.insert(id(1), id(3), Some(id(2)));
        mutations.despawn_subtree(id(5));
        world.commit(mutations).unwrap();
        assert_eq!(world.scroll_content_extent(id(1)), brute(&world, id(1)));
        assert!(
            !world
                .scroll_content_bounds
                .borrow()
                .entries
                .contains_key(&id(5))
        );
        let mut mutations = MutationQueue::new();
        mutations.despawn_subtree(id(1));
        world.commit(mutations).unwrap();
        let index = world.scroll_content_bounds.borrow();
        assert!(index.entries.is_empty());
        assert!(index.dirty.is_empty());
        assert!(index.dirty_children.is_empty());
    }

    #[test]
    fn scrolling_churn_releases_entries_and_does_not_refresh_another_document() {
        let mut world = fixture(0);
        let mut mutations = MutationQueue::new();
        mutations.create(
            id(1000),
            crate::DocumentId::new(2).unwrap(),
            NodeKind::Document,
        );
        world.commit(mutations).unwrap();
        world.scroll_content_extent(id(1));
        world.scroll_content_extent(id(1000));
        for value in 2..258 {
            let mut mutations = MutationQueue::new();
            mutations.create(
                id(value),
                crate::DocumentId::new(1).unwrap(),
                NodeKind::Text,
            );
            mutations.insert(id(1), id(value), None);
            mutations.write_layout(id(value), rectangle(value as f32));
            world.commit(mutations).unwrap();
            assert_eq!(
                world.scroll_content_extent(id(1)),
                (10.0, value as f32 + 10.0)
            );
            let before = world.scroll_content_bounds.borrow().refreshed;
            assert_eq!(
                world.scroll_content_extent(id(1000)),
                (f32::NEG_INFINITY, f32::NEG_INFINITY)
            );
            assert_eq!(world.scroll_content_bounds.borrow().refreshed, before);
            let mut mutations = MutationQueue::new();
            mutations.despawn_subtree(id(value));
            world.commit(mutations).unwrap();
            assert_eq!(
                world.scroll_content_extent(id(1)),
                (f32::NEG_INFINITY, f32::NEG_INFINITY)
            );
            let index = world.scroll_content_bounds.borrow();
            assert_eq!(index.entries.len(), 2);
            assert!(index.dirty.is_empty());
            assert!(index.dirty_children.is_empty());
        }
    }

    #[test]
    fn reparent_then_delete_does_not_leave_dirty_ids_in_an_unqueried_old_parent() {
        let mut world = fixture(0);
        let mut mutations = MutationQueue::new();
        mutations.create(
            id(999),
            crate::DocumentId::new(1).unwrap(),
            NodeKind::Document,
        );
        world.commit(mutations).unwrap();
        world.scroll_content_extent(id(1));
        world.scroll_content_extent(id(999));
        for value in 2..258 {
            let mut mutations = MutationQueue::new();
            mutations.create(
                id(value),
                crate::DocumentId::new(1).unwrap(),
                NodeKind::Text,
            );
            mutations.insert(id(1), id(value), None);
            mutations.insert(id(999), id(value), None);
            mutations.write_layout(id(value), rectangle(50.0));
            world.commit(mutations).unwrap();
            assert_eq!(world.scroll_content_extent(id(999)), (10.0, 60.0));
            let mut mutations = MutationQueue::new();
            mutations.despawn_subtree(id(value));
            world.commit(mutations).unwrap();
            world.scroll_content_extent(id(999));
            let index = world.scroll_content_bounds.borrow();
            assert_eq!(index.entries.len(), 2);
            assert!(
                index
                    .dirty_children
                    .values()
                    .flatten()
                    .all(|id| world.contains(*id))
            );
        }
        assert_eq!(world.scroll_content_extent(id(1)), brute(&world, id(1)));
        let index = world.scroll_content_bounds.borrow();
        assert!(index.dirty.is_empty());
        assert!(index.dirty_children.is_empty());
        assert!(index.rebuild.is_empty());
    }

    #[test]
    fn unqueried_document_does_not_allocate_content_index_work() {
        let mut world = fixture(4);
        world.scroll_content_extent(id(1));
        let before = world.scroll_content_bounds.borrow().refreshed;
        let document = crate::DocumentId::new(2).unwrap();
        let mut mutations = MutationQueue::new();
        mutations.create(id(10), document, NodeKind::Document);
        for value in 11..1011 {
            mutations.create(id(value), document, NodeKind::Text);
            mutations.insert(id(10), id(value), None);
            mutations.write_layout(id(value), rectangle(value as f32));
        }
        world.commit(mutations).unwrap();
        {
            let index = world.scroll_content_bounds.borrow();
            assert_eq!(index.entries.len(), 5);
            assert!(index.dirty.is_empty());
            assert!(index.dirty_children.is_empty());
            assert!(index.rebuild.is_empty());
        }
        world.scroll_content_extent(id(1));
        assert_eq!(world.scroll_content_bounds.borrow().refreshed, before);
        assert_eq!(world.scroll_content_extent(id(10)), brute(&world, id(10)));
    }

    #[test]
    fn scalar_geometry_validation_does_not_materialize_a_wide_hierarchy() {
        let mut world = fixture(10_000);
        let mut mutations = MutationQueue::new();
        mutations.write_layout(id(1), rectangle(0.0));
        mutations.set_scroll_offset(id(1), crate::ScrollOffset { x: 0.0, y: 4.0 });
        mutations.set_scroll_metrics(
            id(1),
            Some(crate::ScrollMetrics {
                viewport_width: 10.0,
                viewport_height: 10.0,
                content_width: 10.0,
                content_height: 100_000.0,
            }),
        );
        let mut plan = super::super::mutation::ValidationPlan::new(&world);
        plan.validate(mutations.as_slice()).unwrap();
        assert!(plan.nodes.is_empty());
        drop(plan);
        world.commit(mutations).unwrap();
        assert_eq!(world.scroll_offset(id(1)).unwrap().y, 4.0);
        let before = world.generation();
        let mut mutations = MutationQueue::new();
        mutations.despawn_subtree(id(2));
        mutations.set_scroll_offset(id(2), crate::ScrollOffset { x: 0.0, y: 3.0 });
        assert_eq!(
            world.commit(mutations),
            Err(UiWorldError::MissingNode(id(2)))
        );
        assert!(world.contains(id(2)));
        assert_eq!(world.generation(), before);
        let mut mutations = MutationQueue::new();
        mutations.create(
            id(20_000),
            crate::DocumentId::new(1).unwrap(),
            NodeKind::Text,
        );
        mutations.set_scroll_offset(id(20_000), crate::ScrollOffset { x: 0.0, y: 5.0 });
        world.commit(mutations).unwrap();
        assert_eq!(world.scroll_offset(id(20_000)).unwrap().y, 5.0);
    }
}
