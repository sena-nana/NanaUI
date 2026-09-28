//! Vue host input projection boundary.

use crate::*;
use nana_ui_platform::{CanonicalInputEvent, CommittedText, InputPayload};

#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) file_drag_target: Option<NodeHandle>,
    /// The field a composition started in and its last preedit, for the
    /// page's `compositionend` after the Runtime already dropped its
    /// [`nana_ui_runtime::ImeComposition`].
    pub(crate) ime: Option<(NodeHandle, String)>,
    /// The last key press a control handled or the page prevented; text
    /// naming it is not typed.
    #[cfg(feature = "hosted")]
    pub(crate) handled_key: Option<nana_ui_platform::InputSequence>,
    /// Last focus/hover emitted to JS. Scene-host input updates Runtime first;
    /// these remember the previous JS view so blur/over events still fire.
    pub(crate) js_focus: Option<NodeHandle>,
    pub(crate) js_pointer_hover: BTreeMap<u64, Option<NodeHandle>>,
    /// This window's input source when no scene host routes for it; bound
    /// on first use.
    pub(crate) source: Option<nana_ui_runtime::HeadlessInput>,
    /// The generation the last detached source had; the next binds above
    /// it, so a reopened window never reuses one.
    pub(crate) retired_generation: u64,
}

impl VueHost {
    pub(crate) fn register_input_host_ops(&self, api: &mut HostApiRegistry) {
        {
            let document = Arc::clone(&self.document);
            api.register("setPointerCapture", move |args| {
                let node = args
                    .first()
                    .and_then(HostValue::as_f64)
                    .map(|id| NodeHandle(id as u64))
                    .ok_or_else(|| nana_js_engine::JsException::new("missing pointer node"))?;
                let pointer_id = args
                    .get(1)
                    .and_then(HostValue::as_f64)
                    .map(|id| id as u64)
                    .ok_or_else(|| nana_js_engine::JsException::new("missing pointer id"))?;
                let mut document = document
                    .lock()
                    .map_err(|_| nana_js_engine::JsException::new("vue doc poisoned"))?;
                if document.element_tag(node).is_none() {
                    return Err(nana_js_engine::JsException::new(
                        "pointer node is not mounted",
                    ));
                }
                if !document.capture_pointer(pointer_id, node) {
                    return Err(nana_js_engine::JsException::new(
                        "pointer capture could not be committed",
                    ));
                }
                Ok(HostValue::Null)
            });
        }
        {
            let document = Arc::clone(&self.document);
            api.register("releasePointerCapture", move |args| {
                let node = args
                    .first()
                    .and_then(HostValue::as_f64)
                    .map(|id| NodeHandle(id as u64));
                let pointer_id = args
                    .get(1)
                    .and_then(HostValue::as_f64)
                    .map(|id| id as u64)
                    .ok_or_else(|| nana_js_engine::JsException::new("missing pointer id"))?;
                let mut document = document
                    .lock()
                    .map_err(|_| nana_js_engine::JsException::new("vue doc poisoned"))?;
                let released = node.is_some_and(|node| document.release_pointer(pointer_id, node));
                Ok(HostValue::Bool(released))
            });
        }
        {
            let document = Arc::clone(&self.document);
            api.register("hasPointerCapture", move |args| {
                let node = args
                    .first()
                    .and_then(HostValue::as_f64)
                    .map(|id| NodeHandle(id as u64));
                let pointer_id = args
                    .get(1)
                    .and_then(HostValue::as_f64)
                    .map(|id| id as u64)
                    .ok_or_else(|| nana_js_engine::JsException::new("missing pointer id"))?;
                let captured = document
                    .lock()
                    .map_err(|_| nana_js_engine::JsException::new("vue doc poisoned"))?
                    .pointer_capture(pointer_id);
                Ok(HostValue::Bool(captured == node && node.is_some()))
            });
        }
    }
    /// Fire the DOM-style drag events for a file drag the Runtime already
    /// routed, through the same Vue event tree as pointer input. Only nodes
    /// registered with `drop-accepts` receive events; hit-testing uses Runtime
    /// layout boxes, not pointer-event hits. Dropped files are descriptors
    /// with an absolute path; reading their contents remains an application
    /// Host API decision.
    pub fn emit_file_drag_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        kind: FileDragKind,
        paths: &[PathBuf],
        position: Option<(f32, f32)>,
    ) -> Result<bool, JsEngineError> {
        let drop_at = match kind {
            FileDragKind::Cancel => None,
            FileDragKind::Hover | FileDragKind::Drop => position.and_then(|(x, y)| {
                let doc = self.document.lock().expect("vue doc");
                let document = doc.runtime_document().document();
                doc.context()
                    .drop_target_at(document, x, y, &nana_ui_core::DropKind::Files)
                    .map(|(id, _)| NodeHandle::from(id))
            }),
        };
        let detail = file_drag_detail(paths, position);
        let mut allowed = true;
        let previous = self.input_projection.file_drag_target;

        match kind {
            FileDragKind::Hover => {
                if previous != drop_at {
                    if let Some(previous) = previous {
                        allowed &= self.fire_file_drag_event(
                            engine,
                            previous,
                            &["dragleave", "fileleave"],
                            detail.clone(),
                        )?;
                    }
                    if let Some(target) = drop_at {
                        allowed &= self.fire_file_drag_event(
                            engine,
                            target,
                            &["dragenter", "filehover"],
                            detail.clone(),
                        )?;
                    }
                    self.input_projection.file_drag_target = drop_at;
                }
                if let Some(target) = drop_at {
                    allowed &= self.fire_dom_event(engine, target, "dragover", detail)?;
                }
            }
            FileDragKind::Drop => {
                if let Some(target) = drop_at {
                    allowed &= self.fire_file_drag_event(
                        engine,
                        target,
                        &["drop", "filedrop"],
                        detail.clone(),
                    )?;
                }
                if let Some(previous) = self.input_projection.file_drag_target.take()
                    && drop_at != Some(previous)
                {
                    allowed &= self.fire_file_drag_event(
                        engine,
                        previous,
                        &["dragleave", "fileleave"],
                        detail,
                    )?;
                }
            }
            FileDragKind::Cancel => {
                if let Some(previous) = self.input_projection.file_drag_target.take() {
                    allowed &= self.fire_file_drag_event(
                        engine,
                        previous,
                        &["dragleave", "fileleave"],
                        detail,
                    )?;
                }
            }
        }
        engine.run_microtasks()?;
        Ok(allowed)
    }

    fn fire_file_drag_event<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        target: NodeHandle,
        names: &[&str],
        detail: BTreeMap<String, HostValue>,
    ) -> Result<bool, JsEngineError> {
        let mut allowed = true;
        for name in names {
            allowed &= self.fire_dom_event(engine, target, name, detail.clone())?;
        }
        Ok(allowed)
    }
    /// Route a Runtime/bridge action into the queue and JS event listeners.
    pub fn dispatch_bridge_event<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: BridgeEvent,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_bridge_event_inner(engine, event, true)
    }
    pub(crate) fn dispatch_bridge_event_inner<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: BridgeEvent,
        emit_compatibility_click: bool,
    ) -> Result<bool, JsEngineError> {
        let id = event.widget_id();
        if let BridgeEvent::Scroll {
            id,
            offset,
            metrics,
        } = event
        {
            let mut document = self.document.lock().expect("vue doc");
            let changed = crate::scroll::sync_host_scroll_offset(
                &mut document,
                &self.layout_boxes,
                id,
                offset,
                metrics,
            );
            return Ok(changed);
        }
        let committed_input = match &event {
            BridgeEvent::Input { id, value } => Some((*id, value.as_str())),
            _ => None,
        };
        if let Some((id, value)) = committed_input {
            let target = NodeHandle(id);
            let mut document = self.document.lock().expect("vue doc");
            let Some(mut state) = document.text_input_state(target) else {
                return Err(JsEngineError::new(
                    "native input target has no retained text input state",
                ));
            };
            state.synchronize_editor_value(value);
            document.set_text_input_state(target, state);
            document.set_attribute(target, "value", value);
        }
        if let BridgeEvent::Native { name, payload, .. } = &event {
            let detail = match payload {
                HostValue::Object(detail) => detail.clone(),
                value => BTreeMap::from([("value".into(), value.clone())]),
            };
            self.fire_dom_event(engine, NodeHandle(id), name, detail)?;
            engine.run_microtasks()?;
            let _ = self.pump_frame(engine)?;
            return Ok(true);
        }
        let js_events = {
            let mut bridge = self.bridge.lock().expect("vue bridge");
            match &event {
                BridgeEvent::Press { id } => bridge.note_press(*id),
                BridgeEvent::Toggle { id, value } => bridge.note_toggle(*id, *value),
                BridgeEvent::Select { id } => bridge.note_select(*id),
                BridgeEvent::SelectValue { id, value } => {
                    bridge.note_select_value(*id, value.clone())
                }
                BridgeEvent::Input { id, value } => bridge.note_input(*id, value.clone()),
                BridgeEvent::Change { id, value } => bridge.note_change(*id, *value),
                BridgeEvent::Scroll { .. } | BridgeEvent::Native { .. } => Vec::new(),
                #[cfg(feature = "scene-view")]
                BridgeEvent::MenuSearch { .. } | BridgeEvent::MenuPath { .. } => {
                    // Host-only menu chrome; no JS listener required.
                    Vec::new()
                }
            }
        };
        #[cfg(feature = "scene-view")]
        if matches!(
            &event,
            BridgeEvent::MenuSearch { .. } | BridgeEvent::MenuPath { .. }
        ) {
            return Ok(true);
        }
        if js_events.is_empty() {
            return Ok(false);
        }
        for name in js_events {
            if name == "click" && !emit_compatibility_click {
                continue;
            }
            let mut detail = BTreeMap::new();
            match &event {
                BridgeEvent::Toggle { value, .. } => {
                    detail.insert("value".into(), HostValue::Bool(*value));
                    detail.insert("checked".into(), HostValue::Bool(*value));
                }
                BridgeEvent::SelectValue { value, .. } | BridgeEvent::Input { value, .. } => {
                    detail.insert("value".into(), HostValue::string(value));
                }
                BridgeEvent::Change { value, .. } => {
                    detail.insert("value".into(), HostValue::Number(*value));
                }
                _ => {}
            }
            self.fire_dom_event(engine, NodeHandle(id), name, detail)?;
        }
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        // Drop drained duplicates from note_* (host already consumed the intent).
        let _ = self.bridge.lock().expect("vue bridge").drain_events();
        Ok(true)
    }
    fn dispatch_chip_dismiss<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
    ) -> Result<bool, JsEngineError> {
        let outcome = {
            let doc = self.document.lock().expect("vue doc");
            let Ok(id) = nana_ui_runtime::StableNodeId::try_from(target) else {
                return Ok(false);
            };
            let Some(chip) = doc.context().chip_dismiss_target(id) else {
                return Ok(false);
            };
            let disabled = doc
                .context()
                .read(chip, |chip| chip.disabled)
                .unwrap_or(true);
            (NodeHandle::from(chip.stable_id()), disabled)
        };
        let (chip, disabled) = outcome;
        if disabled {
            return Ok(true);
        }
        self.fire_dom_event(engine, chip, "dismiss", BTreeMap::new())?;
        Ok(true)
    }

    pub(crate) fn fire_dom_event<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        target: NodeHandle,
        name: &str,
        detail: BTreeMap<String, HostValue>,
    ) -> Result<bool, JsEngineError> {
        let fire = self.callbacks.fire_event.ok_or_else(|| {
            JsEngineError::new("__nanaFireEvent is not bound; call bind_event_bridge")
        })?;
        let args = match self.callbacks.event_window_id {
            Some(window_id) => vec![
                HostValue::Number(window_id as f64),
                HostValue::Number(target.0 as f64),
                HostValue::string(name),
                HostValue::Object(detail),
            ],
            None => vec![
                HostValue::Number(target.0 as f64),
                HostValue::string(name),
                HostValue::Object(detail),
            ],
        };
        let result = engine.invoke(fire, &args)?;
        Ok(result.as_bool().unwrap_or(true))
    }
    pub(crate) fn drain_native_dom_events<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        if self.callbacks.fire_event.is_none() {
            return Ok(());
        }
        let events = self.document.lock().expect("vue doc").take_native_events();
        for event in events {
            self.fire_dom_event(engine, NodeHandle(event.id), event.name, event.detail)?;
        }
        Ok(())
    }
    /// Route one event through the Runtime router, as a window's input
    /// source does. Vue's own input API reaches the Runtime only through
    /// here; the page then observes the event like any routed one.
    pub fn route_input(
        &mut self,
        payload: InputPayload,
    ) -> Result<(CanonicalInputEvent, nana_ui_runtime::InputRouteOutcome), JsEngineError> {
        let mut doc = self.document.lock().expect("vue doc");
        // Focus set outside input (a host, a script) is where this event's
        // blur and focus start from.
        self.input_projection.js_focus = doc.focused();
        let now = doc.runtime_now();
        let document = doc.runtime_document().document();
        let context = doc.context_mut();
        let binding = context.input_binding(nana_ui_runtime::HeadlessInput::SOURCE);
        let retired = self.input_projection.retired_generation;
        let source = match &mut self.input_projection.source {
            Some(source) if binding == Some((source.generation(), document)) => source,
            slot => {
                let newest = binding.map_or(retired, |(generation, _)| generation.0.max(retired));
                slot.insert(
                    nana_ui_runtime::HeadlessInput::bind_source(
                        context,
                        nana_ui_runtime::HeadlessInput::SOURCE,
                        nana_ui_platform::EndpointGeneration(newest + 1),
                        document,
                    )
                    .map_err(|error| JsEngineError::new(error.to_string()))?,
                )
            }
        };
        source.set_now(now.max(source.now()));
        let event = source.stamp(payload);
        let outcome = context
            .route_input(&event, source.services_mut(), None)
            .map_err(|error| JsEngineError::new(error.to_string()))?;
        Ok((event, outcome))
    }

    /// Forget this window's input source: what it held is cancelled while
    /// the document is alive, and the next event binds a newer generation.
    #[cfg(feature = "hosted")]
    pub(crate) fn detach_input_source(&mut self) -> Result<(), JsEngineError> {
        self.input_projection.handled_key = None;
        let Some(source) = self.input_projection.source.take() else {
            return Ok(());
        };
        self.input_projection.retired_generation = source.generation().0;
        let mut doc = self.document.lock().expect("vue doc");
        let now = doc.runtime_now();
        doc.context_mut()
            .unbind_input_source(nana_ui_runtime::HeadlessInput::SOURCE, now)
            .map(|_| ())
            .map_err(|error| JsEngineError::new(error.to_string()))
    }

    pub(crate) fn flush_interactive_css_if_needed(&self) {
        let mut bridge = self.bridge.lock().expect("vue bridge");
        if !bridge.has_interactive_css() {
            return;
        }
        let mut doc = self.document.lock().expect("vue doc");
        bridge.reapply_interactive_cascade(&mut doc);
        bridge.sync_cascaded_layout_into_runtime(&mut doc);
        doc.flush_host_frame();
    }
    pub(crate) fn flush_focus_cascade(
        &self,
        previous: Option<NodeHandle>,
        next: Option<NodeHandle>,
    ) {
        let mut bridge = self.bridge.lock().expect("vue bridge");
        if !bridge.has_interactive_css() && !bridge.has_focus_within_css() {
            return;
        }
        let mut doc = self.document.lock().expect("vue doc");
        bridge.on_runtime_focus_change(
            &mut doc,
            previous.map(|node| node.0),
            next.map(|node| node.0),
        );
        bridge.sync_cascaded_layout_into_runtime(&mut doc);
        doc.flush_host_frame();
    }
    /// Fire `blur` and `focus` for a focus move the page has not heard of,
    /// and restyle what `:focus` and `:focus-within` match.
    fn emit_focus_change<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        let previous = self.input_projection.js_focus;
        let next = self.document.lock().expect("vue doc").focused();
        if previous == next {
            return Ok(());
        }
        if let Some(previous) = previous {
            self.fire_dom_event(engine, previous, "blur", BTreeMap::new())?;
        }
        if let Some(next) = next {
            self.fire_dom_event(engine, next, "focus", BTreeMap::new())?;
        }
        self.flush_focus_cascade(previous, next);
        self.input_projection.js_focus = next;
        Ok(())
    }
    pub(crate) fn pointer_detail(
        &self,
        input: PointerInput,
        target: NodeHandle,
    ) -> BTreeMap<String, HostValue> {
        let mut detail = input.detail();
        let Some(bounds) = self
            .document
            .lock()
            .ok()
            .and_then(|doc| get_layout_box_from(&self.layout_boxes, &doc, target))
        else {
            return detail;
        };
        let (local_x, local_y) = self
            .layout_boxes
            .local_point(target, input.client_x, input.client_y)
            .unwrap_or((input.client_x - bounds.x, input.client_y - bounds.y));
        detail.insert("offsetX".into(), HostValue::Number(local_x as f64));
        detail.insert("offsetY".into(), HostValue::Number(local_y as f64));
        detail
    }
    pub(crate) fn pointer_transition_paths(
        &self,
        previous: Option<NodeHandle>,
        next: Option<NodeHandle>,
    ) -> (Vec<NodeHandle>, Vec<NodeHandle>) {
        let doc = self.document.lock().expect("vue doc");
        let path = |start: Option<NodeHandle>| {
            start
                .and_then(|target| doc.event_route(target))
                .map(|route| {
                    std::iter::once(route.target)
                        .chain(route.bubble)
                        .map(NodeHandle::from)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let previous_path = path(previous);
        let next_path = path(next);
        let common = previous_path
            .iter()
            .find(|node| next_path.contains(node))
            .copied();
        let leaving = previous_path
            .into_iter()
            .take_while(|node| Some(*node) != common)
            .collect();
        let mut entering: Vec<_> = next_path
            .into_iter()
            .take_while(|node| Some(*node) != common)
            .collect();
        entering.reverse();
        (leaving, entering)
    }
    pub(crate) fn flush_pointer_capture_events<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        let changes = self
            .document
            .lock()
            .expect("vue doc")
            .take_pointer_capture_changes();
        self.fire_pointer_capture_changes(engine, changes)
    }
    pub(crate) fn fire_pointer_capture_changes<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        changes: Vec<nana_ui_runtime::PointerCaptureChange>,
    ) -> Result<(), JsEngineError> {
        for change in changes {
            let mut detail = BTreeMap::new();
            detail.insert(
                "pointerId".into(),
                HostValue::Number(change.pointer_id as f64),
            );
            self.fire_dom_event(
                engine,
                NodeHandle::from(change.target),
                if change.captured {
                    "gotpointercapture"
                } else {
                    "lostpointercapture"
                },
                detail,
            )?;
        }
        Ok(())
    }
    pub(crate) fn semantic_default_action<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        requested_value: Option<f64>,
        click_detail: Option<BTreeMap<String, HostValue>>,
    ) -> Result<SemanticActionResult, JsEngineError> {
        let widget = self
            .bridge
            .lock()
            .expect("vue bridge")
            .get(target.0)
            .cloned();
        let Some(widget) = widget else {
            return Ok(SemanticActionResult::default());
        };
        if widget.props.disabled || widget.props.loading {
            return Ok(SemanticActionResult {
                handled: true,
                default_prevented: false,
            });
        }
        if let Some(click_detail) = click_detail
            && !self.fire_dom_event(engine, target, "click", click_detail)?
        {
            return Ok(SemanticActionResult {
                handled: true,
                default_prevented: true,
            });
        }
        if let Some(for_id) = crate::widget_map::attr_value(&widget.props, &["for"])
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            && (widget.props.element_tag.eq_ignore_ascii_case("label")
                || widget.props.role.eq_ignore_ascii_case("label"))
        {
            let associated = {
                let bridge = self.bridge.lock().expect("vue bridge");
                bridge
                    .widgets()
                    .find(|candidate| candidate.props.element_id == for_id)
                    .map(|candidate| NodeHandle(candidate.id))
            };
            if let Some(associated) = associated {
                return self.semantic_default_action(engine, associated, requested_value, None);
            }
        }
        let event = match widget.kind {
            WidgetKind::Switch | WidgetKind::Checkbox | WidgetKind::Radio => {
                Some(BridgeEvent::Toggle {
                    id: target.0,
                    value: !widget.props.toggled,
                })
            }
            WidgetKind::Range => requested_value.map(|value| BridgeEvent::Change {
                id: target.0,
                value: quantize_range_value(&widget.props, value),
            }),
            WidgetKind::ListItem
            | WidgetKind::SidebarRow
            | WidgetKind::InteractiveCard
            | WidgetKind::TableRow => Some(BridgeEvent::Select { id: target.0 }),
            WidgetKind::Button | WidgetKind::IconButton | WidgetKind::Chip => {
                Some(BridgeEvent::Press { id: target.0 })
            }
            WidgetKind::SettingsCollapsibleCard => Some(BridgeEvent::Toggle {
                id: target.0,
                value: !widget.props.toggled,
            }),
            _ => None,
        };
        if let Some(event) = event {
            self.dispatch_bridge_event_inner(engine, event, false)?;
        }
        if widget.kind == WidgetKind::Radio {
            exclusive_check_radios(&self.bridge, target.0);
        }
        if widget.kind == WidgetKind::Button
            && is_submit_control(&widget)
            && let Some(form) = ancestor_form(&self.bridge, target.0)
        {
            let _ = self.fire_dom_event(engine, NodeHandle(form), "submit", BTreeMap::new())?;
        }
        Ok(SemanticActionResult {
            handled: true,
            default_prevented: false,
        })
    }
    /// Fire Vue/DOM pointer events for pointer input the Runtime already routed.
    pub fn emit_pointer_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: PointerInput,
    ) -> Result<HostedInputResult, JsEngineError> {
        let (physical_hit, captured, target, event_target, released) = {
            let mut doc = self.document.lock().expect("vue doc");
            let physical_hit = doc.hit_test(input.client_x, input.client_y);
            let mut captured = doc.pointer_capture(input.pointer_id);
            // A capture held by a node the page unmounted ends here.
            let stale = captured.filter(|&node| doc.element_tag(node).is_none());
            if let Some(node) = stale {
                doc.release_pointer(input.pointer_id, node);
                captured = None;
            }
            let released = stale.is_some();
            let target = captured.or_else(|| match input.kind {
                PointerEventKind::Cancel => doc.pointer_hover(input.pointer_id),
                _ => doc
                    .hit_event_target(input.client_x, input.client_y, input.kind.pointer_name())
                    .or(physical_hit),
            });
            let event_target = target.unwrap_or_else(|| doc.mount_root());
            (physical_hit, captured, target, event_target, released)
        };
        if released {
            self.flush_pointer_capture_events(engine)?;
        }
        let detail = self.pointer_detail(input, event_target);

        if matches!(
            input.kind,
            PointerEventKind::Move | PointerEventKind::Cancel
        ) && captured.is_none()
        {
            let previous = self
                .input_projection
                .js_pointer_hover
                .get(&input.pointer_id)
                .copied()
                .flatten();
            if previous != physical_hit {
                if let Some(previous) = previous {
                    let mut transition = detail.clone();
                    transition.insert(
                        "relatedTarget".into(),
                        physical_hit
                            .map(|node| HostValue::Number(node.0 as f64))
                            .unwrap_or(HostValue::Null),
                    );
                    self.fire_dom_event(engine, previous, "pointerout", transition.clone())?;
                    if input.pointer_type == PointerType::Mouse {
                        self.fire_dom_event(engine, previous, "mouseout", transition.clone())?;
                    }
                }
                if let Some(next) = physical_hit {
                    let mut transition = self.pointer_detail(input, next);
                    transition.insert(
                        "relatedTarget".into(),
                        previous
                            .map(|node| HostValue::Number(node.0 as f64))
                            .unwrap_or(HostValue::Null),
                    );
                    self.fire_dom_event(engine, next, "pointerover", transition.clone())?;
                    if input.pointer_type == PointerType::Mouse {
                        self.fire_dom_event(engine, next, "mouseover", transition.clone())?;
                    }
                }
                let (leaving, entering) = self.pointer_transition_paths(previous, physical_hit);
                for node in leaving {
                    let mut transition = self.pointer_detail(input, node);
                    transition.insert(
                        "relatedTarget".into(),
                        physical_hit
                            .map(|n| HostValue::Number(n.0 as f64))
                            .unwrap_or(HostValue::Null),
                    );
                    self.fire_dom_event(engine, node, "pointerleave", transition.clone())?;
                    if input.pointer_type == PointerType::Mouse {
                        self.fire_dom_event(engine, node, "mouseleave", transition)?;
                    }
                }
                for node in entering {
                    let mut transition = self.pointer_detail(input, node);
                    transition.insert(
                        "relatedTarget".into(),
                        previous
                            .map(|n| HostValue::Number(n.0 as f64))
                            .unwrap_or(HostValue::Null),
                    );
                    self.fire_dom_event(engine, node, "pointerenter", transition.clone())?;
                    if input.pointer_type == PointerType::Mouse {
                        self.fire_dom_event(engine, node, "mouseenter", transition)?;
                    }
                }
                self.flush_interactive_css_if_needed();
                self.input_projection
                    .js_pointer_hover
                    .insert(input.pointer_id, physical_hit);
            }
        }

        let mut default_prevented = !self.fire_dom_event(
            engine,
            event_target,
            input.kind.pointer_name(),
            detail.clone(),
        )?;
        self.flush_pointer_capture_events(engine)?;
        if input.pointer_type == PointerType::Mouse
            && let Some(mouse_name) = input.kind.mouse_name()
        {
            default_prevented |=
                !self.fire_dom_event(engine, event_target, mouse_name, detail.clone())?;
        }

        let mut consumed = false;
        match input.kind {
            PointerEventKind::Down => {
                self.flush_interactive_css_if_needed();
                self.emit_focus_change(engine)?;
            }
            PointerEventKind::Up => {
                let pressed = target.or(physical_hit);
                self.flush_interactive_css_if_needed();
                if !default_prevented
                    && let Some(click_target) = pressed
                    && physical_hit == Some(click_target)
                {
                    if self.dispatch_chip_dismiss(engine, click_target)? {
                        consumed = true;
                    } else {
                        let is_semantic = self
                            .bridge
                            .lock()
                            .expect("vue bridge")
                            .contains(click_target.0);
                        if is_semantic {
                            let requested_value =
                                self.pointer_range_value(click_target, input.client_x);
                            let result = self.semantic_default_action(
                                engine,
                                click_target,
                                requested_value,
                                Some(detail.clone()),
                            )?;
                            default_prevented |= result.default_prevented;
                            consumed = result.handled;
                        } else {
                            default_prevented |= !self.fire_dom_event(
                                engine,
                                click_target,
                                "click",
                                detail.clone(),
                            )?;
                        }
                    }
                }
            }
            PointerEventKind::Cancel => self.flush_interactive_css_if_needed(),
            PointerEventKind::Move => {}
        }
        self.flush_pointer_capture_events(engine)?;
        self.drain_native_dom_events(engine)?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(HostedInputResult {
            targeted: target.is_some(),
            default_prevented,
            consumed,
        })
    }
    pub(crate) fn pointer_range_value(&self, target: NodeHandle, x: f32) -> Option<f64> {
        let widget = self
            .bridge
            .lock()
            .expect("vue bridge")
            .get(target.0)
            .cloned()?;
        if widget.kind != WidgetKind::Range {
            return None;
        }
        let bounds = self.document.lock().expect("vue doc").layout_box(target)?;
        let ratio = if bounds.width > 0.0 {
            ((x - bounds.x) / bounds.width).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Some(
            f64::from(widget.props.min)
                + f64::from(ratio) * f64::from(widget.props.max - widget.props.min),
        )
    }
    /// Route a pointer event, then fire the page's events for it. Whether
    /// it had a target.
    pub fn dispatch_pointer<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: PointerInput,
    ) -> Result<bool, JsEngineError> {
        self.route_input(InputPayload::Pointer(input.to_canonical()))?;
        self.emit_pointer_from_runtime(engine, input)
            .map(|result| result.targeted)
    }
    /// Compatibility helper for callers that only expose an atomic click.
    pub fn pointer_click<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        x: f32,
        y: f32,
    ) -> Result<bool, JsEngineError> {
        let down =
            self.dispatch_pointer(engine, PointerInput::mouse(PointerEventKind::Down, x, y))?;
        let up = self.dispatch_pointer(engine, PointerInput::mouse(PointerEventKind::Up, x, y))?;
        Ok(down || up)
    }
    /// Route a wheel event, then fire the page's `wheel` for it.
    pub fn dispatch_wheel_result<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: WheelInput,
    ) -> Result<HostedInputResult, JsEngineError> {
        self.route_input(InputPayload::Wheel(input.to_canonical()))?;
        self.emit_wheel_from_runtime(engine, input)
    }
    /// Fire the page's `wheel` for a wheel event the Runtime already routed.
    pub fn emit_wheel_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: WheelInput,
    ) -> Result<HostedInputResult, JsEngineError> {
        let target = {
            let doc = self.document.lock().expect("vue doc");
            doc.hit_event_target(input.client_x, input.client_y, "wheel")
                .or_else(|| doc.hit_test(input.client_x, input.client_y))
        };
        let Some(target) = target else {
            return Ok(HostedInputResult::default());
        };
        let allowed = self.fire_dom_event(engine, target, "wheel", input.detail())?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(HostedInputResult {
            targeted: true,
            default_prevented: !allowed,
            consumed: true,
        })
    }
    pub fn pointer_wheel<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_wheel_result(engine, WheelInput::pixels(x, y, delta_x, delta_y))
            .map(|result| result.targeted)
    }
    /// Route a key transition, then fire the page's key event at `target`
    /// (the focused node when `None`). `false` when a control or the page
    /// took the key.
    pub fn dispatch_keyboard<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &KeyboardInput,
        target: Option<NodeHandle>,
    ) -> Result<bool, JsEngineError> {
        let (_, outcome) = self.route_input(InputPayload::Key(input.to_canonical()))?;
        let allowed = self.emit_keyboard_from_runtime(engine, input, target)?;
        Ok(allowed && !outcome.handled)
    }
    /// Fire the page's key event for a key the Runtime already routed.
    pub fn emit_keyboard_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &KeyboardInput,
        target: Option<NodeHandle>,
    ) -> Result<bool, JsEngineError> {
        let target = {
            let doc = self.document.lock().expect("vue doc");
            target
                .or_else(|| doc.focused())
                .unwrap_or_else(|| doc.mount_root())
        };
        let repeated = input.repeat;
        let mut allowed =
            self.fire_dom_event(engine, target, input.kind.as_str(), input.detail())?;
        if allowed && input.kind == KeyboardEventKind::Down {
            let key = input.key.to_ascii_lowercase();
            let activate_key =
                !repeated && matches!(key.as_str(), "enter" | " " | "space" | "spacebar");
            if activate_key && self.dispatch_chip_dismiss(engine, target)? {
                allowed = false;
            } else {
                let widget = self
                    .bridge
                    .lock()
                    .expect("vue bridge")
                    .get(target.0)
                    .cloned();
                if let Some(widget) = widget {
                    // A page range is projection-only in the Runtime: its
                    // keyboard steps are the page's to take.
                    let requested_value = match widget.kind {
                        WidgetKind::Range => match key.as_str() {
                            "arrowleft" | "arrowdown" => {
                                Some(f64::from(widget.props.number - widget.props.step))
                            }
                            "arrowright" | "arrowup" => {
                                Some(f64::from(widget.props.number + widget.props.step))
                            }
                            "pagedown" => {
                                Some(f64::from(widget.props.number - widget.props.step * 10.0))
                            }
                            "pageup" => {
                                Some(f64::from(widget.props.number + widget.props.step * 10.0))
                            }
                            "home" => Some(f64::from(widget.props.min)),
                            "end" => Some(f64::from(widget.props.max)),
                            _ => None,
                        },
                        _ => None,
                    };
                    let activates = match widget.kind {
                        WidgetKind::Button
                        | WidgetKind::IconButton
                        | WidgetKind::Chip
                        | WidgetKind::ListItem
                        | WidgetKind::SidebarRow
                        | WidgetKind::InteractiveCard
                        | WidgetKind::TableRow => activate_key,
                        WidgetKind::Switch | WidgetKind::Checkbox | WidgetKind::Radio => {
                            !repeated && matches!(key.as_str(), " " | "space" | "spacebar")
                        }
                        WidgetKind::SettingsCollapsibleCard => activate_key,
                        WidgetKind::Range => requested_value.is_some(),
                        _ => false,
                    };
                    if activates {
                        let result = self.semantic_default_action(
                            engine,
                            target,
                            requested_value,
                            Some(BTreeMap::new()),
                        )?;
                        if result.handled {
                            allowed = false;
                        }
                    }
                }
            }
        }
        if input.kind == KeyboardEventKind::Down {
            self.emit_key_edit(engine, input)?;
        }
        // Tab, roving arrows or an activated control may have moved focus
        // in the route; the page hears it as a browser's would.
        self.emit_focus_change(engine)?;
        self.drain_native_dom_events(engine)?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(allowed)
    }
    /// A key the route turned into an edit (deleting, pasting, undoing)
    /// reaches the page as `beforeinput` and `input`: the field's text moved
    /// away from the value the page last saw. A key that only moved the caret
    /// changed nothing the page hears.
    fn emit_key_edit<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &KeyboardInput,
    ) -> Result<(), JsEngineError> {
        let Some(target) = self.focused_text_input() else {
            return Ok(());
        };
        let edited = {
            let document = self.document.lock().expect("vue doc");
            document.text_input_state(target).is_some_and(|state| {
                *state.value != document.get_attribute(target, "value").unwrap_or_default()
            })
        };
        if edited {
            self.emit_text_events_from_runtime(engine, target, "", key_input_type(input))?;
        }
        Ok(())
    }
    #[cfg(any(test, feature = "hosted"))]
    pub(crate) fn accessibility_focus<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
    ) -> Result<bool, JsEngineError> {
        let previous = {
            let mut document = self.document.lock().expect("vue doc");
            if document.element_tag(target).is_none() {
                return Ok(false);
            }
            let previous = document.focused();
            if previous == Some(target) {
                return Ok(false);
            }
            document.set_focus(target);
            previous
        };
        if let Some(previous) = previous {
            self.fire_dom_event(engine, previous, "blur", BTreeMap::new())?;
        }
        self.fire_dom_event(engine, target, "focus", BTreeMap::new())?;
        self.input_projection.js_focus = Some(target);
        self.flush_focus_cascade(previous, Some(target));
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn accessibility_click<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
    ) -> Result<bool, JsEngineError> {
        let result = self.semantic_default_action(engine, target, None, Some(BTreeMap::new()))?;
        Ok(result.handled && !result.default_prevented)
    }
    #[cfg(any(test, feature = "hosted"))]
    pub(crate) fn accessibility_set_value<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        value: &str,
    ) -> Result<bool, JsEngineError> {
        let range = {
            let bridge = self.bridge.lock().expect("vue bridge");
            bridge
                .get(target.0)
                .filter(|widget| widget.kind == WidgetKind::Range)
                .cloned()
        };
        if let Some(range) = range {
            if range.props.disabled || range.props.loading {
                return Ok(false);
            }
            let Ok(value) = value.parse::<f64>() else {
                return Ok(false);
            };
            let result = self.semantic_default_action(engine, target, Some(value), None)?;
            return Ok(result.handled && !result.default_prevented);
        }
        let supported = {
            let document = self.document.lock().expect("vue doc");
            document.has_text_input_state(target)
                && document.get_attribute(target, "disabled").is_none()
                && document.get_attribute(target, "readonly").is_none()
        };
        if !supported {
            return Ok(false);
        }

        let next = TextInputState::new(value);
        let mut detail = BTreeMap::new();
        detail.insert("data".into(), HostValue::string(value));
        detail.insert(
            "inputType".into(),
            HostValue::string("insertReplacementText"),
        );
        detail.insert("value".into(), HostValue::string(value));
        detail.insert("isComposing".into(), HostValue::Bool(false));
        if !self.fire_dom_event(engine, target, "beforeinput", detail.clone())? {
            return Ok(false);
        }
        {
            let mut document = self.document.lock().expect("vue doc");
            if !document.set_text_input_state(target, next) {
                return Ok(false);
            }
            document.set_attribute(target, "value", value);
        }
        self.fire_dom_event(engine, target, "input", detail)?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }
    #[cfg(any(test, feature = "hosted"))]
    pub(crate) fn accessibility_set_selection<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        selection: nana_ui_runtime::TextSelection,
    ) -> Result<bool, JsEngineError> {
        {
            let mut document = self.document.lock().expect("vue doc");
            if document.get_attribute(target, "disabled").is_some() {
                return Ok(false);
            }
            let Some(mut state) = document.text_input_state(target) else {
                return Ok(false);
            };
            if !selection.is_valid_for(&state.value) || state.selection == selection {
                return Ok(false);
            }
            state.selection = selection;
            if !document.set_text_input_state(target, state) {
                return Ok(false);
            }
        }
        self.fire_dom_event(engine, target, "select", BTreeMap::new())?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }
    /// Route text the platform committed with no key press (a soft
    /// keyboard, a paste), then fire the page's `beforeinput` and `input`.
    pub fn commit_text<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        text: &str,
        input_type: &str,
    ) -> Result<bool, JsEngineError> {
        self.commit_routed_text(engine, CommittedText::new(text), input_type)
    }
    fn commit_routed_text<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        committed: CommittedText,
        input_type: &str,
    ) -> Result<bool, JsEngineError> {
        let text = committed.text.clone();
        let (_, outcome) = self.route_input(InputPayload::Text(committed))?;
        match self.focused_text_input() {
            Some(target) if outcome.handled => {
                self.emit_text_events_from_runtime(engine, target, &text, input_type)
            }
            _ => Ok(false),
        }
    }
    pub(crate) fn text_commit_blocked(&self, target: NodeHandle) -> bool {
        if self
            .bridge
            .lock()
            .expect("vue bridge")
            .get(target.0)
            .is_some_and(|widget| widget.props.disabled || widget.props.read_only)
        {
            return true;
        }
        let document = self.document.lock().expect("vue doc");
        document.element_tag(target).is_none()
            || document.get_attribute(target, "disabled").is_some()
            || document.get_attribute(target, "readonly").is_some()
    }
    /// The focused node, when text can go into it.
    pub(crate) fn focused_text_input(&self) -> Option<NodeHandle> {
        let document = self.document.lock().expect("vue doc");
        document
            .focused()
            .filter(|&target| document.has_text_input_state(target))
    }
    /// Route a page composition event as the platform IME event it stands
    /// for, then fire the page's composition events.
    pub fn dispatch_composition<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &CompositionInput,
    ) -> Result<bool, JsEngineError> {
        let composition = input.to_canonical();
        let (_, outcome) = self.route_input(InputPayload::Composition(composition.clone()))?;
        if input.kind != CompositionEventKind::Start {
            return self.emit_native_ime_from_runtime(engine, &composition, outcome.handled);
        }
        // The page asked for a start alone, not the platform's start and
        // empty preedit.
        let Some(target) = self.focused() else {
            return Ok(false);
        };
        self.input_projection.ime = Some((target, String::new()));
        self.emit_composition_event(engine, target, input)
    }
    fn emit_composition_event<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        input: &CompositionInput,
    ) -> Result<bool, JsEngineError> {
        let mut detail = BTreeMap::new();
        detail.insert("data".into(), HostValue::string(&input.data));
        detail.insert(
            "isComposing".into(),
            HostValue::Bool(input.kind != CompositionEventKind::End),
        );
        self.fire_dom_event(engine, target, input.kind.as_str(), detail)?;
        engine.run_microtasks()?;
        if input.kind == CompositionEventKind::End && !input.data.is_empty() {
            return self.emit_text_events_from_runtime(
                engine,
                target,
                &input.data,
                "insertCompositionText",
            );
        }
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }
    pub(crate) fn emit_text_events_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        data: &str,
        input_type: &str,
    ) -> Result<bool, JsEngineError> {
        let value = {
            let document = self.document.lock().expect("vue doc");
            document
                .text_input_state(target)
                .map(|state| state.value.to_string())
                .or_else(|| document.get_attribute(target, "value"))
                .unwrap_or_default()
        };
        let mut detail = BTreeMap::new();
        detail.insert("data".into(), HostValue::string(data));
        detail.insert("inputType".into(), HostValue::string(input_type));
        detail.insert("value".into(), HostValue::string(&value));
        detail.insert("isComposing".into(), HostValue::Bool(false));
        if !self.fire_dom_event(engine, target, "beforeinput", detail.clone())? {
            return Ok(false);
        }
        self.document
            .lock()
            .expect("vue doc")
            .set_attribute(target, "value", &value);
        self.fire_dom_event(engine, target, "input", detail)?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }
    /// Route a platform IME event, then fire the page's composition and
    /// `input` events for it.
    pub fn dispatch_native_ime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: &nana_ui_platform::CompositionInput,
    ) -> Result<bool, JsEngineError> {
        let (_, outcome) = self.route_input(InputPayload::Composition(event.clone()))?;
        if matches!(
            event,
            nana_ui_platform::CompositionInput::DeleteSurrounding { .. }
        ) && !outcome.handled
        {
            // A span off a character boundary deletes nothing.
            return Ok(false);
        }
        self.emit_native_ime_from_runtime(engine, event, outcome.handled)
    }
    /// Fire the page's composition events for IME input the Runtime already
    /// routed: preedit lives on the Runtime's `ImeComposition`, commits in
    /// its `TextInputState`. `applied` is whether the route changed the
    /// field. A commit or leftover it did not apply ends the composition
    /// with no text, as a cancelled one does: focus leaving an editor
    /// cancels its preedit, so text never lands in the field focus moved to.
    pub fn emit_native_ime_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: &nana_ui_platform::CompositionInput,
        applied: bool,
    ) -> Result<bool, JsEngineError> {
        match event {
            nana_ui_platform::CompositionInput::Enabled => Ok(self.focused().is_some()),
            // A composition that starts is an empty preedit until it updates.
            nana_ui_platform::CompositionInput::Start => self.emit_native_ime_from_runtime(
                engine,
                &nana_ui_platform::CompositionInput::Update {
                    text: String::new(),
                    selection: None,
                },
                applied,
            ),
            nana_ui_platform::CompositionInput::Update { text, .. } => {
                let Some(target) = self.focused() else {
                    return Ok(false);
                };
                let started = self.input_projection.ime.is_none();
                self.input_projection.ime = Some((target, text.clone()));
                if started {
                    self.emit_composition_event(
                        engine,
                        target,
                        &CompositionInput::new(CompositionEventKind::Start, ""),
                    )?;
                }
                self.emit_composition_event(
                    engine,
                    target,
                    &CompositionInput::new(CompositionEventKind::Update, text),
                )
            }
            nana_ui_platform::CompositionInput::Commit(text) => {
                let Some(target) = self
                    .input_projection
                    .ime
                    .take()
                    .map(|(target, _)| target)
                    .or_else(|| self.focused())
                else {
                    return Ok(false);
                };
                let committed = if applied { text.as_str() } else { "" };
                self.emit_composition_event(
                    engine,
                    target,
                    &CompositionInput::new(CompositionEventKind::End, committed),
                )
            }
            nana_ui_platform::CompositionInput::DeleteSurrounding { .. } => {
                self.emit_native_delete_surrounding(engine)
            }
            nana_ui_platform::CompositionInput::Disabled
            | nana_ui_platform::CompositionInput::End => {
                let leftover = self.input_projection.ime.take();
                let Some((target, data)) = leftover else {
                    return Ok(self.focused().is_some());
                };
                if data.is_empty() {
                    return Ok(true);
                }
                let committed = if applied { data } else { String::new() };
                self.emit_composition_event(
                    engine,
                    target,
                    &CompositionInput::new(CompositionEventKind::End, committed),
                )
            }
        }
    }
    fn emit_native_delete_surrounding<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<bool, JsEngineError> {
        let Some(target) = self.focused() else {
            return Ok(false);
        };
        if self.text_commit_blocked(target) {
            return Ok(true);
        }
        let value = {
            let mut document = self.document.lock().expect("vue doc");
            let Some(value) = document
                .text_input_state(target)
                .map(|state| state.value.to_string())
            else {
                return Ok(false);
            };
            document.set_attribute(target, "value", &value);
            value
        };
        let mut detail = BTreeMap::new();
        detail.insert("data".into(), HostValue::string(""));
        detail.insert("inputType".into(), HostValue::string("deleteContent"));
        detail.insert("value".into(), HostValue::string(&value));
        detail.insert("isComposing".into(), HostValue::Bool(false));
        self.fire_dom_event(engine, target, "input", detail)?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }
    /// A key press and, for a printable key, the text it types, delivered as
    /// a keyboard delivers them.
    pub fn dispatch_key<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        key: &str,
        code: &str,
        target: Option<NodeHandle>,
    ) -> Result<bool, JsEngineError> {
        let input = KeyboardInput::key_down(key, code);
        let (pressed, _) = self.route_input(InputPayload::Key(input.to_canonical()))?;
        self.emit_keyboard_from_runtime(engine, &input, target)?;
        let printable =
            key.chars().count() == 1 && !key.chars().next().is_some_and(char::is_control);
        if printable {
            self.commit_routed_text(
                engine,
                CommittedText {
                    text: key.to_owned(),
                    key: Some(pressed.metadata.sequence),
                },
                "insertText",
            )?;
        }
        Ok(true)
    }
    pub fn focused(&self) -> Option<NodeHandle> {
        self.document.lock().expect("vue doc").focused()
    }
}

/// The DOM `inputType` of an edit a key made.
fn key_input_type(input: &KeyboardInput) -> &'static str {
    let command = input.modifiers.control || input.modifiers.meta;
    match (input.key.to_ascii_lowercase().as_str(), command) {
        ("backspace", _) => "deleteContentBackward",
        ("delete", _) => "deleteContentForward",
        ("enter", _) => "insertLineBreak",
        ("z", true) if input.modifiers.shift => "historyRedo",
        ("z", true) => "historyUndo",
        ("y", true) => "historyRedo",
        ("x", true) => "deleteByCut",
        ("v", true) => "insertFromPaste",
        _ => "insertText",
    }
}
