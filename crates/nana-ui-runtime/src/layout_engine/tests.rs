use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, BoxSizing, CalcBinOp, CalcExpr, ClearSpec, DirSpec, DisplaySpec, FlexDirection,
    FlexWrap, FloatSpec, GridLine, GridPlacement, GridRepeatAuto, GridTrack,
    GridTrackListUnsupported, JustifySpec, LayoutStyle, LengthSpec, LineHeightSpec, PositionSpec,
    WhiteSpaceSpec, WritingModeSpec,
};

use crate::{
    ComputedStyle, MutationQueue, NodeKind, NodeStyle, TextContent, TextMetrics, TextShaper,
    UiWorld,
};

use super::*;

fn id(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
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

fn full_boxes(
    world: &UiWorld,
    document: DocumentId,
    viewport: LayoutViewport,
) -> HashMap<StableNodeId, LayoutBox> {
    RuntimeLayoutEngine
        .layout_document(world, document, viewport)
        .unwrap()
        .into_iter()
        .collect::<HashMap<_, _>>()
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
fn scoped_layout_touches_only_the_change_closure_and_matches_full_recompute() {
    let (mut world, document) = column_tree(400);
    let viewport = LayoutViewport::new(300.0, 800.0);
    let mut retained = RetainedLayoutCache::default();

    // Production drains dirty work before layout; the create-time marks
    // must not leak into the scoped measurement below.
    let _ = world.take_system_work();

    // Bootstrap: full pass populates the retained cache with every box.
    let emitted = RuntimeLayoutEngine
        .layout_document_scoped(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    assert_eq!(emitted.len(), 802, "full pass emits every node");
    write_changed_boxes(&mut world, &emitted);
    let _ = world.take_system_work();

    // Change the LAST row: nothing shifts above it, so the scoped pass
    // must recompute only that row's ancestor chain.
    resize_row(&mut world, 399, 26.0);
    let work = world.take_system_work();
    assert!(!work.layout.is_empty());
    let emitted = RuntimeLayoutEngine
        .layout_document_scoped(
            &world,
            document,
            viewport,
            &work.layout,
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
            retained.boxes.get(&node),
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
        .layout_document_scoped(
            &world,
            document,
            viewport,
            &work.layout,
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
            retained.boxes.get(&node),
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
    assert!(written.contains(&shifted_label));
    assert!(extract.render_extraction.contains(&changed_row));
    assert!(extract.render_extraction.contains(&shifted_row));
    assert!(extract.render_extraction.contains(&shifted_label));
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
        .layout_document_scoped(&world, document, viewport, &[], &mut retained, true)
        .unwrap();
    assert_eq!(emitted.len(), 802);
    assert_eq!(retained.materialized_inputs, 802);
    write_changed_boxes(&mut world, &emitted);
    let _ = world.take_system_work();

    resize_row(&mut world, 399, 26.0);
    let work = world.take_system_work();
    let emitted = RuntimeLayoutEngine
        .layout_document_scoped(
            &world,
            document,
            viewport,
            &work.layout,
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
        retained.materialized_inputs <= 16,
        "tail row must not assemble unshifted siblings, materialized {} of 802",
        retained.materialized_inputs
    );
    for (node, box_) in full_boxes(&world, document, viewport) {
        assert_eq!(
            retained.boxes.get(&node),
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
    queue.set_style(
        id(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                display: Some(DisplaySpec::Grid),
                direction: Some(FlexDirection::Row),
                width: Some(LengthSpec::Px(800.0)),
                height: Some(LengthSpec::Px(400.0)),
                grid_columns: Some(vec![GridTrack::Px(220.0), GridTrack::Fr(1.0)]),
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
        .layout_style_tree(&tree, LayoutViewport::new(400.0, 80.0))
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
        .layout_style_tree(&tree, LayoutViewport::new(200.0, 80.0))
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
        .layout_style_tree(&tree, LayoutViewport::new(200.0, 200.0))
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
        .layout_style_tree(&tree, LayoutViewport::new(200.0, 200.0))
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
        .layout_style_tree(&tree, LayoutViewport::new(200.0, 200.0))
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(
        boxes["child"].height, 64.0,
        "2em min-height against parent font-size 32px must be 64px, not 32px"
    );
}

fn box_map(root: &StyleLayoutNode, vw: f32, vh: f32) -> HashMap<String, LayoutBox> {
    RuntimeLayoutEngine
        .layout_style_tree(root, LayoutViewport::new(vw, vh))
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

#[test]
fn display_contents_hoists_children_into_flex_row_gap() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(40.0)),
            gap: Some(LengthSpec::Px(10.0)),
            align_items: AlignSpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![StyleLayoutNode {
            id: "contents".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Contents),
                ..LayoutStyle::default()
            },
            children: vec![px_box("a", 50.0, 40.0), px_box("b", 50.0, 40.0)],
            text: None,
        }],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 40.0);
    assert!(
        !boxes.contains_key("contents"),
        "display:contents must be absent from the box map"
    );
    assert!((boxes["a"].x - 0.0).abs() < 0.01);
    assert!((boxes["b"].x - 60.0).abs() < 0.01);
    assert_eq!(boxes["a"].width, 50.0);
    assert_eq!(boxes["b"].width, 50.0);
}

#[test]
fn grid_2d_auto_flow_wraps_fourth_item_to_second_row() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(100.0)),
            height: Some(LengthSpec::Px(100.0)),
            grid_columns: Some(vec![GridTrack::Px(50.0), GridTrack::Px(50.0)]),
            ..LayoutStyle::default()
        },
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
    let first = StyleLayoutNode {
        id: "a".into(),
        style: LayoutStyle {
            height: Some(LengthSpec::Px(50.0)),
            grid_placement: GridPlacement {
                column_start: GridLine::Span(2),
                ..GridPlacement::default()
            },
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(150.0)),
            height: Some(LengthSpec::Px(100.0)),
            grid_columns: Some(vec![
                GridTrack::Px(50.0),
                GridTrack::Px(50.0),
                GridTrack::Px(50.0),
            ]),
            ..LayoutStyle::default()
        },
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
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(50.0)),
            grid_columns: Some(vec![GridTrack::Px(200.0)]),
            ..LayoutStyle::default()
        },
        children: vec![item],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 50.0);
    assert!((boxes["item"].x - 150.0).abs() < 0.01);
    assert_eq!(boxes["item"].width, 50.0);
}

#[test]
fn grid_auto_fit_fills_two_minmax_tracks_in_500px() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(500.0)),
            height: Some(LengthSpec::Px(50.0)),
            grid_columns_repeat: Some(GridRepeatAuto {
                kind: GridTrackListUnsupported::RepeatAutoFit,
                tracks: vec![GridTrack::MinMax {
                    min_px: 200.0,
                    fr: 1.0,
                    max_px: None,
                }],
                ..Default::default()
            }),
            ..LayoutStyle::default()
        },
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

#[test]
fn white_space_pre_measures_explicit_newlines() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Block),
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
            display: Some(DisplaySpec::Block),
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
    let style = ComputedStyle {
        font_size: 16.0,
        line_height: Some(LineHeightSpec::Absolute(20.0)),
        ..ComputedStyle::default()
    };
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
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(300.0)),
            height: Some(LengthSpec::Px(40.0)),
            grid_columns: Some(vec![GridTrack::Px(100.0), GridTrack::Fr(1.0)]),
            ..LayoutStyle::default()
        },
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
    assert!(
        (used_in_grid_cell(
            Some(spec),
            0.0,
            400.0,
            LayoutViewport::new(400.0, 80.0),
            FontSizeContext::default(),
        ) - 110.0)
            .abs()
            < 0.01,
        "grid used size must resolve calc against the cell, not intrinsic 0"
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
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(400.0)),
            height: Some(LengthSpec::Px(40.0)),
            grid_columns: Some(vec![GridTrack::Px(400.0)]),
            align_items: AlignSpec::Start,
            justify_items: Some(AlignSpec::Start),
            ..LayoutStyle::default()
        },
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
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(300.0)),
            height: Some(LengthSpec::Px(40.0)),
            grid_columns: Some(vec![GridTrack::Px(100.0), GridTrack::Fr(1.0)]),
            // CSS `display:grid` initial align-items is stretch (css_map sets this).
            align_items: AlignSpec::Stretch,
            ..LayoutStyle::default()
        },
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

#[test]
fn same_side_floats_do_not_overlap() {
    let floated = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(60.0)),
            height: Some(LengthSpec::Px(40.0)),
            float: FloatSpec::Left,
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(80.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![floated("a"), floated("b")],
        text: None,
    };
    let boxes = box_map(&tree, 80.0, 80.0);
    assert!((boxes["a"].x - 0.0).abs() < 0.5);
    assert!((boxes["a"].y - 0.0).abs() < 0.5);
    assert!(
        (boxes["b"].y - 40.0).abs() < 0.5,
        "second left float must wrap below, got {:?}",
        boxes["b"]
    );
    assert!((boxes["b"].x - 0.0).abs() < 0.5);
}

#[test]
fn float_own_clear_starts_below_packed_same_side() {
    let left = |id: &str, clear: ClearSpec| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(60.0)),
            height: Some(LengthSpec::Px(40.0)),
            float: FloatSpec::Left,
            clear,
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![left("a", ClearSpec::None), left("b", ClearSpec::Left)],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 80.0);
    assert!((boxes["a"].x - 0.0).abs() < 0.5);
    assert!((boxes["a"].y - 0.0).abs() < 0.5);
    assert!(
        (boxes["b"].y - 40.0).abs() < 0.5 && (boxes["b"].x - 0.0).abs() < 0.5,
        "float with clear:left must start below packed left, not beside it, got {:?}",
        boxes["b"]
    );
}

#[test]
fn subgrid_inherits_parent_column_track_sizes() {
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
            grid_columns_subgrid: true,
            grid_placement: GridPlacement {
                column_start: GridLine::Index(1),
                column_end: GridLine::Index(-1),
                ..GridPlacement::default()
            },
            align_items: AlignSpec::Stretch,
            ..LayoutStyle::default()
        },
        children: vec![cell("a"), cell("b")],
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Grid),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(40.0)),
            grid_columns: Some(vec![GridTrack::Px(80.0), GridTrack::Px(120.0)]),
            align_items: AlignSpec::Stretch,
            ..LayoutStyle::default()
        },
        children: vec![sub],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 40.0);
    assert!(
        (boxes["a"].width - 80.0).abs() < 0.5 && (boxes["a"].x - 0.0).abs() < 0.5,
        "subgrid col 1 must inherit 80px, not split the 200px cell, got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["b"].width - 120.0).abs() < 0.5 && (boxes["b"].x - 80.0).abs() < 0.5,
        "subgrid col 2 must inherit 120px, got {:?}",
        boxes["b"]
    );
}

#[test]
fn ifc_wraps_inline_around_left_float() {
    let inline = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineBlock),
            width: Some(LengthSpec::Px(70.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    };
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![
            StyleLayoutNode {
                id: "float".into(),
                style: LayoutStyle {
                    float: FloatSpec::Left,
                    width: Some(LengthSpec::Px(80.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            },
            inline("a"),
            inline("b"),
            inline("c"),
        ],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 80.0);
    assert!(
        (boxes["float"].x - 0.0).abs() < 0.5 && (boxes["float"].y - 0.0).abs() < 0.5,
        "float stays packed at origin, got {:?}",
        boxes["float"]
    );
    assert!(
        (boxes["a"].x - 80.0).abs() < 0.5 && (boxes["a"].y - 0.0).abs() < 0.5,
        "first inline must start after the left float band, got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["b"].x - 80.0).abs() < 0.5 && (boxes["b"].y - 20.0).abs() < 0.5,
        "second inline wraps in the shortened band, got {:?}",
        boxes["b"]
    );
    assert!(
        (boxes["c"].y - 40.0).abs() < 0.5 && (boxes["c"].x - 0.0).abs() < 0.5,
        "third inline drops below the float to full width, got {:?}",
        boxes["c"]
    );
}

#[test]
fn flex_item_float_is_blockified_not_floated() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(40.0)),
            align_items: AlignSpec::Start,
            ..LayoutStyle::default()
        },
        children: vec![
            StyleLayoutNode {
                id: "a".into(),
                style: LayoutStyle {
                    float: FloatSpec::Left,
                    width: Some(LengthSpec::Px(50.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            },
            px_box("b", 50.0, 40.0),
        ],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 40.0);
    assert!(
        (boxes["a"].x - 0.0).abs() < 0.5 && (boxes["b"].x - 50.0).abs() < 0.5,
        "flex item float must stay a flex item, not pack as a float, got a={:?} b={:?}",
        boxes["a"],
        boxes["b"]
    );
}

#[test]
fn ifc_block_sibling_starts_new_line() {
    let inline = |id: &str, x: f32| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineBlock),
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
            display: Some(DisplaySpec::Block),
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
            display: Some(DisplaySpec::Block),
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
        (boxes["mid"].y - 20.0).abs() < 0.5,
        "block sibling must break the IFC line, got {:?}",
        boxes["mid"]
    );
    assert!(
        (boxes["c"].y - 40.0).abs() < 0.5,
        "inline after block starts a new line, got {:?}",
        boxes["c"]
    );
}

#[test]
fn ifc_block_in_inline_unboxes_like_block_siblings() {
    let inline_block = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineBlock),
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
            display: Some(DisplaySpec::Block),
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
            display: Some(DisplaySpec::Block),
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
        "first inline-block stays on the first line, got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["mid"].y - 20.0).abs() < 0.5,
        "block inside inline must hoist onto its own line, got {:?}",
        boxes["mid"]
    );
    assert!(
        (boxes["c"].y - 40.0).abs() < 0.5,
        "trailing inline-block starts after the hoisted block, got {:?}",
        boxes["c"]
    );
}

#[test]
fn flex_item_inline_with_block_is_not_unboxed() {
    let mid = StyleLayoutNode {
        id: "mid".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Block),
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
            display: Some(DisplaySpec::InlineBlock),
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
            display: Some(DisplaySpec::Block),
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
            display: Some(DisplaySpec::InlineBlock),
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
            display: Some(DisplaySpec::Block),
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

fn floated_box(id: &str, side: FloatSpec, width: f32, height: f32) -> StyleLayoutNode {
    StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            float: side,
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    }
}

fn inline_box(id: &str, width: f32, height: f32) -> StyleLayoutNode {
    StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineBlock),
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        },
        children: Vec::new(),
        text: None,
    }
}

#[test]
fn ifc_line_box_shrinks_around_float_left() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![
            floated_box("fl", FloatSpec::Left, 80.0, 40.0),
            inline_box("a", 50.0, 20.0),
        ],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 80.0);
    assert!((boxes["fl"].x - 0.0).abs() < 0.5);
    assert!(
        (boxes["a"].x - 80.0).abs() < 0.5 && (boxes["a"].y - 0.0).abs() < 0.5,
        "IFC line box must start after the left float, not overlap, got {:?}",
        boxes["a"]
    );
    assert!(
        boxes["a"].x + 0.5 >= boxes["fl"].x + boxes["fl"].width
            || boxes["a"].y + 0.5 >= boxes["fl"].y + boxes["fl"].height,
        "inline vs float must not overlap, a={:?} fl={:?}",
        boxes["a"],
        boxes["fl"]
    );
}

#[test]
fn ifc_inlines_wrap_in_width_beside_float() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![
            floated_box("fl", FloatSpec::Left, 80.0, 40.0),
            inline_box("a", 70.0, 20.0),
            inline_box("b", 70.0, 20.0),
        ],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 80.0);
    assert!(
        (boxes["a"].x - 80.0).abs() < 0.5 && (boxes["a"].y - 0.0).abs() < 0.5,
        "first inline sits in the shortened line, got {:?}",
        boxes["a"]
    );
    assert!(
        (boxes["b"].x - 80.0).abs() < 0.5 && (boxes["b"].y - 20.0).abs() < 0.5,
        "70+70 exceeds remaining 120 so b wraps beside the float, got {:?}",
        boxes["b"]
    );
}

#[test]
fn ifc_uses_full_width_below_float() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![
            floated_box("fl", FloatSpec::Left, 80.0, 40.0),
            inline_box("a", 70.0, 20.0),
            inline_box("b", 70.0, 20.0),
            inline_box("c", 70.0, 20.0),
        ],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 80.0);
    assert!(
        (boxes["c"].x - 0.0).abs() < 0.5 && (boxes["c"].y - 40.0).abs() < 0.5,
        "below the float the line box is full width, got {:?}",
        boxes["c"]
    );
}

#[test]
fn ifc_oversized_inline_drops_below_float() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![
            floated_box("fl", FloatSpec::Left, 80.0, 40.0),
            inline_box("a", 150.0, 20.0),
        ],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 80.0);
    assert!(
        (boxes["a"].x - 0.0).abs() < 0.5 && (boxes["a"].y - 40.0).abs() < 0.5,
        "item wider than remaining width must drop below the float, got {:?}",
        boxes["a"]
    );
}

#[test]
fn ifc_line_box_shrinks_between_left_and_right_floats() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(300.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![
            floated_box("left", FloatSpec::Left, 80.0, 40.0),
            floated_box("right", FloatSpec::Right, 80.0, 40.0),
            inline_box("a", 40.0, 20.0),
        ],
        text: None,
    };
    let boxes = box_map(&tree, 300.0, 80.0);
    assert!((boxes["left"].x - 0.0).abs() < 0.5);
    assert!((boxes["right"].x - 220.0).abs() < 0.5);
    assert!(
        (boxes["a"].x - 80.0).abs() < 0.5 && (boxes["a"].y - 0.0).abs() < 0.5,
        "line box sits between left and right floats, got {:?}",
        boxes["a"]
    );
    assert!(
        boxes["a"].x + boxes["a"].width <= boxes["right"].x + 0.5,
        "inline must not overlap the right float, a={:?} right={:?}",
        boxes["a"],
        boxes["right"]
    );
}

#[test]
fn in_flow_block_does_not_shrink_beside_float() {
    let tree = StyleLayoutNode {
        id: "root".into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..LayoutStyle::default()
        },
        children: vec![
            floated_box("fl", FloatSpec::Left, 80.0, 40.0),
            StyleLayoutNode {
                id: "block".into(),
                style: LayoutStyle {
                    display: Some(DisplaySpec::Block),
                    width: Some(LengthSpec::Px(100.0)),
                    height: Some(LengthSpec::Px(20.0)),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            },
        ],
        text: None,
    };
    let boxes = box_map(&tree, 200.0, 80.0);
    assert!(
        (boxes["block"].x - 0.0).abs() < 0.5 && (boxes["block"].y - 0.0).abs() < 0.5,
        "block formatting does not shrink beside floats, got {:?}",
        boxes["block"]
    );
}

#[test]
fn writing_mode_vertical_rl_ifc_advances_inline_down_block_from_right() {
    let inline = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineBlock),
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
            display: Some(DisplaySpec::Block),
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
            display: Some(DisplaySpec::InlineBlock),
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
            display: Some(DisplaySpec::Block),
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
fn writing_mode_vertical_rtl_skips_inline_reverse() {
    let inline = |id: &str| StyleLayoutNode {
        id: id.into(),
        style: LayoutStyle {
            display: Some(DisplaySpec::InlineBlock),
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
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(80.0)),
            height: Some(LengthSpec::Px(80.0)),
            writing_mode: Some(WritingModeSpec::VerticalRl),
            dir: Some(DirSpec::Rtl),
            ..LayoutStyle::default()
        },
        children: vec![inline("a"), inline("b")],
        text: None,
    };
    let boxes = box_map(&tree, 80.0, 80.0);
    assert!(
        (boxes["a"].y - 0.0).abs() < 0.5 && (boxes["b"].y - 40.0).abs() < 0.5,
        "RTL + vertical is skipped: inlines still go top-to-bottom, got a={:?} b={:?}",
        boxes["a"],
        boxes["b"]
    );
}

#[test]
fn writing_mode_vertical_shaper_keeps_horizontal_metrics() {
    let metrics = crate::MeasureTextShaper.shape(
        id(1),
        &TextContent {
            value: "Hello".into(),
        },
        &ComputedStyle {
            writing_mode: WritingModeSpec::VerticalRl,
            font_size: 10.0,
            ..ComputedStyle::default()
        },
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
            grid_placement: GridPlacement {
                column_start: GridLine::NthName("foo".into(), 2),
                column_end: GridLine::Name("foo".into()),
                ..GridPlacement::default()
            },
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
            grid_columns: Some(vec![GridTrack::Px(80.0), GridTrack::Px(120.0)]),
            grid_column_line_names: Some(vec![
                vec!["foo".into()],
                vec!["foo".into()],
                vec!["foo".into()],
            ]),
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
            grid_placement: GridPlacement {
                column_start: GridLine::NthName("mid".into(), 2),
                column_end: GridLine::Name("mid".into()),
                ..GridPlacement::default()
            },
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
            grid_columns_repeat: Some(GridRepeatAuto {
                kind: GridTrackListUnsupported::RepeatAutoFill,
                tracks: vec![GridTrack::Px(80.0)],
                pattern_line_names: vec![vec!["mid".into()], Vec::new()],
                ..Default::default()
            }),
            // Pattern stored once — engine must expand, not resolve mid 2
            // against this single copy (which would miss and auto-place at 0).
            grid_column_line_names: Some(vec![vec!["mid".into()], Vec::new()]),
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
