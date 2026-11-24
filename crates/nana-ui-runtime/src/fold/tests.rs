use std::time::Duration;

use super::*;
use crate::{
    AnimationFillMode, AnimationId, AnimationSpec, AppContext, DocumentId, Easing, MotionCurve,
    MotionTo, MutationQueue, Stack, UiMutation,
};

fn bounds(width: f32, height: f32) -> LayoutBox {
    LayoutBox {
        x: 10.0,
        y: 20.0,
        width,
        height,
    }
}

fn started(queue: &MutationQueue) -> Vec<AnimationSpec> {
    queue
        .as_slice()
        .iter()
        .filter_map(|mutation| match mutation {
            UiMutation::StartAnimation { animation } => Some((**animation).clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn each_box_scales_onto_the_other() {
    let (down, up) = scales(bounds(200.0, 100.0), bounds(50.0, 40.0)).unwrap();
    assert_eq!(down, PaintTransform::scaling(0.25, 0.4));
    assert_eq!(up, PaintTransform::scaling(4.0, 2.5));
    assert_eq!(scales(bounds(200.0, 100.0), bounds(0.0, 40.0)), None);
    assert_eq!(scales(bounds(200.0, 0.2), bounds(50.0, 40.0)), None);
}

/// A side runs its opacity and its transform from where it is told, on one
/// timing and each on its own curve, on the node's own tracks.
#[test]
fn a_fold_runs_both_tracks_together_on_their_own_curves() {
    let node = StableNodeId::new(7).unwrap();
    let small = PaintTransform::scaling(0.5, 0.25);
    let mut queue = MutationQueue::new();
    queue
        .node(node, Duration::from_millis(40))
        .fold(Pose::SHOWN, Pose::faded(small))
        .duration(Duration::from_millis(300))
        .fade(Easing::Linear)
        .travel(Easing::EaseInOutCubic);
    let [opacity, transform] = started(&queue).try_into().unwrap();
    assert_eq!(
        opacity.id,
        AnimationId::for_node(node, AnimatableProperty::Opacity)
    );
    assert_eq!(opacity.from, MotionValue::Scalar(1.0));
    assert_eq!(opacity.to, MotionTo::Value(MotionValue::Scalar(0.0)));
    assert_eq!(opacity.curve, MotionCurve::Easing(Easing::Linear));
    assert_eq!(
        transform.id,
        AnimationId::for_node(node, AnimatableProperty::Transform)
    );
    assert_eq!(
        transform.from,
        MotionValue::Transform(PaintTransform::IDENTITY)
    );
    assert_eq!(transform.to, MotionTo::Value(MotionValue::Transform(small)));
    assert_eq!(transform.curve, MotionCurve::Easing(Easing::EaseInOutCubic));
    for spec in [&opacity, &transform] {
        assert_eq!(spec.timing.start, Duration::from_millis(40));
        assert_eq!(spec.timing.duration, Duration::from_millis(300));
        assert_eq!(spec.playback.fill_mode, AnimationFillMode::Both);
    }
}

/// A side that shows nothing starts from the other's size; one that shows
/// starts where it is.
#[test]
fn a_fold_starts_from_the_presented_pose_or_hidden_at_the_other_size() {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let node = cx
        .create_component(document, Stack::column(0.0))
        .unwrap()
        .stable_id();
    let large = PaintTransform::scaling(3.0, 2.0);
    assert_eq!(origin(cx.world(), node, large), Pose::SHOWN);

    let small = PaintTransform::scaling(0.5, 0.5);
    let mut queue = MutationQueue::new();
    queue
        .node(node, Duration::ZERO)
        .fold(Pose::SHOWN, Pose::shown(small))
        .duration(Duration::ZERO);
    cx.commit_mutations(queue).unwrap();
    assert_eq!(origin(cx.world(), node, large), Pose::shown(small));

    let mut queue = MutationQueue::new();
    queue
        .node(node, Duration::ZERO)
        .fold(Pose::shown(small), Pose::faded(small))
        .duration(Duration::ZERO);
    cx.commit_mutations(queue).unwrap();
    assert_eq!(origin(cx.world(), node, large), Pose::faded(large));
}
