//! Scene host input coordination.

use super::*;

/// Actions a file drop accepts. AppKit narrows a Control-drag from Finder to
/// Link and refuses the drop when nothing matches, so macOS takes Link too;
/// elsewhere Link would only turn Shift-drags into shortcuts.
#[cfg(target_os = "macos")]
const FILE_DROP_ACTIONS: &[DndAction] = &[DndAction::Copy, DndAction::Link];
#[cfg(not(target_os = "macos"))]
const FILE_DROP_ACTIONS: &[DndAction] = &[DndAction::Copy];

impl<Program: RuntimeProgram> WindowManager<Program> {
    /// Enqueue and drain a native lifecycle event through the same endpoint
    /// contract as pointer/key/wheel input. `None` means the event remains in
    /// the bounded pending queue because endpoint capacity or host-service
    /// backpressure prevented dispatch.
    pub(super) fn route_native_lifecycle_event(
        &mut self,
        id: WindowId,
        event: nana_ui_platform::CanonicalInputEvent,
        now: Duration,
    ) -> Option<nana_ui_platform::InputDisposition> {
        let mut pending = self.input_pending.remove(&id).unwrap_or_default();
        if pending.len() >= super::NATIVE_PENDING_INPUT_CAPACITY {
            self.program
                .report_host_failure(HostFailure::InputDispatch {
                    window: id,
                    error: "native input pending queue is full".into(),
                });
            self.input_pending.insert(id, pending);
            return None;
        }
        pending.push_back(event);
        let source = nana_ui_platform::InputSourceId(id.0);
        let generation =
            nana_ui_platform::EndpointGeneration(*self.input_generations.entry(id).or_insert(1));
        let mut endpoint = self.input_endpoints.remove(&id).unwrap_or_else(|| {
            nana_ui_platform::InputEndpoint::new(source, generation, 1024, 1024 * 1024)
        });
        while let Some(canonical) = pending.pop_front() {
            match endpoint.push(canonical) {
                Ok(_) => {}
                Err(rejected)
                    if matches!(rejected.reason, nana_ui_platform::InputRejection::Capacity) =>
                {
                    pending.push_front(rejected.event);
                    break;
                }
                Err(rejected) => {
                    self.program
                        .report_host_failure(HostFailure::InputDispatch {
                            window: id,
                            error: format!(
                                "native lifecycle endpoint rejected canonical event: {:?}",
                                rejected.reason
                            ),
                        });
                }
            }
        }
        let mut disposition = nana_ui_platform::InputDisposition::default();
        while let Some(canonical) = endpoint.front().cloned() {
            let routed = self.program.write_document(id, |document| {
                self.input_router
                    .route(document.context_mut(), &canonical, now, None)
            });
            match routed.transpose() {
                Ok(Some(result)) => {
                    disposition.handled |= result.handled;
                    disposition.prevent_default |= result.prevent_default;
                }
                Ok(None) => {}
                Err(crate::runtime_input::InputRouterError::HostServiceBackpressure) => break,
                Err(error) => self
                    .program
                    .report_host_failure(HostFailure::InputDispatch {
                        window: id,
                        error: error.to_string(),
                    }),
            }
            endpoint.pop();
        }
        self.input_endpoints.insert(id, endpoint);
        self.input_pending.insert(id, pending);
        Some(disposition)
    }

    pub(super) fn handle_window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        event: WinitWindowEvent,
    ) {
        let geometry = self.geometry_of(id);
        self.input_mut(id)
            .set_coordinate_extents(geometry.logical_size, geometry.physical_size);
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
            self.input_mut(id).set_cursor_physical(*position, scale);
        }
        if let WinitWindowEvent::PointerLeft {
            position: Some(position),
            ..
        } = &event
        {
            let scale = self.scale_factor(id);
            self.input_mut(id).set_cursor_physical(*position, scale);
        }
        if self.forward_pointer_action_for(id, &event) == ForwardPointerAction::RestorePassthrough {
            self.restore_forward_passthrough(event_loop, id);
            return;
        }
        // Leaving the native surface is not a lifecycle cancellation. Emit a
        // canonical leave so hover state is cleared while a capture owner, if
        // present, stays alive for subsequent moves and release. The legacy
        // mapped Cancel path remains for frame-move handling.
        if pointer_left && !self.frame_move_active(id) {
            let _ = self.dispatch_pointer_leave(id, &event);
            self.reset_window_cursor(id);
            self.request_redraw(id);
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
                        self.rescale_startup_splash(id, *scale_factor);
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
                    // Nothing of an occluded window shows — minimized or
                    // covered — so nothing can hover it: report the leave the
                    // platform may never send, as hiding does.
                    self.hide_pointer_presence(event_loop, id);
                    if event_loop.exiting() || !self.window_contexts.contains_key(&id) {
                        return;
                    }
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
                // Window focus is a canonical lifecycle event as well as a
                // host observation. Route it before forwarding so Runtime can
                // revoke capture/hover and clear document focus consistently
                // across native, Vue, and headless sources.
                let _ = self.dispatch_focus(id, *focused);
                if !self.window_contexts.contains_key(&id) {
                    return;
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
                let _ = event_loop.set_valid_dnd_actions(*transfer, FILE_DROP_ACTIONS);
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
                    // A release waiting on these paths would otherwise never end.
                    Err(_) => self.input_mut(id).abandon_drop(*transfer, id),
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

    fn dispatch_pointer_leave(
        &mut self,
        id: WindowId,
        event: &WinitWindowEvent,
    ) -> Option<nana_ui_platform::InputDisposition> {
        let WinitWindowEvent::PointerLeft {
            device_id,
            primary,
            kind,
            ..
        } = event
        else {
            return None;
        };
        let pointer = map_pointer_kind(kind, *primary, *device_id).pointer_id;
        let now = self.animation_clock.runtime_time(Instant::now());
        let source = nana_ui_platform::InputSourceId(id.0);
        let generation =
            nana_ui_platform::EndpointGeneration(*self.input_generations.get(&id).unwrap_or(&1));
        let sequence = self.input_mut(id).next_canonical_sequence();
        let canonical = nana_ui_platform::CanonicalInputEvent {
            metadata: nana_ui_platform::InputMetadata {
                source,
                device: self.input_of(id).canonical_device,
                generation,
                sequence,
                timestamp: nana_ui_platform::InputTimestamp(
                    now.as_nanos().min(u128::from(u64::MAX)) as u64,
                ),
            },
            payload: nana_ui_platform::InputPayload::PointerLeave {
                pointer_id: nana_ui_platform::PointerId(pointer),
            },
        };
        let disposition = self.route_native_lifecycle_event(id, canonical, now);
        let _ = self.drain_host_service_requests(id);
        disposition
    }
    pub(super) fn dispatch_input(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        input: InputEvent,
    ) -> nana_ui_platform::InputDisposition {
        let now = self.animation_clock.runtime_time(Instant::now());
        let source = nana_ui_platform::InputSourceId(id.0);
        let generation =
            nana_ui_platform::EndpointGeneration(*self.input_generations.entry(id).or_insert(1));
        let sequence = self.input_mut(id).next_canonical_sequence();
        let document = self
            .program
            .read_document(id, |document| document.document());
        let mut disposition = nana_ui_platform::InputDisposition::default();
        let mut routed_snapshot = None;
        if let Some(document) = document {
            self.input_router
                .ensure_attached(source, generation, document);
            let metadata = nana_ui_platform::InputMetadata {
                source,
                device: self.input_of(id).canonical_device,
                generation,
                sequence,
                timestamp: nana_ui_platform::InputTimestamp(
                    now.as_nanos().min(u128::from(u64::MAX)) as u64,
                ),
            };
            let canonical_events = nana_ui_platform::lower_input_event(&input, metadata);
            if canonical_events.len() > 1 {
                self.input_mut(id)
                    .advance_canonical_sequence(canonical_events.len() - 1);
            }
            let mut pending = self.input_pending.remove(&id).unwrap_or_default();
            for canonical in canonical_events {
                if pending.len() < super::NATIVE_PENDING_INPUT_CAPACITY {
                    pending.push_back(canonical);
                } else {
                    self.program
                        .report_host_failure(HostFailure::InputDispatch {
                            window: id,
                            error: "native input pending queue is full".into(),
                        });
                    break;
                }
            }
            let mut endpoint = self.input_endpoints.remove(&id).unwrap_or_else(|| {
                nana_ui_platform::InputEndpoint::new(source, generation, 1024, 1024 * 1024)
            });
            while let Some(canonical) = pending.pop_front() {
                let rejected = match endpoint.push(canonical) {
                    Ok(_) => continue,
                    Err(rejected) => rejected,
                };
                if matches!(rejected.reason, nana_ui_platform::InputRejection::Capacity) {
                    pending.push_front(rejected.event);
                    break;
                }
                self.program
                    .report_host_failure(HostFailure::InputDispatch {
                        window: id,
                        error: format!(
                            "native input endpoint rejected canonical event: {:?}",
                            rejected.reason
                        ),
                    });
            }
            while let Some(canonical) = endpoint.front().cloned() {
                let routed = self.program.write_document(id, |document| {
                    self.input_router.route(
                        document.context_mut(),
                        &canonical,
                        now,
                        Some(&mut self.text),
                    )
                });
                match routed.transpose() {
                    Ok(Some(result)) => {
                        disposition.handled |= result.handled;
                        disposition.prevent_default |= result.prevent_default;
                        routed_snapshot = self.input_router.last_route_snapshot();
                    }
                    Ok(None) => {}
                    Err(crate::runtime_input::InputRouterError::HostServiceBackpressure) => {
                        break;
                    }
                    Err(error) => self
                        .program
                        .report_host_failure(HostFailure::InputDispatch {
                            window: id,
                            error: error.to_string(),
                        }),
                }
                // The endpoint retains the event only while host-service
                // backpressure is active; all other outcomes are terminal for
                // this queued event and may release its capacity.
                endpoint.pop();
            }
            self.input_endpoints.insert(id, endpoint);
            self.input_pending.insert(id, pending);
        }
        let host_service_changed = self.drain_host_service_requests(id);
        let chrome_action = self.title_bar_chrome_action(id, &input);
        // Runtime may already have consumed the event (prevent_default). Scene
        // still delivers input_event so Gallery can drain leftover host input and
        // Vue can emit JS. Leftover winit handling stays gated by the caller.
        // Program messages stay queued until the next frame so navigation
        // coalesces and does not run inside the pointer handler.
        // Runtime has already resolved capture above. Reuse that owner for
        // program observation so a captured move outside the surface does not
        // perform a second global hit-test; only uncaptured pointer/wheel
        // observations need the document index.
        let captured_hit = routed_snapshot
            .filter(|snapshot| {
                snapshot.source == source
                    && snapshot.device == self.input_of(id).canonical_device
                    && snapshot.kind == crate::runtime_input::CanonicalInputKind::Pointer
            })
            .and_then(|snapshot| snapshot.capture_owner);
        let pointer_hit = captured_hit.or_else(|| {
            self.program
                .read_document(id, |document| input_pointer_hit(Some(document), &input))
                .flatten()
        });
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
        if host_service_changed {
            update = update.merge(RuntimeProgramUpdate::redraw(id));
        }
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

    fn dispatch_focus(
        &mut self,
        id: WindowId,
        focused: bool,
    ) -> nana_ui_platform::InputDisposition {
        let now = self.animation_clock.runtime_time(Instant::now());
        let source = nana_ui_platform::InputSourceId(id.0);
        let generation =
            nana_ui_platform::EndpointGeneration(*self.input_generations.entry(id).or_insert(1));
        let sequence = self.input_mut(id).next_canonical_sequence();
        let document = self
            .program
            .read_document(id, |document| document.document());
        let mut disposition = nana_ui_platform::InputDisposition::default();
        if let Some(document) = document {
            self.input_router
                .ensure_attached(source, generation, document);
            let canonical = nana_ui_platform::CanonicalInputEvent {
                metadata: nana_ui_platform::InputMetadata {
                    source,
                    device: self.input_of(id).canonical_device,
                    generation,
                    sequence,
                    timestamp: nana_ui_platform::InputTimestamp(
                        now.as_nanos().min(u128::from(u64::MAX)) as u64,
                    ),
                },
                payload: nana_ui_platform::InputPayload::Focus { focused },
            };
            if let Some(result) = self.route_native_lifecycle_event(id, canonical, now) {
                disposition = result;
            }
        }
        // Focus transitions emit IME lifecycle intents even when no pointer
        // or key event follows. Drain them at the same native event-loop
        // boundary so a blur/enable cannot leave the bounded queue occupied.
        let _ = self.drain_host_service_requests(id);
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
