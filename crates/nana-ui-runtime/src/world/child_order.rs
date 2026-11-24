//! A parent's children put into a new order as one edit
//! ([`UiMutation::ReorderChildren`]).
//!
//! Moving children one `Insert` at a time finds each in the list and shifts
//! the rest, in the batch's validation and again when it is applied, so a
//! list reversed or sorted cost the square of its length: 16,000 rows took
//! 0.26 s. One reorder checks the list once and writes it once.

use super::mutation::ValidationPlan;
use super::*;

/// How many children `current` and `ordered` start with in common, and how
/// many of the rest they end with in common.
pub(crate) fn common_ends(current: &[StableNodeId], ordered: &[StableNodeId]) -> (usize, usize) {
    let head = current
        .iter()
        .zip(ordered)
        .take_while(|(a, b)| a == b)
        .count();
    let tail = current[head..]
        .iter()
        .rev()
        .zip(ordered[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (head, tail)
}

/// Marks one longest strictly increasing subsequence of the known positions
/// (patience sorting, O(n log n)).
pub(crate) fn longest_increasing(positions: &[Option<usize>]) -> Vec<bool> {
    // tails[k]: index in `positions` ending the best subsequence of length k+1.
    let mut tails: Vec<usize> = Vec::new();
    let mut previous: Vec<Option<usize>> = vec![None; positions.len()];
    for (index, position) in positions.iter().enumerate() {
        let Some(position) = *position else {
            continue;
        };
        let length = tails.partition_point(|&tail| {
            positions[tail].expect("tails hold known positions") < position
        });
        if length > 0 {
            previous[index] = Some(tails[length - 1]);
        }
        if length == tails.len() {
            tails.push(index);
        } else {
            tails[length] = index;
        }
    }
    let mut stays = vec![false; positions.len()];
    let mut cursor = tails.last().copied();
    while let Some(index) = cursor {
        stays[index] = true;
        cursor = previous[index];
    }
    stays
}

/// For each child of `ordered`, where it is in `current`, if it is there.
pub(crate) fn positions_in(
    current: &[StableNodeId],
    ordered: &[StableNodeId],
) -> Vec<Option<usize>> {
    let position: NodeMap<usize> = current
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    ordered.iter().map(|id| position.get(id).copied()).collect()
}

impl ValidationPlan<'_> {
    /// `order` must hold the children `parent` has when the batch reaches
    /// this mutation, each once.
    pub(super) fn reorder(
        &mut self,
        parent: StableNodeId,
        order: &[StableNodeId],
    ) -> Result<(), UiWorldError> {
        let invalid = UiWorldError::InvalidChildOrder { parent };
        let children = &self.node(parent)?.children;
        if children.len() != order.len() {
            return Err(invalid);
        }
        // The children both lists start and end with are the same ones in
        // the same places; only the ones between can be missing or repeated.
        let (head, tail) = common_ends(children, order);
        let between = head..order.len() - tail;
        let mut unplaced: NodeSet = children[between.clone()].iter().copied().collect();
        if !order[between.clone()]
            .iter()
            .all(|child| unplaced.remove(child))
        {
            return Err(invalid);
        }
        // No parent changes and no depth does: written past `node_mut`, so
        // the batch keeps reading depths the world already has.
        self.nodes
            .get_mut(&parent)
            .expect("read just above")
            .children[between.clone()]
        .copy_from_slice(&order[between]);
        Ok(())
    }
}

impl UiWorld {
    /// Whether `parent`'s children are in `order` already.
    pub(super) fn children_in_order(&self, parent: StableNodeId, order: &[StableNodeId]) -> bool {
        self.nodes
            .get(parent)
            .is_some_and(|node| node.hierarchy.children.as_slice() == order)
    }

    /// Applies [`UiMutation::ReorderChildren`].
    ///
    /// The children outside one longest run that keeps its order are the
    /// ones that moved: each is told so as an `Insert` under the parent it
    /// already has tells it, and the rest are left alone, as they are when
    /// a sibling is moved past them. Nothing changes parent, depth, mount
    /// state or document, so none of that is visited.
    pub(super) fn reorder_children(
        &mut self,
        parent: StableNodeId,
        order: &[StableNodeId],
        report: &mut CommitReport,
    ) {
        let (between, moved) = {
            let current = &self.record(parent).hierarchy.children;
            let (head, tail) = common_ends(current, order);
            let between = head..order.len() - tail;
            let stays = longest_increasing(&positions_in(
                &current[between.clone()],
                &order[between.clone()],
            ));
            let moved: Vec<StableNodeId> = order[between.clone()]
                .iter()
                .zip(stays)
                .filter(|(_, stays)| !stays)
                .map(|(child, _)| *child)
                .collect();
            (between, moved)
        };
        // Read while each child is still where it was.
        for &child in &moved {
            self.invalidate_scroll_topology(child, Some(parent));
        }
        self.scroll.layout_touched = true;
        self.record_child_list_invalidation(parent);
        for &child in &moved {
            self.clear_layout_results_subtree(child);
        }
        let hierarchy = self.hierarchy_mut(parent);
        Arc::make_mut(&mut hierarchy.children)[between.clone()]
            .copy_from_slice(&order[between.clone()]);
        #[cfg(test)]
        {
            self.tree.child_list_steps += between.len() as u64;
        }
        self.renumber_children_between(parent, between.start, order.len() - between.end);
        for &child in &moved {
            // The subtree goes with its root and keeps what it inherits and
            // its own geometry until layout writes back what changed.
            self.mark(
                child,
                DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
            );
        }
        self.mark_ancestors(
            parent,
            DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
        );
        self.record_child_list_invalidation(parent);
        report.reparented += moved.len();
        self.note_structural_change(parent);
    }
}

#[cfg(test)]
mod tests {
    use super::super::reflow_oracle::{
        assert_matches_cold, bundled_face_shaper, column, fixed, node, product_frame, styled,
    };
    use super::*;
    use crate::{AppContext, LayoutViewport, MutationQueue, TextContent};
    use nana_ui_core::{LengthSpec, OverflowSpec};

    const PARENT: u64 = 2;
    const FIRST: u64 = 10;
    const LABELS: u64 = 100_000;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(700.0, 600.0)
    }

    fn add_row(queue: &mut MutationQueue, row: u64) {
        queue.create(
            node(row),
            document(),
            NodeKind::Element { tag: "div".into() },
        );
        queue.insert(node(PARENT), node(row), None);
        // No two neighbours the same height: a row in another place is at
        // another offset, and so is every row after it.
        queue.set_style(
            node(row),
            styled(fixed(200.0, 12.0 + (row % 7) as f32 * 3.0)),
        );
        let label = node(LABELS + row);
        queue.create(label, document(), NodeKind::Text);
        queue.insert(node(row), label, None);
        queue.set_text(
            label,
            TextContent {
                value: format!("row {row}").into(),
            },
        );
    }

    /// A column of labelled rows in `order`, laid out once. One that
    /// `scrolls` is shorter than its rows and keeps a scrolling area.
    fn rows(order: &[u64], scrolls: bool) -> AppContext {
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(), NodeKind::Document);
        queue.create(
            node(PARENT),
            document(),
            NodeKind::Element { tag: "div".into() },
        );
        queue.insert(node(1), node(PARENT), None);
        let mut parent = column(Some(400.0));
        if scrolls {
            parent.height = Some(LengthSpec::Px(240.0));
            parent.overflow_y = OverflowSpec::Scroll;
        }
        queue.set_style(node(PARENT), styled(parent));
        for &row in order {
            add_row(&mut queue, row);
        }
        let mut cx = AppContext::new();
        cx.commit_mutations(queue).unwrap();
        product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
        cx.world_mut().take_system_work();
        cx
    }

    fn ids(order: &[u64]) -> Vec<StableNodeId> {
        order.iter().map(|row| node(*row)).collect()
    }

    /// The parent's children, each checked to sit at the index it records.
    fn children(world: &UiWorld) -> Vec<StableNodeId> {
        let children = world.node(node(PARENT)).unwrap().children.to_vec();
        for (index, id) in children.iter().enumerate() {
            assert_eq!(
                world.recorded_child_index(node(PARENT), *id),
                Some(index),
                "{id:?}'s recorded index"
            );
        }
        children
    }

    /// The moves the framework queued before there was a reorder: the
    /// children outside the longest kept run, each before its successor.
    fn insert_per_moved_child(
        current: &[StableNodeId],
        ordered: &[StableNodeId],
        queue: &mut MutationQueue,
    ) {
        let (head, tail) = common_ends(current, ordered);
        let end = ordered.len() - tail;
        let stays = longest_increasing(&positions_in(&current[head..end], &ordered[head..end]));
        let mut next = ordered.get(end).copied();
        for (index, &child) in ordered[head..end].iter().enumerate().rev() {
            if !stays[index] {
                queue.insert(node(PARENT), child, next);
            }
            next = Some(child);
        }
    }

    fn shuffled(mut order: Vec<u64>, mut seed: u64) -> Vec<u64> {
        for index in (1..order.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            order.swap(index, (seed % (index as u64 + 1)) as usize);
        }
        order
    }

    /// The same rows in orders that move one row, a few, a block and all.
    fn orders(all: &[u64]) -> Vec<(&'static str, Vec<u64>)> {
        let n = all.len();
        let mut reversed = all.to_vec();
        reversed.reverse();
        let mut last_first = all.to_vec();
        last_first.rotate_right(1);
        let mut swapped = all.to_vec();
        swapped.swap(3, n - 4);
        let mut halves = all.to_vec();
        halves.rotate_left(n / 2);
        let mut middle_reversed = all.to_vec();
        middle_reversed[n / 4..3 * n / 4].reverse();
        vec![
            ("reversed", reversed),
            ("last row first", last_first),
            ("two rows swapped", swapped),
            ("halves exchanged", halves),
            ("middle reversed", middle_reversed),
            ("shuffled", shuffled(all.to_vec(), 0x2545_f491_4f6c_dd1d)),
            (
                "shuffled again",
                shuffled(all.to_vec(), 0x9e37_79b9_7f4a_7c15),
            ),
        ]
    }

    /// What a commit left for the frame after it: the parents whose child
    /// lists it says changed, which nodes still have a published result and
    /// which a scrolling area that is current, and the frame's work down to
    /// each dirty node.
    fn left(world: &mut UiWorld) -> impl PartialEq + std::fmt::Debug {
        world.flush_scroll_content();
        let nodes: Vec<_> = world
            .document_order(document())
            .into_iter()
            .map(|id| {
                (
                    id,
                    world.has_local_layout_result(id),
                    world.scroll.content_bounds.borrow().stale(id),
                )
            })
            .collect();
        (
            world.take_structural_change_parents(),
            nodes,
            world.take_system_work(),
        )
    }

    /// One reorder leaves a world what an insert per moved child leaves it:
    /// the same children and the same things for the next frame to do.
    /// After that frame the rows lie as they do when they are built in that
    /// order.
    #[test]
    fn a_reorder_leaves_what_an_insert_per_moved_child_leaves() {
        let all: Vec<u64> = (FIRST..FIRST + 60).collect();
        for scrolls in [false, true] {
            for (shape, order) in orders(&all) {
                let shape = format!("{shape}, scrolls: {scrolls}");
                let mut moves = MutationQueue::new();
                insert_per_moved_child(&ids(&all), &ids(&order), &mut moves);
                let mut reorder = MutationQueue::new();
                reorder.reorder_children(node(PARENT), ids(&order));

                let (mut moved, mut reordered) = (rows(&all, scrolls), rows(&all, scrolls));
                let moves_queued = moves.len();
                let by_moves = moved.world_mut().commit_ref(&moves).unwrap();
                let by_reorder = reordered.world_mut().commit_ref(&reorder).unwrap();
                assert_eq!(children(reordered.world()), ids(&order), "{shape}");
                assert_eq!(children(moved.world()), ids(&order), "{shape}");
                assert_eq!(by_reorder.reparented, moves_queued, "{shape}");
                assert_eq!(by_reorder.reparented, by_moves.reparented, "{shape}");
                assert_eq!(
                    left(reordered.world_mut()),
                    left(moved.world_mut()),
                    "{shape}"
                );

                // What was left has been taken to be compared, so the frame
                // runs on a world that still has it.
                let mut framed = rows(&all, scrolls);
                framed.commit_mutations(reorder).unwrap();
                product_frame(
                    &mut framed,
                    document(),
                    viewport(),
                    &mut bundled_face_shaper(),
                );
                assert_matches_cold(&mut framed, &mut rows(&order, scrolls), document());
            }
        }
    }

    #[test]
    fn an_order_that_is_not_the_children_each_once_is_refused() {
        let all: Vec<u64> = (FIRST..FIRST + 8).collect();
        let mut cx = rows(&all, false);
        let outsider = node(900);
        let mut queue = MutationQueue::new();
        queue.create(
            outsider,
            document(),
            NodeKind::Element { tag: "div".into() },
        );
        cx.commit_mutations(queue).unwrap();
        let before = children(cx.world());
        let generation = cx.world().generation();

        let mut missing = before.clone();
        missing.remove(4);
        let mut extra = before.clone();
        extra.insert(4, outsider);
        let mut repeated = before.clone();
        repeated[5] = repeated[2];
        let mut replaced = before.clone();
        replaced[5] = outsider;
        let mut names_parent = before.clone();
        names_parent[0] = node(PARENT);
        for (shape, order) in [
            ("one left out", missing),
            ("one brought in", extra),
            ("one named twice", repeated),
            ("one exchanged for an outsider", replaced),
            ("the parent itself", names_parent),
            ("none", Vec::new()),
        ] {
            let mut queue = MutationQueue::new();
            queue.reorder_children(node(PARENT), order);
            assert_eq!(
                cx.world_mut().commit(queue),
                Err(UiWorldError::InvalidChildOrder {
                    parent: node(PARENT)
                }),
                "{shape}"
            );
            assert_eq!(children(cx.world()), before, "{shape}");
            assert_eq!(cx.world().generation(), generation, "{shape}");
        }
        let mut queue = MutationQueue::new();
        queue.reorder_children(node(901), before.clone());
        assert_eq!(
            cx.world_mut().commit(queue),
            Err(UiWorldError::MissingNode(node(901)))
        );
    }

    #[test]
    fn the_order_the_children_are_in_changes_nothing() {
        let all: Vec<u64> = (FIRST..FIRST + 8).collect();
        let mut cx = rows(&all, false);
        let generation = cx.world().generation();
        let mut queue = MutationQueue::new();
        queue.reorder_children(node(PARENT), ids(&all));
        let report = cx.commit_mutations(queue).unwrap();
        assert_eq!(report.reparented, 0);
        assert_eq!(cx.world().generation(), generation);
        assert!(cx.world_mut().take_system_work().is_empty());
    }

    /// The order is checked against the children the batch has left when it
    /// reaches the reorder: one it brought in is named, one it took out is
    /// not.
    #[test]
    fn a_reorder_names_the_children_its_batch_has_left() {
        let all: Vec<u64> = (FIRST..FIRST + 8).collect();
        let mut cx = rows(&all, false);
        let fresh = FIRST + 100;
        let stale: Vec<u64> = all.iter().rev().copied().collect();
        let mut wanted: Vec<u64> = stale.iter().copied().filter(|row| *row != all[3]).collect();
        wanted.insert(2, fresh);

        let batch = |order: &[u64]| {
            let mut queue = MutationQueue::new();
            add_row(&mut queue, fresh);
            queue.park_subtree(node(all[3]));
            queue.reorder_children(node(PARENT), ids(order));
            queue
        };
        let before = children(cx.world());
        assert_eq!(
            cx.commit_mutations(batch(&stale))
                .map(|_| ())
                .map_err(|_| ()),
            Err(()),
            "the order still names the parked row and not the new one"
        );
        assert_eq!(children(cx.world()), before);
        assert!(!cx.world().contains(node(fresh)));

        cx.commit_mutations(batch(&wanted)).unwrap();
        assert_eq!(children(cx.world()), ids(&wanted));
        assert_eq!(cx.world().parent_id(node(all[3])), None);
        product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
        let mut cold = rows(&wanted, false);
        // The parked row is still in the document it left the tree of.
        let mut queue = MutationQueue::new();
        add_row(&mut queue, all[3]);
        queue.park_subtree(node(all[3]));
        cold.commit_mutations(queue).unwrap();
        product_frame(
            &mut cold,
            document(),
            viewport(),
            &mut bundled_face_shaper(),
        );
        assert_matches_cold(&mut cx, &mut cold, document());
    }

    /// Reordering a parent's children reads and writes its list a constant
    /// number of times, whatever moves: an insert per moved child shifted
    /// the list for each, and reversing it cost the square of its length.
    #[test]
    fn reordering_n_children_visits_n() {
        const CHILDREN: u64 = 4_000;
        let all: Vec<u64> = (FIRST..FIRST + CHILDREN).collect();
        let wide = || {
            let mut world = UiWorld::new();
            let mut queue = MutationQueue::new();
            queue.create(node(1), document(), NodeKind::Document);
            for id in std::iter::once(PARENT).chain(all.iter().copied()) {
                queue.create(
                    node(id),
                    document(),
                    NodeKind::Element { tag: "div".into() },
                );
                queue.insert(node(if id == PARENT { 1 } else { PARENT }), node(id), None);
            }
            world.commit(queue).unwrap();
            world
        };
        for (shape, order) in orders(&all) {
            let mut world = wide();
            let mut queue = MutationQueue::new();
            queue.reorder_children(node(PARENT), ids(&order));
            world.tree.child_list_steps = 0;
            world.commit(queue).unwrap();
            assert!(
                world.tree.child_list_steps <= 2 * CHILDREN,
                "{shape}: {} child-list steps reordering {CHILDREN}",
                world.tree.child_list_steps
            );
            assert_eq!(children(&world), ids(&order), "{shape}");
        }
    }

    /// The framework's own reordering of a long list -- a keyed list
    /// reversed, shuffled, or filled in between the rows it has -- reads and
    /// writes the list a constant number of times as well.
    #[test]
    fn the_framework_puts_a_long_list_in_order_in_one_pass() {
        const CHILDREN: u64 = 4_000;
        let all: Vec<u64> = (FIRST..FIRST + CHILDREN).collect();
        let listed = |rows: &[u64]| {
            let mut queue = MutationQueue::new();
            queue.create(node(1), document(), NodeKind::Document);
            for id in std::iter::once(PARENT).chain(rows.iter().copied()) {
                queue.create(
                    node(id),
                    document(),
                    NodeKind::Element { tag: "div".into() },
                );
                queue.insert(node(if id == PARENT { 1 } else { PARENT }), node(id), None);
            }
            let mut cx = AppContext::new();
            cx.commit_mutations(queue).unwrap();
            cx
        };
        let steps = |cx: &mut AppContext, order: &[u64]| {
            cx.world_mut().tree.child_list_steps = 0;
            assert!(cx.reconcile_children(node(PARENT), &ids(order)).unwrap());
            assert_eq!(children(cx.world()), ids(order));
            cx.world().tree.child_list_steps
        };
        for (shape, order) in orders(&all) {
            // Two rows exchanged far apart are two inserts, each a few
            // passes over the list; the rest are one reorder.
            let steps = steps(&mut listed(&all), &order);
            assert!(
                steps <= 6 * CHILDREN,
                "{shape}: {steps} child-list steps for {CHILDREN} children"
            );
        }

        // Every other row is there; the rest are made and put between them.
        let (there, made): (Vec<u64>, Vec<u64>) = all.iter().partition(|row| *row % 2 == 0);
        let mut cx = listed(&there);
        let mut queue = MutationQueue::new();
        for &row in &made {
            queue.create(
                node(row),
                document(),
                NodeKind::Element { tag: "div".into() },
            );
        }
        cx.commit_mutations(queue).unwrap();
        let steps = steps(&mut cx, &all);
        assert!(
            steps <= 3 * CHILDREN,
            "filled in: {steps} child-list steps for {CHILDREN} children"
        );
    }
}
