//! A lost device is reported by `Device::poll`, not a panic.
//!
//! Upstream `wgpu::Device::poll` panicked when the backend's fence wait or
//! fence read reported the device lost (`VK_ERROR_DEVICE_LOST`). The fork
//! returns `PollError::DeviceLost`, runs the device-lost callback once, counts
//! every submission as finished (so completion and mapping callbacks run),
//! and keeps reporting the loss on later polls. On DX12 a removed device's
//! fences read `u64::MAX`, which upstream took as "all work finished"; the
//! fork checks `GetDeviceRemovedReason` and reports the same loss.
//!
//! No real GPU reset is triggered. Vulkan uses the fork's test-only
//! `simulate_device_lost` hook on the hal device (it waits for the device to
//! go idle, then makes fence waits, fence reads and submits report
//! `DeviceError::Lost`). DX12 uses `ID3D12Device5::RemoveDevice`, D3D12's own
//! API for testing device removal, which removes only this device. The load
//! is one small buffer copy.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nana_wgpu_fork_tests::{device, no_adapter};

const WAIT: Duration = Duration::from_secs(5);

fn buffer(device: &wgpu::Device, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("device lost poll test"),
        size: 256,
        usage,
        mapped_at_creation: false,
    })
}

fn copy(device: &wgpu::Device, queue: &wgpu::Queue, from: &wgpu::Buffer, to: &wgpu::Buffer) {
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(from, 0, to, 0, from.size());
    queue.submit([encoder.finish()]);
}

/// Lose `device` with `lose`, then check what polls report and which
/// callbacks ran.
fn check(test: &str, backend: wgpu::Backends, lose: impl FnOnce(&wgpu::Device) -> bool) {
    let Some((_, device, queue)) = device(backend, wgpu::Features::empty()) else {
        no_adapter(test, backend);
        return;
    };
    let lost = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&lost);
    device.set_device_lost_callback(move |reason, message| {
        seen.lock().unwrap().push((reason, message));
    });

    let from = buffer(
        &device,
        wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
    );
    let to = buffer(
        &device,
        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    );
    copy(&device, &queue, &from, &to);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(WAIT),
        })
        .expect("the copy finishes before the loss");

    // Work whose callbacks are pending when the device is lost.
    copy(&device, &queue, &from, &to);
    let done = Arc::new(AtomicU32::new(0));
    let work_done = Arc::clone(&done);
    queue.on_submitted_work_done(move || {
        work_done.fetch_add(1, Ordering::AcqRel);
    });
    let mapped = Arc::new(AtomicUsize::new(0));
    let map_result = Arc::clone(&mapped);
    to.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        map_result.store(1 + usize::from(result.is_ok()), Ordering::Release);
    });

    if !lose(&device) {
        println!("{test}: skipped, the backend cannot lose this device on demand");
        return;
    }

    let first = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(WAIT),
    });
    assert!(
        matches!(first, Err(wgpu::PollError::DeviceLost)),
        "{test}: a wait on a lost device reports the loss, got {first:?}"
    );
    {
        let lost = lost.lock().unwrap();
        assert_eq!(lost.len(), 1, "{test}: the lost callback runs once");
        assert_eq!(lost[0].0, wgpu::DeviceLostReason::Unknown, "{test}");
    }
    assert_eq!(
        done.load(Ordering::Acquire),
        1,
        "{test}: work submitted before the loss counts as finished"
    );
    assert_ne!(
        mapped.load(Ordering::Acquire),
        0,
        "{test}: a pending map is resolved"
    );

    // Later polls keep reporting the loss, and the callback does not run again.
    for poll in [
        wgpu::PollType::Poll,
        wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::ZERO),
        },
    ] {
        let result = device.poll(poll);
        assert!(
            matches!(result, Err(wgpu::PollError::DeviceLost)),
            "{test}: a later poll reports the loss, got {result:?}"
        );
    }

    // Work on the lost device is rejected quietly, not by a panic.
    copy(&device, &queue, &from, &to);
    let _ = buffer(&device, wgpu::BufferUsages::COPY_DST);
    let result = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(15)),
    });
    assert!(
        matches!(result, Err(wgpu::PollError::DeviceLost)),
        "{test}: got {result:?}"
    );
    assert_eq!(
        lost.lock().unwrap().len(),
        1,
        "{test}: the callback ran once"
    );
}

#[test]
fn vulkan_poll_reports_device_lost() {
    check(
        "vulkan_poll_reports_device_lost",
        wgpu::Backends::VULKAN,
        |device| {
            // SAFETY: the hal device is only used to set the test hook.
            let Some(hal) = (unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }) else {
                return false;
            };
            hal.simulate_device_lost();
            true
        },
    );
}

#[cfg(windows)]
#[test]
fn dx12_poll_reports_removed_device() {
    use windows::Win32::Graphics::Direct3D12::ID3D12Device5;
    use windows::core::Interface as _;

    check(
        "dx12_poll_reports_removed_device",
        wgpu::Backends::DX12,
        |device| {
            // SAFETY: the raw device is only used to remove it, which is what
            // this test is about; wgpu sees the removal through its fences.
            let Some(hal) = (unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }) else {
                return false;
            };
            let Ok(device5) = hal.raw_device().cast::<ID3D12Device5>() else {
                return false;
            };
            unsafe { device5.RemoveDevice() };
            true
        },
    );
}
