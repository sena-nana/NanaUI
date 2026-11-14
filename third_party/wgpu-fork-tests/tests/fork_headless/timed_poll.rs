//! A timed `Device::poll` racing other threads' polls, on DX12 and Vulkan.
//!
//! A wait that times out reads the fence, then triages the queue. If the GPU
//! finishes and another thread's poll triages everything in between, the
//! queue is empty while the fence value this poll read is still below its
//! wait target. Upstream asserts that this cannot happen and panics; the
//! fork reports the timeout.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use nana_wgpu_fork_tests::{device, no_adapter};

/// How long the threads race. Each submission is a small buffer copy.
const RACE: Duration = Duration::from_secs(3);
const POLLERS: usize = 4;
const SUBMITTERS: usize = 2;

fn copy(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    from: &wgpu::Buffer,
    to: &wgpu::Buffer,
) -> wgpu::SubmissionIndex {
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(from, 0, to, 0, from.size());
    queue.submit([encoder.finish()])
}

fn buffer(device: &wgpu::Device, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("timed poll test"),
        size: 1 << 20,
        usage,
        mapped_at_creation: false,
    })
}

fn race(test: &str, backend: wgpu::Backends) {
    let Some((_, device, queue)) = device(backend, wgpu::Features::empty()) else {
        no_adapter(test, backend);
        return;
    };
    let from = Arc::new(buffer(&device, wgpu::BufferUsages::COPY_SRC));
    let stop = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + RACE;
    let mut threads = Vec::new();
    for _ in 0..SUBMITTERS {
        let (device, queue, from, stop) =
            (device.clone(), queue.clone(), from.clone(), stop.clone());
        threads.push(thread::spawn(move || {
            let to = buffer(&device, wgpu::BufferUsages::COPY_DST);
            while !stop.load(Ordering::Relaxed) {
                copy(&device, &queue, &from, &to);
                let _ = device.poll(wgpu::PollType::Poll);
            }
        }));
    }
    for _ in 0..POLLERS {
        let (device, queue, from, stop) =
            (device.clone(), queue.clone(), from.clone(), stop.clone());
        threads.push(thread::spawn(move || {
            let to = buffer(&device, wgpu::BufferUsages::COPY_DST);
            while !stop.load(Ordering::Relaxed) {
                // A wait that almost always times out, for this thread's own
                // submission and for the newest one.
                let index = copy(&device, &queue, &from, &to);
                let _ = device.poll(wgpu::PollType::Wait {
                    submission_index: Some(index),
                    timeout: Some(Duration::ZERO),
                });
                let _ = device.poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(Duration::ZERO),
                });
            }
        }));
    }
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    stop.store(true, Ordering::Relaxed);
    let panicked = threads
        .into_iter()
        .map(thread::JoinHandle::join)
        .filter(Result::is_err)
        .count();
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(15)),
        })
        .expect("the device drains");
    assert_eq!(
        panicked, 0,
        "{test}: {panicked} threads panicked in Device::poll"
    );
}

#[test]
fn timed_waits_racing_other_polls_report_timeouts_on_dx12() {
    race("dx12", wgpu::Backends::DX12);
}

#[test]
fn timed_waits_racing_other_polls_report_timeouts_on_vulkan() {
    race("vulkan", wgpu::Backends::VULKAN);
}
