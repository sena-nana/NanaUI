use std::sync::Arc;

use nana_ui_runtime::{
    AccessibilityRole, AccessibilityState, DocumentId, LayoutBox, LayoutStyle, LengthSpec,
    MutationQueue, NodeKind, NodeStyle, OverflowSpec, PositionSpec, ScrollOffset, StableNodeId,
    UiWorld,
};
use nana_ui_scene::{SceneRect, UiScene};

fn id(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}
fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}
fn rect(y: f32, height: f32) -> LayoutBox {
    LayoutBox {
        x: 0.0,
        y,
        width: 100.0,
        height,
    }
}
fn style(layout: LayoutStyle) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        ..Default::default()
    }
}

fn page() -> (UiWorld, UiScene) {
    let mut world = UiWorld::new();
    let mut q = MutationQueue::new();
    q.create(id(1), document(), NodeKind::Document);
    for n in 2..=6 {
        q.create(id(n), document(), NodeKind::Element { tag: "div".into() });
    }
    q.insert(id(1), id(2), None);
    q.insert(id(2), id(3), None);
    q.insert(id(3), id(4), None);
    q.insert(id(4), id(5), None);
    q.insert(id(2), id(6), None);
    q.set_style(
        id(2),
        style(LayoutStyle {
            overflow_y: OverflowSpec::Scroll,
            ..Default::default()
        }),
    );
    q.set_style(
        id(4),
        style(LayoutStyle {
            position: PositionSpec::Sticky,
            offset_top: Some(LengthSpec::Px(0.0)),
            background: Some([1.0, 0.0, 0.0, 1.0]),
            z_index: Some(1),
            ..Default::default()
        }),
    );
    q.set_accessibility(
        id(4),
        AccessibilityState {
            role: AccessibilityRole::Button,
            ..Default::default()
        },
    );
    for (n, bounds) in [
        (1, rect(0.0, 300.0)),
        (2, rect(0.0, 100.0)),
        (3, rect(0.0, 200.0)),
        (4, rect(20.0, 20.0)),
        (5, rect(25.0, 10.0)),
        (6, rect(200.0, 200.0)),
    ] {
        q.write_layout(id(n), bounds);
    }
    world.commit(q).unwrap();
    world
        .resolve_styles(&world.document_order(document()))
        .unwrap();
    world.rebuild_hit_test(document());
    world.take_system_work();
    let mut scene = UiScene::default();
    scene.apply_delta(world.extract_document(document()), []);
    (world, scene)
}

fn scroll(world: &mut UiWorld, scene: &mut UiScene, offset: f32) {
    scroll_to(world, scene, ScrollOffset { x: 0.0, y: offset });
}

fn scroll_to(world: &mut UiWorld, scene: &mut UiScene, offset: ScrollOffset) {
    let mut q = MutationQueue::new();
    q.set_scroll_offset(id(2), offset);
    world.commit(q).unwrap();
    let work = world.take_system_work();
    assert!(
        work.layout_frontier_seeds.is_empty(),
        "scroll must not measure or place"
    );
    assert!(work.text.is_empty(), "scroll must not shape");
    for (scroller, delta) in world.take_scroll_hit_updates() {
        world.update_hit_test_scroll(document(), scroller, delta);
    }
    scene.apply_delta(world.extract_nodes(&work.render_extraction), []);
}

fn assert_y(world: &UiWorld, scene: &UiScene, node: u64, expected: f32) {
    assert_eq!(
        world.presentation_input_bounds(id(node)).unwrap().y,
        expected
    );
    assert_eq!(scene.draw_node_bounds(id(node)).unwrap().y, expected);
    assert_eq!(
        world.project_accessibility_nodes_in_viewport(&[id(node)])[0]
            .bounds
            .y,
        expected
    );
}

#[test]
fn issue200_sticky_scroll_projects_paint_hit_a11y_and_children_without_layout() {
    let (mut world, mut scene) = page();
    let generation = world.layout_generation();
    let original = world.layout_box(id(4));
    let viewport = SceneRect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };
    scene.visible_operations(viewport).unwrap();
    for (offset, expected) in [
        (10.0, 10.0),
        (70.0, 0.0),
        (190.0, -10.0),
        (70.0, 0.0),
        (0.0, 20.0),
    ] {
        scroll(&mut world, &mut scene, offset);
        assert_y(&world, &scene, 4, expected);
        assert_y(&world, &scene, 5, expected + 5.0);
        assert_eq!(world.layout_box(id(4)), original);
        assert_eq!(world.layout_generation(), generation);
        if expected >= 0.0 {
            assert_eq!(
                world.hit_test(document(), 50.0, expected + 1.0),
                Some(id(4))
            );
        }
        let mut cold = UiScene::default();
        cold.apply_delta(world.extract_document(document()), []);
        assert_eq!(scene.draw_node_bounds(id(4)), cold.draw_node_bounds(id(4)));
        assert_eq!(
            scene.visible_operations(viewport).unwrap(),
            cold.visible_operations(viewport).unwrap()
        );
    }
}

#[test]
fn issue200_sticky_insets_can_change_without_changing_identity() {
    let (mut world, mut scene) = page();
    scroll(&mut world, &mut scene, 70.0);
    let mut layout = world.node_style(id(4)).unwrap().clone();
    Arc::make_mut(&mut layout.layout).offset_top = Some(LengthSpec::Percent(10.0));
    let mut q = MutationQueue::new();
    q.set_style(id(4), layout);
    world.commit(q).unwrap();
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    world.rebuild_hit_test(document());
    scene.apply_delta(world.extract_document(document()), []);
    assert_y(&world, &scene, 4, 10.0);
    assert_eq!(world.node(id(4)).unwrap().children.as_slice(), &[id(5)]);
}

fn restyle(world: &mut UiWorld, node: u64, update: impl FnOnce(&mut LayoutStyle)) {
    let mut next = world.node_style(id(node)).unwrap().clone();
    update(Arc::make_mut(&mut next.layout));
    let mut q = MutationQueue::new();
    q.set_style(id(node), next);
    world.commit(q).unwrap();
    world
        .resolve_styles(&world.document_order(document()))
        .unwrap();
    world.rebuild_hit_test(document());
    world.take_system_work();
}

#[test]
fn issue200_sticky_horizontal_scroll_and_bottom_insets() {
    let (mut world, mut scene) = page();
    restyle(&mut world, 2, |s| {
        s.overflow_x = OverflowSpec::Scroll;
        s.overflow_y = OverflowSpec::Visible;
    });
    restyle(&mut world, 4, |s| {
        s.offset_top = None;
        s.offset_left = Some(LengthSpec::Px(0.0));
    });
    let mut q = MutationQueue::new();
    for n in 1..=6 {
        let b = world.layout_box(id(n)).unwrap();
        q.write_layout(
            id(n),
            LayoutBox {
                x: b.y,
                y: b.x,
                width: b.height,
                height: b.width,
            },
        );
    }
    world.commit(q).unwrap();
    world.rebuild_hit_test(document());
    world.take_system_work();
    scene.apply_delta(world.extract_document(document()), []);
    for (offset, expected) in [(10.0, 10.0), (70.0, 0.0), (190.0, -10.0), (0.0, 20.0)] {
        scroll_to(&mut world, &mut scene, ScrollOffset { x: offset, y: 0.0 });
        assert_eq!(world.presentation_input_bounds(id(4)).unwrap().x, expected);
        assert_eq!(scene.draw_node_bounds(id(4)).unwrap().x, expected);
    }

    let (mut world, mut scene) = page();
    restyle(&mut world, 4, |s| {
        s.offset_top = None;
        s.offset_bottom = Some(LengthSpec::Px(0.0));
    });
    let mut q = MutationQueue::new();
    q.write_layout(id(4), rect(150.0, 20.0));
    q.write_layout(id(5), rect(155.0, 10.0));
    world.commit(q).unwrap();
    world.rebuild_hit_test(document());
    world.take_system_work();
    scene.apply_delta(world.extract_document(document()), []);
    assert_y(&world, &scene, 4, 80.0);
    for (offset, expected) in [(70.0, 80.0), (150.0, 0.0), (190.0, -40.0), (0.0, 80.0)] {
        scroll(&mut world, &mut scene, offset);
        assert_y(&world, &scene, 4, expected);
    }
}

#[test]
fn issue200_sticky_without_insets_scrolls_in_normal_flow() {
    let (mut world, mut scene) = page();
    restyle(&mut world, 4, |s| s.offset_top = None);
    scene.apply_delta(world.extract_document(document()), []);
    scroll(&mut world, &mut scene, 70.0);
    assert_y(&world, &scene, 4, -50.0);
}

#[test]
fn issue200_sticky_uses_the_nearest_nested_scrollport() {
    let (mut world, mut scene) = page();
    restyle(&mut world, 3, |s| s.overflow_y = OverflowSpec::Scroll);
    let mut q = MutationQueue::new();
    q.write_layout(id(3), rect(0.0, 100.0));
    q.create(
        id(7),
        document(),
        NodeKind::Element {
            tag: "filler".into(),
        },
    );
    q.insert(id(3), id(7), None);
    q.write_layout(id(7), rect(100.0, 200.0));
    world.commit(q).unwrap();
    world.rebuild_hit_test(document());
    world.take_system_work();
    scene.apply_delta(world.extract_document(document()), []);
    let mut q = MutationQueue::new();
    q.set_scroll_offset(id(3), ScrollOffset { x: 0.0, y: 70.0 });
    world.commit(q).unwrap();
    for (scroller, delta) in world.take_scroll_hit_updates() {
        world.update_hit_test_scroll(document(), scroller, delta);
    }
    world.take_system_work();
    scene.apply_delta(world.extract_document(document()), []);
    assert_y(&world, &scene, 4, 0.0);
    scroll(&mut world, &mut scene, 30.0);
    assert_y(&world, &scene, 4, -30.0);
}

#[test]
fn issue200_nested_sticky_boxes_share_the_parents_presented_position() {
    let (mut world, mut scene) = page();
    restyle(&mut world, 5, |s| {
        s.position = PositionSpec::Sticky;
        s.offset_top = Some(LengthSpec::Px(0.0));
    });
    scene.apply_delta(world.extract_document(document()), []);
    scroll(&mut world, &mut scene, 70.0);
    assert_y(&world, &scene, 4, 0.0);
    assert_y(&world, &scene, 5, 5.0);
    assert_eq!(world.hit_test(document(), 50.0, 6.0), Some(id(5)));
}

#[test]
fn issue200_reparenting_rebinds_sticky_scroll_dependencies() {
    let (mut world, mut scene) = page();
    let mut q = MutationQueue::new();
    q.create(
        id(7),
        document(),
        NodeKind::Element {
            tag: "scroll".into(),
        },
    );
    q.create(
        id(8),
        document(),
        NodeKind::Element {
            tag: "filler".into(),
        },
    );
    q.insert(id(1), id(7), None);
    q.insert(id(7), id(8), None);
    q.set_style(
        id(7),
        style(LayoutStyle {
            overflow_y: OverflowSpec::Scroll,
            ..Default::default()
        }),
    );
    q.write_layout(id(7), rect(0.0, 100.0));
    q.write_layout(id(8), rect(200.0, 200.0));
    q.insert(id(7), id(3), None);
    world.commit(q).unwrap();
    world
        .resolve_styles(&world.document_order(document()))
        .unwrap();
    world.rebuild_hit_test(document());
    world.take_system_work();
    scene.apply_delta(world.extract_document(document()), []);
    let before = world.last_work_counters().hit_test_nodes_rebuilt;
    scroll(&mut world, &mut scene, 70.0);
    assert_y(&world, &scene, 4, 20.0);
    assert_eq!(world.last_work_counters().hit_test_nodes_rebuilt, before);
    let mut q = MutationQueue::new();
    q.set_scroll_offset(id(7), ScrollOffset { x: 0.0, y: 70.0 });
    world.commit(q).unwrap();
    for (scroller, delta) in world.take_scroll_hit_updates() {
        world.update_hit_test_scroll(document(), scroller, delta);
    }
    world.take_system_work();
    scene.apply_delta(world.extract_document(document()), []);
    assert_y(&world, &scene, 4, 0.0);
}

#[test]
fn issue200_sticky_scroll_work_does_not_grow_with_unrelated_sticky_nodes() {
    let mut costs = Vec::new();
    for count in [1_000, 10_000] {
        let (mut world, mut scene) = page();
        let mut q = MutationQueue::new();
        q.create(
            id(7),
            document(),
            NodeKind::Element {
                tag: "other-document-region".into(),
            },
        );
        q.insert(id(1), id(7), None);
        q.write_layout(id(7), rect(500.0, 100.0));
        let layout = style(LayoutStyle {
            position: PositionSpec::Sticky,
            offset_top: Some(LengthSpec::Px(0.0)),
            ..Default::default()
        });
        for n in 100..count + 100 {
            q.create(
                id(n),
                document(),
                NodeKind::Element {
                    tag: "sticky".into(),
                },
            );
            q.insert(id(7), id(n), None);
            q.set_style(id(n), layout.clone());
            q.write_layout(id(n), rect(500.0, 10.0));
        }
        world.commit(q).unwrap();
        world
            .resolve_styles(&world.document_order(document()))
            .unwrap();
        world.rebuild_hit_test(document());
        world.take_system_work();
        scene.apply_delta(world.extract_document(document()), []);
        let generation = world.layout_generation();
        for offset in [70.0, 190.0, 0.0] {
            scroll(&mut world, &mut scene, offset);
        }
        let counters = world.last_work_counters();
        assert_eq!(world.layout_generation(), generation);
        assert_eq!(counters.entities_spawned, 0);
        assert_eq!(counters.entities_despawned, 0);
        costs.push(counters.hit_test_nodes_rebuilt);
    }
    assert_eq!(
        costs,
        [Some(1), Some(1)],
        "only the sticky node needs a new hit projection: its offset is its entry's origin, and \
         its child is held relative to it"
    );
}

#[test]
fn issue200_parent_extent_change_updates_a_retained_sticky_branch() {
    let (mut world, mut scene) = page();
    scroll(&mut world, &mut scene, 190.0);
    assert_y(&world, &scene, 4, -10.0);
    let viewport = SceneRect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };
    scene.visible_operations(viewport).unwrap();
    let mut q = MutationQueue::new();
    q.write_layout(id(3), rect(0.0, 300.0));
    world.commit(q).unwrap();
    let work = world.take_system_work();
    assert!(
        work.render_extraction.contains(&id(4)),
        "a changed containing block must publish its sticky dependents"
    );
    assert!(world.rebuild_hit_test_scoped(document(), &work.input_hit_test));
    scene.apply_delta(world.extract_nodes(&work.render_extraction), []);
    assert_y(&world, &scene, 4, 0.0);
    assert_eq!(world.hit_test(document(), 50.0, 1.0), Some(id(4)));
    let mut cold = UiScene::default();
    cold.apply_delta(world.extract_document(document()), []);
    assert_eq!(
        scene.visible_operations(viewport).unwrap(),
        cold.visible_operations(viewport).unwrap()
    );
}

#[test]
fn issue200_logical_insets_follow_inherited_rtl_and_vertical_writing() {
    for (mode, direction) in [
        (
            nana_ui_runtime::WritingModeSpec::HorizontalTb,
            nana_ui_runtime::DirSpec::Rtl,
        ),
        (
            nana_ui_runtime::WritingModeSpec::VerticalRl,
            nana_ui_runtime::DirSpec::Ltr,
        ),
    ] {
        let (mut world, mut scene) = page();
        restyle(&mut world, 2, |s| {
            s.overflow_x = OverflowSpec::Scroll;
            s.overflow_y = OverflowSpec::Visible;
        });
        restyle(&mut world, 3, |s| {
            s.writing_mode = Some(mode);
            s.dir = Some(direction);
        });
        restyle(&mut world, 4, |s| {
            s.offset_top = None;
            if mode.is_vertical() {
                s.logical_inset.set(
                    nana_ui_core::LogicalEdge::BlockStart,
                    Some(LengthSpec::Px(0.0)),
                );
            } else {
                s.logical_inset.set_start(Some(LengthSpec::Px(0.0)));
            }
        });
        let mut q = MutationQueue::new();
        for (n, x, width) in [
            (2, 0.0, 100.0),
            (3, 0.0, 200.0),
            (4, 150.0, 20.0),
            (5, 155.0, 10.0),
            (6, 200.0, 200.0),
        ] {
            q.write_layout(
                id(n),
                LayoutBox {
                    x,
                    y: 0.0,
                    width,
                    height: 100.0,
                },
            );
        }
        world.commit(q).unwrap();
        world.rebuild_hit_test(document());
        world.take_system_work();
        scene.apply_delta(world.extract_document(document()), []);
        assert_eq!(world.presentation_input_bounds(id(4)).unwrap().x, 80.0);
        assert_eq!(scene.draw_node_bounds(id(4)).unwrap().x, 80.0);
        scroll_to(&mut world, &mut scene, ScrollOffset { x: 70.0, y: 0.0 });
        assert_eq!(world.presentation_input_bounds(id(4)).unwrap().x, 80.0);
        assert_eq!(scene.draw_node_bounds(id(4)).unwrap().x, 80.0);
    }
}
