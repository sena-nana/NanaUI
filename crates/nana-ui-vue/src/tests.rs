use super::*;

#[derive(Default)]
struct RecordingEngine {
    invocations: Vec<(JsFunctionId, Vec<HostValue>)>,
    prevent_event: Option<String>,
}

impl JsEngine for RecordingEngine {
    fn initialize(&mut self, _artifact: RuntimeArtifact) -> Result<(), JsEngineError> {
        Ok(())
    }

    fn register_host_api(&mut self, _api: &HostApiRegistry) -> Result<(), JsEngineError> {
        Ok(())
    }

    fn resolve_function(&mut self, name: &str) -> Result<JsFunctionId, JsEngineError> {
        Ok(JsFunctionId(match name {
            "__nanaFireEvent" | "__nanaFireWindowEvent" => 1,
            "__nanaMotionComplete" => 10,
            "__nanaMotionCancel" => 16,
            "__nanaNotifyLayout" => 11,
            "__nanaDrainTimers" => 12,
            "__nanaDrainFetch" => 13,
            "__nanaApplyTheme" | "__nanaApplyWindowTheme" => 14,
            "__nanaPumpLifecycle" | "__nanaPumpWindowLifecycle" => 15,
            _ => 1,
        }))
    }

    fn invoke(
        &mut self,
        target: JsFunctionId,
        args: &[HostValue],
    ) -> Result<HostValue, JsEngineError> {
        self.invocations.push((target, args.to_vec()));
        let allowed = args
            .get(1)
            .and_then(HostValue::as_str)
            .is_none_or(|name| self.prevent_event.as_deref() != Some(name));
        Ok(HostValue::Bool(allowed))
    }

    fn run_microtasks(&mut self) -> Result<(), JsEngineError> {
        Ok(())
    }

    fn interrupt(&mut self) {}
    fn request_gc(&mut self) {}
    fn shutdown(&mut self) {}
}

fn fired_events(engine: &RecordingEngine) -> Vec<(u64, String, BTreeMap<String, HostValue>)> {
    engine
        .invocations
        .iter()
        .filter_map(|(_, args)| {
            let target = args.first()?.as_f64()? as u64;
            let name = args.get(1)?.as_str()?.to_string();
            let detail = args.get(2)?.as_object()?.clone();
            Some((target, name, detail))
        })
        .collect()
}

fn install_input_nodes(host: &mut VueHost) -> (NodeHandle, NodeHandle) {
    let document = host.document();
    let mut doc = document.lock().expect("document");
    let root = doc.mount_root();
    let first = doc.create_element("input");
    let second = doc.create_element("button");
    doc.insert(first, root, None);
    doc.insert(second, root, None);
    drop(doc);

    let store = host.layout_box_store();
    store.begin_frame();
    store.record(first, 0.0, 0.0, 80.0, 40.0);
    store.record(second, 100.0, 0.0, 80.0, 40.0);
    host.sync_scene_layout_boxes();
    (first, second)
}

fn install_focused_native_input(
    host: &mut VueHost,
    value: &str,
) -> (NodeHandle, nana_ui_runtime::DocumentId) {
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, _) = install_input_nodes(host);
    host.bridge().lock().expect("bridge").register(
        input.0,
        WidgetKind::Input,
        WidgetProps {
            value: value.into(),
            ..WidgetProps::default()
        },
    );
    let document_id = {
        let snapshot = host.bridge().lock().expect("bridge").snapshot();
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.sync_semantic_styles(&snapshot);
        doc.set_attribute(input, "value", value);
        doc.set_focus(input);
        doc.runtime_document().document()
    };
    (input, document_id)
}

fn install_textarea_node(host: &mut VueHost, value: &str) -> NodeHandle {
    let document = host.document();
    let mut doc = document.lock().expect("document");
    let root = doc.mount_root();
    let area = doc.create_element("textarea");
    doc.set_attribute(area, "value", value);
    assert!(doc.set_text_input_state(area, TextInputState::new(value)));
    doc.insert(area, root, None);
    doc.set_focus(area);
    drop(doc);

    let store = host.layout_box_store();
    store.begin_frame();
    store.record(area, 0.0, 0.0, 160.0, 80.0);
    host.sync_scene_layout_boxes();
    area
}

fn install_semantic_switch(host: &mut VueHost) -> NodeHandle {
    let node = {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        let node = doc.create_element("nana-switch");
        let root = doc.mount_root();
        doc.insert(node, root, None);
        node
    };
    host.bridge.lock().expect("bridge").register(
        node.0,
        WidgetKind::Switch,
        WidgetProps {
            label: "Preview".into(),
            toggled: false,
            ..Default::default()
        },
    );
    let snapshot = host.bridge.lock().expect("bridge").snapshot();
    let document = host.document();
    let mut doc = document.lock().expect("document");
    doc.sync_semantic_styles(&snapshot);
    doc.apply_layout_boxes(&[(
        node,
        LayoutBox {
            handle: node,
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 32.0,
        },
    )]);
    node
}

#[test]
fn semantic_switch_pointer_default_action_updates_once_and_honors_prevent_default() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let node = install_semantic_switch(&mut host);
    let mut engine = RecordingEngine::default();

    host.pointer_click(&mut engine, 10.0, 10.0).unwrap();
    let events = fired_events(&engine);
    for name in ["click", "change", "update:modelValue"] {
        assert_eq!(
            events.iter().filter(|(_, event, _)| event == name).count(),
            1,
            "{name} must be emitted once"
        );
    }
    assert!(
        host.bridge
            .lock()
            .expect("bridge")
            .get(node.0)
            .unwrap()
            .props
            .toggled
    );

    let mut prevented = RecordingEngine {
        prevent_event: Some("click".into()),
        ..Default::default()
    };
    host.pointer_click(&mut prevented, 10.0, 10.0).unwrap();
    assert!(
        host.bridge
            .lock()
            .expect("bridge")
            .get(node.0)
            .unwrap()
            .props
            .toggled,
        "prevented click must not apply the toggle default action"
    );
    let prevented_events = fired_events(&prevented);
    assert_eq!(
        prevented_events
            .iter()
            .filter(|(_, event, _)| event == "click")
            .count(),
        1
    );
    assert!(
        prevented_events
            .iter()
            .all(|(_, event, _)| event != "change" && event != "update:modelValue")
    );
}

#[test]
fn range_keyboard_and_accessibility_share_quantized_change_action() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let node = {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        let node = doc.create_element("input");
        let root = doc.mount_root();
        doc.insert(node, root, None);
        node
    };
    host.bridge.lock().expect("bridge").register(
        node.0,
        WidgetKind::Range,
        WidgetProps {
            min: 0.0,
            max: 1.0,
            step: 0.25,
            number: 0.5,
            ..Default::default()
        },
    );
    let snapshot = host.bridge.lock().expect("bridge").snapshot();
    host.document()
        .lock()
        .expect("document")
        .sync_semantic_styles(&snapshot);
    let mut engine = RecordingEngine::default();

    assert!(
        !host
            .dispatch_keyboard(
                &mut engine,
                &KeyboardInput::key_down("ArrowRight", "ArrowRight"),
                Some(node),
            )
            .unwrap()
    );
    assert_eq!(
        host.bridge
            .lock()
            .expect("bridge")
            .get(node.0)
            .unwrap()
            .props
            .number,
        0.75
    );
    assert!(
        host.accessibility_set_value(&mut engine, node, "0.88")
            .unwrap()
    );
    assert_eq!(
        host.bridge
            .lock()
            .expect("bridge")
            .get(node.0)
            .unwrap()
            .props
            .number,
        1.0
    );
    let events = fired_events(&engine);
    assert_eq!(
        events
            .iter()
            .filter(|(_, event, _)| event == "change")
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|(_, event, _)| event == "update:modelValue")
            .count(),
        2
    );
}

fn install_sidebar_frame(host: &mut VueHost) -> (NodeHandle, NodeHandle, NodeHandle, NodeHandle) {
    let (frame, top, body, footer, content) = {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        let root = doc.mount_root();
        let frame = doc.create_element("nana-sidebar-frame");
        let top = doc.create_element("nana-column");
        let body = doc.create_element("nana-column");
        let footer = doc.create_element("nana-column");
        let content = doc.create_element("nana-sidebar-row");
        doc.set_attribute(body, "class", "nana-sidebar-frame__body");
        doc.set_attribute(body, "data-slot", "sidebar-body");
        doc.insert(frame, root, None);
        doc.insert(top, frame, None);
        doc.insert(body, frame, None);
        doc.insert(footer, frame, None);
        doc.insert(content, body, None);
        (frame, top, body, footer, content)
    };

    {
        let mut bridge = host.bridge.lock().expect("bridge");
        let mut frame_props = WidgetProps::default();
        frame_props.class_names = vec!["nana-sidebar-frame".into()];
        frame_props
            .layout
            .apply_class_layout_hints(&frame_props.class_names);
        bridge.register(frame.0, WidgetKind::SidebarFrame, frame_props);

        let mut top_props = WidgetProps::default();
        top_props.class_names = vec!["nana-sidebar-frame__top".into()];
        top_props
            .attrs
            .insert("data-slot".into(), "sidebar-top".into());
        top_props
            .layout
            .apply_class_layout_hints(&top_props.class_names);
        bridge.register(top.0, WidgetKind::Column, top_props);

        let mut body_props = WidgetProps::default();
        body_props.class_names = vec!["nana-sidebar-frame__body".into()];
        body_props
            .attrs
            .insert("data-slot".into(), "sidebar-body".into());
        body_props
            .layout
            .apply_class_layout_hints(&body_props.class_names);
        bridge.register(body.0, WidgetKind::Column, body_props);

        let mut footer_props = WidgetProps::default();
        footer_props.class_names = vec!["nana-sidebar-frame__footer".into()];
        footer_props
            .attrs
            .insert("data-slot".into(), "sidebar-footer".into());
        footer_props
            .layout
            .apply_class_layout_hints(&footer_props.class_names);
        bridge.register(footer.0, WidgetKind::Column, footer_props);

        let mut content_props = WidgetProps::default();
        content_props.label = "工作区".into();
        bridge.register(content.0, WidgetKind::SidebarRow, content_props);

        bridge.insert_child(top.0, frame.0, None);
        bridge.insert_child(body.0, frame.0, None);
        bridge.insert_child(footer.0, frame.0, None);
        bridge.insert_child(content.0, body.0, None);
    }

    let snapshot = host.bridge.lock().expect("bridge").snapshot();
    let store = host.layout_box_store();
    store.begin_frame();
    store.record(frame, 0.0, 0.0, 220.0, 320.0);
    store.record(top, 0.0, 0.0, 220.0, 40.0);
    store.record(body, 0.0, 40.0, 220.0, 200.0);
    store.record(content, 0.0, 40.0, 220.0, 400.0);
    store.record(footer, 0.0, 250.0, 220.0, 40.0);
    {
        let mut doc = host.document.lock().expect("document");
        doc.sync_semantic_styles(&snapshot);
        doc.apply_layout_boxes(&store.snapshot());
    }
    (frame, top, body, footer)
}

#[test]
fn sidebar_frame_wheel_updates_runtime_body_without_moving_chrome() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (_frame, top, body, footer) = install_sidebar_frame(&mut host);
    let mut engine = RecordingEngine::default();

    let top_before = host
        .document
        .lock()
        .expect("document")
        .layout_box(top)
        .expect("top box");
    let footer_before = host
        .document
        .lock()
        .expect("document")
        .layout_box(footer)
        .expect("footer box");
    assert_eq!(
        host.document
            .lock()
            .expect("document")
            .scroll_offset(body)
            .y,
        0.0
    );
    assert_eq!(
        host.document.lock().expect("document").scroll_offset(top).y,
        0.0
    );
    assert_eq!(
        host.document
            .lock()
            .expect("document")
            .scroll_offset(footer)
            .y,
        0.0
    );

    let result = host
        .dispatch_wheel_result(&mut engine, WheelInput::pixels(20.0, 80.0, 0.0, -48.0))
        .expect("wheel");
    assert!(result.targeted);
    assert!(!result.default_prevented);
    assert_eq!(
        result.consumed,
        {
            #[cfg(feature = "scene-view")]
            {
                nana_ui::component_uses_runtime(nana_ui::component_ids::SIDEBAR_FRAME)
            }
            #[cfg(not(feature = "scene-view"))]
            {
                true
            }
        },
        "consume hosted wheel only when Scene owns SidebarFrame paint"
    );

    let document = host.document.lock().expect("document");
    assert!(
        document.scroll_offset(body).y > 0.0,
        "body Runtime scroll_offset must move"
    );
    assert_eq!(document.scroll_offset(top).y, 0.0);
    assert_eq!(document.scroll_offset(footer).y, 0.0);
    assert_eq!(document.layout_box(top).expect("top after"), top_before);
    assert_eq!(
        document.layout_box(footer).expect("footer after"),
        footer_before
    );
    drop(document);
    assert!(
        crate::scroll::shared_scroll_offset_store()
            .take_pending()
            .is_empty(),
        "sidebar body must not depend on pending scroll tasks"
    );
}

#[test]
fn sidebar_frame_wheel_prevent_default_does_not_scroll_runtime() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (_frame, top, body, footer) = install_sidebar_frame(&mut host);
    let mut engine = RecordingEngine {
        prevent_event: Some("wheel".into()),
        ..Default::default()
    };

    let result = host
        .dispatch_wheel_result(&mut engine, WheelInput::pixels(20.0, 80.0, 0.0, -48.0))
        .expect("wheel");
    assert!(result.targeted);
    assert!(result.default_prevented);
    assert!(result.consumed);

    let document = host.document.lock().expect("document");
    assert_eq!(document.scroll_offset(body).y, 0.0);
    assert_eq!(document.scroll_offset(top).y, 0.0);
    assert_eq!(document.scroll_offset(footer).y, 0.0);
}

#[test]
fn scene_scroll_event_updates_runtime_without_firing_vue_event() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let document = host.document();
    let scrollport = {
        let mut doc = document.lock().expect("document");
        let root = doc.mount_root();
        let scrollport = doc.create_element("div");
        doc.insert(scrollport, root, None);
        scrollport
    };
    let mut engine = RecordingEngine::default();

    assert!(
        host.dispatch_bridge_event(
            &mut engine,
            BridgeEvent::Scroll {
                id: scrollport.0,
                offset: ScrollOffset { x: 0.0, y: 48.0 },
                metrics: nana_ui_runtime::ScrollMetrics {
                    viewport_width: 100.0,
                    viewport_height: 100.0,
                    content_width: 100.0,
                    content_height: 300.0,
                },
            },
        )
        .expect("dispatch scroll")
    );
    assert_eq!(
        document.lock().expect("document").scroll_offset(scrollport),
        ScrollOffset { x: 0.0, y: 48.0 }
    );
    assert!(engine.invocations.is_empty());

    assert!(
        !host
            .dispatch_bridge_event(
                &mut engine,
                BridgeEvent::Scroll {
                    id: scrollport.0,
                    offset: ScrollOffset { x: 0.0, y: 48.0 },
                    metrics: nana_ui_runtime::ScrollMetrics {
                        viewport_width: 100.0,
                        viewport_height: 100.0,
                        content_width: 100.0,
                        content_height: 300.0,
                    },
                },
            )
            .expect("repeat scroll")
    );
}

#[test]
fn vue_hosts_isolate_paint_geometry_for_equal_node_handles() {
    let first = VueHost::new();
    let second = VueHost::new();
    let node = NodeHandle(2);
    first
        .layout_box_store()
        .record(node, 10.0, 20.0, 30.0, 40.0);
    second
        .layout_box_store()
        .record(node, 100.0, 200.0, 300.0, 400.0);

    assert_eq!(first.layout_box_store().get(node).unwrap().x, 10.0);
    assert_eq!(second.layout_box_store().get(node).unwrap().x, 100.0);

    let layout_x = |host: &VueHost| {
        let api = host.host_api_registry();
        match api
            .call("layoutBox", &[HostValue::Number(node.0 as f64)])
            .expect("layoutBox")
        {
            HostValue::Object(map) => map.get("x").and_then(HostValue::as_f64).unwrap(),
            other => panic!("expected object, got {other:?}"),
        }
    };
    assert_eq!(layout_x(&first), 10.0);
    assert_eq!(layout_x(&second), 100.0);
}

#[test]
fn pointer_capture_keeps_target_and_blur_releases_it() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    host.callbacks.lifecycle_pump = Some(JsFunctionId(2));
    let (first, second) = install_input_nodes(&mut host);
    let mut engine = RecordingEngine::default();

    let mut down = PointerInput::mouse(PointerEventKind::Down, 10.0, 12.0);
    down.pointer_id = 7;
    down.screen_x = 210.0;
    down.screen_y = 312.0;
    down.buttons = 1;
    down.pressure = 0.5;
    down.modifiers.shift = true;
    assert!(
        host.dispatch_pointer(&mut engine, down)
            .expect("pointer down")
    );

    let api = host.host_api_registry();
    api.call(
        "setPointerCapture",
        &[HostValue::Number(first.0 as f64), HostValue::Number(7.0)],
    )
    .expect("capture pointer");
    assert_eq!(
        api.call(
            "hasPointerCapture",
            &[HostValue::Number(first.0 as f64), HostValue::Number(7.0),],
        )
        .expect("query capture"),
        HostValue::Bool(true)
    );

    let mut movement = PointerInput::mouse(PointerEventKind::Move, 120.0, 12.0);
    movement.pointer_id = 7;
    movement.buttons = 1;
    movement.pressure = 0.25;
    movement.tangential_pressure = -0.2;
    movement.tilt_x = 25;
    movement.tilt_y = -12;
    movement.twist = 180;
    movement.modifiers.alt = true;
    assert!(
        host.dispatch_pointer(&mut engine, movement)
            .expect("captured move")
    );

    let events = fired_events(&engine);
    let captured_move = events
        .iter()
        .find(|(_, name, detail)| {
            name == "pointermove"
                && detail.get("pointerId").and_then(HostValue::as_f64) == Some(7.0)
        })
        .expect("pointermove event");
    assert_eq!(captured_move.0, first.0);
    assert_ne!(captured_move.0, second.0);
    assert_eq!(
        captured_move.2.get("clientX").and_then(HostValue::as_f64),
        Some(120.0)
    );
    assert_eq!(
        captured_move.2.get("altKey").and_then(HostValue::as_bool),
        Some(true)
    );
    let tangential_pressure = captured_move
        .2
        .get("tangentialPressure")
        .and_then(HostValue::as_f64)
        .expect("tangential pressure");
    assert!((tangential_pressure + 0.2).abs() < 1e-6);
    assert_eq!(
        captured_move.2.get("tiltX").and_then(HostValue::as_f64),
        Some(25.0)
    );
    assert_eq!(
        captured_move.2.get("tiltY").and_then(HostValue::as_f64),
        Some(-12.0)
    );
    assert_eq!(
        captured_move.2.get("twist").and_then(HostValue::as_f64),
        Some(180.0)
    );
    assert!(events.iter().any(|(target, name, detail)| {
        *target == first.0
            && name == "gotpointercapture"
            && detail.get("pointerId").and_then(HostValue::as_f64) == Some(7.0)
    }));

    host.pump_lifecycle(&mut engine, WindowLifecycleEvent::Blur)
        .expect("window blur");
    assert_eq!(
        api.call(
            "hasPointerCapture",
            &[HostValue::Number(first.0 as f64), HostValue::Number(7.0),],
        )
        .expect("capture released"),
        HostValue::Bool(false)
    );
    assert!(fired_events(&engine).iter().any(|(target, name, detail)| {
        *target == first.0
            && name == "lostpointercapture"
            && detail.get("pointerId").and_then(HostValue::as_f64) == Some(7.0)
    }));
}

#[test]
fn file_drag_tracks_hit_target_and_exposes_file_descriptors() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (first, second) = install_input_nodes(&mut host);
    let mut engine = RecordingEngine::default();
    let paths = vec![
        PathBuf::from("C:/drop/avatar.png"),
        PathBuf::from("C:/drop/background.jpg"),
    ];

    host.dispatch_file_drag(
        &mut engine,
        FileDragEventKind::Hover,
        &paths,
        Some((10.0, 12.0)),
    )
    .expect("hover first target");
    host.dispatch_file_drag(
        &mut engine,
        FileDragEventKind::Hover,
        &paths,
        Some((120.0, 12.0)),
    )
    .expect("hover second target");
    host.dispatch_file_drag(
        &mut engine,
        FileDragEventKind::Drop,
        &paths,
        Some((120.0, 12.0)),
    )
    .expect("drop second target");

    let events = fired_events(&engine);
    assert_eq!(
        events
            .iter()
            .map(|(target, name, _)| (*target, name.as_str()))
            .collect::<Vec<_>>(),
        [
            (first.0, "dragenter"),
            (first.0, "dragover"),
            (first.0, "dragleave"),
            (second.0, "dragenter"),
            (second.0, "dragover"),
            (second.0, "drop"),
        ]
    );
    let files = events
        .last()
        .and_then(|(_, _, detail)| detail.get("files"))
        .and_then(HostValue::as_array)
        .expect("drop files");
    assert_eq!(files.len(), 2);
    let file = files[0].as_object().expect("file descriptor");
    assert_eq!(
        file.get("name").and_then(HostValue::as_str),
        Some("avatar.png")
    );
    assert_eq!(
        file.get("path").and_then(HostValue::as_str),
        Some("C:/drop/avatar.png")
    );
    assert_eq!(
        files[1]
            .as_object()
            .and_then(|file| file.get("name"))
            .and_then(HostValue::as_str),
        Some("background.jpg")
    );
}

#[test]
fn composition_end_commits_through_beforeinput_and_input() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, _) = install_input_nodes(&mut host);
    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.set_attribute(input, "value", "Nana");
        doc.set_focus(input);
    }
    let mut engine = RecordingEngine::default();

    host.dispatch_composition(
        &mut engine,
        &CompositionInput::new(CompositionEventKind::Start, ""),
    )
    .expect("composition start");
    host.dispatch_composition(
        &mut engine,
        &CompositionInput::new(CompositionEventKind::Update, "界"),
    )
    .expect("composition update");
    assert_eq!(
        host.document()
            .lock()
            .expect("document")
            .ime_composition(input)
            .expect("runtime composition")
            .text,
        "界"
    );
    host.dispatch_composition(
        &mut engine,
        &CompositionInput::new(CompositionEventKind::End, "界"),
    )
    .expect("composition end");
    assert!(
        host.document()
            .lock()
            .expect("document")
            .ime_composition(input)
            .is_none()
    );

    let events = fired_events(&engine);
    let names = events
        .iter()
        .map(|(_, name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "compositionstart",
            "compositionupdate",
            "compositionend",
            "beforeinput",
            "input"
        ]
    );
    let input_event = events.last().expect("input event");
    assert_eq!(input_event.0, input.0);
    assert_eq!(
        input_event.2.get("value").and_then(HostValue::as_str),
        Some("Nana界")
    );
    assert_eq!(
        input_event.2.get("inputType").and_then(HostValue::as_str),
        Some("insertCompositionText")
    );
    assert_eq!(
        host.document()
            .lock()
            .expect("document")
            .get_attribute(input, "value")
            .as_deref(),
        Some("Nana界")
    );
}

#[test]
fn committed_text_replaces_runtime_owned_unicode_selection() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, _) = install_input_nodes(&mut host);
    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.set_attribute(input, "value", "你好ab");
        doc.set_focus(input);
        assert!(doc.set_text_input_state(
            input,
            TextInputState {
                value: "你好ab".into(),
                selection: nana_ui_runtime::TextSelection {
                    anchor: 0,
                    focus: "你".len(),
                },
                additional_selections: Vec::new(),
            }
        ));
    }
    let mut engine = RecordingEngine::default();

    assert!(host.commit_text(&mut engine, "娜", "insertText").unwrap());
    let document = host.document();
    let doc = document.lock().expect("document");
    let state = doc.text_input_state(input).expect("text input state");
    assert_eq!(state.value, "娜好ab");
    assert_eq!(
        state.selection,
        nana_ui_runtime::TextSelection::caret("娜".len())
    );
    assert_eq!(doc.get_attribute(input, "value").as_deref(), Some("娜好ab"));
}

#[test]
fn native_ime_commit_updates_runtime_value_and_emits_input() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, _) = install_input_nodes(&mut host);
    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.set_attribute(input, "value", "Nana");
        doc.set_focus(input);
    }
    let mut engine = RecordingEngine::default();

    host.dispatch_native_ime(
        &mut engine,
        &ImeEvent::Preedit {
            text: "世".into(),
            selection: Some((3, 3)),
        },
    )
    .expect("preedit");
    {
        let document = host.document();
        let document = document.lock().expect("document");
        let composition = document
            .ime_composition(input)
            .expect("runtime preedit stays on ImeComposition");
        assert_eq!(composition.text, "世");
        assert_eq!(composition.selection, Some((3, 3)));
        assert_eq!(
            document.get_attribute(input, "value").as_deref(),
            Some("Nana"),
            "preedit must not mutate committed Runtime value"
        );
    }
    host.dispatch_native_ime(&mut engine, &ImeEvent::Commit("世界".into()))
        .expect("commit lifecycle");
    let document = host.document();
    let document = document.lock().expect("document");
    assert!(document.ime_composition(input).is_none());
    let state = document
        .text_input_state(input)
        .expect("runtime text input state");
    assert_eq!(state.value, "Nana世界");
    assert_eq!(
        state.selection,
        nana_ui_runtime::TextSelection::caret("Nana世界".len())
    );
    assert_eq!(
        document.get_attribute(input, "value").as_deref(),
        Some("Nana世界")
    );
    drop(document);

    let events = fired_events(&engine);
    assert_eq!(
        events
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "compositionstart",
            "compositionupdate",
            "compositionend",
            "beforeinput",
            "input"
        ]
    );
    let input_event = events.last().expect("input event");
    assert_eq!(input_event.0, input.0);
    assert_eq!(
        input_event.2.get("value").and_then(HostValue::as_str),
        Some("Nana世界")
    );
    assert_eq!(
        input_event.2.get("inputType").and_then(HostValue::as_str),
        Some("insertCompositionText")
    );
    assert_eq!(
        events.iter().filter(|(_, name, _)| name == "input").count(),
        1,
        "native IME commit must not double-insert"
    );
}

#[test]
fn native_ime_delete_surrounding_updates_runtime_value_and_skips_invalid_spans() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, _) = install_input_nodes(&mut host);
    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.set_attribute(input, "value", "你好");
        doc.set_focus(input);
        assert!(doc.set_text_input_state(input, TextInputState::new("你好")));
    }
    let mut engine = RecordingEngine::default();

    assert!(
        host.dispatch_native_ime(
            &mut engine,
            &ImeEvent::DeleteSurrounding {
                before_bytes: "好".len(),
                after_bytes: 0,
            },
        )
        .unwrap()
    );
    {
        let document = host.document();
        let document = document.lock().expect("document");
        let state = document
            .text_input_state(input)
            .expect("runtime text input state");
        assert_eq!(state.value, "你");
        assert_eq!(
            state.selection,
            nana_ui_runtime::TextSelection::caret("你".len())
        );
        assert_eq!(
            document.get_attribute(input, "value").as_deref(),
            Some("你")
        );
    }

    assert!(
        !host
            .dispatch_native_ime(
                &mut engine,
                &ImeEvent::DeleteSurrounding {
                    before_bytes: 1,
                    after_bytes: 0,
                },
            )
            .unwrap(),
        "non-character-boundary spans must not apply"
    );
    let document = host.document();
    let document = document.lock().expect("document");
    assert_eq!(
        document.text_input_state(input).map(|state| state.value),
        Some("你".into())
    );
}

#[test]
fn scene_host_ime_path_commits_once_into_runtime_then_emits_js() {
    let mut host = VueHost::new();
    let (input, document_id) = install_focused_native_input(&mut host, "Nana");
    let mut engine = RecordingEngine::default();

    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        assert!(
            doc.context_mut()
                .set_ime_preedit(document_id, "世".into(), Some((0, "世".len())))
                .expect("runtime preedit")
        );
    }
    host.emit_native_ime_from_runtime(
        &mut engine,
        &ImeEvent::Preedit {
            text: "世".into(),
            selection: Some((0, "世".len())),
        },
    )
    .expect("emit preedit");
    {
        let document = host.document();
        let document = document.lock().expect("document");
        assert_eq!(
            document.ime_composition(input).map(|ime| ime.text),
            Some("世".into())
        );
        assert_eq!(
            document.text_input_state(input).map(|state| state.value),
            Some("Nana".into()),
            "emit must not write a second preedit buffer"
        );
    }

    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        assert!(
            doc.context_mut()
                .commit_ime(document_id, "世界")
                .expect("runtime commit")
        );
    }
    host.emit_native_ime_from_runtime(&mut engine, &ImeEvent::Commit("世界".into()))
        .expect("emit commit");

    let document = host.document();
    let document = document.lock().expect("document");
    assert!(document.ime_composition(input).is_none());
    let state = document
        .text_input_state(input)
        .expect("runtime committed once");
    assert_eq!(state.value, "Nana世界");
    assert_eq!(
        document.get_attribute(input, "value").as_deref(),
        Some("Nana世界")
    );
    let field = document
        .accessibility_snapshot()
        .into_iter()
        .find(|node| node.id.get() == input.0)
        .expect("committed value stays on the AccessKit TextInput");
    assert_eq!(field.value.as_deref(), Some("Nana世界"));
    drop(document);

    let events = fired_events(&engine);
    assert_eq!(
        events
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "compositionstart",
            "compositionupdate",
            "compositionend",
            "beforeinput",
            "input"
        ]
    );
    assert_eq!(
        events.iter().filter(|(_, name, _)| name == "input").count(),
        1,
        "scene-host IME must not double-insert on emit"
    );
}

#[test]
fn scene_host_ime_disabled_commits_leftover_once() {
    let mut host = VueHost::new();
    let (input, document_id) = install_focused_native_input(&mut host, "Nana");
    let mut engine = RecordingEngine::default();

    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        assert!(
            doc.context_mut()
                .set_ime_preedit(document_id, "世".into(), Some((0, "世".len())))
                .expect("runtime preedit")
        );
    }
    host.emit_native_ime_from_runtime(
        &mut engine,
        &ImeEvent::Preedit {
            text: "世".into(),
            selection: Some((0, "世".len())),
        },
    )
    .expect("emit preedit");
    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        let leftover = doc.ime_composition(input).expect("preedit").text.clone();
        assert!(
            doc.context_mut()
                .commit_ime(document_id, &leftover)
                .expect("runtime leftover commit")
        );
    }
    host.emit_native_ime_from_runtime(&mut engine, &ImeEvent::Disabled)
        .expect("emit disabled");

    let document = host.document();
    let document = document.lock().expect("document");
    assert!(document.ime_composition(input).is_none());
    assert_eq!(
        document
            .text_input_state(input)
            .expect("leftover committed once")
            .value,
        "Nana世"
    );
    drop(document);
    assert_eq!(
        fired_events(&engine)
            .iter()
            .filter(|(_, name, _)| name == "input")
            .count(),
        1
    );
}

#[test]
fn window_blur_keeps_runtime_text_field_focus() {
    let mut host = VueHost::new();
    let (input, _) = install_focused_native_input(&mut host, "NanaUI");
    let mut engine = RecordingEngine::default();
    host.pump_lifecycle(&mut engine, WindowLifecycleEvent::Blur)
        .expect("window blur");
    let document = host.document();
    let doc = document.lock().expect("document");
    assert_eq!(doc.focused(), Some(input));
    let field = doc
        .accessibility_snapshot()
        .into_iter()
        .find(|node| node.id.get() == input.0)
        .expect("TextField remains in the tree");
    assert!(field.focused);
    assert!(
        !fired_events(&engine)
            .iter()
            .any(|(_, name, _)| name == "blur")
    );
}

#[test]
fn native_ime_commit_updates_runtime_textarea_multiline_state() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let area = install_textarea_node(&mut host, "第一行\n");
    let mut engine = RecordingEngine::default();

    host.dispatch_native_ime(
        &mut engine,
        &ImeEvent::Preedit {
            text: "第二".into(),
            selection: Some((0, "第".len())),
        },
    )
    .expect("textarea preedit");
    {
        let document = host.document();
        let document = document.lock().expect("document");
        let composition = document
            .ime_composition(area)
            .expect("textarea keeps CJK preedit selection");
        assert_eq!(composition.text, "第二");
        assert_eq!(composition.selection, Some((0, "第".len())));
        assert_eq!(
            document
                .text_input_state(area)
                .expect("textarea state")
                .value,
            "第一行\n"
        );
    }

    host.dispatch_native_ime(&mut engine, &ImeEvent::Commit("第二行".into()))
        .expect("textarea commit");
    let document = host.document();
    let document = document.lock().expect("document");
    assert!(document.ime_composition(area).is_none());
    let state = document
        .text_input_state(area)
        .expect("textarea runtime state");
    assert_eq!(state.value, "第一行\n第二行");
    assert_eq!(
        state.selection,
        nana_ui_runtime::TextSelection::caret("第一行\n第二行".len())
    );
    assert_eq!(
        document.get_attribute(area, "value").as_deref(),
        Some("第一行\n第二行")
    );
    drop(document);

    let events = fired_events(&engine);
    assert_eq!(
        events
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "compositionstart",
            "compositionupdate",
            "compositionend",
            "beforeinput",
            "input"
        ]
    );
    let input_event = events.last().expect("textarea input event");
    assert_eq!(input_event.0, area.0);
    assert_eq!(
        input_event.2.get("value").and_then(HostValue::as_str),
        Some("第一行\n第二行")
    );
    assert_eq!(
        input_event.2.get("inputType").and_then(HostValue::as_str),
        Some("insertCompositionText")
    );
    assert_eq!(
        events.iter().filter(|(_, name, _)| name == "input").count(),
        1,
        "textarea IME commit must not double-insert"
    );
}

#[test]
fn focused_runtime_textarea_advertises_hosted_ime_request() {
    let mut host = VueHost::new();
    let area = install_textarea_node(&mut host, "第一行\n");
    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.set_focus(area);
    }
    let request = host
        .text_input_request()
        .expect("focused textarea owns IME");
    assert!(request.enabled);
    assert_eq!(request.purpose, nana_ui_platform::TextInputPurpose::Normal);
}

#[test]
fn native_ime_disabled_commits_leftover_runtime_preedit() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, _) = install_input_nodes(&mut host);
    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.set_attribute(input, "value", "Nana");
        doc.set_focus(input);
    }
    let mut engine = RecordingEngine::default();

    host.dispatch_native_ime(
        &mut engine,
        &ImeEvent::Preedit {
            text: "世".into(),
            selection: Some((0, "世".len())),
        },
    )
    .expect("preedit");
    host.dispatch_native_ime(&mut engine, &ImeEvent::Disabled)
        .expect("disabled leftover");

    let document = host.document();
    let document = document.lock().expect("document");
    assert!(document.ime_composition(input).is_none());
    let state = document
        .text_input_state(input)
        .expect("runtime text input state");
    assert_eq!(state.value, "Nana世");
    assert_eq!(
        document.get_attribute(input, "value").as_deref(),
        Some("Nana世")
    );
    drop(document);

    let events = fired_events(&engine);
    assert_eq!(
        events
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "compositionstart",
            "compositionupdate",
            "compositionend",
            "beforeinput",
            "input"
        ]
    );
    assert_eq!(
        events
            .last()
            .and_then(|(_, _, detail)| detail.get("inputType"))
            .and_then(HostValue::as_str),
        Some("insertCompositionText")
    );
}

#[test]
fn commit_text_ignores_disabled_and_read_only_input() {
    for (disabled, read_only) in [(true, false), (false, true)] {
        let mut host = VueHost::new();
        host.callbacks.fire_event = Some(JsFunctionId(1));
        let (input, next) = install_input_nodes(&mut host);
        host.bridge().lock().expect("bridge").register(
            input.0,
            WidgetKind::Input,
            WidgetProps {
                value: "Nana".into(),
                disabled,
                read_only,
                ..WidgetProps::default()
            },
        );
        {
            let document = host.document();
            let mut doc = document.lock().expect("document");
            doc.set_attribute(input, "value", "Nana");
            doc.set_focus(input);
            assert!(doc.set_text_input_state(input, TextInputState::new("Nana")));
        }
        let mut engine = RecordingEngine::default();
        assert!(
            !host.commit_text(&mut engine, "界", "insertText").unwrap(),
            "disabled={disabled} read_only={read_only}"
        );
        assert!(
            host.dispatch_key(&mut engine, "a", "KeyA", Some(input))
                .unwrap()
        );
        {
            let document = host.document();
            let doc = document.lock().expect("document");
            assert_eq!(
                doc.text_input_state(input).expect("text input state").value,
                "Nana"
            );
            assert_eq!(doc.get_attribute(input, "value").as_deref(), Some("Nana"));
        }
        assert!(
            fired_events(&engine)
                .iter()
                .all(|(_, name, _)| name != "beforeinput" && name != "input"),
            "disabled/read-only commit must not fire input events"
        );

        host.document().lock().expect("document").set_focus(next);
        assert!(
            !host.commit_text(&mut engine, "x", "insertText").unwrap(),
            "non-editable focus must not invent text input state"
        );
        assert!(
            host.document()
                .lock()
                .expect("document")
                .text_input_state(next)
                .is_none()
        );
    }

    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, _) = install_input_nodes(&mut host);
    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.set_attribute(input, "value", "Nana");
        doc.set_attribute(input, "readonly", "");
        doc.set_focus(input);
        assert!(doc.set_text_input_state(input, TextInputState::new("Nana")));
    }
    let mut engine = RecordingEngine::default();
    assert!(!host.commit_text(&mut engine, "界", "insertText").unwrap());
    assert_eq!(
        host.document()
            .lock()
            .expect("document")
            .get_attribute(input, "value")
            .as_deref(),
        Some("Nana")
    );
    assert!(
        fired_events(&engine)
            .iter()
            .all(|(_, name, _)| name != "beforeinput" && name != "input")
    );
}

#[test]
fn native_ime_disabled_after_blur_commits_original_field() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, next) = install_input_nodes(&mut host);
    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.set_attribute(input, "value", "Nana");
        doc.set_focus(input);
    }
    let mut engine = RecordingEngine::default();

    host.dispatch_native_ime(
        &mut engine,
        &ImeEvent::Preedit {
            text: "世".into(),
            selection: Some((0, "世".len())),
        },
    )
    .expect("preedit");
    host.document().lock().expect("document").set_focus(next);
    host.dispatch_native_ime(&mut engine, &ImeEvent::Disabled)
        .expect("disabled leftover after blur");

    let document = host.document();
    let document = document.lock().expect("document");
    assert!(document.ime_composition(input).is_none());
    let state = document
        .text_input_state(input)
        .expect("original IME field");
    assert_eq!(state.value, "Nana世");
    assert_eq!(
        document.get_attribute(input, "value").as_deref(),
        Some("Nana世")
    );
    assert!(document.text_input_state(next).is_none());
    assert!(document.get_attribute(next, "value").is_none());
    drop(document);

    let events = fired_events(&engine);
    assert!(
        events
            .iter()
            .any(|(target, name, _)| *target == input.0 && name == "compositionend")
    );
    assert!(
        events
            .iter()
            .any(|(target, name, _)| *target == input.0 && name == "beforeinput")
    );
    assert!(
        events
            .iter()
            .any(|(target, name, _)| *target == input.0 && name == "input")
    );
    assert!(
        !events.iter().any(|(target, name, _)| {
            *target == next.0 && matches!(name.as_str(), "compositionend" | "beforeinput" | "input")
        }),
        "leftover preedit must not insert into the new focus"
    );
    assert_eq!(
        events
            .iter()
            .rev()
            .find(|(_, name, _)| name == "input")
            .and_then(|(_, _, detail)| detail.get("inputType"))
            .and_then(HostValue::as_str),
        Some("insertCompositionText")
    );
}

#[test]
fn native_ime_disabled_clears_blocked_original_without_commit() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, next) = install_input_nodes(&mut host);
    host.bridge().lock().expect("bridge").register(
        input.0,
        WidgetKind::Input,
        WidgetProps {
            value: "Nana".into(),
            ..WidgetProps::default()
        },
    );
    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.set_attribute(input, "value", "Nana");
        doc.set_focus(input);
    }
    let mut engine = RecordingEngine::default();
    host.dispatch_native_ime(
        &mut engine,
        &ImeEvent::Preedit {
            text: "世".into(),
            selection: Some((0, "世".len())),
        },
    )
    .expect("preedit");
    host.bridge()
        .lock()
        .expect("bridge")
        .get_mut(input.0)
        .expect("registered input")
        .props
        .disabled = true;
    host.document().lock().expect("document").set_focus(next);
    host.dispatch_native_ime(&mut engine, &ImeEvent::Disabled)
        .expect("disabled leftover on blocked field");

    let document = host.document();
    let document = document.lock().expect("document");
    assert!(document.ime_composition(input).is_none());
    assert_eq!(
        document
            .text_input_state(input)
            .expect("original field")
            .value,
        "Nana"
    );
    assert_eq!(
        document.get_attribute(input, "value").as_deref(),
        Some("Nana")
    );
    drop(document);
    assert!(
        !fired_events(&engine)
            .iter()
            .any(|(_, name, _)| name == "beforeinput" || name == "input")
    );
}

#[test]
fn native_input_commits_runtime_value_before_firing_vue_events() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, _) = install_input_nodes(&mut host);
    host.bridge().lock().expect("bridge").register(
        input.0,
        WidgetKind::Input,
        WidgetProps {
            value: "你好ab".into(),
            ..WidgetProps::default()
        },
    );
    {
        let document = host.document();
        let mut document = document.lock().expect("document");
        document.set_attribute(input, "value", "你好ab");
        assert!(document.set_text_input_state(input, TextInputState::new("你好ab")));
    }
    let mut engine = RecordingEngine::default();

    assert!(
        host.dispatch_bridge_event(
            &mut engine,
            BridgeEvent::Input {
                id: input.0,
                value: "你娜好ab".into(),
            },
        )
        .expect("native input")
    );

    let document = host.document();
    let document = document.lock().expect("document");
    assert_eq!(
        document.get_attribute(input, "value").as_deref(),
        Some("你娜好ab")
    );
    assert_eq!(
        document.text_input_state(input),
        Some(TextInputState {
            value: "你娜好ab".into(),
            selection: nana_ui_runtime::TextSelection::caret("你娜".len()),
            additional_selections: Vec::new(),
        })
    );
    drop(document);
    assert_eq!(
        fired_events(&engine)
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["input", "update:modelValue"]
    );
}

#[test]
fn tab_and_shift_tab_move_focus_in_document_order() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (first, second) = install_input_nodes(&mut host);
    host.document().lock().expect("document").set_focus(first);
    let mut engine = RecordingEngine::default();

    host.dispatch_keyboard(&mut engine, &KeyboardInput::key_down("Tab", "Tab"), None)
        .expect("tab");
    assert_eq!(host.focused(), Some(second));

    let mut key_up = KeyboardInput::key_down("Tab", "Tab");
    key_up.kind = KeyboardEventKind::Up;
    host.dispatch_keyboard(&mut engine, &key_up, None)
        .expect("tab keyup");
    let mut reverse = KeyboardInput::key_down("Tab", "Tab");
    reverse.modifiers.shift = true;
    host.dispatch_keyboard(&mut engine, &reverse, None)
        .expect("shift tab");
    assert_eq!(host.focused(), Some(first));

    let events = fired_events(&engine);
    assert!(
        events
            .iter()
            .any(|(target, name, _)| { *target == first.0 && name == "blur" })
    );
    assert!(
        events
            .iter()
            .any(|(target, name, _)| { *target == second.0 && name == "focus" })
    );
}

#[test]
fn accessibility_focus_uses_retained_focus_and_dom_lifecycle() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (first, second) = install_input_nodes(&mut host);
    host.document().lock().expect("document").set_focus(first);
    let mut engine = RecordingEngine::default();

    assert!(host.accessibility_focus(&mut engine, second).unwrap());
    assert_eq!(host.focused(), Some(second));
    assert_eq!(
        fired_events(&engine)
            .iter()
            .map(|(target, name, _)| (*target, name.as_str()))
            .collect::<Vec<_>>(),
        [(first.0, "blur"), (second.0, "focus")]
    );
}

#[test]
fn accessibility_set_value_uses_the_committed_text_event_path() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, _) = install_input_nodes(&mut host);
    {
        let document = host.document();
        let mut document = document.lock().expect("document");
        document.set_attribute(input, "value", "旧值");
        assert!(document.set_text_input_state(input, TextInputState::new("旧值")));
    }
    let _ = host.semantic_snapshot();
    let mut engine = RecordingEngine::default();

    assert!(
        host.accessibility_set_value(&mut engine, input, "新的值")
            .unwrap()
    );
    let document = host.document();
    let document = document.lock().expect("document");
    let state = document.text_input_state(input).expect("text input state");
    assert_eq!(state.value, "新的值");
    assert_eq!(
        state.selection,
        nana_ui_runtime::TextSelection::caret("新的值".len())
    );
    assert_eq!(
        fired_events(&engine)
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["beforeinput", "input"]
    );
    assert_eq!(
        fired_events(&engine)
            .last()
            .and_then(|(_, _, detail)| detail.get("inputType"))
            .and_then(HostValue::as_str),
        Some("insertReplacementText")
    );
    drop(document);

    host.document()
        .lock()
        .expect("document")
        .set_attribute(input, "readonly", "");
    let event_count = fired_events(&engine).len();
    assert!(
        !host
            .accessibility_set_value(&mut engine, input, "禁止写入")
            .unwrap()
    );
    assert_eq!(fired_events(&engine).len(), event_count);
}

#[test]
fn accessibility_selection_updates_runtime_and_allows_read_only_text() {
    let mut host = VueHost::new();
    host.callbacks.fire_event = Some(JsFunctionId(1));
    let (input, _) = install_input_nodes(&mut host);
    {
        let document = host.document();
        let mut document = document.lock().expect("document");
        document.set_attribute(input, "value", "你a");
        document.set_attribute(input, "readonly", "");
        assert!(document.set_text_input_state(input, TextInputState::new("你a")));
    }
    let mut engine = RecordingEngine::default();
    let selection = nana_ui_runtime::TextSelection {
        anchor: "你".len(),
        focus: "你a".len(),
    };

    assert!(
        host.accessibility_set_selection(&mut engine, input, selection)
            .unwrap()
    );
    assert_eq!(
        host.document()
            .lock()
            .expect("document")
            .text_input_state(input)
            .unwrap()
            .selection,
        selection
    );
    assert_eq!(
        fired_events(&engine)
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["select"]
    );

    host.document()
        .lock()
        .expect("document")
        .set_attribute(input, "disabled", "");
    assert!(
        !host
            .accessibility_set_selection(
                &mut engine,
                input,
                nana_ui_runtime::TextSelection::caret(0),
            )
            .unwrap()
    );
}

#[test]
fn document_element_set_theme_rebuilds_bg_before_snapshot() {
    // Bugbot: JS Appearance writes dataset.theme via documentElementSet;
    // bridge stylesheet vars must rebuild immediately, not wait for
    // semantic_snapshot (hosts may cache the last snap).
    let host = VueHost::new();
    {
        let bridge_arc = host.bridge();
        let mut bridge = bridge_arc.lock().expect("bridge");
        bridge.register(
            1,
            WidgetKind::Column,
            WidgetProps {
                class_names: vec!["surface".into()],
                ..WidgetProps::default()
            },
        );
        bridge.inject_stylesheet(
            r#"
                :root { --bg: #181818; }
                :root[data-theme="light"] { --bg: #ffffff; }
                .surface { background: var(--bg); width: 100px; height: 40px; }
                "#,
        );
        let light_bg = bridge.get(1).expect("widget").props.layout.background;
        assert_eq!(
            light_bg,
            Some([1.0, 1.0, 1.0, 1.0]),
            "default ThemeMode::Light must resolve light --bg"
        );
    }

    let api = host.host_api_registry();
    api.call(
        "documentElementSet",
        &[
            HostValue::string("dataset"),
            HostValue::string("theme"),
            HostValue::string("dark"),
        ],
    )
    .expect("documentElementSet theme");

    // Assert *before* semantic_snapshot — the whole point of the fix.
    {
        let bridge_arc = host.bridge();
        let bridge = bridge_arc.lock().expect("bridge");
        assert_eq!(bridge.theme(), ThemeMode::Dark);
        let dark_bg = bridge.get(1).expect("widget").props.layout.background;
        assert_eq!(
            dark_bg,
            Some([24.0 / 255.0, 24.0 / 255.0, 24.0 / 255.0, 1.0]),
            "JS dataset.theme must rebuild var(--bg) before next semantic_snapshot"
        );
    }

    let snap = host.semantic_snapshot();
    assert_eq!(snap.theme, ThemeMode::Dark);
}

#[test]
fn set_document_theme_survives_appearance_sync() {
    // Bugbot: setDocumentTheme must write web-api dataset.theme; otherwise
    // semantic_snapshot / appearance → sync_appearance_shared re-applies a
    // stale theme and reverts var(--*).
    let host = VueHost::new();
    {
        let web_api = host.web_api();
        let mut web = web_api.lock().expect("web-api");
        web.set_document_dataset("theme", "light");
    }
    {
        let bridge_arc = host.bridge();
        let mut bridge = bridge_arc.lock().expect("bridge");
        bridge.register(
            1,
            WidgetKind::Column,
            WidgetProps {
                class_names: vec!["surface".into()],
                ..WidgetProps::default()
            },
        );
        bridge.inject_stylesheet(
            r#"
                :root { --bg: #181818; }
                :root[data-theme="light"] { --bg: #ffffff; }
                .surface { background: var(--bg); width: 100px; height: 40px; }
                "#,
        );
    }

    let api = host.host_api_registry();
    api.call("setDocumentTheme", &[HostValue::string("dark")])
        .expect("setDocumentTheme");

    {
        let web_api = host.web_api();
        let web = web_api.lock().expect("web-api");
        assert_eq!(
            web.document_dataset().get("theme").map(String::as_str),
            Some("dark"),
            "setDocumentTheme must mirror into web-api dataset.theme"
        );
    }
    {
        let bridge_arc = host.bridge();
        let bridge = bridge_arc.lock().expect("bridge");
        assert_eq!(bridge.theme(), ThemeMode::Dark);
        let dark_bg = bridge.get(1).expect("widget").props.layout.background;
        assert_eq!(
            dark_bg,
            Some([24.0 / 255.0, 24.0 / 255.0, 24.0 / 255.0, 1.0]),
            "setDocumentTheme must rebuild var(--bg) immediately"
        );
    }

    // Snapshot / appearance sync must not revert to the prior web-api theme.
    let snap = host.semantic_snapshot();
    assert_eq!(snap.theme, ThemeMode::Dark);
    let _ = host.appearance();
    {
        let bridge_arc = host.bridge();
        let bridge = bridge_arc.lock().expect("bridge");
        assert_eq!(bridge.theme(), ThemeMode::Dark);
        let dark_bg = bridge.get(1).expect("widget").props.layout.background;
        assert_eq!(
            dark_bg,
            Some([24.0 / 255.0, 24.0 / 255.0, 24.0 / 255.0, 1.0]),
            "sync_appearance_shared must not revert theme after setDocumentTheme"
        );
    }
}

#[test]
fn next_wakeup_is_none_when_idle() {
    let host = VueHost::new();
    assert!(host.next_wakeup().is_none());
}

#[test]
fn pump_frame_fires_pending_raf_on_host_frame() {
    let mut host = VueHost::new();
    let mut engine = RecordingEngine::default();
    host.bind_event_bridge(&mut engine).unwrap();
    host.web_api().lock().expect("web-api").schedule_raf(1);
    assert!(
        host.next_wakeup().is_some(),
        "pending rAF must request a host wake"
    );

    let fired = host.pump_frame(&mut engine).unwrap();
    assert!(
        fired >= 1,
        "host frame must drain rAF without waiting a fake 16ms"
    );
    assert!(
        host.next_wakeup().is_none(),
        "idle after drain must return None"
    );
}

#[test]
fn pump_frame_invokes_nana_motion_complete_on_transition_end() {
    let mut host = VueHost::new();
    let mut engine = RecordingEngine::default();
    host.bind_event_bridge(&mut engine).unwrap();
    let btn = {
        let document_lock = host.document();
        let mut doc = document_lock.lock().expect("doc");
        let bridge_lock = host.bridge();
        let mut bridge = bridge_lock.lock().expect("bridge");
        let root = doc.mount_root();
        let btn = doc.create_element("button");
        doc.insert(btn, root, None);
        bridge.register(
            btn.0,
            WidgetKind::Button,
            WidgetProps {
                class_names: vec!["btn".into()],
                ..WidgetProps::default()
            },
        );
        bridge.inject_stylesheet(
            ".btn { background: rgb(0, 0, 255); transition: background 200ms linear; } \
                 .btn:hover { background: red; }",
        );
        bridge.resolve_document_layout(&mut doc);
        doc.set_runtime_clock_for_test(std::time::Duration::ZERO);
        doc.set_pointer_hover(0, Some(btn));
        bridge.reapply_interactive_cascade(&mut doc);
        doc.set_runtime_clock_for_test(std::time::Duration::from_millis(220));
        btn
    };
    host.pump_frame(&mut engine).unwrap();
    let complete = engine
        .invocations
        .iter()
        .find(|(id, _)| id.0 == 10)
        .expect("Rust host must call __nanaMotionComplete");
    assert_eq!(complete.1[0].as_f64(), Some(btn.0 as f64));
    let detail = complete.1[1].as_object().expect("motion detail");
    assert_eq!(
        detail.get("type").and_then(HostValue::as_str),
        Some("transitionend")
    );
    assert_eq!(
        detail.get("propertyName").and_then(HostValue::as_str),
        Some("background")
    );
}

#[test]
fn pump_frame_flushes_motion_complete_before_timer_drain() {
    let mut host = VueHost::new();
    let mut engine = RecordingEngine::default();
    host.bind_event_bridge(&mut engine).unwrap();
    {
        let document_lock = host.document();
        let mut doc = document_lock.lock().expect("doc");
        let bridge_lock = host.bridge();
        let mut bridge = bridge_lock.lock().expect("bridge");
        let root = doc.mount_root();
        let btn = doc.create_element("button");
        doc.insert(btn, root, None);
        bridge.register(
            btn.0,
            WidgetKind::Button,
            WidgetProps {
                class_names: vec!["btn".into()],
                ..WidgetProps::default()
            },
        );
        bridge.inject_stylesheet(
            ".btn { background: rgb(0, 0, 255); transition: background 200ms linear; } \
                 .btn:hover { background: red; }",
        );
        bridge.resolve_document_layout(&mut doc);
        doc.set_runtime_clock_for_test(std::time::Duration::ZERO);
        doc.set_pointer_hover(0, Some(btn));
        bridge.reapply_interactive_cascade(&mut doc);
        doc.set_runtime_clock_for_test(std::time::Duration::from_millis(220));
        assert!(bridge.tick_css_animations(&mut doc));
    }
    host.web_api().lock().expect("web-api").schedule_raf(1);
    host.pump_frame(&mut engine).unwrap();
    let complete_at = engine
        .invocations
        .iter()
        .position(|(id, _)| id.0 == 10)
        .expect("Rust complete");
    let drain_at = engine
        .invocations
        .iter()
        .position(|(id, _)| id.0 == 12)
        .expect("timer drain");
    assert!(
        complete_at < drain_at,
        "flush motion complete must run before JS fallback timers"
    );
    assert_eq!(
        engine
            .invocations
            .iter()
            .filter(|(id, _)| id.0 == 10)
            .count(),
        1,
        "transitionend must fire once"
    );
}

#[test]
fn hosted_same_beat_flush_does_not_double_complete_on_next_pump() {
    let mut host = VueHost::new();
    let mut engine = RecordingEngine::default();
    host.bind_event_bridge(&mut engine).unwrap();
    let epoch = Instant::now();
    host.set_host_animation_epoch(epoch);
    {
        let document_lock = host.document();
        let mut doc = document_lock.lock().expect("doc");
        let bridge_lock = host.bridge();
        let mut bridge = bridge_lock.lock().expect("bridge");
        let root = doc.mount_root();
        let btn = doc.create_element("button");
        doc.insert(btn, root, None);
        bridge.register(
            btn.0,
            WidgetKind::Button,
            WidgetProps {
                class_names: vec!["btn".into()],
                ..WidgetProps::default()
            },
        );
        bridge.inject_stylesheet(
            ".btn { background: rgb(0, 0, 255); transition: background 200ms linear; } \
                 .btn:hover { background: red; }",
        );
        bridge.resolve_document_layout(&mut doc);
        doc.set_pointer_hover(0, Some(btn));
        bridge.reapply_interactive_cascade(&mut doc);
        let frame = doc.advance_css_animations(std::time::Duration::from_millis(220));
        assert!(bridge.apply_css_animation_samples(&mut doc, frame));
    }
    host.flush_motion_complete(&mut engine).unwrap();
    host.web_api().lock().expect("web-api").schedule_raf(1);
    host.pump_frame(&mut engine).unwrap();
    assert_eq!(
        engine
            .invocations
            .iter()
            .filter(|(id, _)| id.0 == 10)
            .count(),
        1,
        "hosted apply+flush on T_end plus later pump must not double-dispatch"
    );
}

#[test]
fn pump_frame_nested_raf_follows_next_wakeup_not_busy_loop() {
    struct RescheduleEngine {
        web_api: SharedWebApiState,
        drain_count: usize,
    }
    impl JsEngine for RescheduleEngine {
        fn initialize(&mut self, _artifact: RuntimeArtifact) -> Result<(), JsEngineError> {
            Ok(())
        }
        fn register_host_api(&mut self, _api: &HostApiRegistry) -> Result<(), JsEngineError> {
            Ok(())
        }
        fn resolve_function(&mut self, _name: &str) -> Result<JsFunctionId, JsEngineError> {
            Ok(JsFunctionId(1))
        }
        fn invoke(
            &mut self,
            _target: JsFunctionId,
            args: &[HostValue],
        ) -> Result<HostValue, JsEngineError> {
            if let Some(HostValue::Object(payload)) = args.first()
                && let Some(HostValue::Array(raf)) = payload.get("raf")
                && !raf.is_empty()
            {
                self.drain_count += 1;
                if self.drain_count == 1
                    && let Ok(mut web) = self.web_api.lock()
                {
                    web.schedule_raf(2);
                }
            }
            Ok(HostValue::Null)
        }
        fn run_microtasks(&mut self) -> Result<(), JsEngineError> {
            Ok(())
        }
        fn interrupt(&mut self) {}
        fn request_gc(&mut self) {}
        fn shutdown(&mut self) {}
    }

    let mut host = VueHost::new();
    let mut engine = RescheduleEngine {
        web_api: host.web_api(),
        drain_count: 0,
    };
    host.bind_event_bridge(&mut engine).unwrap();
    host.web_api().lock().expect("web-api").schedule_raf(1);

    let before = Instant::now();
    let fired = host.pump_frame(&mut engine).unwrap();
    assert_eq!(
        engine.drain_count, 1,
        "nested rAF must not drain in the same host frame"
    );
    assert!(fired >= 1);
    let wakeup = host
        .next_wakeup()
        .expect("nested rAF must schedule the next host frame");
    assert!(
        wakeup >= before + std::time::Duration::from_millis(8),
        "nested rAF must wait for next_wakeup (~16ms), not spin"
    );
    assert!(wakeup <= before + std::time::Duration::from_millis(50));
}

#[test]
fn get_user_media_mock_feeds_video_host_texture() {
    let host = VueHost::new();
    let api = host.host_api_registry();
    let stream = api
        .call(
            "mediaDevicesGetUserMedia",
            &[HostValue::Object(
                [("video".into(), HostValue::Bool(true))]
                    .into_iter()
                    .collect(),
            )],
        )
        .expect("mock camera must not hang");
    let stream_id = stream
        .as_object()
        .and_then(|map| map.get("id"))
        .and_then(HostValue::as_u64)
        .expect("stream id");
    let video = api
        .call("mediaCreate", &[HostValue::string("video")])
        .expect("video element");
    let media_id = video
        .as_object()
        .and_then(|map| map.get("id"))
        .and_then(HostValue::as_u64)
        .expect("media id");
    let attached = api
        .call(
            "mediaSetSrcObject",
            &[HostValue::BigInt(media_id), HostValue::BigInt(stream_id)],
        )
        .expect("attach preview");
    let attached = attached.as_object().expect("descriptor");
    assert_eq!(
        attached.get("hasVideoFrame").and_then(HostValue::as_bool),
        Some(true)
    );
    assert_eq!(
        attached.get("slot").and_then(HostValue::as_str),
        Some(format!("video:{media_id}").as_str())
    );

    let document = host.document();
    let mut doc = document.lock().expect("document");
    let node = doc.create_element("video");
    let root = doc.mount_root();
    doc.insert(node, root, None);
    doc.set_attribute(node, "data-nana-video", &media_id.to_string());
    doc.apply_layout_boxes(&[(
        node,
        crate::LayoutBox {
            handle: node,
            x: 0.0,
            y: 0.0,
            width: 160.0,
            height: 90.0,
        },
    )]);
    let id = nana_ui_runtime::StableNodeId::try_from(node).expect("video id");
    let content = doc
        .world()
        .custom_render(id)
        .expect("camera preview must land on CustomRenderNode");
    assert_eq!(content.renderer.as_ref(), "nana.host-texture");
    assert_eq!(content.resource.as_ref(), &format!("video:{media_id}"));
}

#[test]
fn get_user_media_permission_failure_degrades() {
    let host = VueHost::new();
    let api = host.host_api_registry();
    api.call("mediaSetCaptureMode", &[HostValue::string("deny")])
        .expect("deny mode");
    let error = api
        .call(
            "mediaDevicesGetUserMedia",
            &[HostValue::Object(
                [("video".into(), HostValue::Bool(true))]
                    .into_iter()
                    .collect(),
            )],
        )
        .expect_err("permission failure must be testable");
    assert_eq!(error.name, "NotAllowedError");
}

#[test]
fn retain_live_keeps_audio_after_prepare_and_releases_on_remove() {
    let host = VueHost::new();
    let api = host.host_api_registry();
    let audio = api
        .call("mediaCreate", &[HostValue::string("audio")])
        .expect("audio");
    let audio_id = audio
        .as_object()
        .and_then(|map| map.get("id"))
        .and_then(HostValue::as_u64)
        .expect("audio id");
    api.call(
        "mediaSetSrc",
        &[HostValue::BigInt(audio_id), HostValue::string("track.ogg")],
    )
    .expect("audio src");
    let video = api
        .call("mediaCreate", &[HostValue::string("video")])
        .expect("video");
    let video_id = video
        .as_object()
        .and_then(|map| map.get("id"))
        .and_then(HostValue::as_u64)
        .expect("video id");
    api.call(
        "mediaSetSrc",
        &[HostValue::BigInt(video_id), HostValue::string("nana:mock")],
    )
    .expect("video src");

    let audio_node = {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        let node = doc.create_element("audio");
        let root = doc.mount_root();
        doc.insert(node, root, None);
        doc.set_attribute(node, "data-nana-media", &audio_id.to_string());
        node
    };
    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        let node = doc.create_element("video");
        let root = doc.mount_root();
        doc.insert(node, root, None);
        doc.set_attribute(node, "data-nana-media", &video_id.to_string());
        doc.set_attribute(node, "data-nana-video", &video_id.to_string());
    }

    let pending = host.bridge.lock().unwrap().peek_snapshot_changes();
    let sets = host.live_media_sets();
    assert_eq!(
        host.bridge.lock().unwrap().peek_snapshot_changes(),
        pending,
        "media discovery must not consume pending semantic updates"
    );
    assert!(
        sets.retain.iter().any(|id| id.0 == audio_id),
        "CPU retain must include the audio element"
    );
    assert!(
        sets.retain.iter().any(|id| id.0 == video_id),
        "CPU retain must include the video element"
    );
    assert!(
        sets.visual.iter().any(|id| id.0 == video_id),
        "visual GPU set must include video"
    );
    assert!(
        sets.visual.iter().all(|id| id.0 != audio_id),
        "audio must not occupy a visual HostTexture slot"
    );

    host.retain_live_media();
    let playing = api
        .call("mediaPlay", &[HostValue::BigInt(audio_id)])
        .expect("audio must still play after retain_live");
    assert_eq!(
        playing
            .as_object()
            .and_then(|map| map.get("paused"))
            .and_then(HostValue::as_bool),
        Some(false)
    );
    assert_eq!(
        playing
            .as_object()
            .and_then(|map| map.get("hasVideoFrame"))
            .and_then(HostValue::as_bool),
        Some(false)
    );

    {
        let document = host.document();
        let mut doc = document.lock().expect("document");
        doc.remove(audio_node);
    }
    host.retain_live_media();
    let error = api
        .call("mediaPlay", &[HostValue::BigInt(audio_id)])
        .expect_err("removed audio must be released");
    assert!(
        error.message.contains("unknown media"),
        "expected unknown media, got {}",
        error.message
    );
    api.call("mediaPlay", &[HostValue::BigInt(video_id)])
        .expect("video must survive audio prune");
}

#[test]
fn media_play_pause_current_time_host_ops() {
    let host = VueHost::new();
    let api = host.host_api_registry();
    let created = api
        .call("mediaCreate", &[HostValue::string("audio")])
        .expect("audio");
    let id = created
        .as_object()
        .and_then(|map| map.get("id"))
        .and_then(HostValue::as_u64)
        .expect("id");
    api.call(
        "mediaSetSrc",
        &[HostValue::BigInt(id), HostValue::string("track.ogg")],
    )
    .expect("src");
    let playing = api
        .call("mediaPlay", &[HostValue::BigInt(id)])
        .expect("play");
    assert_eq!(
        playing
            .as_object()
            .and_then(|map| map.get("paused"))
            .and_then(HostValue::as_bool),
        Some(false)
    );
    assert_eq!(
        playing
            .as_object()
            .and_then(|map| map.get("hasVideoFrame"))
            .and_then(HostValue::as_bool),
        Some(false),
        "audio must not fabricate a video frame"
    );
    api.call(
        "mediaSetCurrentTime",
        &[HostValue::BigInt(id), HostValue::Number(0.4)],
    )
    .expect("seek");
    let paused = api
        .call("mediaPause", &[HostValue::BigInt(id)])
        .expect("pause");
    let paused = paused.as_object().expect("descriptor");
    assert_eq!(
        paused.get("paused").and_then(HostValue::as_bool),
        Some(true)
    );
    assert_eq!(
        paused.get("currentTime").and_then(HostValue::as_f64),
        Some(0.4)
    );
}
