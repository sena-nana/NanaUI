//! Vue host input projection boundary.

use crate::*;

#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) file_drag_target: Option<NodeHandle>,
    /// Last IME field and leftover preedit for JS `compositionend` after Runtime
    /// has already dropped [`nana_ui_runtime::ImeComposition`].
    pub(crate) ime_target: Option<NodeHandle>,
    pub(crate) ime_preedit: String,
    /// Last focus/hover emitted to JS. Scene-host input updates Runtime first;
    /// these remember the previous JS view so blur/over events still fire.
    pub(crate) js_focus: Option<NodeHandle>,
    pub(crate) js_pointer_hover: BTreeMap<u64, Option<NodeHandle>>,
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
    /// Resolve the topmost node under `(x, y)` for native input routing.
    ///
    /// Scene paint boxes in [`LayoutBoxStore`] win when present so file drag
    /// and early-frame probes match painted geometry. Runtime hit-test is the
    /// fallback when no paint box covers the point.
    pub(crate) fn hit_test_client_point(&self, x: f32, y: f32) -> Option<NodeHandle> {
        let doc = self.document.lock().expect("vue doc");
        if !self.layout_boxes.snapshot().is_empty() {
            let mut stack = vec![doc.mount_root()];
            let mut preorder = Vec::new();
            while let Some(node) = stack.pop() {
                preorder.push(node);
                for child in doc.children_of(node).into_iter().rev() {
                    stack.push(child);
                }
            }
            for handle in preorder.into_iter().rev() {
                if self.layout_boxes.contains_point(handle, x, y) {
                    return Some(handle);
                }
            }
        }
        doc.hit_test(x, y)
    }
    /// Dispatch a native file hover/drop lifecycle through the same Vue event
    /// tree as pointer input. Dropped files are descriptors with an absolute
    /// path; reading their contents remains an application Host API decision.
    pub fn dispatch_file_drag<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        kind: FileDragEventKind,
        paths: &[PathBuf],
        position: Option<(f32, f32)>,
    ) -> Result<bool, JsEngineError> {
        let target_at_position = position.and_then(|(x, y)| self.hit_test_client_point(x, y));
        let mount_root = self.document.lock().expect("vue doc").mount_root();
        let target = target_at_position
            .or(self.input_projection.file_drag_target)
            .unwrap_or(mount_root);
        let detail = file_drag_detail(paths, position);
        let mut allowed = true;

        match kind {
            FileDragEventKind::Hover => {
                if self.input_projection.file_drag_target != Some(target) {
                    if let Some(previous) = self.input_projection.file_drag_target {
                        allowed &=
                            self.fire_dom_event(engine, previous, "dragleave", detail.clone())?;
                    }
                    allowed &= self.fire_dom_event(engine, target, "dragenter", detail.clone())?;
                    self.input_projection.file_drag_target = Some(target);
                }
                allowed &= self.fire_dom_event(engine, target, "dragover", detail)?;
            }
            FileDragEventKind::Drop => {
                allowed &= self.fire_dom_event(engine, target, "drop", detail)?;
                self.input_projection.file_drag_target = None;
            }
            FileDragEventKind::Cancel => {
                if let Some(previous) = self.input_projection.file_drag_target.take() {
                    allowed &= self.fire_dom_event(engine, previous, "dragleave", detail)?;
                }
            }
        }
        engine.run_microtasks()?;
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
    pub(crate) fn focus_target_at(
        &self,
        x: f32,
        y: f32,
    ) -> (Option<NodeHandle>, Option<NodeHandle>) {
        let mut doc = self.document.lock().expect("vue doc");
        let previous = doc.focused();
        let next = doc.hit_test(x, y).and_then(|hit| {
            let route = doc.event_route(hit)?;
            for id in std::iter::once(route.target).chain(route.bubble) {
                let node = NodeHandle::from(id);
                let tag = doc.element_tag(node).unwrap_or_default();
                if is_focusable_tag(&tag) || self.native_component_name(node.0).is_some() {
                    return Some(node);
                }
            }
            None
        });
        if previous != next {
            if let Some(next) = next {
                doc.set_focus(next);
            } else {
                doc.clear_focus();
            }
        }
        (previous, next)
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
    /// Dispatch one browser-style pointer event with hit-testing and capture.
    pub fn dispatch_pointer_result<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: PointerInput,
    ) -> Result<HostedInputResult, JsEngineError> {
        self.dispatch_pointer_result_with(engine, input, true)
    }
    /// Fire Vue/DOM pointer events after the Scene host already applied Runtime input.
    pub fn emit_pointer_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: PointerInput,
    ) -> Result<HostedInputResult, JsEngineError> {
        self.dispatch_pointer_result_with(engine, input, false)
    }
    pub(crate) fn dispatch_pointer_result_with<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: PointerInput,
        commit_runtime: bool,
    ) -> Result<HostedInputResult, JsEngineError> {
        let physical_hit = {
            let doc = self.document.lock().expect("vue doc");
            doc.hit_test(input.client_x, input.client_y)
        };
        let mut captured = self
            .document
            .lock()
            .expect("vue doc")
            .pointer_capture(input.pointer_id);
        if captured.is_some_and(|target| {
            self.document
                .lock()
                .expect("vue doc")
                .element_tag(target)
                .is_none()
        }) {
            if let Some(captured) = captured {
                self.document
                    .lock()
                    .expect("vue doc")
                    .release_pointer(input.pointer_id, captured);
            }
            self.flush_pointer_capture_events(engine)?;
            captured = None;
        }
        let target = captured.or_else(|| {
            if input.kind == PointerEventKind::Cancel {
                return self
                    .document
                    .lock()
                    .expect("vue doc")
                    .pointer_hover(input.pointer_id);
            }
            let doc = self.document.lock().expect("vue doc");
            doc.hit_event_target(input.client_x, input.client_y, input.kind.pointer_name())
                .or(physical_hit)
        });
        let fallback = self.document.lock().expect("vue doc").mount_root();
        let event_target = target.unwrap_or(fallback);
        let detail = self.pointer_detail(input, event_target);

        if matches!(
            input.kind,
            PointerEventKind::Move | PointerEventKind::Cancel
        ) && captured.is_none()
        {
            let previous = if commit_runtime {
                self.document
                    .lock()
                    .expect("vue doc")
                    .pointer_hover(input.pointer_id)
            } else {
                self.input_projection
                    .js_pointer_hover
                    .get(&input.pointer_id)
                    .copied()
                    .flatten()
            };
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
                if commit_runtime {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .set_pointer_hover(input.pointer_id, physical_hit);
                    self.flush_interactive_css_if_needed();
                }
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
                if commit_runtime && let Some(target) = target {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .press_pointer(input.pointer_id, target);
                    self.flush_interactive_css_if_needed();
                }
                let (previous, next) = if commit_runtime {
                    self.focus_target_at(input.client_x, input.client_y)
                } else {
                    let previous = self.input_projection.js_focus;
                    let next = self.document.lock().expect("vue doc").focused();
                    (previous, next)
                };
                if previous != next {
                    if let Some(previous) = previous {
                        self.fire_dom_event(engine, previous, "blur", BTreeMap::new())?;
                    }
                    if let Some(next) = next {
                        self.fire_dom_event(engine, next, "focus", BTreeMap::new())?;
                    }
                    if commit_runtime {
                        self.flush_focus_cascade(previous, next);
                    }
                }
                self.input_projection.js_focus = next;
            }
            PointerEventKind::Up => {
                let pressed = if commit_runtime {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .release_pointer_press(input.pointer_id)
                } else {
                    target.or(physical_hit)
                };
                if commit_runtime && pressed.is_some() {
                    self.flush_interactive_css_if_needed();
                }
                if !default_prevented
                    && let Some(click_target) = pressed
                    && physical_hit == Some(click_target)
                {
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
                        default_prevented |=
                            !self.fire_dom_event(engine, click_target, "click", detail.clone())?;
                        consumed = true;
                    }
                }
            }
            PointerEventKind::Cancel => {
                if commit_runtime {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .release_pointer_press(input.pointer_id);
                    self.flush_interactive_css_if_needed();
                }
            }
            PointerEventKind::Move => {}
        }

        self.flush_pointer_capture_events(engine)?;

        if matches!(input.kind, PointerEventKind::Up | PointerEventKind::Cancel) {
            if commit_runtime {
                let captured = self
                    .document
                    .lock()
                    .expect("vue doc")
                    .pointer_capture(input.pointer_id);
                if let Some(captured) = captured {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .release_pointer(input.pointer_id, captured);
                }
            }
            self.flush_pointer_capture_events(engine)?;
        }
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
    pub fn dispatch_pointer<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: PointerInput,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_pointer_result(engine, input)
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
    pub fn dispatch_wheel_result<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: WheelInput,
    ) -> Result<HostedInputResult, JsEngineError> {
        self.dispatch_wheel_result_with(engine, input, true)
    }
    pub fn emit_wheel_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: WheelInput,
    ) -> Result<HostedInputResult, JsEngineError> {
        self.dispatch_wheel_result_with(engine, input, false)
    }
    pub(crate) fn dispatch_wheel_result_with<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: WheelInput,
        commit_runtime: bool,
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
        let mut consumed = !allowed;
        if allowed && commit_runtime {
            let delta = crate::scroll::wheel_scroll_delta(&input);
            let scrolled = {
                let painted = self.layout_boxes.snapshot();
                let mut document = self.document.lock().expect("vue doc");
                // `pump_frame` / engine flush rewrites fixture chrome and
                // shrinks overflow content. Restore the host paint boxes
                // before committing ScrollOffset so chrome stays put.
                if !painted.is_empty() {
                    document.inject_layout_boxes(&painted);
                }
                let bridge = self.bridge.lock().expect("vue bridge");
                crate::scroll::apply_runtime_wheel_from(
                    &mut document,
                    &bridge,
                    &self.layout_boxes,
                    Some(target),
                    delta,
                )
                .is_some()
            };
            // Consume only after catalog qualification, when Scene owns the
            // offset/clip.
            consumed |= scrolled && {
                #[cfg(feature = "scene-view")]
                {
                    nana_ui::component_uses_runtime(nana_ui::component_ids::SIDEBAR_FRAME)
                }
                #[cfg(not(feature = "scene-view"))]
                {
                    true
                }
            };
        } else if allowed {
            consumed = true;
        }
        Ok(HostedInputResult {
            targeted: true,
            default_prevented: !allowed,
            consumed,
        })
    }
    pub fn dispatch_wheel<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: WheelInput,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_wheel_result(engine, input)
            .map(|result| result.targeted)
    }
    pub fn pointer_wheel<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_wheel(engine, WheelInput::pixels(x, y, delta_x, delta_y))
    }
    pub fn dispatch_keyboard<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &KeyboardInput,
        target: Option<NodeHandle>,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_keyboard_with(engine, input, target, true)
    }
    pub fn emit_keyboard_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &KeyboardInput,
        target: Option<NodeHandle>,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_keyboard_with(engine, input, target, false)
    }
    pub(crate) fn dispatch_keyboard_with<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &KeyboardInput,
        target: Option<NodeHandle>,
        commit_runtime: bool,
    ) -> Result<bool, JsEngineError> {
        let target = {
            let doc = self.document.lock().expect("vue doc");
            target
                .or_else(|| doc.focused())
                .unwrap_or_else(|| doc.mount_root())
        };
        let repeated = self
            .input
            .lock()
            .expect("input state")
            .note_key(&input.code, input.kind == KeyboardEventKind::Down);
        let mut detail = input.detail();
        if repeated {
            detail.insert("repeat".into(), HostValue::Bool(true));
        }
        let mut allowed = self.fire_dom_event(engine, target, input.kind.as_str(), detail)?;
        if allowed && input.kind == KeyboardEventKind::Down {
            let widget = self
                .bridge
                .lock()
                .expect("vue bridge")
                .get(target.0)
                .cloned();
            if let Some(widget) = widget {
                let key = input.key.to_ascii_lowercase();
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
                        "pageup" => Some(f64::from(widget.props.number + widget.props.step * 10.0)),
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
                    | WidgetKind::TableRow => {
                        !repeated && matches!(key.as_str(), "enter" | " " | "space" | "spacebar")
                    }
                    WidgetKind::Switch | WidgetKind::Checkbox | WidgetKind::Radio => {
                        !repeated && matches!(key.as_str(), " " | "space" | "spacebar")
                    }
                    WidgetKind::SettingsCollapsibleCard => {
                        !repeated && matches!(key.as_str(), "enter" | " " | "space" | "spacebar")
                    }
                    WidgetKind::Range => commit_runtime && requested_value.is_some(),
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
        if allowed && input.kind == KeyboardEventKind::Down && input.key.eq_ignore_ascii_case("tab")
        {
            let (previous, next) = if commit_runtime {
                self.advance_tab_focus(input.modifiers.shift)
            } else {
                let previous = self.input_projection.js_focus;
                let next = self.document.lock().expect("vue doc").focused();
                (previous, next)
            };
            if previous != next {
                if let Some(previous) = previous {
                    self.fire_dom_event(engine, previous, "blur", BTreeMap::new())?;
                }
                if let Some(next) = next {
                    self.fire_dom_event(engine, next, "focus", BTreeMap::new())?;
                }
            }
            self.input_projection.js_focus = next;
            if commit_runtime {
                self.flush_focus_cascade(previous, next);
            }
        } else if !commit_runtime {
            self.input_projection.js_focus = self.document.lock().expect("vue doc").focused();
        }
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(allowed)
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
            document.text_input_state(target).is_some()
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
    pub(crate) fn advance_tab_focus(
        &self,
        reverse: bool,
    ) -> (Option<NodeHandle>, Option<NodeHandle>) {
        let mut document = self.document.lock().expect("vue doc");
        let previous = document.focused();
        let root = document.mount_root();
        let mut order = document
            .collect_element_preorder(root)
            .into_iter()
            .map(NodeHandle)
            .filter_map(|node| {
                let tag = document.element_tag(node)?;
                if document.get_attribute(node, "disabled").is_some() {
                    return None;
                }
                let tabindex = document
                    .get_attribute(node, "tabindex")
                    .and_then(|value| value.parse::<i32>().ok());
                if tabindex.is_some_and(|value| value < 0) {
                    return None;
                }
                let naturally_focusable = is_focusable_tag(&tag)
                    || self.native_component_name(node.0).is_some()
                    || document
                        .get_attribute(node, "contenteditable")
                        .is_some_and(|value| value != "false");
                (naturally_focusable || tabindex.is_some()).then_some((tabindex.unwrap_or(0), node))
            })
            .collect::<Vec<_>>();
        order.sort_by_key(|(tabindex, _)| {
            if *tabindex > 0 {
                (0, *tabindex)
            } else {
                (1, 0)
            }
        });
        if order.is_empty() {
            document.clear_focus();
            return (previous, None);
        }
        let current = previous.and_then(|focused| {
            order
                .iter()
                .position(|(_, candidate)| *candidate == focused)
        });
        let next_index = if reverse {
            current.map_or(order.len() - 1, |index| {
                if index == 0 {
                    order.len() - 1
                } else {
                    index - 1
                }
            })
        } else {
            current.map_or(0, |index| (index + 1) % order.len())
        };
        let next = order.get(next_index).map(|(_, node)| *node);
        if let Some(next) = next {
            document.set_focus(next);
        }
        (previous, next)
    }
    /// Commit text from a keyboard or IME into the focused Vue control.
    pub fn commit_text<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        text: &str,
        input_type: &str,
    ) -> Result<bool, JsEngineError> {
        let Some(target) = self.document.lock().expect("vue doc").focused() else {
            return Ok(false);
        };
        self.commit_text_on(engine, target, text, input_type)
    }
    /// Commit text into a specific field. Used so leftover IME after blur
    /// cannot retarget the newly focused node.
    pub(crate) fn commit_text_on<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        text: &str,
        input_type: &str,
    ) -> Result<bool, JsEngineError> {
        if self.text_commit_blocked(target) {
            return Ok(false);
        }
        let (widget_editable, existing, fallback_value, tag, contenteditable) = {
            let widget_editable = self
                .bridge
                .lock()
                .expect("vue bridge")
                .get(target.0)
                .is_some_and(|widget| {
                    widget.kind.is_choice_field()
                        || matches!(
                            widget.kind,
                            WidgetKind::Input
                                | WidgetKind::NumberInput
                                | WidgetKind::Textarea
                                | WidgetKind::ContextMenu
                        )
                });
            let document = self.document.lock().expect("vue doc");
            (
                widget_editable,
                document.text_input_state(target),
                document.get_attribute(target, "value").unwrap_or_default(),
                document.element_tag(target),
                document.get_attribute(target, "contenteditable"),
            )
        };
        let editable = widget_editable
            || matches!(
                tag.as_deref(),
                Some(
                    "input"
                        | "textarea"
                        | "nana-context-menu"
                        | "search-dropdown"
                        | "nana-search"
                        | "nana-dropdown"
                )
            )
            || contenteditable.is_some_and(|value| value != "false");
        let Some(mut state) =
            existing.or_else(|| editable.then(|| TextInputState::new(fallback_value)))
        else {
            return Ok(false);
        };
        if !state.replace_selection(text) {
            return Ok(false);
        }
        let next = state;
        let mut detail = BTreeMap::new();
        detail.insert("data".into(), HostValue::string(text));
        detail.insert("inputType".into(), HostValue::string(input_type));
        detail.insert("value".into(), HostValue::string(&next.value));
        detail.insert("isComposing".into(), HostValue::Bool(false));
        if !self.fire_dom_event(engine, target, "beforeinput", detail.clone())? {
            return Ok(false);
        }
        {
            let mut document = self.document.lock().expect("vue doc");
            if !document.set_text_input_state(target, next.clone()) {
                return Ok(false);
            }
            document.set_attribute(target, "value", &next.value);
        }
        #[cfg(feature = "scene-view")]
        {
            let is_menu = self
                .bridge
                .lock()
                .expect("vue bridge")
                .get(target.0)
                .is_some_and(|widget| widget.kind == WidgetKind::ContextMenu);
            if is_menu {}
        }
        self.fire_dom_event(engine, target, "input", detail)?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
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
    pub(crate) fn remember_ime_target(&mut self, target: NodeHandle, preedit: String) {
        self.input_projection.ime_target = Some(target);
        self.input_projection.ime_preedit = preedit;
    }
    pub(crate) fn clear_ime_target(&mut self) {
        self.input_projection.ime_target = None;
        self.input_projection.ime_preedit.clear();
    }
    pub(crate) fn ime_composition_target(document: &NanaTreeDocument) -> Option<NodeHandle> {
        document
            .collect_element_preorder(document.html_root())
            .into_iter()
            .map(NodeHandle)
            .find(|&node| document.ime_composition(node).is_some())
    }
    /// Composition target and leftover preedit. Runtime drops `ImeComposition`
    /// on blur, so the remembered field wins over current focus.
    pub(crate) fn take_ime_leftover(&mut self) -> Option<(NodeHandle, String)> {
        let remembered = self.input_projection.ime_target.take();
        let remembered_text = std::mem::take(&mut self.input_projection.ime_preedit);
        let mut document = self.document.lock().expect("vue doc");
        let target = Self::ime_composition_target(&document).or(remembered)?;
        let data = document
            .ime_composition(target)
            .map(|ime| ime.text)
            .unwrap_or(remembered_text);
        document.set_ime_composition(target, None);
        Some((target, data))
    }
    pub fn dispatch_composition<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        input: &CompositionInput,
    ) -> Result<bool, JsEngineError> {
        let Some(target) = self.document.lock().expect("vue doc").focused() else {
            return Ok(false);
        };
        {
            let mut document = self.document.lock().expect("vue doc");
            if document.text_input_state(target).is_none() {
                let value = document.get_attribute(target, "value").unwrap_or_default();
                if !document.set_text_input_state(target, TextInputState::new(value)) {
                    return Err(JsEngineError::new(
                        "composition target has no retained text input state",
                    ));
                }
            }
            let composition = match input.kind {
                CompositionEventKind::Start | CompositionEventKind::Update => {
                    Some(nana_ui_runtime::ImeComposition {
                        text: input.data.clone(),
                        selection: None,
                    })
                }
                CompositionEventKind::End => None,
            };
            if !document.set_ime_composition(target, composition) {
                return Err(JsEngineError::new("invalid composition state"));
            }
        }
        match input.kind {
            CompositionEventKind::Start | CompositionEventKind::Update => {
                self.remember_ime_target(target, input.data.clone());
            }
            CompositionEventKind::End => {
                self.clear_ime_target();
            }
        }
        self.dispatch_composition_event(engine, target, input)
    }
    pub(crate) fn dispatch_composition_event<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        input: &CompositionInput,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_composition_event_with(engine, target, input, true)
    }
    pub(crate) fn dispatch_composition_event_with<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        target: NodeHandle,
        input: &CompositionInput,
        commit_runtime: bool,
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
            return if commit_runtime {
                self.commit_text_on(engine, target, &input.data, "insertCompositionText")
            } else {
                self.emit_text_events_from_runtime(
                    engine,
                    target,
                    &input.data,
                    "insertCompositionText",
                )
            };
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
                .map(|state| state.value)
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
    /// Forwards desktop winit IME lifecycle into Vue composition events.
    ///
    /// Preedit stays on Runtime [`nana_ui_runtime::ImeComposition`]. Commit and
    /// leftover Disabled preedit update Runtime [`TextInputState`] on the
    /// original composition field through [`Self::commit_text_on`], matching
    /// [`Self::dispatch_composition`] End even if focus has moved. This path
    /// does not write a second editor buffer.
    pub fn dispatch_native_ime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: &ImeEvent,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_native_ime_with(engine, event, true)
    }
    /// Emit JS composition/`input` after the Scene host already applied Runtime IME.
    pub fn emit_native_ime_from_runtime<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: &ImeEvent,
    ) -> Result<bool, JsEngineError> {
        self.dispatch_native_ime_with(engine, event, false)
    }
    pub(crate) fn dispatch_native_ime_with<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: &ImeEvent,
        commit_runtime: bool,
    ) -> Result<bool, JsEngineError> {
        match event {
            ImeEvent::Enabled => Ok(self.focused().is_some()),
            ImeEvent::Preedit { text, selection } => {
                let Some(target) = self.focused() else {
                    return Ok(false);
                };
                let started = if commit_runtime {
                    let mut document = self.document.lock().expect("vue doc");
                    if document.text_input_state(target).is_none() {
                        let value = document.get_attribute(target, "value").unwrap_or_default();
                        if !document.set_text_input_state(target, TextInputState::new(value)) {
                            return Err(JsEngineError::new(
                                "native IME target has no retained text input state",
                            ));
                        }
                    }
                    let started = document.ime_composition(target).is_none();
                    if !document.set_ime_composition(
                        target,
                        Some(nana_ui_runtime::ImeComposition {
                            text: text.clone(),
                            selection: *selection,
                        }),
                    ) {
                        return Err(JsEngineError::new("invalid native IME preedit state"));
                    }
                    started
                } else {
                    self.input_projection.ime_target.is_none()
                };
                self.remember_ime_target(target, text.clone());
                if started {
                    self.dispatch_composition_event_with(
                        engine,
                        target,
                        &CompositionInput::new(CompositionEventKind::Start, ""),
                        commit_runtime,
                    )?;
                }
                self.dispatch_composition_event_with(
                    engine,
                    target,
                    &CompositionInput::new(CompositionEventKind::Update, text),
                    commit_runtime,
                )
            }
            ImeEvent::Commit(text) => {
                let Some(target) = self.input_projection.ime_target.or_else(|| self.focused())
                else {
                    return Ok(false);
                };
                self.clear_ime_target();
                if commit_runtime {
                    self.document
                        .lock()
                        .expect("vue doc")
                        .set_ime_composition(target, None);
                }
                self.dispatch_composition_event_with(
                    engine,
                    target,
                    &CompositionInput::new(CompositionEventKind::End, text),
                    commit_runtime,
                )
            }
            ImeEvent::DeleteSurrounding {
                before_bytes,
                after_bytes,
            } => self.dispatch_native_delete_surrounding(
                engine,
                *before_bytes,
                *after_bytes,
                commit_runtime,
            ),
            ImeEvent::Disabled => {
                if commit_runtime {
                    let leftover = self.take_ime_leftover();
                    let Some((target, data)) = leftover else {
                        return Ok(self.focused().is_some());
                    };
                    if self.text_commit_blocked(target) {
                        return Ok(true);
                    }
                    return self.dispatch_composition_event_with(
                        engine,
                        target,
                        &CompositionInput::new(CompositionEventKind::End, data),
                        true,
                    );
                }
                let leftover = self.input_projection.ime_target.take().map(|target| {
                    let data = std::mem::take(&mut self.input_projection.ime_preedit);
                    (target, data)
                });
                let Some((target, data)) = leftover else {
                    return Ok(self.focused().is_some());
                };
                if data.is_empty() {
                    return Ok(true);
                }
                self.dispatch_composition_event_with(
                    engine,
                    target,
                    &CompositionInput::new(CompositionEventKind::End, data),
                    false,
                )
            }
        }
    }
    pub(crate) fn dispatch_native_delete_surrounding<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        before_bytes: usize,
        after_bytes: usize,
        commit_runtime: bool,
    ) -> Result<bool, JsEngineError> {
        let Some(target) = self.focused() else {
            return Ok(false);
        };
        if self.text_commit_blocked(target) {
            return Ok(true);
        }
        let next = {
            let document = self.document.lock().expect("vue doc");
            let Some(mut state) = document.text_input_state(target) else {
                return Ok(false);
            };
            if commit_runtime && !state.delete_surrounding(before_bytes, after_bytes) {
                return Ok(false);
            }
            state
        };
        let mut detail = BTreeMap::new();
        detail.insert("data".into(), HostValue::string(""));
        detail.insert("inputType".into(), HostValue::string("deleteContent"));
        detail.insert("value".into(), HostValue::string(&next.value));
        detail.insert("isComposing".into(), HostValue::Bool(false));
        if commit_runtime && !self.fire_dom_event(engine, target, "beforeinput", detail.clone())? {
            return Ok(false);
        }
        if commit_runtime {
            let mut document = self.document.lock().expect("vue doc");
            if !document.set_text_input_state(target, next.clone()) {
                return Err(JsEngineError::new(
                    "native IME delete surrounding could not write text input state",
                ));
            }
            document.set_attribute(target, "value", &next.value);
        }
        self.fire_dom_event(engine, target, "input", detail)?;
        engine.run_microtasks()?;
        let _ = self.pump_frame(engine)?;
        Ok(true)
    }
    /// Legacy keydown helper; printable text is committed separately for compatibility.
    pub fn dispatch_key<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        key: &str,
        code: &str,
        target: Option<NodeHandle>,
    ) -> Result<bool, JsEngineError> {
        let input = KeyboardInput::key_down(key, code);
        self.dispatch_keyboard(engine, &input, target)?;
        if target.or_else(|| self.focused()).is_some()
            && key.chars().count() == 1
            && !key
                .chars()
                .next()
                .is_some_and(|character| character.is_control())
        {
            self.commit_text(engine, key, "insertText")?;
        }
        Ok(true)
    }
    pub fn focused(&self) -> Option<NodeHandle> {
        self.document.lock().expect("vue doc").focused()
    }
    /// Platform IME request for a focused Runtime Input/Textarea.
    ///
    /// When this is enabled, the hosted window must not also feed winit IME
    /// into a second editor.
    pub fn text_input_request(&self) -> Option<nana_ui_platform::TextInputRequest> {
        let target = self.focused()?;
        let document = self.document.lock().ok()?;
        let _ = document.text_input_state(target)?;
        let widget = self.bridge.lock().ok()?.get(target.0).cloned();
        let (disabled, read_only, secure) = widget
            .as_ref()
            .map(|widget| {
                (
                    widget.props.disabled,
                    widget.props.read_only,
                    widget.props.secure,
                )
            })
            .unwrap_or((false, false, false));
        if disabled || read_only {
            return Some(nana_ui_platform::TextInputRequest {
                enabled: false,
                cursor_area: None,
                purpose: nana_ui_platform::TextInputPurpose::Normal,
            });
        }
        let cursor_area =
            crate::get_layout_box_from(&self.layout_boxes, &document, target).map(|layout| {
                nana_ui_core::LogicalRect::new(layout.x, layout.y, layout.width, layout.height)
            });
        Some(nana_ui_platform::TextInputRequest {
            enabled: true,
            cursor_area,
            purpose: if secure {
                nana_ui_platform::TextInputPurpose::Password
            } else {
                nana_ui_platform::TextInputPurpose::Normal
            },
        })
    }
}
