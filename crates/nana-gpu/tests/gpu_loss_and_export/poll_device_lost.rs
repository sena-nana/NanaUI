//! A device lost while polling marks the context lost, and the frame paths
//! that poll or wait keep working instead of panicking.
//!
//! Uses the wgpu fork's test-only Vulkan hook (`simulate_device_lost`), which
//! makes the backend report `VK_ERROR_DEVICE_LOST` without a real GPU reset.
//! The context is adopted with `from_wgpu`, which installs no device-lost
//! callback, so the loss can only reach it through a poll's
//! `PollError::DeviceLost`. Skips without a Vulkan adapter.
#![cfg(all(
    feature = "wgpu-interop",
    any(target_os = "linux", target_os = "windows", target_os = "android")
))]

use nana_gpu::{
    GpuContext, GpuLossReason, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion,
    GpuTextureUsages,
};
use std::time::{Duration, Instant};

#[test]
fn a_poll_that_finds_the_device_lost_marks_the_context_lost() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        println!("skipped: no Vulkan adapter");
        return;
    };
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Default::default())).expect("test device");
    let raw = device.clone();
    let gpu = GpuContext::from_wgpu(adapter, device, queue);
    let descriptor = GpuTextureDescriptor {
        label: Some("poll device lost"),
        width: 4,
        height: 4,
        format: GpuTextureFormat::RGBA8_UNORM,
        usage: GpuTextureUsages::COPY_DST,
    };
    let texture = gpu.create_texture(&descriptor).unwrap();
    // Fill every frame slot so the next `begin_frame` has to wait.
    for _ in 0..8 {
        let _ = gpu.begin_frame("before the loss").submit();
    }

    // SAFETY: the hal device is only used to set the test hook.
    let hal = unsafe { raw.as_hal::<wgpu::hal::api::Vulkan>() }.expect("a Vulkan device");
    hal.simulate_device_lost();
    drop(hal);
    assert!(!gpu.is_lost(), "nothing has polled yet");

    let started = Instant::now();
    // Frame paths that poll or wait: none may panic or block for long.
    for _ in 0..8 {
        let _ = gpu.begin_frame("after the loss").submit();
    }
    assert!(gpu.is_lost(), "the frame-slot wait found the loss");
    let report = gpu.lost_report().expect("a loss report");
    assert_eq!(report.reason, GpuLossReason::Unknown);

    for _ in 0..8 {
        let _ = gpu.begin_producer_frame("producer after the loss").submit();
    }
    let _ = gpu.try_begin_producer_frame("try producer after the loss");
    let _ = gpu.try_begin_frame("try frame after the loss");
    let _ = gpu.write_texture(&texture, GpuTextureRegion::full(4, 4), &[0; 64], 16);
    let _ = gpu.create_texture(&descriptor);
    gpu.poll();
    assert!(gpu.is_lost(), "loss is sticky");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "a lost device ends waits at once, took {:?}",
        started.elapsed()
    );
}
