//! Acceptance tests through the real windowless driver and GPU transfer.

use super::{
    ExternalFrame, ExternalRenderOutcome, ExternalSurfaceError, OutputAlpha, OutputConsumer,
    OutputConsumerId, OutputConsumerKind, OutputOwnership, OutputTextAa, PresentationDriver,
    PresentationError, PresentationId, PresenterCapabilities, RenderTargetRequirements,
};
use crate::{ScenePresentationProfile, test_gpu};
use nana_gpu::{GpuTexture, GpuTextureDescriptor, GpuTextureFormat, GpuTextureUsages};
use nana_ui_runtime::{DocumentId, Text};
use nana_ui_scene::{RuntimeDocument, UiScene};
use std::time::{Duration, Instant};

const EXTENT: [u32; 2] = [64, 48];
const CLEAR: [f32; 4] = [0.12, 0.35, 0.65, 0.42];

#[path = "driver_tests/diagnostics.rs"]
mod diagnostics;
#[path = "driver_tests/headless_fetch.rs"]
mod headless_fetch;
#[path = "driver_tests/headless_resources.rs"]
mod headless_resources;
#[path = "driver_tests/retained_dependencies.rs"]
mod retained_dependencies;
#[path = "driver_tests/scoped_textures.rs"]
mod scoped_textures;
#[path = "driver_tests/shared_painter.rs"]
mod shared_painter;

fn consumer(id: u64) -> OutputConsumer {
    OutputConsumer {
        id: OutputConsumerId(id),
        kind: OutputConsumerKind::External,
        requirements: RenderTargetRequirements {
            presentation: PresentationId(1),
            viewport_revision: 1,
            extent: EXTENT,
            profile: ScenePresentationProfile::sdr(GpuTextureFormat::BGRA8_UNORM),
            alpha: OutputAlpha::GammaPremultiplied,
            text_aa: OutputTextAa::Grayscale,
            device_generation: test_gpu::context().generation(),
            ownership: OutputOwnership::SameDevice,
            persistent: true,
        },
        capabilities: PresenterCapabilities {
            sampled: true,
            copy_destination: true,
            ..Default::default()
        },
    }
}

fn render(driver: &mut PresentationDriver, scene: &UiScene) -> Vec<ExternalRenderOutcome> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let outcomes = driver
            .render_scene(scene, [64.0, 48.0], 0, CLEAR, None, None)
            .expect("render retained presentation");
        if outcomes
            .iter()
            .all(|outcome| !matches!(outcome, ExternalRenderOutcome::Deferred))
        {
            return outcomes;
        }
        assert!(
            Instant::now() < deadline,
            "shared GPU never made a frame slot available"
        );
        driver.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn render_fresh(driver: &mut PresentationDriver, scene: &UiScene) -> Vec<ExternalRenderOutcome> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut revision = 0;
    loop {
        let outcomes = driver
            .render_scene(scene, [64.0, 48.0], revision, CLEAR, None, None)
            .unwrap();
        if outcomes
            .iter()
            .all(|outcome| matches!(outcome, ExternalRenderOutcome::Submitted { .. }))
        {
            return outcomes;
        }
        assert!(
            Instant::now() < deadline,
            "both groups never acquired a shared frame"
        );
        revision += 1;
        driver.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn completed(driver: &mut PresentationDriver, id: OutputConsumerId) -> ExternalFrame {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match driver.sample(id) {
            Ok(frame) => return frame,
            Err(PresentationError::Surface(ExternalSurfaceError::NoCompletedFrame)) => {}
            Err(error) => panic!("sampling failed: {error}"),
        }
        assert!(Instant::now() < deadline, "presentation did not complete");
        driver.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn compatible_consumers_share_the_actual_texture_and_one_render() {
    let first = consumer(1);
    let second = consumer(2);
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[first, second]).unwrap();
    let scene = UiScene::new();
    let outcomes = render_fresh(&mut driver, &scene);
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(
        outcomes[0],
        ExternalRenderOutcome::Submitted { .. }
    ));
    let a = completed(&mut driver, first.id);
    let b = completed(&mut driver, second.id);
    assert!(a.texture().unwrap().ptr_eq(b.texture().unwrap()));
    assert_eq!(a.content_revision(), b.content_revision());
    assert_eq!(driver.work().output_content_revisions, Some(1));
}

#[test]
fn incompatible_groups_share_one_submission_and_reject_canonical_destinations() {
    let gpu = test_gpu::context();
    let first = consumer(1);
    let mut second = consumer(2);
    second.requirements.extent = [80, 48];
    second.requirements.viewport_revision = 2;
    let mut driver = PresentationDriver::new(&gpu);
    driver.configure(&[first, second]).unwrap();
    let outcomes = render_fresh(&mut driver, &UiScene::new());
    assert_eq!(outcomes.len(), 2);
    assert_eq!(driver.work().output_ui_renders, Some(2));
    assert_eq!(driver.work().output_submissions, Some(1));
    let retained = completed(&mut driver, first.id);
    let destination = retained.texture().unwrap().render_target().unwrap();
    let mut frame = gpu.begin_frame("reject canonical feedback");
    assert!(
        driver
            .present_to(first.id, &mut frame, &destination)
            .is_err()
    );
    frame.discard();
}

#[test]
fn exhausted_host_frame_slots_report_deferred_without_encoding_or_submitting() {
    const CHILD: &str = "NANA_UI_TEST_OUTPUT_SLOT_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // Intentionally exhausting the shared pool must not starve parallel
        // GPU tests. A separate process still owns exactly one test device.
        let _guard = test_gpu::lock();
        test_gpu::run_child_with_timeout(
            "output::driver_tests::exhausted_host_frame_slots_report_deferred_without_encoding_or_submitting",
            CHILD,
            "1",
        );
        return;
    }
    let gpu = test_gpu::context();
    let mut driver = PresentationDriver::new(&gpu);
    driver.configure(&[consumer(1)]).unwrap();
    let mut held = Vec::new();
    while let Some(frame) = gpu.try_begin_frame("hold host frame slot") {
        held.push(frame);
    }
    assert_eq!(held.len(), 3, "fixture owns the default frame-slot pool");
    let outcomes = driver
        .render_scene(&UiScene::new(), [64.0, 48.0], 0, CLEAR, None, None)
        .unwrap();
    assert_eq!(outcomes, [ExternalRenderOutcome::Deferred]);
    assert_eq!(driver.work().output_deferred, Some(1));
    assert_eq!(driver.work().output_ui_renders, Some(0));
    assert_eq!(driver.work().output_submissions, Some(0));
    drop(held);
}

#[test]
fn static_output_sampling_and_idempotent_configuration_produce_zero_work() {
    let endpoint = consumer(1);
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[endpoint]).unwrap();
    let scene = UiScene::new();
    render(&mut driver, &scene);
    let original = completed(&mut driver, endpoint.id);
    for _ in 0..240 {
        assert!(!driver.configure(&[endpoint]).unwrap());
        assert!(matches!(
            render(&mut driver, &scene)[0],
            ExternalRenderOutcome::Reused { .. }
        ));
        let sample = driver.sample(endpoint.id).unwrap();
        assert!(
            sample
                .texture()
                .unwrap()
                .ptr_eq(original.texture().unwrap())
        );
        assert_eq!(sample.content_revision(), original.content_revision());
        let work = driver.work();
        assert_eq!(work.output_content_revisions, Some(0));
        assert_eq!(work.output_target_recreates, Some(0));
        assert_eq!(work.output_extra_passes, Some(0));
        assert_eq!(work.output_gpu_copies, Some(0));
    }
}

#[test]
fn adding_and_removing_compatible_consumers_preserves_the_completed_pool() {
    let first = consumer(1);
    let second = consumer(2);
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[first]).unwrap();
    let scene = UiScene::new();
    render(&mut driver, &scene);
    let before = completed(&mut driver, first.id);
    assert!(driver.configure(&[first, second]).unwrap());
    let added = completed(&mut driver, second.id);
    assert!(before.texture().unwrap().ptr_eq(added.texture().unwrap()));
    assert_eq!(before.resource_generation(), added.resource_generation());
    assert!(matches!(
        render(&mut driver, &scene)[0],
        ExternalRenderOutcome::Reused { .. }
    ));
    assert!(driver.configure(&[second]).unwrap());
    let remaining = completed(&mut driver, second.id);
    assert!(
        before
            .texture()
            .unwrap()
            .ptr_eq(remaining.texture().unwrap())
    );
    assert!(driver.sample(first.id).is_err());
}

#[test]
fn failed_configuration_preserves_the_old_plan_and_completed_pixels() {
    let endpoint = consumer(1);
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[endpoint]).unwrap();
    render(&mut driver, &UiScene::new());
    let before = completed(&mut driver, endpoint.id);
    let mut invalid = consumer(2);
    invalid.requirements.extent = [0, 48];
    assert!(driver.configure(&[invalid]).is_err());
    assert_eq!(driver.plan().groups.len(), 1);
    assert!(driver.plan().binding(endpoint.id).is_some());
    assert!(!driver.configure(&[endpoint]).unwrap());
    let after = completed(&mut driver, endpoint.id);
    assert!(before.texture().unwrap().ptr_eq(after.texture().unwrap()));
    assert_eq!(before.content_revision(), after.content_revision());
}

#[test]
fn document_produces_a_complete_presentation_without_a_window() {
    let endpoint = consumer(1);
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[endpoint]).unwrap();
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    document
        .context_mut()
        .create_component(id, Text::new("Headless presentation"))
        .unwrap();
    let first = driver
        .render_document(&mut document, [64.0, 48.0], 0, CLEAR, None, None)
        .unwrap();
    assert!(first.passes > 0);
    assert!(document.scene().primitives().count() > 0);
    // Runtime flushing succeeds independently of a temporarily unavailable
    // GPU frame slot. Retry presentation without rerunning document mutation.
    render(&mut driver, document.scene());
    completed(&mut driver, endpoint.id);
    let second = driver
        .render_document(&mut document, [64.0, 48.0], 0, CLEAR, None, None)
        .unwrap();
    assert!(second.is_idle());
    assert_eq!(driver.work().output_content_revisions, Some(0));
    assert_eq!(driver.work().output_idle_reuse_frames, Some(1));
}

#[test]
fn final_transfer_preserves_encoded_premultiplied_channels_across_sdr_formats() {
    let formats = [
        GpuTextureFormat::BGRA8_UNORM,
        GpuTextureFormat::BGRA8_UNORM_SRGB,
        GpuTextureFormat::RGBA8_UNORM,
        GpuTextureFormat::RGBA8_UNORM_SRGB,
    ];
    for source_format in formats {
        for format in formats {
            let gpu = test_gpu::context();
            let mut external = consumer(1);
            external.requirements.profile = ScenePresentationProfile::sdr(source_format);
            let mut window = consumer(2);
            window.kind = OutputConsumerKind::Window;
            window.requirements.profile = ScenePresentationProfile::sdr(format);
            let mut driver = PresentationDriver::new(&gpu);
            driver.configure(&[window, external]).unwrap();
            assert_eq!(render(&mut driver, &UiScene::new()).len(), 1);
            let source = completed(&mut driver, external.id);
            let destination = gpu
                .create_texture(&GpuTextureDescriptor {
                    label: Some("presentation transfer acceptance"),
                    width: EXTENT[0],
                    height: EXTENT[1],
                    format,
                    usage: GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC,
                })
                .unwrap();
            let mut frame = gpu.begin_frame("presentation transfer test");
            driver
                .present_to(window.id, &mut frame, &destination.render_target().unwrap())
                .unwrap();
            frame.submit();
            let source = source.texture().unwrap();
            let mut expected = read_pixels(source);
            if matches!(
                source.format(),
                GpuTextureFormat::RGBA8_UNORM | GpuTextureFormat::RGBA8_UNORM_SRGB
            ) {
                for pixel in expected.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
            }
            let mut actual = read_pixels(&destination);
            if matches!(
                format,
                GpuTextureFormat::RGBA8_UNORM | GpuTextureFormat::RGBA8_UNORM_SRGB
            ) {
                for pixel in actual.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
            }
            assert!(
                expected
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| pixel[3] > 0 && pixel[3] < 255)
            );
            assert!(
                expected
                    .iter()
                    .zip(&actual)
                    .all(|(a, b)| a.abs_diff(*b) <= 1),
                "SDR transfer changed encoded premultiplied channels for {source_format:?} -> {format:?}"
            );
            assert_eq!(driver.work().output_extra_passes, Some(1));
            assert_eq!(driver.work().output_gpu_convert_passes, Some(1));
        }
    }
}

#[test]
fn incompatible_profiles_share_one_glyph_atlas_and_upload_glyphs_once() {
    let gpu = test_gpu::context();
    let bgra = ScenePresentationProfile::sdr(GpuTextureFormat::BGRA8_UNORM);
    let float = bgra.with_force_float_working();
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    document
        .context_mut()
        .create_component(id, Text::new("Shared glyph atlas"))
        .unwrap();
    super::driver::flush_presentation(
        &mut document,
        nana_ui_runtime::LayoutViewport::new(64.0, 48.0),
        &mut crate::NanaTextShaper::default(),
    )
    .unwrap();
    let scene = document.scene();
    // Each run is a fresh driver, so the figures are a cold first frame.
    let run = |profiles: &[ScenePresentationProfile]| {
        let consumers: Vec<_> = profiles
            .iter()
            .zip(1..)
            .map(|(profile, id)| {
                let mut endpoint = consumer(id);
                endpoint.requirements.profile = *profile;
                endpoint
            })
            .collect();
        let mut driver = PresentationDriver::new(&gpu);
        driver.configure(&consumers).unwrap();
        assert_eq!(driver.plan().groups.len(), profiles.len());
        render_fresh(&mut driver, scene);
        let uploads = driver.work().gpu_upload_bytes.unwrap();
        let pixels: Vec<_> = consumers
            .iter()
            .map(|endpoint| read_pixels(completed(&mut driver, endpoint.id).texture().unwrap()))
            .collect();
        (driver.text_glyph_counters().unwrap(), uploads, pixels)
    };
    let (one, one_uploads, one_pixels) = run(&[bgra]);
    let (_, _, float_pixels) = run(&[float]);
    let (two, two_uploads, two_pixels) = run(&[bgra, float]);
    assert!(one.glyph_rasterized > 0, "the fixture draws text");
    assert_eq!(
        two.glyph_atlas_bytes, one.glyph_atlas_bytes,
        "a second profile adds no atlas pages"
    );
    assert_eq!(
        two.glyph_raster_cache_bytes, one.glyph_raster_cache_bytes,
        "nor raster cache entries"
    );
    assert_eq!(
        two.glyph_upload_bytes, one.glyph_upload_bytes,
        "its glyphs are uploaded once"
    );
    assert!(
        two_uploads <= 2 * one_uploads - one.glyph_upload_bytes as usize,
        "the second group uploads only its own instances: {two_uploads} for two, {one_uploads} for one"
    );
    assert_eq!(two_pixels[0], one_pixels[0], "the first profile's pixels");
    assert_eq!(
        two_pixels[1], float_pixels[0],
        "the second profile's pixels"
    );
}

/// Timing evidence is opt-in and run in release mode; structural invariants
/// remain the ordinary CI gate, independent of the speed of its GPU runner.
#[test]
#[ignore = "release latency evidence; run with --ignored --nocapture --test-threads=1"]
fn retained_output_latency_for_one_and_eight_consumers() {
    let gpu = test_gpu::context();
    let mut driver = PresentationDriver::new(&gpu);
    let scene = UiScene::new();
    for count in [1, 8] {
        let consumers: Vec<_> = (1..=count).map(consumer).collect();
        driver.configure(&consumers).unwrap();
        render(&mut driver, &scene);
        let initial = completed(&mut driver, consumers[0].id);
        let mut configuration = Vec::with_capacity(400);
        let mut producer = Vec::with_capacity(400);
        let mut sampling = Vec::with_capacity(400);
        for iteration in 0..460 {
            let start = Instant::now();
            assert!(!driver.configure(&consumers).unwrap());
            let configure_time = start.elapsed();
            let start = Instant::now();
            assert!(matches!(
                render(&mut driver, &scene)[0],
                ExternalRenderOutcome::Reused { .. }
            ));
            let render_time = start.elapsed();
            let start = Instant::now();
            for endpoint in &consumers {
                let frame = driver.sample(endpoint.id).unwrap();
                assert!(frame.texture().unwrap().ptr_eq(initial.texture().unwrap()));
            }
            let sample_time = start.elapsed();
            assert_eq!(driver.work().output_content_revisions, Some(0));
            assert_eq!(driver.work().output_target_recreates, Some(0));
            assert_eq!(driver.work().output_extra_passes, Some(0));
            assert_eq!(driver.work().output_gpu_copies, Some(0));
            assert_eq!(driver.plan().render_count(), 1);
            if iteration >= 60 {
                configuration.push(configure_time.as_secs_f64() * 1_000_000.0);
                producer.push(render_time.as_secs_f64() * 1_000_000.0);
                sampling.push(sample_time.as_secs_f64() * 1_000_000.0);
            }
        }
        for (stage, mut observations) in [
            ("unchanged_configure", configuration),
            ("idle_producer", producer),
            ("consumer_sampling", sampling),
        ] {
            observations.sort_by(f64::total_cmp);
            println!(
                "output consumers={count} stage={stage} unit=us p50={:.3} p95={:.3} p99={:.3} render_count=0 recreates=0 passes=0 copies=0",
                observations[199], observations[379], observations[395]
            );
        }
    }
}

/// CPU readback is confined to this explicit pixel acceptance fixture.
pub(super) fn read_pixels(texture: &GpuTexture) -> Vec<u8> {
    use nana_gpu::__framework as raw;
    let gpu = test_gpu::context();
    let device = raw::device(&gpu);
    let (width, height) = texture.size();
    let row = width * 4;
    let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("presentation acceptance pixels"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut frame = gpu.begin_frame("presentation acceptance readback");
    raw::encoder(&mut frame).copy_texture_to_buffer(
        raw::texture(texture).as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    frame.submit();
    let (sender, receiver) = std::sync::mpsc::channel();
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).unwrap()
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        gpu.poll();
        if let Ok(result) = receiver.try_recv() {
            result.unwrap();
            break;
        }
        assert!(Instant::now() < deadline, "pixel readback did not complete");
        std::thread::sleep(Duration::from_millis(1));
    }
    let mapped = slice.get_mapped_range().unwrap();
    mapped
        .chunks_exact(padded as usize)
        .flat_map(|row_bytes| row_bytes[..row as usize].iter().copied())
        .collect()
}
