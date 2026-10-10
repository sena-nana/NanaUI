//! Whether a node's subtree reads the block extent it is offered.
//!
//! A box whose height follows its content measures the same whatever height
//! it is offered, unless something in it reads that height: a percentage or
//! fill height, a column that wraps, a grid, a vertical writing mode. The
//! measurement key drops the offered height for a subtree that reads none of
//! it, so a height-only resize finds the box's measurement instead of
//! walking its children again.
//!
//! Each record keeps what its own style says ([`BlockExtentReads::local`]),
//! how its parent's measurement sees it ([`BlockExtentRole`]), and how many
//! of its children pass a read up. A style write reclassifies one node and a
//! child-list edit moves one count; either walks up only while an ancestor's
//! answer changes. Nothing depends on the document, so a parked subtree
//! keeps its answer and coming back costs the walk above it.

use super::*;

/// How a parent's measurement sees a child, for the block extent it offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum BlockExtentRole {
    /// Measured against the extent the parent offers.
    #[default]
    Normal,
    /// Sizes its own height: what it offers its children is its own.
    Cut,
    /// Has no box of its own in the parent's flow (`display: contents`, an
    /// inline that may hoist a block): its children are the parent's.
    Pass,
    /// Not part of the parent's flow measurement: no box, or out of flow.
    Excluded,
}

/// A node's part in whether its ancestors' measurements read the block
/// extent they are offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct BlockExtentReads {
    /// The node's own measurement reads its offered block extent.
    pub(crate) local: bool,
    pub(crate) role: BlockExtentRole,
    /// Children whose subtree passes a read up to this node.
    pub(crate) reading_children: u32,
}

impl BlockExtentReads {
    /// What `style` says, with no children counted.
    pub(crate) fn of(style: &nana_ui_core::LayoutStyle) -> Self {
        Self {
            local: crate::layout_engine::reads_offered_block_extent(style),
            role: role_of(style),
            reading_children: 0,
        }
    }

    /// Whether this node's subtree reads the block extent it is offered.
    pub(crate) fn reads(self) -> bool {
        self.local || self.reading_children > 0
    }

    /// Whether this node passes a read up to its parent.
    pub(crate) fn contributes(self) -> bool {
        matches!(self.role, BlockExtentRole::Normal | BlockExtentRole::Pass) && self.reads()
    }
}

/// The order `collect_flow_children_into` sees a child in.
fn role_of(style: &nana_ui_core::LayoutStyle) -> BlockExtentRole {
    use nana_ui_core::DisplaySpec;
    if style.omits_box() {
        return BlockExtentRole::Excluded;
    }
    if style.display.is_some_and(DisplaySpec::is_contents) {
        return BlockExtentRole::Pass;
    }
    if style.position.is_out_of_flow() {
        return BlockExtentRole::Excluded;
    }
    if style.display == Some(DisplaySpec::Inline) {
        return BlockExtentRole::Pass;
    }
    if style.writing_mode.is_some_and(|mode| mode.is_vertical()) {
        return BlockExtentRole::Normal;
    }
    if crate::layout_engine::sizes_own_height(style) {
        return BlockExtentRole::Cut;
    }
    BlockExtentRole::Normal
}

impl UiWorld {
    /// Whether `id`'s subtree reads the block extent it is offered. A node
    /// that is not here answers yes.
    pub(crate) fn subtree_reads_block_extent(&self, id: StableNodeId) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return true;
        };
        debug_assert_eq!(
            (record.block_reads.local, record.block_reads.role),
            {
                let fresh = BlockExtentReads::of(record.resolved_layout.as_ref());
                (fresh.local, fresh.role)
            },
            "{id:?}'s block extent reads went stale: a resolved layout write skipped refresh_block_extent_reads"
        );
        record.block_reads.reads()
    }

    /// Whether the block extent `id`'s parent offers reaches a read in `id`'s
    /// subtree: `id` passes the extent on and something under it reads it.
    /// A node that is not here answers yes.
    pub(crate) fn block_extent_reaches(&self, id: StableNodeId) -> bool {
        self.nodes
            .get(id)
            .is_none_or(|record| record.block_reads.contributes())
    }

    /// Reclassify `id` after its resolved layout was written, and pass a
    /// change in what it contributes up to its ancestors.
    pub(crate) fn refresh_block_extent_reads(&mut self, id: StableNodeId) {
        let Some(record) = self.nodes.get(id) else {
            return;
        };
        let before = record.block_reads;
        let fresh = BlockExtentReads::of(record.resolved_layout.as_ref());
        if (before.local, before.role) == (fresh.local, fresh.role) {
            return;
        }
        let parent = record.hierarchy.parent;
        let after = BlockExtentReads {
            reading_children: before.reading_children,
            ..fresh
        };
        self.record_mut(id).block_reads = after;
        if let Some(parent) = parent
            && before.contributes() != after.contributes()
        {
            self.bump_block_extent_reads(parent, after.contributes());
        }
    }

    /// `child` joins (`linked`) or leaves `parent`'s children: count its
    /// contribution in or out.
    pub(crate) fn link_block_extent_reads(
        &mut self,
        parent: StableNodeId,
        child: StableNodeId,
        linked: bool,
    ) {
        if self
            .nodes
            .get(child)
            .is_some_and(|record| record.block_reads.contributes())
        {
            self.bump_block_extent_reads(parent, linked);
        }
    }

    /// One more (`up`) or one fewer child of `parent` passes a read up. Walk
    /// up while an ancestor's own contribution changes.
    fn bump_block_extent_reads(&mut self, mut parent: StableNodeId, up: bool) {
        loop {
            #[cfg(test)]
            {
                self.block_read_steps += 1;
            }
            let Some(record) = self.nodes.get_mut(parent) else {
                return;
            };
            let before = record.block_reads.contributes();
            let count = &mut record.block_reads.reading_children;
            if up {
                *count += 1;
            } else {
                debug_assert!(*count > 0, "{parent:?}'s reading children went negative");
                *count = count.saturating_sub(1);
            }
            if record.block_reads.contributes() == before {
                return;
            }
            let Some(next) = record.hierarchy.parent else {
                return;
            };
            parent = next;
        }
    }

    /// Recompute every node's block extent reads from nothing and require the
    /// kept ones to match. O(nodes): the layout-verify guard runs it beside
    /// its full layout.
    #[cfg(any(test, feature = "layout-verify"))]
    pub(crate) fn check_block_extent_reads(&self) {
        // Post-order over every tree, each node once.
        let mut fresh: HashMap<StableNodeId, BlockExtentReads> = HashMap::default();
        let roots: Vec<StableNodeId> = self
            .nodes
            .keys()
            .filter(|id| self.record(*id).hierarchy.parent.is_none())
            .collect();
        for root in roots {
            let mut stack = vec![(root, false)];
            while let Some((id, expanded)) = stack.pop() {
                let record = self.record(id);
                if !expanded {
                    stack.push((id, true));
                    stack.extend(
                        record
                            .hierarchy
                            .children
                            .iter()
                            .map(|child| (*child, false)),
                    );
                    continue;
                }
                let mut reads = BlockExtentReads::of(record.resolved_layout.as_ref());
                reads.reading_children = record
                    .hierarchy
                    .children
                    .iter()
                    .filter(|child| fresh[*child].contributes())
                    .count() as u32;
                assert_eq!(
                    record.block_reads, reads,
                    "{id:?}'s kept block extent reads drifted from its subtree"
                );
                fresh.insert(id, reads);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec};

    use super::super::reflow_oracle::{Builder, styled};
    use super::*;

    fn column() -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Fill),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        }
    }

    fn percent() -> LayoutStyle {
        LayoutStyle {
            height: Some(LengthSpec::Percent(50.0)),
            ..LayoutStyle::default()
        }
    }

    fn fixed_height() -> LayoutStyle {
        LayoutStyle {
            height: Some(LengthSpec::Px(40.0)),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        }
    }

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    /// A chain of `depth` columns under the root; returns the world and the
    /// chain from the top down.
    fn chain(depth: usize) -> (UiWorld, Vec<StableNodeId>) {
        let (mut b, root) = Builder::new(document(), 1);
        let mut ids = Vec::new();
        let mut parent = root;
        for _ in 0..depth {
            parent = b.element(parent, column());
            ids.push(parent);
        }
        let mut world = UiWorld::new();
        world.commit(b.queue).unwrap();
        (world, ids)
    }

    fn restyle(world: &mut UiWorld, id: StableNodeId, layout: LayoutStyle) {
        let mut queue = MutationQueue::new();
        queue.set_style(id, styled(layout));
        world.commit(queue).unwrap();
    }

    /// A percentage at the bottom of a chain reaches every ancestor, and
    /// leaves them again when it goes; a fixed height half way up cuts the
    /// ones above it. Each walk visits the chain once.
    #[test]
    fn a_read_deep_in_a_chain_reaches_every_ancestor_until_a_cut() {
        let (mut world, ids) = chain(12);
        let leaf = *ids.last().unwrap();
        assert!(ids.iter().all(|id| !world.subtree_reads_block_extent(*id)));
        let steps = world.block_read_steps;
        restyle(&mut world, leaf, percent());
        assert!(ids.iter().all(|id| world.subtree_reads_block_extent(*id)));
        assert!(world.block_read_steps - steps <= ids.len() as u64 + 1);
        world.check_block_extent_reads();

        restyle(&mut world, ids[5], fixed_height());
        assert!(
            ids[5..]
                .iter()
                .all(|id| world.subtree_reads_block_extent(*id))
        );
        assert!(
            ids[..5]
                .iter()
                .all(|id| !world.subtree_reads_block_extent(*id))
        );
        world.check_block_extent_reads();

        restyle(&mut world, ids[5], column());
        restyle(&mut world, leaf, column());
        assert!(ids.iter().all(|id| !world.subtree_reads_block_extent(*id)));
        world.check_block_extent_reads();
    }

    /// Two children that read: the parent reads while either stays, and not
    /// once both are gone.
    #[test]
    fn a_parent_reads_while_any_child_passes_a_read_up() {
        let (mut b, root) = Builder::new(document(), 1);
        let parent = b.element(root, column());
        let first = b.element(parent, percent());
        let second = b.element(parent, percent());
        let mut world = UiWorld::new();
        world.commit(b.queue).unwrap();
        assert!(world.subtree_reads_block_extent(parent));
        let mut queue = MutationQueue::new();
        queue.detach(first);
        world.commit(queue).unwrap();
        assert!(world.subtree_reads_block_extent(parent));
        let mut queue = MutationQueue::new();
        queue.despawn_subtree(second);
        world.commit(queue).unwrap();
        assert!(!world.subtree_reads_block_extent(parent));
        world.check_block_extent_reads();
    }

    /// Moving a reading child moves the read; reordering it under the same
    /// parent changes nothing.
    #[test]
    fn a_moved_child_takes_its_read_along() {
        let (mut b, root) = Builder::new(document(), 1);
        let left = b.element(root, column());
        let right = b.element(root, column());
        let reader = b.element(left, percent());
        let other = b.element(left, column());
        let mut world = UiWorld::new();
        world.commit(b.queue).unwrap();
        assert!(world.subtree_reads_block_extent(left));
        assert!(!world.subtree_reads_block_extent(right));

        let mut queue = MutationQueue::new();
        queue.insert(left, reader, None);
        world.commit(queue).unwrap();
        assert!(world.subtree_reads_block_extent(left));
        world.check_block_extent_reads();

        let mut queue = MutationQueue::new();
        queue.insert(right, reader, None);
        world.commit(queue).unwrap();
        assert!(!world.subtree_reads_block_extent(left));
        assert!(world.subtree_reads_block_extent(right));
        assert!(!world.subtree_reads_block_extent(other));
        world.check_block_extent_reads();
    }

    /// A parked subtree keeps what it reads; putting it back costs the walk
    /// above it, not one over the subtree.
    #[test]
    fn a_parked_subtree_comes_back_with_its_read() {
        let (mut world, ids) = chain(4);
        let (mut b, _) = Builder::new(document(), 100);
        let subtree = b.detached(column());
        let mut parent = subtree;
        for _ in 0..50 {
            parent = b.element(parent, column());
        }
        b.element(parent, percent());
        world.commit(b.queue).unwrap();
        assert!(world.subtree_reads_block_extent(subtree));

        let mut queue = MutationQueue::new();
        queue.insert(ids[3], subtree, None);
        world.commit(queue).unwrap();
        assert!(ids.iter().all(|id| world.subtree_reads_block_extent(*id)));

        let mut queue = MutationQueue::new();
        queue.park_subtree(subtree);
        world.commit(queue).unwrap();
        assert!(ids.iter().all(|id| !world.subtree_reads_block_extent(*id)));
        assert!(world.subtree_reads_block_extent(subtree));
        world.check_block_extent_reads();

        let steps = world.block_read_steps;
        let mut queue = MutationQueue::new();
        queue.insert(ids[3], subtree, None);
        world.commit(queue).unwrap();
        assert!(ids.iter().all(|id| world.subtree_reads_block_extent(*id)));
        assert!(world.block_read_steps - steps <= ids.len() as u64 + 1);
        world.check_block_extent_reads();
    }

    /// Keeping the reads current costs the same in a document of 1k nodes
    /// as of 100k: a flip deep in a chain walks the chain, never the page.
    #[test]
    fn upkeep_costs_the_same_at_1k_and_100k_nodes() {
        let mut costs = Vec::new();
        for filler in [1_000u64, 100_000] {
            let (mut b, root) = Builder::new(document(), 1);
            let page = b.element(root, column());
            for _ in 0..filler {
                b.element(page, column());
            }
            let mut parent = page;
            let mut chain = Vec::new();
            for _ in 0..8 {
                parent = b.element(parent, column());
                chain.push(parent);
            }
            let mut world = UiWorld::new();
            world.commit(b.queue).unwrap();
            let leaf = *chain.last().unwrap();
            let steps = world.block_read_steps;
            restyle(&mut world, leaf, percent());
            restyle(&mut world, leaf, column());
            let mut queue = MutationQueue::new();
            queue.detach(chain[0]);
            world.commit(queue).unwrap();
            costs.push(world.block_read_steps - steps);
        }
        assert_eq!(costs[0], costs[1]);
    }

    /// What a node is with nothing set matches the default every record
    /// starts with.
    #[test]
    fn a_new_record_starts_where_its_default_style_puts_it() {
        assert_eq!(
            BlockExtentReads::of(&NodeStyle::default().layout),
            BlockExtentReads::default()
        );
    }
}
