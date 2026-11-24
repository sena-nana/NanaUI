//! A child records its place among its siblings as a number counted from one
//! its parent keeps, so an edit of a child list writes numbers only on the
//! shorter side of it: putting a child first among n writes one number, not
//! n. Finding a sibling by that number must stay right after any edit.
#![cfg(test)]

use super::reflow_oracle::node;
use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::MutationQueue;

const LISTS: [u64; 2] = [2, 3];
const FIRST_CHILD: u64 = 10;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn element(queue: &mut MutationQueue, id: StableNodeId) {
    queue.create(id, document(), NodeKind::Element { tag: "div".into() });
}

/// A document with [`LISTS`] under it, the first holding `children`.
fn lists(children: u64) -> UiWorld {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(), NodeKind::Document);
    for list in LISTS {
        element(&mut queue, node(list));
        queue.insert(node(1), node(list), None);
    }
    for child in FIRST_CHILD..FIRST_CHILD + children {
        element(&mut queue, node(child));
        queue.insert(node(LISTS[0]), node(child), None);
    }
    world.commit(queue).unwrap();
    world
}

/// `list`'s children, each found by the number it records and not by a scan.
fn numbered(world: &UiWorld, list: u64) -> Vec<StableNodeId> {
    let children = world.node(node(list)).unwrap().children.to_vec();
    for (index, child) in children.iter().enumerate() {
        assert_eq!(
            world.recorded_child_index(node(list), *child),
            Some(index),
            "{child:?} in list {list}"
        );
    }
    children
}

/// The numbers one commit of `edit` writes.
fn written(world: &mut UiWorld, edit: impl FnOnce(&mut MutationQueue)) -> u64 {
    let mut queue = MutationQueue::new();
    edit(&mut queue);
    world.tree.children_numbered = 0;
    world.commit(queue).unwrap();
    for list in LISTS {
        numbered(world, list);
    }
    world.tree.children_numbered
}

#[test]
fn an_edit_numbers_the_shorter_side_of_the_list() {
    const CHILDREN: u64 = 4_000;
    let list = node(LISTS[0]);
    let mut world = lists(CHILDREN);
    let at = |world: &UiWorld, index: usize| world.node(list).unwrap().children[index];
    let mut next = FIRST_CHILD + CHILDREN;
    let mut put = |world: &mut UiWorld, index: Option<usize>| {
        next += 1;
        let (child, before) = (node(next), index.map(|index| at(world, index)));
        written(world, |queue| {
            element(queue, child);
            queue.insert(list, child, before);
        })
    };

    assert_eq!(put(&mut world, Some(0)), 1, "a child put first");
    assert_eq!(put(&mut world, None), 1, "a child put last");
    let quarter = CHILDREN as usize / 4;
    assert_eq!(put(&mut world, Some(quarter)), quarter as u64 + 1);
    let len = world.node(list).unwrap().children.len();
    assert_eq!(put(&mut world, Some(len - quarter)), quarter as u64 + 1);

    let first = at(&world, 0);
    assert_eq!(written(&mut world, |queue| queue.despawn_subtree(first)), 0);
    let len = world.node(list).unwrap().children.len();
    let last = at(&world, len - 1);
    assert_eq!(written(&mut world, |queue| queue.despawn_subtree(last)), 0);
    let near_start = at(&world, quarter);
    assert_eq!(
        written(&mut world, |queue| queue.despawn_subtree(near_start)),
        quarter as u64
    );

    // A run taken off the start in one commit, and one child put where
    // another was: the list is as long as it was, so only they are written.
    let run: Vec<StableNodeId> = (0..50).map(|index| at(&world, index)).collect();
    assert_eq!(
        written(&mut world, |queue| {
            for child in run {
                queue.despawn_subtree(child);
            }
        }),
        0
    );
    let middle = world.node(list).unwrap().children.len() / 2;
    let (old, after) = (at(&world, middle), at(&world, middle + 1));
    next += 1;
    assert_eq!(
        written(&mut world, |queue| {
            queue.despawn_subtree(old);
            element(queue, node(next));
            queue.insert(list, node(next), Some(after));
        }),
        1
    );

    // Two children exchanged across ten: the ones between are where they were.
    let mut order = world.node(list).unwrap().children.to_vec();
    order.swap(middle, middle + 10);
    assert_eq!(
        written(&mut world, |queue| queue.reorder_children(list, order)),
        11
    );
}

/// Edits drawn from `seed`, a commit of a few at a time, checked against
/// plain lists.
fn random_edits(seed: u64, commits: usize) {
    let mut state = seed;
    let mut draw = |below: usize| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) as usize) % below.max(1)
    };
    let mut world = lists(24);
    let mut model: [Vec<StableNodeId>; 2] = [numbered(&world, LISTS[0]), Vec::new()];
    let mut next = 1_000;
    for commit in 0..commits {
        let mut queue = MutationQueue::new();
        for _ in 0..1 + draw(4) {
            let list = draw(2);
            let len = model[list].len();
            match draw(6) {
                // A new child, anywhere.
                0 | 1 => {
                    next += 1;
                    let index = draw(len + 1);
                    element(&mut queue, node(next));
                    queue.insert(
                        node(LISTS[list]),
                        node(next),
                        model[list].get(index).copied(),
                    );
                    model[list].insert(index, node(next));
                }
                // A child moved, within its list or to the other one.
                2 if len > 0 => {
                    let child = model[list].remove(draw(len));
                    let to = draw(2);
                    let index = draw(model[to].len() + 1);
                    queue.insert(node(LISTS[to]), child, model[to].get(index).copied());
                    model[to].insert(index, child);
                }
                // A run of neighbours removed.
                3 if len > 0 => {
                    let start = draw(len);
                    let end = (start + 1 + draw(5)).min(len);
                    for child in model[list].drain(start..end) {
                        queue.despawn_subtree(child);
                    }
                }
                // Children here and there removed.
                4 if len > 0 => {
                    for _ in 0..1 + draw(3) {
                        if !model[list].is_empty() {
                            let index = draw(model[list].len());
                            queue.despawn_subtree(model[list].remove(index));
                        }
                    }
                }
                // A stretch turned round.
                5 if len > 1 => {
                    let start = draw(len - 1);
                    let end = (start + 2 + draw(8)).min(len);
                    model[list][start..end].reverse();
                    queue.reorder_children(node(LISTS[list]), model[list].clone());
                }
                _ => {}
            }
        }
        world.commit(queue).unwrap();
        for (list, expected) in LISTS.into_iter().zip(&model) {
            assert_eq!(
                &numbered(&world, list),
                expected,
                "seed {seed}, commit {commit}"
            );
        }
    }
}

#[test]
fn every_child_is_found_by_its_number_after_any_edit() {
    for seed in 1..=40 {
        random_edits(seed, 400);
    }
}
