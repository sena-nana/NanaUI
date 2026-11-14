//! Reference seams for hosts consuming an existing presentation.
//!
//! Native windows, headless encoders and XR runtimes provide destinations;
//! NanaUI transfers the retained presentation on the host's existing device.
//! Third-party native handles, overlay APIs and web bridges stay in the host.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use nana_gpu::{FrameContext, FrameId, GpuContext, GpuSubmission, GpuTexture};

use super::{OutputConsumerId, PresentationDriver, PresentationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayTransportError {
    Unsupported,
    Disconnected,
    Rejected,
}

/// Observable progress of the adapter's last operation. Reused and Deferred
/// both record no transfer, but only Deferred means a consumer holds the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAdapterState {
    Idle,
    Reused,
    Deferred,
    Recorded,
    Pending,
    Presented,
}

/// A native runtime's proof that its GPU has finished reading an overlay.
/// Returning from `SetOverlayTexture` or an equivalent API is insufficient.
///
/// # Safety
/// `is_released` may return true only after all foreign GPU reads complete
/// and the consumer has relinquished any future reads of that revision.
/// While false, this receipt (or the native consumer) must retain the imported
/// resource independently, including if the adapter/receipt is dropped or
/// disconnected. Polling must be non-blocking and safe on the host thread.
pub unsafe trait OverlayRelease {
    fn is_released(&self) -> bool;
}

/// Host-supplied OpenVR/OpenXR/native-overlay integration. The adapter calls
/// `submit` only after the destination copy has completed. The host imports
/// the consumer-owned texture and returns a separate GPU retirement receipt.
/// An error must mean no foreign read was retained. No XR library is linked
/// by NanaUI and implementing this seam is not hardware conformance evidence.
/// `submit` must not block the presentation thread or wait for GPU fences.
///
/// An OpenVR host calls `IVROverlay::SetOverlayTexture` here with its imported
/// destination. The returned receipt must track compositor consumption, not
/// that call's return value. `ClearOverlayTexture`/disconnect stops future
/// sampling; the host must also establish completion of outstanding reads
/// before releasing a slot. If its installed runtime cannot prove completion,
/// it returns `Unsupported` or retains the slot rather than guessing a delay.
pub trait OverlayTransport {
    type Release: OverlayRelease;

    fn submit(&mut self, texture: &GpuTexture) -> Result<Self::Release, OverlayTransportError>;
}

#[derive(Debug)]
pub enum OverlayAdapterError {
    InvalidTargets,
    DeviceMismatch,
    DeviceLost,
    StalePlan,
    Presentation(PresentationError),
    Transport(OverlayTransportError),
}

impl std::fmt::Display for OverlayAdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTargets => f.write_str("overlay needs 2..=8 distinct renderable targets"),
            Self::DeviceMismatch => {
                f.write_str("overlay resource belongs to another GPU generation")
            }
            Self::DeviceLost => f.write_str("overlay device was lost; recreate the adapter"),
            Self::StalePlan => f.write_str("overlay plan does not match its recording/submission"),
            Self::Presentation(error) => error.fmt(f),
            Self::Transport(error) => write!(f, "overlay transport failed: {error:?}"),
        }
    }
}
impl std::error::Error for OverlayAdapterError {}

#[derive(Debug)]
struct Reservation {
    frame: FrameId,
    discarded: AtomicBool,
    revision: Revision,
    sequence: u64,
}

#[derive(Debug, Clone)]
struct Revision {
    resource: u64,
    content: u64,
    texture: GpuTexture,
}

impl Revision {
    fn matches(&self, other: &Self) -> bool {
        self.resource == other.resource
            && self.content == other.content
            && self.texture.ptr_eq(&other.texture)
    }
}

/// One transfer recorded into a host frame. Bind only that frame's submission;
/// discard and abandoned plans never publish a partially written texture.
#[derive(Debug)]
pub struct OverlayFramePlan {
    adapter: u64,
    slot: usize,
    reservation: Arc<Reservation>,
}

enum SlotUse<R> {
    Free,
    Reserved(Weak<Reservation>),
    Pending {
        ready: Arc<AtomicBool>,
        sequence: u64,
    },
    Consuming(R),
}

struct Slot<R> {
    texture: GpuTexture,
    usage: SlotUse<R>,
}

/// Bounded reference adapter for a host-provided XR overlay transport.
///
/// Targets belong to the consumer, are distinct from canonical UI resources,
/// and must not be written elsewhere while installed here. Canonical leases
/// retire after the copy submission; overlay receipts govern only these
/// destination slots. A slow/disconnected runtime fills the bounded pool and
/// `record` returns `None`, preserving the previous presented overlay.
pub struct OverlayHostAdapter<T: OverlayTransport> {
    id: u64,
    gpu: GpuContext,
    consumer: OutputConsumerId,
    transport: T,
    slots: Vec<Slot<T::Release>>,
    last_queued: Option<Revision>,
    queued_sequence: u64,
    next_sequence: u64,
    presented_sequence: u64,
    last_state: OverlayAdapterState,
    trace: Option<super::trace::OutputTraceSnapshot>,
}

impl<T: OverlayTransport> OverlayHostAdapter<T> {
    pub fn new(
        gpu: &GpuContext,
        consumer: OutputConsumerId,
        textures: Vec<GpuTexture>,
        transport: T,
    ) -> Result<Self, OverlayAdapterError> {
        if gpu.is_lost() {
            return Err(OverlayAdapterError::DeviceLost);
        }
        if !(2..=8).contains(&textures.len())
            || textures.iter().enumerate().any(|(index, texture)| {
                texture.render_target().is_err()
                    || textures[..index].iter().any(|other| texture.ptr_eq(other))
            })
        {
            return Err(OverlayAdapterError::InvalidTargets);
        }
        if textures
            .iter()
            .any(|texture| texture.generation() != gpu.generation())
        {
            return Err(OverlayAdapterError::DeviceMismatch);
        }
        static NEXT_ADAPTER: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            id: NEXT_ADAPTER.fetch_add(1, Ordering::Relaxed),
            gpu: gpu.clone(),
            consumer,
            transport,
            slots: textures
                .into_iter()
                .map(|texture| Slot {
                    texture,
                    usage: SlotUse::Free,
                })
                .collect(),
            last_queued: None,
            queued_sequence: 0,
            next_sequence: 0,
            presented_sequence: 0,
            last_state: OverlayAdapterState::Idle,
            trace: None,
        })
    }

    /// Enable allocation-free numeric snapshots. Destinations were supplied
    /// by the host; enabling this does not count their allocation as new work.
    pub fn set_output_trace_enabled(&mut self, enabled: bool) {
        if enabled == self.trace.is_some() {
            return;
        }
        self.trace = enabled.then(|| {
            let texture = &self.slots[0].texture;
            let (width, height) = texture.size();
            super::trace::OutputTraceSnapshot::new(
                0,
                self.consumer,
                super::trace::OutputTraceRole::Consumer,
                super::OutputPath::GpuTransfer,
                texture.format(),
                [width, height],
                self.gpu.generation(),
            )
        });
    }

    pub const fn output_trace(&self) -> Option<super::trace::OutputTraceSnapshot> {
        self.trace
    }

    fn trace_work(&mut self, work: nana_ui_core::OutputWorkObservation, emit_metrics: bool) {
        if emit_metrics {
            super::trace::consumer_metrics(work, None);
        }
        if let Some(trace) = &mut self.trace {
            trace.observe_without_metrics(work, None);
            // Caller-owned frame acquisition and foreign transport internals
            // are outside this adapter's observation, never assumed zero.
            trace.waits = None;
        }
    }

    /// Drive ready/release transitions without waiting for the GPU or runtime.
    pub fn poll(&mut self) -> Result<(), OverlayAdapterError> {
        if self.gpu.is_lost() {
            return Err(OverlayAdapterError::DeviceLost);
        }
        self.gpu.poll();
        // A completion callback may run while polling a lost device. It
        // retires local work, but does not prove that the copy produced valid
        // pixels for a foreign runtime. Preserve receipts on the error path.
        if self.gpu.is_lost() {
            return Err(OverlayAdapterError::DeviceLost);
        }
        let newest = self
            .slots
            .iter()
            .filter_map(|slot| match &slot.usage {
                SlotUse::Pending { ready, sequence }
                    if ready.load(Ordering::Acquire) && *sequence > self.presented_sequence =>
                {
                    Some(*sequence)
                }
                _ => None,
            })
            .max();
        for slot in &mut self.slots {
            let free = match &slot.usage {
                SlotUse::Reserved(reservation) => reservation
                    .upgrade()
                    .is_none_or(|reservation| reservation.discarded.load(Ordering::Acquire)),
                SlotUse::Consuming(release) => release.is_released(),
                _ => false,
            };
            if free {
                slot.usage = SlotUse::Free;
            }
            if let SlotUse::Pending { ready, sequence } = &slot.usage
                && ready.load(Ordering::Acquire)
            {
                let sequence = *sequence;
                if Some(sequence) != newest {
                    slot.usage = SlotUse::Free;
                    continue;
                }
                if self.gpu.is_lost() {
                    return Err(OverlayAdapterError::DeviceLost);
                }
                match self.transport.submit(&slot.texture) {
                    Ok(release) => {
                        slot.usage = SlotUse::Consuming(release);
                        self.presented_sequence = sequence;
                        self.last_state = OverlayAdapterState::Presented;
                    }
                    Err(error) => {
                        slot.usage = SlotUse::Free;
                        self.last_queued = None;
                        return Err(OverlayAdapterError::Transport(error));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn record(
        &mut self,
        driver: &mut PresentationDriver,
        frame: &mut FrameContext,
    ) -> Result<Option<OverlayFramePlan>, OverlayAdapterError> {
        self.last_state = OverlayAdapterState::Idle;
        if self.gpu.is_lost() {
            return Err(OverlayAdapterError::DeviceLost);
        }
        if frame.generation() != self.gpu.generation() {
            return Err(OverlayAdapterError::DeviceMismatch);
        }
        self.poll()?;
        let source = driver
            .sample(self.consumer)
            .map_err(OverlayAdapterError::Presentation)?;
        let revision = Revision {
            resource: source.resource_generation(),
            content: source.content_revision(),
            texture: source
                .texture()
                .map_err(PresentationError::from)
                .map_err(OverlayAdapterError::Presentation)?
                .clone(),
        };
        if self.last_queued.as_ref().is_some_and(|last| last.matches(&revision))
            || self.slots.iter().any(|slot| matches!(&slot.usage, SlotUse::Reserved(active) if active.upgrade().is_some_and(|active| !active.discarded.load(Ordering::Acquire) && active.revision.matches(&revision))))
        {
            self.last_state = OverlayAdapterState::Reused;
            self.trace_work(nana_ui_core::OutputWorkObservation { idle_reuse_frames: 1, ..Default::default() }, true);
            return Ok(None);
        }
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| matches!(slot.usage, SlotUse::Free))
        else {
            self.last_state = OverlayAdapterState::Deferred;
            if let Some(trace) = &mut self.trace {
                trace.deferral_reason = super::trace::OutputTraceDeferral::ConsumerSlots;
            }
            self.trace_work(
                nana_ui_core::OutputWorkObservation {
                    deferred: 1,
                    ..Default::default()
                },
                true,
            );
            return Ok(None);
        };
        if self.slots[index].texture.ptr_eq(&revision.texture) {
            return Err(OverlayAdapterError::InvalidTargets);
        }
        let target = self.slots[index]
            .texture
            .render_target()
            .map_err(|_| OverlayAdapterError::InvalidTargets)?;
        driver
            .present_to(self.consumer, frame, &target)
            .map_err(OverlayAdapterError::Presentation)?;
        if let Some(trace) = &mut self.trace {
            trace.producer = source.provenance().source().get();
            trace.resource_generation = source.resource_generation();
            trace.content_revision = source.content_revision();
        }
        // present_to emitted the consumer pass metrics. This local snapshot
        // observes the same operation without emitting it a second time.
        self.trace_work(
            nana_ui_core::OutputWorkObservation {
                extra_passes: 1,
                gpu_convert_passes: 1,
                ..Default::default()
            },
            false,
        );
        self.next_sequence = self.next_sequence.wrapping_add(1);
        let reservation = Arc::new(Reservation {
            frame: frame.id(),
            discarded: AtomicBool::new(false),
            revision,
            sequence: self.next_sequence,
        });
        let discard = Arc::clone(&reservation);
        frame.on_discard(move || discard.discarded.store(true, Ordering::Release));
        frame.keep_alive_until_complete(Arc::clone(&reservation));
        self.slots[index].usage = SlotUse::Reserved(Arc::downgrade(&reservation));
        self.last_state = OverlayAdapterState::Recorded;
        Ok(Some(OverlayFramePlan {
            adapter: self.id,
            slot: index,
            reservation,
        }))
    }

    pub fn bind_submission(
        &mut self,
        plan: OverlayFramePlan,
        submission: &GpuSubmission,
    ) -> Result<(), OverlayAdapterError> {
        if self.gpu.is_lost() {
            return Err(OverlayAdapterError::DeviceLost);
        }
        if submission.generation() != self.gpu.generation() {
            return Err(OverlayAdapterError::DeviceMismatch);
        }
        if plan.adapter != self.id || plan.reservation.frame != submission.frame()
            || plan.reservation.discarded.load(Ordering::Acquire)
            || !self.slots.get(plan.slot).is_some_and(|slot| matches!(&slot.usage, SlotUse::Reserved(active) if active.upgrade().is_some_and(|active| Arc::ptr_eq(&active, &plan.reservation))))
        { return Err(OverlayAdapterError::StalePlan); }
        let ready = Arc::new(AtomicBool::new(false));
        let completed = Arc::clone(&ready);
        self.gpu
            .on_submission_complete(submission, move || completed.store(true, Ordering::Release))
            .map_err(|_| OverlayAdapterError::DeviceMismatch)?;
        self.slots[plan.slot].usage = SlotUse::Pending {
            ready,
            sequence: plan.reservation.sequence,
        };
        if plan.reservation.sequence >= self.queued_sequence {
            self.queued_sequence = plan.reservation.sequence;
            self.last_queued = Some(plan.reservation.revision.clone());
        }
        self.last_state = OverlayAdapterState::Pending;
        self.trace_work(
            nana_ui_core::OutputWorkObservation {
                submissions: 1,
                ..Default::default()
            },
            true,
        );
        Ok(())
    }

    pub const fn last_state(&self) -> OverlayAdapterState {
        self.last_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::{
        OutputAlpha, OutputConsumer, OutputConsumerKind, OutputOwnership, OutputTextAa,
        PresentationId, PresenterCapabilities, RenderTargetRequirements,
    };
    use crate::{ScenePresentationProfile, test_gpu};
    use nana_gpu::{__framework, GpuTextureDescriptor, GpuTextureFormat, GpuTextureUsages};
    use nana_ui_scene::UiScene;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const EXTENT: [u32; 2] = [32, 24];

    struct Receipt {
        released: Arc<AtomicBool>,
        _texture: GpuTexture,
    }

    // SAFETY: the fixture has no foreign GPU readers. The explicit flag models
    // their release independently from the actual, completed GPU transfer.
    unsafe impl OverlayRelease for Receipt {
        fn is_released(&self) -> bool {
            self.released.load(Ordering::Acquire)
        }
    }

    #[derive(Default)]
    struct TransportState {
        textures: Vec<GpuTexture>,
        released: Vec<Arc<AtomicBool>>,
        reject: Option<OverlayTransportError>,
    }
    struct MockOverlayTransport(Arc<Mutex<TransportState>>);
    impl OverlayTransport for MockOverlayTransport {
        type Release = Receipt;
        fn submit(&mut self, texture: &GpuTexture) -> Result<Receipt, OverlayTransportError> {
            let released = Arc::new(AtomicBool::new(false));
            let mut state = self.0.lock().unwrap();
            if let Some(error) = state.reject.take() {
                return Err(error);
            }
            state.textures.push(texture.clone());
            state.released.push(Arc::clone(&released));
            Ok(Receipt {
                released,
                _texture: texture.clone(),
            })
        }
    }

    fn target(gpu: &GpuContext) -> GpuTexture {
        gpu.create_texture(&GpuTextureDescriptor {
            label: Some("host-provided overlay destination"),
            width: EXTENT[0],
            height: EXTENT[1],
            format: GpuTextureFormat::BGRA8_UNORM,
            usage: GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC,
        })
        .unwrap()
    }

    fn driver(gpu: &GpuContext) -> PresentationDriver {
        let mut driver = PresentationDriver::new(gpu);
        driver
            .configure(&[OutputConsumer {
                id: OutputConsumerId(1),
                kind: OutputConsumerKind::External,
                requirements: RenderTargetRequirements {
                    presentation: PresentationId(1),
                    viewport_revision: 1,
                    extent: EXTENT,
                    profile: ScenePresentationProfile::sdr(GpuTextureFormat::BGRA8_UNORM),
                    alpha: OutputAlpha::GammaPremultiplied,
                    text_aa: OutputTextAa::Grayscale,
                    device_generation: gpu.generation(),
                    ownership: OutputOwnership::SameDevice,
                    persistent: true,
                },
                capabilities: PresenterCapabilities {
                    sampled: true,
                    copy_destination: true,
                    ..Default::default()
                },
            }])
            .unwrap();
        driver
    }

    fn render(
        driver: &mut PresentationDriver,
        scene: &UiScene,
        revision: u64,
    ) -> super::super::ExternalFrame {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut wanted = None;
        loop {
            if wanted.is_none() {
                match driver
                    .render_scene(
                        scene,
                        [32.0, 24.0],
                        revision,
                        [0.2, 0.4, 0.6, 0.5],
                        None,
                        None,
                    )
                    .unwrap()[0]
                {
                    super::super::ExternalRenderOutcome::Submitted {
                        content_revision, ..
                    }
                    | super::super::ExternalRenderOutcome::Reused {
                        content_revision, ..
                    } => wanted = Some(content_revision),
                    super::super::ExternalRenderOutcome::Deferred => {}
                }
            }
            driver.poll();
            if let Ok(frame) = driver.sample(OutputConsumerId(1))
                && Some(frame.content_revision()) == wanted
            {
                return frame;
            }
            assert!(
                Instant::now() < deadline,
                "canonical output did not complete"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn presented(
        adapter: &mut OverlayHostAdapter<MockOverlayTransport>,
        state: &Arc<Mutex<TransportState>>,
        count: usize,
    ) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.lock().unwrap().textures.len() < count {
            adapter.poll().unwrap();
            assert!(
                Instant::now() < deadline,
                "overlay copy did not become ready"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn pixel(gpu: &GpuContext, texture: &GpuTexture) -> [u8; 4] {
        // Explicit test-only readback verifies the real GPU transfer. This is
        // separate from the mock foreign runtime's release conformance.
        let buffer = __framework::device(gpu).create_buffer(&wgpu::BufferDescriptor {
            label: Some("adapter test-only pixel readback"),
            size: 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut frame = gpu.begin_frame("adapter test-only readback");
        __framework::encoder(&mut frame).copy_texture_to_buffer(
            __framework::texture(texture).as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        frame.submit();
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            gpu.poll();
            if let Ok(result) = receive.try_recv() {
                result.unwrap();
                break;
            }
            assert!(Instant::now() < deadline, "pixel readback did not complete");
            std::thread::sleep(Duration::from_millis(1));
        }
        let data = buffer.slice(..).get_mapped_range().unwrap();
        let value = data[..4].try_into().unwrap();
        drop(data);
        buffer.unmap();
        value
    }

    #[test]
    fn overlay_uses_real_gpu_transfer_and_mock_foreign_retirement() {
        let gpu = test_gpu::context();
        let mut driver = driver(&gpu);
        let scene = UiScene::new();
        let canonical = render(&mut driver, &scene, 0);
        let state = Arc::new(Mutex::new(TransportState::default()));
        let mut adapter = OverlayHostAdapter::new(
            &gpu,
            OutputConsumerId(1),
            vec![target(&gpu), target(&gpu)],
            MockOverlayTransport(Arc::clone(&state)),
        )
        .unwrap();
        let mut frame = gpu.begin_frame("consumer-owned overlay copy");
        let plan = adapter.record(&mut driver, &mut frame).unwrap().unwrap();
        assert_eq!(adapter.last_state(), OverlayAdapterState::Recorded);
        assert!(
            state.lock().unwrap().textures.is_empty(),
            "recording is not readiness"
        );
        adapter.bind_submission(plan, &frame.submit()).unwrap();
        assert_eq!(adapter.last_state(), OverlayAdapterState::Pending);
        presented(&mut adapter, &state, 1);
        assert_eq!(adapter.last_state(), OverlayAdapterState::Presented);
        let destination = state.lock().unwrap().textures[0].clone();
        assert!(!destination.ptr_eq(canonical.texture().unwrap()));
        assert_eq!(
            pixel(&gpu, &destination),
            pixel(&gpu, canonical.texture().unwrap())
        );
        let work = *driver.work();
        for _ in 0..240 {
            let mut frame = gpu.begin_frame("static overlay consumer tick");
            assert!(adapter.record(&mut driver, &mut frame).unwrap().is_none());
            assert_eq!(adapter.last_state(), OverlayAdapterState::Reused);
            frame.discard();
        }
        assert_eq!(driver.work().output_extra_passes, work.output_extra_passes);
        assert_eq!(state.lock().unwrap().textures.len(), 1);
        render(&mut driver, &scene, 1);
        let mut second = gpu.begin_frame("second overlay revision");
        let plan = adapter.record(&mut driver, &mut second).unwrap().unwrap();
        adapter.bind_submission(plan, &second.submit()).unwrap();
        presented(&mut adapter, &state, 2);
        render(&mut driver, &scene, 2);
        let mut blocked = gpu.begin_frame("overlay runtime still reads both slots");
        assert!(adapter.record(&mut driver, &mut blocked).unwrap().is_none());
        assert_eq!(adapter.last_state(), OverlayAdapterState::Deferred);
        blocked.discard();
        state.lock().unwrap().released[0].store(true, Ordering::Release);
        let mut next = gpu.begin_frame("released overlay slot");
        assert!(adapter.record(&mut driver, &mut next).unwrap().is_some());
        next.discard();
    }

    #[test]
    fn discarded_or_wrong_submission_overlay_plans_are_not_published() {
        let gpu = test_gpu::context();
        let mut driver = driver(&gpu);
        render(&mut driver, &UiScene::new(), 0);
        let state = Arc::new(Mutex::new(TransportState::default()));
        let mut adapter = OverlayHostAdapter::new(
            &gpu,
            OutputConsumerId(1),
            vec![target(&gpu), target(&gpu)],
            MockOverlayTransport(Arc::clone(&state)),
        )
        .unwrap();
        let mut recorded = gpu.begin_frame("discarded overlay copy");
        let plan = adapter.record(&mut driver, &mut recorded).unwrap().unwrap();
        recorded.discard();
        let unrelated = gpu.begin_frame("unrelated overlay submission").submit();
        assert!(matches!(
            adapter.bind_submission(plan, &unrelated),
            Err(OverlayAdapterError::StalePlan)
        ));
        adapter.poll().unwrap();
        assert!(state.lock().unwrap().textures.is_empty());
        let mut retry = gpu.begin_frame("retry discarded overlay copy");
        assert!(adapter.record(&mut driver, &mut retry).unwrap().is_some());
        retry.discard();
    }

    #[test]
    fn a_canonical_resource_cannot_be_its_own_overlay_transfer_destination() {
        let gpu = test_gpu::context();
        let mut driver = driver(&gpu);
        let source = render(&mut driver, &UiScene::new(), 0);
        let state = Arc::new(Mutex::new(TransportState::default()));
        let mut adapter = OverlayHostAdapter::new(
            &gpu,
            OutputConsumerId(1),
            vec![source.texture().unwrap().clone(), target(&gpu)],
            MockOverlayTransport(state),
        )
        .unwrap();
        let before = driver.work().output_extra_passes;
        let mut frame = gpu.begin_frame("refuse overlay source/destination alias");
        assert!(matches!(
            adapter.record(&mut driver, &mut frame),
            Err(OverlayAdapterError::InvalidTargets)
        ));
        assert_eq!(driver.work().output_extra_passes, before);
        frame.discard();
    }

    #[test]
    fn reverse_submission_cannot_present_an_older_overlay_after_a_newer_revision() {
        let gpu = test_gpu::context();
        let mut driver = driver(&gpu);
        let scene = UiScene::new();
        render(&mut driver, &scene, 0);
        let state = Arc::new(Mutex::new(TransportState::default()));
        let targets = vec![target(&gpu), target(&gpu)];
        let expected = targets[1].clone();
        let mut adapter = OverlayHostAdapter::new(
            &gpu,
            OutputConsumerId(1),
            targets,
            MockOverlayTransport(Arc::clone(&state)),
        )
        .unwrap();
        let mut older = gpu.begin_frame("older overlay reservation");
        let old_plan = adapter.record(&mut driver, &mut older).unwrap().unwrap();
        render(&mut driver, &scene, 1);
        let mut newer = gpu.begin_frame("newer overlay reservation");
        let new_plan = adapter.record(&mut driver, &mut newer).unwrap().unwrap();
        adapter.bind_submission(new_plan, &newer.submit()).unwrap();
        let old_submission = older.submit();
        adapter.bind_submission(old_plan, &old_submission).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let completed = Arc::clone(&done);
        gpu.on_submission_complete(&old_submission, move || {
            completed.store(true, Ordering::Release)
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done.load(Ordering::Acquire) {
            adapter.poll().unwrap();
            assert!(
                Instant::now() < deadline,
                "overlay submissions did not complete"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        adapter.poll().unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.textures.len(), 1);
        assert!(state.textures[0].ptr_eq(&expected));
        drop(state);
        let mut unchanged = gpu.begin_frame("latest overlay already queued");
        assert!(
            adapter
                .record(&mut driver, &mut unchanged)
                .unwrap()
                .is_none()
        );
        unchanged.discard();
    }

    #[test]
    fn a_lost_overlay_device_never_publishes_ready_copies_or_drops_foreign_receipts() {
        const CHILD: &str = "NANA_UI_TEST_OVERLAY_LOSS_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // Marking the shared context lost must not poison parallel GPU
            // tests. The child owns one ordinary test device; loss itself is
            // a host notification, not a physical GPU reset.
            let _guard = test_gpu::lock();
            test_gpu::run_child_with_timeout(
                "output::adapters::tests::a_lost_overlay_device_never_publishes_ready_copies_or_drops_foreign_receipts",
                CHILD,
                "1",
            );
            return;
        }
        let gpu = test_gpu::context();
        let mut driver = driver(&gpu);
        let scene = UiScene::new();
        render(&mut driver, &scene, 0);
        let state = Arc::new(Mutex::new(TransportState::default()));
        let mut adapter = OverlayHostAdapter::new(
            &gpu,
            OutputConsumerId(1),
            vec![target(&gpu), target(&gpu)],
            MockOverlayTransport(Arc::clone(&state)),
        )
        .unwrap();
        let mut first = gpu.begin_frame("overlay consumed before device loss");
        let plan = adapter.record(&mut driver, &mut first).unwrap().unwrap();
        adapter.bind_submission(plan, &first.submit()).unwrap();
        presented(&mut adapter, &state, 1);
        render(&mut driver, &scene, 1);
        let mut second = gpu.begin_frame("overlay ready during device loss");
        let plan = adapter.record(&mut driver, &mut second).unwrap().unwrap();
        adapter.bind_submission(plan, &second.submit()).unwrap();
        // Drive real copy completion without dispatching it to the mock
        // foreign runtime. Its receipt from the first frame is unreleased.
        let deadline = Instant::now() + Duration::from_secs(30);
        while !adapter.slots.iter().any(|slot| matches!(&slot.usage, SlotUse::Pending { ready, .. } if ready.load(Ordering::Acquire))) {
            gpu.poll();
            assert!(Instant::now() < deadline, "overlay transfer did not complete");
            std::thread::sleep(Duration::from_millis(1));
        }
        __framework::mark_lost(
            &gpu,
            nana_gpu::GpuDeviceLost {
                reason: nana_gpu::GpuLossReason::Destroyed,
                message: "overlay fixture host loss".into(),
            },
        );
        assert!(matches!(
            adapter.poll(),
            Err(OverlayAdapterError::DeviceLost)
        ));
        let mut after_loss = gpu.begin_frame("overlay recording after host loss");
        assert!(matches!(
            adapter.record(&mut driver, &mut after_loss),
            Err(OverlayAdapterError::DeviceLost)
        ));
        after_loss.discard();
        assert_eq!(state.lock().unwrap().textures.len(), 1);
        assert!(
            adapter.slots.iter().any(
                |slot| matches!(&slot.usage, SlotUse::Consuming(release) if !release.is_released())
            ),
            "loss is not foreign GPU retirement"
        );
        assert!(
            adapter
                .slots
                .iter()
                .any(|slot| matches!(&slot.usage, SlotUse::Pending { .. })),
            "failed handoff remains unpublished"
        );
    }

    #[test]
    fn a_disconnected_transport_reports_failure_and_can_retry_the_same_revision() {
        let gpu = test_gpu::context();
        let mut driver = driver(&gpu);
        render(&mut driver, &UiScene::new(), 0);
        let state = Arc::new(Mutex::new(TransportState {
            reject: Some(OverlayTransportError::Disconnected),
            ..Default::default()
        }));
        let mut adapter = OverlayHostAdapter::new(
            &gpu,
            OutputConsumerId(1),
            vec![target(&gpu), target(&gpu)],
            MockOverlayTransport(Arc::clone(&state)),
        )
        .unwrap();
        let mut frame = gpu.begin_frame("overlay disconnect during handoff");
        let plan = adapter.record(&mut driver, &mut frame).unwrap().unwrap();
        adapter.bind_submission(plan, &frame.submit()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match adapter.poll() {
                Err(OverlayAdapterError::Transport(OverlayTransportError::Disconnected)) => break,
                Ok(()) => {}
                other => panic!("unexpected handoff outcome {other:?}"),
            }
            assert!(Instant::now() < deadline, "disconnection was not surfaced");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(state.lock().unwrap().textures.is_empty());
        let mut retry = gpu.begin_frame("retry disconnected overlay revision");
        assert!(adapter.record(&mut driver, &mut retry).unwrap().is_some());
        retry.discard();
    }
}
