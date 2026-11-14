//! Vulkan swapchains retired by `Surface::configure` are freed only once
//! their presents are done (Windows).
//!
//! The fork does not wait for the device to go idle when a surface is
//! reconfigured; it keeps the old swapchain until its presents are known done,
//! by present fence (`VK_KHR/EXT_swapchain_maintenance1`), by present wait
//! (`VK_KHR_present_wait`), or, with neither, by a later submission completing.
//! `NANA_WGPU_VULKAN_PRESENT_TRACKING` limits the choice, so this test runs
//! each path the adapter supports: it resizes a real window while frames are
//! in flight, with validation requested, and fails on any error the Vulkan
//! backend logs (validation messages arrive as `log` errors from `wgpu_hal`).
//!
//! The validation layer (`VK_LAYER_KHRONOS_validation`) is not installed on
//! every machine; without it the test still runs and checks the driver's own
//! messages. `NANA_REQUIRE_VULKAN_VALIDATION=1` makes a missing layer fail.
//!
//! The load is small and bounded: a clear per frame, a fixed number of
//! resizes, a wall-clock cap, and a watchdog that ends the process if a
//! driver hangs.
use crate::support;

use std::{
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use nana_wgpu_fork_tests::no_adapter;
use support::{clear, create_window, pump, resize, surface};
use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::DestroyWindow};

const RESIZES: u32 = 24;
const FRAMES_PER_SIZE: u32 = 3;
const WALL_CLOCK_CAP: Duration = Duration::from_secs(15);
const TRACKING_VARIABLE: &str = "NANA_WGPU_VULKAN_PRESENT_TRACKING";

/// Collects `wgpu_hal` log records, where the Vulkan backend reports
/// validation messages and its present tracking.
struct HalLog(Mutex<Vec<(log::Level, String)>>);

impl log::Log for HalLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("wgpu_hal") && metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.0
                .lock()
                .unwrap()
                .push((record.level(), record.args().to_string()));
        }
    }

    fn flush(&self) {}
}

fn hal_log() -> &'static HalLog {
    static LOG: OnceLock<&'static HalLog> = OnceLock::new();
    LOG.get_or_init(|| {
        let log: &'static HalLog = Box::leak(Box::new(HalLog(Mutex::new(Vec::new()))));
        log::set_logger(log).unwrap();
        log::set_max_level(log::LevelFilter::Debug);
        log
    })
}

fn take_log() -> Vec<(log::Level, String)> {
    std::mem::take(&mut *hal_log().0.lock().unwrap())
}

fn count(log: &[(log::Level, String)], needle: &str) -> usize {
    log.iter()
        .filter(|(_, message)| message.contains(needle))
        .count()
}

/// The outcome of one run: the tracking the device picked, or `None` when
/// there is no Vulkan adapter.
struct Run {
    tracking: String,
    validation_layer: bool,
}

/// Resize `window` repeatedly with frames in flight, on a Vulkan device whose
/// present tracking is limited to `limit`.
fn resize_with_frames_in_flight(window: HWND, limit: &str) -> Option<Run> {
    // SAFETY: the other threads (the watchdog) do not touch the environment.
    unsafe { std::env::set_var(TRACKING_VARIABLE, limit) };
    take_log();

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::DEBUG,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let surface = surface(&instance, window);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .ok()?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    // SAFETY: as above.
    unsafe { std::env::remove_var(TRACKING_VARIABLE) };

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

    let started = Instant::now();
    let mut presented = 0;
    let mut frame = |step: u32| match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture)
        | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
            let view = texture.texture.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            clear(&mut encoder, &view, f64::from(step % 7) / 7.0);
            queue.submit([encoder.finish()]);
            queue.present(texture);
            presented += 1;
        }
        // A desktop that hides the window may withhold images.
        wgpu::CurrentSurfaceTexture::Timeout
        | wgpu::CurrentSurfaceTexture::Occluded
        | wgpu::CurrentSurfaceTexture::Outdated => {}
        other => panic!("{limit}: acquire failed: {other:?}"),
    };
    for step in 0..RESIZES {
        if started.elapsed() > WALL_CLOCK_CAP {
            break;
        }
        // Every other step reconfigures at once, without resizing the window
        // first, so the presents just queued are still pending when their
        // swapchain is retired.
        if step % 2 == 0 {
            let width = 160 + (step % 5) * 24;
            let height = 120 + (step % 3) * 16;
            resize(window, width, height);
            configuration.width = width;
            configuration.height = height;
        }
        surface.configure(&device, &configuration);
        for _ in 0..FRAMES_PER_SIZE {
            frame(step);
        }
    }
    // A few more frames, so the last retired swapchains get collected.
    for _ in 0..FRAMES_PER_SIZE {
        frame(RESIZES);
    }
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(5)),
    });
    drop(surface);
    let elapsed = started.elapsed();
    drop((queue, device, adapter, instance));

    let log = take_log();
    let tracking = log
        .iter()
        .find_map(|(_, message)| message.strip_prefix("Vulkan present tracking: "))
        .unwrap_or("unknown")
        .to_owned();
    let validation_layer = count(&log, "unable to find layer") == 0;
    let by_fence = count(&log, "released: present fences");
    let at_configure = count(&log, "released: presents done at configure");
    let by_submission = count(&log, "released: successor submission");
    println!(
        "limit {limit}: tracking {tracking}, validation layer {validation_layer}, \
         {presented} presented in {:.0} ms; retired swapchains released by present fences \
         {by_fence}, at configure {at_configure}, by successor submission {by_submission}",
        elapsed.as_secs_f64() * 1000.0
    );

    let backend_errors: Vec<_> = log
        .iter()
        .filter(|(level, _)| *level == log::Level::Error)
        .map(|(_, message)| message)
        .collect();
    assert!(
        backend_errors.is_empty(),
        "{limit}: the Vulkan backend reported errors: {backend_errors:#?}"
    );
    let errors = errors.lock().unwrap();
    assert!(errors.is_empty(), "{limit}: wgpu raised: {errors:#?}");

    // Each path must actually free what it retired, the way it claims to.
    if presented > RESIZES {
        match tracking.as_str() {
            "PresentFence" => assert!(
                by_fence + at_configure > 0,
                "{limit}: no retired swapchain was released by its present fences"
            ),
            "PresentWait" => assert!(
                at_configure > 0,
                "{limit}: no swapchain's last present was seen done at configure"
            ),
            "Heuristic" => assert!(
                by_submission > 0,
                "{limit}: no retired swapchain was released after a successor submission"
            ),
            other => panic!("{limit}: unexpected present tracking {other:?}"),
        }
    }
    Some(Run {
        tracking,
        validation_layer,
    })
}

#[test]
fn retired_swapchains_are_freed_after_their_presents() {
    if !nana_wgpu_fork_tests::adapter_required() {
        eprintln!(
            "retired_swapchains_are_freed_after_their_presents: skipped \
             (set NANA_REQUIRE_WGPU_FORK_TESTS=1 to run windowed swapchain stress)"
        );
        return;
    }
    // A hang inside a driver cannot be aborted from the loop.
    std::thread::spawn(|| {
        std::thread::sleep(WALL_CLOCK_CAP * 3 + Duration::from_secs(30));
        eprintln!("vulkan_present_tracking: watchdog fired, the driver hung");
        std::process::exit(2);
    });
    hal_log();
    let window = create_window();
    pump();
    let mut runs = Vec::new();
    for limit in ["fence", "wait", "none"] {
        match resize_with_frames_in_flight(window, limit) {
            Some(run) => runs.push((limit, run)),
            None => break,
        }
    }
    unsafe { DestroyWindow(window).unwrap() };
    if runs.is_empty() {
        no_adapter(
            "retired_swapchains_are_freed_after_their_presents",
            wgpu::Backends::VULKAN,
        );
        return;
    }
    let require_layer =
        std::env::var_os("NANA_REQUIRE_VULKAN_VALIDATION").is_some_and(|value| value == "1");
    for (limit, run) in &runs {
        if !run.validation_layer {
            assert!(
                !require_layer,
                "{limit}: VK_LAYER_KHRONOS_validation is missing and \
                 NANA_REQUIRE_VULKAN_VALIDATION=1"
            );
            println!("{limit}: VK_LAYER_KHRONOS_validation is not installed; driver messages only");
        }
    }
    // `none` always ends up with the heuristic.
    let none = runs.iter().find(|(limit, _)| *limit == "none");
    if let Some((_, run)) = none {
        assert_eq!(run.tracking, "Heuristic");
    }
}
