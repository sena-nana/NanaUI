//! Scene-host instrumentation (Issue #227). Everything here is free when
//! diagnostics are off: one relaxed load per call site.

use std::time::{Duration, Instant};

use nana_diagnostics::framework::{gpu, host};
use nana_diagnostics::metric;

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
        metric!(gpu::UPLOAD_BYTES, work.gpu_upload_bytes);
        metric!(gpu::DRAW_CALLS, work.draw_calls);
        metric!(gpu::BUFFER_REALLOCATIONS, work.gpu_buffer_reallocations);
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
