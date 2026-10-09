//! Scene host side of window outputs: the second paint of a window's scene
//! into its offscreen output, inside the frame the host records anyway.

use super::*;
use crate::output::window_output::{OutputStep, RecordedOutput, WindowOutputState, output_profile};
use crate::runtime_host::FrameDemand;

impl<Program: RuntimeProgram> WindowManager<Program> {
    /// Applies the program's current answer for `id`'s output and delivers
    /// whatever that changed. Returns whether an output is configured.
    pub(super) fn sync_window_output(&mut self, id: WindowId) -> bool {
        let config = self.program.window_output(id);
        if config.is_none() && !self.window_outputs.contains_key(&id) {
            return false;
        }
        self.window_outputs
            .entry(id)
            .or_insert_with(|| WindowOutputState::new(id))
            .set_config(config);
        self.flush_window_output(id);
        if config.is_none() {
            self.window_outputs.remove(&id);
        }
        config.is_some()
    }

    /// What `id`'s output adds to the window's frame demand: a cadence while
    /// it cannot present, or the instant a throttled frame comes due.
    pub(super) fn window_output_demand(&self, id: WindowId, can_present: bool) -> FrameDemand {
        let configured = self.program.window_output(id);
        let state = self.window_outputs.get(&id);
        match (configured, state) {
            (Some(config), _) if !can_present && config.while_hidden => FrameDemand::Continuous(
                config
                    .max_fps
                    .unwrap_or(crate::output::window_output::WINDOW_OUTPUT_HIDDEN_FPS),
            ),
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
        clear_color: [f32; 4],
    ) -> Option<OutputStep> {
        if !self.window_outputs.contains_key(&id) {
            return None;
        }
        let geometry = self.geometry_of(id);
        // Content that moves without the scene's projection moving: custom
        // GPU renderers and compositor animation.
        let volatile = gpu_renderers.is_some()
            || self
                .program
                .read_document(id, |document| document.compositor_needs_tick())
                .unwrap_or(false);
        let image_revision = self.painter_mut(output_profile()).image_revision();
        let host_revision = image_revision ^ self.surface_generation.rotate_left(32);
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

    /// Paints `scene` into `id`'s output inside `frame`, as planned.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_window_output(
        &mut self,
        id: WindowId,
        step: Option<OutputStep>,
        scene: &nana_ui_scene::UiScene,
        frame: &mut nana_gpu::FrameContext,
        host_textures: Option<&crate::HostTextureRegistry>,
        gpu_renderers: Option<&crate::SceneGpuRendererRegistry>,
        fetch_host: Option<nana_ui_platform::SharedFetchHost>,
    ) -> Option<RecordedOutput> {
        let step = step?;
        if !matches!(step, OutputStep::Record { .. }) {
            return None;
        }
        let mut state = self.window_outputs.remove(&id)?;
        let painter = self.painter_mut(output_profile());
        painter.set_resource_fetch_host(fetch_host);
        let recorded = state.record(step, frame, painter, scene, host_textures, gpu_renderers);
        self.window_outputs.insert(id, state);
        recorded
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
        }
        self.flush_window_output(id);
    }

    fn flush_window_output(&mut self, id: WindowId) {
        let Some(state) = self.window_outputs.get_mut(&id) else {
            return;
        };
        let targets = state.take_retired_targets();
        let statuses = state.take_statuses();
        for target in targets {
            for painter in self.painters.values_mut() {
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
        for target in state.target_ids() {
            for painter in self.painters.values_mut() {
                painter.remove_target(target);
            }
        }
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
        let geometry = self.geometry_of(id);
        let viewport = LayoutViewport::new(geometry.logical_size.0, geometry.logical_size.1);
        let Some(flush) = self
            .program
            .write_document(id, |document| document.flush(viewport, &mut self.text))
        else {
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
            clear_color,
        );
        if producers.is_none() && !matches!(step, Some(OutputStep::Record { .. })) {
            // An unchanged output and no producers: nothing to record.
            self.flush_window_output(id);
            return true;
        }
        let fetch_host = self.program.resource_fetch_host(id);
        let mut frame = self
            .graphics
            .gpu()
            .begin_frame("NanaUI hidden window output");
        let prepared = match producers {
            Some(producers) => match producers.encode_scene(scene.as_ref(), &mut frame) {
                Ok(prepared) => Some(prepared),
                Err(error) => {
                    drop(frame);
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
        let recorded = self.record_window_output(
            id,
            step,
            scene.as_ref(),
            &mut frame,
            host_textures.as_ref(),
            gpu_renderers.as_ref(),
            fetch_host,
        );
        let submission = frame.submit();
        if let Some(prepared) = prepared {
            prepared.submitted(&submission);
        }
        let output = self.complete_window_output(id, recorded, &submission);
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
