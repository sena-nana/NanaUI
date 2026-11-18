//! Optional numeric output observations. Querying a snapshot never allocates.
//! High-frequency observations use metrics; only an explicit diagnostic
//! capture publishes correlated events. Capture is not a frame-loop API.

use std::sync::atomic::{AtomicU64, Ordering};

use nana_gpu::{DeviceGeneration, GpuSubmission, GpuTextureFormat};
pub use nana_ui_core::OutputWorkObservation;

use super::{OutputConsumerId, OutputPath};

/// Host-observed GPU and output presentation counts for one frame.
/// `None` means the host path did not record that stage for the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresentationWork {
    pub batch_rebuilds: Option<usize>,
    pub draw_batches: Option<usize>,
    pub draw_calls: Option<usize>,
    pub gpu_upload_bytes: Option<usize>,
    pub gpu_buffer_reallocations: Option<usize>,
    pub output_extra_passes: Option<usize>,
    pub output_gpu_copies: Option<usize>,
    pub output_target_recreates: Option<usize>,
    pub output_content_revisions: Option<usize>,
    pub output_idle_reuse_frames: Option<usize>,
    pub output_resolve_count: Option<usize>,
    pub output_gpu_copy_bytes: Option<usize>,
    pub output_gpu_convert_passes: Option<usize>,
    pub output_plan_rebuilds: Option<usize>,
    pub output_ui_renders: Option<usize>,
    pub output_submissions: Option<usize>,
    pub output_cpu_readbacks: Option<usize>,
    pub output_deferred: Option<usize>,
}

impl PresentationWork {
    pub fn record_gpu_work(&mut self, work: crate::GpuWorkObservation) {
        fn add(slot: &mut Option<usize>, delta: usize) {
            let value = slot.get_or_insert(0);
            *value = value.saturating_add(delta);
        }
        add(&mut self.batch_rebuilds, work.batch_rebuilds);
        add(&mut self.draw_batches, work.draw_batches);
        add(&mut self.draw_calls, work.draw_calls);
        add(&mut self.gpu_upload_bytes, work.gpu_upload_bytes);
        add(
            &mut self.gpu_buffer_reallocations,
            work.gpu_buffer_reallocations,
        );
    }

    pub fn record_output_work(&mut self, work: OutputWorkObservation) {
        fn add(slot: &mut Option<usize>, delta: usize) {
            let value = slot.get_or_insert(0);
            *value = value.saturating_add(delta);
        }
        add(&mut self.output_extra_passes, work.extra_passes);
        add(&mut self.output_gpu_copies, work.gpu_copies);
        add(&mut self.output_target_recreates, work.target_recreates);
        add(&mut self.output_content_revisions, work.content_revisions);
        add(&mut self.output_idle_reuse_frames, work.idle_reuse_frames);
        add(&mut self.output_resolve_count, work.resolve_count);
        add(&mut self.output_gpu_copy_bytes, work.gpu_copy_bytes);
        add(&mut self.output_gpu_convert_passes, work.gpu_convert_passes);
        add(&mut self.output_plan_rebuilds, work.plan_rebuilds);
        add(&mut self.output_ui_renders, work.ui_renders);
        add(&mut self.output_submissions, work.submissions);
        add(&mut self.output_cpu_readbacks, work.cpu_readbacks);
        add(&mut self.output_deferred, work.deferred);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum OutputTraceRole {
    Producer = 1,
    Consumer = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u64)]
pub enum OutputTraceState {
    #[default]
    Unobserved = 0,
    Recorded = 1,
    Submitted = 2,
    Reused = 3,
    Deferred = 4,
    Failed = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u64)]
pub enum OutputTraceDeferral {
    #[default]
    None = 0,
    SourceSlots = 1,
    FrameSlots = 2,
    ConsumerSlots = 3,
    NativeRelease = 4,
    NativeReservation = 5,
    SurfaceAcquire = 6,
}

/// The reason for the latest allocation observed in this trace scope.
/// Enabling tracing over already allocated resources does not invent an
/// allocation. A supplied destination's allocation belongs to its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u64)]
pub enum OutputRecreateReason {
    #[default]
    None = 0,
    Created = 1,
    Extent = 2,
    Format = 3,
    Device = 4,
    Requirements = 5,
}

/// Actual acquisition of the submitted frame. `try_begin_frame` succeeds
/// without waiting; legacy `begin_frame` may have waited for that frame's
/// slot. `None` means unobserved, never zero or foreign GPU retirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutputTraceWait {
    pub frame_slot_waits: u64,
    pub frame_slot_stalls: u64,
}

impl OutputTraceWait {
    pub fn from_submission(submission: &GpuSubmission) -> Self {
        let acquisition = submission.acquisition();
        Self {
            frame_slot_waits: acquisition.frame_slot_waits,
            frame_slot_stalls: acquisition.frame_slot_stalls,
        }
    }
}

/// Work accumulated since tracing was enabled for this owner/topology.
/// Versions describe the latest encoded or reused version, not a promise
/// of GPU completion. Consumer work excludes the source producer's work.
/// A zero producer/version means no version has yet been observed in this
/// trace scope; it is not a published frame or resource identity.
/// Wait observations describe only the latest actor call; SDK internals,
/// foreign queues and caller-owned frame acquisition remain unobserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputTraceSnapshot {
    pub producer: u64,
    pub consumer: OutputConsumerId,
    pub role: OutputTraceRole,
    pub path: OutputPath,
    pub format: GpuTextureFormat,
    pub extent: [u32; 2],
    pub device_generation: DeviceGeneration,
    pub resource_generation: u64,
    pub content_revision: u64,
    pub work: OutputWorkObservation,
    pub waits: Option<OutputTraceWait>,
    pub recreate_reason: OutputRecreateReason,
    pub state: OutputTraceState,
    pub deferral_reason: OutputTraceDeferral,
}

impl OutputTraceSnapshot {
    pub fn new(
        producer: u64,
        consumer: OutputConsumerId,
        role: OutputTraceRole,
        path: OutputPath,
        format: GpuTextureFormat,
        extent: [u32; 2],
        device_generation: DeviceGeneration,
    ) -> Self {
        Self {
            producer,
            consumer,
            role,
            path,
            format,
            extent,
            device_generation,
            resource_generation: 0,
            content_revision: 0,
            work: OutputWorkObservation::default(),
            waits: None,
            recreate_reason: OutputRecreateReason::None,
            state: OutputTraceState::Unobserved,
            deferral_reason: OutputTraceDeferral::None,
        }
    }

    /// Record actual work from this owner only. Hosts may use this for native
    /// transports NanaUI cannot observe. Do not populate unknown SDK work.
    pub fn observe(&mut self, work: OutputWorkObservation, waits: Option<OutputTraceWait>) {
        self.observe_without_metrics(work, waits);
        if self.role == OutputTraceRole::Consumer {
            consumer_metrics(work, waits);
        }
    }

    pub(crate) fn observe_without_metrics(
        &mut self,
        work: OutputWorkObservation,
        waits: Option<OutputTraceWait>,
    ) {
        self.work.accumulate(work);
        self.waits = waits;
        self.state = if work.deferred != 0 {
            OutputTraceState::Deferred
        } else if work.submissions != 0 {
            OutputTraceState::Submitted
        } else if work.ui_renders != 0 || work.extra_passes != 0 || work.gpu_copies != 0 {
            OutputTraceState::Recorded
        } else if work.idle_reuse_frames != 0 {
            OutputTraceState::Reused
        } else {
            self.state
        };
        if self.state != OutputTraceState::Deferred {
            self.deferral_reason = OutputTraceDeferral::None;
        }
    }

    /// Emit a bounded Debug-level diagnostic snapshot on an explicit request.
    /// Do not call from a frame/poll loop. Ordinary per-frame queries should
    /// copy this struct; high-frequency work already uses static metrics.
    pub fn capture_diagnostics(&self) {
        if !nana_diagnostics::enabled(nana_diagnostics::Severity::Debug) {
            return;
        }
        static NEXT_CAPTURE: AtomicU64 = AtomicU64::new(1);
        let trace = NEXT_CAPTURE.fetch_add(1, Ordering::Relaxed);
        use nana_diagnostics::framework::gpu as d;
        nana_diagnostics::event!(
            d::OUTPUT_TRACE_ROUTE,
            trace = trace,
            producer = self.producer,
            consumer = self.consumer.0,
            path = path_code(self.path)
        );
        nana_diagnostics::event!(
            d::OUTPUT_TRACE_TARGET,
            trace = trace,
            format = format_code(self.format),
            width = self.extent[0] as u64,
            height = self.extent[1] as u64
        );
        nana_diagnostics::event!(
            d::OUTPUT_TRACE_VERSION,
            trace = trace,
            device = self.device_generation.get(),
            resource = self.resource_generation,
            revision = self.content_revision
        );
        nana_diagnostics::event!(
            d::OUTPUT_TRACE_WORK,
            trace = trace,
            passes = self.work.extra_passes as u64,
            copies = self.work.gpu_copies as u64,
            copy_bytes = self.work.gpu_copy_bytes as u64
        );
        let waits = self.waits.unwrap_or_default();
        nana_diagnostics::event!(
            d::OUTPUT_TRACE_WAIT,
            trace = trace,
            observed = u64::from(self.waits.is_some()),
            waits = waits.frame_slot_waits,
            stalls = waits.frame_slot_stalls
        );
        nana_diagnostics::event!(
            d::OUTPUT_TRACE_RECREATE,
            trace = trace,
            count = self.work.target_recreates as u64,
            reason = self.recreate_reason as u64,
            role = self.role as u64
        );
        nana_diagnostics::event!(
            d::OUTPUT_TRACE_TRANSPORT,
            trace = trace,
            submissions = self.work.submissions as u64,
            readbacks = self.work.cpu_readbacks as u64,
            deferred = self.work.deferred as u64
        );
        nana_diagnostics::event!(
            d::OUTPUT_TRACE_PRODUCER,
            trace = trace,
            renders = self.work.ui_renders as u64,
            resolves = self.work.resolve_count as u64,
            reuses = self.work.idle_reuse_frames as u64
        );
        nana_diagnostics::event!(
            d::OUTPUT_TRACE_STATE,
            trace = trace,
            state = self.state as u64,
            reason = self.deferral_reason as u64,
            role = self.role as u64
        );
    }
}

pub(crate) fn consumer_metrics(work: OutputWorkObservation, waits: Option<OutputTraceWait>) {
    use nana_diagnostics::framework::gpu as d;
    macro_rules! count {
        ($descriptor:ident, $value:expr) => {
            if $value != 0 {
                nana_diagnostics::metric!(d::$descriptor, $value as u64);
            }
        };
    }
    count!(OUTPUT_CONSUMER_PASSES, work.extra_passes);
    count!(OUTPUT_CONSUMER_COPIES, work.gpu_copies);
    count!(OUTPUT_CONSUMER_COPY_BYTES, work.gpu_copy_bytes);
    count!(OUTPUT_CONSUMER_READBACKS, work.cpu_readbacks);
    count!(OUTPUT_CONSUMER_DEFERRED, work.deferred);
    count!(OUTPUT_CONSUMER_SUBMISSIONS, work.submissions);
    count!(OUTPUT_CONSUMER_REUSES, work.idle_reuse_frames);
    if let Some(waits) = waits {
        count!(OUTPUT_CONSUMER_WAITS, waits.frame_slot_waits);
        count!(OUTPUT_CONSUMER_STALLS, waits.frame_slot_stalls);
    }
}

// Persisted diagnostic codes: append, never change an existing assignment.
fn path_code(path: OutputPath) -> u64 {
    match path {
        OutputPath::Unsupported(_) => 0,
        OutputPath::DirectSurface => 1,
        OutputPath::SharedTexture => 2,
        OutputPath::NativeShared => 3,
        OutputPath::GpuTransfer => 4,
    }
}

fn format_code(format: GpuTextureFormat) -> u64 {
    [
        GpuTextureFormat::RGBA8_UNORM,
        GpuTextureFormat::RGBA8_UNORM_SRGB,
        GpuTextureFormat::BGRA8_UNORM,
        GpuTextureFormat::BGRA8_UNORM_SRGB,
        GpuTextureFormat::RGBA16_FLOAT,
        GpuTextureFormat::RGB10A2_UNORM,
        GpuTextureFormat::R8_UNORM,
    ]
    .iter()
    .position(|candidate| *candidate == format)
    .map_or(0, |index| index as u64 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_waits_stay_unknown_and_codes_are_stable() {
        assert_eq!(path_code(OutputPath::DirectSurface), 1);
        assert_eq!(path_code(OutputPath::GpuTransfer), 4);
        assert_eq!(format_code(GpuTextureFormat::BGRA8_UNORM), 3);
        let mut snapshot = OutputTraceSnapshot::new(
            1,
            OutputConsumerId(2),
            OutputTraceRole::Consumer,
            OutputPath::GpuTransfer,
            GpuTextureFormat::BGRA8_UNORM,
            [1, 1],
            crate::test_gpu::context().generation(),
        );
        snapshot.observe(
            OutputWorkObservation {
                extra_passes: 1,
                ..Default::default()
            },
            None,
        );
        assert_eq!(snapshot.waits, None);
        assert_eq!(snapshot.work.ui_renders, 0);
        assert_eq!(snapshot.work.extra_passes, 1);
    }
}
