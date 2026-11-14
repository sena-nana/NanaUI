//! `Surface::configure` while another thread keeps submitting (Windows).
//!
//! The fork waits only for the surface's own last submission (wgpu-core
//! `configure_surface`, the DX12 present fence, Vulkan's retired swapchain)
//! instead of draining the device, so a concurrent submission must neither
//! fail the configure (`GpuWaitTimeout`) nor raise any other error.
//!
//! It needs a window, so it creates a small one. The load is deliberately
//! tiny and bounded: one 64×64 clear in flight at a time, a fixed number of
//! reconfigures, a wall-clock cap, and a watchdog that ends the process if a
//! driver hangs.
use crate::support;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use nana_wgpu_fork_tests::no_adapter;
use support::{clear, create_window, pump, resize, surface};
use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::DestroyWindow};

const RECONFIGURES: u32 = 40;
const WALL_CLOCK_CAP: Duration = Duration::from_secs(20);

/// One small clear at a time until `stop`; returns the number submitted.
fn produce(device: &wgpu::Device, queue: &wgpu::Queue, stop: &AtomicBool) -> u64 {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fork reconfigure producer target"),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut submits = 0;
    let deadline = Instant::now() + WALL_CLOCK_CAP;
    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
        let mut encoder = device.create_command_encoder(&Default::default());
        clear(&mut encoder, &view, (submits % 5) as f64 / 5.0);
        let index = queue.submit([encoder.finish()]);
        submits += 1;
        // Never more than one producer submission in flight.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: Some(Duration::from_secs(2)),
        });
    }
    submits
}

/// Reconfigure a surface on `backend` while a producer thread submits.
/// Returns `false` when there is no adapter for it.
fn reconfigure_under_submission(backend: wgpu::Backends, window: HWND) -> bool {
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

    let errors = Arc::new(Mutex::new(Vec::<String>::new()));
    device.on_uncaptured_error({
        let errors = errors.clone();
        Arc::new(move |error: wgpu::Error| errors.lock().unwrap().push(error.to_string()))
    });

    let capabilities = surface.get_capabilities(&adapter);
    let mut configuration = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: capabilities.formats[0],
        color_space: Default::default(),
        width: 160,
        height: 120,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: capabilities.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &configuration);

    let stop = Arc::new(AtomicBool::new(false));
    let producer = {
        let (device, queue, stop) = (device.clone(), queue.clone(), stop.clone());
        std::thread::spawn(move || produce(&device, &queue, &stop))
    };
    // Let the producer get going before the first reconfigure.
    std::thread::sleep(Duration::from_millis(100));

    let started = Instant::now();
    let mut presented = 0;
    for step in 0..RECONFIGURES {
        if started.elapsed() > WALL_CLOCK_CAP {
            break;
        }
        let width = 160 + (step % 5) * 24;
        let height = 120 + (step % 3) * 16;
        resize(window, width, height);
        configuration.width = width;
        configuration.height = height;
        surface.configure(&device, &configuration);

        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                let view = texture.texture.create_view(&Default::default());
                let mut encoder = device.create_command_encoder(&Default::default());
                clear(&mut encoder, &view, f64::from(step % 7) / 7.0);
                queue.submit([encoder.finish()]);
                queue.present(texture);
                presented += 1;
            }
            // A desktop that hides the window may withhold images; the
            // reconfigures still ran.
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Outdated => {}
            other => panic!("{backend:?}: acquire failed: {other:?}"),
        }
    }
    stop.store(true, Ordering::Relaxed);
    let submits = producer.join().unwrap();
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(5)),
    });
    drop(surface);

    let info = adapter.get_info();
    println!(
        "{:?} {:?}: {RECONFIGURES} reconfigures, {presented} presented, {submits} producer submits, {:.0} ms",
        info.backend,
        info.name,
        started.elapsed().as_secs_f64() * 1000.0
    );
    let errors = errors.lock().unwrap();
    assert!(
        errors.is_empty(),
        "{backend:?}: reconfigure under concurrent submission raised: {errors:#?}"
    );
    assert!(
        submits > 1,
        "{backend:?}: the producer never ran alongside the reconfigures"
    );
    true
}

#[test]
fn reconfigure_does_not_fail_while_another_thread_submits() {
    if !nana_wgpu_fork_tests::adapter_required() {
        eprintln!(
            "reconfigure_does_not_fail_while_another_thread_submits: skipped \
             (set NANA_REQUIRE_WGPU_FORK_TESTS=1 to run windowed swapchain stress)"
        );
        return;
    }
    // A hang inside a driver cannot be aborted from the loop.
    std::thread::spawn(|| {
        std::thread::sleep(WALL_CLOCK_CAP * 2 + Duration::from_secs(20));
        eprintln!("surface_reconfigure: watchdog fired, a backend hung");
        std::process::exit(2);
    });
    let window = create_window();
    pump();
    let mut ran = false;
    for backend in [wgpu::Backends::DX12, wgpu::Backends::VULKAN] {
        if reconfigure_under_submission(backend, window) {
            ran = true;
        } else {
            println!("surface_reconfigure: no {backend:?} adapter for this window");
        }
    }
    unsafe { DestroyWindow(window).unwrap() };
    if !ran {
        no_adapter(
            "reconfigure_does_not_fail_while_another_thread_submits",
            wgpu::Backends::DX12 | wgpu::Backends::VULKAN,
        );
    }
}
