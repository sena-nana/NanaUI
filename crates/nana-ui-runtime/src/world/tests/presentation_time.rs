//! Issue #300: the document's presentation time can be set, played and
//! paused. One seek samples layout-class tracks and moves the compositor
//! clock to the same time; host frames do not move a paused document.

use super::*;
use crate::{AnimationEventKind, PresentationPlayback};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn width_spec(id: u64, target: StableNodeId, duration_ms: u64) -> AnimationSpec {
    AnimationSpec::new(
        AnimationId::new(id).unwrap(),
        target,
        Duration::ZERO,
        ms(duration_ms),
        ms(16),
        Easing::Linear,
    )
    .with_property(AnimatableProperty::Width)
    .with_range(
        MotionValue::Scalar(0.0),
        MotionTo::Value(MotionValue::Scalar(100.0)),
    )
}

/// A document with a 1 s width track (Layout) and a 1 s opacity track
/// (Compositor) on node 1, sampled once at 0.
fn animated_world() -> UiWorld {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(1), NodeKind::Document);
    queue.start_animation(width_spec(1, node(1), 1_000));
    queue.start_animation(opacity_overlay_spec(2, node(1), 0, 1_000, 0.0, 1.0));
    world.commit(queue).unwrap();
    world.advance_animations(Duration::ZERO);
    world
}

fn width(world: &UiWorld) -> Option<f32> {
    match world.node_style(node(1))?.layout.width? {
        LengthSpec::Px(px) => Some(px),
        _ => None,
    }
}

fn opacity(world: &UiWorld) -> Option<f32> {
    match world.presentation_motion_value(
        node(1),
        AnimatableProperty::Opacity,
        world.animation_now(),
    )? {
        MotionValue::Scalar(value) => Some(value),
        _ => None,
    }
}

fn close(left: Option<f32>, right: f32) -> bool {
    left.is_some_and(|left| (left - right).abs() < 1e-3)
}

#[test]
fn a_seek_puts_layout_and_compositor_samples_at_one_time() {
    let mut world = animated_world();
    world.set_presentation_playback(PresentationPlayback::Paused);
    world.seek_presentation(ms(300));
    assert_eq!(world.animation_now(), ms(300));
    assert!(close(width(&world), 30.0), "{:?}", width(&world));
    assert!(close(opacity(&world), 0.3), "{:?}", opacity(&world));
}

#[test]
fn round_trip_seeks_return_both_samples_to_the_same_value() {
    let mut world = animated_world();
    world.set_presentation_playback(PresentationPlayback::Paused);
    world.seek_presentation(ms(700));
    let (late_width, late_opacity) = (width(&world), opacity(&world));
    world.seek_presentation(ms(200));
    let (early_width, early_opacity) = (width(&world), opacity(&world));
    assert!(close(early_width, 20.0) && close(early_opacity, 0.2));

    // Present frames in between: the paused document does not move, and the
    // result does not depend on whether a present happened.
    world.sync_presentation_clock(ms(5_000));
    world.advance_animations(ms(5_016));
    assert_eq!(world.animation_now(), ms(200));
    assert!(close(width(&world), 20.0));

    world.seek_presentation(ms(700));
    assert_eq!(width(&world), late_width);
    assert_eq!(opacity(&world), late_opacity);
    world.seek_presentation(ms(200));
    assert_eq!(width(&world), early_width);
    assert_eq!(opacity(&world), early_opacity);
}

#[test]
fn either_host_path_alone_keeps_a_seeked_document_at_its_time() {
    let mut world = animated_world();
    world.set_presentation_playback(PresentationPlayback::Paused);
    world.seek_presentation(ms(400));
    // Compositor-only present.
    world.sync_presentation_clock(ms(900));
    assert_eq!(world.animation_now(), ms(400));
    assert!(close(opacity(&world), 0.4));
    // CPU advance alone.
    let frame = world.advance_animations(ms(950));
    assert_eq!(world.animation_now(), ms(400));
    assert!(frame.samples.is_empty(), "nothing is due at a held time");
    assert!(close(width(&world), 40.0));
}

#[test]
fn a_paused_document_wakes_nobody_and_playing_resumes_without_a_jump() {
    let mut world = animated_world();
    world.advance_animations(ms(100));
    world.set_presentation_playback(PresentationPlayback::Paused);
    assert!(!world.presentation_advancing());
    assert_eq!(world.next_presentation_wakeup(), None);
    world.advance_animations(ms(3_000));
    assert_eq!(world.animation_now(), ms(100));

    world.set_presentation_playback(PresentationPlayback::REAL_TIME);
    // The first host time after playing pairs with the held time.
    world.advance_animations(ms(10_000));
    assert_eq!(world.animation_now(), ms(100));
    world.advance_animations(ms(10_200));
    assert_eq!(world.animation_now(), ms(300));
    assert!(close(width(&world), 30.0));
    // Deadlines map back to host time.
    let wake = world.next_presentation_wakeup().expect("still running");
    assert_eq!(wake, ms(10_216));
}

#[test]
fn two_documents_keep_their_own_time() {
    let mut paused = animated_world();
    let mut playing = animated_world();
    paused.set_presentation_playback(PresentationPlayback::Paused);
    paused.advance_animations(ms(500));
    playing.advance_animations(ms(500));
    assert_eq!(paused.animation_now(), Duration::ZERO);
    assert_eq!(playing.animation_now(), ms(500));
    assert!(close(width(&playing), 50.0));
    playing.seek_presentation(ms(800));
    assert_eq!(paused.animation_now(), Duration::ZERO);
}

#[test]
fn real_time_playback_is_the_host_time_unchanged() {
    let mut world = animated_world();
    assert_eq!(
        world.presentation_playback(),
        PresentationPlayback::REAL_TIME
    );
    for host in [16, 33, 50, 120] {
        world.advance_animations(ms(host));
        assert_eq!(world.animation_now(), ms(host));
        assert_eq!(
            world.next_presentation_wakeup(),
            world.next_animation_deadline()
        );
    }
    world.sync_presentation_clock(ms(130));
    assert_eq!(world.animation_now(), ms(130));
}

#[test]
fn a_rate_scales_the_document_time() {
    let mut world = animated_world();
    world.set_presentation_playback(PresentationPlayback::Playing { rate: 0.5 });
    world.advance_animations(ms(100));
    world.advance_animations(ms(500));
    assert_eq!(world.animation_now(), ms(200));
    assert!(close(width(&world), 20.0));
}

#[test]
fn a_seek_past_the_end_finishes_the_run_once() {
    let mut world = animated_world();
    world.set_presentation_playback(PresentationPlayback::Paused);
    let frame = world.seek_presentation(ms(1_500));
    assert!(frame.events.is_empty(), "delivered by the next advance");
    assert!(close(width(&world), 100.0));
    let finished = world.advance_animations(ms(1_600)).events;
    assert_eq!(
        finished
            .iter()
            .filter(|event| event.kind == AnimationEventKind::Finished)
            .count(),
        2
    );
    assert!(world.advance_animations(ms(1_700)).events.is_empty());
}

#[test]
fn stepping_advances_by_the_interval_and_makes_it_the_frame_interval() {
    let mut world = animated_world();
    world.set_presentation_playback(PresentationPlayback::Paused);
    world.step_presentation(ms(40));
    world.step_presentation(ms(40));
    assert_eq!(world.animation_now(), ms(80));
    assert_eq!(world.presentation_frame_interval(), ms(40));
    // Below the track's own 16 ms sampling, a step still writes its value.
    world.step_presentation(ms(5));
    assert!(close(width(&world), 8.5));
}

#[test]
fn a_seek_back_before_the_start_waits_for_the_start_again() {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(1), NodeKind::Document);
    let mut spec = width_spec(1, node(1), 1_000);
    spec.timing.start = ms(200);
    queue.start_animation(spec);
    world.commit(queue).unwrap();
    world.set_presentation_playback(PresentationPlayback::Paused);
    world.seek_presentation(ms(500));
    assert_eq!(world.next_animation_deadline(), Some(ms(516)));
    world.seek_presentation(ms(100));
    assert_eq!(world.next_animation_deadline(), Some(ms(200)));
}

#[test]
fn replacing_a_track_samples_the_new_curve_at_the_current_time() {
    let mut world = animated_world();
    world.set_presentation_playback(PresentationPlayback::Paused);
    world.seek_presentation(ms(500));
    assert!(close(width(&world), 50.0));

    let tracks = world.animation_tracks(node(1));
    assert_eq!(tracks.len(), 2);
    let mut edited = tracks
        .into_iter()
        .find(|spec| spec.property == AnimatableProperty::Width)
        .unwrap();
    edited.timing.duration = ms(2_000);
    let mut queue = MutationQueue::new();
    queue.replace_animation(edited);
    world.commit(queue).unwrap();

    let frame = world.advance_animations(ms(9_000));
    assert_eq!(world.animation_now(), ms(500), "still held");
    assert!(close(width(&world), 25.0), "{:?}", width(&world));
    assert!(frame.events.is_empty(), "a replacement is not a cancel");
    let ids = world
        .animation_tracks(node(1))
        .iter()
        .map(|spec| spec.id.get())
        .collect::<Vec<_>>();
    assert_eq!(ids, [1, 2], "no second id");
}

#[test]
fn replacing_a_missing_track_is_rejected() {
    let mut world = animated_world();
    let mut queue = MutationQueue::new();
    queue.replace_animation(width_spec(9, node(1), 100));
    assert!(world.commit(queue).is_err());
}
