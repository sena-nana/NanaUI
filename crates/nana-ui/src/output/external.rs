//! Retained, window-independent presentation targets.
//!
//! `ExternalSurface` is deliberately a producer-side contract.  It owns a
//! bounded set of persistent GPU textures and asks the normal
//! [`SceneWgpuPainter`] to render into one of them.  A consumer samples the
//! last completed texture through [`ExternalFrame`]; sampling at a higher
//! cadence never starts a new UI frame.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use nana_gpu::{
    DeviceGeneration, FrameContext, FrameId, GpuContext, GpuError, GpuSubmission, GpuTexture,
    GpuTextureDescriptor, GpuTextureFormat, GpuTextureUsages,
};
use nana_ui_platform::SharedFetchHost;
use nana_ui_scene::UiScene;

use super::provenance::{OutputProvenance, OutputSurfaceId};
use super::trace::OutputWorkObservation;
use crate::{
    AlphaEncoding, HostTextureRegistry, SceneGpuRendererRegistry, ScenePaintError,
    ScenePaintViewport, ScenePresentationProfile, SceneWgpuPainter,
};

const DEFAULT_SLOTS: usize = 3;
const MIN_SLOTS: usize = 2;
pub(crate) const MAX_SLOTS: usize = 8;

/// Configuration for a retained external target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalSurfaceConfig {
    pub extent: [u32; 2],
    pub presentation: ScenePresentationProfile,
    pub slots: usize,
    pub copy_source: bool,
}

impl ExternalSurfaceConfig {
    pub const fn new(extent: [u32; 2], format: GpuTextureFormat) -> Self {
        Self {
            extent,
            presentation: ScenePresentationProfile::sdr(format),
            slots: DEFAULT_SLOTS,
            copy_source: false,
        }
    }

    pub const fn with_presentation(
        extent: [u32; 2],
        presentation: ScenePresentationProfile,
    ) -> Self {
        Self {
            extent,
            presentation,
            slots: DEFAULT_SLOTS,
            copy_source: false,
        }
    }

    pub const fn with_slots(mut self, slots: usize) -> Self {
        self.slots = slots;
        self
    }

    pub const fn with_copy_source(mut self, enabled: bool) -> Self {
        self.copy_source = enabled;
        self
    }

    pub const fn format(self) -> GpuTextureFormat {
        self.presentation.target_format
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalSurfaceError {
    InvalidExtent,
    InvalidSlotCount {
        slots: usize,
    },
    InvalidViewport,
    /// [`ExternalSurface::record`] without a painter on a surface that was
    /// created to borrow its host's.
    NoPainter,
    /// The plan was made for another resource generation, or was already
    /// bound.
    StalePlan,
    NoAvailableTarget,
    NoCompletedFrame,
    StaleFrame,
    DeviceMismatch {
        expected: DeviceGeneration,
        found: DeviceGeneration,
    },
    DeviceLost,
    /// A sampled completed revision already contains this producer.
    CompositionCycle {
        surface: OutputSurfaceId,
    },
    Gpu(GpuError),
    Paint(ScenePaintError),
}

impl std::fmt::Display for ExternalSurfaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidExtent => formatter.write_str("external surface extent must be non-zero"),
            Self::InvalidSlotCount { slots } => write!(
                formatter,
                "external surface needs {MIN_SLOTS}..={MAX_SLOTS} slots, got {slots}"
            ),
            Self::InvalidViewport => formatter.write_str(
                "external surface viewport physical size does not match its target extent",
            ),
            Self::NoPainter => formatter
                .write_str("external surface was created without a painter and was given none"),
            Self::StalePlan => {
                formatter.write_str("external surface plan belongs to an older target generation")
            }
            Self::NoAvailableTarget => formatter
                .write_str("all external surface targets are in flight or leased by a consumer"),
            Self::NoCompletedFrame => {
                formatter.write_str("external surface has no completed frame")
            }
            Self::StaleFrame => {
                formatter.write_str("external surface frame belongs to an old resource generation")
            }
            Self::DeviceMismatch { expected, found } => write!(
                formatter,
                "external surface resource belongs to device {found}, expected {expected}"
            ),
            Self::DeviceLost => formatter.write_str("external surface device is lost"),
            Self::CompositionCycle { surface } => write!(
                formatter,
                "output composition feeds surface {} back into itself",
                surface.get()
            ),
            Self::Gpu(error) => error.fmt(formatter),
            Self::Paint(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ExternalSurfaceError {}

impl From<GpuError> for ExternalSurfaceError {
    fn from(error: GpuError) -> Self {
        Self::Gpu(error)
    }
}

impl From<ScenePaintError> for ExternalSurfaceError {
    fn from(error: ScenePaintError) -> Self {
        Self::Paint(error)
    }
}

/// What happened when a producer was asked to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalRenderOutcome {
    /// The scene was encoded and submitted.  The revision becomes sampleable
    /// after the device reports completion through [`GpuContext`].
    Submitted {
        resource_generation: u64,
        content_revision: u64,
        frame: FrameId,
    },
    /// No commands were recorded: the consumer can keep sampling the last
    /// completed revision.
    Reused {
        resource_generation: u64,
        content_revision: u64,
    },
    /// A non-blocking frame slot or retained target was unavailable.  The
    /// previous completed frame remains published.
    Deferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LeaseKey {
    resource_generation: u64,
    slot: usize,
}

#[derive(Debug, Default)]
struct LeaseQueue {
    released: Vec<LeaseKey>,
}

#[derive(Debug)]
struct ExternalFrameLease {
    key: LeaseKey,
    queue: Arc<Mutex<LeaseQueue>>,
}

impl Drop for ExternalFrameLease {
    fn drop(&mut self) {
        self.queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .released
            .push(self.key);
    }
}

/// A consumer lease on the latest completed external frame.
///
/// The texture is retained by this value until it is dropped.  Dropping (or
/// explicitly calling [`Self::retire`]) releases the producer slot at the
/// next producer operation; no CPU/GPU wait is performed. Keep this value
/// alive while using [`Self::texture`], and call [`Self::validate`] after a
/// possible resize or device replacement.
#[derive(Debug, Clone)]
pub struct ExternalFrame {
    texture: GpuTexture,
    resource_generation: u64,
    content_revision: u64,
    presentation: ScenePresentationProfile,
    alpha_encoding: AlphaEncoding,
    provenance: OutputProvenance,
    current_generation: Arc<AtomicU64>,
    _lease: Arc<ExternalFrameLease>,
}

impl ExternalFrame {
    /// Whether this lease still belongs to the producer's current target
    /// generation. Old leases remain droppable and keep their texture alive,
    /// but must not be imported into a replacement device/target.
    pub fn is_current(&self) -> bool {
        self.current_generation.load(Ordering::Acquire) == self.resource_generation
    }

    pub fn validate(&self) -> Result<(), ExternalSurfaceError> {
        self.is_current()
            .then_some(())
            .ok_or(ExternalSurfaceError::StaleFrame)
    }

    /// The sampled texture handle, after verifying that the producer has not
    /// resized or replaced its device since this lease was acquired.
    pub fn texture(&self) -> Result<&GpuTexture, ExternalSurfaceError> {
        self.validate()?;
        Ok(&self.texture)
    }

    pub const fn resource_generation(&self) -> u64 {
        self.resource_generation
    }

    pub const fn content_revision(&self) -> u64 {
        self.content_revision
    }

    pub fn device_generation(&self) -> DeviceGeneration {
        self.texture.generation()
    }

    pub const fn presentation(&self) -> ScenePresentationProfile {
        self.presentation
    }

    pub const fn alpha_encoding(&self) -> AlphaEncoding {
        self.alpha_encoding
    }

    pub fn provenance(&self) -> &OutputProvenance {
        &self.provenance
    }

    pub fn retire(self) {
        drop(self);
    }

    /// Release this consumer's reference only after its submitted GPU work
    /// completes. Other clones can continue displaying or sampling the same
    /// revision; the producer reuses the slot after the last reference ends.
    /// This remains valid after resize, so old-generation work can retire.
    pub fn retire_after(
        self,
        gpu: &GpuContext,
        submission: &GpuSubmission,
    ) -> Result<(), ExternalSurfaceError> {
        let expected = self.device_generation();
        for found in [gpu.generation(), submission.generation()] {
            if expected != found {
                return Err(ExternalSurfaceError::DeviceMismatch { expected, found });
            }
        }
        gpu.on_submission_complete(submission, move || drop(self))?;
        Ok(())
    }
}

#[derive(Debug)]
struct Reservation {
    key: u64,
    frame: Mutex<Option<FrameId>>,
    discarded: AtomicBool,
    provenance: Mutex<Option<OutputProvenance>>,
}

impl Reservation {
    fn is_live(&self) -> bool {
        !self.discarded.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
struct TargetSlot {
    texture: GpuTexture,
    state: SlotState,
    leases: usize,
    reservation: Weak<Reservation>,
    alpha_encoding: AlphaEncoding,
    frame: Option<FrameId>,
    provenance: Option<OutputProvenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Available,
    InFlight { frame: FrameId, revision: u64 },
    Published { revision: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Published {
    slot: usize,
    revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Completion {
    generation: u64,
    slot: usize,
    frame: FrameId,
    revision: u64,
}

/// A revision [`ExternalSurface::prepare`] decided to encode: the slot it goes
/// to and what it will be published as. Record it into a frame with
/// [`ExternalSurface::record`], then hand the frame's submission to
/// [`ExternalSurface::bind_submission`]; a plan whose frame is dropped is
/// simply forgotten.
#[derive(Debug)]
#[must_use = "a plan does nothing until it is recorded and bound"]
pub struct ExternalFramePlan {
    resource_generation: u64,
    slot: usize,
    revision: u64,
    key: u64,
    reservation: Arc<Reservation>,
    alpha_encoding: AlphaEncoding,
    host_revision: u64,
}

impl ExternalFramePlan {
    #[cfg(feature = "hosted")]
    pub(crate) const fn slot_index(&self) -> usize {
        self.slot
    }

    pub const fn content_revision(&self) -> u64 {
        self.revision
    }

    pub const fn resource_generation(&self) -> u64 {
        self.resource_generation
    }
}

/// What [`ExternalSurface::prepare`] found.
#[derive(Debug)]
pub enum ExternalPrepare {
    /// Nothing changed since the published revision: record nothing.
    Reused {
        resource_generation: u64,
        content_revision: u64,
    },
    /// No target slot is free (in flight or leased). Record nothing.
    Deferred,
    /// Record this revision.
    Record(ExternalFramePlan),
}

/// A retained presentation producer independent of a native Window.
pub struct ExternalSurface {
    id: OutputSurfaceId,
    gpu: GpuContext,
    config: ExternalSurfaceConfig,
    /// `None` for a surface that records with its host's painter.
    painter: Option<SceneWgpuPainter>,
    /// Mixed into every slot's [`crate::RenderTargetId`], so slots of several
    /// surfaces can share one painter with each other and with windows.
    target_namespace: u64,
    slots: Vec<TargetSlot>,
    published: Option<Published>,
    resource_generation: u64,
    next_content_revision: u64,
    next_plan_revision: u64,
    renderer_tick: u64,
    last_key: Option<u64>,
    completions: Arc<Mutex<Vec<Completion>>>,
    leases: Arc<Mutex<LeaseQueue>>,
    current_generation: Arc<AtomicU64>,
    alpha_encoding: AlphaEncoding,
    last_work: OutputWorkObservation,
}

impl Drop for ExternalSurface {
    fn drop(&mut self) {
        // Teardown invalidates new imports, while leases already captured by
        // submitted GPU work continue owning their allocation until retirement.
        self.current_generation.store(0, Ordering::Release);
    }
}

impl std::fmt::Debug for ExternalSurface {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExternalSurface")
            .field("extent", &self.config.extent)
            .field("slots", &self.slots.len())
            .field("resource_generation", &self.resource_generation)
            .field("published", &self.published)
            .finish_non_exhaustive()
    }
}

impl ExternalSurface {
    pub fn new(
        gpu: &GpuContext,
        config: ExternalSurfaceConfig,
    ) -> Result<Self, ExternalSurfaceError> {
        validate_config(config)?;
        if gpu.is_lost() {
            return Err(ExternalSurfaceError::DeviceLost);
        }
        let painter = SceneWgpuPainter::try_new_with_presentation(gpu, config.presentation)?;
        Self::build(gpu, config, Some(painter), 0)
    }

    /// A surface that records with a painter its host passes to
    /// [`Self::record`] (a window's own), so the output shares its pipelines,
    /// glyph atlas and image cache instead of building a second set.
    /// `target_namespace` keeps its slots' [`crate::RenderTargetId`]s apart
    /// from every other target that painter serves.
    pub fn with_host_painter(
        gpu: &GpuContext,
        config: ExternalSurfaceConfig,
        target_namespace: u64,
    ) -> Result<Self, ExternalSurfaceError> {
        Self::build(gpu, config, None, target_namespace)
    }

    fn build(
        gpu: &GpuContext,
        config: ExternalSurfaceConfig,
        painter: Option<SceneWgpuPainter>,
        target_namespace: u64,
    ) -> Result<Self, ExternalSurfaceError> {
        validate_config(config)?;
        if gpu.is_lost() {
            return Err(ExternalSurfaceError::DeviceLost);
        }
        let slots = create_slots(gpu, config)?;
        Ok(Self {
            id: OutputSurfaceId::next(),
            gpu: gpu.clone(),
            config,
            painter,
            target_namespace,
            slots,
            published: None,
            resource_generation: 1,
            next_content_revision: 0,
            next_plan_revision: 0,
            renderer_tick: 0,
            last_key: None,
            completions: Arc::new(Mutex::new(Vec::new())),
            leases: Arc::new(Mutex::new(LeaseQueue::default())),
            current_generation: Arc::new(AtomicU64::new(1)),
            alpha_encoding: AlphaEncoding::Linear,
            last_work: OutputWorkObservation::default(),
        })
    }

    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }

    pub const fn id(&self) -> OutputSurfaceId {
        self.id
    }

    pub(crate) fn owns_target(&self, target: &nana_gpu::GpuRenderTarget) -> bool {
        self.slots.iter().any(|slot| {
            nana_gpu::__framework::texture(&slot.texture)
                == nana_gpu::__framework::target_view(target).texture()
        })
    }

    pub const fn config(&self) -> ExternalSurfaceConfig {
        self.config
    }

    pub const fn resource_generation(&self) -> u64 {
        self.resource_generation
    }

    pub fn published_revision(&mut self) -> Option<u64> {
        self.drain_events();
        self.published.map(|published| published.revision)
    }

    pub fn set_alpha_encoding(&mut self, encoding: AlphaEncoding) {
        if self.alpha_encoding != encoding {
            self.alpha_encoding = encoding;
            // Alpha storage is part of the presentation contract. A target
            // can be retained, but the first frame after this boundary must
            // be encoded with the new transfer semantics.
            self.last_key = None;
        }
    }

    /// Set the host used for policy-controlled `url(...)` resources. Changing
    /// it invalidates the retained revision because the painter's bindings
    /// may now resolve through a different egress.
    pub fn set_resource_fetch_host(&mut self, host: Option<SharedFetchHost>) {
        if let Some(painter) = self.painter.as_mut() {
            painter.set_resource_fetch_host(host);
        }
        self.last_key = None;
    }

    /// The painter target ids of the current slots. A host that records with
    /// its own painter removes them from it when it drops or resizes this
    /// surface.
    pub fn target_ids(&self) -> impl Iterator<Item = crate::RenderTargetId> + '_ {
        (0..self.slots.len()).map(|slot| self.target_id(self.resource_generation, slot))
    }

    fn target_id(&self, generation: u64, slot: usize) -> crate::RenderTargetId {
        crate::RenderTargetId(self.target_namespace ^ external_target_id(generation, slot))
    }

    /// Drive WGPU's non-blocking completion callbacks and retire leases that
    /// consumers dropped. This is safe to call at any consumer cadence.
    pub fn poll(&mut self) {
        self.gpu.poll();
        self.drain_events();
        self.drain_leases();
    }

    /// Output counters for the last producer operation. A static `sample()`
    /// does not change this observation because sampling is consumer work.
    pub const fn last_work(&self) -> OutputWorkObservation {
        self.last_work
    }

    /// Resize or change the presentation profile at a configuration boundary.
    /// New resources are created before the old surface is replaced; a failed
    /// resize therefore leaves the previous completed frame usable.
    pub fn resize(
        &mut self,
        gpu: &GpuContext,
        config: ExternalSurfaceConfig,
    ) -> Result<(), ExternalSurfaceError> {
        validate_config(config)?;
        if gpu.is_lost() {
            return Err(ExternalSurfaceError::DeviceLost);
        }
        if self.config == config && self.gpu.same_device(gpu) {
            self.last_work = OutputWorkObservation::default();
            return Ok(());
        }
        if !self.gpu.same_device(gpu) {
            return Err(ExternalSurfaceError::DeviceMismatch {
                expected: self.gpu.generation(),
                found: gpu.generation(),
            });
        }
        let slots = create_slots(gpu, config)?;
        let painter = if self.painter.is_some() {
            Some(SceneWgpuPainter::try_new_with_presentation(
                gpu,
                config.presentation,
            )?)
        } else {
            None
        };
        self.drain_events();
        self.config = config;
        self.gpu = gpu.clone();
        self.painter = painter;
        self.slots = slots;
        self.published = None;
        self.last_key = None;
        self.resource_generation = self.resource_generation.saturating_add(1);
        self.current_generation
            .store(self.resource_generation, Ordering::Release);
        self.last_work = OutputWorkObservation {
            target_recreates: self.slots.len(),
            ..Default::default()
        };
        nana_diagnostics::metric!(
            nana_diagnostics::framework::gpu::OUTPUT_TARGET_RECREATES,
            self.slots.len() as u64
        );
        Ok(())
    }

    /// Replace the device generation after device loss.  Old samples are
    /// discarded and consumers must acquire a new frame from this surface.
    pub fn replace_gpu(&mut self, gpu: &GpuContext) -> Result<(), ExternalSurfaceError> {
        if gpu.is_lost() {
            return Err(ExternalSurfaceError::DeviceLost);
        }
        if self.gpu.same_device(gpu) {
            self.last_work = OutputWorkObservation::default();
            return Ok(());
        }
        let slots = create_slots(gpu, self.config)?;
        let painter = if self.painter.is_some() {
            Some(SceneWgpuPainter::try_new_with_presentation(
                gpu,
                self.config.presentation,
            )?)
        } else {
            None
        };
        self.gpu = gpu.clone();
        self.painter = painter;
        self.slots = slots;
        self.published = None;
        self.last_key = None;
        self.resource_generation = self.resource_generation.saturating_add(1);
        self.current_generation
            .store(self.resource_generation, Ordering::Release);
        self.completions
            .lock()
            .map(|mut events| events.clear())
            .ok();
        self.last_work = OutputWorkObservation {
            target_recreates: self.slots.len(),
            ..Default::default()
        };
        nana_diagnostics::metric!(
            nana_diagnostics::framework::gpu::OUTPUT_TARGET_RECREATES,
            self.slots.len() as u64
        );
        Ok(())
    }

    /// Produce a new retained revision if the scene or host content changed.
    /// `host_revision` must include external producer/texture revisions that
    /// are not represented by [`UiScene::projection_revision`].
    ///
    /// This is [`Self::prepare`], [`Self::record`] into a frame of its own
    /// and [`Self::bind_submission`], with this surface's own painter.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        scene: &UiScene,
        host_revision: u64,
        viewport: ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<ExternalRenderOutcome, ExternalSurfaceError> {
        let host_revision = if let Some(renderers) = gpu_renderers {
            let renderer_revision = renderers.retained_revision(scene).unwrap_or_else(|| {
                self.renderer_tick = self.renderer_tick.wrapping_add(1);
                self.renderer_tick
            });
            let mut revision = DefaultHasher::new();
            host_revision.hash(&mut revision);
            renderer_revision.hash(&mut revision);
            revision.finish()
        } else {
            host_revision
        };
        let plan = match self.prepare(scene, host_revision, viewport, host_textures)? {
            ExternalPrepare::Reused {
                resource_generation,
                content_revision,
            } => {
                return Ok(ExternalRenderOutcome::Reused {
                    resource_generation,
                    content_revision,
                });
            }
            ExternalPrepare::Deferred => return Ok(ExternalRenderOutcome::Deferred),
            ExternalPrepare::Record(plan) => plan,
        };
        let Some(mut frame) = self.gpu.try_begin_frame("NanaUI external surface") else {
            self.record_deferred();
            return Ok(ExternalRenderOutcome::Deferred);
        };
        if let Err(error) = self.record(
            &plan,
            &mut frame,
            None,
            scene,
            viewport,
            host_textures,
            gpu_renderers,
        ) {
            frame.discard();
            return Err(error);
        }
        let submitted = frame.submit();
        self.last_work.submissions += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_SUBMISSIONS);
        self.bind_submission(plan, &submitted)
    }

    /// Poll for a safely writable slot before scheduling graph preparations.
    /// This does not reserve a slot, record commands or wait for the GPU.
    pub(crate) fn has_record_capacity(&mut self) -> Result<bool, ExternalSurfaceError> {
        if self.gpu.is_lost() {
            return Err(ExternalSurfaceError::DeviceLost);
        }
        self.poll();
        if self.gpu.is_lost() {
            return Err(ExternalSurfaceError::DeviceLost);
        }
        Ok(self.available_slot().is_some())
    }

    /// Decide whether this presentation needs a new revision, and where it
    /// would go. Records nothing and never waits.
    pub fn prepare(
        &mut self,
        scene: &UiScene,
        host_revision: u64,
        viewport: ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
    ) -> Result<ExternalPrepare, ExternalSurfaceError> {
        self.last_work = OutputWorkObservation::default();
        if self.gpu.is_lost() {
            return Err(ExternalSurfaceError::DeviceLost);
        }
        self.poll();
        if self.gpu.is_lost() {
            return Err(ExternalSurfaceError::DeviceLost);
        }
        if viewport.physical_size != self.config.extent {
            return Err(ExternalSurfaceError::InvalidViewport);
        }
        if let Some(painter) = self.painter.as_mut() {
            painter.refresh_image_resources();
        }
        let key = render_key(
            scene,
            host_revision,
            referenced_texture_revision(scene, host_textures)?,
            viewport,
            self.config.presentation,
            self.alpha_encoding,
            self.resource_generation,
            self.painter
                .as_ref()
                .map_or(0, SceneWgpuPainter::image_revision),
        );
        // The published frame is this content only when it is the latest
        // revision submitted; an older one still showing while the newer
        // completes is a different scene.
        if self.last_key == Some(key)
            && let Some(published) = self.published
            && published.revision == self.next_content_revision
        {
            self.last_work.idle_reuse_frames = 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_IDLE_REUSE_FRAMES);
            return Ok(ExternalPrepare::Reused {
                resource_generation: self.resource_generation,
                content_revision: published.revision,
            });
        }
        // A submitted revision is not sampleable until completion. Do not
        // render the same immutable scene into another slot while the queue
        // is still in flight; a changed key is allowed to pipeline normally.
        if self.last_key == Some(key)
            && self
                .slots
                .iter()
                .any(|slot| matches!(slot.state, SlotState::InFlight { .. }))
        {
            self.record_deferred();
            return Ok(ExternalPrepare::Deferred);
        }
        if self.slots.iter().any(|slot| {
            slot.reservation
                .upgrade()
                .is_some_and(|reservation| reservation.key == key && reservation.is_live())
        }) {
            self.record_deferred();
            return Ok(ExternalPrepare::Deferred);
        }
        let Some(slot) = self.available_slot() else {
            self.record_deferred();
            return Ok(ExternalPrepare::Deferred);
        };
        self.next_plan_revision = self.next_plan_revision.saturating_add(1);
        let reservation = Arc::new(Reservation {
            key,
            frame: Mutex::new(None),
            discarded: AtomicBool::new(false),
            provenance: Mutex::new(None),
        });
        self.slots[slot].reservation = Arc::downgrade(&reservation);
        Ok(ExternalPrepare::Record(ExternalFramePlan {
            resource_generation: self.resource_generation,
            slot,
            revision: self.next_plan_revision,
            key,
            reservation,
            alpha_encoding: self.alpha_encoding,
            host_revision,
        }))
    }

    /// The texture `plan` records into, for work the host appends after the
    /// scene in the same frame (a conversion pass, an export copy).
    pub fn plan_texture(
        &self,
        plan: &ExternalFramePlan,
    ) -> Result<&GpuTexture, ExternalSurfaceError> {
        if plan.resource_generation != self.resource_generation
            || !plan.reservation.is_live()
            || !self.slots.get(plan.slot).is_some_and(|slot| {
                slot.reservation
                    .upgrade()
                    .is_some_and(|owner| Arc::ptr_eq(&owner, &plan.reservation))
            })
        {
            return Err(ExternalSurfaceError::StalePlan);
        }
        self.slots
            .get(plan.slot)
            .map(|slot| &slot.texture)
            .ok_or(ExternalSurfaceError::StalePlan)
    }

    /// Paint `scene` into the plan's slot inside the caller's `frame`. With
    /// `painter` the host's painter records it (see
    /// [`Self::with_host_painter`]); without, this surface's own. Nothing is
    /// published until [`Self::bind_submission`].
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        plan: &ExternalFramePlan,
        frame: &mut FrameContext,
        painter: Option<&mut SceneWgpuPainter>,
        scene: &UiScene,
        viewport: ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ExternalSurfaceError> {
        if plan.alpha_encoding != self.alpha_encoding {
            return Err(ExternalSurfaceError::StalePlan);
        }
        if frame.generation() != self.gpu.generation() {
            return Err(ExternalSurfaceError::DeviceMismatch {
                expected: self.gpu.generation(),
                found: frame.generation(),
            });
        }
        if plan
            .reservation
            .frame
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
        {
            return Err(ExternalSurfaceError::StalePlan);
        }
        if viewport.physical_size != self.config.extent {
            return Err(ExternalSurfaceError::InvalidViewport);
        }
        let (frozen_textures, sampled) = match host_textures {
            Some(registry) => {
                let plan = scene
                    .frame_plan()
                    .map_err(|error| ScenePaintError::InvalidRenderGraph(error.to_string()))?;
                let slots = plan.custom_nodes.iter().filter_map(|id| {
                    let primitive = scene.primitive(*id)?;
                    match &primitive.kind {
                        nana_ui_scene::ScenePrimitiveKind::Custom { node, .. }
                            if node.renderer.as_ref() == "nana.host-texture" =>
                        {
                            Some(node.resource.as_ref())
                        }
                        _ => None,
                    }
                });
                let (frozen, sampled) = registry.snapshot_output_bindings(slots);
                (Some(frozen), sampled)
            }
            None => (None, Vec::new()),
        };
        let provenance = OutputProvenance::composed(self.id, &sampled)
            .map_err(|surface| ExternalSurfaceError::CompositionCycle { surface })?;
        if render_key(
            scene,
            plan.host_revision,
            referenced_texture_revision(scene, frozen_textures.as_ref())?,
            viewport,
            self.config.presentation,
            self.alpha_encoding,
            self.resource_generation,
            self.painter
                .as_ref()
                .map_or(0, SceneWgpuPainter::image_revision),
        ) != plan.key
        {
            return Err(ExternalSurfaceError::StalePlan);
        }
        let target = self.plan_texture(plan)?.render_target()?;
        let id = self.target_id(plan.resource_generation, plan.slot);
        let encoding = self.alpha_encoding;
        let painter = match painter {
            Some(painter) => painter,
            None => self
                .painter
                .as_mut()
                .ok_or(ExternalSurfaceError::NoPainter)?,
        };
        painter.try_select_presentation(self.config.presentation)?;
        painter.set_alpha_encoding(encoding);
        painter.paint_target(
            id,
            scene,
            frame,
            &target,
            viewport,
            frozen_textures.as_ref(),
            gpu_renderers,
        )?;
        *plan
            .reservation
            .provenance
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(provenance);
        *plan
            .reservation
            .frame
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(frame.id());
        let reservation = Arc::clone(&plan.reservation);
        frame.on_discard(move || reservation.discarded.store(true, Ordering::Release));
        frame.keep_alive_until_complete(Arc::clone(&plan.reservation));
        self.slots[plan.slot].alpha_encoding = encoding;
        self.last_work.ui_renders += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_UI_RENDERS);
        Ok(())
    }

    /// Mark the plan's slot in flight on `submission`; it is published when
    /// that submission completes.
    pub fn bind_submission(
        &mut self,
        plan: ExternalFramePlan,
        submission: &GpuSubmission,
    ) -> Result<ExternalRenderOutcome, ExternalSurfaceError> {
        self.plan_texture(&plan)?;
        if plan.resource_generation != self.resource_generation
            || plan.slot >= self.slots.len()
            || !matches!(self.slots[plan.slot].state, SlotState::Available)
            || *plan
                .reservation
                .frame
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                != Some(submission.frame())
        {
            return Err(ExternalSurfaceError::StalePlan);
        }
        let completion = Arc::clone(&self.completions);
        let event = Completion {
            generation: plan.resource_generation,
            slot: plan.slot,
            frame: submission.frame(),
            revision: plan.revision,
        };
        self.gpu
            .on_submission_complete(submission, move || {
                if let Ok(mut events) = completion.lock() {
                    events.push(event);
                }
            })
            .map_err(ExternalSurfaceError::Gpu)?;
        self.slots[plan.slot].state = SlotState::InFlight {
            frame: submission.frame(),
            revision: plan.revision,
        };
        self.slots[plan.slot].frame = Some(submission.frame());
        self.slots[plan.slot].provenance = plan
            .reservation
            .provenance
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if plan.revision >= self.next_content_revision {
            self.next_content_revision = plan.revision;
            self.last_key = Some(plan.key);
        }
        self.last_work.content_revisions = 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_CONTENT_REVISIONS);
        Ok(ExternalRenderOutcome::Submitted {
            resource_generation: self.resource_generation,
            content_revision: plan.revision,
            frame: submission.frame(),
        })
    }

    /// Acquire the most recently completed frame.  This only takes a lease;
    /// it never starts a render or submits work.
    pub fn sample(&mut self) -> Result<ExternalFrame, ExternalSurfaceError> {
        if self.gpu.is_lost() {
            return Err(ExternalSurfaceError::DeviceLost);
        }
        self.poll();
        if self.gpu.is_lost() {
            return Err(ExternalSurfaceError::DeviceLost);
        }
        let Some(published) = self.published else {
            return Err(ExternalSurfaceError::NoCompletedFrame);
        };
        self.lease_slot(published.slot, published.revision)
    }

    /// Host-only lease on the exact output its submitted frame produced.
    /// Submission ordering lets a host append GPU work without waiting for
    /// publication; ordinary external sampling stays completion-only.
    pub(crate) fn lease_submitted(
        &mut self,
        submission: &GpuSubmission,
    ) -> Result<ExternalFrame, ExternalSurfaceError> {
        if submission.generation() != self.gpu.generation() {
            return Err(ExternalSurfaceError::DeviceMismatch {
                expected: self.gpu.generation(),
                found: submission.generation(),
            });
        }
        let (index, revision) =
            self.slots
                .iter()
                .enumerate()
                .find_map(|(index, slot)| {
                    if slot.frame != Some(submission.frame()) {
                        return None;
                    }
                    match slot.state {
                        SlotState::InFlight { revision, .. }
                        | SlotState::Published { revision } => Some((index, revision)),
                        SlotState::Available => None,
                    }
                })
                .ok_or(ExternalSurfaceError::StaleFrame)?;
        self.lease_slot(index, revision)
    }

    fn lease_slot(
        &mut self,
        index: usize,
        revision: u64,
    ) -> Result<ExternalFrame, ExternalSurfaceError> {
        let slot = self
            .slots
            .get_mut(index)
            .ok_or(ExternalSurfaceError::StaleFrame)?;
        slot.leases = slot.leases.saturating_add(1);
        Ok(ExternalFrame {
            texture: slot.texture.clone(),
            resource_generation: self.resource_generation,
            content_revision: revision,
            presentation: self.config.presentation,
            alpha_encoding: slot.alpha_encoding,
            provenance: slot
                .provenance
                .clone()
                .expect("published output has recorded provenance"),
            current_generation: Arc::clone(&self.current_generation),
            _lease: Arc::new(ExternalFrameLease {
                key: LeaseKey {
                    resource_generation: self.resource_generation,
                    slot: index,
                },
                queue: Arc::clone(&self.leases),
            }),
        })
    }

    fn record_deferred(&mut self) {
        self.last_work.deferred += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_DEFERRED);
    }

    fn available_slot(&self) -> Option<usize> {
        self.slots.iter().enumerate().find_map(|(index, slot)| {
            (slot.leases == 0
                && !slot
                    .reservation
                    .upgrade()
                    .is_some_and(|reservation| reservation.is_live())
                && !matches!(slot.state, SlotState::InFlight { .. })
                && self.published.map(|published| published.slot) != Some(index))
            .then_some(index)
        })
    }

    fn drain_events(&mut self) {
        let events = self
            .completions
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default();
        for event in events {
            if event.generation != self.resource_generation
                || event.slot >= self.slots.len()
                || !matches!(
                    self.slots[event.slot].state,
                    SlotState::InFlight { frame, revision }
                        if frame == event.frame && revision == event.revision
                )
            {
                continue;
            }
            if self
                .published
                .is_some_and(|published| published.revision > event.revision)
            {
                self.slots[event.slot].state = SlotState::Available;
                continue;
            }
            if let Some(previous) = self.published
                && previous.slot != event.slot
                && self.slots[previous.slot].leases == 0
            {
                self.slots[previous.slot].state = SlotState::Available;
            }
            self.slots[event.slot].state = SlotState::Published {
                revision: event.revision,
            };
            self.published = Some(Published {
                slot: event.slot,
                revision: event.revision,
            });
        }
    }

    fn drain_leases(&mut self) {
        let released = self
            .leases
            .lock()
            .map(|mut queue| std::mem::take(&mut queue.released))
            .unwrap_or_default();
        for key in released {
            if key.resource_generation == self.resource_generation
                && let Some(slot) = self.slots.get_mut(key.slot)
            {
                slot.leases = slot.leases.saturating_sub(1);
                if slot.leases == 0
                    && self.published.map(|published| published.slot) != Some(key.slot)
                    && !matches!(slot.state, SlotState::InFlight { .. })
                {
                    slot.state = SlotState::Available;
                }
            }
        }
    }
}

fn validate_config(config: ExternalSurfaceConfig) -> Result<(), ExternalSurfaceError> {
    if config.extent.contains(&0) {
        return Err(ExternalSurfaceError::InvalidExtent);
    }
    if !(MIN_SLOTS..=MAX_SLOTS).contains(&config.slots) {
        return Err(ExternalSurfaceError::InvalidSlotCount {
            slots: config.slots,
        });
    }
    Ok(())
}

fn create_slots(
    gpu: &GpuContext,
    config: ExternalSurfaceConfig,
) -> Result<Vec<TargetSlot>, ExternalSurfaceError> {
    let mut slots = Vec::with_capacity(config.slots);
    for _ in 0..config.slots {
        let usage = if config.copy_source {
            GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_SRC
        } else {
            GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::SAMPLED
        };
        let texture = gpu.create_texture(&GpuTextureDescriptor {
            label: Some("nana-ui.external-surface"),
            width: config.extent[0],
            height: config.extent[1],
            format: config.presentation.target_format,
            usage,
        })?;
        slots.push(TargetSlot {
            texture,
            state: SlotState::Available,
            leases: 0,
            reservation: Weak::new(),
            alpha_encoding: AlphaEncoding::Linear,
            frame: None,
            provenance: None,
        });
    }
    Ok(slots)
}

fn external_target_id(generation: u64, slot: usize) -> u64 {
    generation
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(slot as u64 + 1)
}

fn referenced_texture_revision(
    scene: &UiScene,
    registry: Option<&HostTextureRegistry>,
) -> Result<Option<u64>, ExternalSurfaceError> {
    let Some(registry) = registry else {
        return Ok(None);
    };
    let plan = scene
        .frame_plan()
        .map_err(|error| ScenePaintError::InvalidRenderGraph(error.to_string()))?;
    Ok(Some(registry.output_revision(
        plan.custom_nodes.iter().filter_map(|id| {
            let primitive = scene.primitive(*id)?;
            match &primitive.kind {
                nana_ui_scene::ScenePrimitiveKind::Custom { node, .. }
                    if node.renderer.as_ref() == "nana.host-texture" =>
                {
                    Some(node.resource.as_ref())
                }
                _ => None,
            }
        }),
    )))
}

// Every independent retained input must participate in the cache identity.
#[allow(clippy::too_many_arguments)]
fn render_key(
    scene: &UiScene,
    host_revision: u64,
    texture_revision: Option<u64>,
    viewport: ScenePaintViewport,
    presentation: ScenePresentationProfile,
    alpha_encoding: AlphaEncoding,
    resource_generation: u64,
    image_revision: u64,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    scene.projection_revision().hash(&mut hasher);
    scene.presentation_epoch().hash(&mut hasher);
    scene.motion_gpu_source().hash(&mut hasher);
    scene.motion_gpu_structure_epoch().hash(&mut hasher);
    if scene.compositor_needs_tick() {
        scene.motion_gpu_now().hash(&mut hasher);
    }
    host_revision.hash(&mut hasher);
    texture_revision.hash(&mut hasher);
    resource_generation.hash(&mut hasher);
    image_revision.hash(&mut hasher);
    presentation.hash(&mut hasher);
    alpha_encoding.hash(&mut hasher);
    viewport.logical_size[0].to_bits().hash(&mut hasher);
    viewport.logical_size[1].to_bits().hash(&mut hasher);
    viewport.physical_size.hash(&mut hasher);
    viewport.scale_factor.to_bits().hash(&mut hasher);
    viewport.scene_origin[0].to_bits().hash(&mut hasher);
    viewport.scene_origin[1].to_bits().hash(&mut hasher);
    viewport.target_origin[0].to_bits().hash(&mut hasher);
    viewport.target_origin[1].to_bits().hash(&mut hasher);
    viewport.clear_color.map(f32::to_bits).hash(&mut hasher);
    viewport.clear.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retained_surface() -> ExternalSurface {
        ExternalSurface::new(
            &crate::test_gpu::context(),
            ExternalSurfaceConfig::new([32, 24], GpuTextureFormat::RGBA8_UNORM).with_slots(2),
        )
        .unwrap()
    }

    fn viewport() -> ScenePaintViewport {
        ScenePaintViewport {
            logical_size: [32.0, 24.0],
            physical_size: [32, 24],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        }
    }

    fn prepare(surface: &mut ExternalSurface, scene: &UiScene, revision: u64) -> ExternalFramePlan {
        match surface.prepare(scene, revision, viewport(), None).unwrap() {
            ExternalPrepare::Record(plan) => plan,
            other => panic!("expected reserved target, got {other:?}"),
        }
    }

    #[test]
    fn prepared_targets_are_exclusive_and_abandoned_plans_release_them() {
        let mut surface = retained_surface();
        let scene = UiScene::new();
        let first = prepare(&mut surface, &scene, 0);
        let second = prepare(&mut surface, &scene, 1);
        assert_ne!(first.slot, second.slot);
        assert!(matches!(
            surface.prepare(&scene, 2, viewport(), None),
            Ok(ExternalPrepare::Deferred)
        ));
        let slot = first.slot;
        drop(first);
        assert_eq!(prepare(&mut surface, &scene, 2).slot, slot);
    }

    #[test]
    fn an_unsubmitted_reservation_does_not_duplicate_the_same_content() {
        let mut surface = retained_surface();
        let scene = UiScene::new();
        let plan = surface.prepare(&scene, 0, viewport(), None).unwrap();
        assert!(matches!(plan, ExternalPrepare::Record(_)));
        assert!(matches!(
            surface.prepare(&scene, 0, viewport(), None),
            Ok(ExternalPrepare::Deferred)
        ));
        assert_eq!(surface.last_work().ui_renders, 0);
        assert_eq!(surface.last_work().submissions, 0);
        drop(plan);
        assert!(matches!(
            surface.prepare(&scene, 0, viewport(), None),
            Ok(ExternalPrepare::Record(_))
        ));
    }

    #[test]
    fn a_plan_cannot_be_used_by_another_surface_or_an_unrelated_submission() {
        let mut surface = retained_surface();
        let scene = UiScene::new();
        let other = retained_surface();
        let plan = prepare(&mut surface, &scene, 0);
        assert!(matches!(
            other.plan_texture(&plan),
            Err(ExternalSurfaceError::StalePlan)
        ));
        let unrelated = surface.gpu.begin_frame("unrelated output frame").submit();
        assert_eq!(
            surface.bind_submission(plan, &unrelated),
            Err(ExternalSurfaceError::StalePlan)
        );
        assert_eq!(surface.published_revision(), None);
    }

    #[test]
    fn discarding_recorded_output_invalidates_the_plan_and_releases_its_slot() {
        let mut surface = retained_surface();
        let scene = UiScene::new();
        let plan = prepare(&mut surface, &scene, 0);
        let mut frame = surface.gpu.begin_frame("discard retained output");
        surface
            .record(&plan, &mut frame, None, &scene, viewport(), None, None)
            .unwrap();
        let held = prepare(&mut surface, &scene, 1);
        assert_ne!(held.slot, plan.slot);
        frame.discard();
        assert!(matches!(
            surface.plan_texture(&plan),
            Err(ExternalSurfaceError::StalePlan)
        ));
        assert_eq!(prepare(&mut surface, &scene, 2).slot, plan.slot);
        assert_eq!(surface.published_revision(), None);
    }

    #[test]
    fn dropping_a_recorded_plan_keeps_the_target_reserved_until_frame_discard() {
        let mut surface = retained_surface();
        let scene = UiScene::new();
        let plan = prepare(&mut surface, &scene, 0);
        let slot = plan.slot;
        let mut frame = surface.gpu.begin_frame("orphan retained output");
        surface
            .record(&plan, &mut frame, None, &scene, viewport(), None, None)
            .unwrap();
        drop(plan);
        let held = prepare(&mut surface, &scene, 1);
        assert_ne!(held.slot, slot);
        assert!(matches!(
            surface.prepare(&scene, 2, viewport(), None),
            Ok(ExternalPrepare::Deferred)
        ));
        frame.discard();
        assert_eq!(prepare(&mut surface, &scene, 2).slot, slot);
    }

    #[test]
    fn output_requires_the_submission_of_the_frame_that_recorded_it() {
        let mut surface = retained_surface();
        let scene = UiScene::new();
        let plan = prepare(&mut surface, &scene, 0);
        let mut recorded = surface.gpu.begin_frame("recorded output");
        surface
            .record(&plan, &mut recorded, None, &scene, viewport(), None, None)
            .unwrap();
        let unrelated = surface.gpu.begin_frame("wrong output submission").submit();
        assert_eq!(
            surface.bind_submission(plan, &unrelated),
            Err(ExternalSurfaceError::StalePlan)
        );
        recorded.discard();
        assert_eq!(surface.published_revision(), None);
    }

    #[test]
    fn cloned_frames_share_the_lease_until_the_last_consumer_retires() {
        let mut surface = retained_surface();
        let scene = UiScene::new();
        let plan = prepare(&mut surface, &scene, 0);
        let mut frame = surface.gpu.begin_frame("shared output lease");
        surface
            .record(&plan, &mut frame, None, &scene, viewport(), None, None)
            .unwrap();
        let submission = frame.submit();
        surface.bind_submission(plan, &submission).unwrap();
        let sample = surface.lease_submitted(&submission).unwrap();
        let slot = sample._lease.key.slot;
        let copy = sample.clone();
        sample.retire();
        surface.drain_leases();
        assert_eq!(surface.slots[slot].leases, 1);
        let pending = Arc::downgrade(&copy._lease);
        copy.retire_after(&surface.gpu, &submission).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while pending.upgrade().is_some() {
            surface.poll();
            assert!(
                std::time::Instant::now() < deadline,
                "consumer did not retire"
            );
            std::thread::yield_now();
        }
        surface.drain_leases();
        assert_eq!(surface.slots[slot].leases, 0);
    }

    #[test]
    fn dropping_the_producer_invalidates_imports_without_breaking_gpu_retirement() {
        let mut surface = retained_surface();
        let scene = UiScene::new();
        let plan = prepare(&mut surface, &scene, 0);
        let gpu = surface.gpu.clone();
        let mut frame = gpu.begin_frame("retired producer output");
        surface
            .record(&plan, &mut frame, None, &scene, viewport(), None, None)
            .unwrap();
        let submission = frame.submit();
        surface.bind_submission(plan, &submission).unwrap();
        let lease = surface.lease_submitted(&submission).unwrap();
        let resource = lease.texture().unwrap().clone();
        drop(surface);
        assert_eq!(lease.validate(), Err(ExternalSurfaceError::StaleFrame));
        assert_eq!(resource.generation(), gpu.generation());
        lease.retire_after(&gpu, &submission).unwrap();
        gpu.poll();
    }

    #[test]
    fn changing_alpha_while_a_recording_is_pending_does_not_reuse_old_encoding() {
        let mut surface = retained_surface();
        let scene = UiScene::new();
        let ExternalPrepare::Record(plan) = surface.prepare(&scene, 0, viewport(), None).unwrap()
        else {
            panic!("target must be available");
        };
        let mut frame = surface.gpu.begin_frame("pending alpha encoding");
        surface
            .record(&plan, &mut frame, None, &scene, viewport(), None, None)
            .unwrap();
        surface.set_alpha_encoding(AlphaEncoding::Gamma);
        let submitted = frame.submit();
        surface.bind_submission(plan, &submitted).unwrap();
        let old = surface.lease_submitted(&submitted).unwrap();
        assert_eq!(old.alpha_encoding(), AlphaEncoding::Linear);
        assert!(!matches!(
            surface.prepare(&scene, 0, viewport(), None),
            Ok(ExternalPrepare::Reused { .. })
        ));
    }

    #[test]
    fn reverse_submission_of_reserved_plans_cannot_relabel_newer_content() {
        let mut surface = retained_surface();
        let scene = UiScene::new();
        let ExternalPrepare::Record(older) = surface.prepare(&scene, 10, viewport(), None).unwrap()
        else {
            panic!("first target must be available");
        };
        let ExternalPrepare::Record(newer) = surface.prepare(&scene, 20, viewport(), None).unwrap()
        else {
            panic!("second target must be available");
        };
        let latest_revision = newer.content_revision();
        let mut first = surface.gpu.begin_frame("newer reserved output");
        surface
            .record(&newer, &mut first, None, &scene, viewport(), None, None)
            .unwrap();
        surface.bind_submission(newer, &first.submit()).unwrap();
        let mut second = surface.gpu.begin_frame("older reserved output");
        surface
            .record(&older, &mut second, None, &scene, viewport(), None, None)
            .unwrap();
        surface.bind_submission(older, &second.submit()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while surface.published_revision() != Some(latest_revision) {
            surface.poll();
            assert!(
                std::time::Instant::now() < deadline,
                "output did not publish"
            );
            std::thread::yield_now();
        }
        assert!(matches!(
            surface.prepare(&scene, 20, viewport(), None),
            Ok(ExternalPrepare::Reused { content_revision, .. })
                if content_revision == latest_revision
        ));
        assert!(!matches!(
            surface.prepare(&scene, 10, viewport(), None),
            Ok(ExternalPrepare::Reused { .. })
        ));
    }

    #[test]
    fn config_rejects_zero_extent_and_unbounded_slot_counts() {
        assert_eq!(
            validate_config(ExternalSurfaceConfig::new(
                [0, 480],
                GpuTextureFormat::RGBA8_UNORM,
            )),
            Err(ExternalSurfaceError::InvalidExtent)
        );
        assert_eq!(
            validate_config(
                ExternalSurfaceConfig::new([640, 480], GpuTextureFormat::RGBA8_UNORM)
                    .with_slots(MIN_SLOTS - 1)
            ),
            Err(ExternalSurfaceError::InvalidSlotCount {
                slots: MIN_SLOTS - 1
            })
        );
        assert_eq!(
            validate_config(
                ExternalSurfaceConfig::new([640, 480], GpuTextureFormat::RGBA8_UNORM)
                    .with_slots(MAX_SLOTS + 1)
            ),
            Err(ExternalSurfaceError::InvalidSlotCount {
                slots: MAX_SLOTS + 1
            })
        );
    }
}
