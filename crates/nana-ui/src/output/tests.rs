//! GPU acceptance fixtures for retained, window-independent output.
//!
//! These tests intentionally use the process-shared test GPU.  They exercise
//! the producer contract only; a consumer samples a completed texture and
//! never owns a frame encoder or submits work.

use super::{ExternalRenderOutcome, ExternalSurface, ExternalSurfaceConfig, ExternalSurfaceError};
use crate::{ScenePaintViewport, test_gpu};
use nana_gpu::GpuTextureFormat;
use nana_ui_scene::UiScene;
use std::time::{Duration, Instant};

const EXTENT: [u32; 2] = [64, 48];

fn viewport(extent: [u32; 2]) -> ScenePaintViewport {
    ScenePaintViewport {
        logical_size: [extent[0] as f32, extent[1] as f32],
        physical_size: extent,
        scale_factor: 1.0,
        scene_origin: [0.0, 0.0],
        target_origin: [0.0, 0.0],
        clear_color: [0.02, 0.03, 0.04, 1.0],
        clear: true,
    }
}

fn surface() -> ExternalSurface {
    ExternalSurface::new(
        &test_gpu::context(),
        ExternalSurfaceConfig::new(EXTENT, GpuTextureFormat::RGBA8_UNORM),
    )
    .expect("test GPU must support an RGBA8 retained target")
}

/// Queue callbacks are driven by `GpuContext::poll`, which is deliberately
/// non-blocking.  Keep this bounded: a hung adapter fails the test instead of
/// making the suite wait forever.
fn render_initial(surface: &mut ExternalSurface, scene: &UiScene) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match surface
            .render(scene, 0, viewport(EXTENT), None, None)
            .expect("initial retained render")
        {
            ExternalRenderOutcome::Submitted { .. } => return,
            ExternalRenderOutcome::Deferred => {
                surface.poll();
                assert!(
                    Instant::now() < deadline,
                    "retained output never acquired a frame slot"
                );
                std::thread::yield_now();
            }
            ExternalRenderOutcome::Reused { .. } => {
                panic!("new surface unexpectedly had a published frame")
            }
        }
    }
}

fn completed_sample(surface: &mut ExternalSurface) -> super::ExternalFrame {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        surface.poll();
        if let Ok(frame) = surface.sample() {
            return frame;
        }
        assert!(
            Instant::now() < deadline,
            "retained output did not complete"
        );
        std::thread::yield_now();
    }
}

#[test]
fn static_output_can_be_sampled_240_times_without_new_producer_work() {
    let mut surface = surface();
    let scene = UiScene::new();
    render_initial(&mut surface, &scene);
    let first_frame = completed_sample(&mut surface);
    let revision = first_frame.content_revision();
    let generation = first_frame.resource_generation();
    drop(first_frame);

    let mut samples = 0usize;
    for _ in 0..240 {
        let outcome = surface
            .render(&scene, 0, viewport(EXTENT), None, None)
            .expect("static render must be reusable");
        assert_eq!(
            outcome,
            ExternalRenderOutcome::Reused {
                resource_generation: generation,
                content_revision: revision,
            }
        );
        let frame = surface
            .sample()
            .expect("completed frame remains sampleable");
        assert_eq!(frame.resource_generation(), generation);
        assert_eq!(frame.content_revision(), revision);
        samples += 1;
        drop(frame);
    }
    assert_eq!(samples, 240);
    let work = surface.last_work();
    assert_eq!(work.content_revisions, 0);
    assert_eq!(work.target_recreates, 0);
    assert_eq!(work.extra_passes, 0);
    assert_eq!(work.gpu_copies, 0);
    assert_eq!(work.idle_reuse_frames, 1);
}

#[test]
fn multiple_consumers_can_lease_the_same_completed_revision() {
    let mut surface = surface();
    let scene = UiScene::new();
    render_initial(&mut surface, &scene);
    let first = completed_sample(&mut surface);
    let second = surface.sample().expect("second consumer lease");
    assert_eq!(first.resource_generation(), second.resource_generation());
    assert_eq!(first.content_revision(), second.content_revision());
    assert_eq!(
        first.texture().unwrap().generation(),
        second.texture().unwrap().generation()
    );
    drop(first);
    drop(second);
}

#[test]
fn resize_invalidates_old_leases_and_noop_resize_does_not_recreate() {
    let gpu = test_gpu::context();
    let config = ExternalSurfaceConfig::new(EXTENT, GpuTextureFormat::RGBA8_UNORM);
    let mut surface = ExternalSurface::new(&gpu, config).unwrap();
    let scene = UiScene::new();
    render_initial(&mut surface, &scene);
    let old = completed_sample(&mut surface);
    let generation = surface.resource_generation();

    surface
        .resize(&gpu, config)
        .expect("equal config is a no-op");
    assert_eq!(surface.resource_generation(), generation);
    assert_eq!(surface.last_work(), Default::default());
    old.validate().expect("no-op resize keeps leases current");

    let resized = ExternalSurfaceConfig::new([80, 48], GpuTextureFormat::RGBA8_UNORM);
    surface
        .resize(&gpu, resized)
        .expect("resize retained target");
    assert!(surface.resource_generation() > generation);
    assert_eq!(old.validate(), Err(ExternalSurfaceError::StaleFrame));
    assert!(surface.last_work().target_recreates >= resized.slots);
}

#[test]
fn failed_resize_preserves_the_current_completed_frame() {
    let gpu = test_gpu::context();
    let config = ExternalSurfaceConfig::new(EXTENT, GpuTextureFormat::RGBA8_UNORM);
    let mut surface = ExternalSurface::new(&gpu, config).unwrap();
    let scene = UiScene::new();
    render_initial(&mut surface, &scene);
    let frame = completed_sample(&mut surface);
    let generation = surface.resource_generation();

    let invalid = ExternalSurfaceConfig::new([0, EXTENT[1]], GpuTextureFormat::RGBA8_UNORM);
    assert_eq!(
        surface.resize(&gpu, invalid),
        Err(ExternalSurfaceError::InvalidExtent)
    );
    assert_eq!(surface.resource_generation(), generation);
    frame
        .validate()
        .expect("failed resize leaves old lease valid");
    assert_eq!(surface.published_revision(), Some(frame.content_revision()));
}

#[test]
fn a_render_of_new_content_never_reports_the_older_revision_as_reused() {
    let mut surface = surface();
    let scene = UiScene::new();
    render_initial(&mut surface, &scene);
    let first = completed_sample(&mut surface).content_revision();
    // A new host revision is new content: submitted, not yet completed. A
    // slow adapter may still hold the first frame's slot; wait for one.
    let deadline = Instant::now() + Duration::from_secs(5);
    let second = loop {
        match surface
            .render(&scene, 1, viewport(EXTENT), None, None)
            .expect("changed content renders")
        {
            ExternalRenderOutcome::Submitted {
                content_revision, ..
            } => break content_revision,
            ExternalRenderOutcome::Deferred => {
                surface.poll();
                assert!(Instant::now() < deadline, "no slot for new content");
                std::thread::yield_now();
            }
            ExternalRenderOutcome::Reused {
                content_revision, ..
            } => {
                panic!("new content reported as reused revision {content_revision}")
            }
        }
    };
    assert_ne!(first, second);
    // Asked again for the same new content before it completes, the surface
    // may defer or reuse the new revision, never offer the old one as it.
    for _ in 0..8 {
        match surface
            .render(&scene, 1, viewport(EXTENT), None, None)
            .expect("repeat render")
        {
            ExternalRenderOutcome::Reused {
                content_revision, ..
            } => assert_eq!(content_revision, second),
            ExternalRenderOutcome::Deferred => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}
