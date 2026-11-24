//! A group composites over what it can change, not over the whole dest.

use super::*;

/// Two small nested opacity groups composite a few hundred pixels each, in
/// order, and nothing outside them changes.
#[test]
fn small_nested_opacity_groups_limit_composite_fill_and_keep_pixels() {
    let (device, queue) = test_device();
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    let mut scene = UiScene::new();
    scene.apply_delta(
        [
            translucent_parent(1, &[2, 3], 0.0, 0.0, 200.0, 100.0, 0.5),
            colored_quad_child(2, 1, 10.0, 10.0, 20.0, 20.0, [1.0, 0.0, 0.0, 1.0]),
            child_of(translucent_parent(3, &[4], 0.0, 0.0, 200.0, 100.0, 0.5), 1),
            colored_quad_child(4, 3, 50.0, 10.0, 20.0, 20.0, [0.0, 0.0, 1.0, 1.0]),
        ],
        [],
    );
    let pixels = paint_scene_rgba(
        &device,
        &queue,
        &mut painter,
        &scene,
        [200.0, 100.0],
        [200, 100],
        1.0,
    );
    let counts = painter.core.last_dest_pass_counts.expect("a painted frame");
    assert!(counts.group >= 2, "nested groups: {counts:?}");
    assert!(
        counts.composite_pixels > 0 && counts.composite_pixels < 4_000,
        "small groups composite small: {counts:?}"
    );
    assert!(pixel(&pixels, 200, 20, 20)[0].abs_diff(128) <= 2);
    assert!(pixel(&pixels, 200, 60, 20)[2].abs_diff(64) <= 2);
    assert_eq!(pixel(&pixels, 200, 40, 20), [0, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 200, 190, 90), [0, 0, 0, 255]);
}

/// A fractional edge keeps its antialiased fringe inside the scissor, and a
/// group reaching past the dest is clipped at its edge.
#[test]
fn normal_group_keeps_antialias_fringe_and_clips_at_target_edge() {
    use nana_ui_runtime::BlendMode;
    let pixels = paint_one_painter(
        PaintFn(|cx: &mut nana_ui_runtime::PaintContext<'_>| {
            cx.fill_path(&rect_path(0.0, 0.0, 80.0, 40.0), [0.0, 1.0, 0.0, 1.0]);
            cx.push_layer(0.5, BlendMode::Normal);
            let mut disc = nana_ui_runtime::PaintPath::new();
            disc.ellipse(LayoutBox {
                x: 10.3,
                y: 10.6,
                width: 19.0,
                height: 19.0,
            });
            cx.fill_path(&disc, [1.0, 0.0, 0.0, 1.0]);
            cx.push_layer(0.5, BlendMode::Normal);
            cx.fill_path(&rect_path(70.0, 10.0, 30.0, 20.0), [0.0, 0.0, 1.0, 1.0]);
            cx.pop_layer();
            cx.pop_layer();
        }),
        80,
        40,
    );
    let at = |x, y| pixel(&pixels, 80, x, y);
    let centre = at(20, 20);
    assert!(
        centre[0].abs_diff(128) <= 2 && centre[1].abs_diff(128) <= 2,
        "{centre:?}"
    );
    let rim = at(10, 20);
    assert!(rim[0] > 0 && rim[0] < centre[0], "antialiased rim: {rim:?}");
    for (x, y) in [(9, 20), (20, 8), (31, 20), (20, 31), (50, 20)] {
        assert_eq!(at(x, y), [0, 255, 0, 255], "outside the group at ({x},{y})");
    }
    let clipped = at(79, 20);
    assert!(
        clipped[2].abs_diff(64) <= 2 && clipped[1].abs_diff(191) <= 2,
        "at the dest edge: {clipped:?}"
    );
}

/// A drop shadow paints past its source: its group composites everywhere.
#[test]
fn spreading_filter_keeps_pixels_outside_source_bounds() {
    let (device, queue) = test_device();
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    let mut scene = UiScene::new();
    scene.apply_delta(
        [extracted_div(
            1,
            &[],
            8.0,
            8.0,
            16.0,
            16.0,
            nana_ui_core::LayoutStyle {
                paint_colors: nana_ui_core::PaintColorSlots {
                    background: Some(nana_ui_core::PaintColor::srgb([1.0, 0.0, 0.0, 1.0])),
                    ..Default::default()
                }
                .into(),
                paint: nana_ui_core::PaintStyle {
                    filter: Some(nana_ui_core::ColorFilter {
                        drop_shadow: Some(nana_ui_core::FilterDropShadow {
                            offset_x: 16.0,
                            offset_y: 0.0,
                            blur_radius: 0.0,
                            color: [0.0, 0.0, 1.0, 1.0].into(),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
            Some([1.0, 0.0, 0.0, 1.0]),
        )],
        [],
    );
    let pixels = paint_scene_rgba(
        &device,
        &queue,
        &mut painter,
        &scene,
        [64.0, 64.0],
        [64, 64],
        1.0,
    );
    let counts = painter.core.last_dest_pass_counts.expect("a painted frame");
    assert_eq!(
        counts.composite_pixels,
        64 * 64,
        "spreading filter: {counts:?}"
    );
    assert!(pixel(&pixels, 64, 12, 16)[0] > 250);
    assert!(
        pixel(&pixels, 64, 32, 16)[2] > 250,
        "the shadow past the source"
    );
    assert_eq!(pixel(&pixels, 64, 4, 16), [0, 0, 0, 255]);
}

/// A quad in an opacity group sliding on the GPU paints where the motion has
/// it, past the bounds it was encoded with: while anything moves on the GPU
/// the group composites over the whole dest.
#[test]
fn a_group_moving_on_the_gpu_is_not_clipped_to_its_encoded_bounds() {
    use nana_ui_core::{
        PaintTransform,
        motion::{AnimatableProperty, Easing, MotionTo, MotionValue},
    };
    use nana_ui_runtime::{AnimationId, AnimationSpec, MutationQueue, UiWorld};
    use std::time::Duration;

    let node = StableNodeId::new(2).unwrap();
    // From 32px right to rest: halfway through, 16px right.
    let spec = AnimationSpec::new(
        AnimationId::new(1).unwrap(),
        node,
        Duration::ZERO,
        Duration::from_millis(400),
        Duration::from_millis(16),
        Easing::Linear,
    )
    .with_property(AnimatableProperty::Transform)
    .with_range(
        MotionValue::Transform(PaintTransform {
            e: 32.0,
            ..PaintTransform::default()
        }),
        MotionTo::Value(MotionValue::Transform(PaintTransform::default())),
    );
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(node, DocumentId::new(1).unwrap(), NodeKind::Document);
    queue.start_animation(spec);
    world.commit(queue).unwrap();
    world.advance_animations(Duration::ZERO);
    let quad = colored_quad_child(2, 1, 0.0, 0.0, 16.0, 16.0, [1.0, 0.0, 0.0, 1.0]);
    let mut moving = world.extract_nodes(&[node]).remove(0);
    moving.parent = quad.parent;
    moving.layout = quad.layout;
    moving.source_style = quad.source_style;
    moving.style = quad.style;
    let mut scene = UiScene::new();
    scene.apply_delta(
        [
            translucent_parent(1, &[2], 0.0, 0.0, 64.0, 64.0, 0.5),
            moving,
        ],
        [],
    );
    scene.apply_presentation(
        world.presentation_store(),
        Duration::from_millis(200),
        Some(world.motion_descriptors()),
    );
    assert_ne!(
        scene.compositor_gpu_motion_ids(node).0,
        0,
        "the quad must move on the GPU"
    );

    let (device, queue) = test_device();
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    let pixels = paint_scene_rgba(
        &device,
        &queue,
        &mut painter,
        &scene,
        [64.0, 64.0],
        [64, 64],
        1.0,
    );
    assert!(
        pixel(&pixels, 64, 24, 8)[0].abs_diff(128) <= 2,
        "the slid quad: {:?}",
        pixel(&pixels, 64, 24, 8)
    );
    assert_eq!(
        pixel(&pixels, 64, 8, 8),
        [0, 0, 0, 255],
        "where it was encoded"
    );
}

const CARD: [f32; 4] = [0.0, 0.0, 96.0, 96.0];

/// A rounded `overflow: hidden` box, `children` inside it.
fn rounded_overflow(value: u64, children: &[u64], bounds: [f32; 4], radius: f32) -> ExtractedNode {
    let [x, y, width, height] = bounds;
    extracted_div(
        value,
        children,
        x,
        y,
        width,
        height,
        nana_ui_core::LayoutStyle {
            overflow_x: OverflowSpec::Hidden,
            overflow_y: OverflowSpec::Hidden,
            border_radius: Some(radius),
            ..nana_ui_core::LayoutStyle::default()
        },
        None,
    )
}

/// A green quad at `quad` under two nested rounded clips: a card at `outer`
/// with radius 16, and inside it `inner` with radius `inner_radius`, painted
/// into 96 by 96 pixels.
fn paint_under_nested_rounded_clips(
    outer: [f32; 4],
    inner: [f32; 4],
    inner_radius: f32,
    quad: [f32; 4],
) -> (Vec<u8>, DestPassCounts) {
    let (device, queue) = test_device();
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    let mut scene = UiScene::new();
    let [x, y, width, height] = quad;
    scene.apply_delta(
        [
            rounded_overflow(1, &[2], outer, 16.0),
            child_of(rounded_overflow(2, &[3], inner, inner_radius), 1),
            colored_quad_child(3, 2, x, y, width, height, [0.0, 1.0, 0.0, 1.0]),
        ],
        [],
    );
    let pixels = paint_scene_rgba(
        &device,
        &queue,
        &mut painter,
        &scene,
        [96.0, 96.0],
        [96, 96],
        1.0,
    );
    let counts = painter.core.last_dest_pass_counts.expect("a painted frame");
    (pixels, counts)
}

/// Content well inside an outer rounded clip, away from its corners, is kept
/// whole by it: only the innermost clip, which the quad's own shader applies,
/// cuts anything, so no dest group opens for the outer one.
#[test]
fn an_outer_rounded_clip_away_from_its_corners_opens_no_group() {
    let (pixels, counts) = paint_under_nested_rounded_clips(
        CARD,
        [24.0, 24.0, 48.0, 48.0],
        6.0,
        [24.0, 24.0, 48.0, 48.0],
    );
    assert_eq!(counts.group, 0, "no clip group: {counts:?}");
    // The inner clip still rounds the quad's own corners.
    assert_eq!(pixel(&pixels, 96, 48, 48), [0, 255, 0, 255]);
    assert_eq!(pixel(&pixels, 96, 24, 48), [0, 255, 0, 255]);
    assert_eq!(pixel(&pixels, 96, 24, 24)[1], 0, "inner corner cut");
    assert_eq!(pixel(&pixels, 96, 10, 10), [0, 0, 0, 255]);
}

/// Content reaching an outer rounded clip's corner square still composites
/// through a group, which cuts what the inner clip alone would keep.
#[test]
fn an_outer_rounded_clip_still_cuts_content_at_its_corners() {
    let (pixels, counts) =
        paint_under_nested_rounded_clips(CARD, [0.0, 0.0, 96.0, 96.0], 4.0, [0.0, 0.0, 96.0, 96.0]);
    assert!(
        counts.group >= 1,
        "the outer clip needs its group: {counts:?}"
    );
    // (2, 2) lies inside the inner radius-4 corner but outside the outer
    // radius-16 one.
    assert_eq!(pixel(&pixels, 96, 2, 2)[1], 0, "outer corner cut");
    assert_eq!(pixel(&pixels, 96, 48, 48), [0, 255, 0, 255]);
    assert_eq!(
        pixel(&pixels, 96, 48, 1),
        [0, 255, 0, 255],
        "straight edge kept"
    );
}

/// An outer straight edge on half a pixel ramps the pixel it crosses, which
/// the scissor, rounded out to whole pixels, keeps whole: content reaching it
/// still needs the outer clip's group.
#[test]
fn an_outer_rounded_clip_still_ramps_a_fractional_straight_edge() {
    let (pixels, counts) = paint_under_nested_rounded_clips(
        [0.0, 0.0, 95.5, 96.0],
        [24.0, 24.0, 100.0, 48.0],
        6.0,
        [24.0, 24.0, 100.0, 48.0],
    );
    assert!(
        counts.group >= 1,
        "the outer clip needs its group: {counts:?}"
    );
    let edge = pixel(&pixels, 96, 95, 48);
    assert!(
        (100..=160).contains(&edge[1]),
        "half covered at the fractional edge: {edge:?}"
    );
    assert_eq!(pixel(&pixels, 96, 60, 48), [0, 255, 0, 255]);
}

/// A 16 px red square at `x`, 56 down, whose own group blurs it by
/// `blur` px, opting into the wide cap or not, with `shutter` open.
fn blurred_square(value: u64, x: f32, blur: f32, wide: bool, shutter: f32) -> ExtractedNode {
    extracted_div(
        value,
        &[],
        x,
        56.0,
        16.0,
        16.0,
        nana_ui_core::LayoutStyle {
            paint_colors: nana_ui_core::PaintColorSlots {
                background: Some(nana_ui_core::PaintColor::srgb([1.0, 0.0, 0.0, 1.0])),
                ..Default::default()
            }
            .into(),
            paint: nana_ui_core::PaintStyle {
                filter: (blur > 0.0).then_some(nana_ui_core::ColorFilter {
                    blur_radius: blur,
                    ..Default::default()
                }),
                wide_filter_blur: wide,
                motion_shutter: shutter,
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
        Some([1.0, 0.0, 0.0, 1.0]),
    )
}

fn paint_256x128(scene: &UiScene) -> Vec<u8> {
    let (device, queue) = test_device();
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    paint_scene_rgba(
        &device,
        &queue,
        &mut painter,
        scene,
        [256.0, 128.0],
        [256, 128],
        1.0,
    )
}

/// Issue #314: a group that opts into the wide cap blurs past 16 px, up to
/// 64 px; a button beside it with the same 40 px blur stays at 16 px.
#[test]
fn only_a_group_that_opts_in_blurs_past_sixteen_pixels() {
    let mut scene = UiScene::new();
    scene.apply_delta(
        [
            blurred_square(1, 48.0, 40.0, true, 0.0),
            blurred_square(2, 176.0, 40.0, false, 0.0),
        ],
        [],
    );
    let pixels = paint_256x128(&scene);
    let red = |x| pixel(&pixels, 256, x, 64)[0];
    // 28 px left of each square: inside a 40 px blur's reach, past 16 px.
    assert!(red(20) > 4, "the wide group reaches 28 px: {}", red(20));
    assert_eq!(red(148), 0, "the button stays at the 16 px cap");
    assert!(
        red(166) > 4,
        "the button blurs within its cap: {}",
        red(166)
    );

    // Past 64 px, the wide group holds 64.
    let mut far = UiScene::new();
    far.apply_delta([blurred_square(1, 120.0, 400.0, true, 0.0)], []);
    let mut cap = UiScene::new();
    cap.apply_delta([blurred_square(1, 120.0, 64.0, true, 0.0)], []);
    assert_eq!(paint_256x128(&far), paint_256x128(&cap));
}

/// Issue #323: an open shutter streaks a group along its own transform's
/// velocity at `now`, behind its motion and no longer than the exposure
/// moved it; a closed one, the default, draws the box alone.
#[test]
fn an_open_shutter_streaks_behind_the_motion_and_a_closed_one_does_not() {
    use nana_ui_core::{
        PaintTransform,
        motion::{
            AnimatableProperty, AnimationPlayback, Easing, MotionCurve, MotionTargetId,
            MotionTiming, MotionTrack, MotionTrackId, MotionValue, PresentationStore,
        },
    };
    use std::time::Duration;
    // 200 px/s to the right: at 500 ms it is 100 px along.
    let track = MotionTrack::transition(
        MotionTrackId::new(1).unwrap(),
        MotionTargetId::new(1).unwrap(),
        AnimatableProperty::Transform,
        MotionValue::Transform(PaintTransform::default()),
        MotionValue::Transform(PaintTransform {
            e: 200.0,
            ..PaintTransform::default()
        }),
        MotionTiming::new(
            Duration::ZERO,
            Duration::from_secs(1),
            Duration::from_millis(16),
        ),
        MotionCurve::Easing(Easing::Linear),
        AnimationPlayback::default(),
    );
    let mut store = PresentationStore::new();
    store.insert(track, MotionValue::Transform(PaintTransform::default()));
    let paint = |shutter: f32| {
        let mut scene = UiScene::new();
        scene.apply_delta([blurred_square(1, 8.0, 0.0, false, shutter)], []);
        // A 100 ms frame: the full shutter exposes 20 px of motion.
        scene.set_presentation_frame_interval(Duration::from_millis(100));
        scene.apply_presentation(&store, Duration::from_millis(500), None);
        paint_256x128(&scene)
    };
    let open = paint(1.0);
    let red = |x| pixel(&open, 256, x, 64)[0];
    // The square now spans 108..124, and moved 20 px in the exposure: it is
    // smeared over 88..124, no pixel covered all the time.
    assert_eq!(red(87), 0, "21 px behind, past the 20 px streak");
    assert!(red(88) > 0, "20 px behind: {}", red(88));
    assert!(red(98) > 100, "10 px behind: {}", red(98));
    assert!(red(108) > red(120), "the leading edge is covered least");
    assert_eq!(red(124), 0, "nothing ahead of the motion");

    let closed = paint(0.0);
    assert_eq!(
        pixel(&closed, 256, 98, 64)[0],
        0,
        "a closed shutter streaks nothing"
    );
    assert!(pixel(&closed, 256, 108, 64)[0] > 250 && pixel(&closed, 256, 123, 64)[0] > 250);
}

/// Issue #312: the painter composites a group's filter as the compositor
/// sampled it at `now`.
#[test]
fn a_filter_track_paints_the_sample_at_now() {
    use nana_ui_core::{
        ColorFilter,
        motion::{
            AnimatableProperty, AnimationPlayback, Easing, MotionCurve, MotionTargetId,
            MotionTiming, MotionTrack, MotionTrackId, MotionValue, PresentationStore,
        },
    };
    use std::time::Duration;
    let track = MotionTrack::transition(
        MotionTrackId::new(1).unwrap(),
        MotionTargetId::new(1).unwrap(),
        AnimatableProperty::Filter,
        MotionValue::Filter(ColorFilter::default().into()),
        MotionValue::Filter(
            ColorFilter {
                brightness: 0.0,
                ..ColorFilter::default()
            }
            .into(),
        ),
        MotionTiming::new(
            Duration::ZERO,
            Duration::from_secs(1),
            Duration::from_millis(16),
        ),
        MotionCurve::Easing(Easing::Linear),
        AnimationPlayback::default(),
    );
    let mut store = PresentationStore::new();
    store.insert(track, MotionValue::Filter(ColorFilter::default().into()));
    let mut scene = UiScene::new();
    scene.apply_delta([blurred_square(1, 120.0, 0.0, false, 0.0)], []);
    let mut red_at = |ms| {
        scene.apply_presentation(&store, Duration::from_millis(ms), None);
        pixel(&paint_256x128(&scene), 256, 128, 64)[0]
    };
    let start = red_at(0);
    let middle = red_at(500);
    let late = red_at(950);
    assert!(start > 250, "{start}");
    assert!(late < middle && middle < start, "{start} {middle} {late}");
    assert!(late < 40, "{late}");
}
