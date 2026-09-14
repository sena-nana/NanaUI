//! Scene host windows coordination.

use super::*;

impl<Program: RuntimeProgram> WindowManager<Program> {
    pub(super) fn apply_window_command(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        command: WindowCommand,
    ) {
        let known = self.known_window_ids();
        match route_window_command(&command, &known) {
            RoutedWindowCommand::SetMousePassthrough(id) => {
                let WindowCommand::SetMousePassthrough { enabled, .. } = command else {
                    return;
                };
                let _ = self.set_mouse_passthrough(event_loop, id, enabled);
            }
            RoutedWindowCommand::Ignore => {}
            RoutedWindowCommand::Open(id) => {
                let WindowCommand::Open { settings, .. } = command else {
                    return;
                };
                {
                    let event = self
                        .open_window(event_loop, id, settings)
                        .unwrap_or_else(|error| WindowEvent::OpenFailed { id, error });
                    let update = self.program.window_event(event, &self.context_for(id));
                    self.program
                        .sync_animation_clock(self.animation_clock.epoch());
                    self.apply_update(event_loop, update, None);
                }
            }
            RoutedWindowCommand::Focus(id) => self.focus_window(id),
            RoutedWindowCommand::Close(id) => self.close_window(event_loop, id),
            RoutedWindowCommand::SetTitle(id) => {
                let WindowCommand::SetTitle { title, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id) {
                    window.set_title(&title);
                }
            }
            RoutedWindowCommand::Move(id) => {
                let WindowCommand::Move { position, .. } = command else {
                    return;
                };
                self.move_window(id, position);
            }
            RoutedWindowCommand::SetBounds(id) => {
                let WindowCommand::SetBounds { position, size, .. } = command else {
                    return;
                };
                self.set_window_bounds(id, position, size);
            }
            RoutedWindowCommand::SetFullscreen(id) => {
                let WindowCommand::SetFullscreen { fullscreen, .. } = command else {
                    return;
                };
                self.mutate_native_style(id, |window| {
                    window.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
                });
            }
            RoutedWindowCommand::SetSimpleFullscreen(id) => {
                let WindowCommand::SetSimpleFullscreen { fullscreen, .. } = command else {
                    return;
                };
                self.mutate_native_style(id, |window| {
                    #[cfg(target_os = "macos")]
                    window.set_simple_fullscreen(fullscreen);
                    #[cfg(not(target_os = "macos"))]
                    window.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
                });
            }
            RoutedWindowCommand::SetMinimized(id) => {
                let WindowCommand::SetMinimized { minimized, .. } = command else {
                    return;
                };
                self.mutate_native_style(id, |window| window.set_minimized(minimized));
            }
            RoutedWindowCommand::SetMaximized(id) => {
                let WindowCommand::SetMaximized { maximized, .. } = command else {
                    return;
                };
                if self
                    .mutate_native_style(id, |window| window.set_maximized(maximized))
                    .is_none()
                {
                    return;
                }
                self.resize_window(id);
                // The cached geometry is now current, so the native resize that
                // follows will not request this frame itself.
                if self.sync_geometry(id) {
                    self.request_redraw(id);
                }
                let update = self.program.window_event(
                    WindowEvent::Resized {
                        id,
                        geometry: self.geometry_of(id),
                    },
                    &self.context_for(id),
                );
                self.apply_update(event_loop, update, None);
            }
            RoutedWindowCommand::SetAlwaysOnTop(id) => {
                let WindowCommand::SetAlwaysOnTop { always_on_top, .. } = command else {
                    return;
                };
                self.mutate_native_style(id, |window| {
                    window.set_window_level(window_level(always_on_top));
                });
            }
            RoutedWindowCommand::SetIcon(id) => {
                let WindowCommand::SetIcon { icon, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id) {
                    apply_scene_window_icon(
                        window.as_ref(),
                        icon.as_ref(),
                        id == WindowId::PRIMARY,
                    );
                }
            }
            RoutedWindowCommand::SetMenuBar(id) => {
                let WindowCommand::SetMenuBar { bar, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id) {
                    // The host owns the handle; the application only described
                    // the menu. An empty bar removes it.
                    let bar = bar.clone().unwrap_or_default();
                    nana_window::install_menu_bar(window.as_ref(), &bar);
                }
            }
            RoutedWindowCommand::OpenFileDialog(id) => {
                let WindowCommand::OpenFileDialog { request, .. } = command else {
                    return;
                };
                self.open_file_dialog(event_loop, id, request);
            }
            RoutedWindowCommand::SetApplicationIcon => {
                let WindowCommand::SetApplicationIcon { icon } = command else {
                    return;
                };
                match icon {
                    Some(icon) => register_application_icon(icon),
                    None => clear_registered_application_icon(),
                }
                for id in self.known_window_ids() {
                    if let Some(window) = self.window(id) {
                        apply_scene_window_icon(window.as_ref(), None, id == WindowId::PRIMARY);
                    }
                }
                apply_application_icon(&nana_app_icon::resolved_application_icon(None));
            }
            RoutedWindowCommand::Drag(id) => {
                if let Some(window) = self.window(id) {
                    drag_scene_window(window.as_ref());
                }
            }
        }
    }
    fn set_mouse_passthrough(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        enabled: bool,
    ) -> Result<(), crate::WindowError> {
        let result = self
            .mutate_native_style(id, |window| {
                window
                    .set_cursor_hittest(!enabled)
                    .map_err(window_request_error)
            })
            .unwrap_or(Err(crate::WindowError::WindowClosed));
        let update = self.program.window_event(
            WindowEvent::MousePassthroughChanged {
                id,
                enabled,
                result: result.as_ref().copied().map_err(ToString::to_string),
            },
            &self.context_for(id),
        );
        self.apply_update(event_loop, update, None);
        result
    }

    pub(super) fn open_window(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        settings: WindowDescriptor,
    ) -> Result<WindowEvent, String> {
        if self.shutting_down {
            return Err("window host has stopped".into());
        }
        if self.render_suspended {
            return Err("shared GPU recovery pending".into());
        }
        if self.window(id).is_some() {
            return Err("window identity is already live".into());
        }
        crate::window_service::validate_descriptor(&settings).map_err(|error| error.to_string())?;
        if let Some(parent) = settings.parent
            && (self.window(parent).is_none() || self.closing_windows.contains(&parent))
        {
            return Err("parent window does not exist".into());
        }
        if settings.modal {
            let parent = settings
                .parent
                .ok_or_else(|| "modal window requires a parent".to_string())?;
            if self.window(parent).is_none() {
                return Err(format!("modal parent window {} does not exist", parent.0));
            }
            if self.active_modal_child(parent).is_some() {
                return Err(format!(
                    "modal parent window {} is already blocked",
                    parent.0
                ));
            }
        }
        let parent = settings
            .parent
            .and_then(|parent| self.window(parent).cloned());
        let attributes = scene_aux_window_attributes(
            &settings,
            parent.as_deref(),
            &scene_display_bounds_with_work_area(event_loop, settings.constrain_to_work_area),
        )?;
        let window: Arc<dyn winit::window::Window> = Arc::from(
            event_loop
                .create_window(attributes.with_visible(false))
                .map_err(|error| error.to_string())?,
        );
        let mut pending_native = PendingNativeWindow(Some(window.clone()));
        apply_scene_window_icon(
            window.as_ref(),
            settings.icon.as_ref(),
            id == WindowId::PRIMARY,
        );
        let material = apply_window_surface(
            window.as_ref(),
            self.last_theme,
            &settings,
            self.program.window_material_mode_for(id),
            self.program.appearance_backdrop_opacity_for(id),
        );
        let surface = self
            .graphics
            .create_surface_with_mode(
                Arc::clone(&window),
                window_wants_transparent_surface(
                    settings.transparent,
                    self.program.window_material_mode_for(id),
                ),
                Program::surface_mode(),
            )
            .map_err(|error| error.to_string())?;
        let format = surface.format();
        let _ = self.painter_mut(format);
        #[cfg(not(target_os = "android"))]
        let accessibility = {
            Some(HostedAccessibility::new(
                Arc::clone(&window),
                true,
                window.scale_factor() as f32,
            ))
        };
        let geometry = window_geometry(window.as_ref());
        #[cfg(target_os = "windows")]
        let modal_parent = settings.modal.then_some(settings.parent).flatten();
        let size_move = LiveSizeMove::install(window.as_ref())?;
        self.windows.register(id);
        let context = program_context(
            self.message_tx.clone(),
            Arc::clone(&self.host_work),
            &self.graphics,
            id,
            geometry,
            self.tasks.clone(),
            material,
            surface.alpha_mode(),
            window.theme().map(system_appearance_from_winit),
        )
        .with_windows(&self.windows);
        if let Err(error) = self.program.initialize_window(id, &context) {
            self.windows.unregister(id);
            self.program.discard_window(id);
            // PendingNativeWindow owns platform cleanup for every failed stage.
            return Err(error);
        }
        pending_native.0 = None;
        self.window_ids.insert(window.id(), id);
        self.window_contexts.insert(
            id,
            WindowContext {
                surface_retry: None,
                applied_appearance: None,
                cursor_override: None,
                cursor_visible_override: None,
                material_override: None,
                surface,
                geometry,
                input: InputTracker::default(),
                material,
                settings,
                #[cfg(not(target_os = "android"))]
                accessibility,
                accessibility_pending: PendingAccessibility::default(),
                size_move,
            },
        );
        #[cfg(target_os = "windows")]
        if let Some(parent) = modal_parent.and_then(|parent| self.window(parent)) {
            parent.set_enable(false);
        }
        self.mutate_native_style(id, |window| {
            window.set_visible(self.settings_of(id).visible)
        });
        window.request_redraw();
        self.prepare_window_chrome(id, geometry.maximized);
        Ok(WindowEvent::Ready { id, geometry })
    }
    pub(super) fn close_window(&mut self, event_loop: &dyn ActiveEventLoop, id: WindowId) {
        if !self.window_contexts.contains_key(&id) || !self.closing_windows.insert(id) {
            return;
        }
        self.windows.unregister(id);
        let children: Vec<_> = self
            .window_contexts
            .iter()
            .filter_map(|(&child, host)| (host.settings.parent == Some(id)).then_some(child))
            .collect();
        for child in children {
            self.close_window(event_loop, child);
        }
        if let Some(window) = self.window(id) {
            window.set_visible(false);
        }
        self.browsers.retain(|(window, _), _| *window != id);
        self.close_file_dialog(event_loop, id);
        self.chrome.remove(&id);
        self.bind_after_present.remove(&id);
        #[cfg(target_os = "macos")]
        self.present_transaction_pinned.remove(&id);
        self.frame_schedules.remove(&id);
        self.texture_subscriptions.remove(&id);
        if let Ok(mut targets) = self.image_targets.lock() {
            remove_image_target_index(&mut targets, &mut self.image_window_keys, id);
        } else {
            self.image_window_keys.remove(&id);
        }
        self.occluded.remove(&id);
        for painter in self.painters.values_mut() {
            painter.remove_target(crate::RenderTargetId(id.0));
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some((_, live)) = self
            .live_frame_resize
            .take_if(|(session, _)| *session == id)
            && let Some(window) = self.window(id)
        {
            live.end(window.as_ref());
        }
        // Cleared before `Closed` runs, so a window that callback reopens with the
        // same identity is not treated as closing.
        self.closing_windows.remove(&id);
        if let Some(host) = self.window_contexts.remove(&id) {
            #[cfg(target_os = "windows")]
            if let Some(parent_id) = host
                .settings
                .modal
                .then_some(host.settings.parent)
                .flatten()
                && !self.closing_windows.contains(&parent_id)
                // A modal opened while this one was closing still blocks the parent.
                && self.active_modal_child(parent_id).is_none()
                && let Some(parent) = self.window(parent_id)
            {
                parent.set_enable(true);
                self.focus_window(parent_id);
            }
            self.window_ids.remove(&host.surface.window().id());
            self.ime.remove(&id);
            drop(host);
            let update = self
                .program
                .window_event(WindowEvent::Closed { id }, &self.context_for(id));
            self.apply_update(event_loop, update, None);
        }
        if self.window_contexts.is_empty() && !self.embedded {
            event_loop.exit();
        }
    }
    pub(super) fn focus_window(&self, mut id: WindowId) {
        while let Some(modal) = self.active_modal_child(id) {
            id = modal;
        }
        self.mutate_native_style(id, |window| window.set_visible(true));
        if let Some(window) = self.window(id) {
            window.focus_window();
        }
    }
    pub(super) fn move_window(&self, id: WindowId, position: (f32, f32)) {
        self.mutate_native_style(id, |window| {
            window.set_outer_position(winit::dpi::Position::Logical(
                winit::dpi::LogicalPosition::new(f64::from(position.0), f64::from(position.1)),
            ));
        });
    }
    pub(super) fn set_window_bounds(&self, id: WindowId, position: (f32, f32), size: (f32, f32)) {
        self.mutate_native_style(id, |window| {
            window.set_outer_position(winit::dpi::Position::Logical(
                winit::dpi::LogicalPosition::new(f64::from(position.0), f64::from(position.1)),
            ));
            let _ = window.request_surface_size(winit::dpi::Size::Logical(
                winit::dpi::LogicalSize::new(
                    f64::from(size.0.max(1.0)),
                    f64::from(size.1.max(1.0)),
                ),
            ));
        });
    }
    pub(super) fn active_modal_child(&self, parent: WindowId) -> Option<WindowId> {
        self.window_contexts.iter().find_map(|(id, host)| {
            (host.settings.modal
                && host.settings.parent == Some(parent)
                && !self.closing_windows.contains(id))
            .then_some(*id)
        })
    }
    pub(super) fn sync_appearance(&mut self) {
        self.last_theme = self.program.theme_mode();
        if self.render_suspended {
            return;
        }
        for id in self.known_window_ids() {
            if self.window_contexts[&id].surface_retry.is_some() {
                continue;
            }
            if let Err(error) = self.sync_window_material(id) {
                self.suspend_surface(id, error);
            }
        }
    }

    pub(super) fn sync_window_material(&mut self, id: WindowId) -> Result<(), HostedGpuError> {
        let material_override = self
            .window_contexts
            .get(&id)
            .and_then(|host| host.material_override);
        self.apply_window_material(id, material_override)
    }

    fn apply_window_material(
        &mut self,
        id: WindowId,
        material_override: Option<nana_window::MaterialEffect>,
    ) -> Result<(), HostedGpuError> {
        let mode = self.program.window_material_mode_for(id);
        let desired = WindowAppearance {
            theme: self.program.theme_mode(),
            material: material_override.unwrap_or(mode),
            opacity: AppearanceSettings::clamp_backdrop_opacity(
                self.program.appearance_backdrop_opacity_for(id),
            ),
        };
        let Some(host) = self.window_contexts.get_mut(&id) else {
            return Ok(());
        };
        let outcome = apply_changed_appearance(&mut host.applied_appearance, desired, || {
            clear_system_material(host.surface.window().as_ref());
            let outcome = apply_window_surface(
                host.surface.window().as_ref(),
                desired.theme,
                &host.settings,
                desired.material,
                desired.opacity,
            );
            self.graphics.apply_surface_alpha_mode(
                &mut host.surface,
                window_wants_transparent_surface(host.settings.transparent, desired.material),
            )?;
            Ok(outcome)
        })?;
        if let Some(outcome) = outcome {
            host.material = outcome;
            self.request_redraw(id);
        }
        Ok(())
    }

    /// GPU replacement requires reapplying effects even when the request is unchanged.
    pub(super) fn refresh_material(&mut self) {
        for host in self.window_contexts.values_mut() {
            host.applied_appearance = None;
        }
        self.sync_appearance();
    }
    /// Refreshes the cached window geometry from the live window state and
    /// reports whether it moved.
    pub(super) fn sync_geometry(&mut self, id: WindowId) -> bool {
        let previous = self.geometry_of(id);
        if let Some(host) = self.window_contexts.get_mut(&id) {
            host.geometry = window_geometry(host.surface.window().as_ref());
        }
        let changed = self.geometry_of(id) != previous;
        let maximized = self.geometry_of(id).maximized;
        if let Some(session) = self.chrome.get_mut(&id) {
            session.state.update(WindowChromeEvent::MaximizedChanged {
                window: id,
                maximized,
            });
        }
        // Title bars mirror only maximize transitions; moves and plain resizes
        // leave them alone, and optimistic chrome toggles are not overwritten.
        if maximized != previous.maximized {
            self.sync_title_bar_maximized(id, maximized);
        }
        changed
    }
    /// Pins transaction presents while the OS's own frame-resize gesture is
    /// moving the window, and reports whether that gesture is active.
    #[cfg(target_os = "macos")]
    pub(super) fn sync_native_live_resize_presents(&mut self, id: WindowId) -> bool {
        let active = self
            .window(id)
            .is_some_and(|window| nana_window::native_live_resize_active(window.as_ref()));
        if active {
            self.pin_present_transaction(id);
        }
        active
    }
    #[cfg(target_os = "macos")]
    pub(super) fn pin_present_transaction(&mut self, id: WindowId) {
        if self.present_transaction_pinned.contains(&id) {
            return;
        }
        if let Some(window) = self.window(id)
            && nana_window::set_present_transaction(window.as_ref(), true)
        {
            self.present_transaction_pinned.insert(id);
        }
    }
    /// Releases transaction presents once their resize gesture is over; the
    /// pinned mode serializes every present with a Core Animation commit and
    /// costs latency in steady-state frames.
    #[cfg(target_os = "macos")]
    pub(super) fn unpin_idle_present_transactions(&mut self) {
        let pinned: Vec<WindowId> = self.present_transaction_pinned.iter().copied().collect();
        for id in pinned {
            let Some(window) = self.window(id) else {
                self.present_transaction_pinned.remove(&id);
                continue;
            };
            if nana_window::native_live_resize_active(window.as_ref()) || self.is_live_resize(id) {
                continue;
            }
            nana_window::set_present_transaction(window.as_ref(), false);
            self.present_transaction_pinned.remove(&id);
        }
    }
    pub(super) fn resize_window(&mut self, id: WindowId) {
        if self.render_suspended {
            return;
        }
        let live = self.is_live_resize(id);
        if let Some(host) = self.window_contexts.get_mut(&id)
            && host.surface_retry.is_none()
        {
            self.graphics.prepare_surface_frame(&mut host.surface, live);
        }
    }
    pub(super) fn is_live_resize(&self, id: WindowId) -> bool {
        if self.size_move_active(id) {
            return true;
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            self.live_frame_resize
                .as_ref()
                .is_some_and(|(session, _)| *session == id)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        false
    }
    pub(super) fn size_move_active(&self, id: WindowId) -> bool {
        self.window_contexts
            .get(&id)
            .is_some_and(|host| host.size_move.is_active())
    }
    /// Program callbacks earlier in the same dispatch may have closed `id`.
    pub(super) fn sync_window_cursor(&mut self, id: WindowId) {
        let Some(host) = self.window_contexts.get_mut(&id) else {
            return;
        };
        if host.input.begin_cursor_sync(std::time::Instant::now()) {
            self.sync_window_cursor_now(id);
        }
    }

    /// Refresh the cursor after a document flush even when pointer-driven
    /// synchronization ran moments earlier in the same frame.
    pub(super) fn sync_window_cursor_forced(&mut self, id: WindowId) {
        // Treat the forced probe as the latest sync so a pointer event in the
        // same frame does not immediately repeat the document walk.
        let Some(host) = self.window_contexts.get_mut(&id) else {
            return;
        };
        host.input.cursor_sync_last = Some(std::time::Instant::now());
        self.sync_window_cursor_now(id);
    }

    /// Restore the native cursor after the pointer leaves this window. This
    /// must not probe the document: the last in-window target may have had
    /// `cursor:none`, and that state must not leak outside the window.
    pub(super) fn reset_window_cursor(&self, id: WindowId) {
        if let Some(window) = self.window(id) {
            window.set_cursor_visible(true);
            window.set_cursor(CursorIcon::Default.into());
        }
    }

    fn sync_window_cursor_now(&mut self, id: WindowId) {
        let cursor = self.input_of(id).cursor;
        let frame_edge = self.frame_resize_edge_at(id, cursor.0, cursor.1);
        let (handle, css_cursor, text_field) = self
            .program
            .read_document(id, |document| {
                let context = document.context();
                let document_id = document.document();
                let handle = context
                    .split_handle_near(document_id, cursor.0, cursor.1)
                    .or_else(|| context.dock_handle_near(document_id, cursor.0, cursor.1))
                    .or_else(|| context.workspace_handle_near(document_id, cursor.0, cursor.1))
                    .and_then(|handle| context.world().layout_box(handle))
                    .map(|bounds| (bounds.width, bounds.height));
                let target = context.pointer_target(document_id, cursor.0, cursor.1);
                let text_field =
                    target.is_some_and(|node| context.world().text_input(node).is_some());
                let css_cursor = target
                    .and_then(|node| context.world().computed_style(node))
                    .and_then(|style| style.cursor_specified.then_some(style.cursor));
                (handle, css_cursor, text_field)
            })
            .unwrap_or((None, None, false));
        if let Some(window) = self.window(id) {
            let (icon, visible) = scene_cursor_icon(frame_edge, handle, css_cursor, text_field);
            let host = self.window_contexts.get(&id).unwrap();
            let icon = host.cursor_override.unwrap_or(icon);
            let visible = host.cursor_visible_override.unwrap_or(visible);
            window.set_cursor_visible(visible);
            if visible {
                window.set_cursor(icon.into());
            }
        }
    }
    pub(super) fn consume_frame_resize(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        input: &InputEvent,
    ) -> bool {
        // The live frame-resize session exists only on macOS and Windows; the
        // rest of this function never touches the event loop.
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = event_loop;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some((session, live)) = self.live_frame_resize
            && session == id
        {
            // The pinned winit win32 proc synthesizes `PointerLeft` from a
            // client-rect bounds check even while the drag is captured, so a
            // fast drag crossing the window edge arrives as `Cancel` and must
            // not end the session; only Up, a fresh primary press (lost Up),
            // or focus loss does.
            match input {
                InputEvent::Pointer {
                    phase: PointerPhase::Move,
                    ..
                } => {
                    if let Some(window) = self.window(id) {
                        let _ = live.update(window.as_ref());
                    }
                    // `setFrame` from inside this pointer dispatch leaves
                    // winit's `SurfaceResized` queued for the next run-loop
                    // pass, and a redraw that waits for it lets the compositor
                    // composite the moved frame with the old drawable
                    // stretched. Sync geometry and paint in this stack, like
                    // the native live-resize path already does.
                    self.sync_geometry(id);
                    self.redraw(event_loop, id);
                    self.sync_window_cursor(id);
                    return true;
                }
                InputEvent::Pointer {
                    phase: PointerPhase::Cancel,
                    ..
                } => {
                    self.sync_window_cursor(id);
                    return true;
                }
                InputEvent::Pointer {
                    phase: PointerPhase::Up,
                    ..
                } => {
                    self.end_live_frame_resize(id);
                    self.request_redraw(id);
                    self.sync_window_cursor(id);
                    return true;
                }
                InputEvent::Pointer {
                    phase: PointerPhase::Down,
                    button: 0,
                    is_primary: true,
                    ..
                } => self.end_live_frame_resize(id),
                _ => {}
            }
        }
        let InputEvent::Pointer {
            phase: PointerPhase::Down,
            button: 0,
            is_primary: true,
            x,
            y,
            ..
        } = input
        else {
            return false;
        };
        let Some(edge) = self.frame_resize_edge_at(id, *x, *y) else {
            return false;
        };
        self.start_frame_resize(id, edge);
        true
    }
    /// Ends the live frame resize for `id` if one is running, releasing the
    /// mouse capture.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(super) fn end_live_frame_resize(&mut self, id: WindowId) {
        if let Some((_, live)) = self
            .live_frame_resize
            .take_if(|(session, _)| *session == id)
            && let Some(window) = self.window(id)
        {
            live.end(window.as_ref());
        }
    }
    pub(super) fn start_frame_resize(&mut self, id: WindowId, edge: WindowResizeEdge) {
        let Some(window) = self.window(id).cloned() else {
            return;
        };
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            if let Some(live) =
                nana_window::LiveFrameResize::begin(window.as_ref(), frame_resize_edge(edge))
            {
                self.live_frame_resize = Some((id, live));
                // Apply the live present policy before the first moved frame.
                // The mode switch reconfigures the swapchain; paying it on the
                // gesture's first redraw would stall exactly the frame the
                // window starts following the pointer. The native size-move
                // path does not need this: its ENTER hook forces a repaint
                // before the first size change.
                self.resize_window(id);
                #[cfg(target_os = "macos")]
                self.pin_present_transaction(id);
                return;
            }
        }
        resize_scene_window(window.as_ref(), edge);
    }
    pub(super) fn frame_resize_edge_at(
        &mut self,
        id: WindowId,
        x: f32,
        y: f32,
    ) -> Option<WindowResizeEdge> {
        let fullscreen = self
            .window(id)
            .is_some_and(|window| window.fullscreen().is_some());
        let edge = frame_resize_edge_for(
            self.settings_of(id),
            &self.geometry_of(id),
            fullscreen,
            x,
            y,
        )?;
        if self.caption_control_at(id, x, y) {
            return None;
        }
        Some(edge)
    }
    fn caption_control_at(&mut self, id: WindowId, x: f32, y: f32) -> bool {
        self.program
            .read_document(id, |document| {
                pointer_hits_window_control(document.context(), document.document(), x, y)
            })
            .unwrap_or(false)
    }
    pub(super) fn settings_of(&self, id: WindowId) -> &WindowDescriptor {
        self.window_contexts
            .get(&id)
            .map(|host| &host.settings)
            .unwrap_or(&self.settings)
    }

    fn mutate_native_style<R>(
        &self,
        id: WindowId,
        mutate: impl FnOnce(&dyn winit::window::Window) -> R,
    ) -> Option<R> {
        let result = self.window(id).map(|window| mutate(window.as_ref()));
        if result.is_some() {
            self.restore_client_chrome(id);
        }
        result
    }

    fn restore_client_chrome(&self, id: WindowId) {
        let Some(host) = self.window_contexts.get(&id) else {
            return;
        };
        apply_client_chrome_after_create(host.surface.window().as_ref(), &host.settings);
    }
    pub(super) fn scale_factor(&self, id: WindowId) -> f32 {
        self.window(id)
            .map(|window| normalized_scale_factor(window.scale_factor() as f32))
            .unwrap_or(1.0)
    }
    pub(super) fn title_bar_chrome_action(
        &mut self,
        id: WindowId,
        input: &InputEvent,
    ) -> Option<WindowChromeAction> {
        let program = &mut self.program;
        let chrome = &mut self.chrome;
        program
            .read_document(id, |document| {
                let session = chrome
                    .entry(id)
                    .or_insert_with(|| WindowChromeSession::new(id));
                apply_title_bar_pointer(
                    &mut session.state,
                    &mut session.drag,
                    document.context(),
                    document.document(),
                    input,
                )
            })
            .flatten()
    }
    pub(super) fn merge_title_bar_chrome(
        &mut self,
        id: WindowId,
        action: Option<WindowChromeAction>,
        mut update: RuntimeProgramUpdate,
    ) -> RuntimeProgramUpdate {
        let Some(action) = action else {
            return update;
        };
        let maximized = self
            .chrome
            .get(&id)
            .is_some_and(|session| session.state.is_maximized());
        if action == WindowChromeAction::ToggleMaximize {
            self.sync_title_bar_maximized(id, maximized);
        }
        update
            .window_commands
            .extend(window_commands_for_chrome_action(id, action, maximized));
        update
    }
    pub(super) fn prepare_window_chrome(&mut self, id: WindowId, maximized: bool) {
        let session = self
            .chrome
            .entry(id)
            .or_insert_with(|| WindowChromeSession::new(id));
        session.state.update(WindowChromeEvent::PrepareWindow(id));
        session.state.update(WindowChromeEvent::MaximizedChanged {
            window: id,
            maximized,
        });
        self.sync_title_bar_maximized(id, maximized);
    }
    /// Align the window's title bars with its maximized state through the
    /// component index, touching only bars that differ.
    pub(super) fn sync_title_bar_maximized(&mut self, id: WindowId, maximized: bool) {
        self.program.write_document(id, |document| {
            let document_id = document.document();
            let context = document.context_mut();
            let stale = context
                .world()
                .nodes_of_component(
                    document_id,
                    nana_ui_runtime::component_descriptors::APP_TITLE_BAR.type_id,
                )
                .filter(|&node| {
                    context
                        .read(Entity::<AppTitleBar>::from_stable_id(node), |bar| {
                            bar.maximized != maximized
                        })
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>();
            for bar in stale {
                let _ = context.update_component(
                    Entity::<AppTitleBar>::from_stable_id(bar),
                    |bar, _| {
                        bar.maximized = maximized;
                    },
                );
            }
        });
    }
}

impl<Program: RuntimeProgram> WindowManager<Program> {
    pub(super) fn drain_window_requests(&mut self, event_loop: &dyn ActiveEventLoop) {
        use crate::window_service::Request;
        let remaining = super::schedule::drain_host_batch(
            || {
                if self.shutting_down || event_loop.exiting() {
                    return false;
                }
                let Some(request) = self
                    .window_requests
                    .as_ref()
                    .and_then(|requests| requests.try_recv().ok())
                else {
                    return false;
                };
                match request {
                    Request::Displays(reply) => {
                        reply.finish(Ok(super::display::display_infos(event_loop)));
                    }
                    Request::Material(id, generation, effect, reply) => {
                        if !self.windows.is_current(id, generation) {
                            reply.finish(Err(crate::WindowError::WindowClosed));
                            return true;
                        }
                        if let Some(host) = self.window_contexts.get(&id)
                            && (self.render_suspended || host.surface_retry.is_some())
                        {
                            reply.finish(Err(crate::WindowError::OperationFailed(
                                "window surface recovery pending".into(),
                            )));
                            return true;
                        }
                        match self.apply_window_material(id, Some(effect)) {
                            Ok(()) => {
                                self.window_contexts.get_mut(&id).unwrap().material_override =
                                    Some(effect);
                                reply.finish(Ok(self.material_of(id)));
                            }
                            Err(error) => {
                                let message = error.to_string();
                                self.suspend_surface(id, error);
                                reply.finish(Err(crate::WindowError::OperationFailed(message)));
                            }
                        }
                    }
                    Request::Create(settings, reply) => {
                        reply.finish(self.create_service_window(event_loop, settings));
                    }
                    Request::Native(id, generation, callback) => {
                        if !self.windows.is_current(id, generation) {
                            callback(Err(crate::WindowError::WindowClosed));
                            return true;
                        }
                        use raw_window_handle::HasWindowHandle;
                        match self.window(id) {
                            Some(window) => callback(
                                window
                                    .window_handle()
                                    .map_err(|e| crate::WindowError::Unsupported(e.to_string())),
                            ),
                            None => callback(Err(crate::WindowError::WindowClosed)),
                        }
                    }
                    Request::Control(id, generation, control, reply) => {
                        if !self.windows.is_current(id, generation) {
                            reply.finish(Err(crate::WindowError::WindowClosed));
                            return true;
                        }
                        reply.finish(self.control_window(event_loop, id, control));
                    }
                }
                !self.shutting_down && !event_loop.exiting()
            },
            Instant::now,
        );
        if remaining {
            self.host_work.wake();
        }
    }

    /// Allocate a host-owned identity and open a fully initialized window.
    pub(super) fn create_service_window(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        settings: WindowDescriptor,
    ) -> Result<crate::WindowHandle, crate::WindowError> {
        use crate::WindowError;
        if self.shutting_down {
            return Err(WindowError::HostStopped);
        }
        // Skip program-chosen identities (such as hashed dock windows) already live here.
        let id = loop {
            let id = WindowId(self.next_window_id);
            self.next_window_id = self.next_window_id.checked_add(1).ok_or_else(|| {
                WindowError::InitializationFailed("window identities exhausted".into())
            })?;
            if !self.window_contexts.contains_key(&id) {
                break id;
            }
        };
        let event = self
            .open_window(event_loop, id, settings)
            .map_err(WindowError::InitializationFailed)?;
        let update = self.program.window_event(event, &self.context_for(id));
        self.apply_update(event_loop, update, None);
        // `Ready` has been delivered, so creation itself succeeded; a window its
        // own `Ready` handling closed resolves as closed, not as a failed creation.
        if self.shutting_down {
            Err(WindowError::HostStopped)
        } else if self.window(id).is_some() {
            Ok(self.windows.handle(id))
        } else {
            Err(WindowError::WindowClosed)
        }
    }

    fn control_window(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        control: crate::window_service::Control,
    ) -> Result<(), crate::WindowError> {
        use crate::window_service::{Control, validate_size};
        use crate::{WindowCursor, WindowError, WindowLevel};
        let window = self.window(id).cloned().ok_or(WindowError::WindowClosed)?;
        match control {
            Control::Visible(visible) => {
                self.mutate_native_style(id, |window| window.set_visible(visible));
                // Frames were deferred while hidden; a shown window repaints
                // without relying on the program to request it.
                if visible {
                    self.request_redraw(id);
                }
                let update = self.program.window_event(
                    WindowEvent::VisibilityChanged {
                        id,
                        hidden: !visible,
                    },
                    &self.context_for(id),
                );
                self.apply_update(event_loop, update, None);
            }
            Control::Size(size) => {
                validate_size(size)?;
                self.mutate_native_style(id, |window| {
                    window.request_surface_size(winit::dpi::LogicalSize::new(size.0, size.1).into())
                });
            }
            Control::MinSize(size) => {
                if let Some(size) = size {
                    validate_size(size)?;
                }
                self.mutate_native_style(id, |window| {
                    window.set_min_surface_size(
                        size.map(|s| winit::dpi::LogicalSize::new(s.0, s.1).into()),
                    )
                });
            }
            Control::MaxSize(size) => {
                if let Some(size) = size {
                    validate_size(size)?;
                }
                self.mutate_native_style(id, |window| {
                    window.set_max_surface_size(
                        size.map(|s| winit::dpi::LogicalSize::new(s.0, s.1).into()),
                    )
                });
            }
            Control::Resizable(value) => {
                self.mutate_native_style(id, |window| window.set_resizable(value));
            }
            Control::Level(level) => {
                self.mutate_native_style(id, |window| {
                    window.set_window_level(match level {
                        WindowLevel::Normal => winit::window::WindowLevel::Normal,
                        WindowLevel::AlwaysOnTop => winit::window::WindowLevel::AlwaysOnTop,
                        WindowLevel::AlwaysOnBottom => winit::window::WindowLevel::AlwaysOnBottom,
                    })
                });
            }
            Control::ContentProtected(protected) => {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                window.set_content_protected(protected);
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    let _ = protected;
                    return Err(WindowError::Unsupported("native content protection".into()));
                }
            }
            Control::CursorVisible(visible) => {
                self.window_contexts
                    .get_mut(&id)
                    .unwrap()
                    .cursor_visible_override = Some(visible);
                window.set_cursor_visible(visible);
            }
            Control::Cursor(cursor) => {
                let host = self.window_contexts.get_mut(&id).unwrap();
                host.cursor_override = match cursor {
                    WindowCursor::Automatic => {
                        host.cursor_visible_override = None;
                        None
                    }
                    WindowCursor::Default => Some(CursorIcon::Default),
                    WindowCursor::Pointer => Some(CursorIcon::Pointer),
                    WindowCursor::Text => Some(CursorIcon::Text),
                    WindowCursor::Move => Some(CursorIcon::Move),
                    WindowCursor::Grab => Some(CursorIcon::Grab),
                    WindowCursor::Grabbing => Some(CursorIcon::Grabbing),
                    WindowCursor::NotAllowed => Some(CursorIcon::NotAllowed),
                    WindowCursor::Crosshair => Some(CursorIcon::Crosshair),
                    WindowCursor::Wait => Some(CursorIcon::Wait),
                };
                self.sync_window_cursor_now(id);
            }
            Control::Redraw => window.request_redraw(),
            Control::Resize(edge) => {
                if !resize_custom_frame(window.as_ref(), frame_resize_edge(edge)) {
                    window
                        .drag_resize_window(match edge {
                            WindowResizeEdge::North => winit::window::ResizeDirection::North,
                            WindowResizeEdge::South => winit::window::ResizeDirection::South,
                            WindowResizeEdge::East => winit::window::ResizeDirection::East,
                            WindowResizeEdge::West => winit::window::ResizeDirection::West,
                            WindowResizeEdge::NorthEast => {
                                winit::window::ResizeDirection::NorthEast
                            }
                            WindowResizeEdge::NorthWest => {
                                winit::window::ResizeDirection::NorthWest
                            }
                            WindowResizeEdge::SouthEast => {
                                winit::window::ResizeDirection::SouthEast
                            }
                            WindowResizeEdge::SouthWest => {
                                winit::window::ResizeDirection::SouthWest
                            }
                        })
                        .map_err(window_request_error)?;
                }
            }
            Control::Command(WindowCommand::Drag(_)) => {
                window.drag_window().map_err(window_request_error)?;
            }
            Control::Command(WindowCommand::SetMousePassthrough { enabled, .. }) => {
                self.set_mouse_passthrough(event_loop, id, enabled)?;
            }
            Control::Command(WindowCommand::Move { position, .. })
                if !position.0.is_finite() || !position.1.is_finite() =>
            {
                return Err(WindowError::InvalidParameter(
                    "position must be finite".into(),
                ));
            }
            Control::Command(command) => self.apply_window_command(event_loop, command),
        }
        // Style-changing operations restore client chrome at their mutation
        // boundary. Redraw/cursor requests must not rewrite native chrome, and
        // commands already use the same boundary in apply_window_command.
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WindowAppearance {
    theme: crate::ThemeMode,
    material: nana_window::MaterialEffect,
    opacity: f32,
}

fn apply_changed_appearance(
    applied: &mut Option<WindowAppearance>,
    desired: WindowAppearance,
    apply: impl FnOnce() -> Result<MaterialOutcome, HostedGpuError>,
) -> Result<Option<MaterialOutcome>, HostedGpuError> {
    if *applied == Some(desired) {
        return Ok(None);
    }
    let outcome = apply()?;
    *applied = Some(desired);
    Ok(Some(outcome))
}

#[cfg(test)]
mod appearance_tests {
    use super::*;
    #[test]
    fn unchanged_windows_do_not_reapply_native_material() {
        let original = WindowAppearance {
            theme: crate::ThemeMode::Dark,
            material: nana_window::MaterialEffect::Solid,
            opacity: 1.0,
        };
        let mut windows = [Some(original); 3];
        let changed = WindowAppearance {
            opacity: 0.5,
            ..original
        };
        let mut calls = Vec::new();
        for (index, cache) in windows.iter_mut().enumerate() {
            let desired = if index == 1 { changed } else { original };
            let result = apply_changed_appearance(cache, desired, || {
                calls.push(index);
                Ok(MaterialOutcome::chosen_solid())
            })
            .unwrap();
            assert_eq!(result.is_some(), index == 1);
        }
        assert_eq!(calls, vec![1]);
        assert_eq!(windows[1], Some(changed));
        assert!(
            apply_changed_appearance(&mut windows[1], changed, || panic!(
                "unchanged material was reapplied"
            ))
            .unwrap()
            .is_none()
        );
        assert!(
            apply_changed_appearance(&mut windows[1], original, || Err(
                HostedGpuError::SurfaceValidation
            ))
            .is_err()
        );
        assert_eq!(windows[1], Some(changed));
    }
    #[test]
    fn theme_changes_and_device_replacement_invalidate_appearance() {
        let original = WindowAppearance {
            theme: crate::ThemeMode::Dark,
            material: nana_window::MaterialEffect::Solid,
            opacity: 1.0,
        };
        let mut cached = Some(original);
        let light = WindowAppearance {
            theme: crate::ThemeMode::Light,
            ..original
        };
        let calls = std::cell::Cell::new(0);
        let apply = |cache: &mut Option<WindowAppearance>, desired| {
            apply_changed_appearance(cache, desired, || {
                calls.set(calls.get() + 1);
                Err(HostedGpuError::SurfaceValidation)
            })
        };
        assert!(apply(&mut cached, light).is_err());
        cached = None;
        assert!(apply(&mut cached, original).is_err());
        assert_eq!(calls.get(), 2);
    }
}

fn window_request_error(error: winit::error::RequestError) -> crate::WindowError {
    match error {
        winit::error::RequestError::NotSupported(_) => {
            crate::WindowError::Unsupported(error.to_string())
        }
        _ => crate::WindowError::OperationFailed(error.to_string()),
    }
}

#[cfg(test)]
mod request_error_tests {
    use super::*;

    #[test]
    fn ignored_request_is_an_operation_failure_not_a_missing_capability() {
        assert!(matches!(
            window_request_error(winit::error::RequestError::Ignored),
            crate::WindowError::OperationFailed(_)
        ));
        assert!(matches!(
            window_request_error(winit::error::RequestError::NotSupported(
                winit::error::NotSupportedError::new("fixture capability")
            )),
            crate::WindowError::Unsupported(_)
        ));
    }
}
