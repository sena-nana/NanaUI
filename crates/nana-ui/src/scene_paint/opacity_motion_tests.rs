//! Compare a whole-card fade with the fully composed card, not individual alphas.
use super::*;
use nana_ui_core::{
    Easing,
    motion::{AnimatableProperty, MotionTo, MotionValue},
};
use nana_ui_runtime::{AnimationId, AnimationSpec, UiWorld};
use std::time::Duration;

fn animate(world: &mut UiWorld, target: StableNodeId, from: f32, to: f32, start: u64) {
    animate_for(world, target, from, to, start, 100);
}

fn animate_for(
    world: &mut UiWorld,
    target: StableNodeId,
    from: f32,
    to: f32,
    start: u64,
    duration: u64,
) {
    let mut queue = MutationQueue::new();
    queue.start_animation(
        AnimationSpec::new(
            AnimationId::new(target.get()).unwrap(),
            target,
            Duration::from_millis(start),
            Duration::from_millis(duration),
            Duration::from_millis(16),
            Easing::Linear,
        )
        .with_property(AnimatableProperty::Opacity)
        .with_range(
            MotionValue::Scalar(from),
            MotionTo::Value(MotionValue::Scalar(to)),
        )
        .with_playback(nana_ui_runtime::AnimationPlayback::running(
            nana_ui_runtime::AnimationIteration::ONCE,
            nana_ui_runtime::AnimationDirection::Normal,
            nana_ui_runtime::AnimationFillMode::Forwards,
        )),
    );
    world.commit(queue).unwrap();
}

fn card_scene(theme: nana_ui_core::ThemeAppearance) -> (UiScene, StableNodeId) {
    use nana_ui_core::{CardKind, PositionSpec};
    use nana_ui_runtime::{Card, Stack};
    let mut cx = AppContext::new();
    cx.set_preset_theme(theme).unwrap();
    let document = DocumentId::new(1).unwrap();
    let root = cx
        .create_component(
            document,
            Stack::column(0.0).with_layout(|l| {
                l.padding = Some(LengthSpec::Px(24.0));
            }),
        )
        .unwrap();
    let card = cx
        .create_component(
            document,
            Card::new()
                .kind(CardKind::Flat)
                .padding(12.0)
                .painter(painted_outline::OutlinePainter)
                .style(
                    Stack::column(8.0)
                        .with_layout(|l| {
                            l.position = PositionSpec::Relative;
                            l.width = Some(LengthSpec::Px(360.0));
                            l.height = Some(LengthSpec::Px(260.0));
                            l.padding_top = Some(LengthSpec::Px(56.0));
                        })
                        .node_style(),
                ),
        )
        .unwrap();
    let button = cx
        .create_component(
            document,
            RuntimeButton::new("Card content")
                .icon(nana_ui_core::Icon::Close)
                .kind(ButtonKind::Selected),
        )
        .unwrap();
    cx.append_child(root, card).unwrap();
    cx.append_child(card, button).unwrap();
    let ids = [root.stable_id(), card.stable_id(), button.stable_id()];
    cx.resolve_styles(&ids).unwrap();
    cx.shape_text(&ids, &mut crate::NanaTextShaper::default())
        .unwrap();
    cx.layout_document(document, nana_ui_runtime::LayoutViewport::new(420.0, 320.0))
        .unwrap();
    let mut scene = UiScene::new();
    scene.apply_delta(cx.world().extract_document(document), []);
    (scene, card.stable_id())
}

fn assert_whole_card_fade(actual: &[u8], full: &[u8], alpha: f32, ms: u64) {
    let mut worst = (0, 0, 0, 0);
    for (i, (&actual, &full)) in actual.iter().zip(full).enumerate() {
        if i % 4 == 3 {
            continue;
        }
        let expected = (f32::from(full) * alpha).round() as u8;
        let error = actual.abs_diff(expected);
        if error > worst.0 {
            worst = (error, i, actual, expected);
        }
    }
    assert!(
        worst.0 <= 3,
        "whole-card fade @{ms}ms alpha {alpha}: worst error/index/actual/expected {worst:?}"
    );
}

#[test]
fn card_opacity_motion_composites_the_outline_and_content_once() {
    let (device, queue) = test_device();
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    let size = [420, 320];
    let logical = [420.0, 320.0];
    for theme in [
        nana_ui_core::ThemeAppearance::Light,
        nana_ui_core::ThemeAppearance::Dark,
    ] {
        let (mut scene, card) = card_scene(theme);
        let full = paint_scene_rgba(&device, &queue, &mut painter, &scene, logical, size, 1.0);
        let mut world = UiWorld::new();
        let mut create = MutationQueue::new();
        create.create(card, DocumentId::new(1).unwrap(), NodeKind::Document);
        world.commit(create).unwrap();
        // Enter, leave, interrupt a reveal with a leave, then reverse that leave.
        for (start, from, to, samples) in [
            (
                0,
                0.0,
                1.0,
                vec![
                    (0, 0.0),
                    (8, 0.08),
                    (16, 0.16),
                    (25, 0.25),
                    (50, 0.5),
                    (75, 0.75),
                    (100, 1.0),
                    (125, 1.0),
                ],
            ),
            (
                200,
                1.0,
                0.0,
                vec![
                    (200, 1.0),
                    (225, 0.75),
                    (250, 0.5),
                    (275, 0.25),
                    (300, 0.0),
                    (325, 0.0),
                ],
            ),
            (400, 0.0, 1.0, vec![(400, 0.0), (450, 0.5)]),
            (450, 0.5, 0.0, vec![(450, 0.5), (500, 0.25)]),
            (500, 0.25, 1.0, vec![(500, 0.25), (550, 0.625), (600, 1.0)]),
        ] {
            world.advance_animations(Duration::from_millis(start));
            animate(&mut world, card, from, to, start);
            for (ms, alpha) in samples {
                let now = Duration::from_millis(ms);
                world.advance_animations(now);
                scene.apply_presentation(
                    world.presentation_store(),
                    now,
                    Some(world.motion_descriptors()),
                );
                let actual =
                    paint_scene_rgba(&device, &queue, &mut painter, &scene, logical, size, 1.0);
                assert_whole_card_fade(&actual, &full, alpha, ms);
            }
        }
        // Reduced motion uses a zero-duration track, with the same endpoints.
        for (ms, from, to) in [(700, 1.0, 0.0), (800, 0.0, 1.0)] {
            let now = Duration::from_millis(ms);
            world.advance_animations(now);
            animate_for(&mut world, card, from, to, ms, 0);
            world.advance_animations(now);
            scene.apply_presentation(
                world.presentation_store(),
                now,
                Some(world.motion_descriptors()),
            );
            let actual =
                paint_scene_rgba(&device, &queue, &mut painter, &scene, logical, size, 1.0);
            assert_whole_card_fade(&actual, &full, to, ms);
        }
    }
}

#[test]
fn nested_opacity_motion_matches_static_group_composition() {
    use nana_ui_core::LayoutStyle;
    let nodes = |alpha: f32| {
        let mut nodes = vec![
            extracted_div(
                1,
                &[2],
                0.0,
                0.0,
                64.0,
                64.0,
                LayoutStyle {
                    opacity: Some(alpha),
                    ..Default::default()
                }
                .background([1.0, 0.0, 0.0, 1.0]),
                Some([1.0, 0.0, 0.0, 1.0]),
            ),
            extracted_div(
                2,
                &[3],
                8.0,
                8.0,
                40.0,
                40.0,
                LayoutStyle {
                    opacity: Some(alpha),
                    ..Default::default()
                }
                .background([0.0, 0.0, 1.0, 1.0]),
                Some([0.0, 0.0, 1.0, 1.0]),
            ),
            extracted_div(
                3,
                &[],
                16.0,
                16.0,
                16.0,
                16.0,
                LayoutStyle {
                    opacity: Some(0.35),
                    ..Default::default()
                }
                .background([0.0, 1.0, 0.0, 1.0]),
                Some([0.0, 1.0, 0.0, 1.0]),
            ),
        ];
        nodes[1].parent = Some(StableNodeId::new(1).unwrap());
        nodes[2].parent = Some(StableNodeId::new(2).unwrap());
        nodes
    };
    let mut reference = UiScene::new();
    reference.apply_delta(nodes(0.5), []);
    let mut actual = UiScene::new();
    actual.apply_delta(nodes(1.0), []);
    let mut world = UiWorld::new();
    let mut create = MutationQueue::new();
    for i in 1..=2 {
        create.create(
            StableNodeId::new(i).unwrap(),
            DocumentId::new(1).unwrap(),
            NodeKind::Document,
        );
    }
    world.commit(create).unwrap();
    for i in 1..=2 {
        animate(&mut world, StableNodeId::new(i).unwrap(), 0.0, 1.0, 0);
    }
    world.advance_animations(Duration::from_millis(50));
    actual.apply_presentation(
        world.presentation_store(),
        world.animation_now(),
        Some(world.motion_descriptors()),
    );
    let (device, queue) = test_device();
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    let expected = paint_scene_rgba(
        &device,
        &queue,
        &mut painter,
        &reference,
        [64.0; 2],
        [64; 2],
        1.0,
    );
    let actual = paint_scene_rgba(
        &device,
        &queue,
        &mut painter,
        &actual,
        [64.0; 2],
        [64; 2],
        1.0,
    );
    for (actual, expected) in actual
        .as_chunks::<4>()
        .0
        .iter()
        .zip(expected.as_chunks::<4>().0)
    {
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(a, e)| a.abs_diff(*e) <= 2),
            "nested composition {actual:?} != {expected:?}"
        );
    }
}
