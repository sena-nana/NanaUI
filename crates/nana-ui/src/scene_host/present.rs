//! Scene host present coordination.

use super::*;
#[cfg(target_os = "windows")]
use crate::SceneGpuRendererRegistry;

impl<Program: RuntimeProgram> SceneReady<Program> {
    pub(super) fn redraw(&mut self, event_loop: &dyn ActiveEventLoop, id: WindowId) {
        if self.render_suspended || !self.can_present(id) {
            return;
        }
        if self.graphics.take_device_lost() {
            self.recover_device(event_loop);
            return;
        }
        if id != WindowId::PRIMARY && !self.auxiliary.contains_key(&id) {
            return;
        }
        let queued = self.drain_program_messages(id);
        self.apply_update(event_loop, queued, Some(id));
        if event_loop.exiting() || self.render_suspended {
            return;
        }
        self.resize_window(id);
        self.program.prepare_window_frame(id, &self.context_for(id));
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
                return;
            }
        };
        let Some(pending) = self.accessibility_pending_mut(id) else {
            return;
        };
        pending.stage(update.accessibility);
        let Some(scene) = self
            .program
            .write_document(id, |document| document.shared_scene())
        else {
            self.program
                .host_failure(HostFailure::MissingDocument { window: id });
            return;
        };
        self.sync_native_browsers(id, scene.as_ref());
        self.update_image_targets(id, scene.as_ref());
        let format = if id == WindowId::PRIMARY {
            self.graphics.format()
        } else {
            let Some(auxiliary) = self.auxiliary.get(&id) else {
                // prepare_window_frame may have closed this auxiliary surface
                // after the redraw guard above admitted it.
                self.program
                    .host_failure(HostFailure::AuxiliarySurfaceLost { window: id });
                return;
            };
            auxiliary.surface.format()
        };
        let frame = match self.acquire_frame(id) {
            Ok(HostedSurfaceFrame::Ready(frame)) => frame,
            Ok(HostedSurfaceFrame::Retry) => {
                self.request_redraw(id);
                return;
            }
            Ok(HostedSurfaceFrame::Skipped) => return,
            Err(error) => {
                self.suspend_rendering(error);
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
                let pending = Arc::clone(&self.texture_redraws);
                let proxy = self.proxy.clone();
                self.texture_subscriptions.insert(
                    id,
                    registry.subscribe(move |slot| {
                        if slot.is_empty() || slots.contains(slot) {
                            pending.lock().expect("texture redraws").insert(id);
                            proxy.wake_up();
                        }
                    }),
                );
            }
        } else {
            self.texture_subscriptions.remove(&id);
        }
        // Only the Windows native-content path inserts into the registry.
        #[allow(unused_mut)]
        let mut gpu_renderers = self.program.scene_gpu_renderers(id);
        #[cfg(target_os = "windows")]
        let composition = if id == WindowId::PRIMARY {
            self.graphics.windows_composition().cloned()
        } else {
            self.auxiliary
                .get(&id)
                .and_then(|host| host.surface.windows_composition())
                .cloned()
        };
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
                return;
            }
            let renderer = self.native_renderers.entry(format).or_default().clone();
            gpu_renderers
                .get_or_insert_with(SceneGpuRendererRegistry::new)
                .insert(nana_ui_runtime::NATIVE_CONTENT_RENDERER, renderer);
        }
        let theme = self.program.theme_mode();
        let paint = self.painter_mut(format).paint_target(
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
            self.request_redraw(id);
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
        if id == WindowId::PRIMARY {
            self.graphics.discard_frame(frame);
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
            self.graphics
                .discard_surface_frame(&mut host.surface, frame);
        }
    }

    pub(super) fn acquire_frame(
        &mut self,
        id: WindowId,
    ) -> Result<HostedSurfaceFrame, HostedGpuError> {
        if id == WindowId::PRIMARY {
            self.graphics.acquire_frame()
        } else {
            let host = self
                .auxiliary
                .get_mut(&id)
                .ok_or(HostedGpuError::SurfaceValidation)?;
            self.graphics.acquire_surface_frame(&mut host.surface)
        }
    }
    pub(super) fn recover_device(&mut self, event_loop: &dyn ActiveEventLoop) {
        let window = Arc::clone(self.graphics.window());
        let _ = apply_window_surface(
            window.as_ref(),
            self.last_theme,
            &self.settings,
            self.last_material_mode,
            self.program
                .appearance_backdrop_opacity_for(WindowId::PRIMARY),
        );
        match pollster::block_on(self.graphics.recreate(wgpu::Features::empty())) {
            Ok(graphics) => {
                let mut painters = HashMap::new();
                painters.insert(
                    graphics.format(),
                    SceneWgpuPainter::new(
                        graphics.resources().device(),
                        graphics.resources().queue(),
                        graphics.format(),
                    ),
                );
                let previous = std::mem::take(&mut self.auxiliary);
                let recovery_windows: Vec<WindowId> = std::iter::once(WindowId::PRIMARY)
                    .chain(previous.keys().copied())
                    .collect();
                let mut rebuilt = HashMap::new();
                let mut failed = Vec::new();
                for (id, mut host) in previous {
                    let window = Arc::clone(host.surface.window());
                    host.material = apply_window_surface(
                        window.as_ref(),
                        self.last_theme,
                        &host.settings,
                        self.program.window_material_mode_for(id),
                        self.program.appearance_backdrop_opacity_for(id),
                    );
                    match graphics.recreate_surface(&host.surface) {
                        Ok(surface) => {
                            let format = surface.format();
                            painters.entry(format).or_insert_with(|| {
                                SceneWgpuPainter::new(
                                    graphics.resources().device(),
                                    graphics.resources().queue(),
                                    format,
                                )
                            });
                            host.surface = surface;
                            rebuilt.insert(id, host);
                        }
                        Err(_) => failed.push((id, host.surface.window().id())),
                    }
                }
                self.graphics = graphics;
                self.painters = painters;
                self.install_image_wakers();
                self.native_renderers.clear();
                self.auxiliary = rebuilt;
                self.refresh_material();
                self.next_gpu_retry = None;
                self.render_suspended = false;
                invalidate_program_host_textures(recovery_windows, |id| {
                    self.program.host_textures(id)
                });
                self.program.rebuild_gpu(&self.context());
                for (id, window_id) in failed {
                    self.window_ids.remove(&window_id);
                    let update = self
                        .program
                        .window_event(WindowEvent::Closed { id }, &self.context_for(id));
                    self.apply_update(event_loop, update, None);
                    if event_loop.exiting() {
                        return;
                    }
                }
                self.request_redraw_all();
            }
            Err(_) => {
                self.render_suspended = true;
                self.next_gpu_retry = Some(Instant::now() + GPU_RETRY_INTERVAL);
            }
        }
    }
    pub(super) fn suspend_rendering(&mut self, _error: HostedGpuError) {
        self.render_suspended = true;
        self.next_gpu_retry = Some(Instant::now() + GPU_RETRY_INTERVAL);
    }
    pub(super) fn painter_mut(&mut self, format: wgpu::TextureFormat) -> &mut SceneWgpuPainter {
        let resources = self.graphics.resources();
        let painter = self.painters.entry(format).or_insert_with(|| {
            SceneWgpuPainter::new(resources.device(), resources.queue(), format)
        });
        let targets = Arc::clone(&self.image_targets);
        let redraws = Arc::clone(&self.texture_redraws);
        let proxy = self.proxy.clone();
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
    }
}
