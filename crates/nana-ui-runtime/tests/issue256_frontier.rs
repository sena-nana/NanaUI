//! Structural tests for the dependency-aware dirty frontier.

use std::collections::HashMap;
use std::sync::Arc;

use nana_ui_core::LengthSpec;
use nana_ui_core::{
    InvalidationKind, InvalidationReason, LayoutDependencyFootprint, LayoutFieldMask,
    LayoutInvalidation, LayoutInvalidationSource, LayoutMetricDelta,
};
use nana_ui_runtime::{
    DocumentId, LayoutDependencyGraph, LayoutFrontier, LayoutFrontierSeed, LayoutStyle,
    MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld,
};

fn id(value: u64) -> StableNodeId {
    StableNodeId::new(value).expect("test ids are non-zero")
}

fn document(value: u64) -> DocumentId {
    DocumentId::new(value).expect("test documents are non-zero")
}

#[test]
fn mutation_authority_emits_typed_style_seed() {
    let mut world = UiWorld::new();
    let mut create = MutationQueue::new();
    create.create(id(1), document(1), NodeKind::Document);
    create.create(
        id(2),
        document(1),
        NodeKind::Element {
            tag: "section".into(),
        },
    );
    create.insert(id(1), id(2), None);
    world.commit(create).expect("initial tree is valid");
    let _ = world.take_system_work();

    let mut update = MutationQueue::new();
    update.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(120.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(update).expect("style mutation is valid");
    let work = world.take_system_work();
    assert!(
        work.layout_frontier_seeds
            .iter()
            .any(|seed| seed.node == id(2))
    );
    let seed = work
        .layout_frontier_seeds
        .iter()
        .find(|seed| seed.node == id(2))
        .expect("style mutation must publish a typed seed");
    assert_eq!(seed.invalidation.source, LayoutInvalidationSource::Author);
    assert!(
        seed.invalidation
            .reason
            .intersects(InvalidationReason::STYLE)
    );
    assert!(
        seed.invalidation
            .changed_inputs
            .intersects(LayoutFieldMask::SIZING)
    );
    assert!(
        seed.invalidation
            .affected_axes
            .intersects(LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT)
    );
}

fn parent_map() -> HashMap<StableNodeId, StableNodeId> {
    // root 1 -> cards 2..11 -> labels 100..199
    let mut parents = HashMap::new();
    for card in 0..10 {
        let card_id = id(2 + card);
        parents.insert(card_id, id(1));
        for label in 0..10 {
            parents.insert(id(100 + card * 10 + label), card_id);
        }
    }
    parents
}

fn seed(node: StableNodeId, kind: InvalidationKind) -> LayoutFrontierSeed {
    LayoutFrontierSeed::new(
        node,
        LayoutInvalidation::new(
            LayoutInvalidationSource::Text,
            InvalidationReason::TEXT,
            kind,
            LayoutFieldMask::TYPOGRAPHY,
            LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE,
        ),
    )
}

#[test]
fn batched_seeds_union_shared_ancestors_and_bound_edge_visits() {
    let parents = parent_map();
    let seeds = (0..10)
        .flat_map(|card| (0..10).map(move |label| id(100 + card * 10 + label)))
        .flat_map(|node| [node, node])
        .map(|node| seed(node, InvalidationKind::MEASURE));
    let frontier = LayoutFrontier::from_seeds(seeds, |node| parents.get(&node).copied(), |_| false);

    assert_eq!(frontier.seeds(), 200);
    assert!(
        frontier.seed_merges() >= 100,
        "same-node seeds and shared card/root nodes must merge"
    );
    assert!(frontier.measure_nodes().len() >= 100);
    assert!(frontier.placement_nodes().is_empty());
    // The union closure has 111 nodes and 110 parent edges.  A batch must not
    // perform one complete root walk per seed.
    assert!(
        frontier.dependency_edges_visited() <= 2 * 110,
        "edge visits {} exceed the union-closure bound",
        frontier.dependency_edges_visited()
    );
    assert_eq!(frontier.full_document_fallbacks(), 0);
}

#[test]
fn none_metric_delta_stops_without_seeding_or_ancestor_walk() {
    let parents = parent_map();
    let mut frontier = LayoutFrontier::default();
    frontier.propagate_metric_delta(
        id(100),
        LayoutMetricDelta::NONE,
        |node| parents.get(&node).copied(),
        |_| false,
    );

    assert!(frontier.nodes().is_empty());
    assert_eq!(frontier.seeds(), 0);
    assert_eq!(frontier.dependency_edges_visited(), 0);
    assert_eq!(frontier.propagations_stopped(), 1);
}

#[test]
fn scroll_metric_delta_keeps_scroll_frontier_on_ancestors() {
    let parents = parent_map();
    let mut frontier = LayoutFrontier::default();
    frontier.propagate_metric_delta(
        id(100),
        LayoutMetricDelta::SCROLL_EXTENT,
        |node| parents.get(&node).copied(),
        |_| false,
    );

    assert!(frontier.scroll_nodes().contains(&id(100)));
    assert!(frontier.scroll_nodes().contains(&id(2)));
    assert!(frontier.scroll_nodes().contains(&id(1)));
}

#[test]
fn empty_invalidation_does_not_enter_frontier() {
    let parents = parent_map();
    let frontier = LayoutFrontier::from_seeds(
        [LayoutFrontierSeed::new(id(100), LayoutInvalidation::none())],
        |node| parents.get(&node).copied(),
        |_| false,
    );
    assert!(frontier.nodes().is_empty());
    assert_eq!(frontier.seeds(), 0);
    assert_eq!(frontier.propagations_stopped(), 1);
}

#[test]
fn placement_only_seed_does_not_enter_ancestor_measure_frontier() {
    let parents = parent_map();
    let frontier = LayoutFrontier::from_seeds(
        [seed(id(100), InvalidationKind::PLACEMENT)],
        |node| parents.get(&node).copied(),
        |_| false,
    );

    assert!(frontier.measure_nodes().is_empty());
    assert!(frontier.placement_nodes().contains(&id(100)));
    assert!(frontier.placement_nodes().contains(&id(2)));
    assert!(frontier.placement_nodes().contains(&id(1)));
}

#[test]
fn isolated_context_stops_frontier_before_unrelated_ancestors() {
    let parents = parent_map();
    let frontier = LayoutFrontier::from_seeds(
        [seed(id(100), InvalidationKind::MEASURE)],
        |node| parents.get(&node).copied(),
        |node| node == id(2),
    );

    assert!(frontier.contains(id(100)));
    assert!(frontier.contains(id(2)));
    assert!(!frontier.contains(id(1)));
    assert!(frontier.propagations_stopped() >= 1);
}

#[test]
fn dependency_graph_keeps_top_down_and_context_work_local() {
    let root = id(1);
    let first = id(2);
    let second = id(3);
    let unrelated = id(4);
    let dependency = LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT
        .union(LayoutDependencyFootprint::DEPENDS_ON_SIBLING_PREFIX)
        .union(LayoutDependencyFootprint::CONTEXT_LOCAL_COUPLING);
    let mut graph = LayoutDependencyGraph::default();
    graph.add_parent_dependency(root, first, dependency);
    graph.add_parent_dependency(
        root,
        unrelated,
        LayoutDependencyFootprint::CONSUMES_PARENT_BLOCK_CONSTRAINT,
    );
    graph.add_context_dependency(first, second, dependency);

    let seed = LayoutFrontierSeed::new(
        root,
        LayoutInvalidation::new(
            LayoutInvalidationSource::Runtime,
            InvalidationReason::PARENT_CONSTRAINT,
            InvalidationKind::MEASURE,
            LayoutFieldMask::SIZING,
            LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT,
        ),
    );
    let frontier = LayoutFrontier::from_dependency_graph([seed], &graph);

    assert!(frontier.measure_nodes().contains(&root));
    assert!(frontier.measure_nodes().contains(&first));
    assert!(!frontier.measure_nodes().contains(&unrelated));

    let context_seed = LayoutFrontierSeed::new(
        first,
        LayoutInvalidation::new(
            LayoutInvalidationSource::Runtime,
            InvalidationReason::SIBLING,
            InvalidationKind::CONTEXT_REFLOW,
            LayoutFieldMask::ALIGNMENT,
            LayoutDependencyFootprint::DEPENDS_ON_SIBLING_PREFIX,
        ),
    );
    let context_frontier = LayoutFrontier::from_dependency_graph([context_seed], &graph);
    assert!(context_frontier.context_nodes().contains(&second));
    assert!(context_frontier.placement_nodes().contains(&second));
}

#[test]
fn dependency_graph_respects_isolated_seed_boundary() {
    let root = id(1);
    let isolated = id(2);
    let leaf = id(3);
    let mut graph = LayoutDependencyGraph::default();
    graph.add_parent_dependency(
        root,
        isolated,
        LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS,
    );
    graph.add_parent_dependency(
        isolated,
        leaf,
        LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS,
    );
    graph.isolate(isolated);

    let frontier = LayoutFrontier::from_dependency_graph(
        [LayoutFrontierSeed::new(
            leaf,
            LayoutInvalidation::new(
                LayoutInvalidationSource::Text,
                InvalidationReason::TEXT,
                InvalidationKind::MEASURE,
                LayoutFieldMask::TYPOGRAPHY,
                LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE
                    .union(LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS),
            ),
        )],
        &graph,
    );
    assert!(frontier.contains(leaf));
    assert!(frontier.contains(isolated));
    assert!(!frontier.contains(root));
}
