//! Scene host present coordination.

use super::*;

impl<Program: RuntimeProgram> SceneReady<Program> {
    pub(super) fn redraw(&mut self, event_loop: &dyn ActiveEventLoop, id: WindowId) {
        if self.render_suspended {
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
        let pending = if !update.accessibility.updated.is_empty()
            || !update.accessibility.removed.is_empty()
        {
            Some(AccessibilityUpdate::Delta(update.accessibility))
        } else {
            None
        };
        let Some(scene) = self
            .program
            .write_document(id, |document| document.shared_scene())
        else {
            self.program
                .host_failure(HostFailure::MissingDocument { window: id });
            return;
        };
        if let Some(pending) = pending {
            *self.accessibility_pending_mut(id) = Some(pending);
        }
        if let Some(producers) = self.program.scene_resource_producers(id)
            && let Err(error) = producers.encode_scene(
                scene.as_ref(),
                self.graphics.resources().device(),
                self.graphics.resources().queue(),
            )
        {
            self.program.host_failure(HostFailure::ResourceProduction {
                window: id,
                error: error.to_string(),
            });
            return;
        }
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
        let host_textures = self.program.host_textures(id);
        let gpu_renderers = resolve_scene_gpu_renderers(
            self.program.scene_gpu_renderers(id),
            self.default_scene_gpu_renderers.clone(),
        );
        let theme = self.program.theme_mode();
        let paint = self.painter_mut(format).paint(
            scene.as_ref(),
            &mut encoder,
            &target,
            scene_paint_viewport(&geometry, material, theme),
            host_textures.as_ref(),
            gpu_renderers.as_ref(),
        );
        if let Err(error) = paint {
            self.program.host_failure(HostFailure::UnpaintableScene {
                window: id,
                error: error.to_string(),
            });
            self.request_redraw(id);
            return;
        }
        let submit_started = std::time::Instant::now();
        self.graphics.resources().queue().submit([encoder.finish()]);
        self.painter_mut(format)
            .record_submit(submit_started.elapsed());
        self.graphics.present(frame);
        self.apply_ime_request(id);
        let mut update = self
            .program
            .window_frame_presented(id, &self.context_for(id));
        if self.bind_after_present.remove(&id) {
            update = update.merge(self.program.bind_window(id, &self.context_for(id)));
        }
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
        #[cfg(not(target_os = "android"))]
        if !self.is_live_resize(id) {
            self.synchronize_accessibility(id);
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
            self.settings.transparent,
            self.last_material_mode,
            self.program.appearance_backdrop_opacity(),
        );
        match pollster::block_on(HostedGpuContext::new(
            window,
            wgpu::Features::empty(),
            window_wants_transparent_surface(self.settings.transparent, self.last_material_mode),
        )) {
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
                        host.settings.transparent,
                        self.last_material_mode,
                        self.program.appearance_backdrop_opacity(),
                    );
                    match graphics.create_surface(
                        window,
                        window_wants_transparent_surface(
                            host.settings.transparent,
                            self.last_material_mode,
                        ),
                    ) {
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
                for painter in painters.values_mut() {
                    let proxy = self.proxy.clone();
                    painter.set_image_waker(Arc::new(move || proxy.wake_up()));
                }
                self.painters = painters;
                self.auxiliary = rebuilt;
                self.default_scene_gpu_renderers = Some(default_scene_gpu_renderers_with_host(
                    Arc::clone(self.graphics.resources().device()),
                    Arc::clone(self.graphics.resources().queue()),
                ));
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
        let proxy = self.proxy.clone();
        self.painters.entry(format).or_insert_with(|| {
            let mut painter = SceneWgpuPainter::new(resources.device(), resources.queue(), format);
            painter.set_image_waker(Arc::new(move || proxy.wake_up()));
            painter
        })
    }
}
