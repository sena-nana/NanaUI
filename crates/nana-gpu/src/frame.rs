use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::policy::TransientResourceKey;
use crate::{DeviceGeneration, FrameSlotId, GpuContext};

/// Identity of one [`FrameContext`], increasing per [`GpuContext`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FrameId(NonZeroU64);

impl FrameId {
    pub(crate) fn new(value: u64) -> Self {
        Self(NonZeroU64::new(value).expect("frame id counter overflowed"))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Debug for FrameId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "FrameId({})", self.0)
    }
}

/// One frame's command recording. It owns the encoder from
/// [`GpuContext::begin_frame`] until [`Self::submit`]; dropping it instead
/// is the discard.
///
/// Discarding is safe for painters: every owner that recorded
/// [`RetainedWrites`] into this frame gets those keys back as rolled back, so
/// state that assumed the recorded GPU writes landed is rebuilt before it is
/// used again.
pub struct FrameContext {
    gpu: GpuContext,
    id: FrameId,
    encoder: Option<wgpu::CommandEncoder>,
    retained: Vec<(RetainedWrites, u64)>,
    frame_slot: Option<FrameSlotId>,
    transient_registry: Arc<Mutex<Vec<(TransientResourceKey, wgpu::Buffer)>>>,
}

impl fmt::Debug for FrameContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrameContext")
            .field("id", &self.id)
            .field("generation", &self.gpu.generation())
            .field("retained", &self.retained.len())
            .finish()
    }
}

impl FrameContext {
    pub(crate) fn new(
        gpu: GpuContext,
        id: FrameId,
        encoder: wgpu::CommandEncoder,
        frame_slot: Option<FrameSlotId>,
    ) -> Self {
        Self {
            gpu,
            id,
            encoder: Some(encoder),
            retained: Vec::new(),
            frame_slot,
            transient_registry: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn id(&self) -> FrameId {
        self.id
    }

    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }

    pub fn generation(&self) -> DeviceGeneration {
        self.gpu.generation()
    }

    pub(crate) fn transient_registry(
        &self,
    ) -> Arc<Mutex<Vec<(TransientResourceKey, wgpu::Buffer)>>> {
        self.transient_registry.clone()
    }

    /// Record that `owner` wrote state for `key` into this frame and treats it
    /// as done from now on. Submitting settles the record; dropping the frame
    /// rolls it back into `owner`.
    pub fn record_retained_writes(&mut self, owner: &RetainedWrites, key: u64) {
        if self
            .retained
            .iter()
            .any(|(recorded, recorded_key)| recorded.ptr_eq(owner) && *recorded_key == key)
        {
            return;
        }
        owner.record(key, self.id);
        self.retained.push((owner.clone(), key));
    }

    /// Finish the encoder and submit it. Holds the submission guard only for
    /// the submit itself.
    pub fn submit(mut self) -> GpuSubmission {
        let encoder = self.encoder.take().expect("frame encoder");
        let started = Instant::now();
        let commands = encoder.finish();
        let index = {
            let _submission = self.gpu.lock_submission();
            // The reconfiguration guard is shared between submitters. A
            // separate mutex makes token order match actual queue order.
            let _order = self
                .gpu
                .inner
                .submission_order
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let index = self.gpu.inner.queue.submit([commands]);
            let submission = self
                .gpu
                .inner
                .next_submission
                .fetch_add(1, Ordering::Relaxed);
            self.gpu.policy().bind_pipeline_retirement(submission);
            (index, submission)
        };
        let cpu_duration = started.elapsed();
        for (owner, key) in self.retained.drain(..) {
            owner.settle(key, self.id);
        }
        let completed = index.1;
        if let Some(slot) = self.frame_slot.take() {
            let policy = self.gpu.policy().clone();
            let frame_token = self.id.get();
            let queue = self.gpu.inner.queue.clone();
            let transient_buffers = self
                .transient_registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain(..)
                .collect::<Vec<_>>();
            queue.on_submitted_work_done(move || {
                policy.release_frame_slot_exact(slot, frame_token);
                for (key, buffer) in transient_buffers {
                    policy.release_transient_buffer(key, buffer);
                }
                policy.collect_retired(completed);
            });
        } else {
            // A frame without a slot is still a valid submission in a host
            // that uses the compatibility path. Keep the same completion
            // guarantee for pooled buffers.
            let policy = self.gpu.policy().clone();
            let transient_buffers = self
                .transient_registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain(..)
                .collect::<Vec<_>>();
            if !transient_buffers.is_empty() {
                let queue = self.gpu.inner.queue.clone();
                queue.on_submitted_work_done(move || {
                    for (key, buffer) in transient_buffers {
                        policy.release_transient_buffer(key, buffer);
                    }
                    policy.collect_retired(completed);
                });
            }
        }
        GpuSubmission {
            frame: self.id,
            generation: self.gpu.generation(),
            cpu_duration,
            index: index.0,
            submission: index.1,
        }
    }

    /// Drop the recording without submitting. Same as dropping the frame.
    pub fn discard(self) {}

    pub(crate) fn encoder_mut(&mut self) -> &mut wgpu::CommandEncoder {
        self.encoder
            .as_mut()
            .expect("frame encoder is present until submit")
    }
}

impl Drop for FrameContext {
    fn drop(&mut self) {
        // The encoder goes first: nothing may still reference resources the
        // owners are about to rebuild. Without one, either `submit` finished
        // (and settled every record) or it unwound part-way, in which case
        // the records are rolled back too: nothing says the work was queued.
        if let Some(encoder) = self.encoder.take() {
            drop(encoder);
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::FRAMES_DISCARDED);
        }
        if let Some(slot) = self.frame_slot.take() {
            self.gpu
                .policy()
                .release_frame_slot_exact(slot, self.id.get());
        }
        for (key, buffer) in self
            .transient_registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            self.gpu.policy().release_transient_buffer(key, buffer);
        }
        for (owner, key) in self.retained.drain(..) {
            owner.roll_back(key, self.id);
        }
    }
}

/// A submitted frame.
pub struct GpuSubmission {
    frame: FrameId,
    generation: DeviceGeneration,
    cpu_duration: Duration,
    pub(crate) index: wgpu::SubmissionIndex,
    submission: u64,
}

impl fmt::Debug for GpuSubmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuSubmission")
            .field("frame", &self.frame)
            .field("generation", &self.generation)
            .field("cpu_duration", &self.cpu_duration)
            .field("submission", &self.submission)
            .finish()
    }
}

impl GpuSubmission {
    pub fn frame(&self) -> FrameId {
        self.frame
    }

    pub fn generation(&self) -> DeviceGeneration {
        self.generation
    }

    /// CPU time spent finishing and submitting the encoder.
    pub fn cpu_duration(&self) -> Duration {
        self.cpu_duration
    }

    /// Monotonic token assigned under the queue submission lock. It is valid
    /// for retirement bookkeeping and is independent of frame creation order.
    pub fn submission(&self) -> u64 {
        self.submission
    }
}

#[derive(Default)]
struct RetainedState {
    /// Keys recorded into a frame that is neither submitted nor dropped yet.
    pending: Vec<(u64, FrameId)>,
    rolled_back: Vec<u64>,
}

#[derive(Default)]
struct RetainedInner {
    rolled_back: AtomicBool,
    reported: AtomicBool,
    state: Mutex<RetainedState>,
}

/// Ledger of state an owner (a painter) keeps on the assumption that writes
/// it recorded into a [`FrameContext`] reach the GPU. A discarded frame hands
/// its keys back here; the owner drains them and rebuilds that state.
#[derive(Clone, Default)]
pub struct RetainedWrites {
    inner: Arc<RetainedInner>,
}

impl fmt::Debug for RetainedWrites {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetainedWrites")
            .field("rolled_back", &self.has_rolled_back())
            .finish_non_exhaustive()
    }
}

impl RetainedWrites {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether any key was rolled back since the last drain. One atomic load.
    pub fn has_rolled_back(&self) -> bool {
        self.inner.rolled_back.load(Ordering::Acquire)
    }

    /// Hand every rolled-back key to `rebuild`, once each.
    pub fn drain_rolled_back(&self, mut rebuild: impl FnMut(u64)) {
        let keys = {
            let mut state = self.lock();
            self.inner.rolled_back.store(false, Ordering::Release);
            std::mem::take(&mut state.rolled_back)
        };
        for key in keys {
            rebuild(key);
        }
    }

    /// Whether a frame other than `frame` recorded `key` and is still
    /// unsubmitted. Recording on top of it would assume writes that frame may
    /// still discard, or submit after this one.
    pub fn in_flight(&self, key: u64, frame: FrameId) -> bool {
        self.lock()
            .pending
            .iter()
            .any(|&(pending, owner)| pending == key && owner != frame)
    }

    fn ptr_eq(&self, other: &RetainedWrites) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    fn lock(&self) -> MutexGuard<'_, RetainedState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn record(&self, key: u64, frame: FrameId) {
        self.lock().pending.push((key, frame));
    }

    fn settle(&self, key: u64, frame: FrameId) {
        self.lock()
            .pending
            .retain(|&(pending, owner)| !(pending == key && owner == frame));
    }

    fn roll_back(&self, key: u64, frame: FrameId) {
        {
            let mut state = self.lock();
            state
                .pending
                .retain(|&(pending, owner)| !(pending == key && owner == frame));
            if !state.rolled_back.contains(&key) {
                state.rolled_back.push(key);
            }
            self.inner.rolled_back.store(true, Ordering::Release);
        }
        // A host that discards frames it painted into is either recovering
        // from a failure or has a bug; either way once per ledger is enough.
        if !self.inner.reported.swap(true, Ordering::Relaxed) {
            nana_diagnostics::event!(
                nana_diagnostics::framework::gpu::RETAINED_FRAME_DISCARDED,
                target = key
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolled_back_keys_drain_once_and_clear_the_flag() {
        let ledger = RetainedWrites::new();
        let frame = FrameId::new(1);
        ledger.record(7, frame);
        ledger.record(9, frame);
        assert!(ledger.in_flight(7, FrameId::new(2)));
        assert!(!ledger.in_flight(7, frame), "a frame never blocks itself");
        ledger.roll_back(7, frame);
        ledger.roll_back(7, frame);
        ledger.settle(9, frame);
        assert!(ledger.has_rolled_back());
        assert!(!ledger.in_flight(7, FrameId::new(2)));
        assert!(!ledger.in_flight(9, FrameId::new(2)));
        let mut drained = Vec::new();
        ledger.drain_rolled_back(|key| drained.push(key));
        assert_eq!(drained, [7]);
        assert!(!ledger.has_rolled_back());
        ledger.drain_rolled_back(|_| panic!("nothing left to drain"));
    }
}
