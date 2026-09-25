//! Device-backed checks of the GPU contract.

use std::sync::mpsc;
use std::time::Duration;

use nana_gpu::__framework;
use nana_gpu::{
    GpuContext, GpuError, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion,
    GpuTextureUsages, RetainedWrites,
};

fn context() -> GpuContext {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))
    .expect("GPU contract tests require a WGPU adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("GPU contract tests require a WGPU device");
    __framework::adopt(adapter, device, queue)
}

fn texture(gpu: &GpuContext, usage: GpuTextureUsages) -> nana_gpu::GpuTexture {
    gpu.create_texture(&GpuTextureDescriptor {
        label: Some("contract test"),
        width: 4,
        height: 2,
        format: GpuTextureFormat::RGBA8_UNORM,
        usage,
    })
    .expect("4x2 rgba8 texture")
}

#[test]
fn contexts_share_a_generation_only_with_their_clones() {
    let a = context();
    let b = context();
    assert!(a.same_device(&a.clone()));
    assert_eq!(a.generation(), a.clone().generation());
    assert!(!a.same_device(&b));
    let first = a.begin_frame("first");
    let second = a.begin_frame("second");
    assert!(second.id() > first.id());
    assert_eq!(first.generation(), a.generation());
}

#[test]
fn texture_creation_refuses_extents_the_device_cannot_hold() {
    let gpu = context();
    let max = gpu.capabilities().max_texture_dimension_2d();
    for (width, height) in [(0, 1), (1, 0), (max + 1, 1)] {
        let error = gpu
            .create_texture(&GpuTextureDescriptor {
                label: None,
                width,
                height,
                format: GpuTextureFormat::RGBA8_UNORM,
                usage: GpuTextureUsages::SAMPLED,
            })
            .expect_err("invalid extent");
        assert_eq!(error, GpuError::InvalidExtent { width, height, max });
    }
    let compressed = __framework::format_from_wgpu(wgpu::TextureFormat::Bc1RgbaUnorm);
    assert_eq!(
        gpu.create_texture(&GpuTextureDescriptor {
            label: None,
            width: 6,
            height: 4,
            format: compressed,
            usage: GpuTextureUsages::SAMPLED,
        })
        .expect_err("not whole 4x4 blocks"),
        GpuError::InvalidExtent {
            width: 6,
            height: 4,
            max
        }
    );
    assert_eq!(
        gpu.create_texture(&GpuTextureDescriptor {
            label: None,
            width: 1,
            height: 1,
            format: GpuTextureFormat::RGBA8_UNORM,
            usage: GpuTextureUsages::empty(),
        })
        .expect_err("no usage"),
        GpuError::EmptyUsage
    );
    let target = texture(
        &gpu,
        GpuTextureUsages::SAMPLED | GpuTextureUsages::RENDER_TARGET,
    );
    let view = target.render_target().expect("render target usage");
    assert_eq!(view.size(), [4, 2]);
    assert_eq!(view.generation(), gpu.generation());
    assert_eq!(
        texture(&gpu, GpuTextureUsages::SAMPLED)
            .render_target()
            .expect_err("no render target usage"),
        GpuError::MissingUsage(GpuTextureUsages::RENDER_TARGET)
    );
}

#[test]
fn uploads_are_validated_before_they_reach_the_queue() {
    let gpu = context();
    let other = context();
    let writable = texture(&gpu, GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST);
    let row = [0u8; 16];
    let pixels = [0u8; 32];
    assert_eq!(
        gpu.write_texture(&writable, GpuTextureRegion::full(4, 2), &pixels, 16),
        Ok(())
    );
    assert_eq!(
        gpu.write_texture(
            &writable,
            GpuTextureRegion {
                x: 1,
                y: 1,
                width: 2,
                height: 1
            },
            &row[..8],
            8
        ),
        Ok(()),
        "a dirty rect uploads only its own bytes"
    );
    assert_eq!(
        other.write_texture(&writable, GpuTextureRegion::full(4, 2), &pixels, 16),
        Err(GpuError::DeviceMismatch {
            expected: other.generation(),
            found: gpu.generation(),
        })
    );
    assert_eq!(
        gpu.write_texture(
            &texture(&gpu, GpuTextureUsages::SAMPLED),
            GpuTextureRegion::full(4, 2),
            &pixels,
            16
        ),
        Err(GpuError::MissingUsage(GpuTextureUsages::COPY_DST))
    );
    for region in [
        GpuTextureRegion::full(5, 2),
        GpuTextureRegion {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 1,
        },
        GpuTextureRegion {
            x: 5,
            y: 0,
            width: 0,
            height: 1,
        },
    ] {
        assert_eq!(
            gpu.write_texture(&writable, region, &pixels, 32),
            Err(GpuError::RegionOutOfBounds)
        );
    }
    assert_eq!(
        gpu.write_texture(&writable, GpuTextureRegion::full(0, 2), &[], 0),
        Ok(()),
        "an empty region inside the texture is a no-op, as in WGPU"
    );
    assert_eq!(
        gpu.write_texture(&writable, GpuTextureRegion::full(4, 2), &pixels, 12),
        Err(GpuError::RowTooShort {
            needed: 16,
            provided: 12
        })
    );
    assert_eq!(
        gpu.write_texture(&writable, GpuTextureRegion::full(4, 2), &pixels[..31], 16),
        Err(GpuError::DataTooShort {
            needed: 32,
            provided: 31
        })
    );
}

#[test]
fn a_dropped_frame_rolls_back_what_it_recorded_and_a_submitted_one_settles() {
    let gpu = context();
    let ledger = RetainedWrites::new();

    let mut submitted = gpu.begin_frame("submitted");
    submitted.record_retained_writes(&ledger, 1);
    submitted.record_retained_writes(&ledger, 1);
    let later = gpu.begin_frame("later");
    assert!(ledger.in_flight(1, later.id()));
    let submission = submitted.submit();
    assert_eq!(submission.generation(), gpu.generation());
    assert!(!ledger.in_flight(1, later.id()));
    assert!(!ledger.has_rolled_back());
    drop(later);
    assert!(!ledger.has_rolled_back(), "nothing was recorded into it");

    let mut discarded = gpu.begin_frame("discarded");
    discarded.record_retained_writes(&ledger, 1);
    discarded.record_retained_writes(&ledger, 2);
    discarded.discard();
    let mut keys = Vec::new();
    ledger.drain_rolled_back(|key| keys.push(key));
    keys.sort_unstable();
    assert_eq!(keys, [1, 2]);
    assert!(!ledger.has_rolled_back());
}

#[test]
fn submission_waits_while_a_surface_reconfigures() {
    let gpu = context();
    let (started, waiting) = mpsc::channel();
    let (submitted, submissions) = mpsc::channel();
    let writer = gpu.clone();
    let gate = __framework::lock_reconfigure(&writer);
    let producer = {
        let gpu = gpu.clone();
        std::thread::spawn(move || {
            started.send(()).unwrap();
            gpu.begin_frame("producer").submit();
            submitted.send(()).unwrap();
        })
    };
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        submissions
            .recv_timeout(Duration::from_millis(200))
            .is_err(),
        "a submit must wait while the surface is reconfiguring"
    );
    drop(gate);
    submissions.recv_timeout(Duration::from_secs(5)).unwrap();
    producer.join().unwrap();
}

#[test]
fn a_loss_marked_by_the_host_is_sticky_and_reported() {
    let gpu = context();
    assert!(!gpu.is_lost());
    __framework::mark_lost(
        &gpu,
        nana_gpu::GpuDeviceLost {
            reason: nana_gpu::GpuLossReason::Destroyed,
            message: "host".into(),
        },
    );
    assert!(gpu.clone().is_lost());
    assert_eq!(gpu.lost_report().unwrap().message, "host");
}
