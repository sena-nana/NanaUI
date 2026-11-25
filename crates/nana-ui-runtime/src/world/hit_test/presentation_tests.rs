use super::*;
use crate::{
    AccessibilityRole, AccessibilityState, AnimatableProperty, AnimationId, AnimationSpec, Easing,
    MotionTo, MotionValue,
};
use nana_ui_core::{PaintMat4, PaintTransform};
use std::{sync::Arc, time::Duration};

fn node(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}

fn document(value: u64) -> DocumentId {
    DocumentId::new(value).unwrap()
}

fn box_at(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
    LayoutBox {
        x,
        y,
        width,
        height,
    }
}

fn transform_overlay(
    id: u64,
    target: StableNodeId,
    from: PaintTransform,
    to: PaintTransform,
) -> AnimationSpec {
    AnimationSpec::new(
        AnimationId::new(id).unwrap(),
        target,
        Duration::ZERO,
        Duration::from_millis(100),
        Duration::from_millis(16),
        Easing::Linear,
    )
    .with_property(AnimatableProperty::Transform)
    .with_range(
        MotionValue::Transform(from),
        MotionTo::Value(MotionValue::Transform(to)),
    )
}

fn width_overlay(id: u64, target: StableNodeId, from: f32, to: f32) -> AnimationSpec {
    AnimationSpec::new(
        AnimationId::new(id).unwrap(),
        target,
        Duration::ZERO,
        Duration::from_millis(100),
        Duration::from_millis(16),
        Easing::Linear,
    )
    .with_property(AnimatableProperty::Width)
    .with_range(
        MotionValue::Scalar(from),
        MotionTo::Value(MotionValue::Scalar(to)),
    )
}

fn style_with_transform(transform: PaintTransform) -> NodeStyle {
    let mut layout = LayoutStyle::default();
    layout.transforms.transform = Some(transform);
    NodeStyle {
        layout: Arc::new(layout),
        ..NodeStyle::default()
    }
}

fn commit_button(
    world: &mut UiWorld,
    layout: LayoutBox,
    style: NodeStyle,
    animation: AnimationSpec,
) {
    let mut queue = MutationQueue::new();
    queue.create(
        node(1),
        document(1),
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.write_layout(node(1), layout);
    queue.set_style(node(1), style);
    queue.set_interaction(
        node(1),
        InteractionState {
            pointer_events: true,
            focusable: true,
        },
    );
    queue.start_animation(animation);
    world.commit(queue).unwrap();
    world.advance_animations(Duration::ZERO);
    world.rebuild_hit_test(document(1));
}

#[test]
fn hit_test_follows_presentation_translate_not_logical_target() {
    let mut world = UiWorld::new();
    let target = PaintTransform {
        e: 40.0,
        ..PaintTransform::default()
    };
    commit_button(
        &mut world,
        box_at(0.0, 0.0, 40.0, 40.0),
        style_with_transform(target),
        transform_overlay(1, node(1), PaintTransform::default(), target),
    );

    let before = world.presentation_values_cpu_sampled();
    let idle = world.advance_animations(Duration::from_millis(50));
    assert!(idle.samples.is_empty());
    assert_eq!(idle.animation_deadlines_scanned, 0);
    assert_eq!(idle.animations_considered, 0);
    assert_eq!(world.presentation_values_cpu_sampled(), before);
    assert_eq!(
        world.motion_work_counters().presentation_values_cpu_sampled,
        before
    );
    assert_eq!(
        world
            .node_style(node(1))
            .unwrap()
            .layout
            .transforms
            .transform,
        Some(target),
        "logical transform stays at the target"
    );

    assert_eq!(world.hit_test(document(1), 30.0, 10.0), Some(node(1)));
    assert_eq!(world.hit_test(document(1), 70.0, 10.0), None);
    assert!(world.presentation_values_cpu_sampled() > before);

    let local = world
        .pointer_layout_position(node(1), 30.0, 10.0)
        .expect("maps through presentation");
    assert!((local.0 - 10.0).abs() < 1e-4, "got {}", local.0);
    assert!((local.1 - 10.0).abs() < 1e-4, "got {}", local.1);
}

#[test]
fn hit_test_follows_presentation_scale_not_logical_target() {
    let mut world = UiWorld::new();
    let target = PaintTransform {
        a: 2.0,
        d: 2.0,
        ..PaintTransform::default()
    };
    commit_button(
        &mut world,
        box_at(0.0, 0.0, 40.0, 40.0),
        style_with_transform(target),
        transform_overlay(1, node(1), PaintTransform::default(), target),
    );
    world.advance_animations(Duration::from_millis(50));

    // Mid scale is 1.5 around the 20,20 origin → visual box -10..50.
    // Logical scale 2 occupies -20..60, so x=55 is the logical rest, not presentation.
    assert_eq!(world.hit_test(document(1), 20.0, 20.0), Some(node(1)));
    assert_eq!(world.hit_test(document(1), 55.0, 20.0), None);
    assert_eq!(
        world.hit_test_candidates(document(1), 20.0, 20.0).first(),
        Some(&node(1))
    );
}

#[test]
fn hit_test_respects_ellipse_clip_path() {
    let mut world = UiWorld::new();
    let style = NodeStyle {
        layout: Arc::new(LayoutStyle {
            paint: nana_ui_core::PaintStyle {
                clip_path: Some(nana_ui_core::ClipPath::Circle(nana_ui_core::ClipCircle {
                    radius: nana_ui_core::ClipShapeRadius::ClosestSide,
                    cx: nana_ui_core::LengthSpec::Percent(50.0),
                    cy: nana_ui_core::LengthSpec::Percent(50.0),
                })),
                ..nana_ui_core::PaintStyle::default()
            }
            .into(),
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    };
    commit_button(
        &mut world,
        box_at(0.0, 0.0, 100.0, 100.0),
        style,
        transform_overlay(
            1,
            node(1),
            PaintTransform::default(),
            PaintTransform::default(),
        ),
    );
    assert_eq!(world.hit_test(document(1), 50.0, 50.0), Some(node(1)));
    assert_eq!(world.hit_test(document(1), 2.0, 2.0), None);
}

#[test]
fn hit_test_respects_polygon_clip_path() {
    let mut world = UiWorld::new();
    let point = |x, y| nana_ui_core::ClipPoint {
        x: nana_ui_core::LengthSpec::Percent(x),
        y: nana_ui_core::LengthSpec::Percent(y),
    };
    let style = NodeStyle {
        layout: Arc::new(LayoutStyle {
            paint: nana_ui_core::PaintStyle {
                clip_path: Some(nana_ui_core::ClipPath::Polygon(vec![
                    point(50.0, 0.0),
                    point(100.0, 100.0),
                    point(0.0, 100.0),
                ])),
                ..nana_ui_core::PaintStyle::default()
            }
            .into(),
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    };
    commit_button(
        &mut world,
        box_at(0.0, 0.0, 100.0, 100.0),
        style,
        transform_overlay(
            1,
            node(1),
            PaintTransform::default(),
            PaintTransform::default(),
        ),
    );
    assert_eq!(world.hit_test(document(1), 50.0, 20.0), Some(node(1)));
    assert_eq!(world.hit_test(document(1), 5.0, 5.0), None);
}

#[test]
fn hit_test_respects_rounded_inset_clip_path() {
    let mut world = UiWorld::new();
    let style = NodeStyle {
        layout: Arc::new(LayoutStyle {
            paint: nana_ui_core::PaintStyle {
                clip_path: Some(nana_ui_core::ClipPath::Inset(nana_ui_core::ClipInset {
                    top: nana_ui_core::LengthSpec::Px(0.0),
                    right: nana_ui_core::LengthSpec::Px(0.0),
                    bottom: nana_ui_core::LengthSpec::Px(0.0),
                    left: nana_ui_core::LengthSpec::Px(0.0),
                    round: Some(nana_ui_core::LengthSpec::Px(20.0)),
                })),
                ..nana_ui_core::PaintStyle::default()
            }
            .into(),
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    };
    commit_button(
        &mut world,
        box_at(0.0, 0.0, 100.0, 100.0),
        style,
        transform_overlay(
            1,
            node(1),
            PaintTransform::default(),
            PaintTransform::default(),
        ),
    );
    assert_eq!(world.hit_test(document(1), 50.0, 50.0), Some(node(1)));
    assert_eq!(world.hit_test(document(1), 2.0, 2.0), None);
}

#[test]
fn tiny_finite_presentation_scale_remains_hittable() {
    let transform = [0.0001, 0.0, 0.0, 0.0001, 0.0, 0.0];
    assert!(transformed_contains(
        box_at(0.0, 0.0, 1_000_000.0, 1_000_000.0),
        transform,
        [0.0, 0.0],
        50.0,
        50.0,
    ));
}

#[test]
fn layout_width_animation_keeps_logical_hit_box() {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(
        node(1),
        document(1),
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.write_layout(node(1), box_at(0.0, 0.0, 80.0, 40.0));
    queue.set_style(
        node(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(nana_ui_core::LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.start_animation(width_overlay(1, node(1), 40.0, 80.0));
    world.commit(queue).unwrap();
    world.advance_animations(Duration::from_millis(50));
    world.rebuild_hit_test(document(1));

    assert!(
        world
            .presentation_applied_value(
                node(1),
                AnimatableProperty::Width,
                Duration::from_millis(50)
            )
            .is_some()
    );
    assert_eq!(world.hit_test(document(1), 70.0, 10.0), Some(node(1)));
    assert_eq!(world.hit_test(document(1), 90.0, 10.0), None);
    let bounds = world.presentation_input_bounds(node(1)).unwrap();
    assert!((bounds.width - 80.0).abs() < 1e-4, "got {}", bounds.width);
}

#[test]
fn descendant_hit_test_follows_ancestor_presentation_transform() {
    let mut world = UiWorld::new();
    let target = PaintTransform {
        e: 40.0,
        ..PaintTransform::default()
    };
    let mut queue = MutationQueue::new();
    queue.create(
        node(1),
        document(1),
        NodeKind::Element { tag: "row".into() },
    );
    queue.create(
        node(2),
        document(1),
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.insert(node(1), node(2), None);
    queue.write_layout(node(1), box_at(0.0, 0.0, 40.0, 40.0));
    queue.write_layout(node(2), box_at(0.0, 0.0, 40.0, 40.0));
    queue.set_style(node(1), style_with_transform(target));
    queue.set_interaction(
        node(1),
        InteractionState {
            pointer_events: false,
            focusable: false,
        },
    );
    queue.start_animation(transform_overlay(
        1,
        node(1),
        PaintTransform::default(),
        target,
    ));
    world.commit(queue).unwrap();
    world.advance_animations(Duration::from_millis(50));
    world.rebuild_hit_test(document(1));

    assert_eq!(world.hit_test(document(1), 30.0, 10.0), Some(node(2)));
    assert_eq!(world.hit_test(document(1), 70.0, 10.0), None);
}

#[test]
fn hit_test_follows_projective_parent_and_motion_overlay() {
    let parent = node(1);
    let child = node(2);
    let mat = PaintMat4::perspective(800.0)
        .expect("perspective")
        .then(PaintMat4::rotate_y(30_f32.to_radians()));
    let target = PaintTransform {
        e: 60.0,
        ..PaintTransform::default()
    };
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(
        parent,
        document(1),
        NodeKind::Element {
            tag: "panel".into(),
        },
    );
    queue.create(
        child,
        document(1),
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.insert(parent, child, None);
    queue.write_layout(parent, box_at(0.0, 0.0, 128.0, 96.0));
    queue.write_layout(child, box_at(20.0, 20.0, 24.0, 24.0));
    let mut parent_layout = LayoutStyle::default();
    parent_layout.transforms.transform_3d = Some(mat);
    queue.set_style(
        parent,
        NodeStyle {
            layout: Arc::new(parent_layout),
            ..NodeStyle::default()
        },
    );
    let mut child_layout = LayoutStyle::default();
    child_layout.transforms.origin = Some(nana_ui_core::TransformOrigin {
        x: nana_ui_core::LengthSpec::Percent(25.0),
        y: nana_ui_core::LengthSpec::Percent(75.0),
    });
    queue.set_style(
        child,
        NodeStyle {
            layout: Arc::new(child_layout),
            ..NodeStyle::default()
        },
    );
    queue.set_interaction(
        parent,
        InteractionState {
            pointer_events: false,
            focusable: false,
        },
    );
    queue.set_interaction(
        child,
        InteractionState {
            pointer_events: true,
            focusable: true,
        },
    );
    queue.start_animation(transform_overlay(
        1,
        child,
        PaintTransform::default(),
        target,
    ));
    world.commit(queue).unwrap();
    world.advance_animations(Duration::from_millis(50));
    world.rebuild_hit_test(document(1));

    let forward = |([a, b, c, d, e, f], [g, h]): ([f32; 6], [f32; 2]), x: f32, y: f32| {
        let w = g * x + h * y + 1.0;
        [(a * x + c * y + e) / w, (b * x + d * y + f) / w]
    };
    let current = world
        .layout_projection_transform(child)
        .expect("presented projective transform");
    let current_center = forward(current, 32.0, 32.0);
    assert_eq!(
        world.hit_test(document(1), current_center[0], current_center[1]),
        Some(child),
        "hit-test must follow the normalized projective presentation"
    );

    let mut parent_style = LayoutStyle::default();
    parent_style.transforms.transform_3d = Some(mat);
    let parent_transform = parent_style
        .world_scene_transform(0.0, 0.0, 128.0, 96.0)
        .expect("parent projective transform");
    let old_center = forward(
        super::then_hit(
            parent_transform,
            ([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [0.0, 0.0]),
        ),
        32.0,
        32.0,
    );
    assert_ne!(
        world.hit_test(document(1), old_center[0], old_center[1]),
        Some(child),
        "old logical geometry must not remain hittable during motion"
    );
}

#[test]
fn focus_and_accessibility_bounds_follow_presentation_transform() {
    let mut world = UiWorld::new();
    let target = PaintTransform {
        e: 40.0,
        ..PaintTransform::default()
    };
    let mut queue = MutationQueue::new();
    queue.create(
        node(1),
        document(1),
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.write_layout(node(1), box_at(0.0, 0.0, 40.0, 40.0));
    queue.set_style(node(1), style_with_transform(target));
    queue.set_interaction(
        node(1),
        InteractionState {
            pointer_events: true,
            focusable: true,
        },
    );
    queue.set_accessibility(
        node(1),
        AccessibilityState {
            role: AccessibilityRole::Button,
            ..AccessibilityState::default()
        },
    );
    queue.request_focus(document(1), Some(node(1)));
    queue.start_animation(transform_overlay(
        1,
        node(1),
        PaintTransform::default(),
        target,
    ));
    world.commit(queue).unwrap();
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    world.advance_animations(Duration::from_millis(50));

    let bounds = world.focused_geometry(document(1)).expect("focus geometry");
    assert!((bounds.x - 20.0).abs() < 1e-4, "got {}", bounds.x);
    assert!((bounds.width - 40.0).abs() < 1e-4, "got {}", bounds.width);
    assert_eq!(world.presentation_input_bounds(node(1)), Some(bounds));

    let projected = world.project_accessibility_nodes(&[node(1)]);
    assert_eq!(projected.len(), 1);
    assert!((projected[0].bounds.x - 20.0).abs() < 1e-4);
    assert!((projected[0].bounds.width - 40.0).abs() < 1e-4);
}

#[test]
fn idle_compositor_frame_does_not_sample_presentation_for_hit_test() {
    let mut world = UiWorld::new();
    commit_button(
        &mut world,
        box_at(0.0, 0.0, 40.0, 40.0),
        style_with_transform(PaintTransform {
            e: 40.0,
            ..PaintTransform::default()
        }),
        transform_overlay(
            1,
            node(1),
            PaintTransform::default(),
            PaintTransform {
                e: 40.0,
                ..PaintTransform::default()
            },
        ),
    );
    let before = world.motion_work_counters().presentation_values_cpu_sampled;
    assert_eq!(before, 0);
    for ms in [10, 40, 70] {
        let frame = world.advance_animations(Duration::from_millis(ms));
        assert!(frame.samples.is_empty());
        assert_eq!(frame.animation_deadlines_scanned, 0);
        assert_eq!(
            world.motion_work_counters().presentation_values_cpu_sampled,
            before
        );
    }
}

#[test]
fn layout_width_animation_writes_px_not_scale() {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(
        node(1),
        document(1),
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.write_layout(node(1), box_at(0.0, 0.0, 80.0, 40.0));
    queue.set_style(
        node(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(nana_ui_core::LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.start_animation(width_overlay(1, node(1), 40.0, 80.0));
    world.commit(queue).unwrap();
    world.advance_animations(Duration::from_millis(50));

    let width = match world.layout_style(node(1)).unwrap().width {
        Some(nana_ui_core::LengthSpec::Px(px)) => px,
        other => panic!("expected px width, got {other:?}"),
    };
    assert!((width - 60.0).abs() < 1e-3, "got {width}");
    assert_eq!(
        world
            .node_style(node(1))
            .unwrap()
            .layout
            .transforms
            .transform,
        None,
        "width animation must not steal a scale transform"
    );
    assert!(
        world
            .presentation_applied_value(
                node(1),
                AnimatableProperty::Width,
                Duration::from_millis(50)
            )
            .is_some()
    );
}

/// Issue 326: each sample of a width animation is a sizing change of the
/// animated node, not an unknown cause that lays its whole subtree out again.
#[test]
fn a_layout_width_sample_seeds_a_sizing_change_not_a_forced_subtree() {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(
        node(1),
        document(1),
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.write_layout(node(1), box_at(0.0, 0.0, 80.0, 40.0));
    queue.set_style(
        node(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(nana_ui_core::LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.start_animation(width_overlay(1, node(1), 40.0, 80.0));
    world.commit(queue).unwrap();
    let _ = world.take_system_work();
    world.advance_animations(Duration::from_millis(50));
    let seeds = world.take_system_work().layout_frontier_seeds;
    let seed = seeds
        .iter()
        .find(|seed| seed.node == node(1))
        .expect("the sample seeds layout");
    assert_ne!(
        seed.invalidation.affected_axes,
        nana_ui_core::LayoutDependencyFootprint::ALL
    );
    assert!(
        seed.invalidation
            .changed_inputs
            .contains(nana_ui_core::LayoutStyleChange::SIZING)
    );
    assert!(
        !seed
            .invalidation
            .reason
            .intersects(nana_ui_core::InvalidationReason::UNKNOWN)
    );
}

#[test]
fn layout_length_samples_and_unbind_do_not_mark_descendants_dirty() {
    for prop in [
        AnimatableProperty::Width,
        AnimatableProperty::Height,
        AnimatableProperty::Padding,
        AnimatableProperty::Margin,
    ] {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "panel".into(),
            },
        );
        queue.create(
            node(3),
            document(1),
            NodeKind::Element {
                tag: "child".into(),
            },
        );
        queue.create(node(4), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.insert(node(3), node(4), None);
        queue.set_text(
            node(4),
            crate::TextContent {
                value: "nested".into(),
            },
        );
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(nana_ui_core::LengthSpec::Px(120.0)),
                    height: Some(nana_ui_core::LengthSpec::Px(80.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        let anim_id = AnimationId::new(10).unwrap();
        queue.start_animation(
            AnimationSpec::new(
                anim_id,
                node(2),
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(prop)
            .with_range(
                MotionValue::Scalar(20.0),
                MotionTo::Value(MotionValue::Scalar(80.0)),
            ),
        );
        world.commit(queue).unwrap();
        let _ = world.take_system_work();

        world.advance_animations(Duration::from_millis(50));
        let work = world.take_system_work();
        assert_eq!(
            work.render_extraction,
            vec![node(2)],
            "{prop:?}: only target node should be marked RENDER on sample"
        );
        assert!(
            work.input_hit_test.is_empty(),
            "{prop:?}: sample must not eagerly mark INPUT before layout writeback: {:?}",
            work.input_hit_test
        );
        assert!(
            work.accessibility.is_empty(),
            "{prop:?}: sample must not eagerly mark ACCESSIBILITY before layout writeback: {:?}",
            work.accessibility
        );
        let seed = work
            .layout_frontier_seeds
            .iter()
            .find(|seed| seed.node == node(2))
            .expect("the sample seeds layout");
        assert_ne!(
            seed.invalidation.affected_axes,
            nana_ui_core::LayoutDependencyFootprint::ALL,
            "{prop:?}: sample must emit narrow footprint"
        );

        let mut stop = MutationQueue::new();
        stop.stop_animation(anim_id);
        world.commit(stop).unwrap();
        let unbind_work = world.take_system_work();
        assert_eq!(
            unbind_work.render_extraction,
            vec![node(2)],
            "{prop:?}: unbind should mark only target RENDER"
        );
        assert!(
            unbind_work.input_hit_test.is_empty(),
            "{prop:?}: unbind must not eagerly mark INPUT: {:?}",
            unbind_work.input_hit_test
        );
        assert!(
            unbind_work.accessibility.is_empty(),
            "{prop:?}: unbind must not eagerly mark ACCESSIBILITY: {:?}",
            unbind_work.accessibility
        );
        let unbind_seed = unbind_work
            .layout_frontier_seeds
            .iter()
            .find(|seed| seed.node == node(2))
            .expect("unbind seeds layout");
        assert_ne!(
            unbind_seed.invalidation.affected_axes,
            nana_ui_core::LayoutDependencyFootprint::ALL,
            "{prop:?}: unbind must emit narrow footprint"
        );
    }
}

#[test]
fn layout_length_and_same_frame_seeds_merge_without_degrading_to_all() {
    let setup_world = || {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "panel".into(),
            },
        );
        queue.create(node(3), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.set_text(
            node(3),
            crate::TextContent {
                value: "label".into(),
            },
        );
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(nana_ui_core::LengthSpec::Px(100.0)),
                    height: Some(nana_ui_core::LengthSpec::Px(60.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let _ = world.take_system_work();
        world
    };

    let assert_narrow_seed =
        |seeds: &[crate::LayoutFrontierSeed], target: StableNodeId, case: &str| {
            let seed = seeds
                .iter()
                .find(|seed| seed.node == target)
                .unwrap_or_else(|| panic!("{case}: missing seed for {target:?}"));
            assert_ne!(
                seed.invalidation.affected_axes,
                nana_ui_core::LayoutDependencyFootprint::ALL,
                "{case}: merged seed degraded to ALL: {:?}",
                seed.invalidation
            );
            assert!(
                !seed
                    .invalidation
                    .reason
                    .intersects(nana_ui_core::InvalidationReason::UNKNOWN),
                "{case}: merged seed became UNKNOWN: {:?}",
                seed.invalidation
            );
            assert!(
                !seed.invalidation.is_unknown_forced_subtree(),
                "{case}: merged seed became forced subtree: {:?}",
                seed.invalidation
            );
        };

    let start_len = |queue: &mut MutationQueue, id: u64, prop: AnimatableProperty| {
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(id).unwrap(),
                node(2),
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(prop)
            .with_range(
                MotionValue::Scalar(40.0),
                MotionTo::Value(MotionValue::Scalar(120.0)),
            ),
        );
    };

    // 1. Width + Height animations in the same frame.
    {
        let mut world = setup_world();
        let mut queue = MutationQueue::new();
        start_len(&mut queue, 1, AnimatableProperty::Width);
        start_len(&mut queue, 2, AnimatableProperty::Height);
        world.commit(queue).unwrap();
        world.advance_animations(Duration::from_millis(50));
        let seeds = world.take_system_work().layout_frontier_seeds;
        assert_narrow_seed(&seeds, node(2), "width + height");
    }

    // 2. Width animation + SetStyle (padding + offset_top) in the same frame.
    {
        let mut world = setup_world();
        let mut queue = MutationQueue::new();
        start_len(&mut queue, 1, AnimatableProperty::Width);
        world.commit(queue).unwrap();
        let _ = world.take_system_work();

        world.advance_animations(Duration::from_millis(50));
        let mut queue = MutationQueue::new();
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(nana_ui_core::LengthSpec::Px(100.0)),
                    height: Some(nana_ui_core::LengthSpec::Px(60.0)),
                    padding: Some(nana_ui_core::LengthSpec::Px(8.0)),
                    offset_top: Some(nana_ui_core::LengthSpec::Px(4.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let seeds = world.take_system_work().layout_frontier_seeds;
        assert_narrow_seed(&seeds, node(2), "width + SetStyle");
    }

    // 3. Width animation + FontAxis animation on the same node in the same frame.
    {
        let mut world = setup_world();
        let mut queue = MutationQueue::new();
        start_len(&mut queue, 1, AnimatableProperty::Width);
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(2).unwrap(),
                node(2),
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::FontAxis(*b"wght"))
            .with_range(
                MotionValue::Scalar(400.0),
                MotionTo::Value(MotionValue::Scalar(700.0)),
            ),
        );
        world.commit(queue).unwrap();
        let _ = world.take_system_work();

        world.advance_animations(Duration::from_millis(50));
        let work = world.take_system_work();
        assert_narrow_seed(&work.layout_frontier_seeds, node(2), "width + FontAxis");
        assert!(
            work.input_hit_test.is_empty(),
            "FontAxis + Width must not eagerly mark INPUT_ELSEWHERE"
        );
    }

    // 4. Padding animation + SetText in the same frame.
    {
        let mut world = setup_world();
        let mut queue = MutationQueue::new();
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                node(3),
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Padding)
            .with_range(
                MotionValue::Scalar(2.0),
                MotionTo::Value(MotionValue::Scalar(12.0)),
            ),
        );
        world.commit(queue).unwrap();
        let _ = world.take_system_work();

        world.advance_animations(Duration::from_millis(50));
        let mut queue = MutationQueue::new();
        queue.set_text(
            node(3),
            crate::TextContent {
                value: "longer label".into(),
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        let mut shaper = crate::MeasureTextShaper;
        world.resolve_styles(&work.style).unwrap();
        world.shape_text(&work.text, &mut shaper).unwrap();
        let post_shape = world.take_system_work();
        let mut merged = work.layout_frontier_seeds;
        for seed in post_shape.layout_frontier_seeds {
            if let Some(existing) = merged.iter_mut().find(|item| item.node == seed.node) {
                existing.invalidation = existing.invalidation.merge(seed.invalidation);
            } else {
                merged.push(seed);
            }
        }
        assert_narrow_seed(&merged, node(3), "padding + SetText");
    }

    // 5. Height animation + child insertion on the same parent in the same frame.
    {
        let mut world = setup_world();
        let mut queue = MutationQueue::new();
        start_len(&mut queue, 1, AnimatableProperty::Height);
        world.commit(queue).unwrap();
        let _ = world.take_system_work();

        world.advance_animations(Duration::from_millis(50));
        let mut queue = MutationQueue::new();
        queue.create(
            node(4),
            document(1),
            NodeKind::Element {
                tag: "extra".into(),
            },
        );
        queue.insert(node(2), node(4), None);
        world.commit(queue).unwrap();
        let seeds = world.take_system_work().layout_frontier_seeds;
        assert_narrow_seed(&seeds, node(2), "height + child insertion");
    }
}

#[test]
fn layout_length_minimal_text_constraint_invalidation() {
    let build = |definite_height: bool| {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(node(2), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.set_text(
            node(2),
            crate::TextContent {
                value: "hello".into(),
            },
        );
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(nana_ui_core::LengthSpec::Px(100.0)),
                    height: definite_height.then_some(nana_ui_core::LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let _ = world.take_system_work();
        world
    };

    // Margin and Width do not eagerly bump TextDirty::CONSTRAINT before layout.
    for prop in [AnimatableProperty::Margin, AnimatableProperty::Width] {
        let mut world = build(true);
        let before = world.text_revisions(node(2)).unwrap().constraint;
        let mut queue = MutationQueue::new();
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                node(2),
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(prop)
            .with_range(
                MotionValue::Scalar(10.0),
                MotionTo::Value(MotionValue::Scalar(50.0)),
            ),
        );
        world.commit(queue).unwrap();
        world.advance_animations(Duration::from_millis(50));
        assert_eq!(
            world.text_revisions(node(2)).unwrap().constraint,
            before,
            "{prop:?}: must not eagerly invalidate text constraint before layout"
        );
    }

    // Height when already definite: does not eagerly bump constraint revision.
    {
        let mut world = build(true);
        let before = world.text_revisions(node(2)).unwrap().constraint;
        let mut queue = MutationQueue::new();
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                node(2),
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Height)
            .with_range(
                MotionValue::Scalar(40.0),
                MotionTo::Value(MotionValue::Scalar(80.0)),
            ),
        );
        world.commit(queue).unwrap();
        world.advance_animations(Duration::from_millis(50));
        assert_eq!(
            world.text_revisions(node(2)).unwrap().constraint,
            before,
            "definite Height animation must not eagerly bump constraint revision"
        );
    }

    // Height when auto -> definite -> auto: bumps on auto<->definite transitions only.
    {
        let mut world = build(false);
        let before = world.text_revisions(node(2)).unwrap().constraint;
        let anim = AnimationId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.start_animation(
            AnimationSpec::new(
                anim,
                node(2),
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Height)
            .with_range(
                MotionValue::Scalar(40.0),
                MotionTo::Value(MotionValue::Scalar(80.0)),
            ),
        );
        world.commit(queue).unwrap();
        world.advance_animations(Duration::from_millis(20));
        let after_first = world.text_revisions(node(2)).unwrap().constraint;
        assert!(
            after_first > before,
            "auto -> definite height transition must bump constraint revision"
        );
        world.advance_animations(Duration::from_millis(60));
        assert_eq!(
            world.text_revisions(node(2)).unwrap().constraint,
            after_first,
            "steady definite height sample must not eagerly bump constraint revision"
        );
        let mut stop = MutationQueue::new();
        stop.stop_animation(anim);
        world.commit(stop).unwrap();
        assert!(
            world.text_revisions(node(2)).unwrap().constraint > after_first,
            "definite -> auto height unbind must bump constraint revision"
        );
    }

    // Padding: eagerly bumps constraint revision on the target itself.
    {
        let mut world = build(true);
        let before = world.text_revisions(node(2)).unwrap().constraint;
        let mut queue = MutationQueue::new();
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                node(2),
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Padding)
            .with_range(
                MotionValue::Scalar(0.0),
                MotionTo::Value(MotionValue::Scalar(16.0)),
            ),
        );
        world.commit(queue).unwrap();
        world.advance_animations(Duration::from_millis(50));
        assert!(
            world.text_revisions(node(2)).unwrap().constraint > before,
            "Padding animation must invalidate target constraint revision"
        );
    }
}

#[test]
fn flip_play_keeps_last_layout_and_hit_test_follows_presentation() {
    let mut world = UiWorld::new();
    let first = nana_ui_core::FlipRect::new(10.0, 0.0, 40.0, 40.0);
    let last = nana_ui_core::FlipRect::new(40.0, 0.0, 40.0, 40.0);
    let mut queue = MutationQueue::new();
    queue.create(
        node(1),
        document(1),
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.write_layout(node(1), box_at(40.0, 0.0, 40.0, 40.0));
    queue.set_style(node(1), NodeStyle::default());
    queue.set_interaction(
        node(1),
        InteractionState {
            pointer_events: true,
            focusable: true,
        },
    );
    queue.start_layout_flip(
        node(1),
        first,
        last,
        Duration::ZERO,
        Duration::from_millis(100),
        crate::Easing::Linear,
    );
    world.commit(queue).unwrap();
    world.rebuild_hit_test(document(1));

    assert_eq!(world.layout_box(node(1)).unwrap().x, 40.0);
    assert_eq!(
        world
            .node_style(node(1))
            .unwrap()
            .layout
            .transforms
            .transform,
        None
    );
    match world.presentation_applied_value(node(1), AnimatableProperty::Transform, Duration::ZERO) {
        Some(MotionValue::Transform(transform)) => {
            assert!((transform.e + 30.0).abs() < 1e-3, "got {}", transform.e);
            assert_eq!(transform.a, 1.0);
        }
        other => panic!("expected invert overlay, got {other:?}"),
    }

    assert_eq!(
        world.hit_test(document(1), 20.0, 10.0),
        Some(node(1)),
        "hit-test follows presentation invert"
    );
    assert_eq!(
        world.hit_test(document(1), 70.0, 10.0),
        None,
        "Last box is not hittable until Play reaches identity"
    );

    world.advance_animations(Duration::from_millis(100));
    world.rebuild_hit_test(document(1));
    match world.presentation_applied_value(
        node(1),
        AnimatableProperty::Transform,
        Duration::from_millis(100),
    ) {
        Some(MotionValue::Transform(transform)) => {
            assert!(transform.e.abs() < 1e-3, "got {}", transform.e);
        }
        None => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(world.hit_test(document(1), 70.0, 10.0), Some(node(1)));
    assert_eq!(world.layout_box(node(1)).unwrap().x, 40.0);
}

#[test]
fn flip_animate_size_uses_layout_class_not_scale() {
    let mut world = UiWorld::new();
    let first = nana_ui_core::FlipRect::new(0.0, 0.0, 40.0, 20.0);
    let last = nana_ui_core::FlipRect::new(80.0, 0.0, 80.0, 20.0);
    let mut queue = MutationQueue::new();
    queue.create(
        node(1),
        document(1),
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    queue.write_layout(node(1), box_at(80.0, 0.0, 80.0, 20.0));
    queue.set_style(
        node(1),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(nana_ui_core::LengthSpec::Px(80.0)),
                height: Some(nana_ui_core::LengthSpec::Px(20.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue
        .node(node(1), Duration::ZERO)
        .flip(first, last)
        .animate_size()
        .duration(Duration::from_millis(100))
        .ease(crate::Easing::Linear);
    world.commit(queue).unwrap();
    world.advance_animations(Duration::from_millis(50));

    assert_eq!(world.layout_box(node(1)).unwrap().x, 80.0);
    let width = match world.layout_style(node(1)).unwrap().width {
        Some(nana_ui_core::LengthSpec::Px(px)) => px,
        other => panic!("expected px width, got {other:?}"),
    };
    assert!((width - 60.0).abs() < 1e-3, "got {width}");
    match world.presentation_applied_value(
        node(1),
        AnimatableProperty::Transform,
        Duration::from_millis(50),
    ) {
        Some(MotionValue::Transform(transform)) => {
            assert_eq!(transform.a, 1.0);
            assert_eq!(transform.d, 1.0);
            assert!((transform.e + 40.0).abs() < 1e-3, "got {}", transform.e);
        }
        other => panic!("expected FLIP translate overlay, got {other:?}"),
    }
}

/// `preserve-3d` on the parent fails the child's `matrix3d` closed, and paint
/// refuses it. The pointer has to refuse it too: an overlay is sampled on the
/// way to a node's local transform, and taking it before the refusal would
/// leave a click landing where the node is not drawn.
#[test]
fn a_closed_3d_context_refuses_a_presented_transform_for_the_pointer_too() {
    let sampled = |overlay: bool| {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element {
                tag: "stage".into(),
            },
        );
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "card".into() },
        );
        queue.insert(node(1), node(2), None);
        queue.write_layout(node(1), box_at(0.0, 0.0, 80.0, 40.0));
        queue.write_layout(node(2), box_at(0.0, 0.0, 40.0, 40.0));
        let mut persp_layout = LayoutStyle::default();
        persp_layout.transforms.preserve_3d = true;
        queue.set_style(
            node(1),
            NodeStyle {
                layout: Arc::new(persp_layout),
                ..NodeStyle::default()
            },
        );
        let mut child_layout = LayoutStyle::default();
        child_layout.transforms.transform_3d = Some(
            nana_ui_core::PaintMat4::perspective(800.0)
                .unwrap()
                .then(nana_ui_core::PaintMat4::rotate_y(30_f32.to_radians())),
        );
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(child_layout),
                ..NodeStyle::default()
            },
        );
        if overlay {
            queue.start_animation(transform_overlay(
                1,
                node(2),
                PaintTransform::default(),
                PaintTransform {
                    e: 40.0,
                    ..PaintTransform::default()
                },
            ));
        }
        world.commit(queue).unwrap();
        world.advance_animations(Duration::from_millis(50));
        world.rebuild_hit_test(document(1));
        world
            .presentation_input_bounds(node(2))
            .expect("input bounds")
    };

    // No overlay: the closed context already refuses the node's own 3D
    // transform, so its input box is its plain layout box.
    let refused = sampled(false);
    assert_eq!(refused, box_at(0.0, 0.0, 40.0, 40.0));

    // With one: still refused. A node the rule has flattened must not be
    // moved by an overlay the pointer samples and paint does not.
    assert_eq!(
        sampled(true),
        refused,
        "a presented transform reopened a closed 3D context for the pointer"
    );
}

#[test]
fn parent_perspective_and_perspective_origin_drive_child_trapezoid_hit_test() {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(
        node(1),
        document(1),
        NodeKind::Element {
            tag: "stage".into(),
        },
    );
    queue.create(
        node(2),
        document(1),
        NodeKind::Element { tag: "card".into() },
    );
    queue.insert(node(1), node(2), None);
    queue.write_layout(node(1), box_at(0.0, 0.0, 200.0, 200.0));
    queue.write_layout(node(2), box_at(50.0, 50.0, 100.0, 100.0));

    let mut stage_layout = LayoutStyle::default();
    stage_layout.transforms.css_perspective = Some(200.0);
    stage_layout.transforms.perspective_origin = Some(nana_ui_core::TransformOrigin {
        x: nana_ui_core::LengthSpec::Px(100.0),
        y: nana_ui_core::LengthSpec::Px(100.0),
    });
    queue.set_style(
        node(1),
        NodeStyle {
            layout: Arc::new(stage_layout),
            ..NodeStyle::default()
        },
    );

    let mut card_layout = LayoutStyle::default();
    card_layout.transforms.transform_3d = Some(PaintMat4::rotate_y(45_f32.to_radians()));
    queue.set_style(
        node(2),
        NodeStyle {
            layout: Arc::new(card_layout),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    world.rebuild_hit_test(document(1));

    // Right edge (x = 150) rotates away (-z) and shrinks toward y = 100, so
    // the original top-right corner (145, 55) falls outside the trapezoid.
    assert_ne!(
        world.hit_test_candidates(document(1), 145.0, 55.0).first(),
        Some(&node(2)),
        "untransformed corner outside projected trapezoid must miss"
    );
    // Left edge (x = 50) rotates toward (+z) and grows taller past y = 50..150,
    // so (58, 42) falls inside the flared left half of the trapezoid.
    assert_eq!(
        world.hit_test_candidates(document(1), 58.0, 42.0).first(),
        Some(&node(2)),
        "point inside flared trapezoid half must hit"
    );
}

#[test]
fn backface_visibility_hidden_refuses_hits_when_rotated_past_90_deg() {
    let hit_at_angle = |deg: f32| {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element {
                tag: "stage".into(),
            },
        );
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "card".into() },
        );
        queue.create(
            node(3),
            document(1),
            NodeKind::Element {
                tag: "child".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.write_layout(node(1), box_at(0.0, 0.0, 200.0, 200.0));
        queue.write_layout(node(2), box_at(50.0, 50.0, 100.0, 100.0));
        queue.write_layout(node(3), box_at(60.0, 60.0, 80.0, 80.0));

        let mut stage_layout = LayoutStyle::default();
        stage_layout.transforms.css_perspective = Some(800.0);
        queue.set_style(
            node(1),
            NodeStyle {
                layout: Arc::new(stage_layout),
                ..NodeStyle::default()
            },
        );

        let mut card_layout = LayoutStyle::default();
        card_layout.transforms.transform_3d = Some(PaintMat4::rotate_y(deg.to_radians()));
        card_layout.transforms.backface_visibility = nana_ui_core::BackfaceVisibility::Hidden;
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(card_layout),
                ..NodeStyle::default()
            },
        );

        world.commit(queue).unwrap();
        world.rebuild_hit_test(document(1));
        world.hit_test_candidates(document(1), 100.0, 100.0)
    };

    let front = hit_at_angle(30.0);
    assert_eq!(front.first(), Some(&node(3)), "front-facing child hits");
    let back = hit_at_angle(180.0);
    assert!(
        !back.contains(&node(2)) && !back.contains(&node(3)),
        "back-facing card and its flattened child must not hit: {back:?}"
    );
}

#[test]
fn shared_element_handoff_keeps_last_layout_tracks_hit_test_a11y_and_fires_completion_deadline() {
    let mut world = UiWorld::new();
    let doc = document(1);
    let key = nana_ui_core::SharedElementKey::new("hero-card");
    let policy =
        nana_ui_core::SharedElementTransition::new(Duration::from_millis(100)).ease(Easing::Linear);

    let mut q1 = MutationQueue::new();
    q1.create(
        node(1),
        doc,
        NodeKind::Element {
            tag: "thumb".into(),
        },
    );
    q1.write_layout(node(1), box_at(20.0, 20.0, 40.0, 40.0));
    q1.set_style(node(1), NodeStyle::default());
    q1.register_shared_element(node(1), key.clone(), policy);
    world.commit(q1).unwrap();

    // Unmount source node A while mounting target node B at Last (160, 20, 40, 40).
    let mut q2 = MutationQueue::new();
    q2.despawn_subtree(node(1));
    q2.create(
        node(2),
        doc,
        NodeKind::Element {
            tag: "button".into(),
        },
    );
    q2.write_layout(node(2), box_at(160.0, 20.0, 40.0, 40.0));
    q2.set_style(node(2), NodeStyle::default());
    q2.set_interaction(
        node(2),
        InteractionState {
            pointer_events: true,
            focusable: true,
        },
    );
    q2.set_accessibility(
        node(2),
        AccessibilityState {
            role: AccessibilityRole::Button,
            ..AccessibilityState::default()
        },
    );
    q2.claim_shared_element(node(2), key.clone(), policy, Duration::ZERO);
    world.commit(q2).unwrap();

    let completed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let completed_sink = Arc::clone(&completed);
    world.on_shared_element_complete(node(2), move |target, kind| {
        completed_sink.lock().unwrap().push((target, kind));
    });

    // Authoritative logical layout stays Last without transform mutation.
    assert_eq!(world.layout_box(node(2)).unwrap().x, 160.0);
    assert_eq!(
        world
            .node_style(node(2))
            .unwrap()
            .layout
            .transforms
            .transform,
        None
    );

    // Compositor transform starts at invert(First - Last) without scale, and #87
    // schedules only the completion deadline (100ms).
    let initial_frame = world.advance_animations(Duration::ZERO);
    assert_eq!(
        initial_frame.next_deadline,
        Some(Duration::from_millis(100))
    );
    world.rebuild_hit_test(doc);
    assert_eq!(
        world.hit_test(doc, 35.0, 35.0),
        Some(node(2)),
        "hit-test at t=0 follows First origin via presentation invert"
    );
    assert_eq!(world.hit_test(doc, 175.0, 35.0), None);
    let a11y_start = world.project_accessibility_nodes(&[node(2)]);
    assert!((a11y_start[0].bounds.x - 20.0).abs() < 1e-3);

    // Mid-flight at 50ms: compositor track scans zero CPU deadlines while HitIndex
    // and AccessKit sample live presentation geometry at x=90.
    let mid_frame = world.advance_animations(Duration::from_millis(50));
    assert!(mid_frame.samples.is_empty());
    assert_eq!(mid_frame.animation_deadlines_scanned, 0);
    assert!(completed.lock().unwrap().is_empty());
    world.rebuild_hit_test(doc);
    assert_eq!(world.hit_test(doc, 105.0, 35.0), Some(node(2)));
    let a11y_mid = world.project_accessibility_nodes(&[node(2)]);
    assert!((a11y_mid[0].bounds.x - 90.0).abs() < 1e-3);
    assert_eq!(world.layout_box(node(2)).unwrap().x, 160.0);

    // #87 completion deadline at 100ms fires event + callback and lands at Last.
    let end_frame = world.advance_animations(Duration::from_millis(100));
    assert_eq!(
        end_frame.events,
        vec![crate::AnimationEvent {
            id: crate::component_animation_id(crate::component_animation_kinds::FLIP, node(2))
                .unwrap(),
            target: node(2),
            kind: crate::AnimationEventKind::Finished,
        }]
    );
    assert_eq!(
        *completed.lock().unwrap(),
        vec![(node(2), crate::AnimationEventKind::Finished)]
    );
    world.rebuild_hit_test(doc);
    assert_eq!(world.hit_test(doc, 175.0, 35.0), Some(node(2)));
    let a11y_end = world.project_accessibility_nodes(&[node(2)]);
    assert!((a11y_end[0].bounds.x - 160.0).abs() < 1e-3);
}

#[test]
fn shared_element_size_modes_morph_without_width_to_scale() {
    let mut world = UiWorld::new();
    let doc = document(1);

    let last_style = NodeStyle {
        layout: Arc::new(LayoutStyle {
            width: Some(nana_ui_core::LengthSpec::Px(120.0)),
            height: Some(nana_ui_core::LengthSpec::Px(60.0)),
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    };

    // 1) LayoutSize: Layout-class Width/Height animate while Transform keeps a=1, d=1.
    let layout_policy = nana_ui_core::SharedElementTransition::new(Duration::from_millis(100))
        .ease(Easing::Linear)
        .size_mode(nana_ui_core::SharedElementSizeMode::LayoutSize);
    let mut q1 = MutationQueue::new();
    q1.create(node(1), doc, NodeKind::Element { tag: "a".into() });
    q1.write_layout(node(1), box_at(10.0, 10.0, 40.0, 20.0));
    q1.set_style(node(1), NodeStyle::default());
    q1.register_shared_element(node(1), "hero-size", layout_policy);
    world.commit(q1).unwrap();

    let mut q2 = MutationQueue::new();
    q2.despawn_subtree(node(1));
    q2.create(node(2), doc, NodeKind::Element { tag: "b".into() });
    q2.write_layout(node(2), box_at(90.0, 30.0, 120.0, 60.0));
    q2.set_style(node(2), last_style.clone());
    q2.claim_shared_element(node(2), "hero-size", layout_policy, Duration::ZERO);
    world.commit(q2).unwrap();

    world.advance_animations(Duration::from_millis(50));
    match world.presentation_applied_value(
        node(2),
        AnimatableProperty::Transform,
        Duration::from_millis(50),
    ) {
        Some(MotionValue::Transform(t)) => {
            assert_eq!((t.a, t.b, t.c, t.d), (1.0, 0.0, 0.0, 1.0));
            assert!((t.e + 40.0).abs() < 1e-3, "got {}", t.e);
            assert!((t.f + 10.0).abs() < 1e-3, "got {}", t.f);
        }
        other => panic!("expected translate-only Transform, got {other:?}"),
    }
    let width_mid = match world.layout_style(node(2)).unwrap().width {
        Some(nana_ui_core::LengthSpec::Px(px)) => px,
        other => panic!("expected interpolated px width, got {other:?}"),
    };
    let height_mid = match world.layout_style(node(2)).unwrap().height {
        Some(nana_ui_core::LengthSpec::Px(px)) => px,
        other => panic!("expected interpolated px height, got {other:?}"),
    };
    assert!((width_mid - 80.0).abs() < 1e-3, "got {width_mid}");
    assert!((height_mid - 40.0).abs() < 1e-3, "got {height_mid}");
    assert_eq!(
        world.node_style(node(2)).unwrap().layout.width,
        Some(nana_ui_core::LengthSpec::Px(120.0)),
        "logical width must stay Last"
    );
    world.advance_animations(Duration::from_millis(100));
    assert_eq!(
        world.layout_style(node(2)).unwrap().width,
        Some(nana_ui_core::LengthSpec::Px(120.0))
    );

    // 2) ClipPresentation & CrossFade: zero scale mutation and expected compositor tracks.
    let clip_policy = nana_ui_core::SharedElementTransition::new(Duration::from_millis(100))
        .ease(Easing::Linear)
        .size_mode(nana_ui_core::SharedElementSizeMode::ClipPresentation)
        .cross_fade(true);
    let mut q3 = MutationQueue::new();
    q3.create(node(3), doc, NodeKind::Element { tag: "c".into() });
    q3.write_layout(node(3), box_at(0.0, 0.0, 30.0, 30.0));
    q3.set_style(node(3), NodeStyle::default());
    q3.register_shared_element(node(3), "hero-clip", clip_policy);
    world.commit(q3).unwrap();

    let mut q4 = MutationQueue::new();
    q4.despawn_subtree(node(3));
    q4.create(node(4), doc, NodeKind::Element { tag: "d".into() });
    q4.write_layout(node(4), box_at(100.0, 0.0, 120.0, 60.0));
    q4.set_style(node(4), last_style);
    q4.claim_shared_element(
        node(4),
        "hero-clip",
        clip_policy,
        Duration::from_millis(100),
    );
    world.commit(q4).unwrap();

    world.advance_animations(Duration::from_millis(150));
    match world.presentation_applied_value(
        node(4),
        AnimatableProperty::Clip,
        Duration::from_millis(150),
    ) {
        Some(MotionValue::Scalar(p)) => assert!((p - 0.5).abs() < 1e-3, "got {p}"),
        other => panic!("expected Clip presentation track, got {other:?}"),
    }
    match world.presentation_applied_value(
        node(4),
        AnimatableProperty::Opacity,
        Duration::from_millis(150),
    ) {
        Some(MotionValue::Scalar(o)) => assert!((o - 0.5).abs() < 1e-3, "got {o}"),
        other => panic!("expected CrossFade opacity track, got {other:?}"),
    }
    match world.presentation_applied_value(
        node(4),
        AnimatableProperty::Transform,
        Duration::from_millis(150),
    ) {
        Some(MotionValue::Transform(t)) => {
            assert_eq!(
                (t.a, t.d),
                (1.0, 1.0),
                "never encode width mismatch as scale"
            );
            assert!((t.e + 50.0).abs() < 1e-3, "got {}", t.e);
        }
        other => panic!("expected translate-only Transform, got {other:?}"),
    }
}
