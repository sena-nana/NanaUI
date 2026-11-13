//! The application presentation driver shared by native and windowless hosts.
//! Application state and the retained document remain owned by the caller.

use nana_gpu::{FrameContext, GpuContext, GpuRenderTarget};
use nana_ui_core::{OutputWorkObservation, WorkCounters};
use nana_ui_runtime::{FrameworkError, LayoutViewport};
use nana_ui_scene::{RuntimeDocument, RuntimeFrameUpdate, UiScene};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) mod frame_driver;

use super::transfer::OutputTransfer;
use super::{
    ExternalFrame, ExternalRenderOutcome, ExternalSurface, ExternalSurfaceConfig,
    ExternalSurfaceError, OutputAlpha, OutputConsumer, OutputConsumerId, OutputPath, OutputPlan,
    OutputTextAa, RenderTargetPlanner, RenderTargetRequirements,
};
use crate::{
    AlphaEncoding, HostTextureRegistry, NanaTextShaper, SceneGpuRendererRegistry,
    ScenePaintViewport,
};

static NEXT_DRIVER_TARGET: AtomicU64 = AtomicU64::new(1);
type ImageWaker = Arc<dyn Fn(&str) + Send + Sync>;

/// No OS window, surface, event loop or second text engine is needed to flush.
pub(crate) fn flush_presentation(
    document: &mut RuntimeDocument,
    viewport: LayoutViewport,
    text: &mut NanaTextShaper,
) -> Result<RuntimeFrameUpdate, FrameworkError> {
    document.flush(viewport, text)
}

struct Target {
    requirements: RenderTargetRequirements,
    surface: ExternalSurface,
    transfers: Vec<OutputTransfer>,
}

/// Host-owned output orchestration on the host's existing GPU context.
///
/// Configure consumers at a structural boundary; render a changed document
/// once per compatible group. `sample` never schedules or submits UI work.
/// Window-only direct rendering stays in the native presenter, outside this
/// retained texture driver.
pub struct PresentationDriver {
    gpu: GpuContext,
    planner: RenderTargetPlanner,
    targets: Vec<Target>,
    text: NanaTextShaper,
    work: WorkCounters,
    pending_plan_rebuilds: usize,
    pending_target_recreates: usize,
    /// One painter for every group: profiles share its atlases and caches,
    /// and each working format keeps a pipeline set inside it.
    painter: Option<crate::SceneWgpuPainter>,
    renderer_tick: u64,
    fetch_host: Option<nana_ui_platform::SharedFetchHost>,
    image_waker: Option<ImageWaker>,
    producers: Option<Arc<crate::SceneResourceProducerRegistry>>,
    resource_revision: u64,
    prepared_producers: Option<u64>,
    traces: Option<Vec<super::trace::OutputTraceSnapshot>>,
}

#[derive(Debug)]
pub enum PresentationError {
    Runtime(FrameworkError),
    Surface(ExternalSurfaceError),
    ResourceProduction(crate::SceneResourceProduceError),
    /// A direct surface or native-export resource must be provided by its host.
    PresenterRequired(OutputConsumerId),
    UnsupportedConsumer(OutputConsumerId),
    InvalidViewport,
}

impl std::fmt::Display for PresentationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Runtime(error) => error.fmt(f),
            Self::Surface(error) => error.fmt(f),
            Self::ResourceProduction(error) => error.fmt(f),
            Self::PresenterRequired(id) => {
                write!(f, "consumer {} needs its concrete presenter", id.0)
            }
            Self::UnsupportedConsumer(id) => write!(f, "unsupported output consumer {}", id.0),
            Self::InvalidViewport => {
                f.write_str("presentation viewport must be finite and positive")
            }
        }
    }
}
impl std::error::Error for PresentationError {}
impl From<ExternalSurfaceError> for PresentationError {
    fn from(value: ExternalSurfaceError) -> Self {
        Self::Surface(value)
    }
}

impl PresentationDriver {
    #[cfg(test)]
    pub(super) fn has_pending_image_work(&self) -> bool {
        self.painter
            .iter()
            .any(|painter| painter.has_pending_images() || painter.has_image_updates())
    }

    /// The one painter every group shares, once a consumer is configured.
    #[cfg(test)]
    pub(super) fn shared_painter(&mut self) -> &mut crate::SceneWgpuPainter {
        self.painter.as_mut().expect("configured painter")
    }

    pub fn new(gpu: &GpuContext) -> Self {
        Self {
            gpu: gpu.clone(),
            planner: RenderTargetPlanner::new(),
            targets: Vec::new(),
            text: NanaTextShaper::default(),
            work: WorkCounters::default(),
            pending_plan_rebuilds: 0,
            pending_target_recreates: 0,
            painter: None,
            renderer_tick: 0,
            fetch_host: None,
            image_waker: None,
            producers: None,
            resource_revision: 0,
            prepared_producers: None,
            traces: None,
        }
    }

    /// Install the host's policy-gated image transport. `None` refuses HTTP(S),
    /// as on the window path; data and local images do not need this transport.
    /// Keep the same Arc identity to retain that host's cached image resources.
    pub fn set_resource_fetch_host(&mut self, host: Option<nana_ui_platform::SharedFetchHost>) {
        let unchanged = match (&self.fetch_host, &host) {
            (None, None) => true,
            (Some(current), Some(next)) => Arc::ptr_eq(current, next),
            _ => false,
        };
        if unchanged {
            return;
        }
        let previous = self.fetch_host.take();
        self.fetch_host = host;
        self.resource_revision = self.resource_revision.wrapping_add(1);
        if let Some(painter) = self.painter.as_mut() {
            if let Some(previous) = &previous {
                painter.retire_resource_fetch_host(previous);
            }
            painter.set_resource_fetch_host(self.fetch_host.clone());
        }
    }

    /// Schedule the owning headless host when an asynchronous image completes.
    /// The callback carries the same resource key as the window image waker.
    pub fn set_image_update_waker(&mut self, wake: Arc<dyn Fn(&str) + Send + Sync>) {
        if let Some(painter) = self.painter.as_mut() {
            painter.set_image_update_waker(Arc::clone(&wake));
        }
        self.image_waker = Some(wake);
    }

    /// Graph preparations share the output frame and its single submission.
    /// A producer without a preparation version is deliberately volatile.
    pub fn set_resource_producers(
        &mut self,
        producers: Option<Arc<crate::SceneResourceProducerRegistry>>,
    ) {
        let unchanged = match (&self.producers, &producers) {
            (None, None) => true,
            (Some(current), Some(next)) => Arc::ptr_eq(current, next),
            _ => false,
        };
        if unchanged {
            return;
        }
        self.producers = producers;
        self.resource_revision = self.resource_revision.wrapping_add(1);
        self.prepared_producers = None;
    }

    /// Configuration is transactional: failed negotiation/allocation keeps the
    /// previous plan and last completed resources usable.
    pub fn configure(&mut self, consumers: &[OutputConsumer]) -> Result<bool, PresentationError> {
        if self.planner.consumers_match(consumers) {
            return Ok(false);
        }
        if let Some(first) = consumers.first()
            && let Some(other) = consumers.iter().find(|consumer| {
                consumer.requirements.presentation != first.requirements.presentation
            })
        {
            return Err(PresentationError::UnsupportedConsumer(other.id));
        }
        let mut candidate = RenderTargetPlanner::new();
        candidate.update(consumers);
        for group in &candidate.plan().groups {
            for binding in &group.consumers {
                match binding.path {
                    OutputPath::Unsupported(_) => {
                        return Err(PresentationError::UnsupportedConsumer(binding.consumer));
                    }
                    OutputPath::DirectSurface | OutputPath::NativeShared => {
                        return Err(PresentationError::PresenterRequired(binding.consumer));
                    }
                    _ => {}
                }
            }
            if group.requirements.device_generation != self.gpu.generation()
                || group.requirements.text_aa != OutputTextAa::Grayscale
                || group.requirements.alpha == OutputAlpha::Straight
            {
                return Err(PresentationError::UnsupportedConsumer(
                    group.consumers[0].consumer,
                ));
            }
        }
        // Allocate replacements before mutating the live topology. Matching
        // groups retain their pool, revisions and outstanding leases.
        let mut replacements = Vec::new();
        for group in &candidate.plan().groups {
            if self
                .targets
                .iter()
                .any(|target| target.requirements == group.requirements)
            {
                continue;
            }
            let namespace = NEXT_DRIVER_TARGET
                .fetch_add(1, Ordering::Relaxed)
                .rotate_left(24);
            let mut surface = ExternalSurface::with_host_painter(
                &self.gpu,
                ExternalSurfaceConfig::with_presentation(
                    group.requirements.extent,
                    group.requirements.profile,
                )
                .with_copy_source(true),
                namespace,
            )?;
            let allocations = surface.config().slots;
            self.pending_target_recreates += allocations;
            self.work.record_output_work(OutputWorkObservation {
                target_recreates: allocations,
                ..Default::default()
            });
            nana_diagnostics::metric!(
                nana_diagnostics::framework::gpu::OUTPUT_TARGET_RECREATES,
                allocations as u64
            );
            surface.set_alpha_encoding(match group.requirements.alpha {
                OutputAlpha::GammaPremultiplied => AlphaEncoding::Gamma,
                _ => AlphaEncoding::Linear,
            });
            replacements.push(Target {
                requirements: group.requirements,
                surface,
                transfers: Vec::new(),
            });
        }
        let mut old = std::mem::take(&mut self.targets);
        for group in &candidate.plan().groups {
            let target = if let Some(index) = old
                .iter()
                .position(|target| target.requirements == group.requirements)
            {
                old.remove(index)
            } else {
                let index = replacements
                    .iter()
                    .position(|target| target.requirements == group.requirements)
                    .expect("allocated group");
                replacements.remove(index)
            };
            self.targets.push(target);
        }
        if let Some(painter) = self.painter.as_mut() {
            for target in old {
                for id in target.surface.target_ids() {
                    painter.remove_target(id);
                }
            }
        }
        let profiles: Vec<_> = candidate
            .plan()
            .groups
            .iter()
            .map(|group| group.requirements.profile)
            .collect();
        if let Some(&first) = profiles.first() {
            let painter = self.painter.get_or_insert_with(|| {
                let mut painter = crate::SceneWgpuPainter::new_with_presentation(&self.gpu, first);
                painter.set_resource_fetch_host(self.fetch_host.clone());
                if let Some(wake) = self.image_waker.clone() {
                    painter.set_image_update_waker(wake);
                }
                painter
            });
            // Build every group's pipelines here, at the structural boundary,
            // rather than on its first frame.
            for &profile in &profiles {
                painter.select_presentation(profile);
            }
            painter.retain_presentations(&profiles);
        } else {
            self.painter = None;
        }
        self.planner.update(consumers);
        self.rebuild_output_traces(true);
        self.pending_plan_rebuilds += 1;
        self.work.record_output_work(OutputWorkObservation {
            plan_rebuilds: 1,
            ..Default::default()
        });
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_PLAN_REBUILDS);
        Ok(true)
    }

    /// Glyph counters of the one painter every group shares; `None` while no
    /// consumer is configured and there is no painter.
    pub fn text_glyph_counters(&self) -> Option<crate::TextGlyphCounters> {
        self.painter
            .as_ref()
            .map(crate::SceneWgpuPainter::text_glyph_counters)
    }

    /// Icon-atlas and URL-texture counters of that painter.
    pub fn painter_resource_counters(&self) -> Option<crate::ScenePainterResourceCounters> {
        self.painter
            .as_ref()
            .map(crate::SceneWgpuPainter::resource_counters)
    }

    pub fn plan(&self) -> &OutputPlan {
        self.planner.plan()
    }
    /// Aggregate of the latest producer call and subsequent `present_to`
    /// recordings, retained for compatibility. A final consumer pass is not
    /// producer work; use owner-attributed `output_traces` or capture this
    /// snapshot before consumer recording to evaluate producer idleness.
    /// Caller-owned consumer submits are not inferred from a recorded pass.
    pub fn work(&self) -> &WorkCounters {
        &self.work
    }

    /// Enable owner-attributed numeric snapshots. Allocation occurs here and
    /// at topology changes only. Disabled by default; no per-frame events.
    pub fn set_output_trace_enabled(&mut self, enabled: bool) {
        if enabled == self.traces.is_some() {
            return;
        }
        self.traces = enabled.then(Vec::new);
        self.rebuild_output_traces(false);
    }

    pub fn output_traces(&self) -> &[super::trace::OutputTraceSnapshot] {
        self.traces.as_deref().unwrap_or_default()
    }

    fn rebuild_output_traces(&mut self, configured: bool) {
        use super::trace::{
            OutputRecreateReason as R, OutputTraceRole as Role, OutputTraceSnapshot,
        };
        let Some(previous) = self.traces.as_mut() else {
            return;
        };
        let mut next = Vec::with_capacity(
            self.targets.len()
                + self
                    .planner
                    .plan()
                    .groups
                    .iter()
                    .map(|group| group.consumers.len())
                    .sum::<usize>(),
        );
        for (target, group) in self.targets.iter().zip(&self.planner.plan().groups) {
            let producer = target.surface.id().get();
            for (role, consumer, path, format) in std::iter::once((
                Role::Producer,
                OutputConsumerId(0),
                super::OutputPath::SharedTexture,
                target.requirements.profile.target_format,
            ))
            .chain(group.consumers.iter().map(|binding| {
                (
                    Role::Consumer,
                    binding.consumer,
                    binding.path,
                    binding.target_profile.target_format,
                )
            })) {
                let mut snapshot = previous
                    .iter()
                    .find(|old| {
                        old.producer == producer && old.consumer == consumer && old.role == role
                    })
                    .copied()
                    .unwrap_or_else(|| {
                        OutputTraceSnapshot::new(
                            producer,
                            consumer,
                            role,
                            path,
                            format,
                            target.requirements.extent,
                            self.gpu.generation(),
                        )
                    });
                snapshot.path = path;
                snapshot.format = format;
                snapshot.extent = target.requirements.extent;
                snapshot.device_generation = self.gpu.generation();
                snapshot.resource_generation = target.surface.resource_generation();
                if configured
                    && role == Role::Producer
                    && !previous
                        .iter()
                        .any(|old| old.producer == producer && old.role == role)
                {
                    let old = previous.iter().find(|old| {
                        old.role == Role::Consumer
                            && group
                                .consumers
                                .iter()
                                .any(|binding| binding.consumer == old.consumer)
                    });
                    snapshot.recreate_reason = match old {
                        None => R::Created,
                        Some(old) if old.device_generation != snapshot.device_generation => {
                            R::Device
                        }
                        Some(old) if old.extent != snapshot.extent => R::Extent,
                        Some(old) if old.format != snapshot.format => R::Format,
                        Some(_) => R::Requirements,
                    };
                    snapshot.work.target_recreates = target.surface.config().slots;
                }
                next.push(snapshot);
            }
        }
        *previous = next;
    }

    fn trace_outcomes(
        &mut self,
        outcomes: &[ExternalRenderOutcome],
        mut waits: Option<super::OutputTraceWait>,
        early_defer: bool,
    ) {
        use super::trace::OutputTraceRole as Role;
        let Some(traces) = self.traces.as_mut() else {
            return;
        };
        let mut owns_submission = self.work.output_submissions.unwrap_or(0) != 0;
        for (target, outcome) in self.targets.iter().zip(outcomes) {
            let producer = target.surface.id().get();
            for trace in traces.iter_mut().filter(|trace| trace.producer == producer) {
                trace.resource_generation = target.surface.resource_generation();
                if let ExternalRenderOutcome::Submitted {
                    content_revision, ..
                }
                | ExternalRenderOutcome::Reused {
                    content_revision, ..
                } = outcome
                {
                    trace.content_revision = *content_revision;
                }
                if trace.role == Role::Producer {
                    let mut work = if early_defer {
                        OutputWorkObservation {
                            deferred: 1,
                            ..Default::default()
                        }
                    } else {
                        target.surface.last_work()
                    };
                    if matches!(outcome, ExternalRenderOutcome::Deferred) {
                        trace.deferral_reason = if work.deferred == 0 && !early_defer {
                            super::trace::OutputTraceDeferral::FrameSlots
                        } else {
                            super::trace::OutputTraceDeferral::SourceSlots
                        };
                        work.deferred = work.deferred.max(1);
                    }
                    // A multi-group driver physically submits one frame. Its
                    // first submitted producer owns that shared submit count.
                    let acquisition = if owns_submission
                        && matches!(outcome, ExternalRenderOutcome::Submitted { .. })
                    {
                        work.submissions = 1;
                        owns_submission = false;
                        waits.take()
                    } else {
                        None
                    };
                    trace.observe(work, acquisition);
                }
            }
        }
    }

    /// Flush the same RuntimeDocument used by window hosts, then produce its
    /// retained output. Consumers of a presentation share one logical viewport.
    #[allow(clippy::too_many_arguments)]
    pub fn render_document(
        &mut self,
        document: &mut RuntimeDocument,
        logical_size: [f32; 2],
        host_revision: u64,
        clear_color: [f32; 4],
        textures: Option<&HostTextureRegistry>,
        renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<RuntimeFrameUpdate, PresentationError> {
        if !logical_size
            .iter()
            .all(|value| value.is_finite() && *value > 0.0)
        {
            return Err(PresentationError::InvalidViewport);
        }
        let update = flush_presentation(
            document,
            LayoutViewport::new(logical_size[0], logical_size[1]),
            &mut self.text,
        )
        .map_err(PresentationError::Runtime)?;
        self.render_scene(
            document.scene(),
            logical_size,
            host_revision,
            clear_color,
            textures,
            renderers,
        )?;
        Ok(update)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_scene(
        &mut self,
        scene: &UiScene,
        logical_size: [f32; 2],
        host_revision: u64,
        clear_color: [f32; 4],
        textures: Option<&HostTextureRegistry>,
        renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<Vec<ExternalRenderOutcome>, PresentationError> {
        if !logical_size
            .iter()
            .all(|value| value.is_finite() && *value > 0.0)
        {
            return Err(PresentationError::InvalidViewport);
        }
        self.work = WorkCounters::default();
        self.work.record_output_work(OutputWorkObservation {
            plan_rebuilds: std::mem::take(&mut self.pending_plan_rebuilds),
            target_recreates: std::mem::take(&mut self.pending_target_recreates),
            ..Default::default()
        });
        let mut outcomes = Vec::with_capacity(self.targets.len());
        let mut plans = Vec::new();
        let mut recording = None;
        let mut trace_submission = None;
        let renderer_revision = renderers.map(|registry| registry.retained_revision(scene));
        let (producers_used, producer_revision) = self
            .producers
            .as_ref()
            .map_or((false, Some(0)), |registry| {
                registry.retained_revision(scene)
            });
        let volatile =
            renderer_revision == Some(None) || (producers_used && producer_revision.is_none());
        if volatile {
            self.renderer_tick = self.renderer_tick.wrapping_add(1);
        }
        let producer_key = if producers_used {
            let mut producer_key = std::collections::hash_map::DefaultHasher::new();
            scene.instance_id().hash(&mut producer_key);
            self.resource_revision.hash(&mut producer_key);
            producer_revision.hash(&mut producer_key);
            producer_key.finish()
        } else {
            0
        };
        let mut prepared_resources = None;
        if !self.targets.is_empty()
            && producers_used
            && (producer_revision.is_none() || self.prepared_producers != Some(producer_key))
        {
            let mut writable = false;
            for target in &mut self.targets {
                if target.surface.has_record_capacity()? {
                    writable = true;
                    break;
                }
            }
            if !writable {
                self.work.record_output_work(OutputWorkObservation {
                    deferred: self.targets.len(),
                    ..Default::default()
                });
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::gpu::OUTPUT_DEFERRED,
                    self.targets.len() as u64
                );
                outcomes.resize(self.targets.len(), ExternalRenderOutcome::Deferred);
                self.trace_outcomes(&outcomes, None, true);
                return Ok(outcomes);
            }
            let Some(mut frame) = self.gpu.try_begin_frame("NanaUI presentation resources") else {
                self.work.record_output_work(OutputWorkObservation {
                    deferred: self.targets.len(),
                    ..Default::default()
                });
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::gpu::OUTPUT_DEFERRED,
                    self.targets.len() as u64
                );
                outcomes.resize(self.targets.len(), ExternalRenderOutcome::Deferred);
                self.trace_outcomes(&outcomes, None, true);
                return Ok(outcomes);
            };
            prepared_resources = Some(
                self.producers
                    .as_ref()
                    .expect("scene uses an installed resource producer")
                    .encode_scene(scene, &mut frame)
                    .map_err(PresentationError::ResourceProduction)?,
            );
            recording = Some(frame);
        }
        for (index, target) in self.targets.iter_mut().enumerate() {
            let extent = target.requirements.extent;
            let scale =
                (extent[0] as f32 / logical_size[0]).min(extent[1] as f32 / logical_size[1]);
            let size = [extent[0] as f32 / scale, extent[1] as f32 / scale];
            let viewport = ScenePaintViewport {
                logical_size: size,
                physical_size: extent,
                scale_factor: scale,
                scene_origin: [0.0; 2],
                target_origin: [
                    (size[0] - logical_size[0]) * 0.5,
                    (size[1] - logical_size[1]) * 0.5,
                ],
                clear_color,
                clear: true,
            };
            let painter = self.painter.as_mut().expect("configured painter");
            let image_revision =
                frame_driver::PresentationFrameDriver::refresh_image_revision(painter);
            let revision = if renderer_revision.is_none()
                && !producers_used
                && self.resource_revision == 0
                && image_revision == 0
            {
                host_revision
            } else {
                let mut revision = std::collections::hash_map::DefaultHasher::new();
                host_revision.hash(&mut revision);
                renderer_revision.hash(&mut revision);
                producer_revision.hash(&mut revision);
                self.resource_revision.hash(&mut revision);
                image_revision.hash(&mut revision);
                if volatile {
                    self.renderer_tick.hash(&mut revision);
                }
                revision.finish()
            };
            let prepared = frame_driver::PresentationFrameDriver::prepare(
                scene,
                None,
                Some((
                    &mut target.surface,
                    frame_driver::RetainedPresentation {
                        revision,
                        viewport,
                        alpha: match target.requirements.alpha {
                            OutputAlpha::GammaPremultiplied => AlphaEncoding::Gamma,
                            _ => AlphaEncoding::Linear,
                        },
                        parameters: Default::default(),
                    },
                )),
                Some(self.planner.plan()),
                textures,
            )?;
            if let Some(outcome) = prepared.outcome {
                outcomes.push(outcome);
                continue;
            }
            outcomes.push(ExternalRenderOutcome::Deferred);
            if recording.is_none() {
                recording = self.gpu.try_begin_frame("NanaUI presentation");
            }
            if let Some(frame) = recording.as_mut() {
                let encoded = frame_driver::PresentationFrameDriver::encode(
                    prepared,
                    scene,
                    None,
                    frame,
                    &self.gpu,
                    &mut self.painter,
                    Some(&mut target.surface),
                    None,
                    &mut None,
                    textures,
                    renderers,
                    self.fetch_host.clone(),
                )?;
                if let Some(work) = encoded.gpu_work {
                    self.work.record_gpu_work(work);
                }
                if let Some(recorded) = encoded.retained {
                    plans.push((index, recorded));
                }
            } else {
                self.work.record_output_work(OutputWorkObservation {
                    deferred: 1,
                    ..Default::default()
                });
                nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_DEFERRED);
            }
        }
        if let Some(frame) = recording {
            let submission = frame.submit();
            if self.traces.is_some() {
                trace_submission = Some(super::OutputTraceWait::from_submission(&submission));
            }
            if let Some(prepared) = prepared_resources {
                prepared.submitted(&submission);
                self.prepared_producers = Some(producer_key);
            }
            for (index, recorded) in plans {
                let completed = frame_driver::PresentationFrameDriver::finish(
                    &mut self.targets[index].surface,
                    recorded,
                    &submission,
                    false,
                )?;
                debug_assert!(completed.lease.is_none());
                outcomes[index] = completed.outcome;
            }
            self.work.record_output_work(OutputWorkObservation {
                submissions: 1,
                ..Default::default()
            });
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_SUBMISSIONS);
        }
        for target in &self.targets {
            target.surface.record_last_work(&mut self.work);
        }
        self.trace_outcomes(&outcomes, trace_submission, false);
        Ok(outcomes)
    }

    /// Lease the canonical resource. Metadata on the frame describes its
    /// actual encoding; use `present_to` for a consumer needing final transfer.
    pub fn sample(
        &mut self,
        consumer: OutputConsumerId,
    ) -> Result<ExternalFrame, PresentationError> {
        let index = self.group_index(consumer)?;
        Ok(self.targets[index].surface.sample()?)
    }

    pub fn poll(&mut self) {
        for target in &mut self.targets {
            target.surface.poll();
        }
    }

    /// Record only the final transfer into a host-provided destination. The
    /// sampled canonical lease automatically survives the consuming submission.
    pub fn present_to(
        &mut self,
        consumer: OutputConsumerId,
        frame: &mut FrameContext,
        destination: &GpuRenderTarget,
    ) -> Result<(), PresentationError> {
        let index = self.group_index(consumer)?;
        let binding = *self
            .planner
            .plan()
            .binding(consumer)
            .ok_or(PresentationError::UnsupportedConsumer(consumer))?;
        if destination.format() != binding.target_profile.target_format {
            return Err(PresentationError::UnsupportedConsumer(consumer));
        }
        if self
            .targets
            .iter()
            .any(|target| target.surface.owns_target(destination))
        {
            return Err(PresentationError::Surface(ExternalSurfaceError::Gpu(
                nana_gpu::GpuError::UnsupportedCapability(
                    "presentation destination belongs to a canonical target pool",
                ),
            )));
        }
        let target = &mut self.targets[index];
        let lease = target.surface.sample()?;
        let source = lease.texture()?;
        let srgb = source.format().is_srgb();
        let transfer = match target
            .transfers
            .iter()
            .position(|transfer| transfer.matches(&self.gpu, destination.format(), srgb))
        {
            Some(index) => &mut target.transfers[index],
            None => {
                target
                    .transfers
                    .push(OutputTransfer::new(&self.gpu, destination.format(), srgb));
                target.transfers.last_mut().expect("inserted transfer")
            }
        };
        transfer
            .record(frame, source, destination)
            .map_err(ExternalSurfaceError::Gpu)?;
        let consumer_work = OutputWorkObservation {
            extra_passes: 1,
            gpu_convert_passes: 1,
            ..Default::default()
        };
        super::trace::consumer_metrics(consumer_work, None);
        if let Some(trace) = self.traces.as_mut().and_then(|traces| {
            traces.iter_mut().find(|trace| {
                trace.role == super::trace::OutputTraceRole::Consumer && trace.consumer == consumer
            })
        }) {
            trace.producer = lease.provenance().source().get();
            trace.resource_generation = lease.resource_generation();
            trace.content_revision = lease.content_revision();
            // Emit metrics exactly once above, independent of snapshot mode.
            trace.observe_without_metrics(consumer_work, None);
        }
        frame.keep_alive_until_complete(lease);
        self.work.record_output_work(consumer_work);
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_EXTRA_PASSES);
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_GPU_CONVERT_PASSES);
        Ok(())
    }

    fn group_index(&self, consumer: OutputConsumerId) -> Result<usize, PresentationError> {
        self.planner
            .plan()
            .groups
            .iter()
            .position(|group| {
                group
                    .consumers
                    .iter()
                    .any(|binding| binding.consumer == consumer)
            })
            .ok_or(PresentationError::UnsupportedConsumer(consumer))
    }
}
