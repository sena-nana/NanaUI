//! Issue #304: a document drawn at a time the caller sets, at the target's
//! own pixels, with no window or Surface. One frame runs in order: set `now`,
//! flush (explicit rectangles placed), compositor evaluates, draw.
use super::*;
use crate::output::{DocumentFrame, PresentationFrameTime};
use nana_ui_core::LayoutStyle;
use nana_ui_core::motion::{AnimatableProperty, Easing, MotionTo, MotionValue};
use nana_ui_runtime::{
    AnimationId, AnimationSpec, DisplaySpec, ExplicitRect, LengthSpec, MutationQueue, NodeKind,
    NodeStyle, PresentationPlayback, StableNodeId,
};
use std::sync::Arc;

const LAYER: ExplicitRect = ExplicitRect {
    x: 8.0,
    y: 8.0,
    width: 16.0,
    height: 16.0,
};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn frame_time(now: u64, interval: u64) -> PresentationFrameTime {
    PresentationFrameTime::new(ms(now), ms(interval)).expect("non-zero interval")
}

/// A free board filling the viewport, holding one opaque red layer placed by
/// its explicit rectangle and fading in over 400 ms on the compositor.
fn free_layer_document() -> RuntimeDocument {
    let document_id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(document_id);
    let root = StableNodeId::new(1).unwrap();
    let board = StableNodeId::new(2).unwrap();
    let layer = StableNodeId::new(3).unwrap();
    let element = |queue: &mut MutationQueue, parent, id, layout: LayoutStyle| {
        queue.create(id, document_id, NodeKind::Element { tag: "div".into() });
        queue.insert(parent, id, None);
        queue.set_style(
            id,
            NodeStyle {
                layout: Arc::new(layout),
                ..NodeStyle::default()
            },
        );
    };
    let mut queue = MutationQueue::new();
    queue.create(root, document_id, NodeKind::Document);
    element(
        &mut queue,
        root,
        board,
        LayoutStyle {
            display: Some(DisplaySpec::Free),
            width: Some(LengthSpec::Px(64.0)),
            height: Some(LengthSpec::Px(48.0)),
            ..LayoutStyle::default()
        },
    );
    element(
        &mut queue,
        board,
        layer,
        LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            ..LayoutStyle::default()
        },
    );
    queue.set_explicit_rect(layer, Some(LAYER));
    queue.start_animation(
        AnimationSpec::new(
            AnimationId::new(1).unwrap(),
            layer,
            Duration::ZERO,
            ms(400),
            ms(16),
            Easing::Linear,
        )
        .with_property(AnimatableProperty::Opacity)
        .with_range(
            MotionValue::Scalar(0.0),
            MotionTo::Value(MotionValue::Scalar(1.0)),
        ),
    );
    document.context_mut().commit_mutations(queue).unwrap();
    document
        .context_mut()
        .set_presentation_playback(PresentationPlayback::Paused);
    document
}

/// Draw the frame for `time`, retrying deferred groups on the same scene,
/// then wait for exactly that frame to complete.
fn draw_at(
    driver: &mut PresentationDriver,
    document: &mut RuntimeDocument,
    time: PresentationFrameTime,
) -> (DocumentFrame, ExternalFrame) {
    let mut frame = driver
        .render_document_at(document, time, [64.0, 48.0], 0, CLEAR, None, None)
        .expect("render at time");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !frame.is_drawn() {
        assert!(Instant::now() < deadline, "no frame slot for the export");
        driver.poll();
        std::thread::sleep(Duration::from_millis(1));
        frame.outcomes = render_retry(driver, document.scene());
    }
    let revision = match frame.outcomes[0] {
        ExternalRenderOutcome::Submitted {
            content_revision, ..
        }
        | ExternalRenderOutcome::Reused {
            content_revision, ..
        } => content_revision,
        ExternalRenderOutcome::Deferred => unreachable!("drawn"),
    };
    loop {
        let completed = completed(driver, OutputConsumerId(1));
        if completed.content_revision() == revision {
            return (frame, completed);
        }
        assert!(
            Instant::now() < deadline,
            "frame for {time:?} never completed"
        );
        driver.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn render_retry(driver: &mut PresentationDriver, scene: &UiScene) -> Vec<ExternalRenderOutcome> {
    driver
        .render_scene(scene, [64.0, 48.0], 0, CLEAR, None, None)
        .expect("retry presentation")
}

/// BGRA8 texel at logical (x, y); the target is 1:1 with the viewport.
fn texel(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
    let at = (y * EXTENT[0] as usize + x) * 4;
    pixels[at..at + 4].try_into().unwrap()
}

#[test]
fn a_document_is_drawn_at_the_time_and_target_pixels_the_caller_sets() {
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[consumer(1)]).unwrap();
    let mut document = free_layer_document();

    let (first, early) = draw_at(&mut driver, &mut document, frame_time(100, 40));
    assert_eq!(first.time.now(), ms(100));
    assert_eq!(document.context().animation_now(), ms(100));
    // The caller's step is the frame interval, not the display's default.
    assert_eq!(document.context().presentation_frame_interval(), ms(40));
    let early = read_pixels(early.texture().unwrap());
    let (_, late) = draw_at(&mut driver, &mut document, frame_time(300, 40));
    let late = read_pixels(late.texture().unwrap());

    // The layer sits where its explicit rectangle puts it; outside it the
    // target keeps the clear colour.
    let outside = texel(&late, 40, 32);
    assert_eq!(texel(&late, 4, 4), outside);
    assert_ne!(texel(&late, 16, 16), outside, "layer at its rectangle");
    assert_ne!(texel(&late, 26, 26), texel(&late, 16, 16), "rectangle ends");
    // The compositor evaluated at each time: more of the layer shows later.
    let red = |pixels: &[u8]| texel(pixels, 16, 16)[2];
    assert!(
        red(&early) < red(&late),
        "opacity at 100 ms must be below 300 ms: {} vs {}",
        red(&early),
        red(&late)
    );

    // Going back to a time draws that time's pixels again, byte for byte.
    let (back, again) = draw_at(&mut driver, &mut document, frame_time(100, 20));
    assert_eq!(back.time.interval(), ms(20));
    assert_eq!(document.context().presentation_frame_interval(), ms(20));
    assert_eq!(read_pixels(again.texture().unwrap()), early);
}

#[test]
fn stepping_draws_each_frame_and_an_unchanged_time_reuses_the_last() {
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[consumer(1)]).unwrap();
    let mut document = free_layer_document();
    let mut time = frame_time(0, 100);
    let mut revisions = Vec::new();
    for _ in 0..3 {
        time = time.next();
        let (frame, completed) = draw_at(&mut driver, &mut document, time);
        assert!(matches!(
            frame.outcomes[0],
            ExternalRenderOutcome::Submitted { .. }
        ));
        revisions.push(completed.content_revision());
    }
    assert_eq!(time.now(), ms(300));
    assert!(revisions.windows(2).all(|pair| pair[0] < pair[1]));
    // The same time again: nothing new to draw, the finished frame stands.
    let (frame, completed) = draw_at(&mut driver, &mut document, time);
    assert!(matches!(
        frame.outcomes[0],
        ExternalRenderOutcome::Reused { .. }
    ));
    assert_eq!(completed.content_revision(), *revisions.last().unwrap());
    assert_eq!(driver.work().output_submissions, Some(0));
}

#[test]
fn a_frame_time_needs_an_interval() {
    assert!(PresentationFrameTime::new(ms(10), Duration::ZERO).is_none());
    let time = frame_time(10, 25);
    assert_eq!(time.next().now(), ms(35));
    assert_eq!(time.next().interval(), ms(25));
}
