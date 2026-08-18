//! Observed GPU work for Issue #8 counters.
//!
//! Values are recorded only on a real encode/submit path. CPU-only Runtime
//! drains never construct this sink, so WorkCounters GPU fields stay `None`.

use std::cell::RefCell;
use std::time::Duration;

use nana_ui_core::{FrameStage, GpuWorkObservation};
use nana_ui_runtime::FrameProfiler;

/// Shared accumulator for one Scene/WGPU frame.
#[derive(Debug, Default)]
pub struct GpuWorkSink {
    work: RefCell<GpuWorkObservation>,
}

impl GpuWorkSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_upload(&self, bytes: usize) {
        self.work.borrow_mut().record_upload(bytes);
    }

    pub fn record_realloc(&self) {
        self.work.borrow_mut().record_realloc();
    }

    pub fn record_batch_rebuild(&self) {
        self.work.borrow_mut().record_batch_rebuild();
    }

    pub fn record_draw_batch(&self) {
        self.work.borrow_mut().record_draw_batch();
    }

    pub fn record_draw_call(&self) {
        self.work.borrow_mut().record_draw_call();
    }

    pub fn snapshot(&self) -> GpuWorkObservation {
        *self.work.borrow()
    }
}

/// Stage timings a GPU host measured while encoding/submitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuStageTimings {
    pub batch: Duration,
    pub gpu_upload: Duration,
    pub encode: Duration,
    pub submit: Duration,
}

impl GpuStageTimings {
    /// Fold host-measured GPU stages onto a FrameProfiler.
    ///
    /// Runtime-only hosts must call [`FrameProfiler::mark_runtime_unsupported`]
    /// instead and never invent these durations.
    pub fn record_on(self, profiler: &mut FrameProfiler) {
        profiler.record(FrameStage::Batch, self.batch);
        profiler.record(FrameStage::GpuUpload, self.gpu_upload);
        profiler.record(FrameStage::Encode, self.encode);
        profiler.record(FrameStage::Submit, self.submit);
    }
}
