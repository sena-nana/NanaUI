//! `Surface::wait_for_frame_latency` (Windows).
//!
//! On DX12 it takes the swap chain's frame-latency waitable ahead of the
//! acquire, which then must not wait on it a second time: with
//! `desired_maximum_frame_latency: 1` on FIFO, a second wait would block each
//! acquire for about a refresh interval. Repeated calls before the acquire
//! return at once. On Vulkan, and on a surface not yet configured, the call is
//! a no-op that returns `true`.
//!
//! The load is a 64×64-ish clear per frame, one frame at a time, a fixed
//! number of frames, a wall-clock cap and a watchdog.
#![cfg(windows)]

mod support;

use std::time::{Duration, Instant};

use nana_wgpu_fork_tests::no_adapter;
use support::{clear, create_window, pump, surface};
use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::DestroyWindow};

const FRAMES: u32 = 40;
const WALL_CLOCK_CAP: Duration = Duration::from_secs(15);
const WAIT: Option<Duration> = Some(Duration::from_secs(1));

/// Presents `FRAMES` frames, waiting before each acquire. Returns `false`
/// when there is no adapter for `backend`.
fn wait_then_acquire(backend: wgpu::Backends, window: HWND) -> bool {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: backend,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let surface = surface(&instance, window);
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..Default::default()
    })) else {
        return false;
    };
    let Ok((device, queue)) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
    else {
        return false;
    };

    // Nothing to wait on before the surface has a swap chain.
    assert!(surface.wait_for_frame_latency(Some(Duration::ZERO)));

    let capabilities = surface.get_capabilities(&adapter);
    surface.configure(
        &device,
        &wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: capabilities.formats[0],
            color_space: Default::default(),
            width: 96,
            height: 64,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        },
    );

    let started = Instant::now();
    let mut acquires = Vec::new();
    let mut presented = 0;
    for frame in 0..FRAMES {
        if started.elapsed() > WALL_CLOCK_CAP {
            break;
        }
        assert!(
            surface.wait_for_frame_latency(WAIT),
            "{backend:?}: the frame-latency wait timed out after {WAIT:?}"
        );
        // Already waited: the second call does not take another count.
        let again = Instant::now();
        assert!(surface.wait_for_frame_latency(Some(Duration::ZERO)));
        assert!(again.elapsed() < Duration::from_millis(50));

        let acquire = Instant::now();
        let texture = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            // A desktop that hides the window may withhold images.
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                continue;
            }
            other => panic!("{backend:?}: acquire failed: {other:?}"),
        };
        acquires.push(acquire.elapsed());
        let view = texture.texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        clear(&mut encoder, &view, f64::from(frame % 7) / 7.0);
        let index = queue.submit([encoder.finish()]);
        queue.present(texture);
        presented += 1;
        // One frame in flight.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: Some(Duration::from_secs(2)),
        });
    }
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(5)),
    });
    drop(surface);

    acquires.sort_unstable();
    let median = acquires.get(acquires.len() / 2).copied();
    let info = adapter.get_info();
    println!(
        "{:?} {:?}: {presented} frames presented, acquire median {median:?}, {:.0} ms",
        info.backend,
        info.name,
        started.elapsed().as_secs_f64() * 1000.0
    );
    assert!(presented > 0, "{backend:?}: no frame was presented");
    // The acquire found its count already taken. A second wait would block
    // most acquires for about a refresh interval on this FIFO swap chain.
    // Vulkan paces inside its acquire; the call is a no-op there.
    if let Some(median) = median.filter(|_| backend == wgpu::Backends::DX12) {
        assert!(
            median < Duration::from_millis(4),
            "{backend:?}: acquire still waits after wait_for_frame_latency (median {median:?})"
        );
    }
    true
}

#[test]
fn acquire_after_a_frame_latency_wait_does_not_wait_again() {
    // A hang inside a driver cannot be aborted from the loop.
    std::thread::spawn(|| {
        std::thread::sleep(WALL_CLOCK_CAP * 2 + Duration::from_secs(20));
        eprintln!("frame_latency_wait: watchdog fired, a backend hung");
        std::process::exit(2);
    });
    let window = create_window();
    pump();
    let mut ran = false;
    for backend in [wgpu::Backends::DX12, wgpu::Backends::VULKAN] {
        if wait_then_acquire(backend, window) {
            ran = true;
        } else {
            println!("frame_latency_wait: no {backend:?} adapter for this window");
        }
    }
    unsafe { DestroyWindow(window).unwrap() };
    if !ran {
        no_adapter(
            "acquire_after_a_frame_latency_wait_does_not_wait_again",
            wgpu::Backends::DX12 | wgpu::Backends::VULKAN,
        );
    }
}
