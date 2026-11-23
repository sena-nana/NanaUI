//! Issue #304: a document drawn at a time the caller sets, at the target's
//! own pixels, with no window or Surface. One frame runs in order: set `now`,
//! align time-sampled resources, flush (explicit rectangles placed),
//! compositor evaluates, draw. No encoder takes part.
use super::*;
use crate::MediaSampleOutcome;
use crate::output::{DocumentFrame, MediaSampler, PresentationFrameTime};
use crate::{FrameBinding, HostTextureAlphaMode, HostTextureRegistry, MediaFrame};
use nana_frame_exchange::{
    CopyOutcome, DEFAULT_CAPACITY, FrameColorMetadata, FrameExchange, FrameInbox,
};
use nana_gpu::{GpuContext, GpuTextureRegion};
use nana_ui_core::LayoutStyle;
use nana_ui_core::motion::{AnimatableProperty, Easing, MotionTo, MotionValue};
use nana_ui_runtime::{
    AnimationId, AnimationSpec, ComponentView, DisplaySpec, ExplicitRect, LengthSpec, MediaPacing,
    MediaSampleRequest, MediaTimeMapping, MutationQueue, NodeKind, NodeStyle, PresentationPlayback,
    StableNodeId, UiWorld, Video,
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
    layer_document(|layer| {
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
        )
    })
}

/// The board and layer of [`free_layer_document`], the layer running the
/// track `animation` makes for it.
fn layer_document(animation: impl FnOnce(StableNodeId) -> AnimationSpec) -> RuntimeDocument {
    board_document(red_layer(), animation, |_, _, _| {})
}

fn red_layer() -> LayoutStyle {
    LayoutStyle {
        background: Some([1.0, 0.0, 0.0, 1.0]),
        ..LayoutStyle::default()
    }
}

/// A free board filling the viewport with one layer styled `layer_style` at
/// [`LAYER`], running `animation`; `extra` adds more children to the board.
fn board_document(
    layer_style: LayoutStyle,
    animation: impl FnOnce(StableNodeId) -> AnimationSpec,
    extra: impl FnOnce(&UiWorld, &mut MutationQueue, StableNodeId),
) -> RuntimeDocument {
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
    element(&mut queue, board, layer, layer_style);
    queue.set_explicit_rect(layer, Some(LAYER));
    queue.start_animation(animation(layer));
    extra(document.context().world(), &mut queue, board);
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
    draw_with_media(driver, document, time, None, None).expect("render at time")
}

/// [`draw_at`] with host textures and a media sampler; a stepped frame with
/// an unaligned resource comes back as the error, nothing drawn.
fn draw_with_media(
    driver: &mut PresentationDriver,
    document: &mut RuntimeDocument,
    time: PresentationFrameTime,
    textures: Option<&HostTextureRegistry>,
    media: Option<MediaSampler<'_>>,
) -> Result<(DocumentFrame, ExternalFrame), PresentationError> {
    let mut frame = driver.render_document_at(
        document,
        time,
        [64.0, 48.0],
        0,
        CLEAR,
        textures,
        None,
        media,
    )?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !frame.is_drawn() {
        assert!(Instant::now() < deadline, "no frame slot for the export");
        driver.poll();
        std::thread::sleep(Duration::from_millis(1));
        frame.outcomes = render_retry(driver, document.scene(), textures);
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
            return Ok((frame, completed));
        }
        assert!(
            Instant::now() < deadline,
            "frame for {time:?} never completed"
        );
        driver.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn render_retry(
    driver: &mut PresentationDriver,
    scene: &UiScene,
    textures: Option<&HostTextureRegistry>,
) -> Vec<ExternalRenderOutcome> {
    driver
        .render_scene(scene, [64.0, 48.0], 0, CLEAR, textures, None)
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

/// Issue #312: a group's filter is evaluated at the frame's `now` with the
/// rest of the compositor: each time draws its own sample, and going back
/// draws the earlier pixels again.
#[test]
fn a_group_filter_is_drawn_at_the_frame_time() {
    use nana_ui_core::ColorFilter;
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[consumer(1)]).unwrap();
    let mut document = layer_document(|layer| {
        AnimationSpec::new(
            AnimationId::new(1).unwrap(),
            layer,
            Duration::ZERO,
            ms(400),
            ms(16),
            Easing::Linear,
        )
        .with_property(AnimatableProperty::Filter)
        .with_range(
            MotionValue::Filter(ColorFilter::default().into()),
            MotionTo::Value(MotionValue::Filter(
                ColorFilter {
                    brightness: 0.0,
                    ..ColorFilter::default()
                }
                .into(),
            )),
        )
    });
    let red = |pixels: &[u8]| texel(pixels, 16, 16)[2];
    let (_, early) = draw_at(&mut driver, &mut document, frame_time(100, 40));
    let early = read_pixels(early.texture().unwrap());
    let (_, late) = draw_at(&mut driver, &mut document, frame_time(300, 40));
    let late = read_pixels(late.texture().unwrap());
    assert!(
        red(&late) < red(&early),
        "darker at 300 ms than at 100 ms: {} vs {}",
        red(&late),
        red(&early)
    );
    // The layer's style holds no filter: the sample lives in the compositor.
    let layer = StableNodeId::new(3).unwrap();
    assert_eq!(
        document
            .context()
            .world()
            .node_style(layer)
            .and_then(|style| style.layout.paint.filter),
        None
    );
    let (_, again) = draw_at(&mut driver, &mut document, frame_time(100, 40));
    assert_eq!(read_pixels(again.texture().unwrap()), early);
}

const CLIP: &str = "clip";
const VIDEO: ExplicitRect = ExplicitRect {
    x: 36.0,
    y: 8.0,
    width: 16.0,
    height: 16.0,
};

/// A 4×4 opaque frame of one colour (RGBA8) the producer copies from.
fn solid(gpu: &GpuContext, rgba: [u8; 4]) -> GpuTexture {
    let texture = gpu
        .create_texture(&GpuTextureDescriptor {
            label: Some("presentation time media source"),
            width: 4,
            height: 4,
            format: GpuTextureFormat::RGBA8_UNORM,
            usage: GpuTextureUsages::COPY_SRC | GpuTextureUsages::COPY_DST,
        })
        .expect("media source");
    gpu.write_texture(
        &texture,
        GpuTextureRegion {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        },
        &rgba.repeat(16),
        16,
    )
    .expect("fill media source");
    texture
}

/// Publish `source` as the frame for resource time `media_time`, as a
/// decoder would, and wait until the exchange has it.
fn publish(exchange: &mut FrameExchange<u64, MediaFrame>, source: &GpuTexture, media_time: f64) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match exchange.copy_from_with_payload(
            source,
            1,
            FrameColorMetadata::unknown(),
            MediaFrame::new(media_time, ()),
        ) {
            CopyOutcome::Submitted => break,
            CopyOutcome::PoolFull => {
                assert!(Instant::now() < deadline, "no exchange slot came back");
                exchange.poll();
                std::thread::yield_now();
            }
            other => panic!("copy {other:?}"),
        }
    }
    while !exchange.poll() {
        assert!(Instant::now() < deadline, "publication never completed");
        std::thread::yield_now();
    }
}

fn sample(
    binding: &mut FrameBinding<u64, MediaFrame>,
    inbox: &FrameInbox<u64, MediaFrame>,
    request: &MediaSampleRequest,
) -> MediaSampleOutcome {
    binding.prepare_media(Some(inbox), |_| true, request)
}

/// Issue #321 / #322: one frame runs in the document's order with a free
/// layer, an isolated group effect and a time-sampled resource, on no window
/// Surface and no encoder. The time is set first, so the resource request
/// reads it; a stepped frame waits for a resource frame within half the
/// caller's step (not the display interval), then the explicit rectangles
/// and the group filter at that time are drawn with the aligned frame.
#[test]
fn a_frame_sets_now_aligns_media_places_layers_and_composites_in_order() {
    use nana_ui_core::ColorFilter;
    let gpu = test_gpu::context();
    let mut driver = PresentationDriver::new(&gpu);
    driver.configure(&[consumer(1)]).unwrap();
    let registry = HostTextureRegistry::new();
    let mut exchange = FrameExchange::<u64, MediaFrame>::new_with_payload(
        &gpu,
        DEFAULT_CAPACITY,
        1,
        Arc::new(|| {}),
    );
    let inbox = exchange.inbox();
    let mut binding = FrameBinding::<u64, MediaFrame>::new_with_payload(
        &gpu,
        registry.slot(CLIP),
        HostTextureAlphaMode::Premultiplied,
    );
    let video_id = StableNodeId::new(4).unwrap();
    let mut document = board_document(
        red_layer(),
        |layer| {
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                layer,
                Duration::ZERO,
                ms(400),
                ms(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Filter)
            .with_range(
                MotionValue::Filter(ColorFilter::default().into()),
                MotionTo::Value(MotionValue::Filter(
                    ColorFilter {
                        brightness: 0.0,
                        ..ColorFilter::default()
                    }
                    .into(),
                )),
            )
        },
        |world, queue, board| {
            // The resource plays from 0 s at document time 0.
            let video =
                Video::new(CLIP).media_time(Some(MediaTimeMapping::new(Duration::ZERO, 0.0)));
            queue.create(video_id, DocumentId::new(1).unwrap(), video.node_kind());
            queue.insert(board, video_id, None);
            video.project(video_id, world, queue);
            queue.set_explicit_rect(video_id, Some(VIDEO));
        },
    );
    let green = solid(&gpu, [0, 255, 0, 255]);
    let blue = solid(&gpu, [0, 0, 255, 255]);

    // Nothing published: the stepped frame fails explicitly, undrawn. The
    // request already reads the new time and the caller's 40 ms step.
    let time = frame_time(100, 40);
    let unaligned = |result: Result<_, PresentationError>| match result {
        Err(PresentationError::MediaUnaligned(request)) => request,
        Err(other) => panic!("expected an unaligned resource, got {other}"),
        Ok(_) => panic!("a stepped frame drew without its resource frame"),
    };
    let request = unaligned(draw_with_media(
        &mut driver,
        &mut document,
        time,
        Some(&registry),
        Some(&mut |request| sample(&mut binding, &inbox, request)),
    ));
    assert_eq!(request.node, video_id);
    assert_eq!(request.slot.as_str(), CLIP);
    assert_eq!(request.media_time, 0.1);
    assert_eq!(request.frame_interval, ms(40));
    assert_eq!(request.pacing, MediaPacing::Stepped);

    // A frame 15 ms off stands for this time under the caller's 40 ms step,
    // though half a 60 Hz display interval (8.3 ms) would reject it.
    publish(&mut exchange, &green, 0.115);
    let (frame, early) = draw_with_media(
        &mut driver,
        &mut document,
        time,
        Some(&registry),
        Some(&mut |request| sample(&mut binding, &inbox, request)),
    )
    .expect("aligned frame draws");
    assert_eq!(frame.media.len(), 1);
    assert_eq!(frame.media[0].0.media_time, 0.1);
    assert_eq!(
        frame.media[0].1,
        MediaSampleOutcome::Aligned { changed: true }
    );
    let early = read_pixels(early.texture().unwrap());
    binding.presented(Some(&inbox), |_| true);

    // Each node sits at its explicit rectangle; the resource shows the
    // aligned frame, and the board elsewhere keeps the clear colour.
    let outside = texel(&early, 30, 40);
    assert_eq!(texel(&early, 4, 4), outside);
    let [b, g, r, _] = texel(&early, 44, 16);
    assert!(
        g > 200 && r < 40 && b < 40,
        "video shows the green frame: {:?}",
        [b, g, r]
    );
    assert_ne!(texel(&early, 16, 16), outside, "layer at its rectangle");

    // The next step: the green frame is now 25 ms away, past half the step.
    let time = time.next();
    let request = unaligned(draw_with_media(
        &mut driver,
        &mut document,
        time,
        Some(&registry),
        Some(&mut |request| sample(&mut binding, &inbox, request)),
    ));
    assert_eq!(request.media_time, 0.14);
    publish(&mut exchange, &blue, 0.14);
    let (_, late) = draw_with_media(
        &mut driver,
        &mut document,
        time,
        Some(&registry),
        Some(&mut |request| sample(&mut binding, &inbox, request)),
    )
    .expect("aligned frame draws");
    let late = read_pixels(late.texture().unwrap());
    let [b, g, r, _] = texel(&late, 44, 16);
    assert!(
        b > 200 && r < 40 && g < 40,
        "video shows the blue frame: {:?}",
        [b, g, r]
    );
    // The group filter evaluated at each frame's time: darker later.
    let red = |pixels: &[u8]| texel(pixels, 16, 16)[2];
    assert!(
        red(&late) < red(&early),
        "darker at 140 ms than at 100 ms: {} vs {}",
        red(&late),
        red(&early)
    );
}

/// Issue #321: the working colour is converted to the SDR target's encoding
/// only where the frame leaves the painter. An over-white linear red clips
/// in the BGRA8 output while the Scene and the node keep the extended value.
#[test]
fn an_sdr_target_encodes_at_the_output_boundary_and_keeps_scene_colour() {
    use nana_ui_core::PaintColor;
    let hot = PaintColor::LinearScRgb {
        channels: [2.0, 0.0, 0.0],
        alpha: 1.0,
    };
    let mut layer = red_layer();
    layer.paint_colors.background = Some(hot);
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[consumer(1)]).unwrap();
    let mut document = board_document(
        layer,
        |layer| {
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
            )
        },
        |_, _, _| {},
    );
    let (_, frame) = draw_at(&mut driver, &mut document, frame_time(500, 40));
    let pixels = read_pixels(frame.texture().unwrap());
    let [b, g, r, a] = texel(&pixels, 16, 16);
    assert_eq!(
        (r, a),
        (255, 255),
        "over-white red clips to the SDR maximum"
    );
    assert!(
        g < 40 && b < 40,
        "no hue shift into green or blue: {:?}",
        [b, g, r]
    );
    assert!(
        document.scene().has_extended_color(),
        "the Scene keeps working in extended linear colour"
    );
    let layer = StableNodeId::new(3).unwrap();
    assert_eq!(
        document
            .context()
            .world()
            .node_style(layer)
            .and_then(|style| style.layout.paint_colors.background),
        Some(hot)
    );
}
