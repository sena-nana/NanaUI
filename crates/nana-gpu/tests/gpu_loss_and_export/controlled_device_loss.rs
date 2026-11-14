//! Real backend callbacks for controlled device destruction. This is not a
//! hardware/driver-reset or foreign-consumer conformance test.
use nana_gpu::{
    GpuContext, GpuError, GpuLossReason, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion,
    GpuTextureUsages,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "backend callback did not arrive");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn callback_ownership_and_replacement_follow_real_device_destruction() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("controlled loss test requires a real WGPU adapter");
    let request =
        || pollster::block_on(adapter.request_device(&Default::default())).expect("test device");

    // The existing constructor must leave a host's callback alone.
    let (device, queue) = request();
    let host_saw_loss = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&host_saw_loss);
    device.set_device_lost_callback(move |reason, _| {
        seen.store(
            reason == wgpu::DeviceLostReason::Destroyed,
            Ordering::Release,
        );
    });
    let supplied = GpuContext::from_wgpu(adapter.clone(), device, queue);
    supplied.wgpu().device().destroy();
    until(|| {
        supplied.poll();
        host_saw_loss.load(Ordering::Acquire)
    });

    // A standalone producer opts in to NanaUI's loss authority.
    let (device, queue) = request();
    let old = GpuContext::from_wgpu_with_loss_tracking(adapter.clone(), device, queue);
    let observer = old.clone();
    let descriptor = GpuTextureDescriptor {
        label: Some("controlled loss retained allocation"),
        width: 1,
        height: 1,
        format: GpuTextureFormat::RGBA8_UNORM,
        usage: GpuTextureUsages::COPY_DST,
    };
    let retained = old.create_texture(&descriptor).unwrap();
    let _in_flight = old
        .begin_frame("controlled loss outstanding submission")
        .submit();
    old.wgpu().device().destroy();
    until(|| {
        old.poll();
        observer.is_lost()
    });
    assert_eq!(old.lost_report().unwrap().reason, GpuLossReason::Destroyed);

    let (device, queue) = request();
    let replacement = GpuContext::from_wgpu_with_loss_tracking(adapter, device, queue);
    assert_ne!(old.generation(), replacement.generation());
    assert!(!replacement.is_lost());
    assert!(
        observer.is_lost(),
        "a new device cannot clear the old sticky loss"
    );
    assert_eq!(
        replacement.write_texture(&retained, GpuTextureRegion::full(1, 1), &[0; 4], 4),
        Err(GpuError::DeviceMismatch {
            expected: replacement.generation(),
            found: old.generation(),
        }),
    );
    let fresh = replacement.create_texture(&descriptor).unwrap();
    replacement
        .write_texture(&fresh, GpuTextureRegion::full(1, 1), &[0; 4], 4)
        .unwrap();
    let complete = replacement.begin_frame("replacement upload").submit();
    let completed = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&completed);
    replacement
        .on_submission_complete(&complete, move || done.store(true, Ordering::Release))
        .unwrap();
    until(|| {
        replacement.poll();
        completed.load(Ordering::Acquire)
    });
}
