//! Structural tests for the dependency-aware dirty frontier.

use std::sync::Arc;

use nana_ui_core::LengthSpec;
use nana_ui_core::{
    InvalidationKind, InvalidationReason, LayoutDependencyFootprint, LayoutFieldMask,
    LayoutInvalidation, LayoutInvalidationSource,
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
