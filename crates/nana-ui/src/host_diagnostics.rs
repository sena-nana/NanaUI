//! Scene-host instrumentation (Issue #227). Everything here is free when
//! diagnostics are off: one relaxed load per call site.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use nana_diagnostics::framework::{gpu, host};
use nana_diagnostics::metric;

use crate::ResolvedSurfaceProfile;

use crate::GpuWorkObservation;

/// Describe the adapter the host is rendering with. Called at startup and
/// after every device switch, so the log always names the live GPU.
pub(crate) fn record_adapter(info: &wgpu::AdapterInfo) {
    let Some(diagnostics) = nana_diagnostics::global() else {
        return;
    };
    diagnostics.set_session_info("gpu.name", info.name.clone());
    diagnostics.set_session_info("gpu.backend", format!("{:?}", info.backend));
    diagnostics.set_session_info("gpu.device_type", format!("{:?}", info.device_type));
    // Metal reports neither a driver string nor PCI ids.
    let driver = format!("{} {}", info.driver, info.driver_info);
    if !driver.trim().is_empty() {
        diagnostics.set_session_info("gpu.driver", driver.trim());
    }
    if info.vendor != 0 || info.device != 0 {
        diagnostics.set_session_info(
            "gpu.vendor_device",
            format!("{:04x}:{:04x}", info.vendor, info.device),
        );
    }
}

/// Record the active surface presentation and the latest advisory HDR data.
/// This is change-gated by the surface cache: startup/rebind and an event that
/// changes metadata call it, while steady frames do not.
pub(crate) fn record_surface_hdr(profile: ResolvedSurfaceProfile, info: &wgpu::DisplayHdrInfo) {
    let Some(diagnostics) = nana_diagnostics::global() else {
        return;
    };
    diagnostics.set_session_info(
        "gpu.presentation.requested",
        format!("{:?}", profile.requested),
    );
    diagnostics.set_session_info("gpu.presentation.format", format!("{:?}", profile.format));
    diagnostics.set_session_info(
        "gpu.presentation.color_space",
        format!("{:?}", profile.color_space),
    );
    // Keep the transfer function explicit: a colour-space debug name is useful
    // to humans, while this stable key lets telemetry distinguish the active
    // PQ/HLG signal from an extended-linear or SDR fallback.
    let encoding = match profile.color_space {
        wgpu::SurfaceColorSpace::Bt2100Pq => "pq",
        wgpu::SurfaceColorSpace::Bt2100Hlg => "hlg",
        wgpu::SurfaceColorSpace::ExtendedSrgbLinear => "extended-linear",
        wgpu::SurfaceColorSpace::ExtendedSrgb => "extended-srgb",
        wgpu::SurfaceColorSpace::ExtendedDisplayP3 => "extended-p3",
        wgpu::SurfaceColorSpace::DisplayP3 => "srgb-p3",
        wgpu::SurfaceColorSpace::Srgb => "srgb",
        _ => "unknown",
    };
    diagnostics.set_session_info("gpu.presentation.encoding", encoding);
    diagnostics.set_session_info(
        "gpu.presentation.fallback",
        profile
            .fallback
            .map_or_else(|| "none".to_owned(), |fallback| format!("{fallback:?}")),
    );
    diagnostics.set_session_info(
        "gpu.presentation.headroom",
        info.tone_map_headroom()
            .filter(|value| value.is_finite() && *value >= 1.0)
            .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
    );
    diagnostics.set_session_info(
        "gpu.presentation.sdr_white_nits",
        info.luminance
            .and_then(|luminance| luminance.sdr_white_nits)
            .filter(|value| value.is_finite() && *value > 0.0)
            .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
    );
}

/// One presented frame.
pub(crate) fn frame_presented(
    started: Option<Instant>,
    submit: Duration,
    work: Option<GpuWorkObservation>,
) {
    let Some(started) = started else {
        return;
    };
    metric!(host::REDRAW_NS, started.elapsed());
    metric!(gpu::FRAMES_PRESENTED);
    metric!(gpu::SUBMIT_NS, submit);
    if let Some(work) = work {
        metric!(gpu::DRAW_CALLS, work.draw_calls);
    }
}

pub(crate) fn window_opened(
    id: nana_ui_platform::WindowId,
    geometry: &nana_ui_platform::WindowGeometry,
) {
    nana_diagnostics::event!(
        nana_diagnostics::framework::window::OPENED,
        window = id.0,
        width = geometry.physical_size.0,
        height = geometry.physical_size.1
    );
}

/// Longest gap between two completion polls for which a `gpu.completion`
/// sample is still trusted. Beyond it the host was idle, and the sample
/// would measure the idle time rather than the GPU.
const COMPLETION_POLL_GAP: Duration = Duration::from_millis(50);

/// One submission is watched at a time: the histogram is sampled, not fed
/// every frame, so most frames pay neither the callback nor the poll.
static WATCHING: AtomicBool = AtomicBool::new(false);
static EPOCH: OnceLock<Instant> = OnceLock::new();
/// Start of the latest and the previous completion poll, in ns since EPOCH.
static POLL_NS: AtomicU64 = AtomicU64::new(0);
static PREVIOUS_POLL_NS: AtomicU64 = AtomicU64::new(0);

fn epoch_ns() -> u64 {
    u64::try_from(EPOCH.get_or_init(Instant::now).elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Non-blocking poll that delivers the watched submission's completion
/// callback. Called at redraw start while metrics are on; free when nothing
/// is watched.
pub(crate) fn poll_completions(device: &wgpu::Device) {
    if !WATCHING.load(Ordering::Relaxed) {
        return;
    }
    let now = epoch_ns();
    PREVIOUS_POLL_NS.store(POLL_NS.swap(now, Ordering::Relaxed), Ordering::Relaxed);
    let _ = device.poll(wgpu::PollType::Poll);
}

/// Time the submission just made until the host observes it complete,
/// unless another one is already being watched. Dropped when the observing
/// poll came long after the previous one: the host was idle and the sample
/// would be idle time, not GPU time.
pub(crate) fn watch_submission(queue: &wgpu::Queue) {
    if WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    let submitted = Instant::now();
    POLL_NS.store(epoch_ns(), Ordering::Relaxed);
    queue.on_submitted_work_done(move || {
        let gap = POLL_NS
            .load(Ordering::Relaxed)
            .saturating_sub(PREVIOUS_POLL_NS.load(Ordering::Relaxed));
        if Duration::from_nanos(gap) <= COMPLETION_POLL_GAP {
            metric!(gpu::COMPLETION_NS, submitted.elapsed());
        }
        WATCHING.store(false, Ordering::Relaxed);
    });
}
