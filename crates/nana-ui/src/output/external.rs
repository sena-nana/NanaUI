//! Retained, window-independent presentation targets.
//!
//! `ExternalSurface` is deliberately a producer-side contract.  It owns a
//! bounded set of persistent GPU textures and asks the normal
//! [`SceneWgpuPainter`] to render into one of them.  A consumer samples the
//! last completed texture through [`ExternalFrame`]; sampling at a higher
//! cadence never starts a new UI frame.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nana_gpu::{
    DeviceGeneration, FrameId, GpuContext, GpuError, GpuTexture, GpuTextureDescriptor,
    GpuTextureFormat, GpuTextureUsages,
};
use nana_ui_platform::SharedFetchHost;
use nana_ui_scene::UiScene;

use super::planner::RenderTargetRequirements;
use crate::{
    AlphaEncoding, HostTextureRegistry, SceneGpuRendererRegistry, ScenePaintError,
    ScenePaintViewport, ScenePresentationProfile, SceneWgpuPainter,
};
use nana_ui_core::{OutputWorkObservation, WorkCounters};

const DEFAULT_SLOTS: usize = 3;
const MIN_SLOTS: usize = 2;
const MAX_SLOTS: usize = 8;

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
    NoAvailableTarget,
    NoCompletedFrame,
    StaleFrame,
    DeviceMismatch {
        expected: DeviceGeneration,
        found: DeviceGeneration,
    },
    DeviceLost,
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
    retired: bool,
}

impl ExternalFrameLease {
    fn retire(&mut self) {
        if self.retired {
            return;
        }
        self.retired = true;
        if let Ok(mut queue) = self.queue.lock() {
            queue.released.push(self.key);
        }
    }
}

impl Drop for ExternalFrameLease {
    fn drop(&mut self) {
        self.retire();
    }
}

/// A consumer lease on the latest completed external frame.
///
/// The texture is retained by this value until it is dropped.  Dropping (or
/// explicitly calling [`Self::retire`]) releases the producer slot at the
/// next producer operation; no CPU/GPU wait is performed. Keep this value
/// alive while using [`Self::texture`], and call [`Self::validate`] after a
/// possible resize or device replacement.
#[derive(Debug)]
pub struct ExternalFrame {
    texture: GpuTexture,
    resource_generation: u64,
    content_revision: u64,
    current_generation: Arc<AtomicU64>,
    lease: ExternalFrameLease,
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

    pub fn retire(mut self) {
        self.lease.retire();
    }
}

#[derive(Debug)]
struct TargetSlot {
    texture: GpuTexture,
    state: SlotState,
    leases: usize,
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

/// A retained presentation producer independent of a native Window.
pub struct ExternalSurface {
    gpu: GpuContext,
    config: ExternalSurfaceConfig,
    painter: SceneWgpuPainter,
    slots: Vec<TargetSlot>,
    published: Option<Published>,
    resource_generation: u64,
    next_content_revision: u64,
    last_key: Option<u64>,
    completions: Arc<Mutex<Vec<Completion>>>,
    leases: Arc<Mutex<LeaseQueue>>,
    current_generation: Arc<AtomicU64>,
    alpha_encoding: AlphaEncoding,
    last_work: OutputWorkObservation,
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
        let slots = create_slots(gpu, config)?;
        let painter = SceneWgpuPainter::new_with_presentation(gpu, config.presentation);
        Ok(Self {
            gpu: gpu.clone(),
            config,
            painter,
            slots,
            published: None,
            resource_generation: 1,
            next_content_revision: 0,
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
        self.painter.set_resource_fetch_host(host);
        self.last_key = None;
    }

    /// The canonical target requirements this producer supplies to a
    /// [`RenderTargetPlanner`](super::RenderTargetPlanner).
    pub fn requirements(&self) -> RenderTargetRequirements {
        Self::requirements_for(self.config, self.alpha_encoding)
    }

    fn requirements_for(
        config: ExternalSurfaceConfig,
        alpha_encoding: AlphaEncoding,
    ) -> RenderTargetRequirements {
        let usage = if config.copy_source {
            GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_SRC
        } else {
            GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::SAMPLED
        };
        RenderTargetRequirements::new(config.extent, config.format())
            .with_color_space(config.presentation.color_space)
            .with_alpha(alpha_encoding)
            .persistent(true)
            .with_usage(usage)
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

    /// Fold the last producer observation into a Runtime frame snapshot.
    /// Output producers do not own the caller's `WorkCounters`, so this seam
    /// is explicit at the frame-driver boundary.
    pub fn record_last_work(&self, counters: &mut WorkCounters) {
        counters.record_output_work(self.last_work);
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
        let painter = SceneWgpuPainter::new_with_presentation(gpu, config.presentation);
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
        let painter = SceneWgpuPainter::new_with_presentation(gpu, self.config.presentation);
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
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        scene: &UiScene,
        host_revision: u64,
        viewport: ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<ExternalRenderOutcome, ExternalSurfaceError> {
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
        let key = render_key(
            scene,
            host_revision,
            host_textures.map(HostTextureRegistry::revision),
            viewport,
            self.config.presentation,
            self.resource_generation,
            self.painter.image_revision(),
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
            return Ok(ExternalRenderOutcome::Reused {
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
            return Ok(ExternalRenderOutcome::Deferred);
        }
        let Some(slot_index) = self.available_slot() else {
            return Ok(ExternalRenderOutcome::Deferred);
        };
        let Some(mut frame) = self.gpu.try_begin_frame("NanaUI external surface") else {
            return Ok(ExternalRenderOutcome::Deferred);
        };
        let revision = self.next_content_revision.saturating_add(1);
        let target = self.slots[slot_index].texture.render_target()?;
        self.painter.set_alpha_encoding(self.alpha_encoding);
        let paint = self.painter.paint_target(
            crate::RenderTargetId(external_target_id(self.resource_generation, slot_index)),
            scene,
            &mut frame,
            &target,
            viewport,
            host_textures,
            gpu_renderers,
        );
        if let Err(error) = paint {
            frame.discard();
            return Err(error.into());
        }
        let submitted = frame.submit();
        let completion = Arc::clone(&self.completions);
        let generation = self.resource_generation;
        let event = Completion {
            generation,
            slot: slot_index,
            frame: submitted.frame(),
            revision,
        };
        self.gpu
            .on_submission_complete(&submitted, move || {
                if let Ok(mut events) = completion.lock() {
                    events.push(event);
                }
            })
            .map_err(ExternalSurfaceError::Gpu)?;
        self.slots[slot_index].state = SlotState::InFlight {
            frame: submitted.frame(),
            revision,
        };
        self.next_content_revision = revision;
        self.last_key = Some(key);
        self.last_work.content_revisions = 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_CONTENT_REVISIONS);
        Ok(ExternalRenderOutcome::Submitted {
            resource_generation: self.resource_generation,
            content_revision: revision,
            frame: submitted.frame(),
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
        let slot = self
            .slots
            .get_mut(published.slot)
            .ok_or(ExternalSurfaceError::StaleFrame)?;
        slot.leases = slot.leases.saturating_add(1);
        Ok(ExternalFrame {
            texture: slot.texture.clone(),
            resource_generation: self.resource_generation,
            content_revision: published.revision,
            current_generation: Arc::clone(&self.current_generation),
            lease: ExternalFrameLease {
                key: LeaseKey {
                    resource_generation: self.resource_generation,
                    slot: published.slot,
                },
                queue: Arc::clone(&self.leases),
                retired: false,
            },
        })
    }

    fn available_slot(&self) -> Option<usize> {
        self.slots.iter().enumerate().find_map(|(index, slot)| {
            (slot.leases == 0
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
        });
    }
    Ok(slots)
}

fn external_target_id(generation: u64, slot: usize) -> u64 {
    generation
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(slot as u64 + 1)
}

fn render_key(
    scene: &UiScene,
    host_revision: u64,
    texture_revision: Option<u64>,
    viewport: ScenePaintViewport,
    presentation: ScenePresentationProfile,
    resource_generation: u64,
    image_revision: u64,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    scene.projection_revision().hash(&mut hasher);
    host_revision.hash(&mut hasher);
    texture_revision.hash(&mut hasher);
    resource_generation.hash(&mut hasher);
    image_revision.hash(&mut hasher);
    presentation.hash(&mut hasher);
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

    #[test]
    fn requirements_advertise_copy_source_only_when_requested() {
        let base = ExternalSurfaceConfig::new([640, 480], GpuTextureFormat::RGBA8_UNORM);
        assert!(
            !ExternalSurface::requirements_for(base, AlphaEncoding::Linear)
                .usage
                .contains(GpuTextureUsages::COPY_SRC)
        );
        assert!(
            ExternalSurface::requirements_for(base.with_copy_source(true), AlphaEncoding::Linear)
                .usage
                .contains(GpuTextureUsages::COPY_SRC)
        );
    }
}
