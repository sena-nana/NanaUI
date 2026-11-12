//! Retained window outputs, recorded in the host's frame. Compatible windows
//! consume the canonical output through a final transfer; incompatible pixel
//! contracts retain separate rasterizations.

use super::*;
use crate::output::window_output::{OutputStep, RecordedOutput, WindowOutputState, output_profile};
use crate::runtime_host::FrameDemand;

impl<Program: RuntimeProgram> WindowManager<Program> {
    /// Applies the program's current answer for `id`'s output and delivers
    /// whatever that changed. Returns whether an output is configured.
    pub(super) fn sync_window_output(&mut self, id: WindowId) -> bool {
        let config = self.program.window_output(id);
        if config.is_none() && !self.window_outputs.contains_key(&id) {
            // Establish an unpublished direct owner before acquisition so an
            // initial skip/failure is observable without inventing a frame.
            self.ensure_direct_output_trace(id);
            return false;
        }
        self.window_outputs
            .entry(id)
            .or_insert_with(|| WindowOutputState::new(id))
            .set_config(config);
        let tracing = self.program.output_trace_enabled(id);
        self.window_outputs
            .get_mut(&id)
            .expect("inserted output state")
            .set_output_trace_enabled(tracing);
        self.flush_window_output(id);
        if config.is_none() {
            self.window_outputs.remove(&id);
            self.ensure_direct_output_trace(id);
            self.retain_active_presentations();
        }
        config.is_some()
    }

    /// What `id`'s output adds to the window's frame demand: a cadence while
    /// it cannot present, or the instant a throttled frame comes due.
    pub(super) fn window_output_demand(&self, id: WindowId, can_present: bool) -> FrameDemand {
        let configured = self.program.window_output(id);
        let state = self.window_outputs.get(&id);
        match (configured, state) {
            (Some(config), _) if !can_present && config.while_hidden && !config.suspended => {
                FrameDemand::Continuous(
                    config
                        .max_fps
                        .unwrap_or(crate::output::window_output::WINDOW_OUTPUT_HIDDEN_FPS),
                )
            }
            (Some(_), Some(state)) => state.frame_demand(can_present),
            _ => FrameDemand::OnDemand,
        }
    }

    /// Whether `id`'s output has a new frame to record for `scene`. `None`
    /// when the window has no output.
    pub(super) fn plan_window_output(
        &mut self,
        id: WindowId,
        scene: &nana_ui_scene::UiScene,
        host_textures: Option<&crate::HostTextureRegistry>,
        gpu_renderers: Option<&crate::SceneGpuRendererRegistry>,
        producers: Option<&crate::SceneResourceProducerRegistry>,
        clear_color: [f32; 4],
    ) -> Option<OutputStep> {
        if !self.window_outputs.contains_key(&id) {
            return None;
        }
        let geometry = self.content_geometry_of(id);
        // Unknown custom renderer versions require invalidation. The shared
        // render key tracks actual compositor clocks and presentation epochs.
        let renderer_revision = gpu_renderers.map(|registry| registry.retained_revision(scene));
        let producer_revision = producers
            .map(|registry| registry.retained_revision(scene))
            .filter(|(used, _)| *used)
            .map(|(_, revision)| revision);
        let volatile = renderer_revision.is_some_and(|revision| revision.is_none())
            || producer_revision == Some(None);
        let painter = self.painter_mut(output_profile());
        let image_revision =
            crate::output::driver::frame_driver::PresentationFrameDriver::refresh_image_revision(
                painter,
            );
        let mut host_revision = image_revision
            ^ self.surface_generation.rotate_left(32)
            ^ renderer_revision.flatten().unwrap_or(0).rotate_left(11);
        if let Some(revision) = producer_revision {
            use std::hash::{Hash, Hasher};
            let mut key = std::collections::hash_map::DefaultHasher::new();
            host_revision.hash(&mut key);
            revision.hash(&mut key);
            host_revision = key.finish();
        }
        let gpu = self.graphics.gpu().clone();
        let state = self.window_outputs.get_mut(&id)?;
        Some(state.begin(
            &gpu,
            &geometry,
            scene,
            host_revision,
            volatile,
            host_textures,
            clear_color,
            Instant::now(),
        ))
    }

    /// Supply native presenter resources to the same orchestration authority
    /// used by windowless output. This adapter does not choose a render route.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode_presentation(
        &mut self,
        id: WindowId,
        step: Option<OutputStep>,
        direct: Option<crate::output::driver::frame_driver::DirectPresentation>,
        scene: &nana_ui_scene::UiScene,
        frame: &mut nana_gpu::FrameContext,
        target: Option<&nana_gpu::GpuRenderTarget>,
        host_textures: Option<&crate::HostTextureRegistry>,
        gpu_renderers: Option<&crate::SceneGpuRendererRegistry>,
        fetch_host: Option<nana_ui_platform::SharedFetchHost>,
    ) -> Result<
        (
            Option<RecordedOutput>,
            crate::output::driver::frame_driver::RecordedPresentation,
        ),
        crate::ExternalSurfaceError,
    > {
        use crate::output::driver::frame_driver::PresentationFrameDriver;
        if let Some(direct) = direct {
            self.painter_mut(direct.profile);
        }
        if self.window_outputs.contains_key(&id) {
            self.painter_mut(output_profile());
        }
        let gpu = self.graphics.gpu();
        if let Some(state) = self.window_outputs.get_mut(&id) {
            state.encode_presentation(
                step,
                direct,
                scene,
                frame,
                gpu,
                &mut self.painter,
                target,
                host_textures,
                gpu_renderers,
                fetch_host,
            )
        } else {
            let prepared =
                PresentationFrameDriver::prepare(scene, direct, None, None, host_textures)?;
            let encoded = PresentationFrameDriver::encode(
                prepared,
                scene,
                None,
                frame,
                gpu,
                &mut self.painter,
                None,
                target,
                &mut None,
                host_textures,
                gpu_renderers,
                fetch_host,
            )?;
            Ok((None, encoded))
        }
    }

    /// Publishes a recorded output once its frame was submitted.
    pub(super) fn complete_window_output(
        &mut self,
        id: WindowId,
        recorded: Option<RecordedOutput>,
        submission: &nana_gpu::GpuSubmission,
    ) -> Option<crate::WindowOutputFrame> {
        let recorded = recorded?;
        self.window_outputs
            .get_mut(&id)?
            .complete(recorded, submission, Instant::now())
    }

    /// Hands a produced frame and any status changes to the program.
    pub(super) fn deliver_window_output(
        &mut self,
        id: WindowId,
        frame: Option<crate::WindowOutputFrame>,
    ) {
        if let Some(frame) = frame
            && self.window_contexts.contains_key(&id)
        {
            self.program
                .window_output_frame(id, &frame, &self.context_for(id));
            if frame.resend_requested()
                && let Some(state) = self.window_outputs.get_mut(&id)
            {
                state.resend(Instant::now());
            }
        }
        self.flush_window_output(id);
        self.deliver_output_traces(id);
    }

    /// Copy observations to the host. Publishing log events remains an
    /// explicit diagnostic request outside the frame loop.
    pub(super) fn deliver_output_traces(&mut self, id: WindowId) {
        if !self.program.output_trace_enabled(id) || !self.window_contexts.contains_key(&id) {
            return;
        }
        if let Some(trace) = self.direct_output_traces.get(&id).copied() {
            self.program
                .window_output_trace(id, trace, &self.context_for(id));
        }
        let traces = self
            .window_outputs
            .get(&id)
            .map(|state| state.output_traces())
            .unwrap_or([None; 3]);
        for trace in traces.into_iter().flatten() {
            self.program
                .window_output_trace(id, trace, &self.context_for(id));
        }
    }

    pub(super) fn fail_output_trace(&mut self, id: WindowId) {
        if let Some(trace) = self.direct_output_traces.get_mut(&id) {
            trace.state = crate::output::OutputTraceState::Failed;
            trace.deferral_reason = crate::output::OutputTraceDeferral::None;
            trace.waits = None;
        }
        if let Some(state) = self.window_outputs.get_mut(&id) {
            state.trace_failed();
        }
        self.deliver_output_traces(id);
    }

    pub(super) fn defer_output_trace(
        &mut self,
        id: WindowId,
        reason: crate::output::OutputTraceDeferral,
    ) {
        if let Some(trace) = self.direct_output_traces.get_mut(&id) {
            trace.observe(
                nana_ui_core::OutputWorkObservation {
                    deferred: 1,
                    ..Default::default()
                },
                None,
            );
            trace.deferral_reason = reason;
        }
        if let Some(state) = self.window_outputs.get_mut(&id) {
            state.trace_deferred(reason);
        }
        self.deliver_output_traces(id);
    }

    fn ensure_direct_output_trace(&mut self, id: WindowId) {
        use crate::output::{OutputConsumerId, OutputPath, OutputTraceRole, OutputTraceSnapshot};
        if !self.program.output_trace_enabled(id) {
            self.direct_output_traces.remove(&id);
            return;
        }
        if self.direct_output_traces.contains_key(&id) {
            return;
        }
        let Some(host) = self.window_contexts.get(&id) else {
            return;
        };
        let profile = host.surface.profile().scene_profile();
        let extent = self.geometry_of(id).physical_size;
        self.direct_output_traces.insert(
            id,
            OutputTraceSnapshot::new(
                crate::output::OutputSurfaceId::next().get(),
                OutputConsumerId(1),
                OutputTraceRole::Producer,
                OutputPath::DirectSurface,
                profile.target_format,
                [extent.0, extent.1],
                self.graphics.gpu().generation(),
            ),
        );
    }

    pub(super) fn observe_direct_output_trace(
        &mut self,
        id: WindowId,
        profile: crate::ScenePresentationProfile,
        extent: [u32; 2],
        waits: Option<crate::output::OutputTraceWait>,
    ) {
        self.ensure_direct_output_trace(id);
        let device = self.graphics.gpu().generation();
        let Some(trace) = self.direct_output_traces.get_mut(&id) else {
            return;
        };
        trace.extent = extent;
        trace.format = profile.target_format;
        trace.device_generation = device;
        trace.resource_generation = self.surface_generation.saturating_add(1);
        trace.content_revision = trace.content_revision.saturating_add(1);
        trace.state = crate::output::OutputTraceState::Submitted;
        trace.deferral_reason = crate::output::OutputTraceDeferral::None;
        trace.observe(
            nana_ui_core::OutputWorkObservation {
                ui_renders: 1,
                submissions: 1,
                ..Default::default()
            },
            waits,
        );
    }

    fn flush_window_output(&mut self, id: WindowId) {
        let Some(state) = self.window_outputs.get_mut(&id) else {
            return;
        };
        let targets = state.take_retired_targets();
        let statuses = state.take_statuses();
        if let Some(painter) = self.painter.as_mut() {
            for target in targets {
                painter.remove_target(target);
            }
        }
        for status in statuses {
            if !self.window_contexts.contains_key(&id) {
                break;
            }
            self.program
                .window_output_status(id, status, &self.context_for(id));
        }
    }

    /// The device is being replaced: every output resource is on the old one.
    pub(super) fn retire_window_outputs(&mut self) {
        let ids: Vec<_> = self.window_outputs.keys().copied().collect();
        for id in ids {
            if let Some(state) = self.window_outputs.get_mut(&id) {
                state.device_replaced();
            }
            self.flush_window_output(id);
        }
    }

    /// `id` closed: its output goes without a report.
    pub(super) fn close_window_output(&mut self, id: WindowId) {
        let Some(state) = self.window_outputs.remove(&id) else {
            return;
        };
        if let Some(painter) = self.painter.as_mut() {
            for target in state.target_ids() {
                painter.remove_target(target);
            }
        }
        self.retain_active_presentations();
    }

    /// A window that cannot present but keeps its output: flush its document
    /// and paint only the output, in a frame of its own. Returns whether a
    /// frame was recorded.
    pub(super) fn tick_hidden_output(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        producers: Option<crate::SceneResourceProducerRegistry>,
    ) -> bool {
        let queued = self.drain_program_messages(id);
        self.apply_update(event_loop, queued, Some(id));
        if event_loop.exiting() || !self.window_contexts.contains_key(&id) {
            return false;
        }
        // The output is all that draws this window's compositor motion while
        // it is hidden, so its clock advances here as it does in a present.
        // Without it a glyph reveal is painted where the last present left
        // the clock: the newest glyph stays at the start of its fade.
        self.sync_compositor_clock(id);
        let geometry = self.content_geometry_of(id);
        let viewport = LayoutViewport::new(geometry.logical_size.0, geometry.logical_size.1);
        let Some(flush) = self.program.write_document(id, |document| {
            crate::output::driver::flush_presentation(document, viewport, &mut self.text)
        }) else {
            return false;
        };
        let update = match flush {
            Ok(update) => update,
            Err(error) => {
                self.program
                    .report_host_failure(HostFailure::FrameDidNotSettle {
                        window: id,
                        error: error.to_string(),
                    });
                return false;
            }
        };
        if let Some(pending) = self.accessibility_pending_mut(id) {
            pending.stage(update.accessibility);
        }
        let Some(scene) = self
            .program
            .read_document(id, |document| document.shared_scene())
        else {
            return false;
        };
        self.update_image_targets(id, scene.as_ref());
        let producer_frame = self.producer_frames.prepare(
            id,
            self.graphics.gpu().generation().get(),
            producers.as_ref().map_or((false, Some(0)), |registry| {
                registry.retained_revision(scene.as_ref())
            }),
        );
        let host_textures = self.program.host_textures(id);
        let gpu_renderers = self.program.scene_gpu_renderers(id);
        let theme = self.program.theme();
        let window_background = self.program.window_background();
        let material = self.material_of(id);
        let clear_color =
            scene_paint_viewport(&geometry, material, theme.as_ref(), window_background)
                .clear_color;
        let step = self.plan_window_output(
            id,
            scene.as_ref(),
            host_textures.as_ref(),
            gpu_renderers.as_ref(),
            producers.as_ref(),
            clear_color,
        );
        if producer_frame.is_none() && !matches!(step, Some(OutputStep::Record { .. })) {
            // An unchanged output and no producers: nothing to record.
            if let Some(state) = self.window_outputs.get(&id) {
                let work = state.last_work();
                self.program.write_document(id, |document| {
                    let context = document.context_mut();
                    context.record_output_work(work);
                    context.record_presentation_gpu_work(None);
                });
            }
            self.flush_window_output(id);
            self.deliver_output_traces(id);
            return true;
        }
        let trace_enabled = self.program.output_trace_enabled(id);
        let fetch_host = self.program.resource_fetch_host(id);
        let Some(mut frame) = self
            .graphics
            .gpu()
            .try_begin_frame("NanaUI hidden window output")
        else {
            // Dropping `step` releases its unrecorded reservation. A busy
            // queue schedules a later attempt without blocking this host.
            drop(step);
            if let Some(state) = self.window_outputs.get_mut(&id) {
                state.defer_frame(Instant::now());
                let work = state.last_work();
                self.program.write_document(id, |document| {
                    let context = document.context_mut();
                    context.record_output_work(work);
                    context.record_presentation_gpu_work(None);
                });
            }
            self.retry_frame_soon(id);
            self.deliver_output_traces(id);
            return false;
        };
        let prepared = match producers.filter(|_| producer_frame.is_some()) {
            Some(producers) => match producers.encode_scene(scene.as_ref(), &mut frame) {
                Ok(prepared) => Some(prepared),
                Err(error) => {
                    drop(frame);
                    self.fail_output_trace(id);
                    self.program
                        .report_host_failure(HostFailure::ResourceProduction {
                            window: id,
                            error: error.to_string(),
                        });
                    return false;
                }
            },
            None => None,
        };
        let (recorded, gpu_work) = match self.encode_presentation(
            id,
            step,
            None,
            scene.as_ref(),
            &mut frame,
            None,
            host_textures.as_ref(),
            gpu_renderers.as_ref(),
            fetch_host,
        ) {
            Ok((recorded, encoded)) => (recorded, encoded.gpu_work),
            Err(error) => {
                drop(frame);
                self.fail_output_trace(id);
                self.program
                    .report_host_failure(HostFailure::UnpaintableScene {
                        window: id,
                        error: error.to_string(),
                    });
                return false;
            }
        };
        let submission = frame.submit();
        if let Some(producer_frame) = producer_frame {
            self.producer_frames.submitted(producer_frame);
        }
        if let Some(prepared) = prepared {
            prepared.submitted(&submission);
        }
        let trace_waits =
            trace_enabled.then(|| crate::output::OutputTraceWait::from_submission(&submission));
        if let Some(state) = self.window_outputs.get_mut(&id) {
            state.trace_host_submission(false, trace_waits);
        }
        let output = self.complete_window_output(id, recorded, &submission);
        if let Some(state) = self.window_outputs.get(&id) {
            let mut work = state.last_work();
            work.submissions = 1;
            self.program.write_document(id, |document| {
                let context = document.context_mut();
                context.record_output_work(work);
                context.record_presentation_gpu_work(gpu_work);
            });
        }
        // Only the output painted the scene: its images' sizes are the
        // window's too.
        let targets = self
            .window_outputs
            .get(&id)
            .map(WindowOutputState::target_ids)
            .unwrap_or_default();
        self.commit_image_sizes(id, output_profile(), targets);
        self.deliver_window_output(id, output);
        true
    }
}
