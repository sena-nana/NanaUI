//! Shared presentation orchestration. Native presenters supply targets and
//! pacing; both native and windowless hosts use this target selection,
//! encoding and submission settlement authority.

use std::collections::HashMap;

use nana_gpu::{FrameContext, GpuContext, GpuRenderTarget, GpuSubmission, GpuTexture};
use nana_ui_core::GpuWorkObservation;
use nana_ui_platform::SharedFetchHost;
use nana_ui_scene::UiScene;

use super::super::transfer::OutputTransfer;
use super::super::{
    ExternalFrame, ExternalFramePlan, ExternalPrepare, ExternalRenderOutcome, ExternalSurface,
    ExternalSurfaceError, OutputPlan,
};
use crate::scene_paint::SubpixelOrder;
use crate::{
    AlphaEncoding, HostTextureRegistry, RenderTargetId, SceneGpuRendererRegistry,
    ScenePaintViewport, ScenePresentationParameters, ScenePresentationProfile, SceneWgpuPainter,
};

/// Where [`PresentationFrameDriver::encode`] finds the painter of a profile.
pub(crate) trait PresentationPainters {
    fn painter(
        &mut self,
        gpu: &GpuContext,
        profile: ScenePresentationProfile,
    ) -> &mut SceneWgpuPainter;
}

/// A painter per profile.
impl PresentationPainters for HashMap<ScenePresentationProfile, SceneWgpuPainter> {
    fn painter(
        &mut self,
        gpu: &GpuContext,
        profile: ScenePresentationProfile,
    ) -> &mut SceneWgpuPainter {
        self.entry(profile)
            .or_insert_with(|| SceneWgpuPainter::new_with_presentation(gpu, profile))
    }
}

/// One painter for every profile: the glyph and icon atlases, text caches
/// and URL images are shared, and each working format keeps its pipelines.
impl PresentationPainters for Option<SceneWgpuPainter> {
    fn painter(
        &mut self,
        gpu: &GpuContext,
        profile: ScenePresentationProfile,
    ) -> &mut SceneWgpuPainter {
        let painter =
            self.get_or_insert_with(|| SceneWgpuPainter::new_with_presentation(gpu, profile));
        painter.select_presentation(profile);
        painter
    }
}

#[derive(Clone, Copy)]
pub(crate) struct DirectPresentation {
    pub id: RenderTargetId,
    pub profile: ScenePresentationProfile,
    pub viewport: ScenePaintViewport,
    pub alpha: AlphaEncoding,
    pub text_aa: Option<SubpixelOrder>,
    pub parameters: ScenePresentationParameters,
}

#[derive(Clone, Copy)]
pub(crate) struct RetainedPresentation {
    pub revision: u64,
    pub viewport: ScenePaintViewport,
    pub alpha: AlphaEncoding,
    pub parameters: ScenePresentationParameters,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PresentationRoute {
    Idle,
    Direct,
    Retained,
    Shared,
    Separate,
}

pub(crate) struct PreparedPresentation {
    pub route: PresentationRoute,
    direct: Option<DirectPresentation>,
    retained: Option<(ExternalFramePlan, RetainedPresentation)>,
    pub outcome: Option<ExternalRenderOutcome>,
}

impl PreparedPresentation {
    #[cfg(any(feature = "hosted", test))]
    pub fn retained(plan: ExternalFramePlan, request: RetainedPresentation) -> Self {
        Self {
            route: PresentationRoute::Retained,
            direct: None,
            retained: Some((plan, request)),
            outcome: None,
        }
    }

    pub fn with_direct(
        mut self,
        direct: Option<DirectPresentation>,
        plan: Option<&OutputPlan>,
    ) -> Self {
        self.direct = direct;
        self.route = match (self.direct.is_some(), self.retained.is_some()) {
            (false, false) => PresentationRoute::Idle,
            (true, false) => PresentationRoute::Direct,
            (false, true) => PresentationRoute::Retained,
            (true, true)
                if plan.is_some_and(|plan| plan.groups.len() == 1 && plan.render_count() == 1) =>
            {
                PresentationRoute::Shared
            }
            (true, true) => PresentationRoute::Separate,
        };
        self
    }

    #[cfg(any(feature = "hosted", test))]
    pub fn into_retained(self) -> Option<(ExternalFramePlan, RetainedPresentation)> {
        self.retained
    }
}

pub(crate) struct RecordedRetained {
    pub plan: ExternalFramePlan,
    pub texture: GpuTexture,
}

pub(crate) struct RecordedPresentation {
    pub retained: Option<RecordedRetained>,
    pub retained_error: Option<ExternalSurfaceError>,
    pub direct_ui: bool,
    pub shared: bool,
    pub gpu_work: Option<GpuWorkObservation>,
}

pub(crate) fn accumulate_gpu_work(
    total: &mut Option<GpuWorkObservation>,
    work: Option<GpuWorkObservation>,
) {
    if let Some(work) = work {
        let total = total.get_or_insert_with(GpuWorkObservation::default);
        total.batch_rebuilds = total.batch_rebuilds.saturating_add(work.batch_rebuilds);
        total.draw_batches = total.draw_batches.saturating_add(work.draw_batches);
        total.draw_calls = total.draw_calls.saturating_add(work.draw_calls);
        total.gpu_upload_bytes = total.gpu_upload_bytes.saturating_add(work.gpu_upload_bytes);
        total.gpu_buffer_reallocations = total
            .gpu_buffer_reallocations
            .saturating_add(work.gpu_buffer_reallocations);
    }
}

pub(crate) struct CompletedPresentation {
    pub outcome: ExternalRenderOutcome,
    pub lease: Option<ExternalFrame>,
}

pub(crate) struct PresentationFrameDriver;

impl PresentationFrameDriver {
    /// Advance asynchronous resources before any retained dirty decision.
    pub fn refresh_image_revision(painter: &mut SceneWgpuPainter) -> u64 {
        painter.refresh_image_resources();
        painter.image_revision()
    }

    /// No direct-only allocation or planner dispatch: the prepared direct
    /// target is a stack value. Retained dirty selection lives here for all
    /// hosts, and sharing is selected from their cached consumer topology.
    pub fn prepare(
        scene: &UiScene,
        direct: Option<DirectPresentation>,
        retained: Option<(&mut ExternalSurface, RetainedPresentation)>,
        plan: Option<&OutputPlan>,
        textures: Option<&HostTextureRegistry>,
    ) -> Result<PreparedPresentation, ExternalSurfaceError> {
        let mut prepared = PreparedPresentation {
            route: if direct.is_some() {
                PresentationRoute::Direct
            } else {
                PresentationRoute::Idle
            },
            direct,
            retained: None,
            outcome: None,
        };
        if let Some((surface, request)) = retained {
            match surface.prepare(scene, request.revision, request.viewport, textures)? {
                ExternalPrepare::Record(record) => {
                    prepared.retained = Some((record, request));
                }
                ExternalPrepare::Reused {
                    resource_generation,
                    content_revision,
                } => {
                    prepared.outcome = Some(ExternalRenderOutcome::Reused {
                        resource_generation,
                        content_revision,
                    });
                }
                ExternalPrepare::Deferred => {
                    prepared.outcome = Some(ExternalRenderOutcome::Deferred)
                }
            }
        }
        Ok(prepared.with_direct(direct, plan))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        prepared: PreparedPresentation,
        scene: &UiScene,
        retained_scene: Option<&UiScene>,
        frame: &mut FrameContext,
        gpu: &GpuContext,
        painters: &mut impl PresentationPainters,
        mut surface: Option<&mut ExternalSurface>,
        direct_target: Option<&GpuRenderTarget>,
        transfer: &mut Option<OutputTransfer>,
        textures: Option<&HostTextureRegistry>,
        renderers: Option<&SceneGpuRendererRegistry>,
        fetch_host: Option<SharedFetchHost>,
    ) -> Result<RecordedPresentation, ExternalSurfaceError> {
        let mut recorded = RecordedPresentation {
            retained: None,
            retained_error: None,
            direct_ui: false,
            shared: prepared.route == PresentationRoute::Shared,
            gpu_work: None,
        };
        if matches!(
            prepared.route,
            PresentationRoute::Direct | PresentationRoute::Separate
        ) {
            let work = Self::encode_direct(
                prepared.direct.expect("direct route has a target"),
                scene,
                frame,
                gpu,
                painters,
                direct_target.expect("presenter supplied its direct target"),
                textures,
                renderers,
                fetch_host.clone(),
            )?;
            recorded.direct_ui = true;
            accumulate_gpu_work(&mut recorded.gpu_work, work);
        }
        if let Some((plan, request)) = prepared.retained {
            let surface = surface.as_mut().expect("retained route has a producer");
            let profile = surface.config().presentation;
            let painter = painters.painter(gpu, profile);
            painter.set_resource_fetch_host(fetch_host.clone());
            let result = Self::encode_retained(
                surface,
                plan,
                request,
                painter,
                retained_scene.unwrap_or(scene),
                frame,
                if retained_scene.is_some() {
                    None
                } else {
                    textures
                },
                if retained_scene.is_some() {
                    None
                } else {
                    renderers
                },
            );
            match result {
                Ok(retained) => {
                    recorded.retained = Some(retained);
                    accumulate_gpu_work(&mut recorded.gpu_work, painter.last_gpu_work());
                }
                Err(error) if prepared.direct.is_some() => {
                    recorded.retained_error = Some(error);
                    recorded.shared = false;
                    if !recorded.direct_ui {
                        let work = Self::encode_direct(
                            prepared.direct.expect("presenter has a direct fallback"),
                            scene,
                            frame,
                            gpu,
                            painters,
                            direct_target.expect("presenter supplied its direct target"),
                            textures,
                            renderers,
                            fetch_host,
                        )?;
                        recorded.direct_ui = true;
                        accumulate_gpu_work(&mut recorded.gpu_work, work);
                    }
                }
                Err(error) => return Err(error),
            }
        }
        if recorded.shared {
            let source = &recorded
                .retained
                .as_ref()
                .expect("shared route recorded canonical content")
                .texture;
            let target = direct_target.expect("shared presenter supplied its target");
            if !transfer.as_ref().is_some_and(|transfer| {
                transfer.matches(gpu, target.format(), source.format().is_srgb())
            }) {
                *transfer = Some(OutputTransfer::new(
                    gpu,
                    target.format(),
                    source.format().is_srgb(),
                ));
            }
            transfer
                .as_mut()
                .expect("transfer exists")
                .record(frame, source, target)?;
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_EXTRA_PASSES);
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_GPU_CONVERT_PASSES);
            accumulate_gpu_work(
                &mut recorded.gpu_work,
                Some(GpuWorkObservation {
                    draw_batches: 1,
                    draw_calls: 1,
                    ..Default::default()
                }),
            );
        }
        Ok(recorded)
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_direct(
        direct: DirectPresentation,
        scene: &UiScene,
        frame: &mut FrameContext,
        gpu: &GpuContext,
        painters: &mut impl PresentationPainters,
        target: &GpuRenderTarget,
        textures: Option<&HostTextureRegistry>,
        renderers: Option<&SceneGpuRendererRegistry>,
        fetch_host: Option<SharedFetchHost>,
    ) -> Result<Option<GpuWorkObservation>, ExternalSurfaceError> {
        let painter = painters.painter(gpu, direct.profile);
        painter.set_resource_fetch_host(fetch_host);
        painter.set_alpha_encoding(direct.alpha);
        painter.set_subpixel_text(direct.text_aa);
        painter.set_presentation_parameters(direct.parameters);
        painter.paint_target(
            direct.id,
            scene,
            frame,
            target,
            direct.viewport,
            textures,
            renderers,
        )?;
        Ok(painter.last_gpu_work())
    }

    /// Only the frame that actually encoded a reservation can publish it.
    /// Both presenters and headless drivers settle through this same seam.
    pub fn finish(
        surface: &mut ExternalSurface,
        recorded: RecordedRetained,
        submission: &GpuSubmission,
        lease: bool,
    ) -> Result<CompletedPresentation, ExternalSurfaceError> {
        let outcome = surface.bind_submission(recorded.plan, submission)?;
        let lease = if lease {
            Some(surface.lease_submitted(submission)?)
        } else {
            None
        };
        Ok(CompletedPresentation { outcome, lease })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode_retained(
        surface: &mut ExternalSurface,
        plan: ExternalFramePlan,
        request: RetainedPresentation,
        painter: &mut SceneWgpuPainter,
        scene: &UiScene,
        frame: &mut FrameContext,
        textures: Option<&HostTextureRegistry>,
        renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<RecordedRetained, ExternalSurfaceError> {
        painter.set_subpixel_text(None);
        painter.set_alpha_encoding(request.alpha);
        painter.set_presentation_parameters(request.parameters);
        if let Err(error) = surface.record(
            &plan,
            frame,
            Some(painter),
            scene,
            request.viewport,
            textures,
            renderers,
        ) {
            // A failed paint may have encoded partial target work. Keep its
            // reservation until this frame completes (or is discarded), but
            // never return a publishable plan to the presenter.
            frame.keep_alive_until_complete(plan);
            return Err(error);
        }
        Ok(RecordedRetained {
            texture: surface.plan_texture(&plan)?.clone(),
            plan,
        })
    }
}
