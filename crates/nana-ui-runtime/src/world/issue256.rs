//! Issue #256: the dirty frontier.
//!
//! One fixture throughout: a page of groups, each group three wrappers deep,
//! each wrapper holding ten fixed 200x40 cards with one label. A label is
//! eight levels below the document. Frames run the order the product flush
//! runs: shape scheduled text, merge what shaping queued, lay out, re-shape
//! the laid-out scope.
//!
//! - Gate A/D: one label edit inside a fixed card costs the same at 1k, 10k
//!   and 100k nodes, walks at most 32 dependency edges, and admits nothing
//!   outside the card.
//! - Gate B: text whose width, height and baseline held stops at the text:
//!   no parent work, and the stop is counted.
//! - Gate C: a hundred label edits in ten groups build one union closure and
//!   walk each of its edges at most twice; the scratch they build is the
//!   same at 1k and 10k nodes.
//! - Correctness: after a run of edits, every box, published result, hit
//!   entry and accessibility bound equals a cold layout of the same tree.

#![cfg(test)]

use std::collections::HashSet;

use nana_ui_core::WorkCounters;

use super::reflow_oracle::{
    admitted_nodes, assert_matches_cold, bundled_face_shaper, column, fixed, node, product_frame,
    styled,
};
use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::{
    AppContext, LayoutFrontierStats, LayoutViewport, MutationQueue, NanaTextEngineShaper,
    TextContent,
};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(320.0, 480.0)
}

/// One product frame, in the order `RuntimeDocument::flush` runs it.
fn frame(
    context: &mut AppContext,
    document: DocumentId,
    shaper: &mut NanaTextEngineShaper,
) -> WorkCounters {
    product_frame(context, document, viewport(), shaper)
}

const WRAPPERS: u64 = 3;
const CARDS: u64 = 10;
const GROUP_NODES: u64 = 1 + WRAPPERS + 2 * CARDS;

struct Cards {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    groups: u64,
}

impl Cards {
    fn group(group: u64) -> StableNodeId {
        node(3 + group * GROUP_NODES)
    }

    fn wrapper(group: u64, depth: u64) -> StableNodeId {
        node(Self::group(group).get() + 1 + depth)
    }

    fn card(group: u64, card: u64) -> StableNodeId {
        node(Self::group(group).get() + 1 + WRAPPERS + 2 * card)
    }

    fn label(group: u64, card: u64) -> StableNodeId {
        node(Self::card(group, card).get() + 1)
    }

    /// About `nodes` nodes, every label reading `text(label)`.
    fn with_text(nodes: u64, text: impl Fn(StableNodeId) -> String) -> Self {
        let groups = (nodes.saturating_sub(2) / GROUP_NODES).max(1);
        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document, NodeKind::Document);
        queue.create(node(2), document, NodeKind::Element { tag: "page".into() });
        queue.insert(node(1), node(2), None);
        queue.set_style(node(2), styled(column(Some(320.0))));
        for group in 0..groups {
            let mut parent = node(2);
            let group_id = Self::group(group);
            for (index, id) in std::iter::once(group_id)
                .chain((0..WRAPPERS).map(|depth| Self::wrapper(group, depth)))
                .enumerate()
            {
                let tag = if index == 0 { "group" } else { "wrapper" };
                queue.create(id, document, NodeKind::Element { tag: tag.into() });
                queue.insert(parent, id, None);
                queue.set_style(id, styled(column(None)));
                parent = id;
            }
            for card in 0..CARDS {
                let card_id = Self::card(group, card);
                let label_id = Self::label(group, card);
                queue.create(card_id, document, NodeKind::Element { tag: "card".into() });
                queue.insert(parent, card_id, None);
                queue.set_style(card_id, styled(fixed(200.0, 40.0)));
                queue.create(label_id, document, NodeKind::Text);
                queue.insert(card_id, label_id, None);
                queue.set_text(
                    label_id,
                    TextContent {
                        value: text(label_id).into(),
                    },
                );
            }
        }
        context.compat_world_mut().commit(queue).unwrap();
        let mut shaper = bundled_face_shaper();
        frame(&mut context, document, &mut shaper);
        Self {
            context,
            document,
            shaper,
            groups,
        }
    }

    fn new(nodes: u64) -> Self {
        Self::with_text(nodes, |_| "one".into())
    }

    fn set_text(&mut self, id: StableNodeId, value: &str) {
        let mut queue = MutationQueue::new();
        queue.set_text(
            id,
            TextContent {
                value: value.into(),
            },
        );
        self.context.compat_world_mut().commit(queue).unwrap();
    }

    fn frame(&mut self) -> WorkCounters {
        frame(&mut self.context, self.document, &mut self.shaper)
    }

    fn frontier_stats(&self) -> LayoutFrontierStats {
        self.context.layout_frontier_stats(self.document)
    }

    fn world(&self) -> &UiWorld {
        self.context.world()
    }
}

/// What one frame cost the layout, in the units the gates name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Work {
    seeds: usize,
    measure: usize,
    placement: usize,
    contexts: usize,
    edges: usize,
    local_fallbacks: usize,
    full_fallbacks: usize,
    measured: usize,
    placed: usize,
}

impl From<WorkCounters> for Work {
    fn from(counters: WorkCounters) -> Self {
        Self {
            seeds: counters.layout_frontier_seeds,
            measure: counters.layout_frontier_nodes_measure,
            placement: counters.layout_frontier_nodes_placement,
            contexts: counters.layout_frontier_contexts,
            edges: counters.layout_dependency_edges_visited,
            local_fallbacks: counters.layout_local_subtree_fallbacks,
            full_fallbacks: counters.layout_full_document_fallbacks,
            measured: counters.layout_measure_nodes,
            placed: counters.layout_placement_nodes,
        }
    }
}

/// One label in the middle group grows; its fixed card absorbs it.
fn contained_label_edit(nodes: u64) -> (Work, HashSet<StableNodeId>, Cards) {
    let mut cards = Cards::new(nodes);
    let group = cards.groups / 2;
    let label = Cards::label(group, 4);
    cards.set_text(label, "seventeen");
    let work = Work::from(cards.frame());
    let admitted = admitted_nodes(&cards.context, cards.document);
    (work, admitted, cards)
}

fn assert_contained(work: Work, admitted: &HashSet<StableNodeId>, cards: &Cards) {
    let group = cards.groups / 2;
    let label = Cards::label(group, 4);
    let card = Cards::card(group, 4);
    assert_eq!(cards.world().layout_box(card).unwrap().width, 200.0);
    assert!(work.edges <= 32, "{work:?}");
    assert_eq!((work.local_fallbacks, work.full_fallbacks), (0, 0));
    assert!(work.measure >= 1, "the label itself is measured: {work:?}");
    let path: HashSet<StableNodeId> = [label, card].into();
    assert!(
        admitted.is_subset(&path),
        "only the label and its fixed card enter the frontier: {admitted:?}"
    );
}

/// Gates A and D. The same contained edit at 1k and 10k nodes, depth eight.
#[test]
fn issue256_a_contained_label_edit_costs_the_same_at_1k_and_10k() {
    let (small, small_admitted, small_cards) = contained_label_edit(1_000);
    let (large, large_admitted, large_cards) = contained_label_edit(10_000);
    assert!(large_cards.world().len() >= 10 * small_cards.world().len() - 100);
    assert_contained(small, &small_admitted, &small_cards);
    assert_contained(large, &large_admitted, &large_cards);
    assert_eq!(small, large);
    assert_eq!(
        small_cards.frontier_stats().scratch_entries,
        large_cards.frontier_stats().scratch_entries,
        "the pass builds scratch for the closure, not the document"
    );
}

/// Gate D at 100k nodes.
#[test]
fn issue256_a_contained_label_edit_costs_the_same_at_100k() {
    let (small, ..) = contained_label_edit(1_000);
    let (large, large_admitted, large_cards) = contained_label_edit(100_000);
    assert!(large_cards.world().len() >= 99_000);
    assert_contained(large, &large_admitted, &large_cards);
    assert_eq!(small, large);
}

/// Gate B. "99" and "98" have the same width, height and baseline: the text
/// is re-shaped, its parent is not measured, no dependency edge is walked,
/// and the stop is counted.
#[test]
fn issue256_text_whose_exported_metrics_held_stops_at_the_text() {
    let mut cards = Cards::new(1_000);
    let group = cards.groups / 2;
    let label = Cards::label(group, 5);
    let card = Cards::card(group, 5);
    cards.set_text(label, "99");
    cards.frame();
    let label_box = cards.world().layout_box(label).unwrap();
    let card_box = cards.world().layout_box(card).unwrap();
    let baseline = cards
        .world()
        .layout_result(label)
        .and_then(|result| result.first_baseline);

    cards.set_text(label, "98");
    let counters = cards.frame();
    assert!(
        cards.world().last_text_work_counters().layouts_created > 0,
        "the text itself is laid out again"
    );
    assert_eq!(counters.layout_frontier_nodes_measure, 0);
    assert_eq!(counters.layout_dependency_edges_visited, 0);
    assert!(counters.layout_propagations_stopped >= 1);
    assert_eq!(cards.world().layout_box(label), Some(label_box));
    assert_eq!(cards.world().layout_box(card), Some(card_box));
    assert_eq!(
        cards
            .world()
            .layout_result(label)
            .and_then(|result| result.first_baseline),
        baseline
    );
}

/// The hundred labels of ten groups spread across the page.
fn hundred_labels(groups: u64) -> Vec<StableNodeId> {
    (0..10)
        .map(|step| step * groups / 10)
        .flat_map(|group| (0..CARDS).map(move |card| Cards::label(group, card)))
        .collect()
}

fn hundred_seed_batch(nodes: u64) -> (WorkCounters, LayoutFrontierStats) {
    let mut cards = Cards::new(nodes);
    for label in hundred_labels(cards.groups) {
        cards.set_text(label, "seventeen");
    }
    let counters = cards.frame();
    (counters, cards.frontier_stats())
}

/// Gate C. A hundred edits in ten groups are one union closure: each edge of
/// it is walked at most twice, not once per seed's walk to the root, and the
/// scratch the pass builds does not grow with the document.
#[test]
fn issue256_a_hundred_seeds_walk_their_union_closure_once() {
    let (small, small_stats) = hundred_seed_batch(1_000);
    let (large, large_stats) = hundred_seed_batch(10_000);
    for (counters, stats) in [(small, small_stats), (large, large_stats)] {
        assert_eq!(counters.layout_frontier_seeds, stats.seeds);
        assert!(stats.seeds >= 100, "{stats:?}");
        assert!(
            counters.layout_dependency_edges_visited <= 2 * stats.graph_edges,
            "{} edge walks over a {}-edge closure",
            counters.layout_dependency_edges_visited,
            stats.graph_edges
        );
        assert_eq!(counters.layout_full_document_fallbacks, 0);
    }
    assert_eq!(Work::from(small), Work::from(large));
    assert_eq!(small_stats, large_stats);
}

/// Correctness. A grown label, an unchanged-metric edit, a hundred-seed
/// batch and a batch whose text wraps, each in its own frame, leave the same
/// geometry a cold layout of the final tree has.
#[test]
fn issue256_incremental_frames_match_a_cold_layout() {
    let mut cards = Cards::new(1_000);
    let groups = cards.groups;
    let mut final_text = std::collections::HashMap::new();
    let mut edit = |cards: &mut Cards, label: StableNodeId, text: &str| {
        cards.set_text(label, text);
        final_text.insert(label, text.to_owned());
    };

    edit(&mut cards, Cards::label(groups / 2, 4), "seventeen");
    cards.frame();
    edit(&mut cards, Cards::label(groups / 2, 5), "99");
    cards.frame();
    edit(&mut cards, Cards::label(groups / 2, 5), "98");
    cards.frame();
    for label in hundred_labels(groups) {
        edit(&mut cards, label, "seventeen");
    }
    cards.frame();
    for label in hundred_labels(groups).into_iter().step_by(3) {
        edit(
            &mut cards,
            label,
            "a label long enough to wrap inside its card",
        );
    }
    cards.frame();

    let mut cold = Cards::with_text(1_000, |label| {
        final_text
            .get(&label)
            .cloned()
            .unwrap_or_else(|| "one".into())
    });
    assert_matches_cold(&mut cards.context, &mut cold.context, cold.document);
}

/// Every edit of a child list keeps each child's recorded index in it, so
/// the dependency graph finds a node's next sibling without scanning.
#[test]
fn issue256_child_indices_follow_inserts_moves_and_removals() {
    let document = DocumentId::new(1).unwrap();
    let mut context = AppContext::new();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document, NodeKind::Document);
    for list in [2, 3] {
        queue.create(
            node(list),
            document,
            NodeKind::Element { tag: "list".into() },
        );
        queue.insert(node(1), node(list), None);
    }
    for row in 10..16 {
        queue.create(node(row), document, NodeKind::Element { tag: "row".into() });
        queue.insert(node(2), node(row), None);
    }
    context.compat_world_mut().commit(queue).unwrap();
    let indices_hold = |world: &UiWorld| {
        for list in [node(2), node(3)] {
            let children = world.nodes.get(list).unwrap().hierarchy.children.clone();
            for (index, child) in children.iter().enumerate() {
                assert_eq!(
                    world.nodes.get(*child).unwrap().hierarchy.index_in_parent as usize,
                    index,
                    "{child:?} in {children:?}"
                );
            }
        }
    };
    indices_hold(context.world());
    let edits: [&dyn Fn(&mut MutationQueue); 6] = [
        // A new row in the middle.
        &|queue| {
            queue.create(node(20), document, NodeKind::Element { tag: "row".into() });
            queue.insert(node(2), node(20), Some(node(12)));
        },
        // The last row to the front, then the first to the end.
        &|queue| queue.insert(node(2), node(15), Some(node(10))),
        &|queue| queue.insert(node(2), node(15), None),
        // A row into the other list.
        &|queue| queue.insert(node(3), node(11), None),
        &|queue| queue.detach(node(13)),
        &|queue| queue.despawn_subtree(node(20)),
    ];
    for edit in edits {
        let mut queue = MutationQueue::new();
        edit(&mut queue);
        context.compat_world_mut().commit(queue).unwrap();
        indices_hold(context.world());
    }
}
