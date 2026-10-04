//! Focused regression tests for dependency-aware layout propagation.
//!
//! A text node can keep the same inline/block extents while changing its
//! exported baseline.  That is still a metric delta for an ancestor which
//! participates in baseline alignment; treating width/height as the complete
//! metric silently leaves retained placement stale.  Keep this test outside
//! the large engine fixture so it exercises the public Runtime pipeline.

use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_runtime::{
    AlignSpec, ComputedStyle, DocumentId, FlexDirection, LayoutStyle, MutationQueue, NodeKind,
    NodeStyle, StableNodeId, TextContent, TextMetrics, TextShaper, UiWorld,
};

fn id(value: u64) -> StableNodeId {
    StableNodeId::new(value).expect("test ids are non-zero")
}

fn document(value: u64) -> DocumentId {
    DocumentId::new(value).expect("test documents are non-zero")
}

/// Return equal extents for both values while exposing a different baseline.
/// The runtime must compare the full exported metric, not only width/height.
struct BaselineChangingShaper;

impl TextShaper for BaselineChangingShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        _style: &ComputedStyle,
        _constraints: nana_ui_runtime::TextShapeConstraints,
    ) -> TextMetrics {
        TextMetrics {
            width: 30.0,
            height: 10.0,
            ascent: Some(if text.value.as_ref() == "abc" {
                7.0
            } else {
                8.0
            }),
        }
    }
}

#[test]
fn baseline_only_metric_delta_reaches_layout_ancestors() {
    let mut world = UiWorld::new();
    let mut initial = MutationQueue::new();
    initial.create(id(1), document(1), NodeKind::Document);
    initial.create(
        id(2),
        document(1),
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    initial.create(id(3), document(1), NodeKind::Text);
    initial.insert(id(1), id(2), None);
    initial.insert(id(2), id(3), None);
    initial.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                direction: Some(FlexDirection::Row),
                align_items: AlignSpec::Baseline,
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    initial.set_text(id(3), TextContent::new("abc"));
    world.commit(initial).expect("initial tree is valid");

    let mut shaper = BaselineChangingShaper;
    let work = world.take_system_work();
    world
        .resolve_styles(&work.style)
        .expect("initial styles resolve");
    world
        .shape_text(&work.text, &mut shaper)
        .expect("initial text shapes");
    let _ = world.take_system_work();

    let mut update = MutationQueue::new();
    update.set_text(id(3), TextContent::new("xyz"));
    world.commit(update).expect("text mutation is valid");
    let work = world.take_system_work();
    world
        .resolve_styles(&work.style)
        .expect("updated styles resolve");
    world
        .shape_text(&work.text, &mut shaper)
        .expect("updated text shapes");

    let propagated = world.take_system_work();
    assert!(
        propagated
            .layout_frontier_seeds
            .iter()
            .any(|seed| seed.node == id(3)),
        "the changed text node remains layout-dirty"
    );
    assert!(
        propagated
            .layout_frontier_seeds
            .iter()
            .any(|seed| seed.node == id(2)),
        "baseline-only metric changes must reach the consuming parent"
    );
}

#[test]
fn one_frame_metric_batch_emits_each_frontier_node_once() {
    const CARDS: u64 = 10;
    const LABELS_PER_CARD: u64 = 10;
    let mut world = UiWorld::new();
    let mut initial = MutationQueue::new();
    initial.create(id(1), document(1), NodeKind::Document);
    for card in 0..CARDS {
        let card_id = 2 + card;
        initial.create(
            id(card_id),
            document(1),
            NodeKind::Element { tag: "card".into() },
        );
        initial.insert(id(1), id(card_id), None);
        initial.set_style(
            id(card_id),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    direction: Some(FlexDirection::Row),
                    align_items: AlignSpec::Baseline,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        for label in 0..LABELS_PER_CARD {
            let label_id = 100 + card * LABELS_PER_CARD + label;
            initial.create(id(label_id), document(1), NodeKind::Text);
            initial.insert(id(card_id), id(label_id), None);
            initial.set_text(id(label_id), TextContent::new("abc"));
        }
    }
    world.commit(initial).expect("initial tree is valid");

    let mut shaper = BaselineChangingShaper;
    let work = world.take_system_work();
    world
        .resolve_styles(&work.style)
        .expect("initial styles resolve");
    world
        .shape_text(&work.text, &mut shaper)
        .expect("initial text shapes");
    let _ = world.take_system_work();

    let mut batch = MutationQueue::new();
    for card in 0..CARDS {
        for label in 0..LABELS_PER_CARD {
            let label_id = 100 + card * LABELS_PER_CARD + label;
            batch.set_text(id(label_id), TextContent::new("xyz"));
        }
    }
    world.commit(batch).expect("batched text mutation is valid");
    let work = world.take_system_work();
    world
        .resolve_styles(&work.style)
        .expect("updated styles resolve");
    world
        .shape_text(&work.text, &mut shaper)
        .expect("updated text shapes");

    let propagated = world.take_system_work();
    let unique = propagated
        .layout_frontier_seeds
        .iter()
        .map(|seed| seed.node)
        .collect::<HashSet<_>>();
    assert_eq!(
        unique.len(),
        propagated.layout_frontier_seeds.len(),
        "shared ancestors must not be emitted once per seed"
    );
    for card in 0..CARDS {
        assert!(
            unique.contains(&id(2 + card)),
            "baseline consumer enters the frontier"
        );
        for label in 0..LABELS_PER_CARD {
            assert!(unique.contains(&id(100 + card * LABELS_PER_CARD + label)));
        }
    }
    assert!(
        propagated.layout_frontier_seeds.len()
            <= 1 + CARDS as usize + (CARDS * LABELS_PER_CARD) as usize,
        "the batch frontier is bounded by the unique union closure"
    );
}
