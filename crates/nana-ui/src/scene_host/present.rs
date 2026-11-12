//! Scene host present coordination.

use super::*;
#[cfg(target_os = "windows")]
use crate::SceneGpuRendererRegistry;
use crate::hosted_context::SurfaceFrame;

/// A frame slot for one window present, or nothing when every slot is in
/// flight. Callers acquire the swapchain only after this returns. Slots free
/// in completion callbacks, which run when the device is polled: poll once
/// without waiting before giving up.
fn reserve_scene_host_frame(gpu: &nana_gpu::GpuContext) -> Option<nana_gpu::FrameContext> {
    const LABEL: &str = "NanaUI scene host frame";
    gpu.try_begin_frame(LABEL).or_else(|| {
        gpu.poll();
        gpu.try_begin_frame(LABEL)
    })
}

impl<Program: RuntimeProgram> WindowManager<Program> {
    /// `true` when prepare ran (encode may have been skipped). `false` on abort.
    pub(super) fn tick_hidden_gpu(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
    ) -> bool {
        if self.render_suspended {
            return false;
        }
        if self.graphics.take_device_lost() {
            self.recover_device(event_loop);
            return false;
        }
        if !self.window_contexts.contains_key(&id) {
            return false;
        }
        self.sync_compositor_clock(id);
        self.program.prepare_window_frame(id, &self.context_for(id));
        let output = self.sync_window_output(id)
            && self
                .program
                .window_output(id)
                .is_some_and(|config| config.while_hidden);
        if !super::schedule::drawable_surface(self.geometry_of(id).physical_size) {
            return true;
        }
        if output {
            // The output needs the document flushed; it shares the frame
            // with the window's producers.
            let producers = self.program.scene_resource_producers(id);
            return self.tick_hidden_output(event_loop, id, producers);
        }
        let Some(producers) = self.program.scene_resource_producers(id) else {
            self.producer_frames.remove(id);
            return true;
        };
        let Some(scene) = self
            .program
            .read_document(id, |document| document.shared_scene())
        else {
            return false;
        };
        let Some(producer_frame) = self.producer_frames.prepare(
            id,
            self.graphics.gpu().generation().get(),
            producers.retained_revision(scene.as_ref()),
        ) else {
            return true;
        };
        let Some(mut frame) = self
            .graphics
            .gpu()
            .try_begin_frame("NanaUI hidden gpu tick")
        else {
            self.retry_frame_soon(id);
            return false;
        };
        match producers.encode_scene(scene.as_ref(), &mut frame) {
            Ok(prepared) => {
                let submission = frame.submit();
                self.producer_frames.submitted(producer_frame);
                prepared.submitted(&submission);
                true
            }
            Err(error) => {
                drop(frame);
                self.program
                    .report_host_failure(HostFailure::ResourceProduction {
                        window: id,
                        error: error.to_string(),
                    });
                false
            }
        }
    }

    fn rearm_frame_demand(&mut self, id: WindowId) {
        self.update_frame_schedule(id, crate::runtime_host::FrameSchedule::defer);
    }

    /// Every frame slot is in flight: try again shortly, whatever the window
    /// asked for, since the slots free without anything asking for a frame.
    pub(super) fn retry_frame_soon(&mut self, id: WindowId) {
        self.update_frame_schedule(id, crate::runtime_host::FrameSchedule::retry);
    }

    fn serve_frame_demand(&mut self, id: WindowId) {
        self.update_frame_schedule(id, crate::runtime_host::FrameSchedule::advance_served);
    }

    /// A frame may close its own window; a closed window keeps no schedule.
    fn update_frame_schedule(
        &mut self,
        id: WindowId,
        update: impl FnOnce(
            &mut crate::runtime_host::FrameSchedule,
            crate::FrameDemand,
            Instant,
        ) -> Option<Instant>,
    ) {
        if !self.window_contexts.contains_key(&id) {
            return;
        }
        let demand = self.window_frame_demand(id);
        update(
            self.frame_schedules.entry(id).or_default(),
            demand,
            Instant::now(),
        );
    }

    pub(super) fn redraw(&mut self, event_loop: &dyn ActiveEventLoop, id: WindowId) {
        use nana_diagnostics::framework::host as stage;
        if self.render_suspended || !self.can_present(id) {
            self.rearm_frame_demand(id);
            return;
        }
        let frame_started = nana_diagnostics::metrics_enabled().then(Instant::now);
        if frame_started.is_some() {
            // Deliver completion callbacks of earlier submissions for
            // `gpu.completion`. Before the device-lost check, so a loss this
            // poll reports is handled in this frame either way.
            crate::host_diagnostics::poll_completions(self.graphics.gpu());
        }
        if self.graphics.take_device_lost() {
            self.recover_device(event_loop);
            self.rearm_frame_demand(id);
            return;
        }
        if !self.window_contexts.contains_key(&id) {
            self.rearm_frame_demand(id);
            return;
        }
        // Until the startup thread's painter arrives the splash, or nothing,
        // stays up; its arrival wakes the loop and asks for this frame again.
        if !self.take_startup_painter(false) {
            self.rearm_frame_demand(id);
            return;
        }
        // AppKit calls this synchronously from its live-resize tracking loop.
        // A step inside the commit interval returns before surface configure
        // and document layout; the drawable already on screen stays up.
        if self.defer_native_live_resize(id) {
            return;
        }
        // Before the program's messages and frame hook, which may start a
        // reveal or a motion; once more below, just ahead of the flush.
        self.sync_compositor_clock(id);
        let queued = self.drain_program_messages(id);
        self.apply_update(event_loop, queued, Some(id));
        if event_loop.exiting() || self.render_suspended || !self.can_present(id) {
            self.rearm_frame_demand(id);
            return;
        }
        self.resize_window(id);
        let mut stage_clock = crate::host_diagnostics::PresentationClock::start();
        self.program.prepare_window_frame(id, &self.context_for(id));
        stage_clock.mark(&stage::PRESENTATION_PREPARE_NS);
        self.sync_window_output(id);
        self.sync_compositor_clock(id);
        self.reconcile_browser_lifetimes();
        // The content's geometry: under a content zoom the document lays out
        // in a smaller logical viewport and paints at a larger scale.
        let geometry = self.content_geometry_of(id);
        let material = self.material_of(id);
        let viewport = LayoutViewport::new(geometry.logical_size.0, geometry.logical_size.1);
        let Some(flush) = self.program.write_document(id, |document| {
            crate::output::driver::flush_presentation(document, viewport, &mut self.text)
        }) else {
            self.program
                .report_host_failure(HostFailure::MissingDocument { window: id });
            self.fail_output_trace(id);
            self.rearm_frame_demand(id);
            return;
        };
        let update = match flush {
            Ok(update) => update,
            Err(error) => {
                self.fail_output_trace(id);
                // The frame did not settle; Runtime restored its dirty work,
                // so the next redraw retries. Skipping keeps the process alive.
                self.program
                    .report_host_failure(HostFailure::FrameDidNotSettle {
                        window: id,
                        error: error.to_string(),
                    });
                self.rearm_frame_demand(id);
                return;
            }
        };
        // A cursor declaration can change under a resting pointer; the
        // Runtime re-derives it once the document's computed styles settle.
        if update.cursor_changed {
            self.refresh_window_input(id);
        }
        self.sync_native_window_controls(id, &update);
        let Some(pending) = self.accessibility_pending_mut(id) else {
            self.rearm_frame_demand(id);
            return;
        };
        pending.stage(update.accessibility);
        let Some(scene) = self
            .program
            .write_document(id, |document| document.shared_scene())
        else {
            self.program
                .report_host_failure(HostFailure::MissingDocument { window: id });
            self.fail_output_trace(id);
            self.rearm_frame_demand(id);
            return;
        };
        self.sync_native_browsers(id, scene.as_ref());
        self.sync_shadow_body(id, scene.as_ref());
        self.update_image_targets(id, scene.as_ref());
        let Some(host) = self.window_contexts.get(&id) else {
            return;
        };
        let profile = host.surface.profile().scene_profile();
        // Display HDR metadata is cached by the surface and refreshed only on
        // window/display events.  It is a presentation uniform, not part of
        // the profile/pipeline cache key, so headroom changes do not rebuild
        // Scene state or shaders.
        let presentation_parameters = host.surface.presentation_parameters();
        // Decided before the drawable is acquired: a macOS handoff changes how
        // this frame is presented.
        let takes_over = self.prepare_startup_frame(id);
        // A primary window under its splash lays out and settles like any
        // other, so a takeover finds its first frame ready, but presents
        // nothing until something asked to take over.
        if self.startup_holds(id) {
            self.rearm_frame_demand(id);
            return;
        }
        // Take a frame slot before the drawable. A transparent swapchain has
        // nothing else to composite once its drawable is taken, so waiting
        // for a slot after acquiring it blanks the window; with every slot in
        // flight the presented frame stays on screen and this one is skipped.
        let trace_enabled = self.program.output_trace_enabled(id);
        stage_clock.mark(&stage::PRESENTATION_SETTLE_NS);
        let Some(mut recording) = reserve_scene_host_frame(self.graphics.gpu()) else {
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::FRAMES_SKIPPED);
            self.defer_output_trace(id, crate::output::OutputTraceDeferral::FrameSlots);
            self.retry_frame_soon(id);
            return;
        };
        stage_clock.mark(&stage::PRESENTATION_RESERVE_NS);
        let frame = match self.acquire_frame(id) {
            Ok(SurfaceFrame::Ready(frame)) => frame,
            Ok(SurfaceFrame::Retry) => {
                drop(recording);
                self.defer_output_trace(id, crate::output::OutputTraceDeferral::SurfaceAcquire);
                self.request_redraw(id);
                return;
            }
            Ok(SurfaceFrame::Skipped) => {
                drop(recording);
                self.defer_output_trace(id, crate::output::OutputTraceDeferral::SurfaceAcquire);
                nana_diagnostics::metric!(nana_diagnostics::framework::gpu::FRAMES_SKIPPED);
                self.rearm_frame_demand(id);
                return;
            }
            Err(error) => {
                drop(recording);
                self.fail_output_trace(id);
                self.rearm_frame_demand(id);
                self.suspend_surface(id, error);
                return;
            }
        };
        stage_clock.mark(&stage::PRESENTATION_ACQUIRE_NS);
        let target = nana_gpu::__framework::render_target(
            self.graphics.gpu(),
            frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default()),
            frame.texture.format(),
            [frame.texture.width(), frame.texture.height()],
        );
        // Everything recorded for this window goes into one frame. On every
        // failure path it is dropped before the surface texture it targets:
        // nothing may still reference the texture the surface is asked to
        // abandon (DX12), and dropping it rolls the painter's retained writes
        // back.
        let producers = self.program.scene_resource_producers(id);
        let producer_frame = self.producer_frames.prepare(
            id,
            self.graphics.gpu().generation().get(),
            producers.as_ref().map_or((false, Some(0)), |producers| {
                producers.retained_revision(scene.as_ref())
            }),
        );
        let prepared =
            if let Some(producers) = producers.as_ref().filter(|_| producer_frame.is_some()) {
                match producers.encode_scene(scene.as_ref(), &mut recording) {
                    Ok(prepared) => Some(prepared),
                    Err(error) => {
                        drop(recording);
                        drop(target);
                        self.discard_frame(id, frame);
                        self.fail_output_trace(id);
                        self.program
                            .report_host_failure(HostFailure::ResourceProduction {
                                window: id,
                                error: error.to_string(),
                            });
                        self.rearm_frame_demand(id);
                        return;
                    }
                }
            } else {
                None
            };
        stage_clock.mark(&stage::PRESENTATION_RESOURCES_NS);
        let host_textures = self.program.host_textures(id);
        if let Some(registry) = host_textures.as_ref() {
            if let Ok(plan) = scene.frame_plan() {
                let slots = plan
                    .custom_nodes
                    .iter()
                    .filter_map(|node| {
                        let primitive = scene.primitive(*node)?;
                        match &primitive.kind {
                            nana_ui_scene::ScenePrimitiveKind::Custom { node, .. }
                                if node.renderer.as_ref() == "nana.host-texture" =>
                            {
                                Some(Arc::clone(&node.resource))
                            }
                            _ => None,
                        }
                    })
                    .collect::<HashSet<_>>();
                let current = self.texture_subscriptions.get(&id).is_some_and(
                    |(subscribed, subscription)| {
                        *subscribed == slots && subscription.observes(registry)
                    },
                );
                if !current {
                    let pending = Arc::clone(&self.texture_redraws);
                    let proxy = self.proxy.clone();
                    let observed = slots.clone();
                    let subscription = registry.subscribe(move |slot| {
                        if slot.is_empty() || observed.contains(slot) {
                            pending.lock().expect("texture redraws").insert(id);
                            proxy.wake_up();
                        }
                    });
                    self.texture_subscriptions.insert(id, (slots, subscription));
                }
            }
        } else {
            self.texture_subscriptions.remove(&id);
        }
        // Only the Windows native-content path inserts into the registry.
        #[allow(unused_mut)]
        let mut gpu_renderers = self.program.scene_gpu_renderers(id);
        #[cfg(target_os = "windows")]
        let composition = self
            .window_contexts
            .get(&id)
            .and_then(|host| host.surface.windows_composition())
            .cloned();
        #[cfg(target_os = "windows")]
        if let Some(composition) = composition.as_ref() {
            match self.sync_native_content(id, composition, &scene, geometry.logical_size) {
                Ok(true) => {
                    // Native content is drawn into the painter's linear
                    // working target, which is fp16 for P3/HDR profiles.
                    // Caching it by the acquired surface format would create
                    // an incompatible pipeline when that target is unorm.
                    let renderer = self
                        .native_renderers
                        .entry(profile.working_format())
                        .or_default()
                        .clone();
                    gpu_renderers
                        .get_or_insert_with(SceneGpuRendererRegistry::new)
                        .insert(nana_ui_runtime::NATIVE_CONTENT_RENDERER, renderer);
                }
                Ok(false) => {}
                Err(error) => {
                    drop(recording);
                    drop(target);
                    self.discard_frame(id, frame);
                    self.fail_output_trace(id);
                    self.program
                        .report_host_failure(HostFailure::ResourceProduction { window: id, error });
                    self.rearm_frame_demand(id);
                    return;
                }
            }
        }
        let theme = self.program.theme();
        let window_background = self.program.window_background();
        let fetch_host = self.program.resource_fetch_host(id);
        let viewport = scene_paint_viewport(&geometry, material, theme.as_ref(), window_background);
        // Subpixel text only onto a surface the compositor shows opaque: over
        // a transparent material the per-channel coverage would be composited
        // as colored alpha.
        let opaque = self
            .window_contexts
            .get(&id)
            .is_some_and(|host| host.surface.alpha_mode() == crate::SurfaceAlphaMode::Opaque)
            && matches!(
                profile.color_space,
                crate::ScenePresentationColorSpace::Srgb
            );
        let output_step = self.plan_window_output(
            id,
            scene.as_ref(),
            host_textures.as_ref(),
            gpu_renderers.as_ref(),
            producers.as_ref(),
            viewport.clear_color,
        );
        if let Some(state) = self.window_outputs.get_mut(&id) {
            state.can_share_window(self.graphics.gpu(), &geometry, profile, viewport, opaque);
        }
        let direct = crate::output::driver::frame_driver::DirectPresentation {
            id: crate::RenderTargetId(id.0),
            profile,
            viewport,
            alpha: if opaque {
                crate::AlphaEncoding::Linear
            } else {
                crate::AlphaEncoding::Gamma
            },
            text_aa: opaque
                .then(crate::scene_paint::SubpixelOrder::system)
                .flatten(),
            parameters: presentation_parameters,
        };
        stage_clock.mark(&stage::PRESENTATION_PLAN_NS);
        // This frame samples every host texture slot as it stands now, so a
        // wake for this window queued before here is answered by it. Above
        // all the one its own `prepare_window_frame` raised by binding a
        // producer frame into a slot: left queued, every producer frame
        // presented twice. A slot replaced from here on queues a fresh wake.
        if let Ok(mut pending) = self.texture_redraws.lock() {
            pending.remove(&id);
        }
        let (output, encoded_presentation) = match self.encode_presentation(
            id,
            output_step,
            Some(direct),
            scene.as_ref(),
            &mut recording,
            Some(&target),
            host_textures.as_ref(),
            gpu_renderers.as_ref(),
            fetch_host,
        ) {
            Ok(encoded) => encoded,
            Err(error) => {
                drop(recording);
                drop(target);
                self.discard_frame(id, frame);
                self.fail_output_trace(id);
                self.program
                    .report_host_failure(HostFailure::UnpaintableScene {
                        window: id,
                        error: error.to_string(),
                    });
                self.rearm_frame_demand(id);
                return;
            }
        };
        let shared_paint = encoded_presentation.shared;
        let direct_ui = encoded_presentation.direct_ui;
        stage_clock.mark(&stage::PRESENTATION_ENCODE_NS);
        let submission = recording.submit();
        stage_clock.mark(&stage::PRESENTATION_SUBMIT_NS);
        if let Some(producer_frame) = producer_frame {
            self.producer_frames.submitted(producer_frame);
        }
        let trace_waits =
            trace_enabled.then(|| crate::output::OutputTraceWait::from_submission(&submission));
        if let Some(state) = self.window_outputs.get_mut(&id) {
            state.trace_host_submission(direct_ui, trace_waits);
        }
        let output = self.complete_window_output(id, output, &submission);
        let mut output_work = self
            .window_outputs
            .get(&id)
            .map(|state| state.last_work())
            .unwrap_or_default();
        output_work.ui_renders += usize::from(direct_ui);
        output_work.submissions = 1;
        let gpu_work = encoded_presentation.gpu_work;
        self.program.write_document(id, |document| {
            let context = document.context_mut();
            context.record_output_work(output_work);
            context.record_presentation_gpu_work(gpu_work);
        });
        if frame_started.is_some() {
            crate::host_diagnostics::watch_submission(self.graphics.gpu().raw_queue());
        }
        if let Some(prepared) = prepared {
            prepared.submitted(&submission);
        }
        let submit = submission.cpu_duration();
        let painter = self.painter_mut(if shared_paint {
            crate::output::window_output::output_profile()
        } else {
            profile
        });
        painter.record_submit(&submission);
        stage_clock.mark(&stage::PRESENTATION_POST_SUBMIT_NS);
        self.graphics.present_frame(frame);
        self.note_window_presented(id);
        stage_clock.mark(&stage::PRESENTATION_PRESENT_NS);
        self.note_native_live_resize_presented(id);
        crate::host_diagnostics::frame_presented(frame_started, submit, gpu_work);
        if let Some(host) = self.window_contexts.get_mut(&id) {
            self.graphics.apply_pending_reconfigure(&mut host.surface);
        }
        self.serve_frame_demand(id);
        // Before any program callback can move the document away from the
        // scene just painted.
        if shared_paint {
            let targets = self
                .window_outputs
                .get(&id)
                .map(|state| state.target_ids())
                .unwrap_or_default();
            self.commit_image_sizes(id, crate::output::window_output::output_profile(), targets);
        } else {
            self.commit_image_sizes(id, profile, [crate::RenderTargetId(id.0)]);
        }
        if direct_ui {
            self.observe_direct_output_trace(id, profile, viewport.physical_size, trace_waits);
        } else {
            self.direct_output_traces.remove(&id);
        }
        // The one transaction boundary for this window's composition tree.
        // Backends stage; the host publishes, and only when something was
        // staged — a retained compositor does not follow the GPU's frame rate.
        #[cfg(target_os = "windows")]
        if let Some(composition) = composition
            && let Err(error) = composition.commit()
        {
            self.fail_output_trace(id);
            self.program
                .report_host_failure(HostFailure::ResourceProduction {
                    window: id,
                    error: error.to_string(),
                });
            self.request_redraw(id);
            return;
        }
        self.deliver_window_output(id, output);
        // The frame is presented (and, on Windows, its composition published):
        // it may now end the startup.
        if takes_over {
            self.startup_frame_presented(event_loop);
        }
        // Publish semantics for the frame just presented. Application callbacks
        // below may commit new work intended for the next frame.
        #[cfg(not(target_os = "android"))]
        if !self.is_live_resize(id) {
            self.synchronize_accessibility(id);
        }
        // Layout moved the caret, or the field under it: the IME follows.
        self.refresh_window_input(id);
        stage_clock.mark(&stage::PRESENTATION_FINISH_NS);
        let mut update = self
            .program
            .window_frame_presented(id, &self.context_for(id));
        if self.bind_after_present.remove(&id) {
            update = update.merge(self.program.bind_window(id, &self.context_for(id)));
        }
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
    }
    /// Hands the natural sizes `targets` of the `profile` painter learned
    /// while painting `id`'s scene to its document (Issue #263), once the
    /// frame is presented. The boxes that read them lay out on the next
    /// frame, which is asked for.
    pub(super) fn commit_image_sizes(
        &mut self,
        id: WindowId,
        _profile: crate::ScenePresentationProfile,
        targets: impl IntoIterator<Item = crate::RenderTargetId>,
    ) {
        let Some(painter) = self.painter.as_mut() else {
            return;
        };
        let sizes: Vec<_> = targets
            .into_iter()
            .flat_map(|target| painter.take_image_natural_sizes(Some(target)))
            .collect();
        if sizes.is_empty() {
            return;
        }
        let committed = self.program.write_document(id, |document| {
            crate::commit_image_natural_sizes(document.context_mut(), &sizes)
        });
        match committed {
            Some(Ok(true)) => self.request_redraw(id),
            Some(Ok(false)) | None => {}
            Some(Err(error)) => self
                .program
                .report_host_failure(HostFailure::ResourceProduction {
                    window: id,
                    error: error.to_string(),
                }),
        }
    }

    /// Mirrors this frame's native-content regions into the window's
    /// composition tree, doing nothing when nothing they depend on moved.
    ///
    /// Returns whether the scene has native content at all, which is what
    /// decides if the opening renderer has to be registered for this frame.
    /// The decision itself lives on [`crate::native_content::NativeContentMirror`]; this only
    /// hands the result to the backend.
    #[cfg(target_os = "windows")]
    fn sync_native_content(
        &mut self,
        id: WindowId,
        composition: &crate::WindowsComposition,
        scene: &nana_ui_scene::UiScene,
        logical_size: (f32, f32),
    ) -> Result<bool, String> {
        // The mirror stays in the map across the backend call: the context that
        // call receives reports this window's mirroring counters, and taking the
        // mirror out would hand the backend zeros.
        let outcome = self.native_content.entry(id).or_default().sync(
            scene,
            nana_ui_scene::SceneRect {
                x: 0.0,
                y: 0.0,
                width: logical_size.0,
                height: logical_size.1,
            },
        )?;
        // Whether the opening renderer is needed this frame is a property of
        // the scene, not of how many regions survived clipping: an
        // unregistered custom renderer fails the whole frame.
        let present = outcome.scene_has_native_content();
        let crate::native_content::NativeContentSync::Stage { .. } = outcome else {
            return Ok(present);
        };
        // Copied out so the backend call can borrow the host. This is the
        // changed-geometry path, which already walked the scene; a settled frame
        // never gets here.
        let regions = self
            .native_content
            .get(&id)
            .map(|mirror| mirror.regions().to_vec())
            .unwrap_or_default();
        let staged = self.program.native_content_frame(
            id,
            composition.tree(),
            &regions,
            &self.context_for(id),
        );
        if staged.is_err() {
            // The backend did not take them, so the mirror must not go on
            // claiming the compositor has them — the retry after this frame's
            // failure would otherwise find itself settled and stage nothing.
            if let Some(mirror) = self.native_content.get_mut(&id) {
                mirror.invalidate();
            }
        }
        staged.map(|()| present)
    }

    fn discard_frame(&mut self, id: WindowId, frame: wgpu::SurfaceTexture) {
        if let Some(host) = self.window_contexts.get_mut(&id) {
            self.graphics.abandon_frame(&mut host.surface, frame);
        }
    }

    pub(super) fn acquire_frame(&mut self, id: WindowId) -> Result<SurfaceFrame, HostedGpuError> {
        let host = self
            .window_contexts
            .get_mut(&id)
            .ok_or(HostedGpuError::SurfaceValidation)?;
        self.graphics.acquire(&mut host.surface)
    }
    pub(super) fn recover_device(&mut self, _event_loop: &dyn ActiveEventLoop) {
        // Marks the loss handled. `nana-gpu` already recorded the
        // `gpu.device_lost` fault and its snapshot when the loss arrived.
        let _ = self.graphics.take_device_lost_report();
        if self.embedded {
            self.render_suspended = true;
            self.next_gpu_retry = None;
            return;
        }
        let mut recovery_windows = self.known_window_ids();
        if recovery_windows.is_empty() {
            // Nothing can seed a replacement device yet. The next window's
            // first frame finds the loss again and recovers with its surface.
            self.graphics.rearm_device_loss();
            return;
        }
        recovery_windows.sort_unstable();
        // Any live window may seed the replacement device, since adapter choice
        // depends on its surface. A device failure does not, so stop there.
        let mut rebuilt = None;
        let choice = self.graphics.adapter_choice().clone();
        for &id in &recovery_windows {
            let surface = &mut self.window_contexts.get_mut(&id).unwrap().surface;
            match pollster::block_on(crate::HostedGpuShared::rebuild_for_surface(
                surface,
                choice.clone(),
            )) {
                Ok(graphics) => {
                    rebuilt = Some((id, graphics));
                    break;
                }
                Err(HostedGpuError::Device(_)) => break,
                Err(_) => {}
            }
        }
        let Some((base, graphics)) = rebuilt else {
            // Retried every GPU_RETRY_INTERVAL; report the first failure only.
            if self.next_gpu_retry.is_none() {
                nana_diagnostics::fault!(nana_diagnostics::framework::gpu::DEVICE_RECOVERY_FAILED);
            }
            self.render_suspended = true;
            self.next_gpu_retry = Some(Instant::now() + GPU_RETRY_INTERVAL);
            return;
        };
        let mut outcomes = vec![(base, Ok(()))];
        for &id in recovery_windows.iter().filter(|&&id| id != base) {
            let surface = &mut self.window_contexts.get_mut(&id).unwrap().surface;
            outcomes.push((id, graphics.recreate_surface(surface)));
        }
        self.switch_gpu(graphics, outcomes);
    }

    /// Adopt a replacement device. Windows whose surface failed on it recover
    /// individually; every other window presents on the new device immediately.
    /// Adopt `graphics`, onto which every window's surface was already rebound.
    pub(super) fn switch_gpu(
        &mut self,
        graphics: crate::HostedGpuShared,
        outcomes: Vec<(WindowId, Result<(), HostedGpuError>)>,
    ) {
        let mut failed = Vec::new();
        for (id, outcome) in outcomes {
            self.window_contexts.get_mut(&id).unwrap().surface_retry = None;
            if let Err(error) = outcome {
                failed.push((id, error));
            }
        }
        self.graphics = graphics;
        // Before the painters and the program's renderers rebuild: they
        // compile through the cache the store restores here.
        if let Some(cache) = &mut self.pipeline_cache {
            cache.replace_device(self.graphics.gpu());
        }
        self.reset_startup_latch();
        // A replacement device can be on a different backend than the one this
        // process started on. Whether a window opened from now on can reach a
        // platform compositor is that device's answer, not the old one's.
        self.composition = crate::presentation::CompositionAvailability::for_backend(
            self.gpu_backend_policy,
            self.graphics.adapter_info().backend,
        );
        self.retire_window_outputs();
        self.painter = None;
        self.native_renderers.clear();
        self.next_gpu_retry = None;
        self.render_suspended = false;
        self.bump_surface_generation();
        for (id, error) in failed {
            self.suspend_surface(id, error);
        }
        self.refresh_material();
        invalidate_program_host_textures(self.known_window_ids(), |id| {
            self.program.host_textures(id)
        });
        self.program.rebuild_gpu(&self.context());
        crate::host_diagnostics::record_adapter(self.graphics.adapter_info());
        nana_diagnostics::event!(nana_diagnostics::framework::gpu::DEVICE_RECOVERED);
        self.request_redraw_all();
    }
    pub(super) fn suspend_surface(&mut self, id: WindowId, error: HostedGpuError) {
        // WGPU delivers a destroyed device's callback during polling, after its
        // submissions finish. Surface failure alone must not decide its scope.
        self.graphics.gpu().poll();
        // A lost device fails every surface; leave it to process-wide recovery
        // instead of reporting per-window surface failures.
        if !self.embedded && self.graphics.is_device_lost() {
            return;
        }
        let Some(host) = self.window_contexts.get_mut(&id) else {
            return;
        };
        // Surface errors do not imply device loss. Keep the other windows alive,
        // and never let repeated appearance updates postpone or bypass this retry.
        if host.surface_retry.is_none() {
            host.surface_retry = Some(Instant::now() + GPU_RETRY_INTERVAL);
            nana_diagnostics::event!(
                nana_diagnostics::framework::gpu::SURFACE_SUSPENDED,
                window = id.0
            );
            self.program
                .report_host_failure(HostFailure::SurfaceRecovery {
                    window: id,
                    error: error.to_string(),
                });
        }
    }

    pub(super) fn retry_surfaces(&mut self, now: Instant) {
        if self.render_suspended {
            return;
        }
        let mut recovered = Vec::new();
        for id in self.known_window_ids() {
            let host = self.window_contexts.get_mut(&id).unwrap();
            if retry_surface(&mut host.surface, &mut host.surface_retry, now, |surface| {
                self.graphics.recreate_surface(surface)
            }) {
                host.applied_appearance = None;
                recovered.push(id);
            }
        }
        if recovered.is_empty() {
            return;
        }
        self.bump_surface_generation();
        for id in recovered {
            if let Err(error) = self.sync_window_material(id) {
                self.suspend_surface(id, error);
            } else {
                self.request_redraw(id);
            }
        }
    }
    /// One shared painter serves every presentation profile, sharing glyph and
    /// icon atlases, text caches, URL textures and skin materials while
    /// switching working-format pipeline sets on demand.
    pub(super) fn painter_mut(
        &mut self,
        profile: crate::ScenePresentationProfile,
    ) -> &mut SceneWgpuPainter {
        if self.painter.is_none() && self.startup_painter.is_some() {
            self.take_startup_painter(true);
        }
        // A device switch clears the painter; one that still names another
        // device would only refuse every frame.
        let current = self.graphics.gpu().generation();
        if self
            .painter
            .as_ref()
            .is_some_and(|painter| painter.gpu().generation() != current)
        {
            self.painter = None;
        }
        if self.painter.is_none() {
            let painter = SceneWgpuPainter::new_with_presentation(self.graphics.gpu(), profile);
            self.adopt_painter(painter);
        }
        let painter = self.painter.as_mut().expect("painter was just inserted");
        painter.select_presentation(profile);
        painter
    }

    pub(super) fn retain_active_presentations(&mut self) {
        let Some(painter) = self.painter.as_mut() else {
            return;
        };
        let mut profiles: Vec<crate::ScenePresentationProfile> = self
            .window_contexts
            .values()
            .map(|ctx| ctx.surface.profile().scene_profile())
            .collect();
        if !self.window_outputs.is_empty() {
            profiles.push(crate::output::window_output::output_profile());
        }
        if !profiles.is_empty() {
            painter.retain_presentations(&profiles);
        }
    }

    /// Starts saving pipelines compiled from now on to the host's store.
    pub(super) fn watch_pipeline_cache(&mut self) {
        if let Some(cache) = &mut self.pipeline_cache {
            cache.watch(self.graphics.gpu());
        }
    }

    /// Takes the painter the startup thread built, once it has, waiting for it
    /// when `wait`. Returns whether none is still to come.
    pub(super) fn take_startup_painter(&mut self, wait: bool) -> bool {
        let Some((_, receiver)) = &self.startup_painter else {
            return true;
        };
        let received = if wait {
            receiver
                .recv()
                .map_err(|_| mpsc::TryRecvError::Disconnected)
        } else {
            receiver.try_recv()
        };
        let built = match received {
            Ok(built) => built,
            Err(mpsc::TryRecvError::Empty) => return false,
            // The thread stopped without one; a painter is built when needed.
            Err(mpsc::TryRecvError::Disconnected) => {
                self.startup_painter = None;
                self.watch_pipeline_cache();
                return true;
            }
        };
        self.startup_painter = None;
        // The startup thread restored and saved the cache before sending
        // the painter; saves from here on cannot race its restore.
        self.watch_pipeline_cache();
        startup::record_device_start(&self.startup.handle, None, Some(built.build));
        if self.painter.is_none() {
            self.adopt_painter(built.painter);
        }
        true
    }

    /// Installs a painter built elsewhere (the startup thread builds the first
    /// one alongside the device) with this host's image waker.
    pub(super) fn adopt_painter(&mut self, mut painter: SceneWgpuPainter) {
        let (targets, redraws, proxy) = (
            Arc::clone(&self.image_targets),
            Arc::clone(&self.texture_redraws),
            self.proxy.clone(),
        );
        painter.set_image_update_waker(Arc::new(move |key| {
            let ids = targets
                .lock()
                .ok()
                .and_then(|targets| targets.get(key).cloned())
                .unwrap_or_default();
            if !ids.is_empty()
                && let Ok(mut pending) = redraws.lock()
            {
                pending.extend(ids);
            }
            proxy.wake_up();
        }));
        let (redraws, proxy) = (Arc::clone(&self.material_redraws), self.proxy.clone());
        let wake = self.material_waker.get_or_insert_with(|| {
            Arc::new(move || {
                redraws.store(true, std::sync::atomic::Ordering::Release);
                proxy.wake_up();
            })
        });
        painter.set_material_waker(Arc::clone(wake));
        self.note_startup_painter();
        self.painter = Some(painter);
    }
}

/// Preserve the retained native target until a complete replacement is ready.
/// A failed attempt consumes its deadline, so repeated loop wakes cannot spin.
fn retry_surface<T, E>(
    surface: &mut T,
    deadline: &mut Option<Instant>,
    now: Instant,
    rebind: impl FnOnce(&mut T) -> Result<(), E>,
) -> bool {
    if !deadline.is_some_and(|deadline| now >= deadline) {
        return false;
    }
    match rebind(surface) {
        Ok(()) => {
            *deadline = None;
            true
        }
        Err(_) => {
            *deadline = Some(now + GPU_RETRY_INTERVAL);
            false
        }
    }
}

#[cfg(test)]
mod surface_recovery_tests {
    use super::*;

    #[test]
    fn failed_window_retains_surface_and_backs_off_without_touching_healthy_window() {
        let now = Instant::now();
        let mut failed_surface = 10;
        let mut healthy_surface = 20;
        let mut failed_deadline = Some(now);
        let mut healthy_deadline = None;
        assert!(!retry_surface(
            &mut failed_surface,
            &mut failed_deadline,
            now,
            |_| Err::<(), _>("unavailable")
        ));
        assert_eq!(failed_surface, 10);
        assert_eq!(failed_deadline, Some(now + GPU_RETRY_INTERVAL));
        for time in [now, now + GPU_RETRY_INTERVAL / 2] {
            assert!(!retry_surface(
                &mut failed_surface,
                &mut failed_deadline,
                time,
                |_| -> Result<(), ()> { panic!("retried before deadline") }
            ));
            assert!(!retry_surface(
                &mut healthy_surface,
                &mut healthy_deadline,
                time,
                |_| -> Result<(), ()> { panic!("healthy window recreated") }
            ));
        }
        assert!(retry_surface(
            &mut failed_surface,
            &mut failed_deadline,
            now + GPU_RETRY_INTERVAL,
            |surface| {
                assert_eq!(*surface, 10);
                *surface = 11;
                Ok::<_, ()>(())
            }
        ));
        assert_eq!(failed_surface, 11);
        assert_eq!(failed_deadline, None);
        assert_eq!(healthy_surface, 20);
        assert_eq!(healthy_deadline, None);
    }
}

#[cfg(test)]
mod present_order_tests {
    /// `redraw` reserves a frame slot before `acquire_frame`: with every slot
    /// in flight it returns before `get_current_texture`, instead of taking
    /// the drawable and then waiting on the GPU.
    #[test]
    fn redraw_reserves_a_frame_before_acquiring_the_swapchain() {
        let source = include_str!("present.rs");
        let start = source.find("pub(super) fn redraw").expect("redraw");
        let body = &source[start..];
        let end = body[1..]
            .find("\n    fn ")
            .map(|index| index + 1)
            .unwrap_or(body.len());
        let body = &body[..end];
        let reserve = body
            .find("reserve_scene_host_frame")
            .expect("redraw reserves a frame slot");
        let acquire = body
            .find("self.acquire_frame(id)")
            .expect("redraw acquires the swapchain");
        assert!(reserve < acquire, "the drawable waits for a frame slot");
        let before_acquire = &body[..acquire];
        assert!(
            before_acquire.contains("retry_frame_soon"),
            "a full pipeline skips the frame and tries again"
        );
        assert!(
            !body.contains("begin_frame("),
            "the window thread never blocks on a frame slot"
        );
    }
}
