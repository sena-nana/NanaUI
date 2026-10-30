//! Issue #389: removing many children of one parent in one commit filters
//! its child list once in validation and once in apply, and renumbers the
//! children that stay once, from the first one removed: k of n children cost
//! O(n + k), not O(n·k). One removal still pays only for the children after
//! it.
#![cfg(test)]

use std::collections::HashSet;

use super::reflow_oracle::{
    assert_matches_cold, bundled_face_shaper, column, fixed, node, product_frame, styled,
};
use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::{AppContext, LayoutViewport, MutationQueue, TextContent};

const CHILDREN: u64 = 4_000;
const PARENT: u64 = 2;
const FIRST_CHILD: u64 = 10;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn element(queue: &mut MutationQueue, parent: StableNodeId, id: StableNodeId) {
    queue.create(id, document(), NodeKind::Element { tag: "div".into() });
    queue.insert(parent, id, None);
}

fn child(offset: u64) -> StableNodeId {
    node(FIRST_CHILD + offset)
}

/// A document, a parent under it and [`CHILDREN`] children of that parent.
fn wide_parent() -> UiWorld {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(), NodeKind::Document);
    element(&mut queue, node(1), node(PARENT));
    for offset in 0..CHILDREN {
        element(&mut queue, node(PARENT), child(offset));
    }
    world.commit(queue).unwrap();
    world
}

/// The parent's children, each checked to sit at the index it records.
fn children(world: &UiWorld) -> Vec<StableNodeId> {
    let children = world.node(node(PARENT)).unwrap().children.to_vec();
    for (index, id) in children.iter().enumerate() {
        assert_eq!(
            world.nodes.get(*id).unwrap().hierarchy.index_in_parent as usize,
            index,
            "{id:?}'s recorded index"
        );
    }
    children
}

#[derive(Debug, Clone, Copy)]
enum Removal {
    Despawn,
    Detach,
    Park,
}

fn remove(queue: &mut MutationQueue, removal: Removal, id: StableNodeId) {
    match removal {
        Removal::Despawn => queue.despawn_subtree(id),
        Removal::Detach => queue.detach(id),
        Removal::Park => queue.park_subtree(id),
    }
}

/// Each removal shifted, filtered and renumbered the children after it,
/// in validation and again in apply: clearing a keyed list from the front
/// paid the whole tail per row. One commit now pays its parent's list a
/// constant number of times however many of its children leave.
#[test]
fn issue389_removing_k_of_n_children_visits_n_plus_k() {
    let n = CHILDREN;
    let shapes: [(&str, Vec<u64>); 4] = [
        ("all, front first", (0..n).collect()),
        ("front half", (0..n / 2).collect()),
        ("every other", (0..n).step_by(2).collect()),
        ("back half, last first", (n / 2..n).rev().collect()),
    ];
    for removal in [Removal::Despawn, Removal::Detach, Removal::Park] {
        for (shape, offsets) in &shapes {
            let mut world = wide_parent();
            let removed = offsets.iter().map(|&o| child(o)).collect::<HashSet<_>>();
            let mut expected = children(&world);
            expected.retain(|id| !removed.contains(id));
            let mut queue = MutationQueue::new();
            for &offset in offsets {
                remove(&mut queue, removal, child(offset));
            }
            world.child_list_steps = 0;
            world.commit(queue).unwrap();
            let k = offsets.len() as u64;
            assert!(
                world.child_list_steps <= 2 * (n + k),
                "{removal:?}, {shape}: {} child-list steps removing {k} of {n}",
                world.child_list_steps
            );
            assert_eq!(children(&world), expected, "{removal:?}, {shape}");
            for id in &removed {
                assert_eq!(world.parent_id(*id), None, "{removal:?}, {shape}");
            }
        }
    }
}

/// A lone removal is not a run: it shifts and renumbers only the children
/// after it, and nothing filters the whole list.
#[test]
fn issue389_one_removal_pays_only_the_children_after_it() {
    for removal in [Removal::Despawn, Removal::Detach, Removal::Park] {
        let mut world = wide_parent();
        let mut queue = MutationQueue::new();
        remove(&mut queue, removal, child(CHILDREN - 1));
        world.child_list_steps = 0;
        world.commit(queue).unwrap();
        assert_eq!(world.child_list_steps, 0, "{removal:?}, the last child");

        // Ten children after it, each shifted once and renumbered once.
        let mut queue = MutationQueue::new();
        remove(&mut queue, removal, child(CHILDREN - 12));
        world.child_list_steps = 0;
        world.commit(queue).unwrap();
        assert!(
            world.child_list_steps <= 20,
            "{removal:?}: {} steps",
            world.child_list_steps
        );
        assert_eq!(children(&world).len(), CHILDREN as usize - 2);
    }
}

/// What one batch does to the parent's list, one edit at a time.
enum Edit {
    Remove(Removal, u64),
    /// Insert the child at `.0` (a new one past [`CHILDREN`]) before the
    /// child at `.1`, or last.
    Insert(u64, Option<u64>),
}

/// Runs of removals broken up by inserts, a reinsertion of a child the
/// batch detached, a move, a repeated detach and a lone removal leave the
/// list, and every recorded index, where one edit at a time would.
#[test]
fn issue389_removal_runs_between_inserts_keep_order_and_indices() {
    use Edit::{Insert, Remove};
    use Removal::{Despawn, Detach, Park};
    let fresh = CHILDREN;
    let edits = [
        Remove(Despawn, 0),
        Remove(Despawn, 1),
        Remove(Despawn, 2),
        Insert(fresh, Some(5)),
        Remove(Detach, 5),
        Remove(Detach, 6),
        Insert(5, Some(10)),
        Remove(Detach, 10),
        Remove(Despawn, 11),
        Remove(Park, 12),
        Insert(6, None),
        Insert(20, Some(15)),
        Remove(Detach, 40),
        Remove(Detach, 40),
        Remove(Despawn, 41),
        Insert(fresh + 1, Some(3)),
        Remove(Despawn, 3),
        Remove(Despawn, fresh),
        Remove(Park, 50),
    ];
    let mut world = wide_parent();
    let mut expected = children(&world);
    let mut queue = MutationQueue::new();
    for edit in &edits {
        match *edit {
            Remove(removal, offset) => {
                remove(&mut queue, removal, child(offset));
                expected.retain(|id| *id != child(offset));
            }
            Insert(offset, before) => {
                if offset >= fresh {
                    queue.create(
                        child(offset),
                        document(),
                        NodeKind::Element { tag: "div".into() },
                    );
                }
                queue.insert(node(PARENT), child(offset), before.map(child));
                expected.retain(|id| *id != child(offset));
                let index = before
                    .and_then(|before| expected.iter().position(|id| *id == child(before)))
                    .unwrap_or(expected.len());
                expected.insert(index, child(offset));
            }
        }
    }
    world.commit(queue).unwrap();
    assert_eq!(children(&world), expected);
}

const ROWS: u64 = 200;
const LABELS: u64 = 100_000;

fn viewport() -> LayoutViewport {
    LayoutViewport::new(700.0, 600.0)
}

fn add_row(queue: &mut MutationQueue, row: u64, before: Option<u64>) {
    queue.create(
        node(row),
        document(),
        NodeKind::Element { tag: "div".into() },
    );
    queue.insert(node(PARENT), node(row), before.map(node));
    queue.set_style(node(row), styled(fixed(200.0, 20.0)));
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

/// A 400px column of labelled rows, laid out once.
fn rows(rows: &[u64]) -> AppContext {
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(), NodeKind::Document);
    element(&mut queue, node(1), node(PARENT));
    queue.set_style(node(PARENT), styled(column(Some(400.0))));
    for &row in rows {
        add_row(&mut queue, row, None);
    }
    let mut cx = AppContext::new();
    cx.commit_mutations(queue).unwrap();
    product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
    cx
}

/// A bulk removal from the front, a scattered one and a run after an
/// insert in the middle, in one commit: the next frame lays out as the
/// rows that stay laid out cold.
#[test]
fn issue389_bulk_removal_lays_out_as_cold() {
    let all = (FIRST_CHILD..FIRST_CHILD + ROWS).collect::<Vec<_>>();
    let mut cx = rows(&all);
    let mut queue = MutationQueue::new();
    let mut gone = HashSet::new();
    gone.extend(FIRST_CHILD..FIRST_CHILD + 50);
    gone.extend((100..150).step_by(3));
    for &row in &all {
        if gone.contains(&row) {
            queue.despawn_subtree(node(row));
        }
    }
    let inserted = FIRST_CHILD + ROWS;
    add_row(&mut queue, inserted, Some(160));
    for row in 170..180 {
        gone.insert(row);
        queue.despawn_subtree(node(row));
    }
    cx.commit_mutations(queue).unwrap();
    product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());

    let mut stay = all
        .iter()
        .copied()
        .filter(|row| !gone.contains(row))
        .collect::<Vec<_>>();
    let at = stay.iter().position(|row| *row == 160).unwrap();
    stay.insert(at, inserted);
    let mut cold = rows(&stay);
    assert_matches_cold(&mut cx, &mut cold, document());
}

/// Validation reads the list a batch has left so far: a child the batch
/// already detached is no anchor for an insert, as before runs shared a
/// filter.
#[test]
fn issue389_a_detached_child_is_no_insert_anchor_later_in_the_batch() {
    let mut world = wide_parent();
    let before = children(&world);
    let fresh = child(CHILDREN);
    let mut queue = MutationQueue::new();
    queue.detach(child(7));
    queue.detach(child(8));
    queue.create(fresh, document(), NodeKind::Element { tag: "div".into() });
    queue.insert(node(PARENT), fresh, Some(child(8)));
    assert_eq!(
        world.commit(queue),
        Err(crate::UiWorldError::InvalidBefore {
            parent: node(PARENT),
            before: child(8),
        })
    );
    assert_eq!(children(&world), before);
}
