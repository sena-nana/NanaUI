//! Inserting into a laid-out page walks the result chains of every parent
//! that gains a child up to the nearest fixed boundary. A commit remembers
//! the ancestors its walks took ([`super::ResultChainMemo`]), so a subtree
//! inserted under a deep node pays its depth once, not once per node. Each
//! commit here leaves the result tables exactly as walking every chain in
//! full does, and the frame after it lays the page out the same.
#![cfg(test)]

use super::reflow_oracle::{
    assert_matches_cold, bundled_face_shaper, column, fixed, node, product_frame, styled,
};
use super::{DocumentId, NodeKind, StableNodeId};
use crate::{AppContext, LayoutViewport, MutationQueue};

/// The chain under the page: `node(CHAIN_TOP..=CHAIN_BOTTOM)`, each the
/// only child of the one before.
const CHAIN_TOP: u64 = 10;
const CHAIN_BOTTOM: u64 = 40;
/// The chain node that is a fixed border box on a page with a boundary.
const BOUNDARY: u64 = 25;
/// A second branch beside the chain, for moves.
const BRANCH: u64 = 50;
const BRANCH_LEAF: u64 = 51;
/// Ids from here on are the nodes a commit creates.
const FIRST_NEW: u64 = 1_000;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(800.0, 600.0)
}

fn element(queue: &mut MutationQueue, parent: StableNodeId, id: StableNodeId) {
    queue.create(id, document(), NodeKind::Element { tag: "div".into() });
    queue.set_style(id, styled(column(None)));
    queue.insert(parent, id, None);
}

/// A document holding a 400 px column, the chain under it and a branch
/// beside the chain, laid out once; `boundary` makes [`BOUNDARY`] a fixed
/// border box.
fn page(boundary: bool, memo: bool) -> AppContext {
    let mut cx = AppContext::new();
    cx.compat_world_mut().result_chain_memo_off = !memo;
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(), NodeKind::Document);
    queue.create(node(2), document(), NodeKind::Element { tag: "div".into() });
    queue.set_style(node(2), styled(column(Some(400.0))));
    queue.insert(node(1), node(2), None);
    let mut parent = node(2);
    for id in CHAIN_TOP..=CHAIN_BOTTOM {
        element(&mut queue, parent, node(id));
        parent = node(id);
    }
    if boundary {
        queue.set_style(node(BOUNDARY), styled(fixed(300.0, 2_000.0)));
    }
    element(&mut queue, node(2), node(BRANCH));
    element(&mut queue, node(BRANCH), node(BRANCH_LEAF));
    cx.commit_mutations(queue).unwrap();
    product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
    assert!(
        !cx.world().layout_results.is_empty(),
        "the page is laid out before anything is inserted"
    );
    cx
}

/// The two result tables: what is published, and what of it is hidden.
fn tables(cx: &AppContext) -> (Vec<StableNodeId>, Vec<StableNodeId>) {
    let world = cx.world();
    let mut published: Vec<_> = world.layout_results.keys().collect();
    let mut hidden: Vec<_> = world
        .layout_results
        .iter()
        .filter_map(|(id, meta)| meta.suppressed.then_some(id))
        .collect();
    published.sort_unstable();
    hidden.sort_unstable();
    (published, hidden)
}

/// What one commit queues, handed the next free id.
type Batch = fn(&mut MutationQueue, &mut u64);

/// `count` new leaves under `parent`.
fn leaves(queue: &mut MutationQueue, next: &mut u64, parent: StableNodeId, count: u64) {
    for _ in 0..count {
        element(queue, parent, node(*next));
        *next += 1;
    }
}

/// Runs `batches` on a fresh page, one commit and one frame each; returns
/// the page, the tables after each commit and the chain steps all of them
/// walked.
fn run(
    boundary: bool,
    memo: bool,
    batches: &[Batch],
) -> (AppContext, Vec<impl PartialEq + std::fmt::Debug>, u64) {
    let mut cx = page(boundary, memo);
    let mut next = FIRST_NEW;
    let mut after = Vec::new();
    let mut steps = 0;
    for batch in batches {
        let mut queue = MutationQueue::new();
        batch(&mut queue, &mut next);
        let before = cx.world().result_chain_steps;
        cx.commit_mutations(queue).unwrap();
        steps += cx.world().result_chain_steps - before;
        after.push(tables(&cx));
        product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
    }
    (cx, after, steps)
}

/// Every commit leaves the tables as the full walks do, and the frames lay
/// the page out the same. Returns the steps with the memo and without.
fn assert_same_as_full_walks(boundary: bool, batches: &[Batch]) -> (u64, u64) {
    let (mut ours, ours_tables, ours_steps) = run(boundary, true, batches);
    let (mut full, full_tables, full_steps) = run(boundary, false, batches);
    for (index, (ours, full)) in ours_tables.iter().zip(&full_tables).enumerate() {
        assert_eq!(ours, full, "result tables after commit {index}");
    }
    assert_matches_cold(&mut ours, &mut full, document());
    (ours_steps, full_steps)
}

fn bottom() -> StableNodeId {
    node(CHAIN_BOTTOM)
}

/// Leaves inserted under the bottom of a chain with no boundary walk the
/// chain once between them: steps grow with the leaves plus the depth, not
/// their product.
#[test]
fn leaves_under_a_deep_node_walk_its_ancestors_once() {
    const LEAVES: u64 = 64;
    let batch: Batch = |queue, next| leaves(queue, next, bottom(), LEAVES);
    let (ours, full) = assert_same_as_full_walks(false, &[batch]);
    let depth = CHAIN_BOTTOM - CHAIN_TOP + 3;
    assert!(
        ours <= 8 * LEAVES + 4 * depth,
        "{ours} steps for {LEAVES} leaves at depth {depth}"
    );
    assert!(
        full >= 2 * LEAVES * depth,
        "{full} steps without the memo: the case is the one it is for"
    );
}

/// A chain built under the laid-out page top down, one level at a time in
/// one commit: each new level walks to the one above it and stops.
#[test]
fn a_chain_built_top_down_under_a_deep_node_is_linear() {
    const LEVELS: u64 = 64;
    let batch: Batch = |queue, next| {
        let mut parent = bottom();
        for _ in 0..LEVELS {
            element(queue, parent, node(*next));
            parent = node(*next);
            *next += 1;
        }
    };
    let (ours, full) = assert_same_as_full_walks(false, &[batch]);
    assert!(
        ours <= 10 * LEVELS + 4 * (CHAIN_BOTTOM - CHAIN_TOP + 3),
        "{ours}"
    );
    assert!(full >= LEVELS * LEVELS, "{full}");
}

/// The walks stop at the fixed box; the memo remembers where.
#[test]
fn leaves_under_a_fixed_box_stop_at_it() {
    let batch: Batch = |queue, next| leaves(queue, next, bottom(), 16);
    let (ours, full) = assert_same_as_full_walks(true, &[batch]);
    assert!(ours < full, "{ours} / {full}");
}

/// The fixed box an earlier walk stopped at is no longer one: the walks
/// after it must go on past it, to the top.
#[test]
fn a_boundary_that_loosens_mid_commit_lets_later_walks_through() {
    let batch: Batch = |queue, next| {
        leaves(queue, next, bottom(), 8);
        queue.set_style(node(BOUNDARY), styled(column(None)));
        leaves(queue, next, bottom(), 8);
    };
    let (ours, full) = assert_same_as_full_walks(true, &[batch]);
    assert!(ours < full, "{ours} / {full}");
}

/// A chain node becomes a fixed box mid-commit: later walks stop sooner.
#[test]
fn a_boundary_that_appears_mid_commit_stops_later_walks() {
    let batch: Batch = |queue, next| {
        leaves(queue, next, bottom(), 8);
        queue.set_style(node(BOUNDARY), styled(fixed(300.0, 2_000.0)));
        leaves(queue, next, bottom(), 8);
    };
    assert_same_as_full_walks(false, &[batch]);
}

/// The second commit's walks start over: what the first one's walks did
/// was published again by the frame between them.
#[test]
fn each_commit_walks_again() {
    let batch: Batch = |queue, next| leaves(queue, next, bottom(), 4);
    assert_same_as_full_walks(false, &[batch, batch, batch]);
    assert_same_as_full_walks(true, &[batch, batch]);
}

/// Moving part of the chain in the commit: the walks after it follow the
/// new parents.
#[test]
fn moves_and_removals_mid_commit_match_full_walks() {
    let moved: Batch = |queue, next| {
        leaves(queue, next, bottom(), 4);
        queue.insert(node(BRANCH_LEAF), node(30), None);
        leaves(queue, next, bottom(), 4);
        leaves(queue, next, node(29), 4);
    };
    let detached: Batch = |queue, next| {
        leaves(queue, next, bottom(), 4);
        queue.detach(node(35));
        leaves(queue, next, node(34), 4);
        queue.insert(node(BRANCH_LEAF), node(35), None);
        leaves(queue, next, bottom(), 4);
    };
    let parked: Batch = |queue, next| {
        leaves(queue, next, bottom(), 4);
        queue.park_subtree(node(35));
        leaves(queue, next, node(34), 4);
        queue.insert(node(34), node(35), None);
        leaves(queue, next, bottom(), 4);
    };
    let despawned: Batch = |queue, next| {
        leaves(queue, next, bottom(), 4);
        queue.despawn_subtree(node(38));
        leaves(queue, next, node(37), 4);
    };
    for boundary in [false, true] {
        for batch in [moved, detached, parked, despawned] {
            assert_same_as_full_walks(boundary, &[batch]);
        }
    }
}

/// A subtree built detached in one commit and inserted in the next, as
/// `mount_view` does, and one built and inserted in the same commit.
#[test]
fn subtrees_built_before_their_insert_match_full_walks() {
    let parked_then_inserted: [Batch; 2] = [
        |queue, next| {
            let root = node(*next);
            queue.create(root, document(), NodeKind::Element { tag: "div".into() });
            *next += 1;
            let mut parent = root;
            for _ in 0..8 {
                element(queue, parent, node(*next));
                parent = node(*next);
                *next += 1;
            }
            queue.park_subtree(root);
        },
        |queue, _| queue.insert(bottom(), node(FIRST_NEW), None),
    ];
    let same_commit: Batch = |queue, next| {
        let root = node(*next);
        queue.create(root, document(), NodeKind::Element { tag: "div".into() });
        *next += 1;
        leaves(queue, next, root, 4);
        queue.insert(bottom(), root, None);
        leaves(queue, next, root, 4);
        leaves(queue, next, bottom(), 4);
    };
    for boundary in [false, true] {
        assert_same_as_full_walks(boundary, &parked_then_inserted);
        assert_same_as_full_walks(boundary, &[same_commit]);
    }
}

/// An inherited text change hides the results of a whole subtree and its
/// chain; leaves inserted around it walk through what it hid.
#[test]
fn an_inherited_style_change_mid_commit_matches_full_walks() {
    let batch: Batch = |queue, next| {
        leaves(queue, next, bottom(), 4);
        let mut layout = column(None);
        layout.font_size = Some(20.0);
        queue.set_style(node(20), styled(layout));
        leaves(queue, next, bottom(), 4);
        leaves(queue, next, node(18), 4);
    };
    for boundary in [false, true] {
        assert_same_as_full_walks(boundary, &[batch]);
    }
}
