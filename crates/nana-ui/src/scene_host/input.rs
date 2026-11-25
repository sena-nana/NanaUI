//! Scene host input coordination.

use super::*;
use crate::winit_input::{
    LoweredInput, WinitInputAdapter, apply_pen_frame, is_backend_pen_leave, pen_pointer_id,
    platform_window_event,
};

impl<Program: RuntimeProgram> WindowManager<Program> {
    /// Refresh the cached display HDR metadata after a window/display event.
    ///
    /// Surface capabilities remain the authority for selecting HDR; this
    /// event-level query only changes the presentation tone-map parameters.
    /// Keeping the query here avoids an OS capability call on steady frames
    /// (and keeps Metal's main-thread-only query on the event thread).
    /// The window may be on another display: when it is, read that display's
    /// HDR metadata and resolve the surface profile against it. A move within
    /// one display (Windows sends one per step of a drag) does neither.
    pub(super) fn refresh_surface_display(&mut self, id: WindowId) {
        let Some(monitor) = self
            .window(id)
            .map(|window| window.current_monitor().map(|monitor| monitor.id()))
        else {
            return;
        };
        let Some(host) = self.window_contexts.get_mut(&id) else {
            return;
        };
        if host.monitor == monitor {
            return;
        }
        host.monitor = monitor;
        self.refresh_surface_hdr_info(id);
    }

    pub(super) fn refresh_surface_hdr_info(&mut self, id: WindowId) -> bool {
        let Some(host) = self.window_contexts.get_mut(&id) else {
            return false;
        };
        let profile_changed = host.surface.re_resolve_profile(self.graphics.gpu());
        let hdr_changed = host.surface.refresh_hdr_info(self.graphics.gpu());
        let changed = profile_changed || hdr_changed;
        if changed {
            self.request_redraw(id);
        }
        changed
    }

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
        #[cfg(not(target_os = "android"))]
        self.drain_color_pick_requests(event_loop, id);
        // A pen that left goes before whatever this event is.
        self.deliver_pen_leave(event_loop, id);
        if !self.window_contexts.contains_key(&id) {
            return;
        }
        if self
            .window_contexts
            .get(&id)
            .is_some_and(|host| is_backend_pen_leave(&event, &host.pen))
        {
            return;
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
            self.input_mut(id).set_modifiers(modifiers.state());
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
        let gesture_was_active = self.forward_gesture_active(id);
        if self.forward_pointer_action_for(id, &event) == ForwardPointerAction::RestorePassthrough {
            self.restore_forward_passthrough(event_loop, id);
            return;
        }
        if pointer_left && self.frame_move_active(id) {
            // The pointer holds the window it is moving: crossing the
            // window's former bounds is not the pointer leaving it.
            self.reset_window_cursor(id);
            return;
        }
        let scale = self.scale_factor(id);
        let origin = self
            .window(id)
            .and_then(|window| window_screen_origin(window.as_ref(), self.content_zoom_of(id)));
        let now = self.animation_clock.runtime_time(Instant::now());
        let pen_frame = self
            .window_contexts
            .get(&id)
            .and_then(|host| host.pen.current_frame());
        if let Some(mut lowered) = self.input_mut(id).map(&event, scale, origin, now) {
            apply_pen_frame(&mut lowered.payload, pen_frame);
            let drag_press_pos = match lowered.payload {
                InputPayload::Pointer(PointerInput {
                    phase: PointerPhase::Down,
                    button: 0,
                    is_primary: true,
                    x,
                    y,
                    ..
                }) => Some((x, y)),
                _ => None,
            };
            let resample_cursor = !forward_pointer_event(&event);
            if self.consume_frame_move(event_loop, id, &lowered.payload)
                || self.consume_frame_resize(event_loop, id, &lowered.payload)
            {
                self.sync_pointer_capture(id);
                self.settle_forward_passthrough_after_gesture(
                    event_loop,
                    id,
                    gesture_was_active,
                    resample_cursor,
                );
                return;
            }
            let disposition = self.deliver_input(event_loop, id, lowered);
            if !self.window_contexts.contains_key(&id) {
                return;
            }
            self.sync_pointer_capture(id);
            if !disposition.handled
                && !disposition.prevent_default
                && let Some((x, y)) = drag_press_pos
                && !self.frame_move_active(id)
                && !self.frame_resize_active(id)
                && self.window_contexts.get(&id).is_some_and(|host| {
                    let snap = host.interaction_regions.snapshot();
                    snap.revision > 0
                        && snap.classify(x, y) == nana_ui_platform::NativeHitTestClass::Drag
                })
            {
                let _ = self.begin_window_move(event_loop, id);
                self.sync_pointer_capture(id);
            }
            self.settle_forward_passthrough_after_gesture(
                event_loop,
                id,
                gesture_was_active,
                resample_cursor,
            );
            if !self.window_contexts.contains_key(&id) {
                return;
            }
            if pointer_left {
                // The host shows its own cursor once the pointer is gone;
                // `cursor: none` must not leak outside the window.
                self.reset_window_cursor(id);
                self.request_redraw(id);
                return;
            }
            if disposition.prevent_default || event_loop.exiting() {
                return;
            }
        }
        match &event {
            WinitWindowEvent::RedrawRequested => {
                // Moves merged since the last turn land before the frame
                // that shows them.
                self.drain_window_input(event_loop, id);
                if !self.window_contexts.contains_key(&id) {
                    return;
                }
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
                // A Windows companion follows its owner inside the owner's
                // window procedure; record those moves as they land.
                if let Some(host) = self.window_contexts.get_mut(&id) {
                    crate::window_shadow::report_work(&host.shadow, &mut host.shadow_reported);
                }
                // Moving a window can put it on another display, with other
                // HDR metadata and formats. Only a display change re-reads
                // them; presentation parameters refresh without rebuilding
                // the application or Scene state.
                self.refresh_surface_display(id);
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
                        // Shadow content is built at device resolution; the
                        // next present re-derives the body and rebuilds it.
                        if let Some(host) = self.window_contexts.get_mut(&id) {
                            let window = Arc::clone(host.surface.window());
                            host.shadow.rescale(window.as_ref());
                            host.shadow_body = None;
                            crate::window_shadow::report_work(
                                &host.shadow,
                                &mut host.shadow_reported,
                            );
                        }
                    }
                    _ => nana_diagnostics::metric!(nana_diagnostics::framework::window::RESIZES),
                }
                self.sync_window_mode(event_loop, id);
                let geometry_changed = self.sync_geometry(id);
                // Leaving maximized or fullscreen arrives as a resize.
                self.conform_content_aspect(event_loop, id);
                if geometry_changed || matches!(event, WinitWindowEvent::ScaleFactorChanged { .. })
                {
                    self.sync_input_region(id);
                }
                // A scale change usually means another display; a resize
                // step does not, and is not worth a display query.
                if matches!(event, WinitWindowEvent::ScaleFactorChanged { .. }) {
                    self.refresh_surface_display(id);
                    self.apply_window_cursor(id, Some(event_loop));
                }
                #[cfg(target_os = "macos")]
                let native_live_resize = self.native_live_resize_gesture(id);
                #[cfg(not(target_os = "macos"))]
                let native_live_resize = false;
                // Native macOS drags repaint through winit's live-resize hook.
                // Steps inside one refresh interval keep the last drawable and
                // do not relayout; the hook still runs, but it returns before
                // configure. A custom chrome drag paints its steps in-stack
                // and is not this path.
                let defer_program = native_live_resize
                    && matches!(event, WinitWindowEvent::SurfaceResized(_))
                    && self.defer_native_live_resize(id);
                if !defer_program {
                    self.forward_window_event(event_loop, id, &event);
                }
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
                // Minimizing and restoring arrive as occlusion changes.
                self.sync_underlays_of(id);
                self.sync_auxiliaries_of(id);
                if *occluded {
                    self.occluded.insert(id);
                    // Nothing of an occluded window shows — minimized or
                    // covered — so nothing can hover it: report the leave the
                    // platform may never send, as hiding does.
                    self.hide_pointer_presence(event_loop, id);
                    self.reset_forward_passthrough_if_needed(event_loop, id);
                    if event_loop.exiting() || !self.window_contexts.contains_key(&id) {
                        return;
                    }
                } else {
                    self.occluded.remove(&id);
                    self.refresh_surface_hdr_info(id);
                    self.request_redraw(id);
                }
                self.forward_window_event(event_loop, id, &event);
                self.sync_window_mode(event_loop, id);
            }
            WinitWindowEvent::Focused(focused) => {
                // Activation raised the window; its underlay goes back under it.
                if *focused {
                    self.sync_underlays_of(id);
                    self.sync_auxiliary_window(id);
                    self.sync_auxiliaries_of(id);
                    self.refresh_surface_hdr_info(id);
                }
                if !*focused {
                    let gesture_was_active = self.forward_gesture_active(id);
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
                    self.settle_forward_passthrough_after_gesture(
                        event_loop,
                        id,
                        gesture_was_active,
                        true,
                    );
                    if !self.window_contexts.contains_key(&id) {
                        return;
                    }
                }
                if let Some(host) = self.window_contexts.get_mut(&id) {
                    host.input_source.reset_ime();
                }
                // Window focus is canonical input as well as a host
                // observation. Routing it first lets the Runtime cancel what
                // the window held and hand the IME its state again.
                self.deliver_host_input(event_loop, id, InputPayload::Focus { focused: *focused });
                if !self.window_contexts.contains_key(&id) {
                    return;
                }
                self.forward_window_event(event_loop, id, &event);
            }
            WinitWindowEvent::DragEntered { .. }
            | WinitWindowEvent::DragPosition { .. }
            | WinitWindowEvent::DragDropped { .. }
            | WinitWindowEvent::DragLeft { .. }
            | WinitWindowEvent::DataTransferReceived { .. } => {
                // A file drag is input: it reaches drop targets through the
                // router and the program through `input_event`.
                if let Some(drag) = self.handle_file_dnd(event_loop, id, &event) {
                    self.deliver_host_input(event_loop, id, InputPayload::FileDrag(drag));
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
    ) -> Option<FileDragInput> {
        let scale = self.scale_factor(id);
        self.input_mut(id).handle_file_dnd(event_loop, scale, event)
    }
    /// Stamp `lowered` into the window's endpoint. A transition drains at
    /// once, with everything queued before it; a move or wheel waits for the
    /// end of the event-loop turn, merging with the ones after it.
    /// Deliver input the host makes itself (focus, a file drag, a pointer
    /// it ends), on the device the window last heard from.
    pub(super) fn deliver_host_input(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        payload: InputPayload,
    ) -> nana_ui_platform::InputDisposition {
        let now = self.animation_clock.runtime_time(Instant::now());
        let device = self.input_of(id).last_device();
        self.deliver_input(event_loop, id, LoweredInput::event(device, payload, now))
    }

    pub(super) fn deliver_input(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        lowered: LoweredInput,
    ) -> nana_ui_platform::InputDisposition {
        // The title bar follows its gesture as the platform delivers it: a
        // native drag must start inside the event that moved past the
        // threshold, not at the end of the turn.
        let chrome_action = self.title_bar_chrome_action(id, &lowered.payload);
        let deferred = lowered.payload.is_coalescible() && chrome_action.is_none();
        if !self.enqueue_input(event_loop, id, lowered) {
            return nana_ui_platform::InputDisposition::default();
        }
        let disposition = if deferred {
            // The turn ends in `about_to_wait`, which drains this sample.
            // That drain asks for a frame only when the route invalidated work.
            nana_ui_platform::InputDisposition::default()
        } else {
            self.drain_window_input(event_loop, id)
        };
        if chrome_action.is_some() && self.window_contexts.contains_key(&id) {
            let update =
                self.merge_title_bar_chrome(id, chrome_action, RuntimeProgramUpdate::default());
            self.apply_update(event_loop, update, None);
        }
        disposition
    }

    /// Push `lowered` (and the text it types) onto the window's endpoint.
    /// A full endpoint drains first; nothing is dropped for capacity.
    fn enqueue_input(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        lowered: LoweredInput,
    ) -> bool {
        let LoweredInput {
            device,
            payload,
            text,
            now,
        } = lowered;
        let Some(key) = self.push_input(event_loop, id, device, payload, now) else {
            return false;
        };
        if let Some(text) = text {
            // The key transition must still be drained when its committed text
            // cannot fit the bounded payload budget. `push_input` reports the
            // capacity failure to the host; keep the accepted key on the
            // endpoint and make the failure explicit instead of silently
            // dropping the return value (and potentially the key as well).
            if self
                .push_input(
                    event_loop,
                    id,
                    device,
                    InputPayload::Text(nana_ui_platform::CommittedText {
                        text,
                        key: Some(key),
                    }),
                    now,
                )
                .is_none()
            {
                self.drain_window_input(event_loop, id);
                return false;
            }
        }
        true
    }

    fn push_input(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        device: nana_ui_platform::DeviceId,
        payload: InputPayload,
        now: Duration,
    ) -> Option<nana_ui_platform::InputSequence> {
        let host = self.window_contexts.get_mut(&id)?;
        let event = nana_ui_platform::CanonicalInputEvent {
            metadata: host.input_source.sequencer.stamp(device, now),
            payload,
        };
        let sequence = event.metadata.sequence;
        let Err(event) = host.input_source.endpoint.push(event) else {
            return Some(sequence);
        };
        // Full: route what is queued, then queue this one behind it.
        self.drain_window_input(event_loop, id);
        let host = self.window_contexts.get_mut(&id)?;
        if host.input_source.endpoint.push(event).is_err() {
            self.program
                .report_host_failure(HostFailure::InputDispatch {
                    window: id,
                    error: "window input endpoint stayed full after a drain".into(),
                });
            return None;
        }
        Some(sequence)
    }

    /// Route everything the window's endpoint holds, then let the program see
    /// each event with what routing learned about it. The one drain every
    /// native input path goes through.
    pub(super) fn drain_window_input(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
    ) -> nana_ui_platform::InputDisposition {
        let mut disposition = nana_ui_platform::InputDisposition::default();
        let Some(host) = self.window_contexts.get_mut(&id) else {
            return disposition;
        };
        if host.input_source.endpoint.is_empty() {
            return disposition;
        }
        let (_, endpoint, input_id, generation) =
            NativeWindowServices::of(&self.clipboard, &mut host.input_source);
        let bound = self.program.write_document(id, |document| {
            let document_id = document.document();
            let context = document.context_mut();
            match context.bind_input_source(input_id, generation, document_id) {
                Ok(()) => Ok(()),
                Err(nana_ui_runtime::InputBindError::DocumentRebind { .. }) => {
                    // The program gave this window another document: what the
                    // old one held is cancelled, and the source starts over.
                    // Events already queued were stamped while the old
                    // document was active; there is no document id in the
                    // canonical envelope, so never replay them into the new
                    // document.
                    while endpoint.pop().is_some() {}
                    let _ = context.unbind_input_source(input_id, Duration::ZERO);
                    context.bind_input_source(input_id, generation, document_id)
                }
                Err(error) => Err(error),
            }
        });
        match bound {
            Some(Err(error)) => {
                self.program
                    .report_host_failure(HostFailure::InputDispatch {
                        window: id,
                        error: error.to_string(),
                    });
                return disposition;
            }
            None => {
                // No document: the batch cannot be routed; drop it rather
                // than hold it against a window that will never route it.
                if let Some(host) = self.window_contexts.get_mut(&id) {
                    while host.input_source.endpoint.pop().is_some() {}
                }
                return disposition;
            }
            Some(Ok(())) => {}
        }
        let mut update = RuntimeProgramUpdate::default();
        let mut pointer_moved = false;
        // One event at a time: the program observes each event before the
        // next one routes, so what it does with a key press (a page's
        // `keydown.preventDefault()`) stops the text that key typed.
        while let Some(host) = self.window_contexts.get_mut(&id) {
            let text = &mut self.text;
            let (mut services, endpoint, _, _) =
                NativeWindowServices::of(&self.clipboard, &mut host.input_source);
            let routed_event = self
                .program
                .write_document(id, |document| {
                    document
                        .context_mut()
                        .route_next(endpoint, &mut services, Some(text))
                })
                .flatten();
            pointer_moved |= services.cursor_changed;
            let Some(routed_event) = routed_event else {
                break;
            };
            let outcome = match routed_event.result {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.program
                        .report_host_failure(HostFailure::InputDispatch {
                            window: id,
                            error: error.to_string(),
                        });
                    continue;
                }
            };
            pointer_moved |= matches!(routed_event.event.payload, InputPayload::Pointer(_));
            let event_disposition = outcome.disposition();
            disposition.handled |= event_disposition.handled;
            disposition.prevent_default |= event_disposition.prevent_default;
            let program_input = self.program.input_event(
                id,
                crate::RoutedInput {
                    event: &routed_event.event,
                    pointer_hit: outcome.pointer_hit,
                    disposition: event_disposition,
                },
                &self.context_for(id),
            );
            if let Err(error) = &program_input {
                self.program.report_host_failure(HostFailure::InputHandler {
                    window: id,
                    error: error.to_string(),
                });
            }
            update = update.merge(scene_runtime_input_update(
                event_disposition,
                id,
                program_input,
            ));
            if outcome.invalidated_work {
                update = update.merge(RuntimeProgramUpdate::redraw(id));
            }
        }
        if !self.window_contexts.contains_key(&id) {
            return disposition;
        }
        if pointer_moved {
            self.apply_window_cursor(id, Some(event_loop));
        }
        if self
            .program
            .read_document(id, |document| document.context().has_program_messages())
            .unwrap_or(false)
        {
            update = update.merge(RuntimeProgramUpdate::redraw(id));
        }
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
        if !event_loop.exiting() && self.window_contexts.contains_key(&id) {
            self.drain_color_pick_requests(event_loop, id);
        }
        disposition
    }

    /// Drain every window whose moves waited for the end of the turn.
    pub(super) fn drain_deferred_input(&mut self, event_loop: &dyn ActiveEventLoop) {
        let pending: Vec<WindowId> = self
            .window_contexts
            .iter()
            .filter(|(_, host)| !host.input_source.endpoint.is_empty())
            .map(|(id, _)| *id)
            .collect();
        for id in pending {
            if event_loop.exiting() {
                return;
            }
            self.drain_window_input(event_loop, id);
        }
    }

    /// Report a pen that left the window or its sensing range, which the
    /// backend does not deliver (see [`nana_window::PenInput`]).
    pub(super) fn deliver_pen_leave(&mut self, event_loop: &dyn ActiveEventLoop, id: WindowId) {
        if !self
            .window_contexts
            .get(&id)
            .is_some_and(|host| host.pen.take_leave())
        {
            return;
        }
        self.deliver_host_input(
            event_loop,
            id,
            InputPayload::PointerLeave {
                pointer_id: nana_ui_platform::PointerId(pen_pointer_id()),
            },
        );
    }

    pub(super) fn deliver_pen_leaves(&mut self, event_loop: &dyn ActiveEventLoop) {
        let ids: Vec<WindowId> = self.window_contexts.keys().copied().collect();
        for id in ids {
            if event_loop.exiting() {
                return;
            }
            self.deliver_pen_leave(event_loop, id);
        }
    }

    /// Bring the window's cursor and IME in line with a world that changed
    /// without input: after a frame laid out, or a caret moved.
    pub(super) fn refresh_window_input(
        &mut self,
        id: WindowId,
        event_loop: Option<&dyn ActiveEventLoop>,
    ) {
        let Some(host) = self.window_contexts.get_mut(&id) else {
            return;
        };
        let (mut services, _, input_id, _) =
            NativeWindowServices::of(&self.clipboard, &mut host.input_source);
        self.program.write_document(id, |document| {
            document
                .context_mut()
                .refresh_input_effects(input_id, &mut services);
        });
        if services.cursor_changed {
            self.apply_window_cursor(id, event_loop);
        }
    }

    /// Bring every window's native IME to the state the Runtime last asked
    /// for. Runs at the end of the turn, outside any window message: an IME
    /// request made while the platform delivers a key or a composition
    /// re-enters the window procedure.
    pub(super) fn apply_window_ime(&mut self) {
        for (id, host) in &mut self.window_contexts {
            let apply = host.input_source.take_ime_apply();
            if let Err(error) =
                host_services::apply_text_input(host.surface.window().as_ref(), apply)
            {
                host.input_source.applied_ime = None;
                self.program
                    .report_host_failure(HostFailure::ImeDispatch { window: *id, error });
            }
        }
    }

    /// Show the cursor: the program's override, else a frame edge under the
    /// pointer, else what the Runtime asked for.
    pub(super) fn apply_window_cursor(
        &mut self,
        id: WindowId,
        event_loop: Option<&dyn ActiveEventLoop>,
    ) {
        let Some(host) = self.window_contexts.get(&id) else {
            return;
        };
        let cursor = host.input.cursor();
        let scale_milli = nana_window::cursor::quantize_scale_milli(host.geometry.scale_factor);
        let frame_edge = self.frame_resize_edge_at(id, cursor.0, cursor.1);
        let Some(host) = self.window_contexts.get(&id) else {
            return;
        };
        let (resolved_cursor, visible) = match frame_edge {
            Some(edge) => (
                host_services::NativeCursor::System(frame_edge_cursor(edge)),
                true,
            ),
            None => host_services::native_cursor(&host.input_source.runtime_cursor),
        };
        let resolved_cursor = host.cursor_override.clone().unwrap_or(resolved_cursor);
        let visible = host.cursor_visible_override.unwrap_or(visible);
        let target_applied = match &resolved_cursor {
            host_services::NativeCursor::System(icon) => {
                host_services::AppliedCursor::System(*icon)
            }
            host_services::NativeCursor::Custom(spec) => {
                host_services::AppliedCursor::Custom(spec.clone(), scale_milli)
            }
        };
        if host.input_source.applied_cursor.as_ref() == Some(&(target_applied.clone(), visible)) {
            return;
        }
        let window = Arc::clone(host.surface.window());
        if !visible {
            if let Some(host) = self.window_contexts.get_mut(&id) {
                host.input_source.applied_cursor = Some((target_applied, false));
            }
            nana_window::cursor::clear_custom_cursor(window.as_ref());
            window.set_cursor_visible(false);
            return;
        }
        match resolved_cursor {
            host_services::NativeCursor::System(icon) => {
                if let Some(host) = self.window_contexts.get_mut(&id) {
                    host.input_source.applied_cursor = Some((target_applied, true));
                }
                nana_window::cursor::clear_custom_cursor(window.as_ref());
                window.set_cursor_visible(true);
                window.set_cursor(icon.into());
            }
            host_services::NativeCursor::Custom(spec) => {
                let applied =
                    self.apply_custom_cursor_spec(window.as_ref(), &spec, scale_milli, event_loop);
                if let Some(host) = self.window_contexts.get_mut(&id) {
                    host.input_source.applied_cursor = Some((
                        if applied {
                            target_applied
                        } else {
                            host_services::AppliedCursor::System(host_services::system_cursor_icon(
                                spec.fallback(),
                            ))
                        },
                        true,
                    ));
                }
            }
        }
    }

    fn apply_custom_cursor_spec(
        &mut self,
        window: &dyn winit::window::Window,
        spec: &nana_ui_core::CustomCursorSpec,
        scale_milli: u32,
        event_loop: Option<&dyn ActiveEventLoop>,
    ) -> bool {
        if let Some(bitmap) = prepare_custom_cursor_bitmap(spec, scale_milli) {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                let _ = event_loop;
                if let Ok(handle) = self.cursor_cache.get_or_create(&bitmap)
                    && matches!(
                        nana_window::cursor::apply_custom_cursor(
                            window,
                            &handle,
                            bitmap.scale_milli
                        ),
                        nana_window::cursor::CustomCursorOutcome::Applied { .. }
                    )
                {
                    window.set_cursor_visible(true);
                    return true;
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                let key = (bitmap.cache_key, bitmap.scale_milli);
                let custom = if let Some(cached) = self.winit_custom_cursors.get(&key) {
                    Some(cached.clone())
                } else if let Some(event_loop) = event_loop
                    && let Ok(source) = winit::cursor::CustomCursorSource::from_rgba(
                        bitmap.rgba.to_vec(),
                        bitmap.width,
                        bitmap.height,
                        bitmap.hotspot[0],
                        bitmap.hotspot[1],
                    )
                    && let Ok(created) = event_loop.create_custom_cursor(source)
                {
                    if self.winit_custom_cursors.len() >= nana_window::cursor::MAX_CACHED_CURSORS {
                        self.winit_custom_cursors.clear();
                    }
                    self.winit_custom_cursors.insert(key, created.clone());
                    Some(created)
                } else {
                    None
                };
                if let Some(custom) = custom {
                    window.set_cursor_visible(true);
                    window.set_cursor(custom.into());
                    return true;
                }
            }
        }
        nana_window::cursor::clear_custom_cursor(window);
        let fallback_visible = spec.fallback() != nana_ui_core::SystemCursor::None;
        window.set_cursor_visible(fallback_visible);
        if fallback_visible {
            window.set_cursor(host_services::system_cursor_icon(spec.fallback()).into());
        }
        false
    }

    pub(super) fn input_of(&self, id: WindowId) -> &WinitInputAdapter {
        &self
            .window_contexts
            .get(&id)
            .expect("input belongs to a live window")
            .input
    }
    pub(super) fn input_mut(&mut self, id: WindowId) -> &mut WinitInputAdapter {
        &mut self
            .window_contexts
            .get_mut(&id)
            .expect("input belongs to a live window")
            .input
    }
}

pub(super) fn prepare_custom_cursor_bitmap(
    spec: &nana_ui_core::CustomCursorSpec,
    scale_milli: u32,
) -> Option<nana_window::cursor::CursorBitmap> {
    use std::hash::{Hash as _, Hasher as _};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    spec.hash(&mut hasher);
    let cache_key = hasher.finish();
    let target_scale = scale_milli.max(250);

    match spec {
        nana_ui_core::CustomCursorSpec::Raster { .. } => {
            let variant = spec.select_variant(target_scale)?;
            let scale_ratio = 1000.0_f64 / f64::from(variant.scale_milli.max(1));
            let logical_width = ((f64::from(variant.width) * scale_ratio).round() as u16).max(1);
            let logical_height = ((f64::from(variant.height) * scale_ratio).round() as u16).max(1);
            let logical_hotspot = [
                f64::from(variant.hotspot[0]) * scale_ratio,
                f64::from(variant.hotspot[1]) * scale_ratio,
            ];
            Some(nana_window::cursor::CursorBitmap {
                cache_key,
                scale_milli: variant.scale_milli,
                logical_width,
                logical_height,
                logical_hotspot,
                width: variant.width,
                height: variant.height,
                hotspot: variant.hotspot,
                rgba: Arc::clone(&variant.rgba),
            })
        }
        nana_ui_core::CustomCursorSpec::Vector {
            source,
            logical_size,
            hotspot,
            ..
        } => {
            if !spec.is_valid() {
                return None;
            }
            let width =
                ((u32::from(logical_size[0]) * target_scale + 500) / 1000).clamp(1, 256) as u16;
            let height =
                ((u32::from(logical_size[1]) * target_scale + 500) / 1000).clamp(1, 256) as u16;
            let hot_x = ((u32::from(hotspot[0]) * u32::from(width)) / u32::from(logical_size[0]))
                .min(u32::from(width - 1)) as u16;
            let hot_y = ((u32::from(hotspot[1]) * u32::from(height)) / u32::from(logical_size[1]))
                .min(u32::from(height - 1)) as u16;
            let rgba = match source {
                nana_ui_core::CustomCursorVector::Shapes { shapes, color } => {
                    crate::scene_paint::icon_raster::rasterize_shapes_rgba(
                        shapes,
                        *color,
                        u32::from(width),
                        u32::from(height),
                    )?
                }
                nana_ui_core::CustomCursorVector::Svg(bytes) => {
                    crate::scene_paint::icon_raster::rasterize_svg_rgba(
                        cache_key,
                        bytes,
                        u32::from(width),
                        u32::from(height),
                    )?
                }
            };
            Some(nana_window::cursor::CursorBitmap {
                cache_key,
                scale_milli: target_scale,
                logical_width: logical_size[0],
                logical_height: logical_size[1],
                logical_hotspot: [f64::from(hotspot[0]), f64::from(hotspot[1])],
                width,
                height,
                hotspot: [hot_x, hot_y],
                rgba: Arc::from(rgba),
            })
        }
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use nana_ui_core::{
        CustomCursorSpec, CustomCursorVariant, CustomCursorVector, IconPathCommand, IconShape,
        SystemCursor,
    };

    #[test]
    fn prepare_custom_cursor_bitmap_selects_raster_dpi_variant_and_scales_vector_shapes() {
        let v1_bytes: Arc<[u8]> = Arc::from(vec![255u8; 16 * 16 * 4]);
        let v1 = CustomCursorVariant::new(1000, 16, 16, [4, 2], v1_bytes).unwrap();
        let v2_bytes: Arc<[u8]> = Arc::from(vec![200u8; 32 * 32 * 4]);
        let v2 = CustomCursorVariant::new(2000, 32, 32, [8, 4], v2_bytes).unwrap();
        let raster = CustomCursorSpec::Raster {
            variants: Arc::from(vec![v1, v2]),
            fallback: SystemCursor::Pointer,
        };

        let at_1x = prepare_custom_cursor_bitmap(&raster, 1000).unwrap();
        assert_eq!((at_1x.width, at_1x.height), (16, 16));
        assert_eq!(at_1x.hotspot, [4, 2]);
        assert_eq!((at_1x.logical_width, at_1x.logical_height), (16, 16));
        assert_eq!(at_1x.scale_milli, 1000);

        let at_2x = prepare_custom_cursor_bitmap(&raster, 2000).unwrap();
        assert_eq!((at_2x.width, at_2x.height), (32, 32));
        assert_eq!(at_2x.hotspot, [8, 4]);
        assert_eq!((at_2x.logical_width, at_2x.logical_height), (16, 16));
        assert_eq!(at_2x.scale_milli, 2000);

        const RECT_COMMANDS: &[IconPathCommand] = &[
            IconPathCommand::MoveTo([2.0, 2.0]),
            IconPathCommand::LineTo([22.0, 2.0]),
            IconPathCommand::LineTo([22.0, 22.0]),
            IconPathCommand::LineTo([2.0, 22.0]),
            IconPathCommand::Close,
        ];
        let shape_slice: Arc<[IconShape]> = Arc::from(vec![IconShape::Fill(RECT_COMMANDS)]);
        let vector = CustomCursorSpec::Vector {
            source: CustomCursorVector::Shapes {
                shapes: shape_slice,
                color: [255, 128, 0, 255],
            },
            logical_size: [16, 16],
            hotspot: [8, 4],
            fallback: SystemCursor::Crosshair,
        };

        let vec_2x = prepare_custom_cursor_bitmap(&vector, 2000).unwrap();
        assert_eq!((vec_2x.width, vec_2x.height), (32, 32));
        assert_eq!(vec_2x.hotspot, [16, 8]);
        assert_eq!((vec_2x.logical_width, vec_2x.logical_height), (16, 16));
        assert!(vec_2x.rgba.chunks_exact(4).any(|px| px[3] > 0));
    }

    #[test]
    fn prepare_custom_cursor_bitmap_svg_respects_feature_and_falls_back_cleanly() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="2" y="2" width="12" height="12" fill="white"/></svg>"#;
        let spec = CustomCursorSpec::Vector {
            source: CustomCursorVector::Svg(Arc::from(svg.as_slice())),
            logical_size: [16, 16],
            hotspot: [2, 3],
            fallback: SystemCursor::Crosshair,
        };
        let bitmap = prepare_custom_cursor_bitmap(&spec, 1500);
        if cfg!(feature = "svg") {
            let bmp = bitmap.expect("svg cursor should rasterize when svg feature is enabled");
            assert_eq!((bmp.width, bmp.height), (24, 24));
            assert_eq!(bmp.hotspot, [3, 4]);
            assert!(bmp.rgba.chunks_exact(4).any(|px| px[3] > 0));
        } else {
            assert!(bitmap.is_none());
            assert_eq!(spec.fallback(), SystemCursor::Crosshair);
        }
    }
}
