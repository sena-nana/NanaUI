//! Dependencies which change pixels without changing the retained projection.
use super::*;
use nana_ui_core::{BackgroundImage, BackgroundImageFit, ImageSampling, LayoutStyle};
use nana_ui_runtime::{ComputedStyle, ExtractedNode, LayoutBox, NodeKind, NodeStyle, StableNodeId};
use std::sync::Arc;

fn quad(layout: LayoutStyle) -> ExtractedNode {
    ExtractedNode {
        layout_result: None,
        custom_paint: None,
        chrome_radii: Default::default(),
        id: StableNodeId::new(1).unwrap(),
        kind: Arc::new(NodeKind::Element { tag: "div".into() }),
        parent: None,
        visual_parent: None,
        overlay_visual_parent: None,
        children: Arc::new(Vec::new()),
        layout: LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 40.0,
        },
        local_layout: Default::default(),
        layout_frame: nana_ui_runtime::LayoutFrame::Page,
        scroll_offset: Default::default(),
        style: Arc::new(ComputedStyle {
            background: layout.background,
            ..Default::default()
        }),
        source_style: NodeStyle {
            layout: Arc::new(layout),
            ..Default::default()
        },
        text: None,
        text_metrics: None,
        text_layout: None,
        text_preserve_lines: false,
        z_index: 0,
        focused: false,
        editable: false,
        text_spans: Vec::new(),
        standard_visual: None,
        component_geometry: None,
        standard_visual_foreground: None,
        custom_render: None,
        drop_hover: None,
        document_text_selection: Vec::new(),
        document_text_selection_color: [0.0; 4],
        compositor: Default::default(),
        rich_text: None,
        rich_editor: None,
        glyph_presentation: None,
    }
}

fn latest(driver: &mut PresentationDriver, scene: &UiScene) -> ExternalFrame {
    let wanted = match render(driver, scene)[0] {
        ExternalRenderOutcome::Submitted {
            content_revision, ..
        }
        | ExternalRenderOutcome::Reused {
            content_revision, ..
        } => content_revision,
        ExternalRenderOutcome::Deferred => unreachable!(),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(frame) = driver.sample(OutputConsumerId(1))
            && frame.content_revision() == wanted
        {
            return frame;
        }
        assert!(
            Instant::now() < deadline,
            "latest dependent output did not complete"
        );
        driver.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn assert_idle(driver: &mut PresentationDriver, scene: &UiScene) {
    assert!(matches!(
        render(driver, scene)[0],
        ExternalRenderOutcome::Reused { .. }
    ));
    assert_eq!(driver.work().output_ui_renders, Some(0));
    assert_eq!(driver.work().output_submissions, Some(0));
    assert_eq!(driver.work().output_gpu_copies, Some(0));
    assert_eq!(driver.work().output_extra_passes, Some(0));
}

#[test]
fn gpu_compositor_clock_changes_pixels_without_invalidating_the_projection() {
    use nana_ui_core::motion::{
        AnimatableProperty, Easing, MotionTo, MotionValue, PresentationStore,
    };
    use nana_ui_runtime::{AnimationId, AnimationSpec, MutationQueue, UiWorld};
    let node = StableNodeId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut mutations = MutationQueue::new();
    mutations.create(node, DocumentId::new(1).unwrap(), NodeKind::Document);
    mutations.start_animation(
        AnimationSpec::new(
            AnimationId::new(1).unwrap(),
            node,
            Duration::ZERO,
            Duration::from_millis(400),
            Duration::from_millis(16),
            Easing::Linear,
        )
        .with_property(AnimatableProperty::Opacity)
        .with_range(
            MotionValue::Scalar(0.1),
            MotionTo::Value(MotionValue::Scalar(1.0)),
        ),
    );
    world.commit(mutations).unwrap();
    world.advance_animations(Duration::ZERO);
    let colored = || {
        quad(LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            ..Default::default()
        })
    };
    let mut drawable = colored();
    drawable.compositor = world.extract_nodes(&[node])[0].compositor.clone();
    assert!(!drawable.compositor.bindings.is_empty());
    // Keep the declared GPU binding but omit CPU overlays: only the shader's
    // descriptor clock changes. CPU-sampled overlays also move projection.
    let overlays = PresentationStore::default();
    let mut scene = UiScene::new();
    scene.apply_delta([drawable], []);
    scene.request_compositor_layer(node);
    for millis in [0, 16, 80] {
        scene.apply_presentation(
            &overlays,
            Duration::from_millis(millis),
            Some(world.motion_descriptors()),
        );
    }
    assert!(scene.compositor_needs_tick());
    assert!(!scene.motion_gpu_descriptors().is_empty());
    let projection = scene.projection_revision();
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[consumer(1)]).unwrap();
    let first = latest(&mut driver, &scene);
    let first_pixels = read_pixels(first.texture().unwrap());
    assert_idle(&mut driver, &scene);
    scene.apply_presentation(
        &overlays,
        Duration::from_millis(240),
        Some(world.motion_descriptors()),
    );
    assert_eq!(scene.projection_revision(), projection);
    let second = latest(&mut driver, &scene);
    assert!(second.content_revision() > first.content_revision());
    assert_ne!(
        first_pixels,
        read_pixels(second.texture().unwrap()),
        "GPU opacity clock must reach the pixels"
    );
    assert_idle(&mut driver, &scene);
    drop((first, second));
    world.advance_animations(Duration::from_millis(400));
    // Runtime removes the completed binding at its extraction boundary.
    scene.apply_delta([colored()], []);
    scene.clear_compositor_layer_request(node);
    for millis in [400, 600, 1000] {
        scene.apply_presentation(
            &overlays,
            Duration::from_millis(millis),
            Some(world.motion_descriptors()),
        );
        drop(latest(&mut driver, &scene));
    }
    assert!(!scene.compositor_needs_tick());
    scene.apply_presentation(
        &overlays,
        Duration::from_millis(1100),
        Some(world.motion_descriptors()),
    );
    assert_idle(&mut driver, &scene);
}

#[test]
fn asynchronous_image_completion_repaints_static_output_then_returns_to_zero_work() {
    use base64::Engine as _;
    let image = image::RgbaImage::from_fn(256, 256, |x, _| {
        let value = if x % 2 == 0 { 0 } else { 255 };
        image::Rgba([value, value, value, 255])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
    );
    let mut style = LayoutStyle::default();
    style.paint.background_image = Some(
        BackgroundImage::url_with_fit(url, BackgroundImageFit::Stretch)
            .with_sampling(ImageSampling::Resample),
    );
    let mut scene = UiScene::new();
    scene.apply_delta([quad(style)], []);
    let projection = scene.projection_revision();
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[consumer(1)]).unwrap();
    let first = latest(&mut driver, &scene);
    let initial = read_pixels(first.texture().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    let final_frame = loop {
        let frame = latest(&mut driver, &scene);
        if frame.content_revision() > first.content_revision() && !driver.has_pending_image_work() {
            break frame;
        }
        assert!(
            Instant::now() < deadline,
            "static output never collected its asynchronous image result"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(scene.projection_revision(), projection);
    let settled = read_pixels(final_frame.texture().unwrap());
    assert_ne!(
        initial, settled,
        "resampling must change actual output pixels"
    );
    let values = (4..36)
        .flat_map(|y| (4..36).map(move |x| (y * 64 + x) * 4))
        .map(|offset| settled[offset]);
    let (low, high) = values.fold((u8::MAX, u8::MIN), |(lo, hi), value| {
        (lo.min(value), hi.max(value))
    });
    assert!(
        high - low <= 4,
        "settled stripe image should be flat, got {low}..{high}"
    );
    for _ in 0..8 {
        assert_idle(&mut driver, &scene);
    }
}
