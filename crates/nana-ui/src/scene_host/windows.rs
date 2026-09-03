//! Scene host windows coordination.

use super::*;

impl<Program: RuntimeProgram> SceneReady<Program> {
    pub(super) fn apply_window_command(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        command: WindowCommand,
    ) {
        let known = self.known_window_ids();
        match route_window_command(&command, &known) {
            RoutedWindowCommand::Ignore => {}
            RoutedWindowCommand::Open(id) => {
                let WindowCommand::Open { settings, .. } = command else {
                    return;
                };
                if let Ok(event) = self.open_window(event_loop, id, settings) {
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
                if let Some(window) = self.window(id) {
                    window.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
                }
            }
            RoutedWindowCommand::SetMinimized(id) => {
                let WindowCommand::SetMinimized { minimized, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id) {
                    window.set_minimized(minimized);
                }
            }
            RoutedWindowCommand::SetMaximized(id) => {
                let WindowCommand::SetMaximized { maximized, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id).cloned() {
                    window.set_maximized(maximized);
                    self.resize_window(id);
                    self.sync_geometry(id);
                    let update = self.program.window_event(
                        WindowEvent::Resized {
                            id,
                            geometry: self.geometry_of(id),
                        },
                        &self.context_for(id),
                    );
                    self.apply_update(event_loop, update, None);
                }
            }
            RoutedWindowCommand::SetAlwaysOnTop(id) => {
                let WindowCommand::SetAlwaysOnTop { always_on_top, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id) {
                    window.set_window_level(window_level(always_on_top));
                }
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
    pub(super) fn open_window(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        settings: RuntimeWindowSettings,
    ) -> Result<WindowEvent, String> {
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
            &scene_display_bounds(event_loop),
        )?;
        let window: Arc<dyn winit::window::Window> = Arc::from(
            event_loop
                .create_window(attributes)
                .map_err(|error| error.to_string())?,
        );
        apply_scene_window_icon(
            window.as_ref(),
            settings.icon.as_ref(),
            id == WindowId::PRIMARY,
        );
        if !settings.system_caption {
            let _ = prepare_client_chrome(window.as_ref(), f64::from(TITLE_BAR_HEIGHT));
            if settings.transparent {
                let _ = suppress_system_caption(window.as_ref());
            }
        }
        let material = apply_window_surface(
            window.as_ref(),
            self.last_theme,
            settings.transparent,
            self.last_material_mode,
            self.program.appearance_backdrop_opacity(),
        );
        let surface = self
            .graphics
            .create_surface(
                Arc::clone(&window),
                window_wants_transparent_surface(settings.transparent, self.last_material_mode),
            )
            .map_err(|error| error.to_string())?;
        let format = surface.format();
        let _ = self.painter_mut(format);
        #[cfg(not(target_os = "android"))]
        let accessibility = {
            Some(HostedAccessibility::new(
                Arc::clone(&window),
                accessibility_world_generation(&mut self.program, id),
                accessibility_snapshot(&mut self.program, id),
                true,
                window.scale_factor() as f32,
            ))
        };
        let geometry = window_geometry(window.as_ref());
        #[cfg(target_os = "windows")]
        let modal_parent = settings.modal.then_some(settings.parent).flatten();
        self.window_ids.insert(window.id(), id);
        self.auxiliary.insert(
            id,
            SceneAuxiliary {
                surface,
                geometry,
                input: InputTracker::default(),
                material,
                settings,
                #[cfg(not(target_os = "android"))]
                accessibility,
                accessibility_pending: None,
                size_move: LiveSizeMove::install(window.as_ref())?,
            },
        );
        #[cfg(target_os = "windows")]
        if let Some(parent) = modal_parent.and_then(|parent| self.window(parent)) {
            parent.set_enable(false);
        }
        window.set_visible(true);
        window.request_redraw();
        self.prepare_window_chrome(id, geometry.maximized);
        Ok(WindowEvent::Ready { id, geometry })
    }
    pub(super) fn close_window(&mut self, event_loop: &dyn ActiveEventLoop, id: WindowId) {
        if id == WindowId::PRIMARY {
            return;
        }
        self.chrome.remove(&id);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some((_, live)) = self
            .live_frame_resize
            .take_if(|(session, _)| *session == id)
            && let Some(window) = self.window(id)
        {
            live.end(window.as_ref());
        }
        if let Some(host) = self.auxiliary.remove(&id) {
            #[cfg(target_os = "windows")]
            if let Some(parent) = host
                .settings
                .modal
                .then_some(host.settings.parent)
                .flatten()
                .and_then(|parent| self.window(parent))
            {
                parent.set_enable(true);
                parent.focus_window();
            }
            self.window_ids.remove(&host.surface.window().id());
            self.ime.remove(&id);
            drop(host);
            let update = self
                .program
                .window_event(WindowEvent::Closed { id }, &self.context_for(id));
            self.apply_update(event_loop, update, None);
        }
    }
    pub(super) fn focus_window(&self, id: WindowId) {
        if let Some(window) = self.window(id) {
            window.set_visible(true);
            window.focus_window();
        }
    }
    pub(super) fn move_window(&self, id: WindowId, position: (f32, f32)) {
        let Some(window) = self.window(id) else {
            return;
        };
        window.set_outer_position(winit::dpi::Position::Logical(
            winit::dpi::LogicalPosition::new(f64::from(position.0), f64::from(position.1)),
        ));
    }
    pub(super) fn set_window_bounds(&self, id: WindowId, position: (f32, f32), size: (f32, f32)) {
        let Some(window) = self.window(id) else {
            return;
        };
        window.set_outer_position(winit::dpi::Position::Logical(
            winit::dpi::LogicalPosition::new(f64::from(position.0), f64::from(position.1)),
        ));
        let _ = window.request_surface_size(winit::dpi::Size::Logical(
            winit::dpi::LogicalSize::new(f64::from(size.0.max(1.0)), f64::from(size.1.max(1.0))),
        ));
    }
    pub(super) fn active_modal_child(&self, parent: WindowId) -> Option<WindowId> {
        self.auxiliary.iter().find_map(|(id, host)| {
            (host.settings.modal && host.settings.parent == Some(parent)).then_some(*id)
        })
    }
    pub(super) fn sync_appearance(&mut self) {
        let theme = self.program.theme_mode();
        let mode = self.program.window_material_mode();
        if theme != self.last_theme || mode != self.last_material_mode {
            self.last_theme = theme;
            self.last_material_mode = mode;
            self.refresh_material();
            self.request_redraw_all();
        }
    }
    pub(super) fn refresh_material(&mut self) {
        clear_system_material(self.graphics.window().as_ref());
        self.material = apply_window_surface(
            self.graphics.window().as_ref(),
            self.last_theme,
            self.settings.transparent,
            self.last_material_mode,
            self.program.appearance_backdrop_opacity(),
        );
        let mut alpha_error = self
            .graphics
            .apply_alpha_mode(window_wants_transparent_surface(
                self.settings.transparent,
                self.last_material_mode,
            ))
            .err();
        for host in self.auxiliary.values_mut() {
            clear_system_material(host.surface.window().as_ref());
            host.material = apply_window_surface(
                host.surface.window().as_ref(),
                self.last_theme,
                host.settings.transparent,
                self.last_material_mode,
                self.program.appearance_backdrop_opacity(),
            );
            let want_transparent = window_wants_transparent_surface(
                host.settings.transparent,
                self.last_material_mode,
            );
            if let Err(error) = self
                .graphics
                .apply_surface_alpha_mode(&mut host.surface, want_transparent)
            {
                alpha_error = Some(error);
            }
        }
        if let Some(error) = alpha_error {
            self.suspend_rendering(error);
        }
    }
    /// Refreshes the cached window geometry from the live window state and
    /// reports whether it moved.
    pub(super) fn sync_geometry(&mut self, id: WindowId) -> bool {
        let previous = self.geometry_of(id);
        if id == WindowId::PRIMARY {
            self.geometry = window_geometry(self.graphics.window().as_ref());
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
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
        self.sync_title_bar_maximized(id, maximized);
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
        let live = self.is_live_resize(id);
        if id == WindowId::PRIMARY {
            self.graphics.prepare_frame(live);
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
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
        if id == WindowId::PRIMARY {
            self.size_move.is_active()
        } else {
            self.auxiliary
                .get(&id)
                .is_some_and(|host| host.size_move.is_active())
        }
    }
    pub(super) fn sync_window_cursor(&mut self, id: WindowId) {
        if !self
            .input_mut(id)
            .begin_cursor_sync(std::time::Instant::now())
        {
            return;
        }
        let cursor = self.input_of(id).cursor;
        let frame_edge = self.frame_resize_edge_at(id, cursor.0, cursor.1);
        let (handle, text_field) = self
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
                let text_field = context
                    .pointer_target(document_id, cursor.0, cursor.1)
                    .is_some_and(|node| context.world().text_input(node).is_some());
                (handle, text_field)
            })
            .unwrap_or((None, false));
        if let Some(window) = self.window(id) {
            window.set_cursor(scene_cursor_icon(frame_edge, handle, text_field).into());
        }
    }
    pub(super) fn consume_frame_resize(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        input: &InputEvent,
    ) -> bool {
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
        &self,
        id: WindowId,
        x: f32,
        y: f32,
    ) -> Option<WindowResizeEdge> {
        let fullscreen = self
            .window(id)
            .is_some_and(|window| window.fullscreen().is_some());
        frame_resize_edge_for(
            self.settings_of(id),
            &self.geometry_of(id),
            fullscreen,
            x,
            y,
        )
    }
    pub(super) fn settings_of(&self, id: WindowId) -> &RuntimeWindowSettings {
        if id == WindowId::PRIMARY {
            &self.settings
        } else {
            self.auxiliary
                .get(&id)
                .map(|host| &host.settings)
                .unwrap_or(&self.settings)
        }
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
        if action == WindowChromeAction::Close && id == WindowId::PRIMARY {
            update.exit = true;
            return update;
        }
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
    pub(super) fn sync_title_bar_maximized(&mut self, id: WindowId, maximized: bool) {
        self.program.write_document(id, |document| {
            let document_id = document.document();
            let context = document.context_mut();
            let bars = context
                .world()
                .document_order(document_id)
                .into_iter()
                .filter(|&node| {
                    context
                        .read(Entity::<AppTitleBar>::from_stable_id(node), |_| ())
                        .is_ok()
                })
                .collect::<Vec<_>>();
            for bar in bars {
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
