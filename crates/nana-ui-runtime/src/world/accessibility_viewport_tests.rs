use super::*;
use nana_ui_core::PaintTransform;

fn node(id: u64) -> StableNodeId {
    StableNodeId::new(id).unwrap()
}
fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn fixture() -> UiWorld {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    for id in 1..=3 {
        queue.create(
            node(id),
            document(),
            NodeKind::Element { tag: "div".into() },
        );
    }
    queue.insert(node(1), node(2), None);
    queue.insert(node(2), node(3), None);
    for (id, bounds) in [
        (
            1,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 300.0,
            },
        ),
        (
            2,
            LayoutBox {
                x: 10.0,
                y: 200.0,
                width: 300.0,
                height: 200.0,
            },
        ),
        (
            3,
            LayoutBox {
                x: 30.0,
                y: 260.0,
                width: 100.0,
                height: 40.0,
            },
        ),
    ] {
        queue.write_layout(node(id), bounds);
    }
    for id in [1, 2] {
        queue.set_interaction(
            node(id),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
        );
    }
    queue.set_accessibility(
        node(3),
        AccessibilityState {
            role: AccessibilityRole::Button,
            ..Default::default()
        },
    );
    queue.set_scroll_offset(node(1), ScrollOffset { x: 4.0, y: 160.0 });
    queue.set_scroll_offset(node(2), ScrollOffset { x: 6.0, y: 20.0 });
    world.commit(queue).unwrap();
    world.rebuild_hit_test(document());
    world
}

fn button_bounds(world: &UiWorld) -> LayoutBox {
    world
        .project_accessibility(document())
        .into_iter()
        .find(|n| n.id == node(3))
        .unwrap()
        .bounds
}
fn assert_hit_at_accessible_center(world: &UiWorld) {
    let bounds = button_bounds(world);
    assert_eq!(
        world.hit_test(
            document(),
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0
        ),
        Some(node(3))
    );
}

#[test]
fn accessible_bounds_include_nested_scroll_without_changing_layout() {
    let world = fixture();
    let bounds = button_bounds(&world);
    assert_eq!(
        bounds,
        LayoutBox {
            x: 20.0,
            y: 80.0,
            width: 100.0,
            height: 40.0
        }
    );
    assert_eq!(world.layout_box(node(3)).unwrap().y, 260.0);
    assert_hit_at_accessible_center(&world);
}

#[test]
fn accessible_bounds_follow_incremental_scroll_hit_updates() {
    let mut world = fixture();
    let before = button_bounds(&world);
    let mut queue = MutationQueue::new();
    queue.set_scroll_offset(node(1), ScrollOffset { x: 4.0, y: 180.0 });
    world.commit(queue).unwrap();
    world.update_hit_test_scroll(document(), node(1), [0.0, -20.0]);
    assert_eq!(button_bounds(&world).y, before.y - 20.0);
    assert_hit_at_accessible_center(&world);
}

#[test]
fn accessible_bounds_use_the_accumulated_affine_transform() {
    let mut world = fixture();
    let mut queue = MutationQueue::new();
    queue.set_style(
        node(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                transform: Some(PaintTransform {
                    a: 1.5,
                    d: 1.5,
                    e: 15.0,
                    f: 25.0,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    queue.set_style(
        node(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                transform: Some(PaintTransform {
                    e: 12.0,
                    f: -10.0,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    world.commit(queue).unwrap();
    world.rebuild_hit_test(document());
    let bounds = button_bounds(&world);
    assert_eq!(bounds.width, 150.0);
    assert_eq!(bounds.height, 60.0);
    assert_hit_at_accessible_center(&world);
}

#[test]
fn scrolling_publishes_moved_descendants_in_accessibility_delta() {
    let mut world = fixture();
    world.take_system_work();
    let mut queue = MutationQueue::new();
    queue.set_scroll_offset(node(1), ScrollOffset { x: 4.0, y: 180.0 });
    world.commit(queue).unwrap();
    world.update_hit_test_scroll(document(), node(1), [0.0, -20.0]);
    let work = world.take_system_work();
    let delta = world.project_accessibility_delta(&work);
    let button = delta
        .updated
        .iter()
        .find(|item| item.id == node(3))
        .expect("moving a scroll ancestor updates the cached descendant bounds");
    assert_eq!(button.bounds.y, 60.0);
    assert_hit_at_accessible_center(&world);
}

#[test]
fn accessible_bounds_follow_the_hit_index_perspective_contract() {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    let bounds = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 80.0,
    };
    let matrix = PaintMat4::perspective(800.0)
        .unwrap()
        .then(PaintMat4::rotate_y(30_f32.to_radians()));
    queue.create(
        node(3),
        document(),
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.write_layout(node(3), bounds);
    queue.set_style(
        node(3),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                transform_3d: Some(matrix),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    world.commit(queue).unwrap();
    world.rebuild_hit_test(document());
    let projected = button_bounds(&world);
    let corners = matrix
        .around_origin(0.0, 0.0, 100.0, 40.0)
        .projected_corners(0.0, 0.0, 200.0, 80.0)
        .unwrap();
    let left = corners.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
    let right = corners
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!((projected.x - left).abs() < 0.001);
    assert!((projected.width - (right - left)).abs() < 0.001);
    assert_hit_at_accessible_center(&world);
}

#[test]
fn selective_and_full_projection_share_scroll_geometry_across_documents() {
    let mut world = fixture();
    let other_document = DocumentId::new(2).unwrap();
    let mut queue = MutationQueue::new();
    queue.create(
        node(4),
        other_document,
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.write_layout(
        node(4),
        LayoutBox {
            x: 40.0,
            y: 50.0,
            width: 20.0,
            height: 10.0,
        },
    );
    queue.set_style(
        node(4),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                transform: Some(PaintTransform {
                    e: 60.0,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    world.commit(queue).unwrap();
    world.rebuild_hit_test(other_document);
    let nodes = world.project_accessibility_nodes(&[node(3), node(4)]);
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].bounds, button_bounds(&world));
    assert_eq!(nodes[1].bounds.x, 100.0);
    assert_eq!(
        nodes[1].bounds,
        world.project_accessibility(other_document)[0].bounds
    );
}
