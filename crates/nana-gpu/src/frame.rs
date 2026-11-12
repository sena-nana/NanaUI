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

/// Frame-slot work performed while acquiring this exact frame. Unlike a
/// device-policy delta, this cannot include another host thread's waits.
/// These counts describe frame acquisition, not SDK or GPU execution time.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameAcquisition {
    pub frame_slot_waits: u64,
    pub frame_slot_stalls: u64,
}

/// Which submission budget a [`FrameContext`] is held to.
///
/// The device has one queue, shared with the host's window frames and,
/// through the driver, with the desktop compositor. Each lane counts its own
/// unfinished submissions, so a producer's backlog never delays a window frame
/// and window frames never use up a producer's budget.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameLane {
    /// Frames the host presents or renders UI into
    /// ([`GpuContext::begin_frame`], [`GpuContext::try_begin_frame`]). Paced
    /// by frame slots; frames that run without a slot are still held to the
    /// slot count of unfinished submissions.
    Presentation,
    /// Work produced for the UI to show (video, models, frame exchange
    /// copies, custom render producers): [`GpuContext::begin_producer_frame`]
    /// and [`GpuContext::try_begin_producer_frame`]. Takes no frame slot; at
    /// most [`GpuDeviceState::producer_submission_budget`] submissions are
    /// unfinished at once.
    ///
    /// [`GpuDeviceState::producer_submission_budget`]: crate::GpuDeviceState::producer_submission_budget
    Producer,
}

impl FrameLane {
    pub(crate) const fn diagnostic_code(self) -> u64 {
        match self {
            Self::Presentation => 0,
            Self::Producer => 1,
        }
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
    lane: FrameLane,
    encoder: Option<wgpu::CommandEncoder>,
    retained: Vec<(RetainedWrites, u64)>,
    frame_slot: Option<FrameSlotId>,
    acquisition: FrameAcquisition,
    transient_registry: Arc<Mutex<Vec<(TransientResourceKey, wgpu::Buffer)>>>,
    /// Writes renderers queued for this frame; submitted ahead of it.
    uploads: crate::upload::FrameUploads,
    /// Raw queue operations that must follow this frame's `queue.submit`
    /// under the same submission-order lock (native-export ready signals).
    after_submit: Vec<Box<dyn FnOnce() + Send>>,
    on_discard: Vec<Box<dyn FnOnce() + Send>>,
    completion_resources: Vec<Box<dyn Send>>,
}

impl fmt::Debug for FrameContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrameContext")
            .field("id", &self.id)
            .field("lane", &self.lane)
            .field("generation", &self.gpu.generation())
            .field("retained", &self.retained.len())
            .finish()
    }
}

impl FrameContext {
    pub(crate) fn new(
        gpu: GpuContext,
        id: FrameId,
        lane: FrameLane,
        encoder: wgpu::CommandEncoder,
        frame_slot: Option<FrameSlotId>,
        acquisition: FrameAcquisition,
    ) -> Self {
        let uploads = Arc::new(Mutex::new(gpu.inner.upload_ring.batch()));
        Self {
            gpu,
            id,
            lane,
            encoder: Some(encoder),
            retained: Vec::new(),
            frame_slot,
            acquisition,
            transient_registry: Arc::new(Mutex::new(Vec::new())),
            uploads,
            after_submit: Vec::new(),
            on_discard: Vec::new(),
            completion_resources: Vec::new(),
        }
    }

    /// Runs `hook` right after this frame's `queue.submit`, still holding the
    /// submission-order lock, so a raw queue operation it performs is ordered
    /// directly after the frame's commands. Dropped unrun when the frame is
    /// discarded.
    #[cfg_attr(not(all(windows, feature = "native-export")), allow(dead_code))]
    pub(crate) fn after_submit(&mut self, hook: Box<dyn FnOnce() + Send>) {
        self.after_submit.push(hook);
    }

    /// Run a cleanup if this recording is discarded. A successful submit
    /// removes the cleanup without calling it.
    pub fn on_discard(&mut self, cleanup: impl FnOnce() + Send + 'static) {
        self.on_discard.push(Box::new(cleanup));
    }

    /// Keep a sampled resource or reservation alive until this frame's GPU
    /// work completes. Discard releases it without waiting or submitting.
    pub fn keep_alive_until_complete(&mut self, resource: impl Send + 'static) {
        self.completion_resources.push(Box::new(resource));
    }

    pub(crate) fn uploads(&self) -> crate::upload::FrameUploads {
        Arc::clone(&self.uploads)
    }

    /// This frame's writes, behind whatever was queued outside a frame.
    fn take_uploads(&self) -> crate::upload::UploadBatch {
        let mut batch = std::mem::take(
            &mut *self
                .gpu
                .inner
                .pending_uploads
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        crate::upload::merge(
            &mut batch,
            &mut self.uploads.lock().unwrap_or_else(|e| e.into_inner()),
        );
        batch
    }

    pub fn id(&self) -> FrameId {
        self.id
    }

    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }

    /// The submission budget this frame is held to.
    pub const fn lane(&self) -> FrameLane {
        self.lane
    }

    pub const fn acquisition(&self) -> FrameAcquisition {
        self.acquisition
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

    /// Finish the encoder and submit it. Holds the submission-order lock only
    /// for the submit itself.
    ///
    /// When this frame's [`FrameLane`] already has its budget of unfinished
    /// submissions, this first waits for the lane's oldest one to finish.
    /// The wait happens before the submission-order lock is taken, so it
    /// never delays another lane's submit.
    pub fn submit(mut self) -> GpuSubmission {
        let encoder = self.encoder.take().expect("frame encoder");
        let started = Instant::now();
        let commands = encoder.finish();
        let policy = self.gpu.policy().clone();
        self.gpu.admit(self.lane, true);
        // Turned into an in-flight entry by `submitted` below; an unwind
        // before that gives the room back.
        let reservation = Reservation(Some(policy.lane(self.lane)));
        let index = {
            // Makes token order match actual queue order.
            let _order = self
                .gpu
                .inner
                .submission_order
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            // Writes queued outside a frame (or by a dropped one) are taken
            // in submission order: taken earlier, by a submitter that queues
            // later, they would land after a frame recorded against them.
            // Taking a staging chunk may wait for one in flight, never for an
            // unsubmitted one, since every recording happens under this lock.
            let uploads = crate::upload::record(
                &self.gpu.inner.upload_ring,
                &self.gpu.inner.device,
                &self.gpu.inner.queue,
                self.gpu.policy(),
                self.take_uploads(),
            );
            let upload_chunk = uploads.as_ref().map(|uploads| uploads.chunk);
            let index = self.gpu.inner.queue.submit(
                uploads
                    .map(|uploads| uploads.commands)
                    .into_iter()
                    .chain([commands]),
            );
            if let Some(chunk) = upload_chunk {
                self.gpu.inner.upload_ring.submitted(chunk, index.clone());
            }
            // Registered under the lock, right behind this submit: it fires
            // once this submission (and everything before it) finished.
            let done = Arc::new(AtomicBool::new(false));
            let finished = Arc::clone(&done);
            self.gpu
                .inner
                .queue
                .on_submitted_work_done(move || finished.store(true, Ordering::Release));
            reservation.submitted(index.clone(), done);
            for hook in self.after_submit.drain(..) {
                hook();
            }
            let submission = self
                .gpu
                .inner
                .next_submission
                .fetch_add(1, Ordering::Relaxed);
            self.gpu.policy().bind_pipeline_retirement(submission);
            (index, submission)
        };
        let cpu_duration = started.elapsed();
        self.on_discard.clear();
        if !self.completion_resources.is_empty() {
            let resources = std::mem::take(&mut self.completion_resources);
            self.gpu
                .inner
                .queue
                .on_submitted_work_done(move || drop(resources));
        }
        for (owner, key) in self.retained.drain(..) {
            owner.settle(key, self.id);
        }
        let completed = index.1;
        if let Some(slot) = self.frame_slot {
            self.gpu
                .policy()
                .mark_frame_slot_submitted(slot, self.id.get(), index.0.clone());
        }
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
            if !transient_buffers.is_empty() || policy.has_retired() {
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
            acquisition: self.acquisition,
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
            // Queue writes cannot be taken back, and renderers keep CPU
            // mirrors that assume they landed: the next submission lands them.
            let mut pending = self
                .gpu
                .inner
                .pending_uploads
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            crate::upload::merge(
                &mut pending,
                &mut self.uploads.lock().unwrap_or_else(|e| e.into_inner()),
            );
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
        for cleanup in self.on_discard.drain(..) {
            cleanup();
        }
    }
}

/// A lane reservation taken by [`FrameContext::submit`], handed back if the
/// submit unwinds before reaching the queue.
struct Reservation<'a>(Option<&'a crate::budget::SubmissionBudget<wgpu::SubmissionIndex>>);

impl Reservation<'_> {
    fn submitted(mut self, index: wgpu::SubmissionIndex, done: Arc<AtomicBool>) {
        if let Some(lane) = self.0.take() {
            lane.submitted(index, done);
        }
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if let Some(lane) = self.0.take() {
            lane.cancel();
        }
    }
}

/// A submitted frame.
pub struct GpuSubmission {
    frame: FrameId,
    generation: DeviceGeneration,
    cpu_duration: Duration,
    acquisition: FrameAcquisition,
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
    pub const fn acquisition(&self) -> FrameAcquisition {
        self.acquisition
    }

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
