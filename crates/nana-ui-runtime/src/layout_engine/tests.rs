use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, BoxSizing, CalcBinOp, CalcExpr, DirSpec, DisplaySpec, FlexDirection, FlexWrap,
    GridLine, GridPlacement, GridRepeatAuto, GridTrack, GridTrackListUnsupported, JustifySpec,
    LayoutStyle, LengthSpec, LineHeightSpec, PositionSpec, TextOrientationSpec, WhiteSpaceSpec,
    WritingModeSpec,
};

use crate::{
    ComputedStyle, MutationQueue, NodeKind, NodeStyle, TextContent, TextMetrics, TextShaper,
    UiWorld,
};

use super::*;

fn id(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}

fn seeds(ids: &[StableNodeId]) -> Vec<crate::LayoutFrontierSeed> {
    ids.iter()
        .copied()
        .map(crate::LayoutFrontierSeed::layout)
        .collect()
}

#[test]
fn isolated_leaf_preserves_relative_position_and_resolved_padding() {
    use crate::{AppContext, Stack};
    let document = DocumentId::new(42).unwrap();
    let mut context = AppContext::new();
    let root = context
        .create_component(document, Stack::column(0.0).width(LengthSpec::Px(200.0)))
        .unwrap();
    let leaf = context
        .create_detached_component(
            document,
            Stack::from_layout(LayoutStyle {
                width: Some(LengthSpec::Px(100.0)),
                height: Some(LengthSpec::Px(80.0)),
                padding: Some(LengthSpec::Percent(10.0)),
                position: PositionSpec::Relative,
                offset_left: Some(LengthSpec::Px(7.0)),
                layout_isolation: true,
                ..Default::default()
            }),
        )
        .unwrap();
    context.append_child(root, leaf).unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    context.layout_document(document, viewport).unwrap();
    let before = context.world().layout_box(leaf.stable_id()).unwrap();
    assert_eq!((before.x, before.width, before.height), (7.0, 100.0, 80.0));
    context
        .layout_document_with_frontier(document, viewport, &seeds(&[leaf.stable_id()]))
        .unwrap();
    assert_eq!(
        context.world().layout_box(leaf.stable_id()).unwrap(),
        before
    );
    let extracted = context.world().extract_nodes(&[leaf.stable_id()]);
    assert_eq!(
        extracted[0].source_style.layout.resolved_padding().left,
        20.0
    );
    context
        .update_component(root, |root, _| {
            *root = root.clone().width(LengthSpec::Px(300.0))
        })
        .unwrap();
    context
        .layout_document_with_frontier(document, viewport, &seeds(&[root.stable_id()]))
        .unwrap();
    assert_eq!(
        context.world().layout_box(leaf.stable_id()).unwrap(),
        before
    );
    let extracted = context.world().extract_nodes(&[leaf.stable_id()]);
    assert_eq!(
        extracted[0].source_style.layout.resolved_padding().left,
        30.0
    );
}

#[test]
fn scoped_layout_ignores_isolated_dirty_nodes_from_another_document() {
    let first = DocumentId::new(1).unwrap();
    let second = DocumentId::new(2).unwrap();
    let mut world = UiWorld::new();
    let mut mutations = MutationQueue::new();
    mutations.create(id(1), first, NodeKind::Document);
    mutations.create(id(10), second, NodeKind::Document);
    mutations.create(id(11), second, NodeKind::Element { tag: "div".into() });
    mutations.insert(id(10), id(11), None);
    mutations.set_style(
        id(11),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(100.0)),
                height: Some(LengthSpec::Px(100.0)),
                position: PositionSpec::Static,
                layout_isolation: true,
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    world.commit(mutations).unwrap();
    let engine = RuntimeLayoutEngine;
    let mut cache = RetainedLayoutCache::default();
    engine
        .layout_document_with_frontier(
            &world,
            second,
            LayoutViewport::new(800.0, 600.0),
            &[],
            &mut cache,
            true,
        )
        .unwrap();
    let previous = cache.documents[&second].boxes[&id(11)];
    let emitted = engine
        .layout_document_with_frontier(
            &world,
            first,
            LayoutViewport::new(320.0, 240.0),
            &seeds(&[id(11)]),
            &mut cache,
            false,
        )
        .unwrap();
    assert!(
        emitted
            .iter()
            .all(|(node, _)| world.document_of(*node) == Some(first))
    );
    assert_eq!(cache.documents[&second].boxes[&id(11)], previous);
    engine
        .layout_document_with_frontier(
            &world,
            first,
            LayoutViewport::new(320.0, 240.0),
            &[],
            &mut cache,
            true,
        )
        .unwrap();
    assert_eq!(cache.documents[&second].boxes[&id(11)], previous);
    let isolated = engine
        .layout_document_with_frontier(
            &world,
            second,
            LayoutViewport::new(800.0, 600.0),
            &seeds(&[id(11)]),
            &mut cache,
            false,
        )
        .unwrap();
    assert_eq!(isolated, vec![(id(11), previous)]);
    cache.remove_document(first);
    assert!(!cache.documents.contains_key(&first));
    assert_eq!(cache.documents[&second].boxes[&id(11)], previous);
    let empty = DocumentId::new(3).unwrap();
    assert!(
        engine
            .layout_document_with_frontier(
                &world,
                empty,
                LayoutViewport::new(800.0, 600.0),
                &[],
                &mut cache,
                true,
            )
            .unwrap()
            .is_empty()
    );
    assert!(!cache.documents.contains_key(&empty));
}

/// Column of `rows` fixed-height rows, each with one fixed-height label,
/// under document(1) → column(2). Row `r` is id(3 + r*2), label id(4 + r*2).
fn column_tree(rows: u64) -> (UiWorld, DocumentId) {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(1), id(2), None);
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(300.0)),
                height: Some(LengthSpec::Fill),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    for row in 0..rows {
        let row_id = id(3 + row * 2);
        let label_id = id(4 + row * 2);
        queue.create(row_id, document, NodeKind::Element { tag: "div".into() });
        queue.create(label_id, document, NodeKind::Text);
        queue.insert(id(2), row_id, None);
        queue.insert(row_id, label_id, None);
        queue.set_text(
            label_id,
            TextContent {
                value: "行".into()
            },
        );
        queue.set_style(
            row_id,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(300.0)),
                    height: Some(LengthSpec::Px(20.0)),
                    direction: Some(FlexDirection::Row),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            label_id,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(40.0)),
                    height: Some(LengthSpec::Px(20.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    world.commit(queue).unwrap();
    (world, document)
}

#[test]
fn percent_height_child_resolves_against_the_used_content_box() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Element { tag: "div".into() });
    queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(1), id(2), None);
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(100.0)),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(40.0)),
                height: Some(LengthSpec::Percent(50.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(100.0, 200.0))
        .unwrap();
    let parent = layouts
        .iter()
        .find(|(node_id, _)| *node_id == id(1))
        .expect("parent box")
        .1;
    let child = layouts
        .iter()
        .find(|(node_id, _)| *node_id == id(2))
        .expect("child box")
        .1;
    assert_eq!(parent.width, 100.0);
    assert_eq!(parent.height, 100.0);
    assert_eq!(child.width, 40.0);
    assert_eq!(child.height, 50.0);
}

#[test]
fn childless_text_leaf_resolves_to_its_metrics() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Element { tag: "div".into() });
    queue.create(id(2), document, NodeKind::Text);
    queue.create(id(3), document, NodeKind::Text);
    queue.insert(id(1), id(2), None);
    queue.insert(id(1), id(3), None);
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                // A flex column keeps each leaf at its own width; a block
                // would stretch them across.
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_text(
        id(2),
        TextContent {
            value: "row".into(),
        },
    );
    queue.set_text(
        id(3),
        TextContent {
            value: "pad".into(),
        },
    );
    queue.set_style(
        id(3),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                padding: Some(LengthSpec::Px(4.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();

    struct Metrics;
    impl TextShaper for Metrics {
        fn shape(
            &mut self,
            node: StableNodeId,
            _text: &TextContent,
            _style: &ComputedStyle,
            _constraints: crate::TextShapeConstraints,
        ) -> TextMetrics {
            if node == id(2) {
                TextMetrics {
                    width: 48.0,
                    height: 12.0,
                    ascent: Some(9.0),
                }
            } else {
                TextMetrics {
                    width: 20.0,
                    height: 10.0,
                    ascent: Some(8.0),
                }
            }
        }
    }
    world.shape_text(&[id(2), id(3)], &mut Metrics).unwrap();

    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(200.0, 100.0))
        .unwrap();
    let box_of = |node| {
        layouts
            .iter()
            .find(|(node_id, _)| *node_id == node)
            .expect("laid out")
            .1
    };
    let plain = box_of(id(2));
    let padded = box_of(id(3));
    let parent = box_of(id(1));
    assert_eq!(plain.width, 48.0);
    assert_eq!(plain.height, 12.0);
    assert_eq!(padded.width, 28.0);
    assert_eq!(padded.height, 18.0);
    assert_eq!(parent.width, 200.0);
    assert_eq!(parent.height, 30.0);
}

fn resize_row(world: &mut UiWorld, row: u64, height: f32) {
    let mut queue = MutationQueue::new();
    queue.set_style(
        id(3 + row * 2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(300.0)),
                height: Some(LengthSpec::Px(height)),
                direction: Some(FlexDirection::Row),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
}

#[test]
fn returning_to_old_viewport_does_not_restore_sizes_from_before_a_content_change() {
    let (mut world, document) = column_tree(4);
    let mut root_style = NodeStyle::default();
    Arc::make_mut(&mut root_style.layout).height = Some(LengthSpec::Fill);
    let mut mutations = MutationQueue::new();
    mutations.set_style(id(1), root_style);
    let mut row_style = world.node_style(id(3)).unwrap().clone();
    Arc::make_mut(&mut row_style.layout).height = None;
    mutations.set_style(id(3), row_style);
    world.commit(mutations).unwrap();
    let mut retained = RetainedLayoutCache::default();
    let original = LayoutViewport::new(300.0, 800.0);
    let smaller = LayoutViewport::new(300.0, 600.0);
    let engine = RuntimeLayoutEngine;
    let boxes = engine
        .layout_document_with_frontier(&world, document, original, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &boxes);
    let mut label_style = world.node_style(id(4)).unwrap().clone();
    Arc::make_mut(&mut label_style.layout).height = Some(LengthSpec::Px(35.0));
    let mut mutations = MutationQueue::new();
    mutations.set_style(id(4), label_style);
    world.commit(mutations).unwrap();
    let boxes = engine
        .layout_document_with_frontier(
            &world,
            document,
            smaller,
            &seeds(&[id(4)]),
            &mut retained,
            false,
        )
        .unwrap();
    write_changed_boxes(&mut world, &boxes);
    engine
        .layout_document_with_frontier(
            &world,
            document,
            original,
            &seeds(&[id(1)]),
            &mut retained,
            false,
        )
        .unwrap();
    assert_eq!(
        retained.documents[&document].page_boxes(&world),
        full_boxes(&world, document, original)
    );
}

#[test]
fn repeated_viewport_resize_keeps_intrinsic_history_bounded() {
    let (world, document) = column_tree(4);
    let engine = RuntimeLayoutEngine;
    let mut retained = RetainedLayoutCache::default();
    for step in 0..256 {
        let viewport = LayoutViewport::new(300.0 + step as f32, 800.0 + step as f32);
        engine
            .layout_document_with_frontier(
                &world,
                document,
                viewport,
                &seeds(&[id(1)]),
                &mut retained,
                step == 0,
            )
            .unwrap();
        assert_eq!(
            retained.documents[&document].page_boxes(&world),
            full_boxes(&world, document, viewport)
        );
    }
    let memo = &retained.documents[&document];
    assert!(memo.intrinsics.node_count() <= world.len());
    let variants = memo.intrinsics.used_variant_count();
    assert!(variants <= world.len() * 2);
}

fn full_boxes(
    world: &UiWorld,
    document: DocumentId,
    viewport: LayoutViewport,
) -> crate::store::DenseNodeMap<LayoutBox> {
    RuntimeLayoutEngine
        .layout_document(world, document, viewport)
        .unwrap()
        .into_iter()
        .collect()
}

fn write_changed_boxes(
    world: &mut UiWorld,
    emitted: &[(StableNodeId, LayoutBox)],
) -> Vec<StableNodeId> {
    let mut queue = MutationQueue::new();
    let mut written = Vec::new();
    for (id, box_) in emitted {
        if world.layout_box(*id) != Some(*box_) {
            queue.write_layout(*id, *box_);
            written.push(*id);
        }
    }
    if !written.is_empty() {
        world.commit(queue).unwrap();
    }
    written
}

#[test]
fn fixed_layout_island_reuses_outer_layout_and_resizing_reaches_siblings() {
    let (mut world, document) = column_tree(400);
    let viewport = LayoutViewport::new(300.0, 800.0);
    let island = id(203);
    let label = id(204);
    let mut style = world.node_style(island).unwrap().clone();
    Arc::make_mut(&mut style.layout).layout_isolation = true;
    let mut queue = MutationQueue::new();
    queue.set_style(island, style.clone());
    world.commit(queue).unwrap();
    world.take_system_work();
    let mut retained = RetainedLayoutCache::default();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &emitted);
    world.take_system_work();
    let mut queue = MutationQueue::new();
    queue.set_text(
        label,
        TextContent {
            value: "changed text".into(),
        },
    );
    world.commit(queue).unwrap();
    let work = world.take_system_work();
    assert!(
        !work
            .layout_frontier_seeds
            .iter()
            .any(|seed| seed.node == id(2))
    );
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(
            &world,
            document,
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            false,
        )
        .unwrap();
    assert!(emitted.len() <= 2);
    assert_eq!(
        retained.documents[&document].page_boxes(&world),
        full_boxes(&world, document, viewport)
    );
    write_changed_boxes(&mut world, &emitted);
    world.take_system_work();
    Arc::make_mut(&mut style.layout).height = Some(LengthSpec::Px(35.0));
    let mut queue = MutationQueue::new();
    queue.set_style(island, style);
    world.commit(queue).unwrap();
    let work = world.take_system_work();
    assert!(
        work.layout_frontier_seeds
            .iter()
            .any(|seed| seed.node == island)
    );
    RuntimeLayoutEngine
        .layout_document_with_frontier(
            &world,
            document,
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            false,
        )
        .unwrap();
    assert_eq!(
        retained.documents[&document].page_boxes(&world),
        full_boxes(&world, document, viewport)
    );
}

#[test]
fn auto_sized_isolation_request_preserves_parent_layout_dependency() {
    let (mut world, document) = column_tree(8);
    let viewport = LayoutViewport::new(300.0, 800.0);
    let mut style = world.node_style(id(3)).unwrap().clone();
    let layout = Arc::make_mut(&mut style.layout);
    layout.layout_isolation = true;
    layout.height = None;
    let mut queue = MutationQueue::new();
    queue.set_style(id(3), style);
    world.commit(queue).unwrap();
    world.take_system_work();
    let mut retained = RetainedLayoutCache::default();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &emitted);
    world.take_system_work();
    let mut style = world.node_style(id(4)).unwrap().clone();
    Arc::make_mut(&mut style.layout).height = Some(LengthSpec::Px(60.0));
    let mut queue = MutationQueue::new();
    queue.set_style(id(4), style);
    world.commit(queue).unwrap();
    let work = world.take_system_work();
    assert!(
        work.layout_frontier_seeds
            .iter()
            .any(|seed| seed.node == id(4)),
        "auto-size edit must remain a typed seed"
    );
    RuntimeLayoutEngine
        .layout_document_with_frontier(
            &world,
            document,
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            false,
        )
        .unwrap();
    assert_eq!(
        retained.documents[&document].page_boxes(&world),
        full_boxes(&world, document, viewport)
    );
}

#[test]
fn scoped_layout_touches_only_the_change_closure_and_matches_full_recompute() {
    let (mut world, document) = column_tree(400);
    let viewport = LayoutViewport::new(300.0, 800.0);
    let mut retained = RetainedLayoutCache::default();

    // Production drains dirty work before layout; the create-time marks
    // must not leak into the scoped measurement below.
    let _ = world.take_system_work();

    // Bootstrap: full pass populates the retained cache with every box.
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    assert_eq!(emitted.len(), 802, "full pass emits every node");
    write_changed_boxes(&mut world, &emitted);
    let _ = world.take_system_work();

    // Change the LAST row: nothing shifts above it, so the scoped pass
    // must recompute only that row's ancestor chain.
    resize_row(&mut world, 399, 26.0);
    let work = world.take_system_work();
    assert!(!work.layout_frontier_seeds.is_empty());
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(
            &world,
            document,
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            false,
        )
        .unwrap();
    assert!(
        emitted.len() < 16,
        "tail row change must stay O(depth), not relayout {} nodes",
        emitted.len()
    );
    for (node, box_) in full_boxes(&world, document, viewport) {
        assert_eq!(
            retained.documents[&document].page_boxes(&world).get(&node),
            Some(&box_),
            "scoped layout diverged from full recompute at {node:?}"
        );
    }
    write_changed_boxes(&mut world, &emitted);
    let _ = world.take_system_work();

    // Change a MIDDLE row: every row below shifts; the scoped pass must
    // emit exactly the shifted set (rows and their labels) and still
    // match a full recompute. Rows above stay pruned.
    resize_row(&mut world, 200, 32.0);
    let work = world.take_system_work();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(
            &world,
            document,
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            false,
        )
        .unwrap();
    assert!(emitted.len() > 16, "shifted rows must be re-emitted");
    // 199 shifted rows + labels + the change closure; the 400 nodes above
    // the change must stay pruned (well under the 802-node document).
    assert!(
        emitted.len() < 420,
        "rows above the change stay pruned, got {} of 802",
        emitted.len()
    );
    for (node, box_) in full_boxes(&world, document, viewport) {
        assert_eq!(
            retained.documents[&document].page_boxes(&world).get(&node),
            Some(&box_),
            "shifted scoped layout diverged from full recompute at {node:?}"
        );
    }

    let written = write_changed_boxes(&mut world, &emitted);
    let extract = world.take_system_work();
    let changed_row = id(3 + 200 * 2);
    let changed_label = id(4 + 200 * 2);
    let row_above = id(3 + 199 * 2);
    let label_above = id(4 + 199 * 2);
    let shifted_row = id(3 + 201 * 2);
    let shifted_label = id(4 + 201 * 2);
    assert!(written.contains(&changed_row));
    assert!(written.contains(&shifted_row));
    // Its box is measured from the row's: it moves with the row unwritten.
    assert!(!written.contains(&shifted_label));
    assert!(extract.render_extraction.contains(&changed_row));
    assert!(extract.render_extraction.contains(&shifted_row));
    // Nor is it marked: its record moves with the row's.
    assert!(!extract.render_extraction.contains(&shifted_label));
    assert!(extract.layout_moved.contains(&shifted_row));
    assert!(
        !written.contains(&changed_label),
        "bit-identical label of the changed row must not be written"
    );
    assert!(!extract.render_extraction.contains(&changed_label));
    assert!(!extract.render_extraction.contains(&row_above));
    assert!(!extract.render_extraction.contains(&label_above));
}

#[test]
fn scoped_layout_materializes_far_fewer_inputs_than_the_document_for_a_tail_row() {
    let (mut world, document) = column_tree(400);
    let viewport = LayoutViewport::new(300.0, 800.0);
    let mut retained = RetainedLayoutCache::default();
    let _ = world.take_system_work();

    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    assert_eq!(emitted.len(), 802);
    assert_eq!(retained.documents[&document].materialized_inputs, 802);
    write_changed_boxes(&mut world, &emitted);
    let _ = world.take_system_work();

    resize_row(&mut world, 399, 26.0);
    let work = world.take_system_work();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(
            &world,
            document,
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            false,
        )
        .unwrap();
    assert!(
        emitted.len() < 16,
        "tail row change must stay O(depth), not relayout {} nodes",
        emitted.len()
    );
    // Document + column + dirty row (+ label / path ancestors). Unshifted
    // siblings are classified from layout style, not full LayoutInput.
    assert!(
        retained.documents[&document].materialized_inputs <= 16,
        "tail row must not assemble unshifted siblings, materialized {} of 802",
        retained.documents[&document].materialized_inputs
    );
    for (node, box_) in full_boxes(&world, document, viewport) {
        assert_eq!(
            retained.documents[&document].page_boxes(&world).get(&node),
            Some(&box_),
            "on-demand scoped layout diverged from full recompute at {node:?}"
        );
    }
}

#[test]
fn lays_out_shaped_controls_without_application_geometry() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    queue.create(
        id(2),
        document,
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.insert(id(1), id(2), None);
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                // A flex column keeps the button at its own width.
                display: Some(DisplaySpec::Flex),
                width: Some(LengthSpec::Fill),
                direction: Some(FlexDirection::Column),
                padding: Some(LengthSpec::Px(12.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                padding_left: Some(LengthSpec::Px(8.0)),
                padding_right: Some(LengthSpec::Px(8.0)),
                min_height: Some(LengthSpec::Px(32.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_text(
        id(2),
        TextContent {
            value: "Build".into(),
        },
    );
    world.commit(queue).unwrap();
    struct FixedShaper;
    impl TextShaper for FixedShaper {
        fn shape(
            &mut self,
            _id: StableNodeId,
            _text: &TextContent,
            _style: &ComputedStyle,
            _constraints: crate::TextShapeConstraints,
        ) -> TextMetrics {
            TextMetrics {
                width: 40.0,
                height: 18.0,
                ascent: None,
            }
        }
    }
    world.shape_text(&[id(2)], &mut FixedShaper).unwrap();

    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(320.0, 180.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(layouts[&id(1)].width, 320.0);
    assert_eq!(layouts[&id(2)].x, 12.0);
    assert_eq!(layouts[&id(2)].y, 12.0);
    assert_eq!(layouts[&id(2)].width, 56.0);
    assert_eq!(layouts[&id(2)].height, 32.0);
}

/// A button's glyph is measured beside its label, but it lives in the
/// button's visual, not in its style or text. A retained pass that adds or
/// drops one measures the button again instead of keeping the width a
/// measure plan recorded without it (each pass is checked against a full
/// layout under `cfg(test)`).
#[test]
fn a_button_that_gains_or_loses_a_glyph_is_measured_again() {
    use crate::{AppContext, Button, ButtonVisual, MeasureTextShaper, Stack};
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let page = cx
        .create_component(
            document,
            Stack::column(0.0).with_layout(|layout| {
                layout.width = Some(LengthSpec::Px(400.0));
                layout.align_items = AlignSpec::Start;
            }),
        )
        .unwrap();
    let button = cx.create_component(document, Button::new("Play")).unwrap();
    cx.append_child(page, button).unwrap();
    // One frame the way `RuntimeDocument::flush` runs it.
    let frame = |cx: &mut AppContext| {
        let mut shaper = MeasureTextShaper;
        let viewport = LayoutViewport::new(600.0, 400.0);
        for _ in 0..8 {
            let work = cx.take_system_work();
            let mut seeds = work.layout_frontier_seeds.clone();
            if !work.is_empty() {
                cx.resolve_styles(&work.style).unwrap();
                cx.shape_text(&work.text, &mut shaper).unwrap();
            }
            seeds.extend(cx.take_layout_frontier_seeds(document));
            if work.is_empty() && seeds.is_empty() {
                return;
            }
            if !seeds.is_empty() {
                cx.layout_document_for_viewport(document, viewport, &seeds)
                    .unwrap();
                let scope = cx.take_last_layout_scope();
                cx.shape_text_for_layout_scoped(&scope, &mut shaper)
                    .unwrap();
            }
        }
        panic!("the frame did not settle");
    };
    let width = |cx: &AppContext| cx.world().layout_box(button.stable_id()).unwrap().width;
    frame(&mut cx);
    let bare = width(&cx);
    cx.update_component(button, |button, _| {
        button.icon = Some(nana_ui_core::Icon::Add);
    })
    .unwrap();
    frame(&mut cx);
    let Some(ButtonVisual {
        icon_size,
        icon_gap,
        ..
    }) = cx
        .world()
        .widget_visual_as::<ButtonVisual>(button.stable_id())
    else {
        panic!("button visual");
    };
    assert_eq!(width(&cx), bare + icon_size + icon_gap);
    cx.update_component(button, |button, _| button.icon = None)
        .unwrap();
    frame(&mut cx);
    assert_eq!(width(&cx), bare);
}

#[test]
fn display_none_child_does_not_take_a_gap_slot() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    for value in 2..=4 {
        queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(1), id(value), None);
    }
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(40.0)),
                direction: Some(FlexDirection::Row),
                gap: Some(LengthSpec::Px(10.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    for value in [2, 4] {
        queue.set_style(
            id(value),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(50.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    queue.set_style(
        id(3),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                display: Some(nana_ui_core::DisplaySpec::None),
                width: Some(LengthSpec::Px(50.0)),
                height: Some(LengthSpec::Px(40.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(200.0, 40.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(layouts[&id(3)].width, 0.0);
    assert_eq!(layouts[&id(3)].height, 0.0);
    assert_eq!(layouts[&id(2)].x, 0.0);
    assert_eq!(layouts[&id(4)].x, 60.0);
}

#[test]
fn row_fill_uses_remaining_content_width() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    for value in 2..=3 {
        queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(1), id(value), None);
    }
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(300.0)),
                height: Some(LengthSpec::Px(40.0)),
                direction: Some(FlexDirection::Row),
                gap: Some(LengthSpec::Px(10.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(50.0)),
                height: Some(LengthSpec::Fill),
                flex_shrink: Some(0.0),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(3),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Fill),
                height: Some(LengthSpec::Fill),
                margin_left: Some(LengthSpec::Px(10.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(300.0, 40.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(layouts[&id(2)].width, 50.0);
    assert_eq!(layouts[&id(3)].x, 70.0);
    assert_eq!(layouts[&id(3)].width, 230.0);
}

#[test]
fn column_fill_width_subtracts_negative_margins_symmetrically() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(1), id(2), None);
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(100.0)),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Fill),
                height: Some(LengthSpec::Px(10.0)),
                margin_left: Some(LengthSpec::Px(-10.0)),
                margin_right: Some(LengthSpec::Px(-10.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(200.0, 100.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(layouts[&id(2)].x, -10.0);
    assert_eq!(layouts[&id(2)].width, 220.0);
}

#[test]
fn unspecified_flex_shrink_keeps_overflowing_definite_row() {
    // Issue #22: omitted flex-shrink is 0, not CSS initial 1.
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    for value in 2..=3 {
        queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(1), id(value), None);
    }
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(40.0)),
                direction: Some(FlexDirection::Row),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    for value in [2, 3] {
        queue.set_style(
            id(value),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(150.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    world.commit(queue).unwrap();
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(200.0, 40.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(layouts[&id(2)].width, 150.0);
    assert_eq!(layouts[&id(3)].width, 150.0);
    assert_eq!(layouts[&id(3)].x, 150.0);
}

#[test]
fn row_space_between_auto_children_keep_the_trailing_control_inside() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    for value in 2..=5 {
        queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
    }
    queue.create(id(6), document, NodeKind::Text);
    queue.insert(id(1), id(2), None);
    queue.insert(id(2), id(3), None);
    queue.insert(id(2), id(5), None);
    queue.insert(id(3), id(4), None);
    queue.insert(id(4), id(6), None);
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Fill),
                direction: Some(FlexDirection::Column),
                padding: Some(LengthSpec::Px(20.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                direction: Some(FlexDirection::Row),
                justify_content: JustifySpec::SpaceBetween,
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(4),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                height: Some(LengthSpec::Px(16.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(5),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                padding_left: Some(LengthSpec::Px(8.0)),
                padding_right: Some(LengthSpec::Px(8.0)),
                min_height: Some(LengthSpec::Px(32.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_text(
        id(6),
        TextContent {
            value: "Title".into(),
        },
    );
    queue.set_text(
        id(5),
        TextContent {
            value: "Open".into(),
        },
    );
    world.commit(queue).unwrap();
    struct FixedShaper;
    impl TextShaper for FixedShaper {
        fn shape(
            &mut self,
            id: StableNodeId,
            _text: &TextContent,
            _style: &ComputedStyle,
            _constraints: crate::TextShapeConstraints,
        ) -> TextMetrics {
            if id.get() == 6 {
                TextMetrics {
                    width: 180.0,
                    height: 16.0,
                    ascent: None,
                }
            } else {
                TextMetrics {
                    width: 74.0,
                    height: 16.0,
                    ascent: None,
                }
            }
        }
    }
    world.shape_text(&[id(6), id(5)], &mut FixedShaper).unwrap();

    let viewport = LayoutViewport::new(400.0, 200.0);
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, viewport)
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    let trailing = layouts[&id(5)];
    assert!(
        trailing.width > 0.0 && trailing.height > 0.0,
        "trailing control must be hittable, got {trailing:?}"
    );
    assert!(
        trailing.x >= 0.0 && trailing.x + trailing.width <= viewport.width + 0.5,
        "space-between must not push the trailing control outside the viewport, got {trailing:?} viewport={}",
        viewport.width
    );
    assert!(
        layouts[&id(3)].width < layouts[&id(2)].width,
        "auto-width row cluster must shrink instead of eating the header"
    );
    assert!(
        layouts[&id(4)].width < layouts[&id(2)].width,
        "nested auto-width heading must not fill the header, got {:?}",
        layouts[&id(4)]
    );
}

#[test]
fn absolute_panel_children_resolve_fill_against_the_panel_content_box() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    for value in 1..=3 {
        queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
    }
    queue.insert(id(1), id(2), None);
    queue.insert(id(2), id(3), None);
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Fill),
                height: Some(LengthSpec::Fill),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                position: PositionSpec::Absolute,
                offset_left: Some(LengthSpec::Px(8.0)),
                width: Some(LengthSpec::Px(280.0)),
                height: Some(LengthSpec::Px(200.0)),
                padding: Some(LengthSpec::Px(8.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(3),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Fill),
                height: Some(LengthSpec::Px(32.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();

    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(1280.0, 900.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();

    assert_eq!(layouts[&id(2)].width, 280.0);
    assert_eq!(layouts[&id(3)].x, 16.0);
    assert_eq!(layouts[&id(3)].width, 264.0);
}

/// An out-of-flow box with an auto height is as tall as its content, held
/// under a percentage `max-height` that reads its containing block: the
/// viewport for `fixed`, the positioned parent for `absolute`. The content
/// is measured with no definite height, and that indefinite height must not
/// be what the percentage resolves against, or the box measures 0 tall.
#[test]
fn out_of_flow_auto_height_resolves_percent_max_height_against_its_containing_block() {
    let card = |position, max_height| StyleLayoutNode {
        id: "card".into(),
        style: LayoutStyle {
            position,
            offset_left: Some(LengthSpec::Px(10.0)),
            offset_top: Some(LengthSpec::Px(10.0)),
            width: Some(LengthSpec::Px(200.0)),
            max_height: Some(LengthSpec::Percent(max_height)),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        },
        children: vec![px_box("a", 100.0, 120.0), px_box("b", 100.0, 120.0)],
        text: None,
    };
    let page = |card| StyleLayoutNode {
        id: "page".into(),
        style: LayoutStyle {
            position: PositionSpec::Relative,
            width: Some(LengthSpec::Px(400.0)),
            height: Some(LengthSpec::Px(300.0)),
            ..LayoutStyle::default()
        },
        children: vec![card],
        text: None,
    };

    // 80% of a 600 tall viewport leaves the content its 240.
    let boxes = box_map(&page(card(PositionSpec::Fixed, 80.0)), 800.0, 600.0);
    assert_eq!(boxes["card"].height, 240.0, "{:?}", boxes["card"]);
    assert_eq!(boxes["b"].y, boxes["card"].y + 120.0);
    // 50% of a 400 tall viewport holds it to 200.
    let boxes = box_map(&page(card(PositionSpec::Fixed, 50.0)), 800.0, 400.0);
    assert_eq!(boxes["card"].height, 200.0, "{:?}", boxes["card"]);
    // An absolute box reads its positioned parent: 50% of 300.
    let boxes = box_map(&page(card(PositionSpec::Absolute, 50.0)), 800.0, 600.0);
    assert_eq!(boxes["card"].height, 150.0, "{:?}", boxes["card"]);
    let boxes = box_map(&page(card(PositionSpec::Absolute, 90.0)), 800.0, 600.0);
    assert_eq!(boxes["card"].height, 240.0, "{:?}", boxes["card"]);
}

/// The retained passes keep that: a fixed card under `max-height: 50%`
/// follows the viewport it reads and the content it holds.
#[test]
fn a_fixed_card_under_a_percent_max_height_follows_the_viewport_and_its_content() {
    use crate::{AppContext, Stack};
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let page = cx.create_component(document, Stack::column(0.0)).unwrap();
    let card = cx
        .create_component(
            document,
            Stack::column(0.0).with_layout(|layout| {
                layout.position = PositionSpec::Fixed;
                layout.offset_left = Some(LengthSpec::Px(10.0));
                layout.offset_top = Some(LengthSpec::Px(10.0));
                layout.width = Some(LengthSpec::Px(200.0));
                layout.max_height = Some(LengthSpec::Percent(50.0));
            }),
        )
        .unwrap();
    cx.append_child(page, card).unwrap();
    let row = |cx: &mut AppContext| {
        let row = cx
            .create_component(
                document,
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(100.0));
                    layout.height = Some(LengthSpec::Px(120.0));
                }),
            )
            .unwrap();
        cx.append_child(card, row).unwrap();
    };
    row(&mut cx);
    row(&mut cx);
    let height = |cx: &mut AppContext, width: f32, height: f32| {
        cx.layout_document(document, LayoutViewport::new(width, height))
            .unwrap();
        cx.world().layout_box(card.stable_id()).unwrap().height
    };
    assert_eq!(height(&mut cx, 800.0, 600.0), 240.0);
    assert_eq!(height(&mut cx, 800.0, 400.0), 200.0);
    row(&mut cx);
    assert_eq!(height(&mut cx, 800.0, 400.0), 200.0);
    assert_eq!(height(&mut cx, 800.0, 900.0), 360.0);
}

#[test]
fn fixed_content_shrink_accounts_for_flow_chrome_nesting_and_constraints() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    for value in 2..=15 {
        queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
    }

    for child in [id(2), id(6), id(9), id(13), id(15)] {
        queue.insert(id(1), child, None);
    }
    for child in [id(3), id(4), id(5)] {
        queue.insert(id(2), child, None);
    }
    for child in [id(7), id(8)] {
        queue.insert(id(6), child, None);
    }
    for child in [id(10), id(12)] {
        queue.insert(id(9), child, None);
    }
    queue.insert(id(10), id(11), None);
    queue.insert(id(13), id(14), None);

    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Fill),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Shrink),
                direction: Some(FlexDirection::Row),
                gap: Some(LengthSpec::Px(3.0)),
                padding: Some(LengthSpec::Px(2.0)),
                border_width: Some(1.0),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    for (node, width) in [(id(3), 20.0), (id(4), 30.0)] {
        queue.set_style(
            node,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(width)),
                    height: Some(LengthSpec::Px(8.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    queue.set_style(
        id(5),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                position: PositionSpec::Absolute,
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(8.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(6),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Shrink),
                direction: Some(FlexDirection::Column),
                padding: Some(LengthSpec::Px(1.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    for (node, width) in [(id(7), 40.0), (id(8), 25.0)] {
        queue.set_style(
            node,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(width)),
                    height: Some(LengthSpec::Px(8.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    queue.set_style(
        id(9),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Shrink),
                direction: Some(FlexDirection::Row),
                gap: Some(LengthSpec::Px(2.0)),
                padding: Some(LengthSpec::Px(1.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(10),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Shrink),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    for (node, width) in [(id(11), 35.0), (id(12), 10.0)] {
        queue.set_style(
            node,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(width)),
                    height: Some(LengthSpec::Px(8.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    queue.set_style(
        id(13),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Shrink),
                min_width: Some(LengthSpec::Px(50.0)),
                max_width: Some(LengthSpec::Px(55.0)),
                padding: Some(LengthSpec::Px(2.0)),
                border_width: Some(1.0),
                box_sizing: BoxSizing::ContentBox,
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(14),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(20.0)),
                height: Some(LengthSpec::Px(8.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(15),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Shrink),
                max_width: Some(LengthSpec::Px(60.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_text(
        id(15),
        TextContent {
            value: "wide".into(),
        },
    );
    world.commit(queue).unwrap();

    struct WideText;
    impl TextShaper for WideText {
        fn shape(
            &mut self,
            _id: StableNodeId,
            _text: &TextContent,
            _style: &ComputedStyle,
            _constraints: crate::TextShapeConstraints,
        ) -> TextMetrics {
            TextMetrics {
                width: 100.0,
                height: 8.0,
                ascent: None,
            }
        }
    }
    world.shape_text(&[id(15)], &mut WideText).unwrap();

    let layout_at = |width| {
        RuntimeLayoutEngine
            .layout_document(&world, document, LayoutViewport::new(width, 240.0))
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>()
    };
    let narrow = layout_at(320.0);
    let wide = layout_at(640.0);

    for layouts in [&narrow, &wide] {
        assert_eq!(layouts[&id(2)].width, 59.0);
        assert_eq!(layouts[&id(6)].width, 42.0);
        assert_eq!(layouts[&id(9)].width, 49.0);
        assert_eq!(layouts[&id(10)].width, 35.0);
        assert_eq!(layouts[&id(13)].width, 50.0);
        assert_eq!(layouts[&id(15)].width, 60.0);
    }
    for node in [id(2), id(6), id(9), id(10), id(13), id(15)] {
        assert_eq!(narrow[&node].width, wide[&node].width);
    }
}

#[test]
fn row_wrap_breaks_to_the_next_line() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    for value in 2..=5 {
        queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(1), id(value), None);
        queue.set_style(
            id(value),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(80.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                direction: Some(FlexDirection::Row),
                flex_wrap: FlexWrap::Wrap,
                gap: Some(LengthSpec::Px(8.0)),
                align_items: nana_ui_core::AlignSpec::Start,
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(200.0, 160.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(layouts[&id(2)].x, 0.0);
    assert_eq!(layouts[&id(3)].x, 88.0);
    assert_eq!(layouts[&id(4)].x, 0.0);
    assert_eq!(layouts[&id(4)].y, 48.0);
    assert_eq!(layouts[&id(5)].x, 88.0);
    assert_eq!(layouts[&id(5)].y, 48.0);
    assert_eq!(layouts[&id(1)].height, 88.0);
}

/// Lay out a wrapping row `width` wide holding `children`, and answer each
/// child's box.
fn wrapping_row(width: f32, children: &[LayoutStyle]) -> Vec<LayoutBox> {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    for (index, style) in children.iter().enumerate() {
        let child = id(index as u64 + 2);
        queue.create(child, document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(1), child, None);
        queue.set_style(
            child,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    height: Some(LengthSpec::Px(40.0)),
                    ..style.clone()
                }),
                ..NodeStyle::default()
            },
        );
    }
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(width)),
                direction: Some(FlexDirection::Row),
                flex_wrap: FlexWrap::Wrap,
                gap: Some(LengthSpec::Px(8.0)),
                align_items: AlignSpec::Start,
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(width, 400.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    (0..children.len())
        .map(|index| layouts[&id(index as u64 + 2)])
        .collect()
}

/// Items join a wrapping line by the size they would have before growing
/// (CSS Flexbox §9.3): a growing item shares its line with the siblings that
/// fit beside it, then takes what is left of that line.
#[test]
fn a_growing_item_in_a_wrapping_row_shares_its_line_with_siblings_that_fit() {
    let grow = |width: f32| LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        flex_grow: Some(1.0),
        ..LayoutStyle::default()
    };
    let fixed = LayoutStyle {
        width: Some(LengthSpec::Px(80.0)),
        ..LayoutStyle::default()
    };
    let boxes = wrapping_row(300.0, &[grow(100.0), fixed.clone()]);
    assert_eq!((boxes[0].y, boxes[1].y), (0.0, 0.0));
    assert_eq!(boxes[0].width, 300.0 - 8.0 - 80.0);
    assert_eq!(boxes[1].x, 300.0 - 80.0);

    // Two columns side by side while each gets its basis, stacked below it.
    let column = LayoutStyle {
        flex_basis: Some(LengthSpec::Px(140.0)),
        flex_grow: Some(1.0),
        flex_shrink: Some(1.0),
        ..LayoutStyle::default()
    };
    let wide = wrapping_row(300.0, &[column.clone(), column.clone()]);
    assert_eq!((wide[0].y, wide[1].y), (0.0, 0.0));
    assert_eq!((wide[0].width, wide[1].width), (146.0, 146.0));
    let narrow = wrapping_row(250.0, &[column.clone(), column]);
    assert_eq!((narrow[0].y, narrow[1].y), (0.0, 48.0));
    assert_eq!((narrow[0].width, narrow[1].width), (250.0, 250.0));

    // `flex: 1` with a floor: the floor, not the zero basis, decides the line.
    let floored = LayoutStyle {
        flex_basis: Some(LengthSpec::Px(0.0)),
        flex_grow: Some(1.0),
        min_width: Some(LengthSpec::Px(180.0)),
        ..LayoutStyle::default()
    };
    let floors = wrapping_row(300.0, &[floored.clone(), floored.clone()]);
    assert_eq!((floors[0].y, floors[1].y), (0.0, 48.0));
    let floors = wrapping_row(400.0, &[floored.clone(), floored]);
    assert_eq!((floors[0].y, floors[1].y), (0.0, 0.0));

    // A growing item with nothing to size it by is its content's size.
    let auto = wrapping_row(
        300.0,
        &[
            LayoutStyle {
                flex_grow: Some(1.0),
                ..LayoutStyle::default()
            },
            fixed,
        ],
    );
    assert_eq!((auto[0].y, auto[1].y), (0.0, 0.0));
}

/// `min-width: max-content` keeps a growing item at least as wide as its
/// content: the line shrinks its other items first.
#[test]
fn a_max_content_minimum_keeps_a_growing_item_at_its_content() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    let style = |layout: LayoutStyle| NodeStyle {
        layout: Arc::new(layout),
        ..NodeStyle::default()
    };
    for (child, parent, layout) in [
        (
            2,
            1,
            LayoutStyle {
                direction: Some(FlexDirection::Row),
                flex_grow: Some(1.0),
                flex_shrink: Some(1.0),
                min_width: Some(LengthSpec::MaxContent),
                ..LayoutStyle::default()
            },
        ),
        (
            3,
            2,
            LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(20.0)),
                flex_shrink: Some(0.0),
                ..LayoutStyle::default()
            },
        ),
        (
            4,
            1,
            LayoutStyle {
                width: Some(LengthSpec::Px(150.0)),
                height: Some(LengthSpec::Px(20.0)),
                flex_shrink: Some(1.0),
                ..LayoutStyle::default()
            },
        ),
    ] {
        queue.create(id(child), document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(parent), id(child), None);
        queue.set_style(id(child), style(layout));
    }
    queue.set_style(
        id(1),
        style(LayoutStyle {
            width: Some(LengthSpec::Px(300.0)),
            direction: Some(FlexDirection::Row),
            ..LayoutStyle::default()
        }),
    );
    world.commit(queue).unwrap();
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(300.0, 100.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(layouts[&id(2)].width, 200.0);
    assert_eq!(layouts[&id(4)].width, 100.0);
}

#[test]
fn a_wrapping_row_is_measured_in_the_order_it_is_placed() {
    // B 按 `order` 排在最前、独占一行,A 与 C 并排在第二行:两行高。按文档顺序
    // 量会折成 A / B / C 三行,容器高出一整行。
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    for (value, full) in [(2, false), (3, true), (4, false)] {
        queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(1), id(value), None);
        queue.set_style(
            id(value),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(80.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    order: if full { -1 } else { 0 },
                    flex_basis: full.then_some(LengthSpec::Percent(100.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                direction: Some(FlexDirection::Row),
                flex_wrap: FlexWrap::Wrap,
                gap: Some(LengthSpec::Px(8.0)),
                align_items: nana_ui_core::AlignSpec::Start,
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(200.0, 400.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(layouts[&id(3)].y, 0.0);
    assert_eq!(layouts[&id(2)].y, 48.0);
    assert_eq!(layouts[&id(4)].y, 48.0);
    assert_eq!(layouts[&id(1)].height, 88.0);
}

#[test]
fn grid_template_columns_split_free_space() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Element { tag: "div".into() });
    queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
    queue.create(id(3), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(1), id(2), None);
    queue.insert(id(1), id(3), None);
    let mut root_layout = LayoutStyle {
        display: Some(DisplaySpec::Grid),
        direction: Some(FlexDirection::Row),
        width: Some(LengthSpec::Px(800.0)),
        height: Some(LengthSpec::Px(400.0)),
        ..LayoutStyle::default()
    };
    root_layout.grid.columns = Some(vec![GridTrack::Px(220.0), GridTrack::Fr(1.0)]);
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(root_layout),
            ..NodeStyle::default()
        },
    );
    for value in [2, 3] {
        queue.set_style(
            id(value),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    height: Some(LengthSpec::Px(400.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    world.commit(queue).unwrap();
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(800.0, 400.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(layouts[&id(2)].width, 220.0);
    assert_eq!(layouts[&id(3)].x, 220.0);
    assert_eq!(layouts[&id(3)].width, 580.0);
}

#[test]
fn style_tree_matches_document_layout_for_row_gap() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(400.0)),
            height: Some(LengthSpec::Px(80.0)),
            gap: Some(LengthSpec::Px(12.0)),
            align_items: nana_ui_core::AlignSpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![
            StyleLayoutNode {
                id: "a".into(),
                style: LayoutStyle {
                    width: Some(LengthSpec::Px(50.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            },
            StyleLayoutNode {
                id: "b".into(),
                style: LayoutStyle {
                    width: Some(LengthSpec::Px(50.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            },
        ],
        text: None,
    };
    let boxes = RuntimeLayoutEngine
        .layout_style_tree(
            &tree,
            LayoutViewport::new(400.0, 80.0),
            &mut crate::MeasureTextShaper,
        )
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert!((boxes["b"].x - 62.0).abs() < 0.01);
}

#[test]
fn child_em_width_uses_parent_computed_font_size() {
    let tree = StyleLayoutNode {
        id: "parent".into(),
        style: LayoutStyle {
            font_size: Some(32.0),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(80.0)),
            direction: Some(FlexDirection::Row),
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "child".into(),
            style: LayoutStyle {
                width: Some(LengthSpec::Em(2.0)),
                height: Some(LengthSpec::Px(40.0)),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        }],
        text: None,
    };
    let boxes = RuntimeLayoutEngine
        .layout_style_tree(
            &tree,
            LayoutViewport::new(200.0, 80.0),
            &mut crate::MeasureTextShaper,
        )
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(
        boxes["child"].width, 64.0,
        "2em against parent font-size 32px must be 64px, not 32px"
    );
}

#[test]
fn child_em_padding_uses_parent_computed_font_size() {
    let tree = StyleLayoutNode {
        id: "parent".into(),
        style: LayoutStyle {
            // A flex column keeps the child at its own width.
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Column),
            font_size: Some(32.0),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(200.0)),
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "child".into(),
            style: LayoutStyle {
                padding: Some(LengthSpec::Em(1.0)),
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "inner".into(),
                style: LayoutStyle {
                    width: Some(LengthSpec::Px(10.0)),
                    height: Some(LengthSpec::Px(10.0)),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            }],
            text: None,
        }],
        text: None,
    };
    let boxes = RuntimeLayoutEngine
        .layout_style_tree(
            &tree,
            LayoutViewport::new(200.0, 200.0),
            &mut crate::MeasureTextShaper,
        )
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(
        boxes["inner"].x, 32.0,
        "1em padding against inherited 32px font-size must inset content 32px, not 16px"
    );
    assert_eq!(boxes["inner"].y, 32.0);
    assert_eq!(
        boxes["child"].width, 74.0,
        "1em padding on both sides must add 64px to the 10px content box"
    );
    assert_eq!(boxes["child"].height, 74.0);
}

#[test]
fn child_em_absolute_inset_uses_parent_computed_font_size() {
    let tree = StyleLayoutNode {
        id: "parent".into(),
        style: LayoutStyle {
            font_size: Some(32.0),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(200.0)),
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "child".into(),
            style: LayoutStyle {
                position: PositionSpec::Absolute,
                offset_top: Some(LengthSpec::Em(1.0)),
                offset_left: Some(LengthSpec::Em(1.0)),
                width: Some(LengthSpec::Px(40.0)),
                height: Some(LengthSpec::Px(40.0)),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        }],
        text: None,
    };
    let boxes = RuntimeLayoutEngine
        .layout_style_tree(
            &tree,
            LayoutViewport::new(200.0, 200.0),
            &mut crate::MeasureTextShaper,
        )
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(
        boxes["child"].x, 32.0,
        "1em left against inherited 32px font-size must place at 32px, not 16px"
    );
    assert_eq!(
        boxes["child"].y, 32.0,
        "1em top against inherited 32px font-size must place at 32px, not 16px"
    );
}

#[test]
fn child_em_min_height_uses_parent_computed_font_size() {
    let tree = StyleLayoutNode {
        id: "parent".into(),
        style: LayoutStyle {
            font_size: Some(32.0),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(200.0)),
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "child".into(),
            style: LayoutStyle {
                min_height: Some(LengthSpec::Em(2.0)),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        }],
        text: None,
    };
    let boxes = RuntimeLayoutEngine
        .layout_style_tree(
            &tree,
            LayoutViewport::new(200.0, 200.0),
            &mut crate::MeasureTextShaper,
        )
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(
        boxes["child"].height, 64.0,
        "2em min-height against parent font-size 32px must be 64px, not 32px"
    );
}

fn box_map(root: &StyleLayoutNode, vw: f32, vh: f32) -> HashMap<String, LayoutBox> {
    RuntimeLayoutEngine
        .layout_style_tree(
            root,
            LayoutViewport::new(vw, vh),
            &mut crate::MeasureTextShaper,
        )
        .into_iter()
        .collect()
}

fn px_box(id: &str, width: f32, height: f32) -> StyleLayoutNode {
    StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    }
}

#[test]
fn align_content_center_and_space_between_on_wrapped_row() {
    let children = (0..4)
        .map(|i| px_box(&format!("i{i}"), 80.0, 40.0))
        .collect::<Vec<_>>();
    let make = |align_content| StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            direction: Some(FlexDirection::Row),
            flex_wrap: FlexWrap::Wrap,
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(160.0)),
            gap: Some(LengthSpec::Px(8.0)),
            align_items: AlignSpec::Start,
            align_content,
            ..LayoutStyle::default()
        },
        children: children.clone(),
        text: None,
    };
    let center = box_map(&make(JustifySpec::Center), 200.0, 160.0);
    assert!((center["i0"].y - 36.0).abs() < 0.01);
    assert!((center["i1"].y - 36.0).abs() < 0.01);
    assert!((center["i2"].y - 84.0).abs() < 0.01);
    assert!((center["i3"].y - 84.0).abs() < 0.01);
    let between = box_map(&make(JustifySpec::SpaceBetween), 200.0, 160.0);
    assert!((between["i0"].y - 0.0).abs() < 0.01);
    assert!((between["i2"].y - 120.0).abs() < 0.01);
}

/// A row item is as tall as its content at the width the row gives it, not
/// at the width it was first measured with: a `Fill` column beside a fixed
/// side column holds a 16:9 box, whose height follows the column's used
/// width (CSS flexbox: the hypothetical cross size uses the used main size).
#[test]
fn a_row_item_is_as_tall_as_its_content_at_its_used_width() {
    let column = |grow: f32| StyleLayoutNode {
        id: "primary".into(),
        style: LayoutStyle {
            direction: Some(FlexDirection::Column),
            width: Some(LengthSpec::Fill),
            height: Some(LengthSpec::Shrink),
            min_width: Some(LengthSpec::Px(0.0)),
            flex_grow: Some(grow),
            flex_shrink: Some(1.0),
            align_items: AlignSpec::Stretch,
            ..LayoutStyle::default()
        },
        children: vec![
            StyleLayoutNode {
                id: "video".into(),
                style: LayoutStyle {
                    width: Some(LengthSpec::Fill),
                    aspect_ratio: Some(16.0 / 9.0),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            },
            px_box("title", 100.0, 20.0),
        ],
        text: None,
    };
    for grow in [1.0, 0.0] {
        let tree = StyleLayoutNode {
            id: "row".into(),
            style: LayoutStyle {
                direction: Some(FlexDirection::Row),
                width: Some(LengthSpec::Px(1152.0)),
                gap: Some(LengthSpec::Px(16.0)),
                align_items: AlignSpec::Start,
                ..LayoutStyle::default()
            },
            children: vec![column(grow), {
                let mut side = px_box("side", 402.0, 260.0);
                side.style.flex_shrink = Some(0.0);
                side
            }],
            text: None,
        };
        let boxes = box_map(&tree, 1280.0, 800.0);
        let used = 1152.0 - 16.0 - 402.0;
        assert!(
            (boxes["primary"].width - used).abs() < 0.01,
            "{:?}",
            boxes["primary"]
        );
        let video = used * 9.0 / 16.0;
        assert!(
            (boxes["video"].height - video).abs() < 0.01,
            "{:?}",
            boxes["video"]
        );
        assert!(
            (boxes["primary"].height - (video + 20.0)).abs() < 0.01,
            "grow {grow}: the column hugs its content at its used width, got {:?}",
            boxes["primary"]
        );
        assert!(
            (boxes["row"].height - (video + 20.0)).abs() < 0.01,
            "grow {grow}: and so does the row, got {:?}",
            boxes["row"]
        );
    }
}

/// `height: auto` stretches in a row that stretches its items, as an
/// unset height does; a content-sized keyword keeps the content's height.
#[test]
fn an_auto_cross_size_stretches_like_an_unset_one() {
    let item = |id: &str, height| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(40.0)),
            height,
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "row".into(),
        style: LayoutStyle {
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(80.0)),
            align_items: AlignSpec::Stretch,
            ..LayoutStyle::default()
        },
        children: vec![
            item("unset", None),
            item("auto", Some(LengthSpec::Auto)),
            item("shrink", Some(LengthSpec::Shrink)),
        ],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 80.0);
    assert_eq!(boxes["unset"].height, 80.0);
    assert_eq!(boxes["auto"].height, 80.0);
    assert_eq!(boxes["shrink"].height, 0.0);
}

#[test]
fn grid_2d_auto_flow_wraps_fourth_item_to_second_row() {
    let mut style = LayoutStyle {
        display: Some(DisplaySpec::Grid),
        width: Some(LengthSpec::Px(100.0)),
        height: Some(LengthSpec::Px(100.0)),
        ..LayoutStyle::default()
    };
    style.grid.columns = Some(vec![GridTrack::Px(50.0), GridTrack::Px(50.0)]);
    let tree = StyleLayoutNode {
        id: "root".into(),
        style,
        children: (0..4)
            .map(|i| px_box(&format!("i{i}"), 50.0, 50.0))
            .collect(),
        text: None,
    };
    let boxes = box_map(&tree, 100.0, 100.0);
    assert_eq!(boxes["i0"].x, 0.0);
    assert_eq!(boxes["i0"].y, 0.0);
    assert_eq!(boxes["i1"].x, 50.0);
    assert_eq!(boxes["i1"].y, 0.0);
    assert_eq!(boxes["i2"].x, 0.0);
    assert_eq!(boxes["i2"].y, 50.0);
    assert_eq!(boxes["i3"].x, 50.0);
    assert_eq!(boxes["i3"].y, 50.0);
}

#[test]
fn grid_column_span_two_on_three_columns() {
    let mut first_style = LayoutStyle {
        height: Some(LengthSpec::Px(50.0)),
        ..LayoutStyle::default()
    };
    first_style.grid.placement = GridPlacement {
        column_start: GridLine::Span(2),
        ..GridPlacement::default()
    }
    .into();
    let first = StyleLayoutNode {
        id: "a".into(),
        style: first_style,
        children: Vec::new(),
        text: None,
    };
    let mut root_style = LayoutStyle {
        display: Some(DisplaySpec::Grid),
        width: Some(LengthSpec::Px(150.0)),
        height: Some(LengthSpec::Px(100.0)),
        ..LayoutStyle::default()
    };
    root_style.grid.columns = Some(vec![
        GridTrack::Px(50.0),
        GridTrack::Px(50.0),
        GridTrack::Px(50.0),
    ]);
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: root_style,
        children: vec![first, px_box("b", 50.0, 50.0), px_box("c", 50.0, 50.0)],
        text: None,
    };
    let boxes = box_map(&tree, 150.0, 100.0);
    assert!((boxes["a"].x - 0.0).abs() < 0.01);
    assert!((boxes["a"].width - 100.0).abs() < 0.01);
    assert!((boxes["b"].x - 100.0).abs() < 0.01);
    assert!((boxes["b"].y - 0.0).abs() < 0.01);
    assert!((boxes["c"].x - 0.0).abs() < 0.01);
    assert!((boxes["c"].y - 50.0).abs() < 0.01);
}

#[test]
fn grid_justify_self_end_in_definite_column() {
    let mut item = px_box("item", 50.0, 50.0);
    item.style.justify_self = Some(AlignSpec::End);
    let mut style = LayoutStyle {
        display: Some(DisplaySpec::Grid),
        width: Some(LengthSpec::Px(200.0)),
        height: Some(LengthSpec::Px(50.0)),
        ..LayoutStyle::default()
    };
    style.grid.columns = Some(vec![GridTrack::Px(200.0)]);
    let tree = StyleLayoutNode {
        id: "root".into(),
        style,
        children: vec![item],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 50.0);
    assert!((boxes["item"].x - 150.0).abs() < 0.01);
    assert_eq!(boxes["item"].width, 50.0);
}

#[test]
fn grid_auto_fit_fills_two_minmax_tracks_in_500px() {
    let mut style = LayoutStyle {
        display: Some(DisplaySpec::Grid),
        width: Some(LengthSpec::Px(500.0)),
        height: Some(LengthSpec::Px(50.0)),
        ..LayoutStyle::default()
    };
    style.grid.columns_repeat = Some(
        GridRepeatAuto {
            kind: GridTrackListUnsupported::RepeatAutoFit,
            tracks: vec![GridTrack::MinMax {
                min_px: 200.0,
                fr: 1.0,
                max_px: None,
            }],
            ..Default::default()
        }
        .into(),
    );
    let tree = StyleLayoutNode {
        id: "root".into(),
        style,
        children: vec![px_box("a", 50.0, 50.0), px_box("b", 50.0, 50.0)],
        text: None,
    };
    let boxes = box_map(&tree, 500.0, 50.0);
    assert!(
        (boxes["b"].x - 250.0).abs() < 0.5,
        "auto-fit minmax(200px,1fr) in 500px must keep 2 tracks, got b.x={}",
        boxes["b"].x
    );
    assert!((boxes["a"].x - 0.0).abs() < 0.5);
}

/// A wrapping row of chips sized at no width: one chip per line.
fn wrapping_chips(id: &str, width: Option<LengthSpec>) -> StyleLayoutNode {
    StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            direction: Some(FlexDirection::Row),
            flex_wrap: FlexWrap::Wrap,
            gap: Some(LengthSpec::Px(6.0)),
            width,
            ..LayoutStyle::default()
        },
        children: (0..5)
            .map(|index| px_box(&format!("{id}{index}"), 60.0, 20.0))
            .collect(),
        text: None,
    }
}

/// An auto row holds its items laid out at the width their columns give
/// them (CSS Grid §11.5), measured in Chromium as fixture T-G36: two 197px
/// columns, five 60px chips wrapping three and two, a 46px row. Measured
/// before the columns were sized, a `width: 100%` row of chips stood one
/// chip per line (124px) and an auto-width one all on one line.
#[test]
fn grid_auto_row_holds_wrapping_content_at_its_column_width() {
    let grid = |children| {
        let mut style = LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(400.0)),
            gap: Some(LengthSpec::Px(6.0)),
            ..LayoutStyle::default()
        };
        style.grid.columns_repeat = Some(
            GridRepeatAuto {
                kind: GridTrackListUnsupported::RepeatAutoFit,
                tracks: vec![GridTrack::MinMax {
                    min_px: 132.0,
                    fr: 1.0,
                    max_px: None,
                }],
                ..Default::default()
            }
            .into(),
        );
        StyleLayoutNode {
            id: "root".into(),
            style,
            children,
            text: None,
        }
    };
    for width in [
        Some(LengthSpec::Percent(100.0)),
        Some(LengthSpec::Fill),
        None,
    ] {
        let boxes = box_map(
            &grid(vec![
                wrapping_chips("a", width),
                px_box("b", 50.0, 20.0),
                px_box("c", 50.0, 20.0),
            ]),
            800.0,
            300.0,
        );
        assert_eq!(boxes["a"].height, 46.0, "{width:?}: {:?}", boxes["a"]);
        assert_eq!(boxes["a4"].y, 26.0, "{width:?}: wraps three and two");
        assert_eq!(boxes["c"].y, 52.0, "{width:?}: {:?}", boxes["c"]);
        assert_eq!(boxes["root"].height, 72.0, "{width:?}");
    }
}

/// The retained passes follow the columns: a grid that widens lays its row
/// out again at the new column width, where the chips fit on one line.
#[test]
fn a_grid_auto_row_follows_its_columns_when_the_grid_resizes() {
    use crate::{AppContext, Stack};
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let page = cx.create_component(document, Stack::column(0.0)).unwrap();
    let grid = cx
        .create_component(
            document,
            Stack::column(0.0).with_layout(|layout| {
                layout.display = Some(DisplaySpec::Grid);
                layout.width = Some(LengthSpec::Px(400.0));
                layout.gap = Some(LengthSpec::Px(6.0));
                layout.grid.columns = Some(vec![GridTrack::Fr(1.0), GridTrack::Fr(1.0)]);
            }),
        )
        .unwrap();
    cx.append_child(page, grid).unwrap();
    let chips = cx
        .create_component(
            document,
            Stack::row(6.0).with_layout(|layout| {
                layout.flex_wrap = FlexWrap::Wrap;
                layout.width = Some(LengthSpec::Fill);
            }),
        )
        .unwrap();
    cx.append_child(grid, chips).unwrap();
    for _ in 0..5 {
        let chip = cx
            .create_component(
                document,
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(60.0));
                    layout.height = Some(LengthSpec::Px(20.0));
                }),
            )
            .unwrap();
        cx.append_child(chips, chip).unwrap();
    }
    let viewport = LayoutViewport::new(800.0, 300.0);
    cx.layout_document(document, viewport).unwrap();
    // 197px columns: three chips and two.
    assert_eq!(
        cx.world().layout_box(chips.stable_id()).unwrap().height,
        46.0
    );
    cx.update_component(grid, |grid, _| {
        Arc::make_mut(&mut grid.style_mut().layout).width = Some(LengthSpec::Px(700.0));
    })
    .unwrap();
    cx.layout_document(document, viewport).unwrap();
    // 347px columns: all five on one line.
    assert_eq!(
        cx.world().layout_box(chips.stable_id()).unwrap().height,
        20.0
    );
    assert_eq!(
        cx.world().layout_box(grid.stable_id()).unwrap().height,
        20.0
    );
}

#[test]
fn white_space_pre_measures_explicit_newlines() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: None,
            width: Some(LengthSpec::Px(200.0)),
            font_size: Some(16.0),
            line_height: Some(LineHeightSpec::Absolute(20.0)),
            white_space: WhiteSpaceSpec::Pre,
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: Some("ab\ncd".into()),
    };
    let boxes = box_map(&tree, 200.0, 80.0);
    assert!(
        (boxes["root"].height - 40.0).abs() < 0.01,
        "pre + 2 lines × 20px line-height must be 40, got {}",
        boxes["root"].height
    );
}

#[test]
fn white_space_pre_wrap_keeps_newlines_and_wraps_long_lines() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: None,
            width: Some(LengthSpec::Px(200.0)),
            font_size: Some(16.0),
            line_height: Some(LineHeightSpec::Absolute(20.0)),
            white_space: WhiteSpaceSpec::PreWrap,
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: Some("ab\ncd".into()),
    };
    let boxes = box_map(&tree, 200.0, 120.0);
    assert!(
        (boxes["root"].height - 40.0).abs() < 0.01,
        "pre-wrap must keep explicit newlines (not Normal), got {}",
        boxes["root"].height
    );
}

#[test]
fn measure_text_pre_wrap_wraps_long_line_against_max_width() {
    let mut shaper = crate::MeasureTextShaper;
    let style = ComputedStyle::with(|s| {
        s.font_size = 16.0;
        s.line_height = Some(LineHeightSpec::Absolute(20.0));
    });
    let metrics = shaper.shape(
        StableNodeId::new(1).unwrap(),
        &crate::TextContent {
            value: "abcdefghijklmnop\nq".into(),
        },
        &style,
        crate::TextShapeConstraints {
            max_width: Some(200.0),
            wrap: true,
            preserve_lines: true,
            ..crate::TextShapeConstraints::default()
        },
    );
    assert!(
        (metrics.height - 60.0).abs() < 0.01,
        "16em line in 200px + explicit second line → 60, got {}",
        metrics.height
    );
}

#[test]
fn aspect_ratio_square_from_definite_width() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(80.0)),
            aspect_ratio: Some(1.0),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let boxes = box_map(&tree, 400.0, 200.0);
    assert!(
        (boxes["root"].width - 80.0).abs() < 0.01 && (boxes["root"].height - 80.0).abs() < 0.01,
        "80px width + aspect-ratio 1 must be square, got {:?}",
        boxes["root"]
    );
}

#[test]
fn aspect_ratio_auto_width_uses_containing_block() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(400.0)),
            height: Some(LengthSpec::Px(200.0)),
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "block".into(),
            style: LayoutStyle {
                height: Some(LengthSpec::Px(80.0)),
                aspect_ratio: Some(1.0),
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "pct".into(),
                style: LayoutStyle {
                    width: Some(LengthSpec::Percent(50.0)),
                    height: Some(LengthSpec::Px(10.0)),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            }],
            text: None,
        }],
        text: None,
    };
    let boxes = box_map(&tree, 400.0, 200.0);
    assert!(
        (boxes["block"].width - 400.0).abs() < 0.01 && (boxes["block"].height - 80.0).abs() < 0.01,
        "block width:auto + height 80 + aspect-ratio 1 uses CB, not 80×80, got {:?}",
        boxes["block"]
    );
    assert!(
        (boxes["pct"].width - 200.0).abs() < 0.01,
        "% children resolve against the CB, not a shrink-wrapped 80, got {:?}",
        boxes["pct"]
    );
}

#[test]
fn aspect_ratio_row_stretch_does_not_overwrite_transferred_height() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(200.0)),
            align_items: AlignSpec::Stretch,
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "item".into(),
            style: LayoutStyle {
                width: Some(LengthSpec::Px(80.0)),
                aspect_ratio: Some(1.0),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        }],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 200.0);
    assert!(
        (boxes["item"].width - 80.0).abs() < 0.01 && (boxes["item"].height - 80.0).abs() < 0.01,
        "row stretch must not overwrite height transferred from width + ratio, got {:?}",
        boxes["item"]
    );
}

#[test]
fn aspect_ratio_column_stretch_fills_auto_height() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Column),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(200.0)),
            align_items: AlignSpec::Stretch,
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "item".into(),
            style: LayoutStyle {
                aspect_ratio: Some(1.0),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        }],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 200.0);
    assert!(
        (boxes["item"].width - 200.0).abs() < 0.01 && (boxes["item"].height - 200.0).abs() < 0.01,
        "column stretch width then ratio must fill auto height, got {:?}",
        boxes["item"]
    );
}

#[test]
fn grid_percent_and_fill_resolve_against_final_cell() {
    let fill = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Percent(100.0)),
            height: Some(LengthSpec::Fill),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let mut style = LayoutStyle {
        display: Some(DisplaySpec::Grid),
        width: Some(LengthSpec::Px(300.0)),
        height: Some(LengthSpec::Px(40.0)),
        ..LayoutStyle::default()
    };
    style.grid.columns = Some(vec![GridTrack::Px(100.0), GridTrack::Fr(1.0)]);
    let tree = StyleLayoutNode {
        id: "root".into(),
        style,
        children: vec![fill("a"), fill("b")],
        text: None,
    };
    let boxes = box_map(&tree, 300.0, 40.0);
    assert!(
        (boxes["a"].width - 100.0).abs() < 0.5 && (boxes["a"].height - 40.0).abs() < 0.5,
        "100%/Fill must fill the 100px track, not stay 0, got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["b"].width - 200.0).abs() < 0.5 && (boxes["b"].height - 40.0).abs() < 0.5,
        "100%/Fill must fill the 1fr cell, got {:?}",
        boxes["b"]
    );
}

#[test]
fn demote_fill_spec_treats_full_percent_calc_as_indefinite() {
    let calc_100 = LengthSpec::from_calc(CalcExpr::Min(
        Box::new(CalcExpr::Percent(100.0)),
        Box::new(CalcExpr::Percent(100.0)),
    ));
    assert!(calc_100.is_full_percent_fill());
    assert_eq!(demote_fill_spec(Some(calc_100)), None);
    assert_eq!(demote_fill_spec(Some(LengthSpec::Fill)), None);
    assert_eq!(
        demote_fill_spec(Some(LengthSpec::Px(40.0))),
        Some(LengthSpec::Px(40.0))
    );
}

#[test]
fn grid_cell_resolves_unsimplified_calc_against_cell() {
    let spec = LengthSpec::from_calc(CalcExpr::Binary {
        op: CalcBinOp::Add,
        left: Box::new(CalcExpr::Min(
            Box::new(CalcExpr::Px(100.0)),
            Box::new(CalcExpr::Percent(80.0)),
        )),
        right: Box::new(CalcExpr::Px(10.0)),
    });
    assert!(
        matches!(spec, LengthSpec::Calc(_)),
        "min() + px must stay as Calc AST"
    );

    let child = StyleLayoutNode {
        id: "child".into(),
        style: LayoutStyle {
            width: Some(spec),
            height: Some(LengthSpec::Px(30.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let mut root_style = LayoutStyle {
        display: Some(DisplaySpec::Grid),
        width: Some(LengthSpec::Px(400.0)),
        height: Some(LengthSpec::Px(40.0)),
        align_items: AlignSpec::Start,
        justify_items: Some(AlignSpec::Start),
        ..LayoutStyle::default()
    };
    root_style.grid.columns = Some(vec![GridTrack::Px(400.0)]);
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: root_style,
        children: vec![child],
        text: None,
    };
    let boxes = box_map(&tree, 400.0, 40.0);
    assert!(
        (boxes["child"].width - 110.0).abs() < 0.5,
        "placed grid item must be 110px, got {:?}",
        boxes["child"]
    );
}

#[test]
fn empty_grid_item_stretches_into_track() {
    let empty = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle::default(),
        children: Vec::new(),
        text: None,
    };
    let mut root_style = LayoutStyle {
        display: Some(DisplaySpec::Grid),
        width: Some(LengthSpec::Px(300.0)),
        height: Some(LengthSpec::Px(40.0)),
        // CSS `display:grid` initial align-items is stretch (css_map sets this).
        align_items: AlignSpec::Stretch,
        ..LayoutStyle::default()
    };
    root_style.grid.columns = Some(vec![GridTrack::Px(100.0), GridTrack::Fr(1.0)]);
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: root_style,
        children: vec![empty("a"), empty("b")],
        text: None,
    };
    let boxes = box_map(&tree, 300.0, 40.0);
    assert!(
        (boxes["a"].width - 100.0).abs() < 0.5 && (boxes["a"].height - 40.0).abs() < 0.5,
        "empty + stretch must fill the track, got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["b"].width - 200.0).abs() < 0.5 && (boxes["b"].height - 40.0).abs() < 0.5,
        "empty + stretch 1fr, got {:?}",
        boxes["b"]
    );
}

/// The pitch of a line of 20px-high empty inline-flex items in an unset container of the
/// default font (16px, `line-height` 1.2): they sit on the baseline by their
/// bottom edge, and the container's strut adds its descent below them, the
/// font's 0.2em and half of the 0.2em leading (CSS 2.1 §10.8.1).
const STRUT_LINE: f32 = 20.0 + 16.0 * (1.0 - nana_ui_core::TEXT_APPROX_ASCENT_EM) + 16.0 * 0.1;

#[test]
fn ifc_block_sibling_starts_new_line() {
    let inline = |id: &str, x: f32| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineFlex),
            width: Some(LengthSpec::Px(x)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let block = StyleLayoutNode {
        id: "mid".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: None,
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![inline("a", 40.0), block, inline("c", 40.0)],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 80.0);
    assert!((boxes["a"].y - 0.0).abs() < 0.5);
    assert!(
        (boxes["mid"].y - STRUT_LINE).abs() < 0.5,
        "block sibling must break the IFC line, got {:?}",
        boxes["mid"]
    );
    assert!(
        (boxes["c"].y - STRUT_LINE - 20.0).abs() < 0.5,
        "inline after block starts a new line, got {:?}",
        boxes["c"]
    );
}

#[test]
fn ifc_block_in_inline_unboxes_like_block_siblings() {
    let inline_block = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineFlex),
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let mid = StyleLayoutNode {
        id: "mid".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: None,
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "span".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Inline),
                ..LayoutStyle::default()
            },
            children: vec![inline_block("a"), mid, inline_block("c")],
            text: None,
        }],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 80.0);
    assert!(
        (boxes["a"].y - 0.0).abs() < 0.5,
        "first inline-flex stays on the first line, got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["mid"].y - STRUT_LINE).abs() < 0.5,
        "block inside inline must hoist onto its own line, got {:?}",
        boxes["mid"]
    );
    assert!(
        (boxes["c"].y - STRUT_LINE - 20.0).abs() < 0.5,
        "trailing inline-flex starts after the hoisted block, got {:?}",
        boxes["c"]
    );
}

#[test]
fn flex_item_inline_with_block_is_not_unboxed() {
    let mid = StyleLayoutNode {
        id: "mid".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            width: Some(LengthSpec::Px(50.0)),
            height: Some(LengthSpec::Px(40.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(40.0)),
            gap: Some(LengthSpec::Px(10.0)),
            align_items: AlignSpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![
            StyleLayoutNode {
                id: "span".into(),
                style: LayoutStyle {
                    display: Some(DisplaySpec::Inline),
                    ..LayoutStyle::default()
                },
                children: vec![mid],
                text: None,
            },
            px_box("b", 50.0, 40.0),
        ],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 40.0);
    assert!(
        boxes.contains_key("span"),
        "inline flex item must stay one flex item, got {:?}",
        boxes.keys().collect::<Vec<_>>()
    );
    assert!(
        (boxes["span"].x - 0.0).abs() < 0.5 && (boxes["span"].width - 50.0).abs() < 0.5,
        "blockified inline item keeps its block child, got {:?}",
        boxes["span"]
    );
    assert!(
        (boxes["mid"].x - boxes["span"].x).abs() < 0.5,
        "block descendant stays inside the flex item, got mid={:?} span={:?}",
        boxes["mid"],
        boxes["span"]
    );
    assert!(
        (boxes["b"].x - 60.0).abs() < 0.5,
        "second flex item follows the inline item, not a hoisted block, got {:?}",
        boxes["b"]
    );
}

#[test]
fn ifc_text_align_start_packs_to_right_in_rtl() {
    let inline = StyleLayoutNode {
        id: "a".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineFlex),
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: None,
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(40.0)),
            dir: Some(DirSpec::Rtl),
            text_align: nana_ui_core::TextAlignSpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![inline],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 40.0);
    assert!(
        (boxes["a"].x - 160.0).abs() < 0.5,
        "text-align:start in rtl must pack to inline-start (right), got {:?}",
        boxes["a"]
    );
}

#[test]
fn ifc_rtl_places_first_tree_item_at_inline_start() {
    let inline = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineFlex),
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: None,
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(40.0)),
            dir: Some(DirSpec::Rtl),
            ..LayoutStyle::default()
        },
        children: vec![inline("a"), inline("c")],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 40.0);
    assert!(
        (boxes["a"].x - 160.0).abs() < 0.5,
        "first tree-order item sits at RTL inline-start (right), got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["c"].x - 120.0).abs() < 0.5,
        "second tree-order item sits to the left of the first, got {:?}",
        boxes["c"]
    );
}

#[test]
fn writing_mode_vertical_rl_ifc_advances_inline_down_block_from_right() {
    let inline = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineFlex),
            width: Some(LengthSpec::Px(20.0)),
            height: Some(LengthSpec::Px(40.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: None,
            width: Some(LengthSpec::Px(80.0)),
            height: Some(LengthSpec::Px(80.0)),
            writing_mode: Some(WritingModeSpec::VerticalRl),
            ..LayoutStyle::default()
        },
        children: vec![inline("a"), inline("b")],
        text: None,
    };
    let boxes = box_map(&tree, 80.0, 80.0);
    assert!(
        (boxes["a"].x - 60.0).abs() < 0.5 && (boxes["a"].y - 0.0).abs() < 0.5,
        "vertical-rl first inline sits at block-start (right) and inline-start (top), got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["b"].x - 60.0).abs() < 0.5 && (boxes["b"].y - 40.0).abs() < 0.5,
        "second inline advances down the inline axis, got {:?}",
        boxes["b"]
    );
}

#[test]
fn writing_mode_vertical_lr_ifc_places_first_line_on_the_left() {
    let inline = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineFlex),
            width: Some(LengthSpec::Px(20.0)),
            height: Some(LengthSpec::Px(40.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: None,
            width: Some(LengthSpec::Px(80.0)),
            height: Some(LengthSpec::Px(80.0)),
            writing_mode: Some(WritingModeSpec::VerticalLr),
            ..LayoutStyle::default()
        },
        children: vec![inline("a"), inline("b")],
        text: None,
    };
    let boxes = box_map(&tree, 80.0, 80.0);
    assert!(
        (boxes["a"].x - 0.0).abs() < 0.5 && (boxes["a"].y - 0.0).abs() < 0.5,
        "vertical-lr first inline sits at block-start (left), got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["b"].x - 0.0).abs() < 0.5 && (boxes["b"].y - 40.0).abs() < 0.5,
        "second inline advances down the inline axis, got {:?}",
        boxes["b"]
    );
}

#[test]
fn writing_mode_vertical_flex_row_uses_inline_axis() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(80.0)),
            height: Some(LengthSpec::Px(80.0)),
            writing_mode: Some(WritingModeSpec::VerticalRl),
            align_items: AlignSpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![px_box("a", 20.0, 40.0), px_box("b", 20.0, 40.0)],
        text: None,
    };
    let boxes = box_map(&tree, 80.0, 80.0);
    assert!(
        (boxes["a"].x - 60.0).abs() < 0.5 && (boxes["a"].y - 0.0).abs() < 0.5,
        "flex-direction:row in vertical-rl follows the inline axis from the right, got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["b"].x - 60.0).abs() < 0.5 && (boxes["b"].y - 40.0).abs() < 0.5,
        "second flex item stacks down the inline axis, got {:?}",
        boxes["b"]
    );
}

#[test]
fn rtl_row_flex_reverses_item_order_and_flips_justify() {
    // Characterisation: this already worked before RTL grid support, but no
    // test held it and `docs/reference/layout.md` claimed the opposite. Lock it down so
    // the contract is readable from the layout layer, not just the CSS mapping.
    let item = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(20.0)),
            dir: Some(DirSpec::Rtl),
            ..LayoutStyle::default()
        },
        children: vec![item("a"), item("b")],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 20.0);
    assert!(
        (boxes["a"].x - 160.0).abs() < 0.5,
        "first item sits at inline-start, which rtl puts on the right, got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["b"].x - 120.0).abs() < 0.5,
        "second item runs leftwards from the first, got {:?}",
        boxes["b"]
    );
}

#[test]
fn rtl_grid_mirrors_column_order_and_justify_self() {
    let cell = |id: &str, justify: AlignSpec| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(30.0)),
            height: Some(LengthSpec::Px(20.0)),
            justify_self: Some(justify),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let mut root_style = LayoutStyle {
        display: Some(DisplaySpec::Grid),
        width: Some(LengthSpec::Px(200.0)),
        height: Some(LengthSpec::Px(20.0)),
        dir: Some(DirSpec::Rtl),
        ..LayoutStyle::default()
    };
    root_style.grid.columns = Some(vec![GridTrack::Px(80.0), GridTrack::Px(120.0)]);
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: root_style,
        children: vec![cell("a", AlignSpec::Start), cell("b", AlignSpec::Start)],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 20.0);
    // Column 1 (80px) is the rightmost track: 200 - 80 = 120.
    // `justify-self: start` inside it means that track's right edge: 200 - 30.
    assert!(
        (boxes["a"].x - 170.0).abs() < 0.5,
        "grid column 1 is the rightmost track in rtl, and start means its right edge, got {:?}",
        boxes["a"]
    );
    // Column 2 (120px) occupies 0..120; start is its right edge: 120 - 30.
    assert!(
        (boxes["b"].x - 90.0).abs() < 0.5,
        "grid column 2 sits to the left of column 1 in rtl, got {:?}",
        boxes["b"]
    );
}

/// Grid columns are tracks along the inline axis and rows along the block one
/// (CSS Grid §3). In `vertical-rl` the columns run down the page and the rows
/// stack from the right; with `direction: rtl` the columns start at the
/// bottom.
#[test]
fn a_vertical_grid_lays_columns_down_the_inline_axis_and_rows_from_the_right() {
    let cell = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            justify_self: Some(AlignSpec::Stretch),
            align_self: Some(AlignSpec::Stretch),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let grid = |dir| {
        let mut style = LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(100.0)),
            height: Some(LengthSpec::Px(200.0)),
            writing_mode: Some(WritingModeSpec::VerticalRl),
            dir: Some(dir),
            ..LayoutStyle::default()
        };
        style.grid.columns = Some(vec![GridTrack::Px(40.0), GridTrack::Px(60.0)]);
        style.grid.rows = Some(vec![GridTrack::Px(20.0), GridTrack::Px(30.0)]);
        StyleLayoutNode {
            id: "root".into(),
            style,
            children: vec![cell("a"), cell("b"), cell("c")],
            text: None,
        }
    };
    let near = |boxes: &HashMap<String, LayoutBox>, id: &str, expected: (f32, f32, f32, f32)| {
        let got = boxes[id];
        assert!(
            (got.x - expected.0).abs() < 0.5
                && (got.y - expected.1).abs() < 0.5
                && (got.width - expected.2).abs() < 0.5
                && (got.height - expected.3).abs() < 0.5,
            "{id}: expected {expected:?}, got {got:?}"
        );
    };
    // Auto-flow fills row 1 (the rightmost 20px), column by column down the
    // page, then row 2 (the 30px to its left).
    let ltr = box_map(&grid(DirSpec::Ltr), 100.0, 200.0);
    near(&ltr, "a", (80.0, 0.0, 20.0, 40.0));
    near(&ltr, "b", (80.0, 40.0, 20.0, 60.0));
    near(&ltr, "c", (50.0, 0.0, 30.0, 40.0));
    // RTL: column 1 is at the bottom of the 200px inline axis.
    let rtl = box_map(&grid(DirSpec::Rtl), 100.0, 200.0);
    near(&rtl, "a", (80.0, 160.0, 20.0, 40.0));
    near(&rtl, "b", (80.0, 100.0, 20.0, 60.0));
    near(&rtl, "c", (50.0, 160.0, 30.0, 40.0));
}

/// Percentage margins resolve against the containing block's inline size
/// (CSS Box Model §5): its height in a vertical writing mode, not its width.
#[test]
fn percentage_margins_resolve_against_the_inline_size_in_a_vertical_mode() {
    let child = StyleLayoutNode {
        id: "a".into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(20.0)),
            height: Some(LengthSpec::Px(20.0)),
            margin_top: Some(LengthSpec::Percent(10.0)),
            writing_mode: Some(WritingModeSpec::VerticalRl),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(100.0)),
            height: Some(LengthSpec::Px(200.0)),
            writing_mode: Some(WritingModeSpec::VerticalRl),
            align_items: AlignSpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![child],
        text: None,
    };
    let boxes = box_map(&tree, 100.0, 200.0);
    assert!(
        (boxes["a"].y - 20.0).abs() < 0.5,
        "10% of the 200px inline size, not of the 100px width: {:?}",
        boxes["a"]
    );
}

/// A wrapping column packs its lines with the same margins it then places
/// with: percentage margins resolve against the containing block's inline
/// size — the width, even though a column's lines fill its height.
#[test]
fn a_wrapping_column_packs_with_percentage_margins_of_the_width() {
    let item = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(30.0)),
            height: Some(LengthSpec::Px(40.0)),
            margin_top: Some(LengthSpec::Percent(10.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Column),
            flex_wrap: FlexWrap::Wrap,
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(100.0)),
            align_items: AlignSpec::Start,
            align_content: JustifySpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![item("a"), item("b")],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 100.0);
    // 10% of the 200px width is 20px: 20 + 40 + 20 + 40 overflows 100, so
    // `b` starts the second column.
    assert!(
        (boxes["a"].y - 20.0).abs() < 0.5 && (boxes["b"].y - 20.0).abs() < 0.5,
        "both sit 20px down their column: a={:?} b={:?}",
        boxes["a"],
        boxes["b"]
    );
    assert!(
        boxes["b"].x >= boxes["a"].x + 30.0 - 0.5,
        "b wrapped to the next column: a={:?} b={:?}",
        boxes["a"],
        boxes["b"]
    );
}

/// Tracks wider than the content box overflow past the inline-start edge —
/// the left one in RTL. Clamping the mirrored x at zero piles every
/// overflowing column onto the first instead.
#[test]
fn rtl_grid_tracks_overflow_the_start_edge_instead_of_stacking() {
    let cell = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(300.0)),
            height: Some(LengthSpec::Px(20.0)),
            grid: nana_ui_core::GridStyle {
                columns: Some(vec![GridTrack::Px(200.0), GridTrack::Px(200.0)]),
                ..Default::default()
            }
            .into(),
            dir: Some(DirSpec::Rtl),
            ..LayoutStyle::default()
        },
        children: vec![cell("a"), cell("b")],
        text: None,
    };
    let boxes = box_map(&tree, 300.0, 20.0);
    // Track 1 is the rightmost: 300 - 200 = 100.
    assert!(
        (boxes["a"].x - 100.0).abs() < 0.5,
        "first track should sit at 100, got {:?}",
        boxes["a"]
    );
    // Track 2 mirrors to 300 - 200 - 200 = -100, overflowing the left edge.
    assert!(
        (boxes["b"].x + 100.0).abs() < 0.5,
        "second track should overflow to -100, got {:?}",
        boxes["b"]
    );
    assert!(
        boxes["b"].x + 200.0 <= boxes["a"].x + 0.5,
        "the two tracks must not overlap: {:?} vs {:?}",
        boxes["a"],
        boxes["b"]
    );
}

#[test]
fn rtl_grid_keeps_ltr_geometry_when_direction_is_default() {
    let cell = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(30.0)),
            height: Some(LengthSpec::Px(20.0)),
            justify_self: Some(AlignSpec::Start),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(20.0)),
            grid: nana_ui_core::GridStyle {
                columns: Some(vec![GridTrack::Px(80.0), GridTrack::Px(120.0)]),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: vec![cell("a"), cell("b")],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 20.0);
    assert!((boxes["a"].x - 0.0).abs() < 0.5, "got {:?}", boxes["a"]);
    assert!((boxes["b"].x - 80.0).abs() < 0.5, "got {:?}", boxes["b"]);
}

#[test]
fn rtl_column_flex_puts_cross_start_on_the_right() {
    let item = |id: &str, width: f32| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Column),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(60.0)),
            align_items: AlignSpec::Start,
            dir: Some(DirSpec::Rtl),
            ..LayoutStyle::default()
        },
        children: vec![item("a", 40.0), item("b", 60.0)],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 60.0);
    // Column flex: the cross axis is the inline axis, so rtl `align-items:
    // start` is the right edge. The main (block) axis is untouched.
    assert!(
        (boxes["a"].x - 160.0).abs() < 0.5 && (boxes["a"].y - 0.0).abs() < 0.5,
        "got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["b"].x - 140.0).abs() < 0.5 && (boxes["b"].y - 20.0).abs() < 0.5,
        "got {:?}",
        boxes["b"]
    );
}

#[test]
fn rtl_row_flex_cross_axis_stays_block_axis() {
    // A row container's cross axis is vertical; rtl must not touch it.
    let item = |id: &str, height: f32| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(60.0)),
            align_items: AlignSpec::Start,
            dir: Some(DirSpec::Rtl),
            ..LayoutStyle::default()
        },
        children: vec![item("a", 20.0), item("b", 30.0)],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 60.0);
    assert!((boxes["a"].y - 0.0).abs() < 0.5, "got {:?}", boxes["a"]);
    assert!((boxes["b"].y - 0.0).abs() < 0.5, "got {:?}", boxes["b"]);
}

/// A vertical RTL box's inline axis runs from the bottom (CSS Writing Modes
/// §2.1): the first inline, and the first item of a `flex-direction: row`
/// container, sit at the foot of the column.
#[test]
fn writing_mode_vertical_rtl_starts_the_inline_axis_at_the_bottom() {
    let inline = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineFlex),
            width: Some(LengthSpec::Px(20.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let container = |display: Option<DisplaySpec>, direction| StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display,
            direction,
            width: Some(LengthSpec::Px(80.0)),
            height: Some(LengthSpec::Px(80.0)),
            writing_mode: Some(WritingModeSpec::VerticalRl),
            dir: Some(DirSpec::Rtl),
            align_items: AlignSpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![inline("a"), inline("b")],
        text: None,
    };
    for (what, tree) in [
        ("inline formatting context", container(None, None)),
        (
            "flex-direction: row",
            container(Some(DisplaySpec::Flex), Some(FlexDirection::Row)),
        ),
    ] {
        let boxes = box_map(&tree, 80.0, 80.0);
        assert!(
            (boxes["a"].y - 60.0).abs() < 0.5 && (boxes["b"].y - 40.0).abs() < 0.5,
            "{what}: the first item sits at the bottom, the next above it, got a={:?} b={:?}",
            boxes["a"],
            boxes["b"]
        );
        assert!(
            (boxes["a"].x - 60.0).abs() < 0.5,
            "{what}: block-start is still the right, got {:?}",
            boxes["a"]
        );
    }
}

/// In a vertical RTL `flex-direction: column` container the cross axis is the
/// inline one, so `align-items: start` is the bottom.
#[test]
fn writing_mode_vertical_rtl_column_flex_puts_cross_start_at_the_bottom() {
    let item = |id: &str, height: f32| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(20.0)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Column),
            width: Some(LengthSpec::Px(80.0)),
            height: Some(LengthSpec::Px(80.0)),
            writing_mode: Some(WritingModeSpec::VerticalRl),
            dir: Some(DirSpec::Rtl),
            align_items: AlignSpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![item("a", 20.0), item("b", 30.0)],
        text: None,
    };
    let boxes = box_map(&tree, 80.0, 80.0);
    assert!(
        (boxes["a"].y - 60.0).abs() < 0.5 && (boxes["b"].y - 50.0).abs() < 0.5,
        "cross-start is the bottom, got a={:?} b={:?}",
        boxes["a"],
        boxes["b"]
    );
    assert!(
        boxes["a"].x > boxes["b"].x,
        "the main axis is the block one, from the right: a={:?} b={:?}",
        boxes["a"],
        boxes["b"]
    );
}

/// An RTL row that wraps fills its first line with its *first* items, laid
/// from the right: placement is flow-relative, and the page is only turned
/// when an origin is written. (Reversing the item list before wrapping put
/// the last items on the first line.)
#[test]
fn rtl_wrapping_row_fills_its_first_line_with_its_first_items() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            flex_wrap: FlexWrap::Wrap,
            width: Some(LengthSpec::Px(100.0)),
            height: Some(LengthSpec::Px(40.0)),
            align_items: AlignSpec::Start,
            dir: Some(DirSpec::Rtl),
            ..LayoutStyle::default()
        },
        children: (1..=5)
            .map(|index| px_box(&format!("item{index}"), 30.0, 20.0))
            .collect(),
        text: None,
    };
    let boxes = box_map(&tree, 100.0, 40.0);
    // Three 30px items to a 100px line: 1 2 3 on the first, 4 5 on the next.
    for (id, x, y) in [
        ("item1", 70.0, 0.0),
        ("item2", 40.0, 0.0),
        ("item3", 10.0, 0.0),
        ("item4", 70.0, 20.0),
        ("item5", 40.0, 20.0),
    ] {
        assert!(
            (boxes[id].x - x).abs() < 0.5 && (boxes[id].y - y).abs() < 0.5,
            "{id} at ({x}, {y}), got {:?}",
            boxes[id]
        );
    }
}

/// In `vertical-rl` the cross axis of a `flex-direction: row` container is
/// the block axis, which starts at the right: `align-items: start` puts every
/// item against the right edge, not the line against it and a narrower item
/// at the line's far side.
#[test]
fn vertical_rl_cross_start_is_the_right_edge_for_every_item() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(100.0)),
            height: Some(LengthSpec::Px(100.0)),
            writing_mode: Some(WritingModeSpec::VerticalRl),
            align_items: AlignSpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![px_box("wide", 40.0, 30.0), px_box("narrow", 20.0, 30.0)],
        text: None,
    };
    let boxes = box_map(&tree, 100.0, 100.0);
    assert!(
        (boxes["wide"].x - 60.0).abs() < 0.5,
        "got {:?}",
        boxes["wide"]
    );
    assert!(
        (boxes["narrow"].x - 80.0).abs() < 0.5,
        "the narrow item hugs the right edge too, got {:?}",
        boxes["narrow"]
    );
    assert!(
        (boxes["narrow"].y - 30.0).abs() < 0.5,
        "got {:?}",
        boxes["narrow"]
    );
}

#[test]
fn writing_mode_vertical_shaper_keeps_horizontal_metrics() {
    let metrics = crate::MeasureTextShaper.shape(
        id(1),
        &TextContent {
            value: "Hello".into(),
        },
        &ComputedStyle::with(|s| {
            s.writing_mode = WritingModeSpec::VerticalRl;
            s.font_size = 10.0;
        }),
        crate::TextShapeConstraints::default(),
    );
    assert!(
        metrics.width > metrics.height,
        "layout shaper must not swap metrics to fake glyph rotation, got {metrics:?}"
    );
}

#[test]
fn align_items_baseline_uses_shaped_ascent_not_approx_em() {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(1), id(2), None);
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                align_items: AlignSpec::Baseline,
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    for (value, font) in [(3u64, 20.0), (4u64, 20.0)] {
        queue.create(id(value), document, NodeKind::Text);
        queue.insert(id(2), id(value), None);
        queue.set_style(
            id(value),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    font_size: Some(font),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_text(
            id(value),
            TextContent {
                value: if value == 3 { "low" } else { "high" }.into(),
            },
        );
    }
    world.commit(queue).unwrap();
    struct AscentShaper;
    impl TextShaper for AscentShaper {
        fn shape(
            &mut self,
            id: StableNodeId,
            _text: &TextContent,
            _style: &ComputedStyle,
            _constraints: crate::TextShapeConstraints,
        ) -> TextMetrics {
            if id.get() == 3 {
                TextMetrics {
                    width: 40.0,
                    height: 20.0,
                    ascent: Some(8.0),
                }
            } else {
                TextMetrics {
                    width: 40.0,
                    height: 20.0,
                    ascent: Some(16.0),
                }
            }
        }
    }
    world
        .shape_text(&[id(3), id(4)], &mut AscentShaper)
        .unwrap();
    let layouts = RuntimeLayoutEngine
        .layout_document(&world, document, LayoutViewport::new(200.0, 80.0))
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>();
    let approx = nana_ui_core::TEXT_APPROX_ASCENT_EM * 20.0;
    assert!(
        (layouts[&id(4)].y - 0.0).abs() < 0.5,
        "taller ascent anchors the line, got {:?}",
        layouts[&id(4)]
    );
    assert!(
        (layouts[&id(3)].y - 8.0).abs() < 0.5,
        "shaped ascent 8 vs 16 must shift y by 8, not 0.8em ({approx}), got {:?}",
        layouts[&id(3)]
    );
}

#[test]
fn named_line_nth_uses_second_foo() {
    let item = StyleLayoutNode {
        id: "cell".into(),
        style: LayoutStyle {
            height: Some(LengthSpec::Px(40.0)),
            grid: nana_ui_core::GridStyle {
                placement: GridPlacement {
                    column_start: GridLine::NthName("foo".into(), 2),
                    column_end: GridLine::Name("foo".into()),
                    ..GridPlacement::default()
                }
                .into(),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(40.0)),
            grid: nana_ui_core::GridStyle {
                columns: Some(vec![GridTrack::Px(80.0), GridTrack::Px(120.0)]),
                column_line_names: Some(vec![
                    vec!["foo".into()],
                    vec!["foo".into()],
                    vec!["foo".into()],
                ]),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: vec![item],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 40.0);
    assert!(
        (boxes["cell"].x - 80.0).abs() < 0.5 && (boxes["cell"].width - 120.0).abs() < 0.5,
        "foo 2 / next foo must be the 120px track, got {:?}",
        boxes["cell"]
    );
}

#[test]
fn auto_fill_nth_named_line_uses_expanded_copies() {
    let item = StyleLayoutNode {
        id: "cell".into(),
        style: LayoutStyle {
            height: Some(LengthSpec::Px(40.0)),
            grid: nana_ui_core::GridStyle {
                placement: GridPlacement {
                    column_start: GridLine::NthName("mid".into(), 2),
                    column_end: GridLine::Name("mid".into()),
                    ..GridPlacement::default()
                }
                .into(),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(240.0)),
            height: Some(LengthSpec::Px(40.0)),
            grid: nana_ui_core::GridStyle {
                columns_repeat: Some(
                    GridRepeatAuto {
                        kind: GridTrackListUnsupported::RepeatAutoFill,
                        tracks: vec![GridTrack::Px(80.0)],
                        pattern_line_names: vec![vec!["mid".into()], Vec::new()],
                        ..Default::default()
                    }
                    .into(),
                ),
                // Pattern stored once — engine must expand, not resolve mid 2
                // against this single copy (which would miss and auto-place at 0).
                column_line_names: Some(vec![vec!["mid".into()], Vec::new()]),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: vec![item],
        text: None,
    };
    let boxes = box_map(&tree, 240.0, 40.0);
    assert!(
        (boxes["cell"].x - 80.0).abs() < 0.5 && (boxes["cell"].width - 80.0).abs() < 0.5,
        "mid 2 after auto-fit expansion must be the second 80px track, got {:?}",
        boxes["cell"]
    );
}

#[test]
fn grid_auto_slot_overflow_does_not_reuse_origin() {
    let occupied = {
        let mut occ = GridOccupancy::default();
        occ.occupy(0, 0, 1, 2);
        occ
    };
    let (row, col) = search_grid_auto_slot(&occupied, Some(0), None, 1, 1, 2, 1, 0, 0, false);
    assert!(
        !(row == 0 && col == 0),
        "full explicit row must not silently place at (0,0), got ({row},{col})"
    );
    assert_eq!(row, 0);
    assert!(col >= 2, "implicit column past wrap, got {col}");
}

fn spacing_tree(styles: &[(u64, LayoutStyle)]) -> crate::store::DenseNodeMap<LayoutBox> {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut q = MutationQueue::new();
    q.create(id(1), document, NodeKind::Document);
    for (i, (parent, layout)) in styles.iter().enumerate() {
        let node = id(i as u64 + 2);
        q.create(node, document, NodeKind::Element { tag: "div".into() });
        q.insert(id(*parent), node, None);
        q.set_style(
            node,
            NodeStyle {
                layout: Arc::new(layout.clone()),
                ..Default::default()
            },
        );
    }
    world.commit(q).unwrap();
    full_boxes(&world, document, LayoutViewport::new(400.0, 300.0))
}

#[test]
fn spacing_hug_includes_child_margins_on_both_axes() {
    for direction in [FlexDirection::Row, FlexDirection::Column] {
        let boxes = spacing_tree(&[
            (
                1,
                LayoutStyle {
                    width: Some(LengthSpec::Shrink),
                    height: Some(LengthSpec::Shrink),
                    direction: Some(direction),
                    ..Default::default()
                },
            ),
            (
                2,
                LayoutStyle {
                    width: Some(LengthSpec::Px(40.0)),
                    height: Some(LengthSpec::Px(20.0)),
                    margin: Some(LengthSpec::Px(10.0)),
                    ..Default::default()
                },
            ),
        ]);
        assert_eq!((boxes[&id(2)].width, boxes[&id(2)].height), (60.0, 40.0));
        assert_eq!((boxes[&id(3)].x, boxes[&id(3)].y), (10.0, 10.0));
    }
}

#[test]
fn spacing_percent_padding_uses_containing_width() {
    let boxes = spacing_tree(&[
        (
            1,
            LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                ..Default::default()
            },
        ),
        (
            2,
            LayoutStyle {
                width: Some(LengthSpec::Px(100.0)),
                padding: Some(LengthSpec::Percent(10.0)),
                ..Default::default()
            },
        ),
        (
            3,
            LayoutStyle {
                width: Some(LengthSpec::Px(10.0)),
                height: Some(LengthSpec::Px(10.0)),
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(boxes[&id(4)].x - boxes[&id(3)].x, 20.0);
    assert_eq!(boxes[&id(4)].y - boxes[&id(3)].y, 20.0);
    assert_eq!(boxes[&id(3)].height, 50.0);
}

#[test]
fn spacing_negative_margin_extends_flex_fill_budget() {
    let boxes = spacing_tree(&[
        (
            1,
            LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                direction: Some(FlexDirection::Row),
                ..Default::default()
            },
        ),
        (
            2,
            LayoutStyle {
                width: Some(LengthSpec::Fill),
                margin_left: Some(LengthSpec::Px(-10.0)),
                margin_right: Some(LengthSpec::Px(-10.0)),
                height: Some(LengthSpec::Px(20.0)),
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(boxes[&id(3)].width, 220.0);
    assert_eq!(boxes[&id(3)].x, -10.0);
}

#[test]
fn spacing_center_aligns_asymmetric_margin_box() {
    let boxes = spacing_tree(&[
        (
            1,
            LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(100.0)),
                direction: Some(FlexDirection::Row),
                align_items: AlignSpec::Center,
                ..Default::default()
            },
        ),
        (
            2,
            LayoutStyle {
                width: Some(LengthSpec::Px(20.0)),
                height: Some(LengthSpec::Px(20.0)),
                margin_top: Some(LengthSpec::Px(30.0)),
                margin_bottom: Some(LengthSpec::Px(10.0)),
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(boxes[&id(3)].y, 50.0);
}

#[test]
fn spacing_grid_padding_and_margin_use_final_cell() {
    let boxes = spacing_tree(&[
        (
            1,
            LayoutStyle {
                display: Some(DisplaySpec::Grid),
                width: Some(LengthSpec::Px(300.0)),
                height: Some(LengthSpec::Px(100.0)),
                grid: nana_ui_core::GridStyle {
                    columns: Some(vec![GridTrack::Px(100.0), GridTrack::Px(200.0)]),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        ),
        (
            2,
            LayoutStyle {
                width: Some(LengthSpec::Fill),
                margin: Some(LengthSpec::Px(10.0)),
                padding: Some(LengthSpec::Percent(10.0)),
                ..Default::default()
            },
        ),
        (
            3,
            LayoutStyle {
                width: Some(LengthSpec::Px(10.0)),
                height: Some(LengthSpec::Px(10.0)),
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(boxes[&id(3)].x, 10.0);
    assert_eq!(boxes[&id(3)].width, 80.0);
    assert_eq!(boxes[&id(4)].x - boxes[&id(3)].x, 10.0);
}

#[test]
fn spacing_fixed_box_reflows_percent_padding_when_parent_resizes() {
    use crate::{AppContext, Stack};
    let document = DocumentId::new(42).unwrap();
    let mut context = AppContext::new();
    let root = context
        .create_component(document, Stack::column(0.0).width(LengthSpec::Px(200.0)))
        .unwrap();
    let child = context
        .create_detached_component(
            document,
            Stack::from_layout(LayoutStyle {
                width: Some(LengthSpec::Px(100.0)),
                height: Some(LengthSpec::Px(100.0)),
                padding: Some(LengthSpec::Percent(10.0)),
                ..Default::default()
            }),
        )
        .unwrap();
    let leaf = context
        .create_detached_component(document, Stack::column(0.0).height(LengthSpec::Px(10.0)))
        .unwrap();
    context.append_child(root, child).unwrap();
    context.append_child(child, leaf).unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    context.layout_document(document, viewport).unwrap();
    let before = context.world().layout_box(child.stable_id()).unwrap();
    assert_eq!(
        context.world().layout_box(leaf.stable_id()).unwrap().x,
        20.0
    );
    context
        .update_component(root, |root, _| {
            *root = root.clone().width(LengthSpec::Px(300.0))
        })
        .unwrap();
    context
        .layout_document_with_frontier(document, viewport, &seeds(&[root.stable_id()]))
        .unwrap();
    assert_eq!(
        context.world().layout_box(child.stable_id()).unwrap(),
        before
    );
    assert_eq!(
        context.world().layout_box(leaf.stable_id()).unwrap().x,
        30.0
    );
    let extracted = context.world().extract_nodes(&[child.stable_id()]);
    assert_eq!(
        extracted[0].source_style.layout.resolved_padding().left,
        30.0
    );
    assert_eq!(
        context
            .world()
            .node_style(child.stable_id())
            .unwrap()
            .layout
            .padding,
        Some(LengthSpec::Percent(10.0))
    );
}

#[test]
fn spacing_grid_content_box_keeps_padding_in_border_size() {
    let boxes = spacing_tree(&[
        (
            1,
            LayoutStyle {
                display: Some(DisplaySpec::Grid),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(100.0)),
                grid: nana_ui_core::GridStyle {
                    columns: Some(vec![GridTrack::Px(200.0)]),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        ),
        (
            2,
            LayoutStyle {
                width: Some(LengthSpec::Px(40.0)),
                height: Some(LengthSpec::Px(20.0)),
                box_sizing: BoxSizing::ContentBox,
                padding: Some(LengthSpec::Px(10.0)),
                margin: Some(LengthSpec::Px(5.0)),
                ..Default::default()
            },
        ),
    ]);
    assert_eq!((boxes[&id(3)].x, boxes[&id(3)].y), (5.0, 5.0));
    assert_eq!((boxes[&id(3)].width, boxes[&id(3)].height), (60.0, 40.0));
}

#[test]
fn lazy_layout_input_avoids_a_single_item_projection_batch() {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), DocumentId::new(1).unwrap(), NodeKind::Text);
    world.commit(queue).unwrap();
    let expected = world.layout_inputs(&[id(1)]).unwrap().pop().unwrap();
    let mut inputs = LayoutInputMap::new(&world);
    let matches = |view: &PassInput<'_>| {
        **view.children == *expected.children
            && *view.style == *expected.style
            && view.writing == expected.writing
            && view.containing_writing == expected.containing_writing
            && view.text_metrics == expected.text_metrics
    };
    assert!(inputs.get(id(1)).unwrap().is_some_and(matches));
    assert!(inputs.get(id(1)).unwrap().is_some_and(matches));
    assert!(inputs.get(id(2)).unwrap().is_none());
    assert_eq!(inputs.materialized, 1);
}

// ---------------------------------------------------------------------------
// Differential harness for scoped layout.
//
// Every scoped-layout optimization is a decision to NOT recompute something.
// That class of change fails silently: the boxes are simply stale, and no
// assertion in the rest of the suite looks at a node the optimizer decided to
// skip. So the guard has to be an equivalence, driven over enough container
// shapes that a "fast path" which is only valid for simple flex columns cannot
// slip through.
//
// The harness drives a sequence of mutations through the SCOPED entry point,
// exactly as `RuntimeDocument::flush` does (drain work -> scoped layout ->
// write back changed boxes), and after every step compares every node against
// a full recompute from scratch.
// ---------------------------------------------------------------------------

/// One container shape to run the mutation sequence against.
struct DiffShape {
    name: &'static str,
    container: LayoutStyle,
    /// Style for each row; `usize` is the row index.
    row: fn(usize) -> LayoutStyle,
}

fn diff_shapes() -> Vec<DiffShape> {
    fn plain_row(_: usize) -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Px(60.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        }
    }
    fn growing_row(index: usize) -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Px(60.0)),
            height: Some(LengthSpec::Px(20.0)),
            flex_grow: Some(if index.is_multiple_of(3) { 1.0 } else { 0.0 }),
            flex_shrink: Some(1.0),
            ..LayoutStyle::default()
        }
    }
    fn margined_row(index: usize) -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Px(60.0)),
            height: Some(LengthSpec::Px(20.0)),
            margin_top: Some(LengthSpec::Px(index as f32 % 4.0)),
            margin_bottom: Some(LengthSpec::Px(2.0)),
            ..LayoutStyle::default()
        }
    }
    fn auto_margin_row(index: usize) -> LayoutStyle {
        let mut style = plain_row(index);
        if index.is_multiple_of(5) {
            style.margin_top = Some(LengthSpec::Auto);
        }
        style
    }

    let column = |extra: fn(&mut LayoutStyle)| {
        let mut style = LayoutStyle {
            width: Some(LengthSpec::Px(300.0)),
            height: Some(LengthSpec::Px(400.0)),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        };
        extra(&mut style);
        style
    };

    vec![
        DiffShape {
            name: "column-plain",
            container: column(|_| {}),
            row: plain_row,
        },
        DiffShape {
            name: "column-gap",
            container: column(|s| s.gap = Some(LengthSpec::Px(7.0))),
            row: plain_row,
        },
        DiffShape {
            name: "column-justify-center",
            container: column(|s| s.justify_content = JustifySpec::Center),
            row: plain_row,
        },
        DiffShape {
            name: "column-justify-space-between",
            container: column(|s| s.justify_content = JustifySpec::SpaceBetween),
            row: plain_row,
        },
        DiffShape {
            name: "column-justify-end",
            container: column(|s| s.justify_content = JustifySpec::End),
            row: plain_row,
        },
        DiffShape {
            name: "column-align-center",
            container: column(|s| s.align_items = AlignSpec::Center),
            row: plain_row,
        },
        DiffShape {
            name: "column-align-baseline",
            container: column(|s| s.align_items = AlignSpec::Baseline),
            row: plain_row,
        },
        DiffShape {
            name: "column-wrap",
            container: column(|s| {
                s.flex_wrap = FlexWrap::Wrap;
                s.height = Some(LengthSpec::Px(120.0));
            }),
            row: plain_row,
        },
        DiffShape {
            name: "column-grow",
            container: column(|_| {}),
            row: growing_row,
        },
        DiffShape {
            name: "column-margins",
            container: column(|s| s.gap = Some(LengthSpec::Px(3.0))),
            row: margined_row,
        },
        DiffShape {
            name: "column-auto-margins",
            container: column(|_| {}),
            row: auto_margin_row,
        },
        // Reversed axes: placement is flow-relative and turned onto the page
        // where an origin is written, and the sequential replay does the same.
        // `margined_row`'s top and bottom margins differ, so on a reversed
        // column the leading margin is the bottom one.
        DiffShape {
            name: "column-reverse-margins",
            container: column(|s| {
                s.flex_reverse = true;
                s.gap = Some(LengthSpec::Px(3.0));
            }),
            row: margined_row,
        },
        DiffShape {
            name: "column-rtl-cross-start-right",
            container: column(|s| s.dir = Some(DirSpec::Rtl)),
            row: plain_row,
        },
        DiffShape {
            name: "row-rtl",
            container: LayoutStyle {
                width: Some(LengthSpec::Px(400.0)),
                height: Some(LengthSpec::Px(80.0)),
                direction: Some(FlexDirection::Row),
                dir: Some(DirSpec::Rtl),
                ..LayoutStyle::default()
            },
            row: plain_row,
        },
        DiffShape {
            name: "vertical-rl-row",
            container: column(|s| {
                s.direction = Some(FlexDirection::Row);
                s.writing_mode = Some(WritingModeSpec::VerticalRl);
            }),
            row: margined_row,
        },
        DiffShape {
            name: "vertical-rl-rtl-row",
            container: column(|s| {
                s.direction = Some(FlexDirection::Row);
                s.writing_mode = Some(WritingModeSpec::VerticalRl);
                s.dir = Some(DirSpec::Rtl);
                s.gap = Some(LengthSpec::Px(2.0));
            }),
            row: margined_row,
        },
        DiffShape {
            name: "row-plain",
            container: LayoutStyle {
                width: Some(LengthSpec::Px(400.0)),
                height: Some(LengthSpec::Px(80.0)),
                direction: Some(FlexDirection::Row),
                ..LayoutStyle::default()
            },
            row: plain_row,
        },
        // Content-driven containers: the definite-size short circuit in
        // `intrinsic_size_scoped` cannot fire, so these are the shapes that
        // reach `MeasurePlan` at all. Every edit below therefore has to survive
        // BOTH plans agreeing to skip work.
        DiffShape {
            name: "column-auto-height",
            container: column(|s| s.height = None),
            row: plain_row,
        },
        DiffShape {
            name: "column-auto-both",
            container: column(|s| {
                s.width = None;
                s.height = None;
            }),
            row: plain_row,
        },
        DiffShape {
            name: "column-auto-height-gap-margins",
            container: column(|s| {
                s.height = None;
                s.gap = Some(LengthSpec::Px(3.0));
            }),
            row: margined_row,
        },
        DiffShape {
            name: "column-auto-height-align-center",
            container: column(|s| {
                s.height = None;
                s.align_items = AlignSpec::Center;
            }),
            row: plain_row,
        },
        DiffShape {
            name: "column-auto-height-grow",
            container: column(|s| s.height = None),
            row: growing_row,
        },
        DiffShape {
            name: "row-auto-width",
            container: LayoutStyle {
                width: None,
                height: Some(LengthSpec::Px(80.0)),
                direction: Some(FlexDirection::Row),
                ..LayoutStyle::default()
            },
            row: plain_row,
        },
        DiffShape {
            name: "column-auto-height-padded",
            container: column(|s| {
                s.height = None;
                s.padding = Some(LengthSpec::Px(6.0));
                s.box_sizing = BoxSizing::ContentBox;
            }),
            row: plain_row,
        },
        DiffShape {
            name: "row-wrap",
            container: LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(200.0)),
                direction: Some(FlexDirection::Row),
                flex_wrap: FlexWrap::Wrap,
                ..LayoutStyle::default()
            },
            row: plain_row,
        },
    ]
}

/// Root = id(1), container = id(2), row r = id(3 + r*2), its label = id(4 + r*2).
fn diff_tree(shape: &DiffShape, rows: usize) -> (UiWorld, DocumentId) {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(1), id(2), None);
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(shape.container.clone()),
            ..NodeStyle::default()
        },
    );
    for row in 0..rows {
        let row_id = id(3 + row as u64 * 2);
        let label_id = id(4 + row as u64 * 2);
        queue.create(row_id, document, NodeKind::Element { tag: "div".into() });
        queue.create(label_id, document, NodeKind::Text);
        queue.insert(id(2), row_id, None);
        queue.insert(row_id, label_id, None);
        queue.set_text(
            label_id,
            TextContent {
                value: format!("r{row}").into(),
            },
        );
        queue.set_style(
            row_id,
            NodeStyle {
                layout: Arc::new((shape.row)(row)),
                ..NodeStyle::default()
            },
        );
    }
    world.commit(queue).unwrap();
    (world, document)
}

/// Run one scoped pass the way the frame driver does, then assert the retained
/// cache agrees with a full recompute at EVERY node.
/// Emitted boxes and children measured BY THE SCOPED PASS.
struct ScopedStep {
    emitted: usize,
    children_measured: usize,
    measure_plans_reused: usize,
    plans_reused: usize,
    plan_misses: usize,
    plan_rebuilds: usize,
    suffixes_replayed: usize,
    full_document_fallbacks: usize,
}

fn scoped_step_matches_full(
    world: &mut UiWorld,
    document: DocumentId,
    viewport: LayoutViewport,
    retained: &mut RetainedLayoutCache,
    label: &str,
) -> ScopedStep {
    let work = world.take_system_work();
    // Layout reads inherited writing context from the resolved style, as the
    // frame pipeline resolves it before layout.
    world.resolve_styles(&work.style).unwrap();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(
            world,
            document,
            viewport,
            &work.layout_frontier_seeds,
            retained,
            false,
        )
        .unwrap();
    let execution = retained.execution_stats(document);
    let step = ScopedStep {
        emitted: emitted.len(),
        children_measured: execution.children_measured,
        measure_plans_reused: execution.measure_plans_reused,
        plans_reused: execution.placement_plans_reused,
        plan_misses: execution.plan_misses,
        plan_rebuilds: execution.plan_rebuilds,
        suffixes_replayed: execution.suffixes_replayed,
        full_document_fallbacks: retained.frontier_stats(document).full_document_fallbacks,
    };
    write_changed_boxes(world, &emitted);
    let _ = world.take_system_work();

    let expected = full_boxes(world, document, viewport);
    let cached = &retained.documents[&document].page_boxes(world);
    for (node, box_) in &expected {
        // A node that generates no box at all -- `display:contents` -- is never
        // placed, so the scoped pass never emits it and the retained map has no
        // entry, while the full pass fills its `document_order` slot with a
        // default. Absent and zero are the same statement here: the retained
        // map only ever loses an entry by never having had one, so this cannot
        // hide a box the scoped pass dropped.
        let cached = cached.get(&node).copied().unwrap_or_default();
        assert_eq!(
            cached, *box_,
            "{label}: scoped layout diverged from full recompute at {node:?}"
        );
    }
    step
}

/// A reversed axis keeps the sequential replay: a child's position still
/// depends only on the children before it, measured back from the far edge.
/// Edit one row mid-list and the container plan has to be reused — and agree
/// with a full recompute — on every reversed shape.
#[test]
fn reversed_axes_keep_the_sequential_replay() {
    const ROWS: usize = 12;
    let viewport = LayoutViewport::new(320.0, 400.0);
    let reversed = [
        "column-reverse-margins",
        "column-rtl-cross-start-right",
        "vertical-rl-row",
        "vertical-rl-rtl-row",
    ];
    for shape in diff_shapes()
        .into_iter()
        .filter(|shape| reversed.contains(&shape.name))
    {
        let (mut world, document) = diff_tree(&shape, ROWS);
        let mut retained = RetainedLayoutCache::default();
        let _ = world.take_system_work();
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        for (step, height) in [26.0f32, 14.0].into_iter().enumerate() {
            let row = ROWS / 2;
            let mut style = (shape.row)(row);
            style.height = Some(LengthSpec::Px(height));
            let mut queue = MutationQueue::new();
            queue.set_style(
                id(3 + row as u64 * 2),
                NodeStyle {
                    layout: Arc::new(style),
                    ..NodeStyle::default()
                },
            );
            world.commit(queue).unwrap();
            let work = world.take_system_work();
            let emitted = RuntimeLayoutEngine
                .layout_document_with_frontier(
                    &world,
                    document,
                    viewport,
                    &work.layout_frontier_seeds,
                    &mut retained,
                    false,
                )
                .unwrap();
            let replayed = retained.execution_stats(document).suffixes_replayed;
            write_changed_boxes(&mut world, &emitted);
            let _ = world.take_system_work();
            let expected = full_boxes(&world, document, viewport);
            let cached = &retained.documents[&document].page_boxes(&world);
            for (node, box_) in &expected {
                assert_eq!(
                    cached.get(&node).copied().unwrap_or_default(),
                    *box_,
                    "{} step {step}: scoped layout diverged at {node:?}",
                    shape.name
                );
            }
            assert!(
                replayed > 0,
                "{} step {step}: the container must replay its suffix, not relayout",
                shape.name
            );
        }
    }
}

/// An ancestor's `direction` reaches a row that never declared one: the row
/// inherits RTL and its label moves to the right end, while the row's own
/// style, its child list and its label's size are all unchanged. Only the
/// inherited writing context tells the row's cached placement plan it is
/// stale.
#[test]
fn an_inherited_direction_change_retires_a_descendant_containers_plan() {
    const ROWS: usize = 6;
    let viewport = LayoutViewport::new(320.0, 400.0);
    let shape = DiffShape {
        name: "rows-of-labels",
        container: LayoutStyle {
            width: Some(LengthSpec::Px(300.0)),
            height: Some(LengthSpec::Px(400.0)),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        },
        row: |_| LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(300.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
    };
    let (mut world, document) = diff_tree(&shape, ROWS);
    let mut retained = RetainedLayoutCache::default();
    let _ = world.take_system_work();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &emitted);
    let _ = world.take_system_work();
    // A scoped pass first, so every row has a plan to be tempted by.
    let mut queue = MutationQueue::new();
    let mut row = (shape.row)(0);
    row.height = Some(LengthSpec::Px(22.0));
    queue.set_style(
        id(3),
        NodeStyle {
            layout: Arc::new(row),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    scoped_step_matches_full(&mut world, document, viewport, &mut retained, "prime");
    let label_before = retained.documents[&document].page_boxes(&world)[&id(6)];

    let mut container = shape.container.clone();
    container.dir = Some(DirSpec::Rtl);
    let mut queue = MutationQueue::new();
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(container),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    scoped_step_matches_full(
        &mut world,
        document,
        viewport,
        &mut retained,
        "container turned rtl",
    );
    let label_after = retained.documents[&document].page_boxes(&world)[&id(6)];
    assert!(
        label_after.x > label_before.x,
        "the row inherited rtl and its label moved right: {label_before:?} -> {label_after:?}"
    );
}

/// The equivalence itself: for every container shape, a sequence of changes at
/// the head, middle and tail of the child list must keep scoped layout
/// identical to a full recompute.
#[test]
fn scoped_layout_matches_full_recompute_across_container_shapes_and_edits() {
    const ROWS: usize = 24;
    let viewport = LayoutViewport::new(320.0, 400.0);
    for shape in diff_shapes() {
        // If the equivalence below passed without ever consulting `MeasurePlan`
        // -- a guard that went permanently false, a slot that never matched --
        // the harness would be proving nothing about the cache it was extended
        // to cover. Every shape reaches it, content-driven container or not,
        // because the document root is itself content-driven.
        let mut measure_plans_reused = 0usize;
        let (mut world, document) = diff_tree(&shape, ROWS);
        let mut retained = RetainedLayoutCache::default();
        let _ = world.take_system_work();
        // Bootstrap exactly like the driver's first frame.
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();

        // Edits at the tail, middle and head, each repeated so a change and a
        // change-back both go through the scoped path.
        for &row in &[ROWS - 1, ROWS / 2, 0, ROWS - 1, ROWS / 2] {
            for &height in &[26.0f32, 20.0, 13.0] {
                let mut style = (shape.row)(row);
                style.height = Some(LengthSpec::Px(height));
                let mut queue = MutationQueue::new();
                queue.set_style(
                    id(3 + row as u64 * 2),
                    NodeStyle {
                        layout: Arc::new(style),
                        ..NodeStyle::default()
                    },
                );
                world.commit(queue).unwrap();
                measure_plans_reused += scoped_step_matches_full(
                    &mut world,
                    document,
                    viewport,
                    &mut retained,
                    &format!("{} row {row} height {height}", shape.name),
                )
                .measure_plans_reused;
            }
        }

        // Edits that MOVE a child without resizing it. The intrinsic-size check
        // cannot see these, so they are what the per-child style comparison is
        // for: margin, alignment and order all change placement while the
        // measured box stays identical.
        for (label, mutate) in [
            (
                "margin",
                (|style: &mut LayoutStyle| style.margin_top = Some(LengthSpec::Px(9.0)))
                    as fn(&mut LayoutStyle),
            ),
            ("align-self", |style: &mut LayoutStyle| {
                style.align_self = Some(AlignSpec::End)
            }),
            ("order", |style: &mut LayoutStyle| style.order = -1),
            ("grow", |style: &mut LayoutStyle| {
                style.flex_grow = Some(4.0)
            }),
        ] {
            let row = ROWS / 4;
            let mut style = (shape.row)(row);
            mutate(&mut style);
            let mut queue = MutationQueue::new();
            queue.set_style(
                id(3 + row as u64 * 2),
                NodeStyle {
                    layout: Arc::new(style),
                    ..NodeStyle::default()
                },
            );
            world.commit(queue).unwrap();
            measure_plans_reused += scoped_step_matches_full(
                &mut world,
                document,
                viewport,
                &mut retained,
                &format!("{} {label}-only edit", shape.name),
            )
            .measure_plans_reused;
        }

        // A width change on a middle row: cross-axis, not main-axis.
        let mut style = (shape.row)(ROWS / 3);
        style.width = Some(LengthSpec::Px(90.0));
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(3 + (ROWS / 3) as u64 * 2),
            NodeStyle {
                layout: Arc::new(style),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        measure_plans_reused += scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            &format!("{} cross-axis width", shape.name),
        )
        .measure_plans_reused;

        // A change on the CONTAINER itself, which invalidates every child.
        let mut container = shape.container.clone();
        container.gap = Some(LengthSpec::Px(11.0));
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(2),
            NodeStyle {
                layout: Arc::new(container),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        measure_plans_reused += scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            &format!("{} container gap", shape.name),
        )
        .measure_plans_reused;

        // Structural edits: append, then detach and park rows, then edit and
        // put a row back while detached and parked subtrees sit in the world.
        let fresh = id(3 + ROWS as u64 * 2 + 100);
        let mut queue = MutationQueue::new();
        queue.create(fresh, document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(2), fresh, None);
        queue.set_style(
            fresh,
            NodeStyle {
                layout: Arc::new((shape.row)(1)),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        measure_plans_reused += scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            &format!("{} append row", shape.name),
        )
        .measure_plans_reused;

        let mut queue = MutationQueue::new();
        queue.detach(id(3 + 5 * 2));
        world.commit(queue).unwrap();
        measure_plans_reused += scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            &format!("{} detach row 5", shape.name),
        )
        .measure_plans_reused;

        // Issue #228: a parked or detached node anywhere in the world used to
        // turn every container's plans off. Real documents always have one.
        let mut queue = MutationQueue::new();
        queue.park_subtree(id(3 + 9 * 2));
        world.commit(queue).unwrap();
        scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            &format!("{} park row 9", shape.name),
        );
        let mut reused_with_detached = 0;
        for &row in &[ROWS - 1, ROWS / 2 + 1, 1] {
            for &height in &[27.0f32, 20.0] {
                let mut style = (shape.row)(row);
                style.height = Some(LengthSpec::Px(height));
                let mut queue = MutationQueue::new();
                queue.set_style(
                    id(3 + row as u64 * 2),
                    NodeStyle {
                        layout: Arc::new(style),
                        ..NodeStyle::default()
                    },
                );
                world.commit(queue).unwrap();
                let step = scoped_step_matches_full(
                    &mut world,
                    document,
                    viewport,
                    &mut retained,
                    &format!(
                        "{} row {row} height {height} beside detached rows",
                        shape.name
                    ),
                );
                reused_with_detached += step.plans_reused + step.measure_plans_reused;
            }
        }
        assert!(
            reused_with_detached > 0,
            "{}: no plan was reused while detached rows sat in the world",
            shape.name
        );
        for (label, parked) in [("detached row 5", 5u64), ("parked row 9", 9)] {
            let mut queue = MutationQueue::new();
            queue.insert(id(2), id(3 + parked * 2), Some(id(3 + (parked + 1) * 2)));
            world.commit(queue).unwrap();
            scoped_step_matches_full(
                &mut world,
                document,
                viewport,
                &mut retained,
                &format!("{} reinsert {label}", shape.name),
            );
            let mut style = (shape.row)(parked as usize);
            style.height = Some(LengthSpec::Px(31.0));
            let mut queue = MutationQueue::new();
            queue.set_style(
                id(3 + parked * 2),
                NodeStyle {
                    layout: Arc::new(style),
                    ..NodeStyle::default()
                },
            );
            world.commit(queue).unwrap();
            scoped_step_matches_full(
                &mut world,
                document,
                viewport,
                &mut retained,
                &format!("{} resize reinserted {label}", shape.name),
            );
        }

        // Definite-size containers take the intrinsic-size short circuit and
        // never create a MeasurePlan. Only content-sized shapes exercise the
        // plan reuse path guarded by this differential harness.
        if shape.container.width.is_none() || shape.container.height.is_none() {
            assert!(
                measure_plans_reused > 0,
                "{}: the measure plan was never reused, so this shape did not \
                 exercise it",
                shape.name
            );
        }
    }
}

/// A child-list edit on a sequential container replays from the edit rather
/// than walking every child: a row arrives at the end, at the head and in the
/// middle, two arrive at once, rows leave from the middle, the head and the
/// end, one moves within the list, and a hidden and a positioned row arrive.
/// On every shape the scoped pass agrees with a full recompute at every node.
/// On a sequential shape the container walks none of its rows for an edit
/// in its flow: the only children walked are the labels of the rows that
/// moved, placed again where their rows went.
#[test]
fn a_child_list_edit_replays_from_the_edit() {
    const ROWS: usize = 12;
    // The shapes whose plans are sequential: no space to share out, no
    // wrapping, start or stretch alignment, no growing rows.
    const SEQUENTIAL: [&str; 14] = [
        "column-plain",
        "column-gap",
        "column-margins",
        "column-reverse-margins",
        "column-rtl-cross-start-right",
        "row-rtl",
        "vertical-rl-row",
        "vertical-rl-rtl-row",
        "row-plain",
        "column-auto-height",
        "column-auto-both",
        "column-auto-height-gap-margins",
        "column-auto-height-padded",
        "row-auto-width",
    ];
    let viewport = LayoutViewport::new(320.0, 400.0);
    for shape in diff_shapes() {
        let (mut world, document) = diff_tree(&shape, ROWS);
        let mut retained = RetainedLayoutCache::default();
        let _ = world.take_system_work();
        // Bootstrap like the driver's first frame, then resize a row and
        // back, so the container has measured itself on a scoped pass and
        // holds its measure plans.
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        for height in [26.0f32, 20.0] {
            let mut style = (shape.row)(1);
            style.height = Some(LengthSpec::Px(height));
            let mut queue = MutationQueue::new();
            queue.set_style(
                id(5),
                NodeStyle {
                    layout: Arc::new(style),
                    ..NodeStyle::default()
                },
            );
            world.commit(queue).unwrap();
            scoped_step_matches_full(
                &mut world,
                document,
                viewport,
                &mut retained,
                &format!("{} row 1 height {height}", shape.name),
            );
        }
        let mut fresh = 1_000u64;
        let mut arrive = |queue: &mut MutationQueue, before: Option<StableNodeId>, style| {
            let row = id(fresh);
            fresh += 1;
            queue.create(row, document, NodeKind::Element { tag: "div".into() });
            queue.insert(id(2), row, before);
            queue.set_style(
                row,
                NodeStyle {
                    layout: Arc::new(style),
                    ..NodeStyle::default()
                },
            );
        };
        for step in [
            "arrives at the end",
            "arrives at the head",
            "arrives in the middle",
            "arrives twice at once",
            "leaves the middle",
            "leaves the head",
            "leaves the end",
            "moves within the list",
            "arrives hidden",
            "arrives positioned",
        ] {
            let rows = world.node(id(2)).unwrap().children;
            let before: Vec<(StableNodeId, Option<LayoutBox>)> = rows
                .iter()
                .map(|row| (*row, world.layout_box(*row)))
                .collect();
            let row = (shape.row)(0);
            let middle = rows[rows.len() / 2];
            let mut queue = MutationQueue::new();
            match step {
                "arrives at the end" => arrive(&mut queue, None, row),
                "arrives at the head" => arrive(&mut queue, Some(rows[0]), row),
                "arrives in the middle" => arrive(&mut queue, Some(middle), row),
                "arrives twice at once" => {
                    arrive(&mut queue, Some(rows[2]), row.clone());
                    arrive(&mut queue, Some(rows[rows.len() - 3]), row);
                }
                "leaves the middle" => queue.despawn_subtree(middle),
                "leaves the head" => queue.despawn_subtree(rows[0]),
                "leaves the end" => queue.despawn_subtree(rows[rows.len() - 1]),
                "moves within the list" => queue.insert(id(2), rows[1], Some(rows[rows.len() - 2])),
                "arrives hidden" => arrive(
                    &mut queue,
                    Some(middle),
                    LayoutStyle {
                        display: Some(DisplaySpec::None),
                        ..row
                    },
                ),
                "arrives positioned" => arrive(
                    &mut queue,
                    Some(middle),
                    LayoutStyle {
                        position: PositionSpec::Absolute,
                        ..row
                    },
                ),
                _ => unreachable!(),
            }
            world.commit(queue).unwrap();
            let label = format!("{}: a row {step}", shape.name);
            let scoped =
                scoped_step_matches_full(&mut world, document, viewport, &mut retained, &label);
            // A positioned arrival changes the container's overlay, which
            // the replay does not rebuild: the container places its rows.
            if !SEQUENTIAL.contains(&shape.name) || step == "arrives positioned" {
                continue;
            }
            let moved = before
                .iter()
                .filter(|(row, kept)| world.layout_box(*row).is_some_and(|now| Some(now) != *kept))
                .count();
            // The document root may walk its one child, the container, when
            // it has no measure plan yet: a fixed container never put it on
            // a scoped measure frontier before.
            assert!(
                scoped.children_measured <= moved + 1,
                "{label}: {} children walked, {moved} rows moved",
                scoped.children_measured
            );
        }
    }
}

/// A measure plan that patches its container's measurement retires the
/// intrinsic facts measured before the patch: they describe content the
/// patch changed, and a later constraint the container's used sizes do not
/// hold would read them and miss it.
#[test]
fn a_patched_measurement_retires_the_facts_measured_before_it() {
    let shape = diff_shapes()
        .into_iter()
        .find(|shape| shape.name == "column-auto-height")
        .unwrap();
    let viewport = LayoutViewport::new(320.0, 400.0);
    let (mut world, document) = diff_tree(&shape, 12);
    let mut retained = RetainedLayoutCache::default();
    let _ = world.take_system_work();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &emitted);
    let _ = world.take_system_work();
    let container_facts = |retained: &RetainedLayoutCache| {
        retained.documents[&document]
            .intrinsics
            .contains_metrics(id(2))
    };
    // The first scoped pass measures the container's rows and records its
    // plans and facts; the second patches the plan.
    for (pass, height) in [26.0f32, 20.0].into_iter().enumerate() {
        let mut style = (shape.row)(1);
        style.height = Some(LengthSpec::Px(height));
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(5),
            NodeStyle {
                layout: Arc::new(style),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let scoped = scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            &format!("row 1 height {height}"),
        );
        if pass == 0 {
            assert!(
                container_facts(&retained),
                "a measured container keeps its facts"
            );
        } else {
            assert!(
                scoped.measure_plans_reused > 0,
                "the plan answered the edit"
            );
            assert!(
                !container_facts(&retained),
                "a patched container keeps no facts from before the patch"
            );
        }
    }
}

/// A measure plan a pass did not consult while a child of its container
/// changed is not reused: it holds the child as it was. The document root
/// measures under one viewport a frame, so a width resize and back leaves it
/// a plan for each width. A row then grows while the root measures under the
/// narrow width only, and the wide plan, met again after the next resize,
/// would hand back the height from before the row grew.
#[test]
fn a_plan_a_pass_skipped_while_a_child_changed_is_not_reused() {
    let shape = diff_shapes()
        .into_iter()
        .find(|shape| shape.name == "column-auto-height")
        .unwrap();
    let narrow = LayoutViewport::new(320.0, 400.0);
    let wide = LayoutViewport::new(360.0, 400.0);
    let (mut world, document) = diff_tree(&shape, 12);
    let mut retained = RetainedLayoutCache::default();
    let _ = world.take_system_work();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, narrow, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &emitted);
    let _ = world.take_system_work();
    let resize_row = |world: &mut UiWorld, height: f32| {
        let mut style = (shape.row)(1);
        style.height = Some(LengthSpec::Px(height));
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(5),
            NodeStyle {
                layout: Arc::new(style),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
    };
    // Scoped plans under the narrow width, then the wide one, then back.
    resize_row(&mut world, 26.0);
    scoped_step_matches_full(&mut world, document, narrow, &mut retained, "row 1 grows");
    scoped_step_matches_full(&mut world, document, wide, &mut retained, "wide");
    scoped_step_matches_full(&mut world, document, narrow, &mut retained, "narrow");
    // The row grows while the root measures under the narrow width.
    resize_row(&mut world, 40.0);
    scoped_step_matches_full(
        &mut world,
        document,
        narrow,
        &mut retained,
        "row 1 grows again",
    );
    // The wide plan never saw that.
    scoped_step_matches_full(&mut world, document, wide, &mut retained, "wide again");
}

/// The case the style-pointer check cannot see: a child's intrinsic size
/// changing while its own style stays byte-identical.
///
/// A text edit two levels down resizes a content-sized row without touching
/// that row's style. If the container reuses its cached plan here, every
/// sibling below keeps a stale position -- and no assertion about the edited
/// subtree would notice.
#[test]
fn content_growth_under_a_child_moves_its_siblings() {
    let _ = content_growth_under_a_child_moves_its_siblings_with(Some(LengthSpec::Px(600.0)));
}

/// The same case one level harder: the container is CONTENT-SIZED, so the
/// growing row also changes the container's own measurement.
///
/// This is what `MeasurePlan` has to get wrong to be dangerous. The row's own
/// style is byte-identical across the edit, so a plan that compared only styles
/// would hand back the container's previous height and every row below it would
/// keep a stale position -- silently, because nothing asserts about a node the
/// optimizer decided not to touch.
#[test]
fn content_growth_under_a_child_moves_its_siblings_when_the_container_hugs() {
    let _ = content_growth_under_a_child_moves_its_siblings_with(None);
}

/// `auto` and `shrink` heights resolve to no length, as no height does: the
/// container hugs its rows the same way and patches its measure plan with the
/// grown row instead of measuring every row again.
#[test]
fn content_growth_under_a_child_moves_its_siblings_when_the_container_shrinks() {
    let hugging = content_growth_under_a_child_moves_its_siblings_with(None);
    for height in [LengthSpec::Shrink, LengthSpec::Auto] {
        let step = content_growth_under_a_child_moves_its_siblings_with(Some(height));
        assert_eq!(
            step.children_measured, hugging.children_measured,
            "a {height:?} column must take the grown row from its plan as one with \
             no height does, not measure all 12 rows again"
        );
    }
}

fn content_growth_under_a_child_moves_its_siblings_with(
    container_height: Option<LengthSpec>,
) -> ScopedStep {
    let viewport = LayoutViewport::new(320.0, 600.0);
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(1), id(2), None);
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(300.0)),
                // Fixed, so the PLACEMENT plan's own inputs stay identical when
                // a row grows and its per-child checks actually run. `None`
                // instead puts the container on the measure path as well, where
                // the growing row moves the container's own size.
                height: container_height,
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    const ROWS: u64 = 12;
    for row in 0..ROWS {
        let row_id = id(3 + row * 2);
        let label_id = id(4 + row * 2);
        queue.create(row_id, document, NodeKind::Element { tag: "div".into() });
        queue.create(label_id, document, NodeKind::Text);
        queue.insert(id(2), row_id, None);
        queue.insert(row_id, label_id, None);
        queue.set_text(label_id, TextContent { value: "x".into() });
        // Content-sized: no width or height, so the label drives the row.
        queue.set_style(row_id, NodeStyle::default());
        queue.set_style(label_id, NodeStyle::default());
    }
    world.commit(queue).unwrap();

    // Give the labels real metrics so text actually drives the row height.
    let shape = |world: &mut UiWorld| {
        struct Shaper;
        impl TextShaper for Shaper {
            fn shape(
                &mut self,
                _id: StableNodeId,
                text: &TextContent,
                _style: &ComputedStyle,
                _constraints: crate::TextShapeConstraints,
            ) -> TextMetrics {
                TextMetrics {
                    width: text.value.len() as f32 * 8.0,
                    height: 16.0 * (1 + text.value.matches('\n').count()) as f32,
                    ascent: None,
                }
            }
        }
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        world.shape_text(&work.text, &mut Shaper).unwrap();
    };
    shape(&mut world);

    let mut retained = RetainedLayoutCache::default();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &emitted);
    shape(&mut world);
    let _ = world.take_system_work();

    // Prime: a full pass records no measure plans, so without an edit first
    // the pass below would be building the plan rather than being caught out by
    // it. Grow a DIFFERENT row's text so the plan exists and is current.
    let mut queue = MutationQueue::new();
    queue.set_text(id(4), TextContent { value: "zz".into() });
    world.commit(queue).unwrap();
    shape(&mut world);
    scoped_step_matches_full(
        &mut world,
        document,
        viewport,
        &mut retained,
        "priming pass",
    );
    let _ = world.take_system_work();

    // Grow the SECOND row's text. Its own style never changes.
    let label = id(4 + 2);
    let before = world.node_style(id(3 + 2)).unwrap().layout.clone();
    let mut queue = MutationQueue::new();
    queue.set_text(
        label,
        TextContent {
            value: "yyyy\nyyyy\nyyyy".into(),
        },
    );
    world.commit(queue).unwrap();
    shape(&mut world);
    assert!(
        Arc::ptr_eq(&before, &world.node_style(id(3 + 2)).unwrap().layout),
        "the edited row's own style must be untouched, or this test is not \
         exercising the case the style check cannot see"
    );

    let step = scoped_step_matches_full(
        &mut world,
        document,
        viewport,
        &mut retained,
        "text growth under a content-sized row",
    );
    if container_height
        .is_some_and(|height| !matches!(height, LengthSpec::Auto | LengthSpec::Shrink))
    {
        assert!(
            step.plans_reused > 0,
            "the container plan must be reached here, or the intrinsic check \
             this test exists to guard is never consulted"
        );
    } else {
        // The container's own height moved, so its measure plan must not hand
        // back the old one -- the full recompute above would catch that. It is
        // consulted, and takes the row's growth from the sum it recorded rather
        // than measuring every row again; the rows after the grown one only
        // move, taking their labels along. What has to be true is that the plan
        // was reached at all.
        assert!(
            step.measure_plans_reused > 0,
            "a hugging container whose child grew must consult its measure plan, \
             or the check this test exists to guard is never reached"
        );
    }
    step
}

/// A container that only moves takes its children along (see
/// `ContainerPlan::moved_to`). The box before it grows and shrinks back, and
/// everything in it lands where a full layout puts it -- a row in flow
/// shifted relatively, the cells of a grid, and, when the container is
/// positioned, a child placed against its padding box from the top left and
/// one from the bottom right -- while a child fixed to the viewport stays.
/// When the container is static those two hang from the initial containing
/// block and stay as well. Nothing in it misses or records its plan again:
/// the container is padded and bordered, so its content box moves with it.
#[test]
fn a_container_that_only_moves_takes_its_children_along() {
    container_that_only_moves(PositionSpec::Relative);
    container_that_only_moves(PositionSpec::Static);
}

fn container_that_only_moves(position: PositionSpec) {
    let viewport = LayoutViewport::new(320.0, 400.0);
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    let style = |queue: &mut MutationQueue, node: u64, layout: LayoutStyle| {
        queue.set_style(
            id(node),
            NodeStyle {
                layout: Arc::new(layout),
                ..NodeStyle::default()
            },
        );
    };
    let fixed = |width: f32, height: f32| LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        ..LayoutStyle::default()
    };
    queue.create(id(1), document, NodeKind::Document);
    // The page column (2), the box that grows (3) and the container that only
    // moves (4): a row (5), a grid (6) of three cells (7-9), two children
    // positioned in its content box (10, 11) and one fixed (12).
    for (node, parent) in [
        (2, 1),
        (3, 2),
        (4, 2),
        (5, 4),
        (6, 4),
        (7, 6),
        (8, 6),
        (9, 6),
        (10, 4),
        (11, 4),
        (12, 4),
    ] {
        queue.create(id(node), document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(parent), id(node), None);
    }
    style(
        &mut queue,
        2,
        LayoutStyle {
            width: Some(LengthSpec::Px(300.0)),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        },
    );
    style(&mut queue, 3, fixed(60.0, 20.0));
    style(
        &mut queue,
        4,
        LayoutStyle {
            position,
            width: Some(LengthSpec::Px(200.0)),
            direction: Some(FlexDirection::Column),
            padding: Some(LengthSpec::Px(5.0)),
            border_width: Some(2.0),
            gap: Some(LengthSpec::Px(3.0)),
            ..LayoutStyle::default()
        },
    );
    style(
        &mut queue,
        5,
        LayoutStyle {
            position: PositionSpec::Relative,
            offset_left: Some(LengthSpec::Px(7.0)),
            ..fixed(60.0, 12.0)
        },
    );
    style(
        &mut queue,
        6,
        LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(100.0)),
            grid: nana_ui_core::GridStyle {
                columns: Some(vec![GridTrack::Px(40.0), GridTrack::Px(60.0)]),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
    );
    for cell in [7, 8, 9] {
        style(&mut queue, cell, fixed(30.0, 10.0));
    }
    style(
        &mut queue,
        10,
        LayoutStyle {
            position: PositionSpec::Absolute,
            offset_top: Some(LengthSpec::Px(4.0)),
            offset_left: Some(LengthSpec::Px(8.0)),
            ..fixed(30.0, 10.0)
        },
    );
    style(
        &mut queue,
        11,
        LayoutStyle {
            position: PositionSpec::Absolute,
            offset_bottom: Some(LengthSpec::Px(3.0)),
            offset_right: Some(LengthSpec::Px(6.0)),
            ..fixed(20.0, 10.0)
        },
    );
    style(
        &mut queue,
        12,
        LayoutStyle {
            position: PositionSpec::Fixed,
            offset_top: Some(LengthSpec::Px(10.0)),
            offset_left: Some(LengthSpec::Px(10.0)),
            ..fixed(20.0, 20.0)
        },
    );
    world.commit(queue).unwrap();
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    let mut retained = RetainedLayoutCache::default();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &emitted);
    let _ = world.take_system_work();

    let fixed_box = world.layout_box(id(12));
    let absolute = |world: &UiWorld| (world.layout_box(id(10)), world.layout_box(id(11)));
    let absolute_before = absolute(&world);
    for height in [35.0, 20.0, 35.0] {
        let mut queue = MutationQueue::new();
        style(&mut queue, 3, fixed(60.0, height));
        world.commit(queue).unwrap();
        let before = world.layout_box(id(4)).unwrap();
        let label = format!("the box before it {height} px tall");
        let step = scoped_step_matches_full(&mut world, document, viewport, &mut retained, &label);
        let after = world.layout_box(id(4)).unwrap();
        assert_ne!(before.y, after.y, "{label}: the container moved");
        assert_eq!(
            (step.plan_misses, step.plan_rebuilds),
            (0, 0),
            "{label}: what moved took its plans along"
        );
        assert_eq!(world.layout_box(id(12)), fixed_box, "{label}");
        if position == PositionSpec::Static {
            assert_eq!(absolute(&world), absolute_before, "{label}: not its block");
        } else {
            assert_eq!(
                world.layout_box(id(10)).unwrap().y,
                after.y + 2.0 + 4.0,
                "{label}: inside the padding box"
            );
        }
    }
}

/// The harness above proves correctness; this one proves the scoped pass is
/// actually incremental, so a "fix" that just relayouts everything cannot pass
/// both.
///
/// The property: an edit CONTAINED inside a fixed-size row cannot move any
/// other row, so the list container must not touch its other children at all.
/// Layout invalidation still propagates to the container (and to the document
/// root), so this is precisely the case where the dirty set says "the whole
/// spine changed" and the actual work owed is constant.
///
/// Both edit kinds are covered: one CONTAINED inside a fixed-size row, and one
/// that RESIZES the last row. The second shifts nothing either -- there is no
/// row after it -- so it must also stay flat, which is what the container's
/// suffix replay buys.
#[test]
fn scoped_layout_contained_edit_does_not_scan_siblings_as_the_document_grows() {
    // A fixed-height container takes the definite-size short circuit in
    // `intrinsic_size_scoped` and never measures a child at all, so it cannot
    // tell whether the MEASURE side is incremental. A hugging container has no
    // short circuit: it reaches `MeasurePlan` for every frame, and a regression
    // there shows up as a sibling scan that grows with the document.
    contained_edit_stays_flat(Some(LengthSpec::Px(40000.0)), false);
    contained_edit_stays_flat(None, false);
}

/// Issue #228: the same gate with a parked row in the world. Any parked or
/// detached node used to switch every container's plans off, and a real
/// document always has one.
#[test]
fn scoped_layout_contained_edit_stays_flat_beside_a_parked_row() {
    contained_edit_stays_flat(Some(LengthSpec::Px(40000.0)), true);
    contained_edit_stays_flat(None, true);
}

fn contained_edit_stays_flat(container_height: Option<LengthSpec>, parked_row: bool) {
    let hugging = container_height.is_none();
    let viewport = LayoutViewport::new(320.0, 4000.0);
    let shape = DiffShape {
        name: "column-plain",
        container: LayoutStyle {
            width: Some(LengthSpec::Px(300.0)),
            height: container_height,
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        },
        row: |_| LayoutStyle {
            width: Some(LengthSpec::Px(60.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
    };
    let mut emitted_by_rows = Vec::new();
    for rows in [64usize, 512] {
        let (mut world, document) = diff_tree(&shape, rows);
        if parked_row {
            let mut queue = MutationQueue::new();
            queue.park_subtree(id(3 + 2));
            world.commit(queue).unwrap();
        }
        let mut retained = RetainedLayoutCache::default();
        let _ = world.take_system_work();
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();

        // A full pass deliberately records no measure plans -- see the comment
        // at the recording site -- so the FIRST scoped pass after one measures
        // its children and builds them. That priming frame is O(document) by
        // construction; what this gate is about is the steady state after it.
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(4),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(17.0)),
                    height: Some(LengthSpec::Px(5.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            "priming pass",
        );

        let row = rows - 1;
        // 1. Contained: edit the LABEL inside the last row. The row's own size
        //    is fixed, so nothing above or below it can move.
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(4 + row as u64 * 2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(31.0)),
                    height: Some(LengthSpec::Px(9.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let contained = scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            "contained edit",
        );

        // 2. Resize the LAST row. Its size changes, so the container's cached
        //    placement is stale from that index on -- but there is no index
        //    after it, so the suffix is empty and the cost is still constant.
        let mut style = (shape.row)(row);
        style.height = Some(LengthSpec::Px(27.0));
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(3 + row as u64 * 2),
            NodeStyle {
                layout: Arc::new(style),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let resized =
            scoped_step_matches_full(&mut world, document, viewport, &mut retained, "tail resize");
        // Resizing the last row changes a HUGGING container's own height.
        // Placement replays the suffix, but the measure of that container
        // still walks its children. This gate does not treat that walk as
        // the flatness signal; it only holds the fixed-height container,
        // and the contained edit, to a document-independent cost.
        let measured = if hugging {
            contained.children_measured
        } else {
            contained.children_measured.max(resized.children_measured)
        };
        assert!(
            !hugging || resized.children_measured >= rows,
            "a hugging container that really did resize must re-measure its \
             children; if this stops being true the assertion above is \
             measuring nothing"
        );
        emitted_by_rows.push((rows, contained.emitted.max(resized.emitted), measured));
    }
    let (small_rows, small, small_measured) = emitted_by_rows[0];
    let (big_rows, big, big_measured) = emitted_by_rows[1];
    assert!(
        big <= small + 4,
        "a contained edit must emit a bounded set: {small_rows} rows emitted {small}, \
         {big_rows} rows emitted {big}"
    );
    // The point of the whole exercise: an 8x larger document must not make the
    // container re-measure 8x as many children.
    assert!(
        big_measured <= small_measured + 4,
        "a contained edit must not scale its sibling scan with the document: \
         {small_rows} rows measured {small_measured} children, \
         {big_rows} rows measured {big_measured}"
    );
}

/// Issue #258: one flex item's cross size stays on its line, so the other
/// lines are not measured and the pass does not fall back to the document.
#[test]
fn flex_line_intrinsic_change_does_not_measure_unaffected_lines() {
    let viewport = LayoutViewport::new(400.0, 800.0);
    let container = LayoutStyle {
        display: Some(DisplaySpec::Flex),
        direction: Some(FlexDirection::Row),
        flex_wrap: FlexWrap::Wrap,
        width: Some(LengthSpec::Px(200.0)),
        align_items: AlignSpec::Start,
        justify_content: JustifySpec::Start,
        ..LayoutStyle::default()
    };
    let item = LayoutStyle {
        width: Some(LengthSpec::Px(100.0)),
        height: Some(LengthSpec::Px(20.0)),
        ..LayoutStyle::default()
    };
    let mut measured = Vec::new();
    for lines in [2usize, 6] {
        let rows = (0..lines * 2)
            .map(|index| (10 + index as u64, item.clone(), Vec::new()))
            .collect::<Vec<_>>();
        let (mut world, document) = hugging_container_world(&rows, container.clone());
        let mut retained = RetainedLayoutCache::default();
        let _ = world.take_system_work();
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        prime_measure_plans(&mut world, document, viewport, &mut retained, id(10));

        let mut grown = item.clone();
        grown.height = Some(LengthSpec::Px(40.0));
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(10),
            NodeStyle {
                layout: Arc::new(grown),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let step = scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            "flex line cross size",
        );
        assert_eq!(
            step.full_document_fallbacks, 0,
            "{lines} lines fell back to the document"
        );
        let changed = world.layout_box(id(10)).expect("changed item");
        let next_line = world
            .layout_box(id(12))
            .expect("first item of the next line");
        assert_eq!(changed.height, 40.0);
        assert!(
            (next_line.y - 40.0).abs() < 0.01,
            "{lines} lines: the next line stayed at y {}",
            next_line.y
        );
        assert!(
            step.children_measured < lines * 2,
            "{lines} lines measured {} children, including an unaffected line",
            step.children_measured
        );
        measured.push(step.children_measured);
    }
    assert_eq!(
        measured[0], measured[1],
        "unaffected flex lines must not add measure work as lines are added: {measured:?}"
    );
}

/// Issue #258: one grid cell's intrinsic contribution is remeasured, and cells
/// whose tracks did not change are not. A cold layout of the same grid matches.
#[test]
fn grid_cell_intrinsic_change_does_not_measure_unaffected_cells() {
    let viewport = LayoutViewport::new(400.0, 800.0);
    let columns = 3usize;
    let container = LayoutStyle {
        display: Some(DisplaySpec::Grid),
        width: Some(LengthSpec::Px(180.0)),
        grid: nana_ui_core::GridStyle {
            columns: Some(vec![GridTrack::Auto, GridTrack::Auto, GridTrack::Auto]),
            ..Default::default()
        }
        .into(),
        align_items: AlignSpec::Start,
        justify_items: Some(AlignSpec::Start),
        justify_content: JustifySpec::Start,
        ..LayoutStyle::default()
    };
    let cell = LayoutStyle {
        width: Some(LengthSpec::Px(40.0)),
        height: Some(LengthSpec::Px(20.0)),
        ..LayoutStyle::default()
    };
    let mut measured = Vec::new();
    for rows in [2usize, 6] {
        let items = (0..rows * columns)
            .map(|index| (10 + index as u64, cell.clone(), Vec::new()))
            .collect::<Vec<_>>();
        let (mut world, document) = hugging_container_world(&items, container.clone());
        let mut retained = RetainedLayoutCache::default();
        let _ = world.take_system_work();
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        prime_measure_plans(&mut world, document, viewport, &mut retained, id(10));

        let mut grown = cell.clone();
        grown.height = Some(LengthSpec::Px(40.0));
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(10),
            NodeStyle {
                layout: Arc::new(grown),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let step = scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            "grid cell cross size",
        );
        assert_eq!(
            step.full_document_fallbacks, 0,
            "{rows} rows fell back to the document"
        );
        let changed = world.layout_box(id(10)).expect("changed cell");
        let next_row = world
            .layout_box(id(10 + columns as u64))
            .expect("first cell of the next row");
        assert_eq!(changed.height, 40.0);
        assert_eq!(next_row.height, 20.0);
        assert!(
            (next_row.y - 40.0).abs() < 0.01,
            "{rows} rows: the next row stayed at y {}",
            next_row.y
        );
        assert!(
            step.children_measured < rows * columns,
            "{rows} rows measured {} children, including an unaffected cell",
            step.children_measured
        );
        measured.push(step.children_measured);

        let mut painted = cell.clone();
        painted.opacity = Some(0.4);
        let mut paint = MutationQueue::new();
        paint.set_style(
            id(11),
            NodeStyle {
                layout: Arc::new(painted),
                ..NodeStyle::default()
            },
        );
        world.commit(paint).unwrap();
        let work = world.take_system_work();
        assert!(
            work.layout_frontier_seeds.is_empty(),
            "{rows} rows: paint-only on one cell scheduled layout"
        );
    }
    assert_eq!(
        measured[0], measured[1],
        "unaffected grid cells must not add measure work as rows are added: {measured:?}"
    );
}

/// Run the scoped passes that leave every plan recorded and current.
///
/// A full pass records no measure plans, so the first scoped pass after one is
/// the pass that BUILDS them -- it consults nothing and can be fooled by
/// nothing. Any test about what the plans do or refuse to do has to get past
/// that frame first, or it asserts about a code path it never reached.
///
/// The edit is applied and then reverted, so the tree ends exactly where it
/// started with the plans describing that state. Two passes are needed, not
/// one: the second pass rejects the plan the first recorded (that is what the
/// revert is) and records the one the test will meet.
///
/// Nothing here asserts that a plan came out -- an uncacheable container
/// correctly produces none. What proves the priming works is that the
/// deliberately broken implementations these tests exist to catch DO fail
/// them.
fn prime_measure_plans(
    world: &mut UiWorld,
    document: DocumentId,
    viewport: LayoutViewport,
    retained: &mut RetainedLayoutCache,
    victim: StableNodeId,
) {
    let original = world.node_style(victim).unwrap().layout.clone();
    let mut nudged = (*original).clone();
    nudged.margin_top = Some(LengthSpec::Px(
        match nudged.margin_top {
            Some(LengthSpec::Px(px)) => px,
            _ => 0.0,
        } + 3.0,
    ));
    for layout in [Arc::new(nudged), original] {
        let mut queue = MutationQueue::new();
        queue.set_style(
            victim,
            NodeStyle {
                layout,
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        scoped_step_matches_full(world, document, viewport, retained, "priming pass");
    }
}

/// Build a hugging column with `children` laid out under it, run one full pass
/// and return everything a scoped step needs.
fn hugging_container_world(
    rows: &[(u64, LayoutStyle, Vec<(u64, LayoutStyle)>)],
    container: LayoutStyle,
) -> (UiWorld, DocumentId) {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(1), id(2), None);
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(container),
            ..NodeStyle::default()
        },
    );
    for (row, style, children) in rows {
        queue.create(id(*row), document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(2), id(*row), None);
        queue.set_style(
            id(*row),
            NodeStyle {
                layout: Arc::new(style.clone()),
                ..NodeStyle::default()
            },
        );
        for (child, child_style) in children {
            queue.create(
                id(*child),
                document,
                NodeKind::Element { tag: "div".into() },
            );
            queue.insert(id(*row), id(*child), None);
            queue.set_style(
                id(*child),
                NodeStyle {
                    layout: Arc::new(child_style.clone()),
                    ..NodeStyle::default()
                },
            );
        }
    }
    world.commit(queue).unwrap();
    (world, document)
}

/// A row item measured again at the width its line gave it is re-checked at
/// that width by the cached plans: here an edit inside the item changes its
/// height only at its used width (its two boxes stop wrapping there), not
/// at the width it was first measured with.
#[test]
fn a_cached_row_rechecks_an_item_at_its_used_width() {
    let viewport = LayoutViewport::new(1280.0, 800.0);
    let row = LayoutStyle {
        direction: Some(FlexDirection::Row),
        width: Some(LengthSpec::Px(1152.0)),
        gap: Some(LengthSpec::Px(16.0)),
        align_items: AlignSpec::Start,
        ..LayoutStyle::default()
    };
    let primary = LayoutStyle {
        direction: Some(FlexDirection::Row),
        flex_wrap: FlexWrap::Wrap,
        width: Some(LengthSpec::Fill),
        min_width: Some(LengthSpec::Px(0.0)),
        flex_grow: Some(1.0),
        flex_shrink: Some(1.0),
        align_items: AlignSpec::Start,
        ..LayoutStyle::default()
    };
    let block = |width| LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(50.0)),
        ..LayoutStyle::default()
    };
    let side = LayoutStyle {
        width: Some(LengthSpec::Px(402.0)),
        height: Some(LengthSpec::Px(20.0)),
        flex_shrink: Some(0.0),
        ..LayoutStyle::default()
    };
    let (mut world, document) = hugging_container_world(
        &[
            (3, primary, vec![(10, block(400.0)), (11, block(400.0))]),
            (4, side, vec![]),
        ],
        row,
    );
    let mut retained = RetainedLayoutCache::default();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &emitted);
    let _ = world.take_system_work();
    prime_measure_plans(&mut world, document, viewport, &mut retained, id(4));
    assert_eq!(
        retained.documents[&document].page_boxes(&world)[&id(3)].height,
        100.0
    );

    let mut queue = MutationQueue::new();
    queue.set_style(
        id(11),
        NodeStyle {
            layout: Arc::new(block(300.0)),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    scoped_step_matches_full(
        &mut world,
        document,
        viewport,
        &mut retained,
        "an item's content stops wrapping at its used width",
    );
    assert_eq!(
        retained.documents[&document].page_boxes(&world)[&id(3)].height,
        50.0
    );
}

/// `MeasurePlan` records the flow direction its container handed each child as
/// that child's `parent_direction`, and a child's own measurement can depend on
/// it -- `aspect-ratio` stretch-fits the inline axis only when the parent is not
/// a row.
///
/// Flipping the container from column to row leaves every child's own style,
/// child list and available size untouched, so a plan that did not record the
/// direction would hand back the column measurement forever.
#[test]
fn flipping_a_container_to_a_row_remeasures_children_that_depend_on_the_direction() {
    let viewport = LayoutViewport::new(320.0, 600.0);
    let hug = |direction| LayoutStyle {
        width: Some(LengthSpec::Px(300.0)),
        height: None,
        direction: Some(direction),
        ..LayoutStyle::default()
    };
    // `aspect_ratio` with no width of its own: the used width comes from the
    // stretch fit, which is disabled under a row parent.
    let ratio_row = LayoutStyle {
        aspect_ratio: Some(2.0),
        ..LayoutStyle::default()
    };
    let (mut world, document) = hugging_container_world(
        &[
            (3, ratio_row.clone(), vec![]),
            (4, ratio_row.clone(), vec![]),
            (5, ratio_row, vec![]),
        ],
        hug(FlexDirection::Column),
    );
    let mut retained = RetainedLayoutCache::default();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &emitted);
    let _ = world.take_system_work();
    prime_measure_plans(&mut world, document, viewport, &mut retained, id(5));
    let column_box = retained.documents[&document].page_boxes(&world)[&id(3)];

    let mut queue = MutationQueue::new();
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(hug(FlexDirection::Row)),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    scoped_step_matches_full(
        &mut world,
        document,
        viewport,
        &mut retained,
        "container direction flipped to row",
    );
    assert_ne!(
        retained.documents[&document].page_boxes(&world)[&id(3)],
        column_box,
        "the child's measurement must actually depend on the parent direction, \
         or this test cannot detect a plan that ignores it"
    );
}

/// A container's own shaped text competes with its children for the content
/// size, and `set_text` moves that text without touching the container's style,
/// its child list, or any child.
///
/// So the container's own text metrics are a plan input in their own right. A
/// plan that recorded only style and children would hand back the height the
/// old string produced, and every child under it would keep a stale position.
#[test]
fn a_container_whose_own_text_grows_remeasures_itself() {
    struct Shaper;
    impl TextShaper for Shaper {
        fn shape(
            &mut self,
            _id: StableNodeId,
            text: &TextContent,
            _style: &ComputedStyle,
            _constraints: crate::TextShapeConstraints,
        ) -> TextMetrics {
            TextMetrics {
                width: text.value.len() as f32 * 8.0,
                height: 40.0 * (1 + text.value.matches('\n').count()) as f32,
                ascent: None,
            }
        }
    }
    let shape = |world: &mut UiWorld| {
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        world.shape_text(&work.text, &mut Shaper).unwrap();
    };

    let viewport = LayoutViewport::new(320.0, 600.0);
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    // The container hugs, carries text of its own, AND has children. The text
    // is taller than the stacked children, so it is the text that decides the
    // container's height.
    queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(1), id(2), None);
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(300.0)),
                height: None,
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.set_text(
        id(2),
        TextContent {
            value: "one".into(),
        },
    );
    for row in 0..4u64 {
        queue.create(
            id(3 + row),
            document,
            NodeKind::Element { tag: "div".into() },
        );
        queue.insert(id(2), id(3 + row), None);
        queue.set_style(
            id(3 + row),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(60.0)),
                    height: Some(LengthSpec::Px(10.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    world.commit(queue).unwrap();
    shape(&mut world);

    let mut retained = RetainedLayoutCache::default();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &emitted);
    shape(&mut world);
    let _ = world.take_system_work();
    prime_measure_plans(&mut world, document, viewport, &mut retained, id(3));
    let short = retained.documents[&document].page_boxes(&world)[&id(2)];

    let before = world.node_style(id(2)).unwrap().layout.clone();
    let mut queue = MutationQueue::new();
    queue.set_text(
        id(2),
        TextContent {
            value: "one\ntwo\nthree".into(),
        },
    );
    world.commit(queue).unwrap();
    shape(&mut world);
    assert!(
        Arc::ptr_eq(&before, &world.node_style(id(2)).unwrap().layout),
        "the container's own style must be untouched, or this test is not \
         exercising the text-metrics input"
    );
    scoped_step_matches_full(
        &mut world,
        document,
        viewport,
        &mut retained,
        "the container's own text grew",
    );
    assert_ne!(
        retained.documents[&document].page_boxes(&world)[&id(2)].height,
        short.height,
        "the container's own text must decide its height, or this test cannot \
         detect a plan that ignores it"
    );
}

/// Two spellings of the same layout must not disturb a plan; two different
/// layouts must.
///
/// `direction: None` and `direction: Some(Column)` are the same axis --
/// `used_flow_direction` is `unwrap_or(Column)` -- and in a real host they
/// alternate on the same node from one frame to the next, because more than one
/// writer seeds the field. A plan compared with `==` is retired every frame by
/// that alternation while nothing about the layout moved. `Some(Row)` is a
/// different layout: the plan must see it and replay from that child.
#[test]
fn a_default_spelled_two_ways_keeps_the_plan_but_a_real_direction_change_replays_it() {
    let viewport = LayoutViewport::new(320.0, 600.0);
    let leaf = |direction| LayoutStyle {
        width: Some(LengthSpec::Px(60.0)),
        height: Some(LengthSpec::Px(20.0)),
        direction,
        ..LayoutStyle::default()
    };
    // Hugging container, so it is the measure plan that has to hold.
    let hug = LayoutStyle {
        width: Some(LengthSpec::Px(300.0)),
        height: None,
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    };
    let rows: Vec<(u64, LayoutStyle, Vec<(u64, LayoutStyle)>)> =
        (3..=10).map(|row| (row, leaf(None), vec![])).collect();

    const ROWS: usize = 8;
    for (spelling, expect_replay) in [
        // Same axis, different spelling: the plan must survive, so the
        // container must not touch its children.
        (Some(FlexDirection::Column), false),
        // A real change: the plan sees it and replays from the edited child.
        (Some(FlexDirection::Row), true),
    ] {
        let (mut world, document) = hugging_container_world(&rows, hug.clone());
        let mut retained = RetainedLayoutCache::default();
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        prime_measure_plans(&mut world, document, viewport, &mut retained, id(10));

        let mut queue = MutationQueue::new();
        queue.set_style(
            id(4),
            NodeStyle {
                layout: Arc::new(leaf(spelling)),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let step = scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            &format!("child direction respelled as {spelling:?}"),
        );
        // The global "plans reused" counter is too coarse -- other nodes reuse
        // their own plans either way. What this test is about is whether THIS
        // container saw the edit: it replays from the edited child, and
        // re-measures that child alone rather than walking every sibling.
        assert!(
            step.children_measured < ROWS,
            "direction {spelling:?}: children measured {}",
            step.children_measured
        );
        assert_eq!(
            step.suffixes_replayed > 0,
            expect_replay,
            "direction {spelling:?}: expected a replay from the edited child: {expect_replay}"
        );
    }
}

#[test]
fn scoped_layout_reveals_hidden_branch_with_new_positioned_children() {
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(320.0, 620.0);
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    for node in 1..=3 {
        queue.create(id(node), document, NodeKind::Element { tag: "div".into() });
    }
    queue.insert(id(1), id(2), None);
    queue.insert(id(1), id(3), None);
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(320.0)),
                height: Some(LengthSpec::Px(620.0)),
                direction: Some(FlexDirection::Column),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                height: Some(LengthSpec::Px(30.0)),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    // The revealed absolute child's containing block.
    let branch = LayoutStyle {
        position: PositionSpec::Relative,
        width: Some(LengthSpec::Fill),
        height: Some(LengthSpec::Px(300.0)),
        flex_shrink: Some(0.0),
        ..Default::default()
    };
    queue.set_style(
        id(3),
        NodeStyle {
            layout: Arc::new(branch.clone()),
            ..Default::default()
        },
    );
    world.commit(queue).unwrap();
    let mut retained = RetainedLayoutCache::default();
    let full = RuntimeLayoutEngine
        .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    write_changed_boxes(&mut world, &full);
    let _ = world.take_system_work();
    let mut hidden = branch.clone();
    hidden.hidden = true;
    let mut queue = MutationQueue::new();
    queue.set_style(
        id(3),
        NodeStyle {
            layout: Arc::new(hidden),
            ..Default::default()
        },
    );
    world.commit(queue).unwrap();
    let work = world.take_system_work();
    let hidden_boxes = RuntimeLayoutEngine
        .layout_document_with_frontier(
            &world,
            document,
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            false,
        )
        .unwrap();
    write_changed_boxes(&mut world, &hidden_boxes);
    let _ = world.take_system_work();
    // An omitted box has no meaningful origin; only its zero extent matters.
    let hidden_box = world.layout_box(id(3)).unwrap();
    assert_eq!((hidden_box.width, hidden_box.height), (0.0, 0.0));
    let mut queue = MutationQueue::new();
    queue.create(id(4), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(3), id(4), None);
    queue.set_style(
        id(4),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                position: PositionSpec::Absolute,
                offset_top: Some(LengthSpec::Px(104.0)),
                width: Some(LengthSpec::Percent(100.0)),
                height: Some(LengthSpec::Px(104.0)),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    queue.set_style(
        id(3),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                height: Some(LengthSpec::Px(208.0)),
                ..branch
            }),
            ..Default::default()
        },
    );
    world.commit(queue).unwrap();
    scoped_step_matches_full(
        &mut world,
        document,
        viewport,
        &mut retained,
        "reveal with changed content",
    );
    assert_eq!(world.layout_box(id(4)).unwrap().height, 104.0);
    assert_eq!(world.layout_box(id(4)).unwrap().y, 134.0);
    assert_eq!(world.layout_box(id(3)).unwrap().height, 208.0);
}

#[cfg(feature = "rich-text")]
#[test]
fn constrained_auto_height_contains_wrapped_markdown_and_flow_actions() {
    use crate::{AppContext, NativeMarkdown, Stack};
    for (maximum, sizing, declared, minimum) in [
        (
            LengthSpec::Percent(76.0),
            BoxSizing::BorderBox,
            LengthSpec::FitContent,
            None,
        ),
        (
            LengthSpec::min2(
                nana_ui_core::LengthAtom::Percent(76.0),
                nana_ui_core::LengthAtom::Px(620.0),
            ),
            BoxSizing::BorderBox,
            LengthSpec::FitContent,
            None,
        ),
        (
            LengthSpec::Px(620.0),
            BoxSizing::ContentBox,
            LengthSpec::Px(50.0),
            Some(LengthSpec::Px(100.0)),
        ),
        (
            LengthSpec::Px(620.0),
            BoxSizing::BorderBox,
            LengthSpec::Px(50.0),
            Some(LengthSpec::Px(100.0)),
        ),
    ] {
        let document = DocumentId::new(601).unwrap();
        let mut context = AppContext::new();
        let root = context
            .create_component(document, Stack::fill_column(0.0))
            .unwrap();
        let bubble = context
            .create_detached_component(
                document,
                Stack::column(6.0)
                    .width(declared)
                    .padding_xy(14.0, 10.0)
                    .with_layout(|layout| {
                        layout.max_width = Some(maximum);
                        layout.min_width = minimum;
                        layout.box_sizing = sizing;
                    }),
            )
            .unwrap();
        let markdown = context
            .create_detached_component(
                document,
                NativeMarkdown::parse(&"换行内容需要完整参与父容器的自然高度。".repeat(32)),
            )
            .unwrap();
        context.assemble_markdown(markdown).unwrap();
        let actions = context
            .create_detached_component(document, Stack::row(0.0).height(LengthSpec::Px(28.0)))
            .unwrap();
        let following = context
            .create_detached_component(document, Stack::row(0.0).height(LengthSpec::Px(22.0)))
            .unwrap();
        context.append_child(root, bubble).unwrap();
        context.append_child(bubble, markdown).unwrap();
        context.append_child(bubble, actions).unwrap();
        context.append_child(root, following).unwrap();
        for width in [1000.0, 640.0, 400.0, 1000.0] {
            context
                .layout_document(document, LayoutViewport::new(width, 1200.0))
                .unwrap();
            let world = context.world();
            let parent = world.layout_box(bubble.stable_id()).unwrap();
            if minimum.is_some() {
                let expected = if sizing == BoxSizing::ContentBox {
                    128.0
                } else {
                    100.0
                };
                assert_eq!(parent.width, expected);
            }
            let text = world.layout_box(markdown.stable_id()).unwrap();
            let action = world.layout_box(actions.stable_id()).unwrap();
            let next = world.layout_box(following.stable_id()).unwrap();
            assert!(
                action.y >= text.y + text.height,
                "{parent:?} {text:?} {action:?}"
            );
            assert!(
                action.y + action.height + 10.0 <= parent.y + parent.height + 0.1,
                "flow content and padding must fit at {width}: {parent:?} {text:?} {action:?}"
            );
            assert!(
                next.y >= parent.y + parent.height,
                "following row must start after the complete bubble"
            );
        }
    }
}

/// `writing-mode` and `direction` inherit (CSS Writing Modes §3): a container
/// that declares neither lays out in its parent's. A row inside a
/// `vertical-rl` box runs down the page.
#[test]
fn a_container_inherits_its_parents_writing_mode() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(200.0)),
            writing_mode: Some(WritingModeSpec::VerticalRl),
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "inner".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                width: Some(LengthSpec::Px(100.0)),
                height: Some(LengthSpec::Px(100.0)),
                align_items: AlignSpec::Start,
                ..LayoutStyle::default()
            },
            children: vec![px_box("a", 20.0, 20.0), px_box("b", 20.0, 20.0)],
            text: None,
        }],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 200.0);
    let inner = boxes["inner"];
    // Down the inner box's inline axis, against its right (block-start) edge.
    for (id, y) in [("a", 0.0), ("b", 20.0)] {
        assert!(
            (boxes[id].y - (inner.y + y)).abs() < 0.5
                && (boxes[id].x - (inner.x + inner.width - 20.0)).abs() < 0.5,
            "{id} at y={y} against the right edge of {inner:?}, got {:?}",
            boxes[id]
        );
    }
}

/// `text-orientation: upright` makes a vertical box's used direction `ltr`
/// (CSS Writing Modes §5.1): under a `vertical-rl; direction: rtl` parent that
/// sets it, a row starts at the top again instead of the bottom -- inherited,
/// like the writing mode itself.
#[test]
fn an_upright_vertical_box_lays_out_left_to_right() {
    let layout = |orientation| {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: None,
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(200.0)),
                writing_mode: Some(WritingModeSpec::VerticalRl),
                dir: Some(DirSpec::Rtl),
                text_ext: nana_ui_core::TextStyleExt {
                    text_orientation: orientation,
                    ..Default::default()
                }
                .into(),
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "row".into(),
                style: LayoutStyle {
                    display: Some(DisplaySpec::Flex),
                    direction: Some(FlexDirection::Row),
                    width: Some(LengthSpec::Px(40.0)),
                    height: Some(LengthSpec::Px(100.0)),
                    align_items: AlignSpec::Start,
                    ..LayoutStyle::default()
                },
                children: vec![px_box("a", 20.0, 20.0)],
                text: None,
            }],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 200.0);
        boxes["a"].y - boxes["row"].y
    };
    assert!(
        (layout(None) - 80.0).abs() < 0.5,
        "rtl starts at the bottom"
    );
    assert!(
        layout(Some(TextOrientationSpec::Upright)).abs() < 0.5,
        "upright reads ltr: the row starts at the top"
    );
}

/// A box's logical edges land in the writing context it inherits, not only in
/// the one it declares: `padding-inline-start` on a box inside an RTL parent
/// pads its right edge, even though the box was built (and its edges first
/// resolved) without a `direction` of its own.
#[test]
fn logical_edges_land_in_the_inherited_direction() {
    let mut inner = LayoutStyle {
        display: Some(DisplaySpec::Flex),
        direction: Some(FlexDirection::Row),
        width: Some(LengthSpec::Px(100.0)),
        height: Some(LengthSpec::Px(40.0)),
        align_items: AlignSpec::Start,
        ..LayoutStyle::default()
    };
    inner
        .logical_edges
        .padding
        .set_start(Some(LengthSpec::Px(30.0)));
    inner.bake_logical_edges();
    assert_eq!(inner.padding_left, Some(LengthSpec::Px(30.0)));
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(100.0)),
            dir: Some(DirSpec::Rtl),
            align_items: AlignSpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "inner".into(),
            style: inner,
            children: vec![px_box("a", 20.0, 20.0)],
            text: None,
        }],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 100.0);
    let inner = boxes["inner"];
    let a = boxes["a"];
    assert!(
        (a.x - (inner.x + inner.width - 30.0 - 20.0)).abs() < 0.5,
        "a starts 30px in from the right edge of {inner:?}, got {a:?}"
    );
}

/// Inheriting a writing mode and direction is the same as declaring them: a
/// container of every kind -- an IFC, a wrapping flex row, a grid -- lays out
/// identically whether it says `writing-mode` / `direction` itself or gets
/// them from its parent.
#[test]
fn an_inherited_writing_context_lays_out_like_a_declared_one() {
    let inline_block = |id: String| StyleLayoutNode {
        id,
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineFlex),
            width: Some(LengthSpec::Px(30.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let containers: [(&str, LayoutStyle); 3] = [
        ("ifc", LayoutStyle::default()),
        (
            "flex-wrap",
            LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignSpec::Start,
                ..LayoutStyle::default()
            },
        ),
        (
            "grid",
            LayoutStyle {
                display: Some(DisplaySpec::Grid),
                grid: nana_ui_core::GridStyle {
                    columns: Some(vec![GridTrack::Px(40.0), GridTrack::Px(40.0)]),
                    ..Default::default()
                }
                .into(),
                ..LayoutStyle::default()
            },
        ),
    ];
    for mode in [
        WritingModeSpec::HorizontalTb,
        WritingModeSpec::VerticalRl,
        WritingModeSpec::VerticalLr,
    ] {
        for dir in [DirSpec::Ltr, DirSpec::Rtl] {
            for (kind, container) in &containers {
                let layout = |declared: bool| {
                    let mut inner = container.clone();
                    inner.width = Some(LengthSpec::Px(100.0));
                    inner.height = Some(LengthSpec::Px(100.0));
                    if declared {
                        inner.writing_mode = Some(mode);
                        inner.dir = Some(dir);
                    }
                    let tree = StyleLayoutNode {
                        id: "root".into(),
                        style: LayoutStyle {
                            display: None,
                            width: Some(LengthSpec::Px(200.0)),
                            height: Some(LengthSpec::Px(200.0)),
                            writing_mode: Some(mode),
                            dir: Some(dir),
                            ..LayoutStyle::default()
                        },
                        children: vec![StyleLayoutNode {
                            id: "inner".into(),
                            style: inner,
                            children: (0..5).map(|i| inline_block(format!("c{i}"))).collect(),
                            text: None,
                        }],
                        text: None,
                    };
                    box_map(&tree, 200.0, 200.0)
                };
                let (declared, inherited) = (layout(true), layout(false));
                for (id, want) in &declared {
                    assert_eq!(
                        inherited.get(id),
                        Some(want),
                        "{kind} in {mode:?} {dir:?}: {id} inherited vs declared"
                    );
                }
            }
        }
    }
}

/// Percentage margins resolve against the containing block's inline size in
/// the containing block's own writing mode (CSS Writing Modes §7.3). A box
/// that sets a writing mode orthogonal to its parent's still resolves against
/// the parent's inline axis.
#[test]
fn an_orthogonal_box_resolves_percentage_margins_on_its_parents_inline_axis() {
    let case = |parent_mode, child_mode, width: f32, height: f32| {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Column),
                width: Some(LengthSpec::Px(width)),
                height: Some(LengthSpec::Px(height)),
                writing_mode: Some(parent_mode),
                align_items: AlignSpec::Start,
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "a".into(),
                style: LayoutStyle {
                    width: Some(LengthSpec::Px(20.0)),
                    height: Some(LengthSpec::Px(20.0)),
                    margin_top: Some(LengthSpec::Percent(10.0)),
                    writing_mode: Some(child_mode),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            }],
            text: None,
        };
        box_map(&tree, width, height)["a"].y
    };
    // Horizontal parent 200 wide: 10% of its width, whatever the child sets.
    let vertical_child = case(
        WritingModeSpec::HorizontalTb,
        WritingModeSpec::VerticalRl,
        200.0,
        100.0,
    );
    assert!((vertical_child - 20.0).abs() < 0.5, "got {vertical_child}");
    // Vertical parent 200 tall: 10% of its height.
    let horizontal_child = case(
        WritingModeSpec::VerticalRl,
        WritingModeSpec::HorizontalTb,
        100.0,
        200.0,
    );
    assert!(
        (horizontal_child - 20.0).abs() < 0.5,
        "got {horizontal_child}"
    );
}

/// A flex item restyled beyond its cross size in the same edit (a margin,
/// `align-self`, `flex-grow`) is laid out with its new style: the line-local
/// replay only covers a cross-size change.
#[test]
fn flex_line_replay_takes_every_change_an_item_brings() {
    let viewport = LayoutViewport::new(400.0, 800.0);
    let container = LayoutStyle {
        display: Some(DisplaySpec::Flex),
        direction: Some(FlexDirection::Row),
        flex_wrap: FlexWrap::Wrap,
        width: Some(LengthSpec::Px(240.0)),
        align_items: AlignSpec::Start,
        justify_content: JustifySpec::Start,
        ..LayoutStyle::default()
    };
    let item = LayoutStyle {
        width: Some(LengthSpec::Px(100.0)),
        height: Some(LengthSpec::Px(20.0)),
        ..LayoutStyle::default()
    };
    let edits: [fn(&mut LayoutStyle); 3] = [
        |style| style.margin_left = Some(LengthSpec::Px(12.0)),
        |style| style.align_self = Some(AlignSpec::End),
        |style| style.margin_top = Some(LengthSpec::Px(6.0)),
    ];
    for edit in edits {
        let rows = (0..4)
            .map(|index| (10 + index as u64, item.clone(), Vec::new()))
            .collect::<Vec<_>>();
        let (mut world, document) = hugging_container_world(&rows, container.clone());
        let mut retained = RetainedLayoutCache::default();
        let _ = world.take_system_work();
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        prime_measure_plans(&mut world, document, viewport, &mut retained, id(10));
        let mut changed = item.clone();
        changed.height = Some(LengthSpec::Px(40.0));
        edit(&mut changed);
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(11),
            NodeStyle {
                layout: Arc::new(changed),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        scoped_step_matches_full(
            &mut world,
            document,
            viewport,
            &mut retained,
            "restyled item",
        );
    }
}

#[test]
fn issue199_inline_span_keeps_atomic_descendants_and_unboxes_only_real_blocks() {
    let atomic = StyleLayoutNode {
        id: "atom".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineFlex),
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..Default::default()
        },
        children: Vec::new(),
        text: None,
    };
    let span = |id: &str, child| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Inline),
            ..Default::default()
        },
        children: vec![child],
        text: None,
    };
    let root = |child| StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: None,
            width: Some(LengthSpec::Px(200.0)),
            ..Default::default()
        },
        children: vec![child],
        text: None,
    };
    let boxes = box_map(&root(span("span", atomic.clone())), 200.0, 100.0);
    assert_eq!(
        boxes["span"].width, 40.0,
        "an atomic inline child is not a block to hoist"
    );
    let mut block = atomic;
    block.id = "block".into();
    block.style.display = Some(DisplaySpec::Flex);
    let boxes = box_map(&root(span("outer", span("inner", block))), 200.0, 100.0);
    assert_eq!(boxes["block"].width, 40.0);
    assert_eq!(
        boxes["root"].height, 20.0,
        "a block nested through spans remains in its outer flow"
    );
}

/// A pass's table of inputs holds one slot per node it reads, from the
/// lowest to the highest id it met, and a scoped pass over rows spread
/// across a page fills it to the page's highest id every frame. 56 bytes a
/// slot keep the table of a 62 001-node page in one allocation size class
/// below the one the system maps afresh (and faults in) on every frame.
#[test]
fn a_pass_input_slot_stays_small() {
    assert!(std::mem::size_of::<Option<(StableNodeId, PassInput<'static>)>>() <= 56);
}

/// Under an unset `display`, a block-level child whose width is `auto` fills
/// the containing block less its margins, even when `align_items` is left at
/// `Start`. A flex column aligns by `align_items` and keeps the child at its
/// content width.
#[test]
fn block_children_with_auto_width_fill_the_containing_block() {
    let parent = |id: &str, display: Option<DisplaySpec>| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display,
            direction: Some(FlexDirection::Column),
            width: Some(LengthSpec::Px(400.0)),
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: format!("{id}-child"),
            style: LayoutStyle {
                display: None,
                margin_left: Some(LengthSpec::Px(10.0)),
                margin_right: Some(LengthSpec::Px(30.0)),
                ..LayoutStyle::default()
            },
            children: vec![px_box(&format!("{id}-content"), 50.0, 10.0)],
            text: None,
        }],
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        },
        children: vec![
            parent("unset", None),
            parent("flex", Some(DisplaySpec::Flex)),
        ],
        text: None,
    };
    let boxes = box_map(&tree, 800.0, 600.0);
    let child = boxes["unset-child"];
    assert!(
        (child.x - 10.0).abs() < 0.01 && (child.width - 360.0).abs() < 0.01,
        "unset: an auto-width block fills 400 less its margins, got {child:?}"
    );
    let child = boxes["flex-child"];
    assert!(
        (child.width - 50.0).abs() < 0.01,
        "a flex column aligned to the start keeps its child at its content width, got {child:?}"
    );
}

#[test]
fn subgrid_inherits_parent_column_and_row_tracks_gaps_and_nested_levels() {
    let cell = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Fill),
            height: Some(LengthSpec::Fill),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let sub = StyleLayoutNode {
        id: "sub".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            align_items: AlignSpec::Stretch,
            grid: nana_ui_core::GridStyle {
                columns_subgrid: true,
                placement: GridPlacement {
                    column_start: GridLine::Index(1),
                    column_end: GridLine::Index(-1),
                    ..GridPlacement::default()
                }
                .into(),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: vec![cell("a"), cell("b")],
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(210.0)),
            height: Some(LengthSpec::Px(40.0)),
            column_gap: Some(LengthSpec::Px(10.0)),
            align_items: AlignSpec::Stretch,
            grid: nana_ui_core::GridStyle {
                columns: Some(vec![GridTrack::Px(80.0), GridTrack::Px(120.0)]),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: vec![sub],
        text: None,
    };
    let boxes = box_map(&tree, 210.0, 40.0);
    assert!(
        (boxes["a"].width - 80.0).abs() < 0.5 && (boxes["a"].x - 0.0).abs() < 0.5,
        "subgrid col 1 must inherit 80px, not split evenly, got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["b"].width - 120.0).abs() < 0.5 && (boxes["b"].x - 90.0).abs() < 0.5,
        "subgrid col 2 must inherit 120px + 10px parent column-gap, got {:?}",
        boxes["b"]
    );

    // 2D + nested subgrid: grand-subgrid inherits both axes and row-gap.
    let inner_sub = StyleLayoutNode {
        id: "inner-sub".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            align_items: AlignSpec::Stretch,
            grid: nana_ui_core::GridStyle {
                columns_subgrid: true,
                rows_subgrid: true,
                placement: GridPlacement {
                    column_start: GridLine::Index(1),
                    column_end: GridLine::Index(-1),
                    row_start: GridLine::Index(1),
                    row_end: GridLine::Index(-1),
                    ..GridPlacement::default()
                }
                .into(),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: vec![cell("c1"), cell("c2"), cell("c3"), cell("c4")],
        text: None,
    };
    let outer_sub = StyleLayoutNode {
        id: "outer-sub".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            align_items: AlignSpec::Stretch,
            grid: nana_ui_core::GridStyle {
                columns_subgrid: true,
                rows_subgrid: true,
                placement: GridPlacement {
                    column_start: GridLine::Index(2),
                    column_end: GridLine::Index(4),
                    row_start: GridLine::Index(1),
                    row_end: GridLine::Index(3),
                    ..GridPlacement::default()
                }
                .into(),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: vec![inner_sub],
        text: None,
    };
    let nested_tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(300.0)),
            height: Some(LengthSpec::Px(120.0)),
            column_gap: Some(LengthSpec::Px(10.0)),
            row_gap: Some(LengthSpec::Px(8.0)),
            align_items: AlignSpec::Stretch,
            grid: nana_ui_core::GridStyle {
                columns: Some(vec![
                    GridTrack::Px(40.0),
                    GridTrack::Px(70.0),
                    GridTrack::Px(170.0),
                ]),
                rows: Some(vec![GridTrack::Px(30.0), GridTrack::Px(82.0)]),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: vec![outer_sub],
        text: None,
    };
    let nboxes = box_map(&nested_tree, 300.0, 120.0);
    assert!(
        (nboxes["c1"].x - 50.0).abs() < 0.5
            && (nboxes["c1"].y - 0.0).abs() < 0.5
            && (nboxes["c1"].width - 70.0).abs() < 0.5
            && (nboxes["c1"].height - 30.0).abs() < 0.5,
        "c1 = {:?}",
        nboxes["c1"]
    );
    assert!(
        (nboxes["c4"].x - 130.0).abs() < 0.5
            && (nboxes["c4"].y - 38.0).abs() < 0.5
            && (nboxes["c4"].width - 170.0).abs() < 0.5
            && (nboxes["c4"].height - 82.0).abs() < 0.5,
        "c4 = {:?}",
        nboxes["c4"]
    );

    // Standalone subgrid without a parent grid computes `subgrid` to `none`.
    let orphan = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(60.0)),
            grid: nana_ui_core::GridStyle {
                columns_subgrid: true,
                rows_subgrid: true,
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: vec![cell("only")],
        text: None,
    };
    let oboxes = box_map(&orphan, 200.0, 60.0);
    assert!(
        (oboxes["only"].width - 200.0).abs() < 0.5 && (oboxes["only"].height - 60.0).abs() < 0.5,
        "orphan = {:?}",
        oboxes["only"]
    );

    // 1-value `gap` shorthand on a subgrid overrides `parent_gap` rather than
    // being clobbered because `column_gap` / `row_gap` remain `None`.
    let custom_gap_sub = StyleLayoutNode {
        id: "sub-gap".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            gap: Some(LengthSpec::Px(4.0)),
            align_items: AlignSpec::Stretch,
            grid: nana_ui_core::GridStyle {
                columns_subgrid: true,
                placement: GridPlacement {
                    column_start: GridLine::Index(1),
                    column_end: GridLine::Index(-1),
                    ..GridPlacement::default()
                }
                .into(),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: vec![cell("ga"), cell("gb")],
        text: None,
    };
    let custom_gap_tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(216.0)),
            height: Some(LengthSpec::Px(40.0)),
            gap: Some(LengthSpec::Px(16.0)),
            align_items: AlignSpec::Stretch,
            grid: nana_ui_core::GridStyle {
                columns: Some(vec![GridTrack::Px(80.0), GridTrack::Px(120.0)]),
                ..Default::default()
            }
            .into(),
            ..LayoutStyle::default()
        },
        children: vec![custom_gap_sub],
        text: None,
    };
    let gboxes = box_map(&custom_gap_tree, 216.0, 40.0);
    let inner_gap = gboxes["gb"].x - (gboxes["ga"].x + gboxes["ga"].width);
    assert!(
        (inner_gap - 4.0).abs() < 0.5,
        "subgrid 1-value gap=4px must separate items by 4px (not parent 16px), got gap={inner_gap} ({:?}, {:?})",
        gboxes["ga"],
        gboxes["gb"]
    );
    assert!(
        (gboxes["ga"].width - 86.0).abs() < 0.5 && (gboxes["gb"].width - 126.0).abs() < 0.5,
        "tracks must absorb half the (16 - 4) gap delta each: ga={:?}, gb={:?}",
        gboxes["ga"],
        gboxes["gb"]
    );
}

#[test]
fn fit_content_row_respects_non_shrinkable_min_content_and_clamps_when_wrapping() {
    let fixed_child = |name: &str| StyleLayoutNode {
        id: name.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(60.0)),
            height: Some(LengthSpec::Px(40.0)),
            flex_shrink: Some(0.0),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let nowrap_root = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Column),
            align_items: AlignSpec::Start,
            width: Some(LengthSpec::Px(80.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "box".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                width: Some(LengthSpec::FitContent),
                height: Some(LengthSpec::Px(40.0)),
                flex_shrink: Some(0.0),
                ..LayoutStyle::default()
            },
            children: vec![fixed_child("a"), fixed_child("b")],
            text: None,
        }],
        text: None,
    };
    let nowrap_boxes = box_map(&nowrap_root, 80.0, 80.0);
    assert!(
        (nowrap_boxes["box"].width - 120.0).abs() < 0.5,
        "nowrap fit-content row with two non-shrinkable 60px children must be 120px, got {:?}",
        nowrap_boxes["box"]
    );

    let wrap_root = StyleLayoutNode {
        id: "root".into(),
        style: nowrap_root.style.clone(),
        children: vec![StyleLayoutNode {
            id: "box".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                flex_wrap: nana_ui_core::FlexWrap::Wrap,
                width: Some(LengthSpec::FitContent),
                height: Some(LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            },
            children: vec![fixed_child("a"), fixed_child("b")],
            text: None,
        }],
        text: None,
    };
    let wrap_boxes = box_map(&wrap_root, 80.0, 80.0);
    assert!(
        (wrap_boxes["box"].width - 60.0).abs() < 0.5,
        "wrapping fit-content row with two 60px children wraps to 60px lines, got {:?}",
        wrap_boxes["box"]
    );

    let shrink_text_root = StyleLayoutNode {
        id: "root".into(),
        style: nowrap_root.style,
        children: vec![StyleLayoutNode {
            id: "box".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                width: Some(LengthSpec::FitContent),
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "copy".into(),
                style: LayoutStyle {
                    flex_shrink: Some(1.0),
                    min_width: Some(LengthSpec::Px(0.0)),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: Some("A very long single line label that must wrap within 80px".into()),
            }],
            text: None,
        }],
        text: None,
    };
    let shrink_boxes = box_map(&shrink_text_root, 80.0, 80.0);
    assert!(
        shrink_boxes["box"].width <= 80.5,
        "nowrap fit-content row with shrinkable copy must clamp within 80px, got {:?}",
        shrink_boxes["box"]
    );
}

/// Restyles the container of [`column_tree`]: sized by its rows, and laid
/// out by `edit` beside that.
fn restyle_column(world: &mut UiWorld, edit: impl FnOnce(&mut LayoutStyle)) {
    let mut style = LayoutStyle {
        width: Some(LengthSpec::Px(300.0)),
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    };
    edit(&mut style);
    let mut queue = MutationQueue::new();
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(style),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
}

/// What a plan records of its children that a later pass places from.
fn plan_rows(plan: &ContainerPlan) -> (Vec<(StableNodeId, Point, Size, f32)>, f32) {
    let rows = plan
        .entries
        .iter()
        .map(|entry| (entry.child, entry.origin, entry.size, entry.cursor_before))
        .collect();
    (rows, plan.placed_main)
}

/// A pass reads the previous pass's plans and writes only its own tables
/// (#399): a replay that keeps a container's plan records what it moved as
/// a patch the pass commits. After each edit the retained plan is the one a
/// cold layout records, on a forward axis and on a reversed one, where the
/// replay also moves the prefix with the far edge.
#[test]
fn a_replayed_plan_commits_what_a_cold_layout_records() {
    const ROWS: u64 = 12;
    let viewport = LayoutViewport::new(300.0, 800.0);
    for reversed in [false, true] {
        let (mut world, document) = column_tree(ROWS);
        restyle_column(&mut world, |style| style.flex_reverse = reversed);
        let mut retained = RetainedLayoutCache::default();
        let _ = world.take_system_work();
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        for (step, (row, height)) in [(5, 32.0), (9, 12.0), (5, 20.0)].into_iter().enumerate() {
            resize_row(&mut world, row, height);
            let step = scoped_step_matches_full(
                &mut world,
                document,
                viewport,
                &mut retained,
                &format!("reversed {reversed} step {step}"),
            );
            assert!(step.suffixes_replayed > 0, "reversed {reversed}: no replay");
            let mut cold = RetainedLayoutCache::default();
            RuntimeLayoutEngine
                .layout_document_with_frontier(&world, document, viewport, &[], &mut cold, true)
                .unwrap();
            assert_eq!(
                plan_rows(&retained.documents[&document].container_plans[&id(2)]),
                plan_rows(&cold.documents[&document].container_plans[&id(2)]),
                "reversed {reversed}: the committed plan is not the cold one"
            );
        }
    }
}

/// A scroll container and a box that isolates its layout are cut from the
/// pass's placement as boundaries above the span, and an edit inside one
/// still lays out as a cold layout does.
#[test]
fn scroll_containers_and_isolated_boxes_are_natural_boundaries() {
    let viewport = LayoutViewport::new(300.0, 800.0);
    let shapes: [(&str, fn(&mut LayoutStyle)); 3] = [
        ("plain", |_| {}),
        ("scroll", |style| {
            style.height = Some(LengthSpec::Px(200.0));
            style.overflow_y = nana_ui_core::OverflowSpec::Auto;
        }),
        ("isolated", |style| {
            style.height = Some(LengthSpec::Px(400.0));
            style.layout_isolation = true;
        }),
    ];
    for (name, shape) in shapes {
        let (mut world, document) = column_tree(6);
        restyle_column(&mut world, shape);
        let mut retained = RetainedLayoutCache::default();
        let _ = world.take_system_work();
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        let natural = retained
            .execution_stats(document)
            .placement_natural_boundaries;
        assert_eq!(natural, usize::from(name != "plain"), "{name}");
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        resize_row(&mut world, 2, 28.0);
        scoped_step_matches_full(&mut world, document, viewport, &mut retained, name);
    }
}

/// Sections of a tall page for the sliced-layout tests: each 300 px tall
/// with four rows, the page a column 800 px wide inside `page`'s style.
const SECTIONS: u64 = 12;
const SECTION_ROWS: u64 = 4;

fn section(index: u64) -> StableNodeId {
    id(100 + index * 10)
}

fn section_row(index: u64, row: u64) -> StableNodeId {
    id(100 + index * 10 + 1 + row)
}

fn styled(layout: LayoutStyle) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        ..NodeStyle::default()
    }
}

fn row_style(height: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(200.0)),
        height: Some(LengthSpec::Px(height)),
        ..LayoutStyle::default()
    }
}

fn add_section(queue: &mut MutationQueue, document: DocumentId, index: u64) {
    queue.create(
        section(index),
        document,
        NodeKind::Element {
            tag: "section".into(),
        },
    );
    queue.insert(id(2), section(index), None);
    queue.set_style(
        section(index),
        styled(LayoutStyle {
            width: Some(LengthSpec::Px(800.0)),
            height: Some(LengthSpec::Px(300.0)),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        }),
    );
    for row in 0..SECTION_ROWS {
        let node = section_row(index, row);
        queue.create(node, document, NodeKind::Element { tag: "div".into() });
        queue.insert(section(index), node, None);
        queue.set_style(node, styled(row_style(50.0)));
    }
}

fn sectioned_page(page: LayoutStyle) -> (UiWorld, DocumentId) {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    queue.create(id(2), document, NodeKind::Element { tag: "main".into() });
    queue.insert(id(1), id(2), None);
    queue.set_style(id(2), styled(page));
    for index in 0..SECTIONS {
        add_section(&mut queue, document, index);
    }
    world.commit(queue).unwrap();
    (world, document)
}

fn page_column() -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(800.0)),
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    }
}

/// One pass with `placement_nodes` of budget (`None`: unsliced), its boxes
/// written back as a frame does.
fn sliced_pass(
    world: &mut UiWorld,
    document: DocumentId,
    retained: &mut RetainedLayoutCache,
    placement_nodes: Option<usize>,
    force_full: bool,
) -> LayoutExecutionStats {
    let viewport = LayoutViewport::new(800.0, 600.0);
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    retained.set_slice_limit(placement_nodes.map(|nodes| SliceLimit {
        placement_nodes: Some(nodes),
        deadline: None,
    }));
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(
            world,
            document,
            viewport,
            &work.layout_frontier_seeds,
            retained,
            force_full,
        )
        .unwrap();
    write_changed_boxes(world, &emitted);
    let _ = world.take_system_work();
    retained.execution_stats(document)
}

/// Which sections have their rows placed.
fn placed_sections(retained: &RetainedLayoutCache, document: DocumentId) -> Vec<u64> {
    let cache = &retained.documents[&document];
    (0..SECTIONS)
        .filter(|&index| cache.boxes.contains_key(&section_row(index, 0)))
        .collect()
}

/// Passes with no edit and no budget left until nothing is pending: one
/// out-of-view boundary each. Returns how many it took.
fn finish_sliced(
    world: &mut UiWorld,
    document: DocumentId,
    retained: &mut RetainedLayoutCache,
) -> usize {
    for passes in 1..=64 {
        sliced_pass(world, document, retained, Some(0), false);
        if !retained.layout_pending(document) {
            return passes;
        }
    }
    panic!("a sliced layout did not finish");
}

fn assert_full_layout(world: &UiWorld, document: DocumentId, retained: &RetainedLayoutCache) {
    let viewport = LayoutViewport::new(800.0, 600.0);
    let expected = full_boxes(world, document, viewport);
    let cached = retained.documents[&document].page_boxes(world);
    for (node, box_) in &expected {
        assert_eq!(
            cached.get(&node).copied().unwrap_or_default(),
            *box_,
            "a finished sliced layout differs from a full layout at {node:?}"
        );
    }
}

/// A first layout with no budget left places the sections in view, one out
/// of view, and leaves the rest without a box below them; each later pass
/// places the next nearest, and the finished layout is the full one. Every
/// pass on the way is checked by the verify guard: the rest of the page is
/// the full layout's, and nothing under a deferred section is placed.
#[test]
fn a_sliced_first_layout_places_what_is_in_view_first() {
    let (mut world, document) = sectioned_page(page_column());
    let mut retained = RetainedLayoutCache::default();
    let stats = sliced_pass(&mut world, document, &mut retained, Some(0), true);
    // In view: sections 0 and 1 (0–600 px); then the nearest out of view.
    assert_eq!(placed_sections(&retained, document), vec![0, 1, 2]);
    assert_eq!(stats.placement_slices, 1);
    assert_eq!(stats.placement_boundaries_deferred, (SECTIONS - 3) as usize);
    assert!(retained.layout_pending(document));
    // Every section has its box: measurement is not sliced.
    let cache = &retained.documents[&document];
    for index in 0..SECTIONS {
        assert!(cache.boxes.contains_key(&section(index)), "section {index}");
    }
    sliced_pass(&mut world, document, &mut retained, Some(0), false);
    assert_eq!(placed_sections(&retained, document), vec![0, 1, 2, 3]);
    let passes = finish_sliced(&mut world, document, &mut retained);
    assert_eq!(passes, (SECTIONS - 4) as usize);
    assert_full_layout(&world, document, &retained);
    // With room in the budget, a pass places everything at once.
    let (mut world, document) = sectioned_page(page_column());
    let mut retained = RetainedLayoutCache::default();
    let stats = sliced_pass(&mut world, document, &mut retained, Some(10_000), true);
    assert_eq!(stats.placement_boundaries_deferred, 0);
    assert!(!retained.layout_pending(document));
}

/// What is in view is measured on the scrolled page: a page scrolled to its
/// end places its last sections first.
#[test]
fn a_sliced_layout_ranks_by_what_the_scroll_shows() {
    let (mut world, document) = sectioned_page(LayoutStyle {
        height: Some(LengthSpec::Px(600.0)),
        overflow_y: nana_ui_core::OverflowSpec::Auto,
        ..page_column()
    });
    let mut queue = MutationQueue::new();
    queue.set_scroll_overscroll(
        id(2),
        crate::ScrollOffset {
            x: 0.0,
            y: 300.0 * (SECTIONS - 2) as f32,
        },
    );
    world.commit(queue).unwrap();
    let mut retained = RetainedLayoutCache::default();
    sliced_pass(&mut world, document, &mut retained, Some(0), true);
    assert_eq!(
        placed_sections(&retained, document),
        vec![SECTIONS - 3, SECTIONS - 2, SECTIONS - 1]
    );
    finish_sliced(&mut world, document, &mut retained);
    assert_full_layout(&world, document, &retained);
}

/// Edits while sections wait: one inside a deferred section, one that
/// resizes every section (its deferral is redone from the new inputs and
/// counted as cancelled), and one that removes a deferred section (dropped,
/// counted). Each pass is checked against a full layout outside what waits,
/// and the finished layout is the full one.
#[test]
fn edits_while_a_sliced_layout_waits_land_in_it() {
    let (mut world, document) = sectioned_page(page_column());
    let mut retained = RetainedLayoutCache::default();
    sliced_pass(&mut world, document, &mut retained, Some(0), true);
    let mut queue = MutationQueue::new();
    queue.set_style(section_row(SECTIONS - 1, 2), styled(row_style(90.0)));
    world.commit(queue).unwrap();
    sliced_pass(&mut world, document, &mut retained, Some(0), false);
    assert!(retained.layout_pending(document));

    let mut queue = MutationQueue::new();
    queue.set_style(
        id(2),
        styled(LayoutStyle {
            width: Some(LengthSpec::Px(640.0)),
            ..page_column()
        }),
    );
    for index in 0..SECTIONS {
        queue.set_style(
            section(index),
            styled(LayoutStyle {
                width: Some(LengthSpec::Fill),
                height: Some(LengthSpec::Px(300.0)),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
        );
    }
    world.commit(queue).unwrap();
    let stats = sliced_pass(&mut world, document, &mut retained, Some(0), false);
    assert!(stats.placement_deferrals_cancelled > 0, "{stats:?}");

    let mut queue = MutationQueue::new();
    queue.despawn_subtree(section(SECTIONS - 2));
    world.commit(queue).unwrap();
    let stats = sliced_pass(&mut world, document, &mut retained, Some(0), false);
    assert!(stats.placement_deferrals_cancelled > 0, "{stats:?}");

    finish_sliced(&mut world, document, &mut retained);
    assert_full_layout(&world, document, &retained);
}

/// Only what nothing retained is placed under is deferred: on a laid-out
/// page an edit that resizes every section places them all, whatever the
/// budget, while sections just mounted out of view are left but one.
#[test]
fn a_sliced_pass_defers_only_subtrees_with_nothing_placed() {
    let (mut world, document) = sectioned_page(page_column());
    let mut retained = RetainedLayoutCache::default();
    sliced_pass(&mut world, document, &mut retained, None, true);
    // Every section resized: each is placed again, in view or not.
    let mut queue = MutationQueue::new();
    for index in 0..SECTIONS {
        queue.set_style(
            section(index),
            styled(LayoutStyle {
                width: Some(LengthSpec::Px(640.0)),
                height: Some(LengthSpec::Px(320.0)),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
        );
    }
    world.commit(queue).unwrap();
    let stats = sliced_pass(&mut world, document, &mut retained, Some(0), false);
    assert_eq!(stats.placement_boundaries_deferred, 0);
    assert!(
        stats.placement_boundaries >= (SECTIONS - 2) as usize,
        "{stats:?}"
    );

    let mut queue = MutationQueue::new();
    for index in SECTIONS..SECTIONS + 3 {
        add_section(&mut queue, document, index);
    }
    world.commit(queue).unwrap();
    let stats = sliced_pass(&mut world, document, &mut retained, Some(0), false);
    assert_eq!(stats.placement_boundaries_deferred, 2, "{stats:?}");
    finish_sliced(&mut world, document, &mut retained);
    assert_full_layout(&world, document, &retained);
}

/// Gives section `index` of [`sectioned_page`] a shape of its own: grid,
/// reversed axis, a positioned child, a nested scroll container, text.
fn reshape_section(queue: &mut MutationQueue, document: DocumentId, index: u64) {
    let base = LayoutStyle {
        width: Some(LengthSpec::Px(800.0)),
        height: Some(LengthSpec::Px(300.0)),
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    };
    let style = match index % 6 {
        0 => base,
        1 => {
            let mut style = LayoutStyle {
                display: Some(DisplaySpec::Grid),
                ..base
            };
            style.grid.columns = Some(vec![GridTrack::Fr(1.0), GridTrack::Fr(2.0)]);
            style
        }
        2 => LayoutStyle {
            flex_reverse: true,
            ..base
        },
        3 => LayoutStyle {
            position: PositionSpec::Relative,
            ..base
        },
        4 => LayoutStyle {
            overflow_y: nana_ui_core::OverflowSpec::Auto,
            ..base
        },
        _ => LayoutStyle {
            height: None,
            ..base
        },
    };
    queue.set_style(section(index), styled(style));
    let extra = id(100 + index * 10 + 9);
    match index % 6 {
        3 => {
            queue.create(extra, document, NodeKind::Element { tag: "div".into() });
            queue.insert(section(index), extra, None);
            queue.set_style(
                extra,
                styled(LayoutStyle {
                    position: PositionSpec::Absolute,
                    offset_right: Some(LengthSpec::Px(8.0)),
                    offset_bottom: Some(LengthSpec::Px(8.0)),
                    ..row_style(20.0)
                }),
            );
        }
        4 => {
            // The rows go into a nested scroll container.
            queue.create(extra, document, NodeKind::Element { tag: "div".into() });
            queue.insert(section(index), extra, Some(section_row(index, 0)));
            queue.set_style(
                extra,
                styled(LayoutStyle {
                    height: Some(LengthSpec::Px(120.0)),
                    overflow_y: nana_ui_core::OverflowSpec::Scroll,
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                }),
            );
            for row in 0..SECTION_ROWS {
                queue.insert(extra, section_row(index, row), None);
            }
        }
        5 => {
            queue.create(extra, document, NodeKind::Text);
            queue.insert(section(index), extra, None);
            queue.set_text(
                extra,
                TextContent {
                    value: "a paragraph of text under the rows".into(),
                },
            );
        }
        _ => {}
    }
}

/// A sliced first layout of a page whose sections take every shape the
/// placement has, edited while it waits, finishes as the full layout does;
/// every pass on the way is checked by the verify guard.
#[test]
fn a_sliced_layout_of_mixed_sections_finishes_as_a_full_one() {
    let (mut world, document) = sectioned_page(page_column());
    let mut queue = MutationQueue::new();
    for index in 0..SECTIONS {
        reshape_section(&mut queue, document, index);
    }
    world.commit(queue).unwrap();
    let mut retained = RetainedLayoutCache::default();
    let stats = sliced_pass(&mut world, document, &mut retained, Some(0), true);
    assert!(stats.placement_boundaries_deferred > 0);
    sliced_pass(&mut world, document, &mut retained, Some(0), false);
    // A row resized in a section still waiting, and one in a placed one.
    let mut queue = MutationQueue::new();
    queue.set_style(section_row(SECTIONS - 2, 1), styled(row_style(70.0)));
    queue.set_style(section_row(1, 1), styled(row_style(70.0)));
    world.commit(queue).unwrap();
    sliced_pass(&mut world, document, &mut retained, Some(4), false);
    finish_sliced(&mut world, document, &mut retained);
    assert_full_layout(&world, document, &retained);
}

/// A deferred boundary whose origin moves and whose containing width changes
/// while its own fixed size stays the same must still reach `PassSlice::reach`
/// (not be skipped by `keeps_moved_subtree`) and cancel/update its deferral.
#[test]
fn a_deferred_boundary_with_moved_origin_and_changed_containing_is_reached() {
    let (mut world, document) = sectioned_page(page_column());
    let mut retained = RetainedLayoutCache::default();
    sliced_pass(&mut world, document, &mut retained, Some(0), true);
    assert!(retained.layout_pending(document));

    // Change the parent's width (containing block width for every section) and
    // grow an in-view section so every deferred section also moves down, while
    // keeping the deferred sections' own fixed 800x300 size and zero padding.
    let mut queue = MutationQueue::new();
    queue.set_style(
        id(2),
        styled(LayoutStyle {
            width: Some(LengthSpec::Px(640.0)),
            ..page_column()
        }),
    );
    queue.set_style(
        section(0),
        styled(LayoutStyle {
            width: Some(LengthSpec::Px(800.0)),
            height: Some(LengthSpec::Px(360.0)),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        }),
    );
    world.commit(queue).unwrap();
    let stats = sliced_pass(&mut world, document, &mut retained, Some(0), false);
    assert!(stats.placement_deferrals_cancelled > 0, "{stats:?}");
    finish_sliced(&mut world, document, &mut retained);
    assert_full_layout(&world, document, &retained);
}

/// A `position: fixed` non-leaf inside a scrolled container and a zero-height
/// non-leaf wrapper at the top edge of the viewport are both in view and must
/// not be deferred under a tight slice budget.
#[test]
fn a_sliced_pass_places_fixed_in_scroll_and_zero_height_edge_box_in_view() {
    // Part A: `position: fixed` inside a container scrolled far down.
    let (mut world, document) = sectioned_page(LayoutStyle {
        height: Some(LengthSpec::Px(600.0)),
        overflow_y: nana_ui_core::OverflowSpec::Auto,
        ..page_column()
    });
    let fixed_panel = id(900);
    let fixed_child = id(901);
    let mut queue = MutationQueue::new();
    queue.create(
        fixed_panel,
        document,
        NodeKind::Element { tag: "div".into() },
    );
    queue.insert(id(2), fixed_panel, None);
    queue.set_style(
        fixed_panel,
        styled(LayoutStyle {
            position: PositionSpec::Fixed,
            offset_left: Some(LengthSpec::Px(12.0)),
            offset_top: Some(LengthSpec::Px(16.0)),
            width: Some(LengthSpec::Px(120.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        }),
    );
    queue.create(
        fixed_child,
        document,
        NodeKind::Element { tag: "div".into() },
    );
    queue.insert(fixed_panel, fixed_child, None);
    queue.set_style(fixed_child, styled(row_style(24.0)));
    queue.set_scroll_overscroll(id(2), crate::ScrollOffset { x: 0.0, y: 2000.0 });
    world.commit(queue).unwrap();
    let mut retained = RetainedLayoutCache::default();
    sliced_pass(&mut world, document, &mut retained, Some(0), true);
    assert!(
        retained.documents[&document]
            .boxes
            .contains_key(&fixed_child),
        "fixed panel inside scrolled container was deferred"
    );
    finish_sliced(&mut world, document, &mut retained);
    assert_full_layout(&world, document, &retained);

    // Part B: zero-height non-leaf wrapper sitting on the top edge (y = 0).
    let (mut world, document) = sectioned_page(page_column());
    let zero_wrap = id(910);
    let zero_child = id(911);
    let mut queue = MutationQueue::new();
    queue.create(zero_wrap, document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(2), zero_wrap, Some(section(0)));
    queue.set_style(
        zero_wrap,
        styled(LayoutStyle {
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(0.0)),
            ..LayoutStyle::default()
        }),
    );
    queue.create(
        zero_child,
        document,
        NodeKind::Element { tag: "div".into() },
    );
    queue.insert(zero_wrap, zero_child, None);
    queue.set_style(
        zero_child,
        styled(LayoutStyle {
            position: PositionSpec::Absolute,
            offset_left: Some(LengthSpec::Px(4.0)),
            offset_top: Some(LengthSpec::Px(4.0)),
            ..row_style(20.0)
        }),
    );
    world.commit(queue).unwrap();
    let mut retained = RetainedLayoutCache::default();
    sliced_pass(&mut world, document, &mut retained, Some(0), true);
    assert!(
        retained.documents[&document]
            .boxes
            .contains_key(&zero_child),
        "zero-height wrapper at top viewport edge was deferred"
    );
    finish_sliced(&mut world, document, &mut retained);
    assert_full_layout(&world, document, &retained);
}

/// Reparenting an already-placed subtree under a pending deferred boundary
/// disqualifies that boundary from being deferred again, and reparenting one
/// pending boundary under another keeps verify invariants intact.
#[test]
fn reparenting_into_a_deferred_boundary_keeps_verify_and_finishes_cleanly() {
    // Case 1: already-placed row moved into the farthest deferred section.
    let (mut world, document) = sectioned_page(page_column());
    let mut retained = RetainedLayoutCache::default();
    sliced_pass(&mut world, document, &mut retained, Some(0), true);
    assert!(retained.layout_pending(document));

    let moved_row = section_row(0, 0);
    let target = section(SECTIONS - 1);
    let mut queue = MutationQueue::new();
    queue.insert(target, moved_row, None);
    world.commit(queue).unwrap();
    // Budget 0 triggers defers() on `target` once an earlier offscreen section
    // has been placed; `holds_no_retained_placement` must reject deferral so
    // `moved_row` (which has a retained box) is never left under a deferral.
    sliced_pass(&mut world, document, &mut retained, Some(0), false);
    assert!(
        retained.documents[&document].boxes.contains_key(&moved_row),
        "reparented retained row should remain placed"
    );
    finish_sliced(&mut world, document, &mut retained);
    assert_full_layout(&world, document, &retained);

    // Case 2: one pending boundary reparented under another pending boundary.
    let (mut world, document) = sectioned_page(page_column());
    let mut retained = RetainedLayoutCache::default();
    sliced_pass(&mut world, document, &mut retained, Some(0), true);
    let mut queue = MutationQueue::new();
    queue.insert(section(4), section(SECTIONS - 1), None);
    world.commit(queue).unwrap();
    sliced_pass(&mut world, document, &mut retained, Some(0), false);
    finish_sliced(&mut world, document, &mut retained);
    assert_full_layout(&world, document, &retained);
}

#[test]
fn pass_container_plans_patch_overrides_prior_rebuild() {
    let mut retained = HashMap::new();
    let mut remote_overlays = crate::NodeSet::default();
    let container = id(42);
    let child_a = id(43);
    let child_b = id(44);
    let style = Arc::new(LayoutStyle::default());
    let make_entry = |child, y| PlannedChild {
        child,
        style: Arc::clone(&style),
        origin: Point { x: 0.0, y },
        size: Size::new(100.0, 20.0),
        intrinsic: Size::new(100.0, 20.0),
        at_main: None,
        cursor_before: y,
        baseline: PackedOptF32::NONE,
    };
    let make_plan = |entries| ContainerPlan {
        content_origin: Point::ZERO,
        gap: 0.0,
        sequential: true,
        by_child: None,
        origin: Point::ZERO,
        size: Size::new(100.0, 200.0),
        containing: Size::new(100.0, 200.0),
        parent_font_px: 16.0,
        viewport: LayoutViewport::new(800.0, 600.0),
        style: Arc::clone(&style),
        children: Arc::new(vec![child_a, child_b]),
        content: Size::new(100.0, 200.0),
        child_font_px: 16.0,
        child_available: Size::new(100.0, 200.0),
        main_direction: FlexDirection::Column,
        writing: nana_ui_core::WritingContext::default(),
        main_reversed: false,
        cross_reversed: false,
        entries,
        placed_main: 200.0,
        grid: None,
        cross_independent: true,
        main_dependent: false,
        overlay: Vec::new(),
    };
    retained.insert(
        container,
        make_plan(vec![make_entry(child_a, 0.0), make_entry(child_b, 20.0)]),
    );

    let mut pass = PassContainerPlans::default();
    pass.rebuild(container, make_plan(vec![make_entry(child_a, 0.0)]));
    pass.patch(
        container,
        PlanPatch {
            rows: vec![(1, make_entry(child_b, 48.0))],
            placed_main: Some(240.0),
        },
    );
    pass.merge_into(&mut retained, &mut remote_overlays);

    let merged = &retained[&container];
    assert_eq!(merged.entries.len(), 2);
    assert_eq!(merged.entries[1].origin.y, 48.0);
    assert_eq!(merged.placed_main, 240.0);
}

mod overlay_plans {
    //! Issue #258 overlay placement and context-plan memory.
    //!
    //! Owned by the overlay slice so the inline formatting work can keep editing
    //! `tests.rs` and `inline.rs`.

    use std::sync::Arc;

    use nana_ui_core::{AlignSpec, FlexDirection, LayoutStyle, LengthSpec, PositionSpec};

    use crate::{DocumentId, MutationQueue, NodeKind, NodeStyle, UiWorld};

    use super::super::*;

    fn nid(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    fn px(value: f32) -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Px(value)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        }
    }

    fn write_changed(world: &mut UiWorld, emitted: &[(StableNodeId, LayoutBox)]) {
        let mut queue = MutationQueue::new();
        let mut any = false;
        for (id, box_) in emitted {
            if world.layout_box(*id) != Some(*box_) {
                queue.write_layout(*id, *box_);
                any = true;
            }
        }
        if any {
            world.commit(queue).unwrap();
        }
    }

    fn full_boxes(
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
    ) -> Vec<(StableNodeId, LayoutBox)> {
        RuntimeLayoutEngine
            .layout_document(world, document, viewport)
            .unwrap()
    }

    struct Step {
        emitted: Vec<StableNodeId>,
        children_measured: usize,
        full_document_fallbacks: usize,
    }

    fn scoped(
        world: &mut UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
        retained: &mut RetainedLayoutCache,
    ) -> Step {
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(
                world,
                document,
                viewport,
                &work.layout_frontier_seeds,
                retained,
                false,
            )
            .unwrap();
        let cache = &retained.documents[&document];
        let step = Step {
            emitted: emitted.iter().map(|(id, _)| *id).collect(),
            children_measured: cache.execution_stats.children_measured,
            full_document_fallbacks: cache.frontier_stats.full_document_fallbacks,
        };
        write_changed(world, &emitted);
        let _ = world.take_system_work();
        let expected = full_boxes(world, document, viewport);
        let cached = &retained.documents[&document].page_boxes(world);
        for (id, box_) in expected {
            assert_eq!(
                cached.get(&id).copied().unwrap_or_default(),
                box_,
                "scoped layout diverged at {id:?}"
            );
        }
        step
    }

    fn prime(
        world: &mut UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
        retained: &mut RetainedLayoutCache,
    ) {
        let _ = world.take_system_work();
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(world, document, viewport, &[], retained, true)
            .unwrap();
        write_changed(world, &emitted);
        let _ = world.take_system_work();
    }

    fn set_layout(world: &mut UiWorld, id: StableNodeId, layout: LayoutStyle) {
        let mut queue = MutationQueue::new();
        queue.set_style(
            id,
            NodeStyle {
                layout: Arc::new(layout),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
    }

    /// `in_flow` fixed rows, one absolute child, one fixed child, and an unrelated
    /// sibling outside the container.
    fn positioned_world(in_flow: usize, absolute: LayoutStyle) -> (UiWorld, DocumentId) {
        let document = DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(nid(1), document, NodeKind::Document);
        queue.create(nid(2), document, NodeKind::Element { tag: "div".into() });
        queue.create(nid(3), document, NodeKind::Element { tag: "div".into() });
        queue.insert(nid(1), nid(2), None);
        queue.insert(nid(1), nid(3), None);
        queue.set_style(
            nid(2),
            NodeStyle {
                layout: Arc::new(px(40.0)),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            nid(3),
            NodeStyle {
                // The positioned children's containing block.
                layout: Arc::new(LayoutStyle {
                    position: PositionSpec::Relative,
                    width: Some(LengthSpec::Px(400.0)),
                    height: Some(LengthSpec::Px(300.0)),
                    direction: Some(FlexDirection::Column),
                    align_items: AlignSpec::Start,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        for index in 0..in_flow {
            let id = nid(10 + index as u64);
            queue.create(id, document, NodeKind::Element { tag: "div".into() });
            queue.insert(nid(3), id, None);
            queue.set_style(
                id,
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(80.0)),
                        height: Some(LengthSpec::Px(20.0)),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
        }
        queue.create(nid(90), document, NodeKind::Element { tag: "div".into() });
        queue.create(nid(91), document, NodeKind::Element { tag: "div".into() });
        queue.insert(nid(3), nid(90), None);
        queue.insert(nid(3), nid(91), None);
        queue.set_style(
            nid(90),
            NodeStyle {
                layout: Arc::new(absolute),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            nid(91),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    position: PositionSpec::Fixed,
                    offset_left: Some(LengthSpec::Px(4.0)),
                    offset_top: Some(LengthSpec::Px(4.0)),
                    width: Some(LengthSpec::Px(16.0)),
                    height: Some(LengthSpec::Px(16.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        (world, document)
    }

    fn absolute_px(width: f32) -> LayoutStyle {
        LayoutStyle {
            position: PositionSpec::Absolute,
            offset_left: Some(LengthSpec::Px(8.0)),
            offset_top: Some(LengthSpec::Px(8.0)),
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(24.0)),
            ..LayoutStyle::default()
        }
    }

    fn assert_flow_not_placed(step: &Step, in_flow: usize) {
        for index in 0..in_flow {
            assert!(
                !step.emitted.contains(&nid(10 + index as u64)),
                "in-flow sibling {} was placed: {:?}",
                index,
                step.emitted
            );
        }
        assert!(
            !step.emitted.contains(&nid(2)),
            "unrelated branch was placed: {:?}",
            step.emitted
        );
        assert_eq!(step.full_document_fallbacks, 0);
    }

    /// A positioned child's own size change places that child and leaves in-flow
    /// siblings, and an unrelated branch, unplaced.
    #[test]
    fn positioned_content_change_does_not_place_in_flow_siblings() {
        let viewport = LayoutViewport::new(800.0, 600.0);
        let mut measured = Vec::new();
        for in_flow in [4usize, 16] {
            let (mut world, document) = positioned_world(in_flow, absolute_px(40.0));
            let mut retained = RetainedLayoutCache::default();
            prime(&mut world, document, viewport, &mut retained);

            set_layout(&mut world, nid(90), absolute_px(55.0));
            let step = scoped(&mut world, document, viewport, &mut retained);
            assert!(
                step.emitted.contains(&nid(90)),
                "absolute child was not placed: {:?}",
                step.emitted
            );
            assert_flow_not_placed(&step, in_flow);
            assert!(
                step.children_measured < in_flow,
                "{in_flow} in-flow siblings measured {}",
                step.children_measured
            );
            assert_eq!(world.layout_box(nid(90)).unwrap().width, 55.0);
            measured.push(step.children_measured);

            let mut painted = absolute_px(55.0);
            painted.opacity = Some(0.4);
            set_layout(&mut world, nid(90), painted);
            let work = world.take_system_work();
            assert!(
                work.layout_frontier_seeds.is_empty(),
                "paint-only on an absolute child scheduled layout"
            );

            let fixed = LayoutStyle {
                position: PositionSpec::Fixed,
                offset_left: Some(LengthSpec::Px(4.0)),
                offset_top: Some(LengthSpec::Px(4.0)),
                width: Some(LengthSpec::Px(28.0)),
                height: Some(LengthSpec::Px(16.0)),
                ..LayoutStyle::default()
            };
            set_layout(&mut world, nid(91), fixed);
            let step = scoped(&mut world, document, viewport, &mut retained);
            assert!(
                step.emitted.contains(&nid(91)),
                "fixed child was not placed: {:?}",
                step.emitted
            );
            assert_flow_not_placed(&step, in_flow);
            assert_eq!(world.layout_box(nid(91)).unwrap().width, 28.0);
        }
        assert_eq!(
            measured[0], measured[1],
            "in-flow measure work grew with sibling count: {measured:?}"
        );
    }

    /// Growing the containing block places the absolute child that reads it, and
    /// does not place a fixed child whose containing block is the viewport.
    #[test]
    fn containing_block_change_places_dependent_positioned_child() {
        let viewport = LayoutViewport::new(800.0, 600.0);
        let absolute = LayoutStyle {
            position: PositionSpec::Absolute,
            offset_left: Some(LengthSpec::Px(8.0)),
            offset_top: Some(LengthSpec::Px(8.0)),
            width: Some(LengthSpec::Percent(50.0)),
            height: Some(LengthSpec::Px(24.0)),
            ..LayoutStyle::default()
        };
        let (mut world, document) = positioned_world(6, absolute.clone());
        let mut retained = RetainedLayoutCache::default();
        prime(&mut world, document, viewport, &mut retained);
        let before = world.layout_box(nid(90)).unwrap().width;

        let container = LayoutStyle {
            position: PositionSpec::Relative,
            width: Some(LengthSpec::Px(500.0)),
            height: Some(LengthSpec::Px(300.0)),
            direction: Some(FlexDirection::Column),
            align_items: AlignSpec::Start,
            ..LayoutStyle::default()
        };
        set_layout(&mut world, nid(3), container);
        let step = scoped(&mut world, document, viewport, &mut retained);
        assert!(
            step.emitted.contains(&nid(90)),
            "percentage absolute child was not placed when its containing block grew: {:?}",
            step.emitted
        );
        assert!(
            !step.emitted.contains(&nid(91)),
            "fixed child does not depend on the parent's content box: {:?}",
            step.emitted
        );
        assert_flow_not_placed(&step, 6);
        let after = world.layout_box(nid(90)).unwrap().width;
        assert!(
            (after - before).abs() > 1.0,
            "containing block growth did not change the absolute child ({before} -> {after})"
        );
        assert!(
            (after - 250.0).abs() < 0.5,
            "expected 50% of 500, got {after}"
        );
    }

    /// Repeating an edit of one container replaces its plan. The retained entry
    /// count stays with the direct participants and the two measure slots.
    #[test]
    fn repeated_container_edits_do_not_grow_plan_entries() {
        let viewport = LayoutViewport::new(800.0, 600.0);
        let document = DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(nid(1), document, NodeKind::Document);
        queue.create(nid(3), document, NodeKind::Element { tag: "div".into() });
        queue.insert(nid(1), nid(3), None);
        queue.set_style(
            nid(3),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(400.0)),
                    direction: Some(FlexDirection::Column),
                    align_items: AlignSpec::Start,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        for index in 0..6u64 {
            let id = nid(10 + index);
            queue.create(id, document, NodeKind::Element { tag: "div".into() });
            queue.insert(nid(3), id, None);
            queue.set_style(
                id,
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(80.0)),
                        height: Some(LengthSpec::Px(20.0)),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
        }
        queue.create(nid(90), document, NodeKind::Element { tag: "div".into() });
        queue.insert(nid(3), nid(90), None);
        queue.set_style(
            nid(90),
            NodeStyle {
                layout: Arc::new(absolute_px(30.0)),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let mut retained = RetainedLayoutCache::default();
        prime(&mut world, document, viewport, &mut retained);

        let mut counts = Vec::new();
        for step in 0..16 {
            set_layout(
                &mut world,
                nid(10),
                LayoutStyle {
                    width: Some(LengthSpec::Px(80.0)),
                    height: Some(LengthSpec::Px(20.0 + step as f32)),
                    ..LayoutStyle::default()
                },
            );
            let _ = scoped(&mut world, document, viewport, &mut retained);
            counts.push(retained.documents[&document].retained_plan_entries());
            let cache = &retained.documents[&document];
            for plan in cache.container_plans.values() {
                assert!(
                    plan.entries.len() <= plan.children.len(),
                    "flow entries exceeded direct children"
                );
                assert!(
                    plan.overlay.len() <= plan.children.len(),
                    "overlay entries exceeded direct children"
                );
            }
            for slots in cache.measure_plans.values() {
                let occupied = slots.slots.iter().flatten().count();
                assert!(occupied <= 2, "measure slots grew to {occupied}");
                for plan in slots.slots.iter().flatten() {
                    assert!(
                        plan.entries.len() <= plan.children.len(),
                        "measure entries exceeded direct children"
                    );
                }
            }
        }
        assert_eq!(
            counts[8], counts[15],
            "plan entries kept growing across repeated edits: {counts:?}"
        );
    }

    /// A badge anchored to the far corners moves when the container it is
    /// anchored in grows, though nothing about the badge changed.
    #[test]
    fn a_far_anchored_badge_follows_its_growing_container() {
        let badge = LayoutStyle {
            position: PositionSpec::Absolute,
            offset_right: Some(LengthSpec::Px(8.0)),
            offset_bottom: Some(LengthSpec::Px(8.0)),
            width: Some(LengthSpec::Px(16.0)),
            height: Some(LengthSpec::Px(16.0)),
            ..LayoutStyle::default()
        };
        let (mut world, document) = positioned_world(3, badge);
        // Content-sized, so a growing row grows the block the badge is in.
        set_layout(
            &mut world,
            nid(3),
            LayoutStyle {
                position: PositionSpec::Relative,
                width: Some(LengthSpec::Px(400.0)),
                direction: Some(FlexDirection::Column),
                align_items: AlignSpec::Start,
                ..LayoutStyle::default()
            },
        );
        let viewport = LayoutViewport::new(800.0, 600.0);
        let mut retained = RetainedLayoutCache::default();
        prime(&mut world, document, viewport, &mut retained);
        let before = world.layout_box(nid(90)).unwrap();
        set_layout(
            &mut world,
            nid(11),
            LayoutStyle {
                height: Some(LengthSpec::Px(60.0)),
                ..px(80.0)
            },
        );
        scoped(&mut world, document, viewport, &mut retained);
        let after = world.layout_box(nid(90)).unwrap();
        assert_eq!(after.y, before.y + 40.0, "{before:?} -> {after:?}");
    }
}
