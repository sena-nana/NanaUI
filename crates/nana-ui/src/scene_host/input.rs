//! Scene host input coordination.

use super::*;

impl<Program: RuntimeProgram> WindowManager<Program> {
    pub(super) fn handle_window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        event: WinitWindowEvent,
    ) {
        #[cfg(not(target_os = "android"))]
        if let Some(window) = self.window(id).cloned()
            && let Some(accessibility) = self.accessibility_mut(id)
        {
            accessibility.process_event(window.as_ref(), &event);
        }
        #[cfg(not(target_os = "android"))]
        for request in self.take_accessibility_actions(id) {
            let update = match self
                .program
                .accessibility_action(id, request, &self.context_for(id))
            {
                Ok(update) => update,
                Err(error) => {
                    self.program
                        .report_host_failure(HostFailure::AccessibilityAction {
                            window: id,
                            error: error.to_string(),
                        });
                    continue;
                }
            };
            self.apply_update(event_loop, update, None);
            // An action may close its own window; the event has no target left.
            if event_loop.exiting() || !self.window_contexts.contains_key(&id) {
                return;
            }
        }
        // Presence is window state, reported even while a modal child or Forward
        // passthrough keeps the pointer itself from reaching widgets. A window
        // the host is moving is the exception: the pointer holds it, so every
        // crossing of its own former bounds is the window leaving the pointer,
        // not the pointer leaving the window.
        if let Some(signal) = presence::presence_signal(&event, self.geometry_of(id).physical_size)
            .filter(|_| !self.frame_move_active(id))
        {
            self.observe_pointer_presence(event_loop, id, signal);
            if event_loop.exiting() || !self.window_contexts.contains_key(&id) {
                return;
            }
        }
        let pointer_left = matches!(&event, WinitWindowEvent::PointerLeft { .. });
        if let Some(modal) = self.active_modal_child(id)
            && !allows_modal_parent_event(&event)
        {
            if pointer_left {
                self.reset_window_cursor(id);
            }
            self.focus_window(modal);
            return;
        }
        if let WinitWindowEvent::ModifiersChanged(modifiers) = &event {
            self.input_mut(id).modifiers = modifiers.state();
        }
        if self.forward_os_passthrough_ignores_pointer(id, &event) {
            // Sampling owns the pointer while OS hit-testing is still off.
            // Down must not reach widgets until recover flips hit-testing on.
            return;
        }
        if let WinitWindowEvent::PointerMoved { position, .. }
        | WinitWindowEvent::PointerEntered { position, .. }
        | WinitWindowEvent::PointerButton { position, .. } = &event
        {
            let scale = self.scale_factor(id);
            let point = position.to_logical::<f32>(f64::from(scale));
            self.input_mut(id).cursor = (point.x, point.y);
        }
        if let WinitWindowEvent::PointerLeft {
            position: Some(position),
            ..
        } = &event
        {
            let scale = self.scale_factor(id);
            let point = position.to_logical::<f32>(f64::from(scale));
            self.input_mut(id).cursor = (point.x, point.y);
        }
        if self.forward_pointer_action_for(id, &event) == ForwardPointerAction::RestorePassthrough {
            self.restore_forward_passthrough(event_loop, id);
            return;
        }
        if let Some(input) = self.normalized_input(id, &event) {
            if self.consume_frame_move(event_loop, id, &input)
                || self.consume_frame_resize(event_loop, id, &input)
            {
                if pointer_left {
                    self.reset_window_cursor(id);
                }
                return;
            }
            let disposition = self.dispatch_input(event_loop, id, input);
            if !self.window_contexts.contains_key(&id) {
                return;
            }
            if pointer_left {
                self.reset_window_cursor(id);
            } else if matches!(
                &event,
                WinitWindowEvent::PointerMoved { .. } | WinitWindowEvent::PointerEntered { .. }
            ) {
                self.sync_window_cursor(id);
            }
            if disposition.prevent_default || event_loop.exiting() {
                return;
            }
        }
        match &event {
            WinitWindowEvent::RedrawRequested => {
                self.sync_window_mode(event_loop, id);
                self.redraw(event_loop, id);
            }
            // The program decides whether and when the window goes away; an
            // exit that first saves state answers with `Close` or `exit` later.
            WinitWindowEvent::CloseRequested => self.forward_window_event(event_loop, id, &event),
            WinitWindowEvent::Destroyed => self.close_window(event_loop, id),
            // A frame change that comes with a mode change (entering or
            // leaving fullscreen) records the mode first, so the frame is
            // persisted and reported knowing whether it is a fullscreen one.
            WinitWindowEvent::Moved(_) => {
                self.sync_window_mode(event_loop, id);
                self.sync_geometry(id);
                self.forward_window_event(event_loop, id, &event);
            }
            WinitWindowEvent::SurfaceResized(_) | WinitWindowEvent::ScaleFactorChanged { .. } => {
                match &event {
                    WinitWindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                        nana_diagnostics::event!(
                            nana_diagnostics::framework::window::SCALE_FACTOR_CHANGED,
                            window = id.0,
                            scale = *scale_factor
                        );
                    }
                    _ => nana_diagnostics::metric!(nana_diagnostics::framework::window::RESIZES),
                }
                self.sync_window_mode(event_loop, id);
                let geometry_changed = self.sync_geometry(id);
                #[cfg(target_os = "macos")]
                let native_live_resize = self.sync_native_live_resize_presents(id);
                #[cfg(not(target_os = "macos"))]
                let native_live_resize = false;
                self.forward_window_event(event_loop, id, &event);
                // Native macOS drags repaint through winit's live-resize
                // hook, and a custom chrome drag paints its steps in-stack;
                // both would only duplicate the per-step frame here.
                if geometry_changed && !native_live_resize {
                    self.request_redraw(id);
                }
            }
            WinitWindowEvent::Occluded(occluded) => {
                nana_diagnostics::event!(
                    nana_diagnostics::framework::window::OCCLUDED,
                    window = id.0,
                    occluded = *occluded
                );
                if *occluded {
                    self.occluded.insert(id);
                } else {
                    self.occluded.remove(&id);
                    self.request_redraw(id);
                }
                self.forward_window_event(event_loop, id, &event);
                self.sync_window_mode(event_loop, id);
            }
            WinitWindowEvent::Focused(focused) => {
                if !*focused {
                    self.input_mut(id).clear_pointers();
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    self.end_live_frame_resize(id);
                    // Ending a window move tells the document its gesture is
                    // over, so it runs last: that dispatch may close `id`.
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    self.end_live_frame_move(event_loop, id);
                    if !self.window_contexts.contains_key(&id) {
                        return;
                    }
                }
                self.forward_window_event(event_loop, id, &event);
                self.apply_ime_request(id);
            }
            WinitWindowEvent::Ime(ime) => {
                self.handle_ime(event_loop, id, platform_ime_event(ime.clone()))
            }
            WinitWindowEvent::DragEntered { .. }
            | WinitWindowEvent::DragPosition { .. }
            | WinitWindowEvent::DragDropped { .. }
            | WinitWindowEvent::DragLeft { .. }
            | WinitWindowEvent::DataTransferReceived { .. } => {
                if let Some(window_event) = self.handle_file_dnd(event_loop, id, &event) {
                    let update = self
                        .program
                        .window_event(window_event, &self.context_for(id));
                    self.apply_update(event_loop, update, None);
                }
            }
            _ => {}
        }
    }
    pub(super) fn forward_window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        event: &WinitWindowEvent,
    ) {
        if let Some(window_event) = platform_window_event(event, id, self.geometry_of(id)) {
            let update = self
                .program
                .window_event(window_event, &self.context_for(id));
            self.apply_update(event_loop, update, None);
        }
    }
    pub(super) fn handle_file_dnd(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        event: &WinitWindowEvent,
    ) -> Option<WindowEvent> {
        let scale = self.scale_factor(id);
        match event {
            WinitWindowEvent::DragEntered {
                id: transfer,
                position,
            } => {
                if let Some(position) = position {
                    self.input_mut(id).set_cursor_physical(*position, scale);
                }
                if !dnd_advertises_files(event_loop, *transfer) {
                    return None;
                }
                let _ = event_loop.set_valid_dnd_actions(*transfer, &[DndAction::Copy]);
                let serial = event_loop
                    .fetch_data_transfer(*transfer, &TypeHint::UriList)
                    .ok();
                self.input_mut(id).begin_file_drag(*transfer, serial);
                self.input_mut(id).map_file_window_event(event, id)
            }
            WinitWindowEvent::DragPosition { position, .. } => {
                self.input_mut(id).set_cursor_physical(*position, scale);
                self.input_mut(id).map_file_window_event(event, id)
            }
            WinitWindowEvent::DragDropped { id: transfer, .. } => {
                if !self.input_mut(id).pending_file_paths.is_empty() {
                    return self.input_mut(id).map_file_window_event(event, id);
                }
                match event_loop.fetch_data_transfer(*transfer, &TypeHint::UriList) {
                    Ok(serial) => {
                        self.input_mut(id).wait_for_drop_data(*transfer, serial);
                        None
                    }
                    Err(_) => self.input_mut(id).map_file_window_event(event, id),
                }
            }
            WinitWindowEvent::DragLeft { .. } => {
                self.input_mut(id).map_file_window_event(event, id)
            }
            WinitWindowEvent::DataTransferReceived {
                id: transfer,
                serial,
                value,
            } => {
                if !self.input_mut(id).accepts_dnd_serial(*transfer, *serial) {
                    return None;
                }
                match value.try_as_file_paths() {
                    Ok(paths) => self.input_mut(id).ingest_file_paths(*transfer, paths, id),
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Deadlock
                        ) =>
                    {
                        None
                    }
                    Err(_) => None,
                }
            }
            _ => None,
        }
    }
    pub(super) fn normalized_input(
        &mut self,
        id: WindowId,
        event: &WinitWindowEvent,
    ) -> Option<InputEvent> {
        let scale = self.scale_factor(id);
        let origin = self
            .window(id)
            .and_then(|window| window_screen_origin(window.as_ref()));
        self.input_mut(id).map(event, scale, origin)
    }
    pub(super) fn dispatch_input(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        input: InputEvent,
    ) -> nana_ui_platform::InputDisposition {
        let now = self.animation_clock.runtime_time(Instant::now());
        let disposition = match self
            .program
            .write_document(id, |document| {
                let document_id = document.document();
                RuntimeInputAdapter::default().dispatch_with_shaper(
                    document.context_mut(),
                    document_id,
                    &input,
                    now,
                    Some(&mut self.text),
                )
            })
            .transpose()
        {
            Ok(disposition) => disposition.unwrap_or_default(),
            Err(error) => {
                // Drop this input event; the program sees the failure through
                // host_failure instead of the process dying in the event loop.
                self.program
                    .report_host_failure(HostFailure::InputDispatch {
                        window: id,
                        error: error.to_string(),
                    });
                nana_ui_platform::InputDisposition::default()
            }
        };
        let chrome_action = self.title_bar_chrome_action(id, &input);
        // Runtime may already have consumed the event (prevent_default). Scene
        // still delivers input_event so Gallery can drain leftover host input and
        // Vue can emit JS. Leftover winit handling stays gated by the caller.
        // Program messages stay queued until the next frame so navigation
        // coalesces and does not run inside the pointer handler.
        let pointer_hit = self
            .program
            .read_document(id, |document| input_pointer_hit(Some(document), &input))
            .flatten();
        let program_input = self.program.input_event(
            id,
            crate::RoutedInput {
                event: &input,
                pointer_hit,
                disposition,
            },
            &self.context_for(id),
        );
        if let Err(error) = &program_input {
            self.program.report_host_failure(HostFailure::InputHandler {
                window: id,
                error: error.to_string(),
            });
        }
        let mut update = scene_runtime_input_update(disposition, id, program_input);
        if self
            .program
            .read_document(id, |document| document.context().has_program_messages())
            .unwrap_or(false)
        {
            update = update.merge(RuntimeProgramUpdate::redraw(id));
        }
        let update = self.merge_title_bar_chrome(id, chrome_action, update);
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
        disposition
    }
    pub(super) fn input_of(&self, id: WindowId) -> &InputTracker {
        &self
            .window_contexts
            .get(&id)
            .expect("input belongs to a live window")
            .input
    }
    pub(super) fn input_mut(&mut self, id: WindowId) -> &mut InputTracker {
        &mut self
            .window_contexts
            .get_mut(&id)
            .expect("input belongs to a live window")
            .input
    }
}
