//! Scene host present coordination.

use super::*;
#[cfg(target_os = "windows")]
use crate::SceneGpuRendererRegistry;

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
        self.program.prepare_window_frame(id, &self.context_for(id));
        if !super::schedule::drawable_surface(self.geometry_of(id).physical_size) {
            return true;
        }
        let Some(producers) = self.program.scene_resource_producers(id) else {
            return true;
        };
        let Some(scene) = self
            .program
            .read_document(id, |document| document.shared_scene())
        else {
            return false;
        };
        let resources = self.graphics.resources();
        let device = resources.device();
        let queue = resources.queue();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("NanaUI hidden gpu tick"),
        });
        match producers.encode_scene(scene.as_ref(), device, queue, &mut encoder) {
            Ok(prepared) => {
                let submission = queue.submit([encoder.finish()]);
                prepared.submitted(device, submission);
                true
            }
            Err(error) => {
                drop(encoder);
                self.program.host_failure(HostFailure::ResourceProduction {
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
        if self.render_suspended || !self.can_present(id) {
            self.rearm_frame_demand(id);
            return;
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
        let queued = self.drain_program_messages(id);
        self.apply_update(event_loop, queued, Some(id));
        if event_loop.exiting() || self.render_suspended || !self.can_present(id) {
            self.rearm_frame_demand(id);
            return;
        }
        self.resize_window(id);
        self.program.prepare_window_frame(id, &self.context_for(id));
        self.sync_compositor_clock(id);
        self.reconcile_browser_lifetimes();
        let geometry = self.geometry_of(id);
        let material = self.material_of(id);
        let viewport = LayoutViewport::new(geometry.logical_size.0, geometry.logical_size.1);
        let Some(flush) = self
            .program
            .write_document(id, |document| document.flush(viewport, &mut self.text))
        else {
            self.program
                .host_failure(HostFailure::MissingDocument { window: id });
            self.rearm_frame_demand(id);
            return;
        };
        let update = match flush {
            Ok(update) => update,
            Err(error) => {
                // The frame did not settle; Runtime restored its dirty work,
                // so the next redraw retries. Skipping keeps the process alive.
                self.program.host_failure(HostFailure::FrameDidNotSettle {
                    window: id,
                    error: error.to_string(),
                });
                self.rearm_frame_demand(id);
                return;
            }
        };
        // A cursor declaration can change while the pointer is stationary;
        // refresh the native cursor after the document's computed styles settle.
        // Ordinary redraws keep the pointer-event throttle and avoid a full
        // document probe on every animation/GPU frame.
        if update.cursor_changed {
            self.sync_window_cursor_forced(id);
        }
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
                .host_failure(HostFailure::MissingDocument { window: id });
            self.rearm_frame_demand(id);
            return;
        };
        self.sync_native_browsers(id, scene.as_ref());
        self.update_image_targets(id, scene.as_ref());
        let Some(host) = self.window_contexts.get(&id) else {
            return;
        };
        let format = host.surface.format();
        let frame = match self.acquire_frame(id) {
            Ok(HostedSurfaceFrame::Ready(frame)) => frame,
            Ok(HostedSurfaceFrame::Retry) => {
                self.request_redraw(id);
                return;
            }
            Ok(HostedSurfaceFrame::Skipped) => {
                self.rearm_frame_demand(id);
                return;
            }
            Err(error) => {
                self.rearm_frame_demand(id);
                self.suspend_surface(id, error);
                return;
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.graphics.resources().device().create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("NanaUI scene host frame"),
            },
        );
        let prepared = if let Some(producers) = self.program.scene_resource_producers(id) {
            match producers.encode_scene(
                scene.as_ref(),
                self.graphics.resources().device(),
                self.graphics.resources().queue(),
                &mut encoder,
            ) {
                Ok(prepared) => Some(prepared),
                Err(error) => {
                    drop(encoder);
                    drop(target);
                    self.discard_frame(id, frame);
                    self.program.host_failure(HostFailure::ResourceProduction {
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
            let regions = crate::native_content_regions(
                &scene,
                nana_ui_scene::SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: geometry.logical_size.0,
                    height: geometry.logical_size.1,
                },
            );
            let result = regions.and_then(|regions| {
                self.program
                    .native_content_frame(id, composition, &regions, &self.context_for(id))
            });
            if let Err(error) = result {
                drop(encoder);
                drop(target);
                self.discard_frame(id, frame);
                self.program
                    .host_failure(HostFailure::ResourceProduction { window: id, error });
                self.rearm_frame_demand(id);
                return;
            }
            let renderer = self.native_renderers.entry(format).or_default().clone();
            gpu_renderers
                .get_or_insert_with(SceneGpuRendererRegistry::new)
                .insert(nana_ui_runtime::NATIVE_CONTENT_RENDERER, renderer);
        }
        let theme = self.program.theme_mode();
        let fetch_host = self.program.resource_fetch_host(id);
        let painter = self.painter_mut(format);
        // Painters are shared per format, so every window supplies its own
        // egress, including none.
        painter.set_resource_fetch_host(fetch_host);
        let paint = painter.paint_target(
            crate::RenderTargetId(id.0),
            scene.as_ref(),
            &mut encoder,
            &target,
            scene_paint_viewport(&geometry, material, theme),
            host_textures.as_ref(),
            gpu_renderers.as_ref(),
        );
        if let Err(error) = paint {
            drop(encoder);
            drop(target);
            self.discard_frame(id, frame);
            self.program.host_failure(HostFailure::UnpaintableScene {
                window: id,
                error: error.to_string(),
            });
            self.rearm_frame_demand(id);
            return;
        }
        let submit_started = std::time::Instant::now();
        let submission = self.graphics.resources().queue().submit([encoder.finish()]);
        if let Some(prepared) = prepared {
            prepared.submitted(self.graphics.resources().device(), submission);
        }
        self.painter_mut(format)
            .record_submit(submit_started.elapsed());
        self.graphics.present(frame);
        if let Some(host) = self.window_contexts.get_mut(&id) {
            self.graphics.apply_pending_reconfigure(&mut host.surface);
        }
        self.serve_frame_demand(id);
        #[cfg(target_os = "windows")]
        if let Some(composition) = composition
            && let Err(error) = composition.commit()
        {
            self.program.host_failure(HostFailure::ResourceProduction {
                window: id,
                error: error.to_string(),
            });
            self.request_redraw(id);
            return;
        }
        // Publish semantics for the frame just presented. Application callbacks
        // below may commit new work intended for the next frame.
        #[cfg(not(target_os = "android"))]
        if !self.is_live_resize(id) {
            self.synchronize_accessibility(id);
        }
        self.apply_ime_request(id);
        let mut update = self
            .program
            .window_frame_presented(id, &self.context_for(id));
        if self.bind_after_present.remove(&id) {
            update = update.merge(self.program.bind_window(id, &self.context_for(id)));
        }
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
    }
    fn discard_frame(&mut self, id: WindowId, frame: wgpu::SurfaceTexture) {
        if let Some(host) = self.window_contexts.get_mut(&id) {
            self.graphics
                .discard_surface_frame(&mut host.surface, frame);
        }
    }

    pub(super) fn acquire_frame(
        &mut self,
        id: WindowId,
    ) -> Result<HostedSurfaceFrame, HostedGpuError> {
        let host = self
            .window_contexts
            .get_mut(&id)
            .ok_or(HostedGpuError::SurfaceValidation)?;
        self.graphics.acquire_surface_frame(&mut host.surface)
    }
    pub(super) fn recover_device(&mut self, _event_loop: &dyn ActiveEventLoop) {
        if self.embedded {
            self.render_suspended = true;
            self.next_gpu_retry = None;
            return;
        }
        let mut recovery_windows = self.known_window_ids();
        if recovery_windows.is_empty() {
            return;
        }
        recovery_windows.sort_unstable();
        // Any live window may seed the replacement device, since adapter choice
        // depends on its surface. A device failure does not, so stop there.
        let mut rebuilt = None;
        for &id in &recovery_windows {
            let surface = &self.window_contexts[&id].surface;
            match pollster::block_on(crate::HostedGpuShared::rebuild_for_surface(surface)) {
                Ok((graphics, surface)) => {
                    rebuilt = Some((id, graphics, surface));
                    break;
                }
                Err(HostedGpuError::Device(_)) => break,
                Err(_) => {}
            }
        }
        let Some((base, graphics, surface)) = rebuilt else {
            self.render_suspended = true;
            self.next_gpu_retry = Some(Instant::now() + GPU_RETRY_INTERVAL);
            return;
        };
        let surfaces = std::iter::once((base, Ok(surface)))
            .chain(
                recovery_windows
                    .iter()
                    .filter(|&&id| id != base)
                    .map(|&id| {
                        (
                            id,
                            graphics.recreate_surface(&self.window_contexts[&id].surface),
                        )
                    }),
            )
            .collect();
        self.switch_gpu(graphics, surfaces);
    }

    /// Adopt a replacement device. Windows whose surface failed on it recover
    /// individually; every other window presents on the new device immediately.
    pub(super) fn switch_gpu(
        &mut self,
        graphics: crate::HostedGpuShared,
        surfaces: Vec<(WindowId, Result<HostedGpuSurface, HostedGpuError>)>,
    ) {
        let mut failed = Vec::new();
        for (id, surface) in surfaces {
            let host = self.window_contexts.get_mut(&id).unwrap();
            host.surface_retry = None;
            match surface {
                Ok(surface) => host.surface = surface,
                Err(error) => failed.push((id, error)),
            }
        }
        self.graphics = graphics;
        self.painters.clear();
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
        self.request_redraw_all();
    }
    pub(super) fn suspend_surface(&mut self, id: WindowId, error: HostedGpuError) {
        // WGPU delivers a destroyed device's callback during polling, after its
        // submissions finish. Surface failure alone must not decide its scope.
        let _ = self
            .graphics
            .resources()
            .device()
            .poll(wgpu::PollType::Poll);
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
            self.program.host_failure(HostFailure::SurfaceRecovery {
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
    /// Painters are created lazily per surface format and own their image waker
    /// from creation, so the per-frame lookup does no allocation.
    pub(super) fn painter_mut(&mut self, format: wgpu::TextureFormat) -> &mut SceneWgpuPainter {
        let (graphics, targets, redraws, proxy) = (
            &self.graphics,
            &self.image_targets,
            &self.texture_redraws,
            &self.proxy,
        );
        self.painters.entry(format).or_insert_with(|| {
            let resources = graphics.resources();
            let mut painter = SceneWgpuPainter::new(resources.device(), resources.queue(), format);
            let (targets, redraws, proxy) =
                (Arc::clone(targets), Arc::clone(redraws), proxy.clone());
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
            painter
        })
    }
}

/// Preserve the retained native target until a complete replacement is ready.
/// A failed attempt consumes its deadline, so repeated loop wakes cannot spin.
fn retry_surface<T, E>(
    surface: &mut T,
    deadline: &mut Option<Instant>,
    now: Instant,
    recreate: impl FnOnce(&T) -> Result<T, E>,
) -> bool {
    if !deadline.is_some_and(|deadline| now >= deadline) {
        return false;
    }
    match recreate(surface) {
        Ok(replacement) => {
            *surface = replacement;
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
            |_| Err::<i32, _>("unavailable")
        ));
        assert_eq!(failed_surface, 10);
        assert_eq!(failed_deadline, Some(now + GPU_RETRY_INTERVAL));
        for time in [now, now + GPU_RETRY_INTERVAL / 2] {
            assert!(!retry_surface(
                &mut failed_surface,
                &mut failed_deadline,
                time,
                |_| -> Result<i32, ()> { panic!("retried before deadline") }
            ));
            assert!(!retry_surface(
                &mut healthy_surface,
                &mut healthy_deadline,
                time,
                |_| -> Result<i32, ()> { panic!("healthy window recreated") }
            ));
        }
        assert!(retry_surface(
            &mut failed_surface,
            &mut failed_deadline,
            now + GPU_RETRY_INTERVAL,
            |old| {
                assert_eq!(*old, 10);
                Ok::<_, ()>(11)
            }
        ));
        assert_eq!(failed_surface, 11);
        assert_eq!(failed_deadline, None);
        assert_eq!(healthy_surface, 20);
        assert_eq!(healthy_deadline, None);
    }
}
