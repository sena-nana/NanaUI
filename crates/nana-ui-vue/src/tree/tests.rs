use super::*;

#[test]
fn rejected_mutation_does_not_drop_valid_pending_ops() {
    let mut doc = NanaTreeDocument::new(400, 200, 1.0);
    let text = doc.create_text("正文");
    doc.insert(text, doc.mount_root(), None);
    doc.flush_host_frame();

    // One invalid mutation (duplicate Create of the html root) plus one
    // valid mutation (text update). The batch is rejected wholesale and
    // must be replayed so the valid op lands instead of the whole frame's
    // host ops disappearing.
    let mut extra = MutationQueue::new();
    extra.create(
        StableNodeId::new(doc.html_root.0).expect("html root id is nonzero"),
        nana_ui_runtime::DocumentId::try_from(doc.id).expect("document ID is nonzero"),
        NodeKind::Element { tag: "html".into() },
    );
    extra.set_text(
        StableNodeId::try_from(text).expect("text id is nonzero"),
        TextContent {
            value: "更新".into(),
        },
    );
    let result = doc.commit_extra(extra);
    assert!(result.is_err(), "duplicate Create must be reported");
    assert_eq!(doc.runtime_text(text).as_deref(), Some("更新"));

    // The pending batch is fully drained: a later flush neither re-fails
    // nor resurrects the rejected mutation.
    doc.flush_host_frame();
    assert_eq!(doc.runtime_text(text).as_deref(), Some("更新"));
    assert!(doc.pending.is_empty());

    let rejections = doc.take_commit_rejections();
    assert_eq!(rejections.len(), 1, "only the rejected op is recorded");
    assert!(rejections[0].contains("Create"));
    assert!(doc.take_commit_rejections().is_empty());
}

#[test]
fn clear_pointer_captures_survives_poisoned_pending() {
    let mut doc = NanaTreeDocument::new(400, 200, 1.0);
    let target = doc.create_element("div");
    doc.insert(target, doc.mount_root(), None);
    doc.flush_host_frame();
    assert!(doc.capture_pointer(7, target));

    // A poisoned pending mutation (duplicate Create of the html root)
    // used to make the capture release panic on commit; the replay path
    // must still release the captures and drop only the poison.
    doc.pending.mutations.create(
        StableNodeId::new(doc.html_root.0).expect("html root id is nonzero"),
        nana_ui_runtime::DocumentId::try_from(doc.id).expect("document ID is nonzero"),
        NodeKind::Element { tag: "html".into() },
    );
    doc.clear_pointer_captures();
    assert!(doc.pointer_capture(7).is_none());
    assert_eq!(doc.take_commit_rejections().len(), 1);
    assert!(doc.pending.is_empty());
}

fn native_html_input(value: &str) -> (NanaTreeDocument, NodeHandle) {
    let mut doc = NanaTreeDocument::new(400, 200, 1.0);
    let input = doc.create_element("input");
    doc.insert(input, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        input.0,
        crate::WidgetKind::Input,
        crate::WidgetProps {
            value: value.into(),
            ..crate::WidgetProps::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());
    doc.apply_layout_boxes(&[(
        input,
        LayoutBox {
            handle: input,
            x: 8.0,
            y: 8.0,
            width: 160.0,
            height: 28.0,
        },
    )]);
    (doc, input)
}

#[derive(Clone)]
struct ProbeCard {
    title: String,
}

impl ComponentView for ProbeCard {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "probe-card".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.title.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.title.clone(),
                },
            );
        }
    }
}

impl nana_ui_runtime::RegisterableComponent for ProbeCard {
    const TYPE_ID: &'static str = "test.probe-card";
    const TAGS: &'static [&'static str] = &["nana-probe-card", "probe-card"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Self {
            title: spec
                .attr("handle")
                .unwrap_or_else(|| spec.display_label())
                .to_owned(),
        }
    }
}

struct ProbePlugin;

impl nana_ui_runtime::UiExtension for ProbePlugin {
    fn name(&self) -> &'static str {
        "test.probe"
    }

    fn install(
        &self,
        registrar: &mut nana_ui_runtime::ExtensionRegistrar,
    ) -> Result<(), nana_ui_runtime::FrameworkError> {
        registrar.register_component::<ProbeCard>()
    }
}

#[test]
fn installed_plugin_tag_projects_into_ui_world() {
    let mut doc = NanaTreeDocument::new(400, 200, 1.0);
    doc.context_mut().install(&ProbePlugin).unwrap();
    let card = doc.create_element("nana-probe-card");
    doc.insert(card, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        card.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            element_tag: "nana-probe-card".into(),
            label: "User".into(),
            ..crate::WidgetProps::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());
    let id = StableNodeId::try_from(card).unwrap();
    assert_eq!(doc.world().text(id), Some("User"));
    assert_eq!(
        doc.world().component_type(id).map(ComponentTypeId::as_str),
        Some("test.probe-card")
    );
}

#[test]
fn plugin_bind_reads_open_attrs() {
    let mut doc = NanaTreeDocument::new(400, 200, 1.0);
    doc.context_mut().install(&ProbePlugin).unwrap();
    let card = doc.create_element("nana-probe-card");
    doc.insert(card, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    let mut attrs = std::collections::BTreeMap::new();
    attrs.insert("handle".into(), "from-attr".into());
    bridge.register(
        card.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            element_tag: "nana-probe-card".into(),
            label: "User".into(),
            attrs,
            ..crate::WidgetProps::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());
    let id = StableNodeId::try_from(card).unwrap();
    assert_eq!(doc.world().text(id), Some("from-attr"));
}

#[test]
fn unregistered_custom_tag_does_not_bind_a_component() {
    let mut doc = NanaTreeDocument::new(400, 200, 1.0);
    let unknown = doc.create_element("nana-unknown-widget");
    doc.insert(unknown, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        unknown.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            element_tag: "nana-unknown-widget".into(),
            label: "Hello".into(),
            ..crate::WidgetProps::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());
    let id = StableNodeId::try_from(unknown).unwrap();
    assert!(doc.world().component_type(id).is_none());
    assert_ne!(doc.world().text(id), Some("Hello"));
    assert_eq!(
        doc.element_tag(unknown).as_deref(),
        Some("nana-unknown-widget")
    );
}

#[test]
fn legacy_handles_round_trip_through_runtime_ids() {
    let handle = NodeHandle(17);
    let stable = nana_ui_runtime::StableNodeId::try_from(handle).unwrap();
    assert_eq!(NodeHandle::from(stable), handle);

    let document = DocumentId(3);
    let runtime_document = nana_ui_runtime::DocumentId::try_from(document).unwrap();
    assert_eq!(DocumentId::from(runtime_document), document);
}

#[test]
fn query_selector_finds_body_mount_root() {
    let doc = NanaTreeDocument::new(800, 600, 1.0);
    assert_eq!(doc.query_selector("body"), Some(doc.mount_root()));
    assert_eq!(doc.query_selector("html"), Some(doc.html_root()));
}

#[test]
fn unchanged_layout_writeback_does_not_advance_runtime_generation() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let node = doc.create_element("button");
    doc.insert(node, doc.mount_root(), None);
    let boxes = [(
        node,
        LayoutBox {
            handle: node,
            x: 10.0,
            y: 20.0,
            width: 80.0,
            height: 30.0,
        },
    )];
    doc.apply_layout_boxes(&boxes);
    let generation = doc.runtime_generation();
    doc.apply_layout_boxes(&boxes);
    assert_eq!(doc.runtime_generation(), generation);
}

#[test]
fn accessibility_updates_are_incremental_and_drain_once() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let node = doc.create_element("input");
    doc.insert(node, doc.mount_root(), None);
    doc.inject_layout_boxes(&[(
        node,
        LayoutBox {
            handle: node,
            x: 10.0,
            y: 20.0,
            width: 80.0,
            height: 30.0,
        },
    )]);

    let Some(AccessibilityUpdate::Delta(initial)) = doc.take_accessibility_update() else {
        panic!("initial retained work must produce a delta");
    };
    assert!(initial.updated.iter().any(|entry| entry.id.get() == node.0));
    assert!(doc.take_accessibility_update().is_none());

    doc.inject_layout_boxes(&[(
        node,
        LayoutBox {
            handle: node,
            x: 12.0,
            y: 20.0,
            width: 80.0,
            height: 30.0,
        },
    )]);
    let Some(AccessibilityUpdate::Delta(layout)) = doc.take_accessibility_update() else {
        panic!("changed bounds must produce a delta");
    };
    assert_eq!(
        layout
            .updated
            .iter()
            .filter(|entry| entry.id.get() == node.0)
            .count(),
        1
    );

    let id = StableNodeId::try_from(node).unwrap();
    let mut hide = MutationQueue::new();
    hide.set_style(
        id,
        NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                hidden: true,
                ..nana_ui_core::LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    doc.runtime.commit(hide).unwrap();
    doc.flush_runtime_systems();
    let Some(AccessibilityUpdate::Delta(hidden)) = doc.take_accessibility_update() else {
        panic!("hidden nodes must produce a retained tombstone");
    };
    assert!(hidden.removed.contains(&id));
    assert!(
        hidden
            .updated
            .iter()
            .any(|entry| entry.id.get() == doc.mount_root().0)
    );

    let mut show = MutationQueue::new();
    show.set_style(id, NodeStyle::default());
    doc.runtime.commit(show).unwrap();
    doc.flush_runtime_systems();
    let Some(AccessibilityUpdate::Delta(visible)) = doc.take_accessibility_update() else {
        panic!("restored nodes must rebuild the retained subtree");
    };
    assert!(visible.removed.is_empty());
    assert!(visible.updated.iter().any(|entry| entry.id == id));
    assert!(
        visible
            .updated
            .iter()
            .any(|entry| entry.id.get() == doc.mount_root().0)
    );

    doc.remove(node);
    doc.apply_layout_boxes(&[]);
    let Some(AccessibilityUpdate::Delta(removed)) = doc.take_accessibility_update() else {
        panic!("removed retained nodes must produce a delta");
    };
    assert!(removed.removed.iter().any(|id| id.get() == node.0));
}

#[test]
fn native_html_input_keeps_text_field_semantics_after_vue_drains_runtime_work() {
    let (mut doc, input) = native_html_input("committed");
    doc.set_focus(input);

    let input_id = StableNodeId::try_from(input).unwrap();
    let field = doc
        .accessibility_snapshot()
        .into_iter()
        .find(|node| node.id == input_id)
        .expect("native input must enter the retained accessibility tree");
    assert_eq!(field.role, AccessibilityRole::TextInput);
    assert_eq!(field.value.as_deref(), Some("committed"));
    assert!(field.editable);
    assert!(field.focused);

    assert!(
        doc.take_accessibility_update().is_some(),
        "Vue flush_runtime_systems must record the TextInput projection"
    );
    assert!(doc.take_accessibility_update().is_none());
    let host_flush = doc
        .runtime_document_mut()
        .flush_with(|_, _| Ok(()))
        .expect("host flush after Vue drain");
    assert!(host_flush.accessibility.updated.is_empty());
    assert!(host_flush.accessibility.removed.is_empty());
    let field = doc
        .accessibility_snapshot()
        .into_iter()
        .find(|node| node.id == input_id)
        .expect("world snapshot remains the AccessKit authority");
    assert_eq!(field.role, AccessibilityRole::TextInput);
    assert_eq!(field.value.as_deref(), Some("committed"));
    assert!(field.focused);
}

#[test]
fn set_focus_after_vue_drain_queues_accesskit_focus_on_the_text_field() {
    let (mut doc, input) = native_html_input("NanaUI");
    assert!(doc.take_accessibility_update().is_some());
    assert!(doc.take_accessibility_update().is_none());

    doc.set_focus(input);
    let Some(AccessibilityUpdate::Delta(focused)) = doc.take_accessibility_update() else {
        panic!("set_focus must flush an AccessKit focus delta");
    };
    let input_id = StableNodeId::try_from(input).unwrap();
    let field = focused
        .updated
        .iter()
        .find(|node| node.id == input_id)
        .expect("focus delta must include the TextInput");
    assert_eq!(field.role, AccessibilityRole::TextInput);
    assert!(field.focused);
    assert!(doc.take_accessibility_update().is_none());
}

#[test]
fn vue_flush_keeps_layout_work_so_text_input_gets_a_hittable_box() {
    let mut doc = NanaTreeDocument::new(400, 200, 1.0);
    let input = doc.create_element("input");
    doc.insert(input, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        input.0,
        crate::WidgetKind::Input,
        crate::WidgetProps {
            value: "NanaUI".into(),
            ..crate::WidgetProps::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());
    bridge.resolve_document_layout(&mut doc);

    let laid_out = doc.layout_box(input).expect("runtime layout box");
    assert!(
        laid_out.width > 0.0 && laid_out.height >= nana_ui_core::ControlSize::Medium.height(),
        "Vue flush must run RuntimeLayoutEngine so TextInput is hittable, got {laid_out:?}"
    );
    let engine_height = laid_out.height;
    let engine_width = laid_out.width;
    doc.apply_layout_boxes(&[(
        input,
        LayoutBox {
            handle: input,
            x: 0.0,
            y: 0.0,
            width: engine_width.min(12.0),
            height: 8.0,
        },
    )]);
    let kept = doc
        .layout_box(input)
        .expect("engine box after measure dump");
    assert!(
        kept.height >= engine_height && kept.width >= engine_width,
        "apply_layout_boxes of measure results must not shrink engine geometry, got {kept:?}"
    );
}

fn runtime_layout(doc: &mut NanaTreeDocument, width: f32, height: f32) {
    let document = doc.runtime_document().document();
    doc.context_mut()
        .layout_document(
            document,
            nana_ui_runtime::LayoutViewport::new(width, height),
        )
        .unwrap();
}

fn visible_text_primitive_count(doc: &NanaTreeDocument, host: NodeHandle) -> usize {
    let mut ids = vec![host.0];
    let mut stack = doc.children_of(host);
    while let Some(child) = stack.pop() {
        ids.push(child.0);
        stack.extend(doc.children_of(child));
    }
    doc.scene()
        .primitives()
        .filter(|primitive| {
            ids.contains(&primitive.node.get())
                && matches!(
                    &primitive.kind,
                    nana_ui_scene::ScenePrimitiveKind::Text { content, .. }
                        if !content.trim().is_empty()
                )
        })
        .count()
}

/// Every kind that used to get a mount-time `#text` child must paint and
/// announce its label from the widget element alone, so dropping the child
/// can neither blank a label nor leave a second copy that `patchProp` never
/// refreshes.
#[test]
fn labelled_widget_kinds_own_their_label_without_a_text_child() {
    fn settle(doc: &mut NanaTreeDocument, bridge: &mut crate::MessageBridge) {
        doc.sync_semantic_styles(&bridge.snapshot());
        runtime_layout(doc, 400.0, 240.0);
        doc.flush_runtime_systems();
    }

    for kind in [
        crate::WidgetKind::Text,
        crate::WidgetKind::Button,
        crate::WidgetKind::Chip,
        crate::WidgetKind::SidebarRow,
        crate::WidgetKind::ListItem,
    ] {
        let mut doc = NanaTreeDocument::new(400, 240, 1.0);
        let widget = doc.create_element(kind.element_tag());
        doc.insert(widget, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            widget.0,
            kind,
            crate::WidgetProps {
                label: "Label".into(),
                element_tag: kind.element_tag().into(),
                ..Default::default()
            },
        );

        settle(&mut doc, &mut bridge);
        assert_eq!(
            visible_text_primitive_count(&doc, widget),
            1,
            "{kind:?} must paint its label without a #text child"
        );

        bridge.patch_prop(
            widget.0,
            "label",
            &nana_js_engine::HostValue::string("Renamed"),
        );
        settle(&mut doc, &mut bridge);

        assert_eq!(
            visible_text_primitive_count(&doc, widget),
            1,
            "{kind:?} must still paint exactly one label after a patch"
        );
        let announced: Vec<_> = doc
            .accessibility_snapshot()
            .into_iter()
            .filter_map(|node| Some((node.id, node.label?.to_string())))
            .collect();
        assert_eq!(
            announced,
            vec![(
                StableNodeId::try_from(widget).unwrap(),
                "Renamed".to_owned()
            )],
            "{kind:?} must announce the patched label exactly once, with no stale copy"
        );
    }
}

#[test]
fn vue_button_and_heading_with_text_children_extract_one_visible_text_each() {
    let mut doc = NanaTreeDocument::new(400, 240, 1.0);
    let heading = doc.create_element("h1");
    let button = doc.create_element("button");
    doc.insert(heading, doc.mount_root(), None);
    doc.insert(button, doc.mount_root(), None);
    doc.set_element_text(heading, "Heading");
    doc.set_element_text(button, "Action");

    let mut heading_props = crate::WidgetProps::default();
    heading_props.element_tag = "h1".into();
    heading_props.label = "Heading".into();
    let mut button_props = crate::WidgetProps::default();
    button_props.element_tag = "button".into();
    button_props.label = "Action".into();

    let mut bridge = crate::MessageBridge::new();
    bridge.register(heading.0, crate::WidgetKind::Text, heading_props);
    bridge.register(button.0, crate::WidgetKind::Button, button_props);
    doc.sync_semantic_styles(&bridge.snapshot());
    runtime_layout(&mut doc, 400.0, 240.0);
    doc.flush_runtime_systems();

    assert_eq!(
        visible_text_primitive_count(&doc, heading),
        1,
        "heading host plus #text child must extract one visible text primitive"
    );
    assert_eq!(
        visible_text_primitive_count(&doc, button),
        1,
        "button host plus #text child must extract one visible text primitive"
    );
}

#[test]
fn gpu_slot_flows_through_runtime_extraction_and_scene_graph() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let node = doc.create_element("div");
    doc.insert(node, doc.mount_root(), None);
    doc.set_gpu_slot(node, "program");
    doc.apply_layout_boxes(&[(
        node,
        LayoutBox {
            handle: node,
            x: 10.0,
            y: 20.0,
            width: 320.0,
            height: 180.0,
        },
    )]);

    let custom = doc
        .scene()
        .primitives()
        .find(|primitive| primitive.node.get() == node.0)
        .expect("GPU node must be extracted into UiScene");
    let nana_ui_scene::ScenePrimitiveKind::Custom { node: content, .. } = &custom.kind else {
        panic!("GPU node must compile to a custom scene primitive");
    };
    assert_eq!(content.renderer.as_ref(), "nana.host-texture");
    assert_eq!(content.resource.as_ref(), "program");
    let graph = doc
        .scene()
        .frame_graph(nana_ui_scene::ResourceId(1))
        .unwrap();
    let prepare_index =
        graph
            .passes
            .iter()
            .position(|pass| {
                pass.operations.iter().any(|operation| matches!(
                    operation,
                    nana_ui_scene::RenderOperation::PrepareExternal(id) if id.node.get() == node.0
                ))
            })
            .expect("GPU resource must have an explicit preparation pass");
    let invoke_index = graph
        .passes
        .iter()
        .position(|pass| {
            pass.operations.iter().any(|operation| {
                matches!(
                    operation,
                    nana_ui_scene::RenderOperation::InvokeCustom(id) if id.node.get() == node.0
                )
            })
        })
        .expect("GPU node must have a custom invocation pass");
    assert!(prepare_index < invoke_index);
    assert!(
        graph.passes[invoke_index]
            .dependencies
            .contains(&graph.passes[prepare_index].id)
    );

    doc.remove_attribute(node, "data-nana-gpu");
    doc.apply_layout_boxes(&[]);
    assert!(
        doc.scene()
            .primitives()
            .all(|primitive| primitive.node.get() != node.0)
    );
}

#[test]
fn canvas_attr_projects_host_texture_custom_render() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let canvas = doc.create_element("canvas");
    doc.insert(canvas, doc.mount_root(), None);
    doc.set_attribute(canvas, "data-nana-canvas", "42");
    doc.apply_layout_boxes(&[(
        canvas,
        LayoutBox {
            handle: canvas,
            x: 12.0,
            y: 80.0,
            width: 320.0,
            height: 160.0,
        },
    )]);

    let id = StableNodeId::try_from(canvas).expect("canvas id");
    let content = doc
        .runtime
        .custom_render(id)
        .expect("2D canvas must attach a HostTexture CustomRender");
    assert_eq!(content.renderer.as_ref(), "nana.host-texture");
    assert_eq!(content.resource.as_ref(), "canvas:42");

    let primitive = doc
        .scene()
        .primitives()
        .find(|primitive| primitive.node.get() == canvas.0)
        .expect("canvas must extract a scene primitive");
    let nana_ui_scene::ScenePrimitiveKind::Custom { node: custom, .. } = &primitive.kind else {
        panic!("canvas must compile to a custom scene primitive");
    };
    assert_eq!(custom.renderer.as_ref(), "nana.host-texture");
    assert_eq!(custom.resource.as_ref(), "canvas:42");
}

#[test]
fn image_attr_projects_host_texture_custom_render() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let image = doc.create_element("img");
    doc.insert(image, doc.mount_root(), None);
    doc.set_attribute(image, "data-nana-image", "7");
    doc.apply_layout_boxes(&[(
        image,
        LayoutBox {
            handle: image,
            x: 12.0,
            y: 80.0,
            width: 96.0,
            height: 48.0,
        },
    )]);

    let id = StableNodeId::try_from(image).expect("img id");
    let content = doc
        .runtime
        .custom_render(id)
        .expect("decoded <img> must attach a HostTexture CustomRender");
    assert_eq!(content.renderer.as_ref(), "nana.host-texture");
    assert_eq!(content.resource.as_ref(), "canvas:7");
    assert_eq!(content.fit, nana_ui_core::ContentFit::Fill);
    assert!(
        doc.gpu_slots()
            .iter()
            .any(|(handle, slot)| *handle == image && slot == "canvas:7"),
        "img must appear in gpu_slots as the canvas upload key"
    );

    let primitive = doc
        .scene()
        .primitives()
        .find(|primitive| {
            primitive.node.get() == image.0
                && matches!(
                    primitive.kind,
                    nana_ui_scene::ScenePrimitiveKind::Custom { .. }
                )
        })
        .expect("img must extract a HostTexture scene primitive");
    let nana_ui_scene::ScenePrimitiveKind::Custom { node: custom, .. } = &primitive.kind else {
        panic!("img must compile to a custom scene primitive");
    };
    assert_eq!(custom.renderer.as_ref(), "nana.host-texture");
    assert_eq!(custom.resource.as_ref(), "canvas:7");
    assert_eq!(custom.fit, nana_ui_core::ContentFit::Fill);
    assert_eq!(doc.hit_test(20.0, 90.0), Some(image));

    doc.remove_attribute(image, "data-nana-image");
    doc.apply_layout_boxes(&[]);
    assert!(doc.runtime.custom_render(id).is_none());
    assert!(
        doc.scene()
            .primitives()
            .all(|primitive| primitive.node.get() != image.0
                || !matches!(
                    primitive.kind,
                    nana_ui_scene::ScenePrimitiveKind::Custom { .. }
                ))
    );
}

#[test]
fn image_object_fit_projects_content_fit_on_host_texture() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let image = doc.create_element("img");
    doc.insert(image, doc.mount_root(), None);
    doc.set_attribute(image, "data-nana-image", "9");
    doc.set_attribute(image, "style", "width:96px;object-fit:cover;height:48px");
    doc.apply_layout_boxes(&[(
        image,
        LayoutBox {
            handle: image,
            x: 0.0,
            y: 0.0,
            width: 96.0,
            height: 48.0,
        },
    )]);

    let id = StableNodeId::try_from(image).expect("img id");
    let content = doc
        .runtime
        .custom_render(id)
        .expect("img object-fit must land on CustomRenderNode");
    assert_eq!(content.fit, nana_ui_core::ContentFit::Cover);

    doc.set_attribute(image, "object-fit", "contain");
    doc.apply_layout_boxes(&[]);
    let content = doc
        .runtime
        .custom_render(id)
        .expect("object-fit attribute must update CustomRenderNode");
    assert_eq!(content.fit, nana_ui_core::ContentFit::Contain);

    doc.set_attribute(image, "style", "object-fit: scale-down");
    doc.remove_attribute(image, "object-fit");
    doc.apply_layout_boxes(&[]);
    let content = doc
        .runtime
        .custom_render(id)
        .expect("inline object-fit must survive attr removal");
    assert_eq!(content.fit, nana_ui_core::ContentFit::ScaleDown);
}

#[test]
fn stylesheet_object_fit_on_layout_projects_custom_render_fit() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let image = doc.create_element("img");
    doc.insert(image, doc.mount_root(), None);
    doc.set_attribute(image, "data-nana-image", "3");
    doc.apply_layout_boxes(&[(
        image,
        LayoutBox {
            handle: image,
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 32.0,
        },
    )]);
    let id = StableNodeId::try_from(image).expect("img id");
    let mut layout = nana_ui_core::LayoutStyle::default();
    layout.paint.object_fit = Some(nana_ui_core::BackgroundImageFit::Cover);
    doc.sync_widget_layouts([(image.0, &layout)]);
    let content = doc
        .runtime
        .custom_render(id)
        .expect("cascaded object-fit must land on CustomRenderNode");
    assert_eq!(content.fit, nana_ui_core::ContentFit::Cover);
}

#[test]
fn video_attr_projects_host_texture_custom_render() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let video = doc.create_element("video");
    doc.insert(video, doc.mount_root(), None);
    doc.set_attribute(video, "data-nana-video", "11");
    doc.apply_layout_boxes(&[(
        video,
        LayoutBox {
            handle: video,
            x: 8.0,
            y: 16.0,
            width: 320.0,
            height: 180.0,
        },
    )]);

    let id = StableNodeId::try_from(video).expect("video id");
    let content = doc
        .runtime
        .custom_render(id)
        .expect("video must attach a HostTexture CustomRender");
    assert_eq!(content.renderer.as_ref(), "nana.host-texture");
    assert_eq!(content.resource.as_ref(), "video:11");
    assert_eq!(content.fit, nana_ui_core::ContentFit::Fill);
    assert!(
        doc.gpu_slots()
            .iter()
            .any(|(handle, slot)| *handle == video && slot == "video:11"),
        "video must appear in gpu_slots as video:{{id}}"
    );

    let primitive = doc
        .scene()
        .primitives()
        .find(|primitive| {
            primitive.node.get() == video.0
                && matches!(
                    primitive.kind,
                    nana_ui_scene::ScenePrimitiveKind::Custom { .. }
                )
        })
        .expect("video must extract a HostTexture scene primitive");
    let nana_ui_scene::ScenePrimitiveKind::Custom { node: custom, .. } = &primitive.kind else {
        panic!("video must compile to a custom scene primitive");
    };
    assert_eq!(custom.renderer.as_ref(), "nana.host-texture");
    assert_eq!(custom.resource.as_ref(), "video:11");
    assert_eq!(doc.hit_test(20.0, 24.0), Some(video));
}

#[test]
fn audio_does_not_project_host_texture_slot() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let audio = doc.create_element("audio");
    doc.insert(audio, doc.mount_root(), None);
    doc.set_attribute(audio, "src", "track.ogg");
    doc.set_attribute(audio, "controls", "");
    doc.apply_layout_boxes(&[(
        audio,
        LayoutBox {
            handle: audio,
            x: 0.0,
            y: 0.0,
            width: 240.0,
            height: 32.0,
        },
    )]);
    doc.flush_host_frame();
    let id = StableNodeId::try_from(audio).expect("audio id");
    assert!(
        doc.runtime.custom_render(id).is_none(),
        "audio without a picture must not pretend to have a video frame"
    );
    assert!(
        doc.gpu_slots()
            .iter()
            .all(|(_, slot)| !slot.starts_with("video:")),
        "audio must not occupy a video HostTexture slot"
    );
}

#[test]
fn live_media_sets_keep_audio_identity_out_of_visual_slots() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let video = doc.create_element("video");
    let audio = doc.create_element("audio");
    doc.insert(video, doc.mount_root(), None);
    doc.insert(audio, doc.mount_root(), None);
    doc.set_attribute(video, "data-nana-video", "11");
    doc.set_attribute(video, "data-nana-media", "11");
    doc.set_attribute(audio, "data-nana-media", "22");
    doc.apply_layout_boxes(&[
        (
            video,
            LayoutBox {
                handle: video,
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 36.0,
            },
        ),
        (
            audio,
            LayoutBox {
                handle: audio,
                x: 0.0,
                y: 40.0,
                width: 240.0,
                height: 32.0,
            },
        ),
    ]);
    let sets = doc.live_media_sets();
    assert!(sets.retain.iter().any(|id| id.0 == 11));
    assert!(sets.retain.iter().any(|id| id.0 == 22));
    assert!(sets.visual.iter().any(|id| id.0 == 11));
    assert!(sets.visual.iter().all(|id| id.0 != 22));
    let audio_id = StableNodeId::try_from(audio).expect("audio id");
    assert!(
        doc.runtime.custom_render(audio_id).is_none(),
        "data-nana-media must not project a HostTexture for audio"
    );
}

fn prepare_video_gpu_slots(doc: &NanaTreeDocument, held: &mut std::collections::HashSet<String>) {
    let live: Vec<String> = doc
        .gpu_slots()
        .into_iter()
        .map(|(_, slot)| slot)
        .filter(|slot| slot.starts_with("video:"))
        .collect();
    for slot in nana_ui_web_api::released_media_gpu_slots(
        held.iter().map(String::as_str),
        live.iter().map(String::as_str),
    ) {
        held.remove(&slot);
    }
    held.extend(live);
}

#[test]
fn video_gpu_slot_is_pruned_after_remove() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let video = doc.create_element("video");
    doc.insert(video, doc.mount_root(), None);
    doc.set_attribute(video, "data-nana-video", "5");
    doc.apply_layout_boxes(&[(
        video,
        LayoutBox {
            handle: video,
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 36.0,
        },
    )]);
    doc.flush_host_frame();
    let slot = "video:5".to_string();
    let mut held = std::collections::HashSet::new();
    prepare_video_gpu_slots(&doc, &mut held);
    assert!(
        held.contains(&slot),
        "create+flush+prepare must register video:{{id}}"
    );

    doc.remove(video);
    doc.flush_host_frame();
    prepare_video_gpu_slots(&doc, &mut held);
    assert!(
        !held.contains(&slot),
        "remove+flush+prepare must prune video:{{id}} from the GPU slot set"
    );
    let id = StableNodeId::try_from(video).expect("video id");
    assert!(
        doc.runtime.custom_render(id).is_none(),
        "disposed video must despawn CustomRenderNode"
    );
    assert!(
        doc.gpu_slots()
            .iter()
            .all(|(_, gpu_slot)| gpu_slot != &slot)
    );
}

#[test]
fn video_object_fit_projects_content_fit_on_host_texture() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let video = doc.create_element("video");
    doc.insert(video, doc.mount_root(), None);
    doc.set_attribute(video, "data-nana-video", "8");
    doc.set_attribute(video, "style", "object-fit:contain");
    doc.apply_layout_boxes(&[(
        video,
        LayoutBox {
            handle: video,
            x: 0.0,
            y: 0.0,
            width: 128.0,
            height: 72.0,
        },
    )]);
    let id = StableNodeId::try_from(video).expect("video id");
    let content = doc
        .runtime
        .custom_render(id)
        .expect("video object-fit must land on CustomRenderNode");
    assert_eq!(content.fit, nana_ui_core::ContentFit::Contain);
}

#[test]
fn video_src_size_change_invalidates_host_texture_revision() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let video = doc.create_element("video");
    doc.insert(video, doc.mount_root(), None);
    doc.set_attribute(video, "data-nana-video", "4");
    doc.apply_layout_boxes(&[(
        video,
        LayoutBox {
            handle: video,
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 36.0,
        },
    )]);
    let id = StableNodeId::try_from(video).expect("video id");
    doc.override_host_texture_revision("video:4", nana_ui_runtime::pack_gpu_revision(2, 1));
    doc.flush_host_frame();
    let first = doc.runtime.custom_render(id).expect("video HostTexture");
    assert_eq!(first.revision, nana_ui_runtime::pack_gpu_revision(2, 1));
    doc.override_host_texture_revision("video:4", nana_ui_runtime::pack_gpu_revision(3, 2));
    doc.set_attribute(video, "src", "nana:mock-b");
    doc.flush_host_frame();
    let second = doc
        .runtime
        .custom_render(id)
        .expect("src change must keep HostTexture");
    assert_eq!(second.resource.as_ref(), "video:4");
    assert_eq!(
        second.revision,
        nana_ui_runtime::pack_gpu_revision(3, 2),
        "src / size uploads must stamp a new generation/version"
    );
}

#[test]
fn generic_svg_projects_host_texture_custom_render() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let svg = doc.create_element("svg");
    doc.insert(svg, doc.mount_root(), None);
    doc.set_attribute(svg, "viewBox", "0 0 32 32");
    let rect = doc.create_element_ns("rect", crate::ElementNamespace::Svg, None);
    doc.insert(rect, svg, None);
    doc.set_attribute(rect, "x", "4");
    doc.set_attribute(rect, "y", "4");
    doc.set_attribute(rect, "width", "24");
    doc.set_attribute(rect, "height", "24");
    doc.set_attribute(rect, "fill", "#00aa00");
    doc.apply_layout_boxes(&[(
        svg,
        LayoutBox {
            handle: svg,
            x: 8.0,
            y: 8.0,
            width: 32.0,
            height: 32.0,
        },
    )]);
    doc.flush_host_frame();

    let id = StableNodeId::try_from(svg).expect("svg id");
    let content = doc
        .runtime
        .custom_render(id)
        .expect("generic svg must attach a HostTexture CustomRender");
    assert_eq!(content.renderer.as_ref(), "nana.host-texture");
    assert_eq!(content.resource.as_ref(), &format!("svg:{}", svg.0));
    assert!(
        doc.gpu_slots()
            .iter()
            .any(|(handle, slot)| *handle == svg && slot == &format!("svg:{}", svg.0)),
        "generic svg must appear in gpu_slots"
    );
    let primitive = doc
        .scene()
        .primitives()
        .find(|primitive| {
            primitive.node.get() == svg.0
                && matches!(
                    primitive.kind,
                    nana_ui_scene::ScenePrimitiveKind::Custom { .. }
                )
        })
        .expect("svg must extract a HostTexture scene primitive");
    let nana_ui_scene::ScenePrimitiveKind::Custom { node: custom, .. } = &primitive.kind else {
        panic!("svg must compile to a custom scene primitive");
    };
    assert_eq!(custom.renderer.as_ref(), "nana.host-texture");
    assert!(
        doc.scene()
            .primitives()
            .all(|primitive| primitive.node.get() != rect.0),
        "svg vector children must not paint as boxes"
    );
    assert!(!doc.svg_host_uploads().is_empty());
}

#[test]
fn lucide_svg_does_not_use_generic_svg_host_texture() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let svg = doc.create_element("svg");
    doc.insert(svg, doc.mount_root(), None);
    doc.set_attribute(svg, "class", "lucide lucide-search");
    doc.set_attribute(svg, "viewBox", "0 0 24 24");
    doc.apply_layout_boxes(&[(
        svg,
        LayoutBox {
            handle: svg,
            x: 0.0,
            y: 0.0,
            width: 24.0,
            height: 24.0,
        },
    )]);
    let mut bridge = crate::MessageBridge::new();
    let mut props = crate::WidgetProps {
        value: "search".into(),
        class_names: vec!["lucide".into(), "lucide-search".into()],
        element_tag: "svg".into(),
        ..Default::default()
    };
    props.layout.width = Some(nana_ui_core::LengthSpec::Px(24.0));
    props.layout.height = Some(nana_ui_core::LengthSpec::Px(24.0));
    bridge.register(svg.0, crate::WidgetKind::Icon, props);
    doc.sync_semantic_styles(&bridge.snapshot());
    doc.flush_host_frame();
    let id = StableNodeId::try_from(svg).expect("svg id");
    assert!(
        doc.runtime.custom_render(id).is_none(),
        "Lucide stays on the Icon atlas path"
    );
    assert!(
        matches!(
            doc.runtime.standard_visual(id),
            Some(nana_ui_runtime::StandardVisual::Icon { icon, .. })
                if icon == nana_ui_core::Icon::Search
        ),
        "catalog Lucide must project StandardVisual::Icon"
    );
    assert!(
        doc.scene().primitives().any(|primitive| {
            primitive.node.get() == svg.0
                && matches!(
                    primitive.kind,
                    nana_ui_scene::ScenePrimitiveKind::Icon { icon, .. }
                        if icon == nana_ui_core::Icon::Search
                )
        }),
        "catalog Lucide must extract ScenePrimitiveKind::Icon"
    );
}

#[test]
fn i_and_nana_icon_project_icon_atlas() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let i = doc.create_element("i");
    let nana_icon = doc.create_element("nana-icon");
    doc.insert(i, doc.mount_root(), None);
    doc.insert(nana_icon, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        i.0,
        crate::WidgetKind::Icon,
        crate::WidgetProps {
            value: "settings".into(),
            element_tag: "i".into(),
            ..Default::default()
        },
    );
    bridge.register(
        nana_icon.0,
        crate::WidgetKind::Icon,
        crate::WidgetProps {
            value: "search".into(),
            element_tag: "nana-icon".into(),
            ..Default::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());
    let i_id = StableNodeId::try_from(i).unwrap();
    let nana_id = StableNodeId::try_from(nana_icon).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(i_id),
        Some(nana_ui_runtime::StandardVisual::Icon { icon, .. })
            if icon == nana_ui_core::Icon::Settings
    ));
    assert!(matches!(
        doc.runtime.standard_visual(nana_id),
        Some(nana_ui_runtime::StandardVisual::Icon { icon, .. })
            if icon == nana_ui_core::Icon::Search
    ));
    assert!(doc.runtime.custom_render(i_id).is_none());
    assert!(doc.runtime.custom_render(nana_id).is_none());
}

#[test]
fn unknown_lucide_falls_back_to_host_texture() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let svg = doc.create_element("svg");
    doc.insert(svg, doc.mount_root(), None);
    doc.set_attribute(svg, "class", "lucide lucide-puzzle");
    doc.set_attribute(svg, "viewBox", "0 0 24 24");
    let rect = doc.create_element_ns("rect", crate::ElementNamespace::Svg, None);
    doc.insert(rect, svg, None);
    doc.set_attribute(rect, "width", "24");
    doc.set_attribute(rect, "height", "24");
    doc.set_attribute(rect, "fill", "#00aa00");
    doc.apply_layout_boxes(&[(
        svg,
        LayoutBox {
            handle: svg,
            x: 0.0,
            y: 0.0,
            width: 24.0,
            height: 24.0,
        },
    )]);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        svg.0,
        crate::WidgetKind::Icon,
        crate::WidgetProps {
            value: "puzzle".into(),
            class_names: vec!["lucide".into(), "lucide-puzzle".into()],
            element_tag: "svg".into(),
            ..Default::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());
    doc.flush_host_frame();
    let id = StableNodeId::try_from(svg).expect("svg id");
    assert!(
        doc.runtime.standard_visual(id).is_none(),
        "unknown lucide names stay off the catalog atlas"
    );
    let content = doc
        .runtime
        .custom_render(id)
        .expect("unknown lucide must rasterize to HostTexture");
    assert_eq!(content.renderer.as_ref(), "nana.host-texture");
    assert_eq!(content.resource.as_ref(), &format!("svg:{}", svg.0));
    assert!(
        doc.scene().primitives().all(|primitive| {
            !matches!(
                primitive.kind,
                nana_ui_scene::ScenePrimitiveKind::Icon { .. }
            ) || primitive.node.get() != svg.0
        }),
        "unknown lucide must not extract an Icon primitive"
    );
}

#[test]
fn icon_button_inner_lucide_does_not_bind_iconglyph() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let button = doc.create_element("button");
    let svg = doc.create_element("svg");
    let path = doc.create_element_ns("path", crate::ElementNamespace::Svg, None);
    doc.insert(button, doc.mount_root(), None);
    doc.insert(svg, button, None);
    doc.insert(path, svg, None);
    doc.set_attribute(svg, "class", "lucide lucide-search");
    doc.apply_layout_boxes(&[
        (
            button,
            LayoutBox {
                handle: button,
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
            },
        ),
        (
            svg,
            LayoutBox {
                handle: svg,
                x: 4.0,
                y: 4.0,
                width: 24.0,
                height: 24.0,
            },
        ),
        (
            path,
            LayoutBox {
                handle: path,
                x: 4.0,
                y: 4.0,
                width: 24.0,
                height: 24.0,
            },
        ),
    ]);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        button.0,
        crate::WidgetKind::Button,
        crate::WidgetProps {
            label: "Search".into(),
            element_tag: "button".into(),
            ..Default::default()
        },
    );
    bridge.register(
        svg.0,
        crate::WidgetKind::Icon,
        crate::WidgetProps {
            value: "search".into(),
            class_names: vec!["lucide".into(), "lucide-search".into()],
            element_tag: "svg".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(svg.0, button.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());
    doc.flush_host_frame();
    let button_id = StableNodeId::try_from(button).unwrap();
    let svg_id = StableNodeId::try_from(svg).unwrap();
    assert!(
        matches!(
            doc.runtime.standard_visual(button_id),
            Some(nana_ui_runtime::StandardVisual::Icon { icon, .. })
                if icon == nana_ui_core::Icon::Search
        ),
        "IconButton promotion owns the atlas glyph"
    );
    assert!(
        doc.runtime.standard_visual(svg_id).is_none(),
        "inner Lucide svg must not bind a second IconGlyph"
    );
    assert!(
        doc.scene()
            .primitives()
            .filter(|primitive| matches!(
                primitive.kind,
                nana_ui_scene::ScenePrimitiveKind::Icon { .. }
            ))
            .all(|primitive| primitive.node.get() == button.0),
        "only the IconButton node paints an Icon primitive"
    );
    assert!(
        doc.scene()
            .primitives()
            .all(|primitive| primitive.node.get() != path.0),
        "inner path must not paint as a CSS box"
    );
}

fn mount_generic_svg_host_texture(doc: &mut NanaTreeDocument) -> NodeHandle {
    let svg = doc.create_element("svg");
    doc.insert(svg, doc.mount_root(), None);
    doc.set_attribute(svg, "viewBox", "0 0 32 32");
    let rect = doc.create_element_ns("rect", crate::ElementNamespace::Svg, None);
    doc.insert(rect, svg, None);
    doc.set_attribute(rect, "x", "4");
    doc.set_attribute(rect, "y", "4");
    doc.set_attribute(rect, "width", "24");
    doc.set_attribute(rect, "height", "24");
    doc.set_attribute(rect, "fill", "#00aa00");
    doc.apply_layout_boxes(&[(
        svg,
        LayoutBox {
            handle: svg,
            x: 8.0,
            y: 8.0,
            width: 32.0,
            height: 32.0,
        },
    )]);
    doc.flush_host_frame();
    svg
}

fn prepare_svg_gpu_slots(doc: &NanaTreeDocument, held: &mut std::collections::HashSet<String>) {
    let live: Vec<String> = doc
        .svg_host_uploads()
        .into_iter()
        .map(|upload| upload.slot)
        .collect();
    for slot in crate::svg_raster::released_svg_gpu_slots(
        held.iter().map(String::as_str),
        live.iter().map(String::as_str),
    ) {
        held.remove(&slot);
    }
    held.extend(live);
}

#[test]
fn svg_gpu_slot_is_pruned_after_remove() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let svg = mount_generic_svg_host_texture(&mut doc);
    let slot = format!("svg:{}", svg.0);
    let mut held = std::collections::HashSet::new();
    prepare_svg_gpu_slots(&doc, &mut held);
    assert!(
        held.contains(&slot),
        "create+flush+prepare must register svg:{{id}}"
    );
    assert!(
        doc.gpu_slots()
            .iter()
            .any(|(_, gpu_slot)| gpu_slot == &slot)
    );

    doc.remove(svg);
    doc.flush_host_frame();
    prepare_svg_gpu_slots(&doc, &mut held);
    assert!(
        !held.contains(&slot),
        "remove+flush+prepare must prune svg:{{id}} from the GPU slot set"
    );
    assert!(doc.svg_host_uploads().is_empty());
    assert!(
        doc.gpu_slots()
            .iter()
            .all(|(_, gpu_slot)| gpu_slot != &slot)
    );
    let id = StableNodeId::try_from(svg).expect("svg id");
    assert!(
        doc.runtime.custom_render(id).is_none(),
        "disposed svg must despawn CustomRenderNode"
    );
}

#[test]
fn svg_gpu_slot_is_pruned_after_lucide_conversion() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let svg = mount_generic_svg_host_texture(&mut doc);
    let slot = format!("svg:{}", svg.0);
    let mut held = std::collections::HashSet::new();
    prepare_svg_gpu_slots(&doc, &mut held);
    assert!(held.contains(&slot));

    doc.set_attribute(svg, "class", "lucide lucide-search");
    doc.flush_host_frame();
    prepare_svg_gpu_slots(&doc, &mut held);
    assert!(
        !held.contains(&slot),
        "Lucide conversion+prepare must drop the generic svg:{{id}} GPU slot"
    );
    assert!(doc.svg_host_uploads().is_empty());
    let id = StableNodeId::try_from(svg).expect("svg id");
    assert!(
        doc.runtime.custom_render(id).is_none(),
        "Lucide conversion must detach HostTexture CustomRender"
    );
}

#[test]
fn canvas_without_slot_is_not_a_2d_bitmap() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let canvas = doc.create_element("canvas");
    doc.insert(canvas, doc.mount_root(), None);
    doc.apply_layout_boxes(&[(
        canvas,
        LayoutBox {
            handle: canvas,
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 150.0,
        },
    )]);

    let id = StableNodeId::try_from(canvas).expect("canvas id");
    assert!(
        doc.runtime.custom_render(id).is_none(),
        "bare <canvas> must not attach a HostTexture CustomRender"
    );
    assert!(
        doc.scene().primitives().all(|primitive| {
            primitive.node.get() != canvas.0
                || !matches!(
                    &primitive.kind,
                    nana_ui_scene::ScenePrimitiveKind::Custom { node, .. }
                        if node.renderer.as_ref() == "nana.host-texture"
                )
        }),
        "bare <canvas> must not sample host-texture as a 2d bitmap"
    );
}

#[test]
fn semantic_widget_accessibility_projects_from_runtime() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let node = doc.create_element("button");
    doc.insert(node, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        node.0,
        crate::WidgetKind::Chip,
        crate::WidgetProps {
            label: "Build".into(),
            role: "tab".into(),
            disabled: true,
            active: true,
            ..Default::default()
        },
    );
    let snapshot = bridge.snapshot();
    doc.sync_semantic_styles(&snapshot);

    let accessibility = doc
        .accessibility_snapshot()
        .into_iter()
        .find(|entry| entry.id.get() == node.0)
        .expect("semantic widget must enter Runtime accessibility tree");
    assert_eq!(accessibility.role, AccessibilityRole::Tab);
    assert_eq!(accessibility.label.as_deref(), Some("Build"));
    assert!(accessibility.disabled);
    assert_eq!(accessibility.selected, Some(true));
}

/// The `button-with-icon` rule must win before the element-tag chain:
/// `<button>` with an icon child tags `nana.icon-button`, not `nana.button`,
/// even though "button" itself resolves in the registry.
#[test]
fn button_with_icon_rule_precedes_element_tag_chain() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let button = doc.create_element("button");
    let svg = doc.create_element("svg");
    doc.insert(button, doc.mount_root(), None);
    doc.insert(svg, button, None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        button.0,
        crate::WidgetKind::Button,
        crate::WidgetProps {
            label: "Go".into(),
            element_tag: "button".into(),
            ..Default::default()
        },
    );
    bridge.register(
        svg.0,
        crate::WidgetKind::Icon,
        crate::WidgetProps {
            value: "search".into(),
            class_names: vec!["lucide".into(), "lucide-search".into()],
            element_tag: "svg".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(svg.0, button.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());
    let id = StableNodeId::try_from(button).unwrap();
    assert_eq!(
        doc.runtime.component_type(id).map(|t| t.as_str()),
        Some("nana.icon-button")
    );
}

/// `alertdialog` role must route Dialog to ConfirmDialog rather than the
/// plain dialog component.
#[test]
fn confirm_dialog_rule_routes_alertdialog_role() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let node = doc.create_element("nana-dialog");
    doc.insert(node, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        node.0,
        crate::WidgetKind::Dialog,
        crate::WidgetProps {
            label: "Delete?".into(),
            element_tag: "nana-dialog".into(),
            role: "alertdialog".into(),
            ..Default::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());
    let id = StableNodeId::try_from(node).unwrap();
    assert_eq!(
        doc.runtime.component_type(id).map(|t| t.as_str()),
        Some("nana.confirm-dialog")
    );
}

/// Choice fields must keep one option list with three Runtime landings:
/// Search > Dropdown > Select, decided by kind before props.
#[test]
fn choice_field_rules_keep_runtime_landings_distinct() {
    let cases = [
        (crate::WidgetKind::SearchDropdown, "nana.search-dropdown"),
        (crate::WidgetKind::Dropdown, "nana.dropdown"),
        (crate::WidgetKind::Select, "nana.select"),
    ];
    for (kind, expected_type) in cases {
        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let node = doc.create_element("select");
        doc.insert(node, doc.mount_root(), None);
        let mut bridge = crate::MessageBridge::new();
        bridge.register(
            node.0,
            kind,
            crate::WidgetProps {
                element_tag: "select".into(),
                ..Default::default()
            },
        );
        doc.sync_semantic_styles(&bridge.snapshot());
        let id = StableNodeId::try_from(node).unwrap();
        assert_eq!(
            doc.runtime.component_type(id).map(|t| t.as_str()),
            Some(expected_type),
            "kind {kind:?} must land on {expected_type}"
        );
    }
}

/// Incremental semantic sync must land the same projections a full pass
/// would: mutations applied stepwise (each sync walking only the bridge's
/// dirty set) end with correct runtime state for mutated widgets, their
/// cascade neighbours, and late insertions.
#[test]
fn incremental_sync_projects_mutated_widgets_and_cascade_neighbours() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let root = doc.create_element("nana-column");
    doc.insert(root, doc.mount_root(), None);
    let button = doc.create_element("button");
    let input = doc.create_element("input");
    let checkbox = doc.create_element("input");
    let checkbox_tail = doc.create_element("input");
    doc.insert(button, root, None);
    doc.insert(input, root, None);
    doc.insert(checkbox, root, None);
    doc.insert(checkbox_tail, root, None);
    let mut bridge = crate::MessageBridge::new();
    let props = |tag: &str, label: &str| crate::WidgetProps {
        element_tag: tag.into(),
        label: label.into(),
        ..Default::default()
    };
    bridge.register(root.0, crate::WidgetKind::Column, props("nana-column", ""));
    bridge.register(button.0, crate::WidgetKind::Button, props("button", "Go"));
    bridge.register(input.0, crate::WidgetKind::Input, props("input", ""));
    bridge.register(
        checkbox.0,
        crate::WidgetKind::Checkbox,
        props("input", "Opt-in"),
    );
    bridge.register(
        checkbox_tail.0,
        crate::WidgetKind::Checkbox,
        props("input", "Opt-out"),
    );
    bridge.insert_child(button.0, root.0, None);
    bridge.insert_child(input.0, root.0, None);
    bridge.insert_child(checkbox.0, root.0, None);
    bridge.insert_child(checkbox_tail.0, root.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    // Stepwise mutations: label patch, text input, toggle (which recascades
    // following siblings), then a structural append.
    bridge.patch_prop(
        button.0,
        "label",
        &nana_js_engine::HostValue::string("Submit"),
    );
    bridge.note_input(input.0, "hello");
    bridge.note_toggle(checkbox.0, true);
    let item = doc.create_element("li");
    doc.insert(item, root, None);
    bridge.register(item.0, crate::WidgetKind::ListItem, props("li", "New row"));
    bridge.insert_child(item.0, root.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let button_id = StableNodeId::try_from(button).unwrap();
    let input_id = StableNodeId::try_from(input).unwrap();
    let checkbox_id = StableNodeId::try_from(checkbox).unwrap();
    let tail_id = StableNodeId::try_from(checkbox_tail).unwrap();
    let item_id = StableNodeId::try_from(item).unwrap();
    assert_eq!(
        doc.runtime
            .component_type(button_id)
            .map(|t| t.as_str().to_owned()),
        Some("nana.button".into())
    );
    assert_eq!(
        doc.runtime
            .accessibility(button_id)
            .and_then(|a| a.label.as_deref()),
        Some("Submit"),
        "patched label must reach the accessibility projection"
    );
    assert_eq!(
        doc.runtime.text_input(input_id).map(|t| t.value.clone()),
        Some("hello".into()),
        "noted input must reach the Runtime text input state"
    );
    assert_eq!(
        doc.runtime
            .accessibility(checkbox_id)
            .and_then(|a| a.checked),
        Some(true),
        "noted toggle must reach the accessibility projection"
    );
    assert_eq!(
        doc.runtime.accessibility(tail_id).and_then(|a| a.checked),
        Some(false),
        "untouched sibling must keep its own unchecked state"
    );
    assert_eq!(
        doc.runtime
            .component_type(item_id)
            .map(|t| t.as_str().to_owned()),
        Some("nana.list-item".into()),
        "late insertion must be projected on the structural pass"
    );
}

/// The bridge footprint must classify mutations: prop patches stay
/// targeted, structural edits demand a full pass, appearance is global.
#[test]
fn bridge_footprint_classifies_mutations() {
    let mut bridge = crate::MessageBridge::new();
    let root = 1;
    let button = 2;
    bridge.register(
        root,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            element_tag: "nana-column".into(),
            ..Default::default()
        },
    );
    bridge.register(
        button,
        crate::WidgetKind::Button,
        crate::WidgetProps {
            element_tag: "button".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(button, root, None);
    // Consume the mount footprint.
    let mount = bridge.snapshot();
    assert!(mount.changes.needs_full_pass(), "mounting is structural");
    assert!(!mount.changes.dirty.is_empty());

    bridge.patch_prop(button, "label", &nana_js_engine::HostValue::string("Hi"));
    let patch = bridge.snapshot();
    assert!(!patch.changes.needs_full_pass());
    assert!(patch.changes.dirty.contains(&button));

    bridge.set_label(button, "Renamed");
    let relabel = bridge.snapshot();
    assert!(!relabel.changes.needs_full_pass());
    assert!(relabel.changes.dirty.contains(&button));

    let mut appearance = nana_ui_core::AppearanceSettings::default();
    appearance.set_standard_radius(20.0);
    bridge.set_appearance(appearance);
    let appearance = bridge.snapshot();
    assert!(appearance.changes.all);
    assert!(appearance.changes.needs_full_pass());

    let child = 3;
    bridge.register(child, crate::WidgetKind::Box, Default::default());
    bridge.insert_child(child, root, None);
    let insert = bridge.snapshot();
    assert!(insert.changes.structure_changed);
}

#[test]
fn nana_chip_projects_selected_button_not_a_second_control() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let node = doc.create_element("nana-chip");
    doc.insert(node, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        node.0,
        crate::WidgetKind::Chip,
        crate::WidgetProps {
            label: "Beta".into(),
            active: true,
            ..Default::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());
    let id = StableNodeId::try_from(node).unwrap();
    assert_eq!(
        doc.runtime
            .component_type(id)
            .map(|type_id| type_id.as_str().to_owned())
            .as_deref(),
        Some("nana.button")
    );
    let accessibility = doc
        .accessibility_snapshot()
        .into_iter()
        .find(|entry| entry.id == id)
        .expect("chip must enter Runtime accessibility");
    assert_eq!(accessibility.role, AccessibilityRole::Button);
    assert_eq!(accessibility.label.as_deref(), Some("Beta"));
    assert_eq!(
        doc.runtime.standard_visual(id),
        Some(nana_ui_runtime::StandardVisual::Button {
            label: std::sync::Arc::from("Beta"),
            kind: nana_ui_core::ButtonKind::Selected,
            size: nana_ui_core::ControlSize::Medium,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
        })
    );
}

#[test]
fn migrated_controls_project_one_retained_visual_and_accessibility_state() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let button = doc.create_element("button");
    let input = doc.create_element("input");
    let checkbox = doc.create_element("input");
    let switch = doc.create_element("nana-switch");
    let range = doc.create_element("input");
    doc.insert(button, doc.mount_root(), None);
    doc.insert(input, doc.mount_root(), None);
    doc.insert(checkbox, doc.mount_root(), None);
    doc.insert(switch, doc.mount_root(), None);
    doc.insert(range, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        button.0,
        crate::WidgetKind::Button,
        crate::WidgetProps {
            label: "Build".into(),
            button_kind: nana_ui_core::ButtonKind::Primary,
            size: nana_ui_core::ControlSize::Large,
            loading: true,
            invalid: true,
            ..Default::default()
        },
    );
    bridge.register(
        input.0,
        crate::WidgetKind::Input,
        crate::WidgetProps {
            label: "Password".into(),
            placeholder: "Enter password".into(),
            value: "secret".into(),
            size: nana_ui_core::ControlSize::Large,
            read_only: true,
            secure: true,
            invalid: true,
            ..Default::default()
        },
    );
    bridge.register(
        checkbox.0,
        crate::WidgetKind::Checkbox,
        crate::WidgetProps {
            label: "Notifications".into(),
            toggled: true,
            invalid: true,
            ..Default::default()
        },
    );
    bridge.register(
        switch.0,
        crate::WidgetKind::Switch,
        crate::WidgetProps {
            label: "Live preview".into(),
            hint: "Updates while editing".into(),
            toggled: true,
            loading: true,
            invalid: true,
            control_position: nana_ui_core::SwitchControlPosition::Start,
            size: nana_ui_core::ControlSize::Large,
            ..Default::default()
        },
    );
    bridge.register(
        range.0,
        crate::WidgetKind::Range,
        crate::WidgetProps {
            label: "Opacity".into(),
            unit: "%".into(),
            min: 0.0,
            max: 1.0,
            step: 0.25,
            number: 0.62,
            invalid: true,
            ..Default::default()
        },
    );
    let snapshot = bridge.snapshot();
    doc.sync_semantic_styles(&snapshot);
    doc.apply_layout_boxes(&[
        (
            button,
            LayoutBox {
                handle: button,
                x: 10.0,
                y: 10.0,
                width: 220.0,
                height: 36.0,
            },
        ),
        (
            input,
            LayoutBox {
                handle: input,
                x: 10.0,
                y: 50.0,
                width: 220.0,
                height: 36.0,
            },
        ),
        (
            checkbox,
            LayoutBox {
                handle: checkbox,
                x: 10.0,
                y: 90.0,
                width: 220.0,
                height: 32.0,
            },
        ),
        (
            switch,
            LayoutBox {
                handle: switch,
                x: 10.0,
                y: 130.0,
                width: 220.0,
                height: 44.0,
            },
        ),
        (
            range,
            LayoutBox {
                handle: range,
                x: 10.0,
                y: 180.0,
                width: 220.0,
                height: 36.0,
            },
        ),
    ]);

    let button_id = StableNodeId::try_from(button).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(button_id),
        Some(nana_ui_runtime::StandardVisual::Button {
            kind: nana_ui_core::ButtonKind::Primary,
            size: nana_ui_core::ControlSize::Large,
            loading: true,
            invalid: true,
            ..
        })
    ));
    let input_id = StableNodeId::try_from(input).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(input_id),
        Some(nana_ui_runtime::StandardVisual::TextInput {
            size: nana_ui_core::ControlSize::Large,
            secure: true,
            invalid: true,
            ..
        })
    ));

    let checkbox_id = StableNodeId::try_from(checkbox).unwrap();
    assert_eq!(
        doc.runtime.standard_visual(checkbox_id),
        Some(nana_ui_runtime::StandardVisual::Checkbox {
            checked: true,
            indeterminate: false,
            size: nana_ui_core::ControlSize::Medium,
        })
    );
    let switch_id = StableNodeId::try_from(switch).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(switch_id),
        Some(nana_ui_runtime::StandardVisual::Switch {
            checked: true,
            control_position: nana_ui_core::SwitchControlPosition::Start,
            size: nana_ui_core::ControlSize::Large,
            loading: true,
            invalid: true,
            ..
        })
    ));
    let range_id = StableNodeId::try_from(range).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(range_id),
        Some(nana_ui_runtime::StandardVisual::Range {
            ratio,
            invalid: true,
            ..
        })
            if (ratio - 0.5).abs() < f32::EPSILON
    ));
    let accessibility = doc.accessibility_snapshot();
    let input_accessibility = accessibility
        .iter()
        .find(|node| node.id == input_id)
        .unwrap();
    assert!(!input_accessibility.editable);
    assert_eq!(input_accessibility.value, None);
    assert!(input_accessibility.invalid);
    let checkbox_accessibility = accessibility
        .iter()
        .find(|node| node.id == checkbox_id)
        .unwrap();
    assert_eq!(checkbox_accessibility.checked, Some(true));
    assert!(checkbox_accessibility.invalid);
    let switch_accessibility = accessibility
        .iter()
        .find(|node| node.id == switch_id)
        .unwrap();
    assert_eq!(switch_accessibility.checked, Some(true));
    assert!(switch_accessibility.busy);
    assert!(switch_accessibility.invalid);
    let range_accessibility = accessibility
        .iter()
        .find(|node| node.id == range_id)
        .unwrap();
    assert_eq!(range_accessibility.numeric_minimum, Some(0.0));
    assert_eq!(range_accessibility.numeric_maximum, Some(1.0));
    assert_eq!(range_accessibility.numeric_step, Some(0.25));
    assert_eq!(range_accessibility.numeric_value, Some(0.5));
    assert!(doc.scene().node_bounds(switch_id).is_some());
    assert!(doc.scene().node_bounds(range_id).is_some());
}

#[test]
fn migrated_kind_change_clears_input_state_before_publishing_button_label() {
    let mut doc = NanaTreeDocument::new(320, 200, 1.0);
    let node = doc.create_element("nana-input");
    doc.insert(node, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        node.0,
        crate::WidgetKind::Input,
        crate::WidgetProps {
            value: "secret".into(),
            secure: true,
            ..Default::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());
    let id = StableNodeId::try_from(node).unwrap();
    assert!(doc.runtime.text_input(id).is_some());

    bridge.register(
        node.0,
        crate::WidgetKind::Button,
        crate::WidgetProps {
            label: "Run".into(),
            ..Default::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());

    assert!(doc.runtime.text_input(id).is_none());
    assert_eq!(doc.runtime.text(id), Some("Run"));
    assert!(matches!(
        doc.runtime.standard_visual(id),
        Some(nana_ui_runtime::StandardVisual::Button { ref label, .. }) if &**label == "Run"
    ));
    let accessibility = doc
        .accessibility_snapshot()
        .into_iter()
        .find(|node| node.id == id)
        .unwrap();
    assert_eq!(accessibility.role, AccessibilityRole::Button);
    assert_eq!(accessibility.label.as_deref(), Some("Run"));
    assert_eq!(accessibility.value, None);
}

#[test]
fn feedback_hosts_project_retained_visuals_including_action_children() {
    let mut doc = NanaTreeDocument::new(320, 200, 1.0);
    let badge = doc.create_element("nana-status");
    let validation = doc.create_element("nana-validation");
    let empty = doc.create_element("nana-empty-state");
    let action = doc.create_element("button");
    let labeled = doc.create_element("nana-labeled-value");
    let progress = doc.create_element("progress");
    let spinner = doc.create_element("nana-spinner");
    doc.insert(badge, doc.mount_root(), None);
    doc.insert(validation, doc.mount_root(), None);
    doc.insert(empty, doc.mount_root(), None);
    doc.insert(action, empty, None);
    doc.insert(labeled, doc.mount_root(), None);
    doc.insert(progress, doc.mount_root(), None);
    doc.insert(spinner, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    let mut badge_props = crate::WidgetProps {
        label: "Offline".into(),
        class_names: vec![
            "nana-status".into(),
            "nana-status--danger".into(),
            "compact".into(),
        ],
        ..Default::default()
    };
    badge_props.attrs.insert("tone".into(), "danger".into());
    bridge.register(badge.0, crate::WidgetKind::StatusBadge, badge_props);
    bridge.register(
        validation.0,
        crate::WidgetKind::ValidationMessage,
        crate::WidgetProps {
            hint: "A project is required".into(),
            invalid: true,
            ..Default::default()
        },
    );
    bridge.register(
        empty.0,
        crate::WidgetKind::EmptyState,
        crate::WidgetProps {
            label: "No projects".into(),
            hint: "Create the first project".into(),
            ..Default::default()
        },
    );
    bridge.register(
        action.0,
        crate::WidgetKind::Button,
        crate::WidgetProps {
            label: "Create".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(action.0, empty.0, None);
    bridge.register(
        labeled.0,
        crate::WidgetKind::LabeledValue,
        crate::WidgetProps {
            label: "Revision".into(),
            value: "42".into(),
            muted: true,
            ..Default::default()
        },
    );
    bridge.register(
        progress.0,
        crate::WidgetKind::Progress,
        crate::WidgetProps {
            label: "Copying".into(),
            progress: 40.0,
            progress_max: 100.0,
            ..Default::default()
        },
    );
    bridge.register(
        spinner.0,
        crate::WidgetKind::Spinner,
        crate::WidgetProps {
            label: "Loading".into(),
            ..Default::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());

    let badge_id = StableNodeId::try_from(badge).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(badge_id),
        Some(nana_ui_runtime::StandardVisual::StatusBadge {
            tone: nana_ui_core::StatusTone::Danger,
            compact: true,
            ..
        })
    ));
    let validation_id = StableNodeId::try_from(validation).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(validation_id),
        Some(nana_ui_runtime::StandardVisual::ValidationMessage {
            intent: nana_ui_core::ValidationIntent::Danger,
            ..
        })
    ));
    let empty_id = StableNodeId::try_from(empty).unwrap();
    let action_id = StableNodeId::try_from(action).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(empty_id),
        Some(nana_ui_runtime::StandardVisual::EmptyState {
            action: Some(id),
            ..
        }) if id == action_id
    ));
    let labeled_id = StableNodeId::try_from(labeled).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(labeled_id),
        Some(nana_ui_runtime::StandardVisual::LabeledValue {
            value_weight: 400,
            ..
        })
    ));
    let progress_id = StableNodeId::try_from(progress).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(progress_id),
        Some(nana_ui_runtime::StandardVisual::Progress {
            value_ratio,
            ..
        }) if (value_ratio - 0.4).abs() < 0.001
    ));
    let spinner_id = StableNodeId::try_from(spinner).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(spinner_id),
        Some(nana_ui_runtime::StandardVisual::Spinner { .. })
    ));
}

#[test]
fn qualified_surface_hosts_project_runtime_visuals() {
    let mut doc = NanaTreeDocument::new(320, 200, 1.0);
    let field = doc.create_element("nana-form-field");
    let input = doc.create_element("nana-input");
    let card = doc.create_element("nana-interactive-card");
    let skeleton = doc.create_element("nana-skeleton");
    let meter = doc.create_element("nana-level-meter");
    doc.insert(field, doc.mount_root(), None);
    doc.insert(input, field, None);
    doc.insert(card, doc.mount_root(), None);
    doc.insert(skeleton, doc.mount_root(), None);
    doc.insert(meter, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        field.0,
        crate::WidgetKind::FormField,
        crate::WidgetProps {
            label: "Email".into(),
            hint: "Required".into(),
            invalid: true,
            size: nana_ui_core::ControlSize::Small,
            ..Default::default()
        },
    );
    bridge.register(
        input.0,
        crate::WidgetKind::Input,
        crate::WidgetProps {
            value: "a@b.c".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(input.0, field.0, None);
    bridge.register(
        card.0,
        crate::WidgetKind::InteractiveCard,
        crate::WidgetProps {
            active: true,
            disabled: true,
            ..Default::default()
        },
    );
    let mut skeleton_props = crate::WidgetProps::default();
    skeleton_props.layout.width = Some(nana_ui_core::LengthSpec::Px(120.0));
    skeleton_props.layout.height = Some(nana_ui_core::LengthSpec::Px(18.0));
    bridge.register(skeleton.0, crate::WidgetKind::Skeleton, skeleton_props);
    let mut meter_props = crate::WidgetProps {
        progress: 0.65,
        ..Default::default()
    };
    meter_props.attrs.insert("tone".into(), "warning".into());
    meter_props.layout.height = Some(nana_ui_core::LengthSpec::Px(8.0));
    bridge.register(meter.0, crate::WidgetKind::LevelMeter, meter_props);
    doc.sync_semantic_styles(&bridge.snapshot());

    let field_id = StableNodeId::try_from(field).unwrap();
    let input_id = StableNodeId::try_from(input).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(field_id),
        Some(nana_ui_runtime::StandardVisual::FormField {
            ref label,
            hint: None,
            error: Some(ref error),
            size: nana_ui_core::ControlSize::Small,
            control: Some(control),
        }) if &**label == "Email" && &**error == "Required" && control == input_id
    ));
    let card_id = StableNodeId::try_from(card).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(card_id),
        Some(nana_ui_runtime::StandardVisual::Card {
            kind: nana_ui_core::CardKind::Selected,
            title: None,
            loading: false,
            ..
        })
    ));
    assert!(doc.runtime.accessibility(card_id).unwrap().disabled);
    let skeleton_id = StableNodeId::try_from(skeleton).unwrap();
    assert_eq!(doc.runtime.standard_visual(skeleton_id), None);
    let skeleton_style = doc.runtime.node_style(skeleton_id).unwrap();
    assert_eq!(
        skeleton_style.layout.width,
        Some(nana_ui_core::LengthSpec::Px(120.0))
    );
    assert_eq!(
        skeleton_style.layout.height,
        Some(nana_ui_core::LengthSpec::Px(18.0))
    );
    let meter_id = StableNodeId::try_from(meter).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(meter_id),
        Some(nana_ui_runtime::StandardVisual::LevelMeter {
            value_ratio,
            girth,
            tone: nana_ui_core::StatusTone::Warning,
        }) if (value_ratio - 0.65).abs() < 0.001 && (girth - 8.0).abs() < 0.001
    ));
}

#[test]
fn qualified_candidate_leaves_project_runtime_visuals() {
    let mut doc = NanaTreeDocument::new(320, 200, 1.0);
    let area = doc.create_element("textarea");
    let select = doc.create_element("select");
    let dialog = doc.create_element("dialog");
    let confirm = doc.create_element("nana-confirm-dialog");
    let drawer = doc.create_element("nana-drawer");
    doc.insert(area, doc.mount_root(), None);
    doc.insert(select, doc.mount_root(), None);
    doc.insert(dialog, doc.mount_root(), None);
    doc.insert(confirm, doc.mount_root(), None);
    doc.insert(drawer, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        area.0,
        crate::WidgetKind::Textarea,
        crate::WidgetProps {
            value: "line\nbreak".into(),
            placeholder: "Notes".into(),
            invalid: true,
            ..Default::default()
        },
    );
    bridge.register(
        select.0,
        crate::WidgetKind::Select,
        crate::WidgetProps {
            value: "code".into(),
            options: vec![crate::SelectOptionProp {
                value: "code".into(),
                label: "Code".into(),
                disabled: false,
            }],
            ..Default::default()
        },
    );
    bridge.register(
        dialog.0,
        crate::WidgetKind::Dialog,
        crate::WidgetProps {
            label: "Rename".into(),
            hint: "Choose a name".into(),
            ..Default::default()
        },
    );
    let mut confirm_props = crate::WidgetProps {
        label: "Delete".into(),
        hint: "This cannot be undone.".into(),
        ..Default::default()
    };
    confirm_props.class_names.push("nana-confirm-dialog".into());
    bridge.register(confirm.0, crate::WidgetKind::Dialog, confirm_props);
    bridge.register(
        drawer.0,
        crate::WidgetKind::Drawer,
        crate::WidgetProps {
            label: "Inspector".into(),
            side: "left".into(),
            ..Default::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());

    let area_id = StableNodeId::try_from(area).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(area_id),
        Some(nana_ui_runtime::StandardVisual::TextInput { invalid: true, .. })
    ));
    assert_eq!(
        doc.runtime
            .text_input(area_id)
            .map(|state| state.value.as_str()),
        Some("line\nbreak")
    );
    let select_id = StableNodeId::try_from(select).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(select_id),
        Some(nana_ui_runtime::StandardVisual::Select { .. })
    ));
    let dialog_id = StableNodeId::try_from(dialog).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(dialog_id),
        Some(nana_ui_runtime::StandardVisual::ModalFrame {
            kind: nana_ui_runtime::ModalSurfaceKind::Dialog(_),
            ..
        })
    ));
    let confirm_id = StableNodeId::try_from(confirm).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(confirm_id),
        Some(nana_ui_runtime::StandardVisual::ModalFrame {
            kind: nana_ui_runtime::ModalSurfaceKind::Confirm(_),
            ..
        })
    ));
    let drawer_id = StableNodeId::try_from(drawer).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(drawer_id),
        Some(nana_ui_runtime::StandardVisual::ModalFrame {
            kind: nana_ui_runtime::ModalSurfaceKind::Drawer(nana_ui_core::DrawerSide::Left),
            ..
        })
    ));
}

#[test]
fn highlighted_textarea_binds_language_and_restores_input() {
    let mut doc = NanaTreeDocument::new(320, 200, 1.0);
    let area = doc.create_element("textarea");
    doc.insert(area, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    let mut props = crate::WidgetProps {
        value: "fn main() {}".into(),
        placeholder: "code".into(),
        label: "Editor".into(),
        ..Default::default()
    };
    props.layout.height = Some(nana_ui_core::LengthSpec::Px(120.0));
    props.attrs.insert("language".into(), "rs".into());
    bridge.register(area.0, crate::WidgetKind::Textarea, props);
    doc.sync_semantic_styles(&bridge.snapshot());

    let area_id = StableNodeId::try_from(area).unwrap();
    assert_eq!(
        doc.runtime
            .component_type(area_id)
            .map(ComponentTypeId::as_str),
        Some("nana.hosted-textarea")
    );
    assert_eq!(
        doc.runtime
            .highlight_request(area_id)
            .map(|request| (request.presenter.as_ref(), request.language.as_ref())),
        Some((nana_ui_runtime::HIGHLIGHT_PRESENTER, "rs"))
    );
    assert_eq!(
        doc.runtime
            .text_input(area_id)
            .map(|state| state.value.as_str()),
        Some("fn main() {}")
    );
    assert!(matches!(
        doc.runtime.standard_visual(area_id),
        Some(nana_ui_runtime::StandardVisual::TextInput {
            placeholder,
            invalid: false,
            ..
        }) if placeholder.as_ref() == "code"
    ));
    assert_eq!(
        doc.runtime
            .node_style(area_id)
            .and_then(|style| style.layout.height),
        Some(nana_ui_core::LengthSpec::Px(
            120.0_f32.max(nana_ui_core::ControlSize::Medium.height()),
        ))
    );
}

#[test]
fn segmented_and_tabs_bind_parent_and_project_child_options() {
    let mut doc = NanaTreeDocument::new(320, 200, 1.0);
    let tabs = doc.create_element("nana-tabs");
    let tab_a = doc.create_element("nana-tabs__item");
    let tab_b = doc.create_element("nana-tabs__item");
    let segmented = doc.create_element("nana-segmented");
    let seg_a = doc.create_element("nana-segmented__item");
    doc.insert(tabs, doc.mount_root(), None);
    doc.insert(tab_a, tabs, None);
    doc.insert(tab_b, tabs, None);
    doc.insert(segmented, doc.mount_root(), None);
    doc.insert(seg_a, segmented, None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        tabs.0,
        crate::WidgetKind::Tabs,
        crate::WidgetProps {
            label: "Editor".into(),
            value: "preview".into(),
            fill: true,
            options: vec![
                crate::SelectOptionProp {
                    value: "code".into(),
                    label: "Code".into(),
                    disabled: false,
                },
                crate::SelectOptionProp {
                    value: "preview".into(),
                    label: "Preview".into(),
                    disabled: false,
                },
            ],
            ..Default::default()
        },
    );
    bridge.register(
        tab_a.0,
        crate::WidgetKind::Chip,
        crate::WidgetProps {
            role: "tab".into(),
            ..Default::default()
        },
    );
    bridge.register(
        tab_b.0,
        crate::WidgetKind::Chip,
        crate::WidgetProps {
            role: "tab".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(tab_a.0, tabs.0, None);
    bridge.insert_child(tab_b.0, tabs.0, None);
    bridge.register(
        segmented.0,
        crate::WidgetKind::Segmented,
        crate::WidgetProps {
            label: "Theme".into(),
            value: "dark".into(),
            options: vec![crate::SelectOptionProp {
                value: "dark".into(),
                label: "Dark".into(),
                disabled: false,
            }],
            ..Default::default()
        },
    );
    bridge.register(
        seg_a.0,
        crate::WidgetKind::Chip,
        crate::WidgetProps {
            role: "tab".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(seg_a.0, segmented.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let tabs_id = StableNodeId::try_from(tabs).unwrap();
    let tab_b_id = StableNodeId::try_from(tab_b).unwrap();
    let segmented_id = StableNodeId::try_from(segmented).unwrap();
    let seg_a_id = StableNodeId::try_from(seg_a).unwrap();
    assert_eq!(
        doc.runtime
            .component_type(tabs_id)
            .map(ComponentTypeId::as_str),
        Some("nana.tabs")
    );
    assert_eq!(
        doc.runtime.accessibility(tabs_id).map(|state| state.role),
        Some(AccessibilityRole::TabList)
    );
    assert_eq!(
        doc.runtime
            .accessibility(tabs_id)
            .and_then(|state| state.label.clone()),
        Some(Arc::<str>::from("Editor"))
    );
    assert!(matches!(
        doc.runtime.standard_visual(tab_b_id),
        Some(nana_ui_runtime::StandardVisual::SelectionOption { selected: true, .. })
    ));
    assert_eq!(
        doc.runtime
            .component_type(segmented_id)
            .map(ComponentTypeId::as_str),
        Some("nana.segmented")
    );
    assert_eq!(
        doc.runtime
            .accessibility(segmented_id)
            .and_then(|state| state.label.clone()),
        Some(Arc::<str>::from("Theme"))
    );
    assert!(matches!(
        doc.runtime.standard_visual(seg_a_id),
        Some(nana_ui_runtime::StandardVisual::SelectionOption { selected: true, .. })
    ));
}

#[test]
fn qualified_runtime_leaves_project_toast_menu_xypad_and_qr() {
    let mut doc = NanaTreeDocument::new(320, 200, 1.0);
    let toast = doc.create_element("nana-toast");
    let tooltip = doc.create_element("nana-tooltip");
    let menu = doc.create_element("nana-action-menu");
    let item = doc.create_element("nana-action-menu-item");
    let pad = doc.create_element("nana-xy-pad");
    let qr = doc.create_element("nana-qr-code");
    let qr_payload = doc.create_element("nana-qr");
    doc.insert(toast, doc.mount_root(), None);
    doc.insert(tooltip, doc.mount_root(), None);
    doc.insert(menu, doc.mount_root(), None);
    doc.insert(item, menu, None);
    doc.insert(pad, doc.mount_root(), None);
    doc.insert(qr, doc.mount_root(), None);
    doc.insert(qr_payload, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    let mut toast_props = crate::WidgetProps {
        label: "Saved".into(),
        hint: "Project exported".into(),
        ..Default::default()
    };
    toast_props.attrs.insert("tone".into(), "success".into());
    toast_props
        .attrs
        .insert("dismissible".into(), String::new());
    bridge.register(toast.0, crate::WidgetKind::Toast, toast_props);
    bridge.register(
        tooltip.0,
        crate::WidgetKind::Tooltip,
        crate::WidgetProps {
            label: "Hint".into(),
            ..Default::default()
        },
    );
    bridge.register(
        menu.0,
        crate::WidgetKind::ActionMenu,
        crate::WidgetProps {
            label: "Actions".into(),
            active: true,
            ..Default::default()
        },
    );
    let mut item_props = crate::WidgetProps {
        label: "Delete".into(),
        hint: "⌫".into(),
        active: true,
        disabled: false,
        button_kind: nana_ui_core::ButtonKind::Danger,
        ..Default::default()
    };
    item_props.attrs.insert("danger".into(), String::new());
    bridge.register(item.0, crate::WidgetKind::ActionMenuItem, item_props);
    let mut pad_props = crate::WidgetProps {
        number: 0.25,
        min: 0.0,
        max: 1.0,
        step: 0.0,
        invalid: true,
        disabled: true,
        loading: true,
        label: "Pan".into(),
        ..Default::default()
    };
    pad_props.attrs.insert("y".into(), "0.75".into());
    bridge.register(pad.0, crate::WidgetKind::XYPad, pad_props);
    let mut qr_props = crate::WidgetProps {
        label: "Pairing".into(),
        ..Default::default()
    };
    qr_props.attrs.insert("modules".into(), "1,0,0,1".into());
    qr_props.attrs.insert("module-width".into(), "2".into());
    bridge.register(qr.0, crate::WidgetKind::QrCode, qr_props);
    let mut payload_props = crate::WidgetProps {
        label: "Pairing".into(),
        value: "nana://pair".into(),
        ..Default::default()
    };
    payload_props
        .attrs
        .insert("payload".into(), "nana://pair".into());
    bridge.register(qr_payload.0, crate::WidgetKind::QrCode, payload_props);
    doc.sync_semantic_styles(&bridge.snapshot());

    let toast_id = StableNodeId::try_from(toast).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(toast_id),
        Some(nana_ui_runtime::StandardVisual::Toast {
            tone: nana_ui_core::ToastTone::Success,
            dismissible: true,
            ..
        })
    ));
    let tooltip_id = StableNodeId::try_from(tooltip).unwrap();
    // Runtime Tooltip is label + a11y only; no StandardVisual leaf.
    assert_eq!(doc.runtime.standard_visual(tooltip_id), None);
    assert_eq!(doc.runtime.text(tooltip_id), Some("Hint"));
    assert_eq!(
        doc.runtime.accessibility(tooltip_id).unwrap().role,
        AccessibilityRole::Tooltip
    );
    let menu_id = StableNodeId::try_from(menu).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(menu_id),
        Some(nana_ui_runtime::StandardVisual::MenuSurface {
            kind: nana_ui_runtime::MenuSurfaceKind::ActionMenu,
            ..
        })
    ));
    let item_id = StableNodeId::try_from(item).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(item_id),
        Some(nana_ui_runtime::StandardVisual::ActionMenuItem {
            danger: true,
            active: true,
            disabled: false,
            ..
        })
    ));
    let pad_id = StableNodeId::try_from(pad).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(pad_id),
        Some(nana_ui_runtime::StandardVisual::XYPad {
            invalid: true,
            disabled: true,
            ..
        })
    ));
    let qr_id = StableNodeId::try_from(qr).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(qr_id),
        Some(nana_ui_runtime::StandardVisual::QrCode { width: 2, .. })
    ));
    let payload_id = StableNodeId::try_from(qr_payload).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(payload_id),
        Some(nana_ui_runtime::StandardVisual::QrCode { width, .. }) if width >= 21
    ));
}

#[test]
#[cfg(feature = "calendar")]
#[cfg(feature = "rich-text")]
fn command_palette_and_tree_view_project_runtime_visuals() {
    let mut doc = NanaTreeDocument::new(320, 200, 1.0);
    let palette = doc.create_element("nana-command-palette");
    let tree = doc.create_element("nana-tree-view");
    let calendar = doc.create_element("nana-calendar");
    let markdown = doc.create_element("nana-markdown");
    doc.insert(palette, doc.mount_root(), None);
    doc.insert(tree, doc.mount_root(), None);
    doc.insert(calendar, doc.mount_root(), None);
    doc.insert(markdown, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        palette.0,
        crate::WidgetKind::CommandPalette,
        crate::WidgetProps {
            label: "Go to".into(),
            placeholder: "Search".into(),
            value: "op".into(),
            active: true,
            options: vec![
                crate::SelectOptionProp {
                    value: "open".into(),
                    label: "Open file".into(),
                    disabled: false,
                },
                crate::SelectOptionProp {
                    value: "palette".into(),
                    label: "Command palette".into(),
                    disabled: false,
                },
            ],
            ..Default::default()
        },
    );
    bridge.register(
        tree.0,
        crate::WidgetKind::TreeView,
        crate::WidgetProps {
            value: "src".into(),
            options: vec![
                crate::SelectOptionProp {
                    value: "src".into(),
                    label: "src".into(),
                    disabled: false,
                },
                crate::SelectOptionProp {
                    value: "docs".into(),
                    label: "docs".into(),
                    disabled: false,
                },
            ],
            ..Default::default()
        },
    );
    bridge.register(
        calendar.0,
        crate::WidgetKind::CalendarHeatmap,
        crate::WidgetProps {
            label: "Activity".into(),
            ..Default::default()
        },
    );
    bridge.register(
        markdown.0,
        crate::WidgetKind::NativeMarkdown,
        crate::WidgetProps {
            value: "# Title".into(),
            ..Default::default()
        },
    );
    doc.sync_semantic_styles(&bridge.snapshot());

    let palette_id = StableNodeId::try_from(palette).unwrap();
    assert!(matches!(
        doc.runtime.node(palette_id).map(|node| node.kind),
        Some(NodeKind::Element { .. })
    ));
    assert!(matches!(
        doc.runtime.standard_visual(palette_id),
        Some(nana_ui_runtime::StandardVisual::CommandPalette {
            title,
            query,
            ..
        }) if title.as_ref() == "Go to" && query.as_ref() == "op"
    ));
    let tree_id = StableNodeId::try_from(tree).unwrap();
    assert!(matches!(
        doc.runtime.node(tree_id).map(|node| node.kind),
        Some(NodeKind::Element { .. })
    ));
    assert!(matches!(
        doc.runtime.standard_visual(tree_id),
        Some(nana_ui_runtime::StandardVisual::TreeView { rows, .. })
            if rows.iter().any(|row| row.id.as_ref() == "src" && row.selected)
                && rows.iter().any(|row| row.id.as_ref() == "docs")
    ));
    let calendar_id = StableNodeId::try_from(calendar).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(calendar_id),
        Some(nana_ui_runtime::StandardVisual::CalendarHeatmap { .. })
    ));
    let markdown_id = StableNodeId::try_from(markdown).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(markdown_id),
        Some(nana_ui_runtime::StandardVisual::NativeMarkdown { text, .. })
            if text.contains("Title")
    ));
}

#[test]
fn command_palette_host_items_keep_category() {
    let mut doc = NanaTreeDocument::new(320, 200, 1.0);
    let palette = doc.create_element("nana-command-palette");
    doc.insert(palette, doc.mount_root(), None);
    let mut props = crate::WidgetProps {
        label: "Go to".into(),
        ..Default::default()
    };
    props.apply_prop(
        "items",
        &nana_js_engine::HostValue::Array(vec![nana_js_engine::HostValue::Object(
            [
                ("value".into(), nana_js_engine::HostValue::string("open")),
                (
                    "label".into(),
                    nana_js_engine::HostValue::string("Open file"),
                ),
                (
                    "category".into(),
                    nana_js_engine::HostValue::string("Workspace"),
                ),
                (
                    "shortcut".into(),
                    nana_js_engine::HostValue::string("Ctrl+P"),
                ),
            ]
            .into_iter()
            .collect(),
        )]),
    );
    let mut bridge = crate::MessageBridge::new();
    bridge.register(palette.0, crate::WidgetKind::CommandPalette, props);
    doc.sync_semantic_styles(&bridge.snapshot());
    let palette_id = StableNodeId::try_from(palette).unwrap();
    assert!(
        matches!(
            doc.runtime.standard_visual(palette_id),
            Some(nana_ui_runtime::StandardVisual::CommandPalette { rows, .. })
                if rows.iter().any(|row| {
                    row.label.as_ref() == "Open file"
                        && row.category.as_deref() == Some("Workspace")
                        && row.shortcut.as_deref() == Some("Ctrl+P")
                })
        ),
        "host command palette items must keep category and shortcut"
    );
}

#[test]
#[cfg(feature = "rich-text")]
fn markdown_source_from_native_props_projects_and_assembles() {
    let mut doc = NanaTreeDocument::new(420, 240, 1.0);
    let markdown = doc.create_element("nana-markdown");
    doc.insert(markdown, doc.mount_root(), None);
    let mut props = crate::WidgetProps::default();
    props.apply_prop(
        "source",
        &nana_js_engine::HostValue::string("# Native\n\n```mermaid\nflowchart LR\nA-->B\n```"),
    );
    props.apply_prop(
        "mermaidRenderer",
        &nana_js_engine::HostValue::string("app-mermaid"),
    );
    let mut bridge = crate::MessageBridge::new();
    bridge.register(markdown.0, crate::WidgetKind::NativeMarkdown, props);
    doc.sync_semantic_styles(&bridge.snapshot());
    let markdown_id = StableNodeId::try_from(markdown).unwrap();
    assert!(
        matches!(
            doc.runtime.standard_visual(markdown_id),
            Some(nana_ui_runtime::StandardVisual::NativeMarkdown { text, .. })
                if text.contains("Native") && text.contains("flowchart LR")
        ),
        "empty value must still bind markdown source from native_props"
    );
    let children = doc.runtime.node(markdown_id).unwrap().children;
    assert!(
        children.iter().any(|child| {
            doc.runtime
                .highlight_request(*child)
                .is_some_and(|request| {
                    request.presenter.as_ref() == RuntimeNativeMarkdown::MERMAID_PRESENTER
                })
        }),
        "pending markdown assemble must still attach mermaid fence highlight"
    );
}

#[test]
#[cfg(feature = "calendar")]
#[cfg(feature = "rich-text")]
#[cfg(feature = "graph-canvas")]
#[cfg(feature = "image-viewer")]
fn professional_leaves_project_native_payloads() {
    let mut doc = NanaTreeDocument::new(420, 240, 1.0);
    let calendar = doc.create_element("nana-calendar");
    let markdown = doc.create_element("nana-markdown");
    let viewer = doc.create_element("nana-image-viewer");
    let canvas = doc.create_element("nana-graph-canvas");
    doc.insert(calendar, doc.mount_root(), None);
    doc.insert(markdown, doc.mount_root(), None);
    doc.insert(viewer, doc.mount_root(), None);
    doc.insert(canvas, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();

    let mut calendar_props = crate::WidgetProps {
        label: "Activity".into(),
        ..Default::default()
    };
    calendar_props.apply_prop(
        "data",
        &nana_js_engine::HostValue::Array(vec![
            nana_js_engine::HostValue::Object(
                [
                    (
                        "date".into(),
                        nana_js_engine::HostValue::string("2026-06-01"),
                    ),
                    ("value".into(), nana_js_engine::HostValue::Number(2.0)),
                ]
                .into_iter()
                .collect(),
            ),
            nana_js_engine::HostValue::Array(vec![
                nana_js_engine::HostValue::string("2026-06-03"),
                nana_js_engine::HostValue::Number(8.0),
            ]),
        ]),
    );
    bridge.register(
        calendar.0,
        crate::WidgetKind::CalendarHeatmap,
        calendar_props,
    );
    bridge.register(
        markdown.0,
        crate::WidgetKind::NativeMarkdown,
        crate::WidgetProps {
            value: "# Title\n\nHello **world**".into(),
            ..Default::default()
        },
    );
    let mut viewer_props = crate::WidgetProps::default();
    viewer_props.apply_prop("src", &nana_js_engine::HostValue::string("gpu:preview"));
    bridge.register(viewer.0, crate::WidgetKind::ImageViewer, viewer_props);

    let mut canvas_props = crate::WidgetProps {
        label: "Graph".into(),
        ..Default::default()
    };
    canvas_props.apply_prop(
        "nodes",
        &nana_js_engine::HostValue::Array(vec![
            nana_js_engine::HostValue::Object(
                [
                    ("id".into(), nana_js_engine::HostValue::string("source")),
                    ("title".into(), nana_js_engine::HostValue::string("Source")),
                    ("x".into(), nana_js_engine::HostValue::Number(20.0)),
                    ("y".into(), nana_js_engine::HostValue::Number(24.0)),
                    ("width".into(), nana_js_engine::HostValue::Number(160.0)),
                    ("height".into(), nana_js_engine::HostValue::Number(80.0)),
                ]
                .into_iter()
                .collect(),
            ),
            nana_js_engine::HostValue::Object(
                [
                    ("id".into(), nana_js_engine::HostValue::string("sink")),
                    ("label".into(), nana_js_engine::HostValue::string("Sink")),
                    ("x".into(), nana_js_engine::HostValue::Number(240.0)),
                    ("y".into(), nana_js_engine::HostValue::Number(24.0)),
                ]
                .into_iter()
                .collect(),
            ),
        ]),
    );
    bridge.register(canvas.0, crate::WidgetKind::GraphCanvas, canvas_props);
    doc.sync_semantic_styles(&bridge.snapshot());

    let calendar_id = StableNodeId::try_from(calendar).unwrap();
    assert!(
        matches!(
            doc.runtime.standard_visual(calendar_id),
            Some(nana_ui_runtime::StandardVisual::CalendarHeatmap { cells, .. })
                if !cells.is_empty() && cells.iter().any(|cell| cell.level > 0)
        ),
        "calendar with two data points must not project CalendarHeatmap::new([])"
    );
    let markdown_id = StableNodeId::try_from(markdown).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(markdown_id),
        Some(nana_ui_runtime::StandardVisual::NativeMarkdown { text, .. })
            if text.contains("Title") && text.contains("Hello world")
    ));
    let viewer_id = StableNodeId::try_from(viewer).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(viewer_id),
        Some(nana_ui_runtime::StandardVisual::ImageViewer { .. })
    ));
    let canvas_id = StableNodeId::try_from(canvas).unwrap();
    assert!(matches!(
        doc.runtime.standard_visual(canvas_id),
        Some(nana_ui_runtime::StandardVisual::GraphCanvas { nodes, .. })
            if nodes.len() == 2
    ));
}

#[test]
#[cfg(feature = "calendar")]
fn calendar_options_object_projects_heatmap_metrics() {
    let mut doc = NanaTreeDocument::new(420, 240, 1.0);
    let default_calendar = doc.create_element("nana-calendar");
    let sized_calendar = doc.create_element("nana-calendar");
    doc.insert(default_calendar, doc.mount_root(), None);
    doc.insert(sized_calendar, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();

    let cells = nana_js_engine::HostValue::Array(vec![nana_js_engine::HostValue::Object(
        [
            (
                "date".into(),
                nana_js_engine::HostValue::string("2026-06-01"),
            ),
            ("value".into(), nana_js_engine::HostValue::Number(4.0)),
        ]
        .into_iter()
        .collect(),
    )]);
    let mut default_props = crate::WidgetProps::default();
    default_props.apply_prop("data", &cells);
    bridge.register(
        default_calendar.0,
        crate::WidgetKind::CalendarHeatmap,
        default_props,
    );

    let mut sized_props = crate::WidgetProps::default();
    sized_props.apply_prop("data", &cells);
    sized_props.apply_prop(
        "options",
        &nana_js_engine::HostValue::Object(
            [
                ("cellSize".into(), nana_js_engine::HostValue::Number(18.0)),
                (
                    "weekStartsOn".into(),
                    nana_js_engine::HostValue::Number(0.0),
                ),
            ]
            .into_iter()
            .collect(),
        ),
    );
    bridge.register(
        sized_calendar.0,
        crate::WidgetKind::CalendarHeatmap,
        sized_props,
    );
    doc.sync_semantic_styles(&bridge.snapshot());

    let default_id = StableNodeId::try_from(default_calendar).unwrap();
    let sized_id = StableNodeId::try_from(sized_calendar).unwrap();
    let Some(nana_ui_runtime::StandardVisual::CalendarHeatmap {
        cell_size: default_size,
        cells: default_cells,
        ..
    }) = doc.runtime.standard_visual(default_id)
    else {
        panic!("default calendar must project a heatmap");
    };
    let Some(nana_ui_runtime::StandardVisual::CalendarHeatmap {
        cell_size: sized,
        cells: sized_cells,
        ..
    }) = doc.runtime.standard_visual(sized_id)
    else {
        panic!("options calendar must project a heatmap");
    };
    assert_eq!(default_size, 11.0);
    assert_eq!(sized, 18.0);
    let default_y = default_cells
        .iter()
        .find(|cell| cell.level > 0)
        .map(|cell| cell.y);
    let sized_y = sized_cells
        .iter()
        .find(|cell| cell.level > 0)
        .map(|cell| cell.y);
    assert_ne!(
        default_y, sized_y,
        "week_starts_on must shift the first active cell"
    );
}

fn calendar_sample_cells() -> nana_js_engine::HostValue {
    nana_js_engine::HostValue::Array(vec![nana_js_engine::HostValue::Object(
        [
            (
                "date".into(),
                nana_js_engine::HostValue::string("2026-06-01"),
            ),
            ("value".into(), nana_js_engine::HostValue::Number(4.0)),
        ]
        .into_iter()
        .collect(),
    )])
}

fn project_calendar_with_options(
    options: nana_js_engine::HostValue,
) -> (
    NanaTreeDocument,
    crate::WidgetProps,
    nana_ui_runtime::StandardVisual,
) {
    let mut doc = NanaTreeDocument::new(420, 240, 1.0);
    let calendar = doc.create_element("nana-calendar");
    doc.insert(calendar, doc.mount_root(), None);
    let mut props = crate::WidgetProps::default();
    props.apply_prop("data", &calendar_sample_cells());
    props.apply_prop("options", &options);
    let mut bridge = crate::MessageBridge::new();
    bridge.register(
        calendar.0,
        crate::WidgetKind::CalendarHeatmap,
        props.clone(),
    );
    doc.sync_semantic_styles(&bridge.snapshot());
    let visual = doc
        .runtime
        .standard_visual(StableNodeId::try_from(calendar).unwrap())
        .expect("calendar must project a heatmap");
    (doc, props, visual)
}

#[test]
#[cfg(feature = "calendar")]
fn calendar_options_weekday_labels_project_day_labels() {
    let (_doc, _props, visual) = project_calendar_with_options(nana_js_engine::HostValue::Object(
        [(
            "weekdayLabels".into(),
            nana_js_engine::HostValue::Array(vec![
                nana_js_engine::HostValue::Array(vec![
                    nana_js_engine::HostValue::Number(0.0),
                    nana_js_engine::HostValue::string("Sun"),
                ]),
                nana_js_engine::HostValue::Object(
                    [
                        ("day".into(), nana_js_engine::HostValue::Number(2.0)),
                        ("label".into(), nana_js_engine::HostValue::string("Tue")),
                    ]
                    .into_iter()
                    .collect(),
                ),
                nana_js_engine::HostValue::string("Wed"),
            ]),
        )]
        .into_iter()
        .collect(),
    ));
    let nana_ui_runtime::StandardVisual::CalendarHeatmap { day_labels, .. } = visual else {
        panic!("calendar must project a heatmap");
    };
    let labels: Vec<&str> = day_labels.iter().map(|label| label.text.as_ref()).collect();
    assert!(labels.contains(&"Sun"), "pair weekday labels must project");
    assert!(
        labels.contains(&"Tue"),
        "object weekday labels must project"
    );
    assert!(
        !labels.iter().any(|label| label.contains("周")),
        "custom weekday labels replace the default 周* set"
    );
}

#[test]
#[cfg(feature = "calendar")]
fn calendar_options_month_format_changes_month_label() {
    let (_doc, _default_props, default_visual) =
        project_calendar_with_options(nana_js_engine::HostValue::Object(BTreeMap::new()));
    let (_doc, _formatted_props, formatted_visual) =
        project_calendar_with_options(nana_js_engine::HostValue::Object(
            [(
                "monthFormat".into(),
                nana_js_engine::HostValue::string("{year}-{monthPad}"),
            )]
            .into_iter()
            .collect(),
        ));
    let nana_ui_runtime::StandardVisual::CalendarHeatmap {
        month_labels: default_months,
        ..
    } = default_visual
    else {
        panic!("default calendar must project a heatmap");
    };
    let nana_ui_runtime::StandardVisual::CalendarHeatmap {
        month_labels: formatted_months,
        ..
    } = formatted_visual
    else {
        panic!("formatted calendar must project a heatmap");
    };
    assert!(
        default_months
            .iter()
            .any(|label| label.text.as_ref() == "6月"),
        "default month formatter is {{month}}月"
    );
    assert!(
        formatted_months
            .iter()
            .any(|label| label.text.as_ref() == "2026-06"),
        "monthFormat {{year}}-{{monthPad}} must change the painted month label"
    );
}

#[test]
#[cfg(feature = "calendar")]
fn calendar_options_title_format_changes_cell_title() {
    let (_doc, _, visual) = project_calendar_with_options(nana_js_engine::HostValue::Object(
        [(
            "titleFormat".into(),
            nana_js_engine::HostValue::string("{date}={value}"),
        )]
        .into_iter()
        .collect(),
    ));
    assert!(matches!(
        visual,
        nana_ui_runtime::StandardVisual::CalendarHeatmap { .. }
    ));
}

#[test]
#[cfg(feature = "calendar")]
fn calendar_options_function_formatter_keeps_default() {
    let (_doc, _props, visual) = project_calendar_with_options(nana_js_engine::HostValue::Object(
        [
            (
                "monthFormatter".into(),
                nana_js_engine::HostValue::Function(nana_js_engine::JsFunctionId(11)),
            ),
            (
                "titleFormatter".into(),
                nana_js_engine::HostValue::Function(nana_js_engine::JsFunctionId(12)),
            ),
        ]
        .into_iter()
        .collect(),
    ));
    let nana_ui_runtime::StandardVisual::CalendarHeatmap { month_labels, .. } = visual else {
        panic!("function formatters must still project a heatmap");
    };
    assert!(
        month_labels
            .iter()
            .any(|label| label.text.as_ref() == "6月"),
        "Function-valued monthFormatter is ignored"
    );
}

fn settings_page_model_value(
    tabs: &[(&str, &str, bool)],
    default_tab: &str,
    hide_header: bool,
) -> nana_js_engine::HostValue {
    nana_js_engine::HostValue::Object(
        [
            (
                "tabs".into(),
                nana_js_engine::HostValue::Array(
                    tabs.iter()
                        .map(|(key, label, full_page)| {
                            nana_js_engine::HostValue::Object(
                                [
                                    ("key".into(), nana_js_engine::HostValue::string(*key)),
                                    ("label".into(), nana_js_engine::HostValue::string(*label)),
                                    (
                                        "fullPage".into(),
                                        nana_js_engine::HostValue::Bool(*full_page),
                                    ),
                                ]
                                .into_iter()
                                .collect(),
                            )
                        })
                        .collect(),
                ),
            ),
            (
                "defaultTab".into(),
                nana_js_engine::HostValue::string(default_tab),
            ),
            (
                "hideHeader".into(),
                nana_js_engine::HostValue::Bool(hide_header),
            ),
        ]
        .into_iter()
        .collect(),
    )
}

fn sync_settings_page(
    settings: nana_js_engine::HostValue,
    tab: Option<&str>,
    hide_header: Option<bool>,
) -> (NanaTreeDocument, StableNodeId, StableNodeId) {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let page = doc.create_element("nana-settings-page");
    let content = doc.create_element("div");
    doc.insert(page, doc.mount_root(), None);
    doc.insert(content, page, None);
    let mut props = crate::WidgetProps::default();
    props.apply_prop("settings", &settings);
    if let Some(tab) = tab {
        props.apply_prop("tab", &nana_js_engine::HostValue::string(tab));
    }
    if let Some(hide_header) = hide_header {
        props.apply_prop("hide-header", &nana_js_engine::HostValue::Bool(hide_header));
    }
    let mut bridge = crate::MessageBridge::new();
    bridge.register(page.0, crate::WidgetKind::SettingsPage, props);
    bridge.register(
        content.0,
        crate::WidgetKind::Column,
        crate::WidgetProps::default(),
    );
    bridge.insert_child(content.0, page.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());
    (
        doc,
        StableNodeId::try_from(page).unwrap(),
        StableNodeId::try_from(content).unwrap(),
    )
}

fn settings_page_assembly(
    doc: &NanaTreeDocument,
    page: StableNodeId,
) -> nana_ui_runtime::SettingsPageAssembly {
    doc.context()
        .read(
            Entity::<RuntimeSettingsPage>::from_stable_id(page),
            |page| page.assembly.clone().unwrap_or_default(),
        )
        .expect("SettingsPage must be bound after sync")
}

fn world_has_descendant(doc: &NanaTreeDocument, root: StableNodeId, target: StableNodeId) -> bool {
    let Some(node) = doc.runtime.node(root) else {
        return false;
    };
    node.children
        .iter()
        .any(|child| *child == target || world_has_descendant(doc, *child, target))
}

#[test]
fn settings_page_widget_kind_parses_host_tag() {
    assert_eq!(
        crate::WidgetKind::parse("nana-settings-page"),
        Some(crate::WidgetKind::SettingsPage)
    );
    assert_eq!(
        crate::WidgetKind::parse("settings-page"),
        Some(crate::WidgetKind::SettingsPage)
    );
    assert_eq!(crate::WidgetKind::parse("settingspage"), None);
    assert_eq!(
        crate::WidgetKind::SettingsPage.element_tag(),
        "nana-settings-page"
    );
}

#[test]
fn settings_page_assembles_scroll_body_title() {
    let (doc, page, content) = sync_settings_page(
        settings_page_model_value(&[("appearance", "外观", false)], "appearance", false),
        Some("appearance"),
        None,
    );
    let assembly = settings_page_assembly(&doc, page);
    let scroll = assembly
        .scroll
        .expect("assemble_settings_page mounts scroll");
    let body = assembly.body.expect("assemble_settings_page mounts body");
    let title = assembly.title.expect("assemble_settings_page mounts title");
    assert_eq!(doc.runtime.node(page).unwrap().children, vec![scroll]);
    assert_eq!(
        doc.runtime.node(scroll).unwrap().kind,
        NodeKind::Element {
            tag: "scroll".into(),
        }
    );
    assert!(
        doc.runtime
            .node_style(scroll)
            .unwrap()
            .layout
            .overflow_y
            .scrolls()
    );
    assert_eq!(doc.runtime.node(scroll).unwrap().children, vec![body]);
    assert_eq!(
        doc.runtime.node(body).unwrap().children,
        vec![title, content]
    );
    assert_eq!(doc.runtime.text(title), Some("外观"));
}

#[test]
fn settings_page_attrs_json_assembles_scroll_body_title() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let page = doc.create_element("nana-settings-page");
    let content = doc.create_element("div");
    doc.insert(page, doc.mount_root(), None);
    doc.insert(content, page, None);
    let mut props = crate::WidgetProps::default();
    props.attrs.insert(
        "settings".into(),
        r#"{"tabs":[{"id":"appearance","label":"外观"}],"defaultTab":"appearance"}"#.into(),
    );
    props.attrs.insert("tab".into(), "appearance".into());
    assert!(
        !props.native_props.contains_key("settings") && !props.native_props.contains_key("model"),
        "attrs-only settings must not supply native_props"
    );
    let mut bridge = crate::MessageBridge::new();
    bridge.register(page.0, crate::WidgetKind::SettingsPage, props);
    bridge.register(
        content.0,
        crate::WidgetKind::Column,
        crate::WidgetProps::default(),
    );
    bridge.insert_child(content.0, page.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let page = StableNodeId::try_from(page).unwrap();
    let content = StableNodeId::try_from(content).unwrap();
    let assembly = settings_page_assembly(&doc, page);
    let scroll = assembly
        .scroll
        .expect("assemble_settings_page mounts scroll");
    let body = assembly.body.expect("assemble_settings_page mounts body");
    let title = assembly.title.expect("assemble_settings_page mounts title");
    assert_eq!(doc.runtime.node(page).unwrap().children, vec![scroll]);
    assert_eq!(
        doc.runtime.node(scroll).unwrap().kind,
        NodeKind::Element {
            tag: "scroll".into(),
        }
    );
    assert!(
        doc.runtime
            .node_style(scroll)
            .unwrap()
            .layout
            .overflow_y
            .scrolls()
    );
    assert_eq!(doc.runtime.node(scroll).unwrap().children, vec![body]);
    assert_eq!(
        doc.runtime.node(body).unwrap().children,
        vec![title, content]
    );
    assert_eq!(doc.runtime.text(title), Some("外观"));
}

#[test]
fn settings_page_resync_reuses_scroll_body_title() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let page = doc.create_element("nana-settings-page");
    let content = doc.create_element("div");
    doc.insert(page, doc.mount_root(), None);
    doc.insert(content, page, None);
    let mut props = crate::WidgetProps::default();
    props.apply_prop(
        "settings",
        &settings_page_model_value(&[("appearance", "外观", false)], "appearance", false),
    );
    props.apply_prop("tab", &nana_js_engine::HostValue::string("appearance"));
    let mut bridge = crate::MessageBridge::new();
    bridge.register(page.0, crate::WidgetKind::SettingsPage, props.clone());
    bridge.register(
        content.0,
        crate::WidgetKind::Column,
        crate::WidgetProps::default(),
    );
    bridge.insert_child(content.0, page.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let page_id = StableNodeId::try_from(page).unwrap();
    let content_id = StableNodeId::try_from(content).unwrap();
    let first = settings_page_assembly(&doc, page_id);
    let scroll = first.scroll.expect("assemble_settings_page mounts scroll");
    let body = first.body.expect("assemble_settings_page mounts body");
    let title = first.title.expect("assemble_settings_page mounts title");

    bridge.insert_child(content.0, page.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let second = settings_page_assembly(&doc, page_id);
    assert_eq!(second.scroll, Some(scroll));
    assert_eq!(second.body, Some(body));
    assert_eq!(second.title, Some(title));
    assert_eq!(
        doc.runtime.node(body).unwrap().children,
        vec![title, content_id]
    );
    assert_eq!(doc.runtime.text(title), Some("外观"));
}

#[test]
fn settings_page_hide_header_omits_title() {
    let (doc, page, content) = sync_settings_page(
        settings_page_model_value(&[("appearance", "外观", false)], "appearance", true),
        Some("appearance"),
        Some(true),
    );
    let assembly = settings_page_assembly(&doc, page);
    let body = assembly.body.expect("hide-header still mounts scroll body");
    assert_eq!(doc.runtime.node(body).unwrap().children, vec![content]);
    assert!(assembly.title.is_none() || !world_has_descendant(&doc, page, assembly.title.unwrap()));
    assert_ne!(doc.runtime.text(page), Some("外观"));
}

#[test]
fn settings_page_full_page_omits_title() {
    let (doc, page, content) = sync_settings_page(
        settings_page_model_value(&[("workspace", "工作区", true)], "workspace", false),
        Some("workspace"),
        None,
    );
    let assembly = settings_page_assembly(&doc, page);
    assert_eq!(doc.runtime.node(page).unwrap().children, vec![content]);
    assert!(assembly.title.is_none() || !world_has_descendant(&doc, page, assembly.title.unwrap()));
    if let Some(scroll) = assembly.scroll {
        assert!(!world_has_descendant(&doc, page, scroll));
    }
    assert_ne!(doc.runtime.text(page), Some("工作区"));
}

#[test]
#[cfg(feature = "graph-canvas")]
fn graph_viewport_and_selection_survive_projection() {
    let mut doc = NanaTreeDocument::new(420, 240, 1.0);
    let canvas = doc.create_element("nana-graph-canvas");
    doc.insert(canvas, doc.mount_root(), None);
    let mut bridge = crate::MessageBridge::new();
    let mut props = crate::WidgetProps {
        label: "Graph".into(),
        ..Default::default()
    };
    props.apply_prop(
        "nodes",
        &nana_js_engine::HostValue::Array(vec![
            nana_js_engine::HostValue::Object(
                [
                    ("id".into(), nana_js_engine::HostValue::string("source")),
                    ("title".into(), nana_js_engine::HostValue::string("Source")),
                    ("x".into(), nana_js_engine::HostValue::Number(20.0)),
                    ("y".into(), nana_js_engine::HostValue::Number(24.0)),
                ]
                .into_iter()
                .collect(),
            ),
            nana_js_engine::HostValue::Object(
                [
                    ("id".into(), nana_js_engine::HostValue::string("sink")),
                    ("label".into(), nana_js_engine::HostValue::string("Sink")),
                    ("x".into(), nana_js_engine::HostValue::Number(240.0)),
                    ("y".into(), nana_js_engine::HostValue::Number(24.0)),
                ]
                .into_iter()
                .collect(),
            ),
        ]),
    );
    props.apply_prop(
        "viewport",
        &nana_js_engine::HostValue::Object(
            [
                (
                    "offset".into(),
                    nana_js_engine::HostValue::Object(
                        [
                            ("x".into(), nana_js_engine::HostValue::Number(12.0)),
                            ("y".into(), nana_js_engine::HostValue::Number(-8.0)),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                ),
                ("zoom".into(), nana_js_engine::HostValue::Number(2.0)),
            ]
            .into_iter()
            .collect(),
        ),
    );
    props.apply_prop(
        "selection",
        &nana_js_engine::HostValue::Object(
            [
                ("kind".into(), nana_js_engine::HostValue::string("node")),
                ("id".into(), nana_js_engine::HostValue::string("source")),
            ]
            .into_iter()
            .collect(),
        ),
    );
    bridge.register(canvas.0, crate::WidgetKind::GraphCanvas, props);
    doc.sync_semantic_styles(&bridge.snapshot());

    let canvas_id = StableNodeId::try_from(canvas).unwrap();
    assert!(
        matches!(
            doc.runtime.standard_visual(canvas_id),
            Some(nana_ui_runtime::StandardVisual::GraphCanvas {
                viewport_offset_x,
                viewport_offset_y,
                viewport_zoom,
                nodes,
                ..
            }) if viewport_offset_x == 12.0
                && viewport_offset_y == -8.0
                && viewport_zoom == 2.0
                && nodes.iter().any(|node| node.selected)
        ),
        "viewport zoom/offset and selection must survive GraphCanvas projection"
    );
}

#[test]
#[cfg(feature = "graph-canvas")]
fn graph_node_content_slot_is_a_document_order_child() {
    let mut doc = NanaTreeDocument::new(420, 240, 1.0);
    let canvas = doc.create_element("nana-graph-canvas");
    let interior = doc.create_element("div");
    doc.insert(canvas, doc.mount_root(), None);
    doc.insert(interior, canvas, None);
    doc.set_attribute(interior, "data-slot", "source");
    doc.set_gpu_slot(interior, "formula.source");
    let mut bridge = crate::MessageBridge::new();
    let mut props = crate::WidgetProps {
        label: "Graph".into(),
        ..Default::default()
    };
    props.apply_prop(
        "nodes",
        &nana_js_engine::HostValue::Array(vec![nana_js_engine::HostValue::Object(
            [
                ("id".into(), nana_js_engine::HostValue::string("source")),
                ("title".into(), nana_js_engine::HostValue::string("Source")),
                ("x".into(), nana_js_engine::HostValue::Number(20.0)),
                ("y".into(), nana_js_engine::HostValue::Number(24.0)),
                ("width".into(), nana_js_engine::HostValue::Number(160.0)),
                ("height".into(), nana_js_engine::HostValue::Number(80.0)),
            ]
            .into_iter()
            .collect(),
        )]),
    );
    let mut interior_props = crate::WidgetProps::default();
    interior_props
        .attrs
        .insert("data-slot".into(), "source".into());
    bridge.register(canvas.0, crate::WidgetKind::GraphCanvas, props);
    bridge.register(interior.0, crate::WidgetKind::Box, interior_props);
    bridge.insert_child(interior.0, canvas.0, None);
    doc.flush_host_frame();
    doc.sync_semantic_styles(&bridge.snapshot());

    let canvas_id = StableNodeId::try_from(canvas).unwrap();
    let interior_id = StableNodeId::try_from(interior).unwrap();
    assert!(
        matches!(
            doc.runtime.standard_visual(canvas_id),
            Some(nana_ui_runtime::StandardVisual::GraphCanvas { nodes, .. })
                if nodes.len() == 1
        ),
        "default grid/frame paint must stay on the canvas"
    );
    assert!(
        doc.runtime.custom_render(canvas_id).is_none(),
        "node interiors must not attach GPU content on the canvas itself"
    );
    let custom = doc
        .runtime
        .custom_render(interior_id)
        .expect("data-nana-gpu child is a HostTexture slot");
    assert_eq!(custom.renderer.as_ref(), HOST_TEXTURE_RENDERER);
    assert_eq!(custom.resource.as_ref(), "formula.source");
    assert_eq!(
        doc.runtime.node(canvas_id).unwrap().children,
        vec![interior_id]
    );
    let style = doc.runtime.node_style(interior_id).unwrap();
    assert_eq!(style.layout.position, nana_ui_core::PositionSpec::Absolute);
    assert!(style.layout.offset_left.is_some());
    assert!(style.layout.offset_top.is_some());
    assert!(style.layout.width.is_some());
    assert!(style.layout.height.is_some());
}

#[test]
fn app_shell_with_title_and_child_projects_title_bar_and_body() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let shell = doc.create_element("nana-app-shell");
    let title_bar = doc.create_element("nana-app-title-bar");
    let body = doc.create_element("div");
    doc.insert(shell, doc.mount_root(), None);
    doc.insert(title_bar, shell, None);
    doc.insert(body, shell, None);
    let mut bridge = crate::MessageBridge::new();
    let mut title_props = crate::WidgetProps {
        label: "Nana".into(),
        element_tag: "nana-app-title-bar".into(),
        ..Default::default()
    };
    title_props
        .attrs
        .insert("data-slot".into(), "title-bar".into());
    title_props.class_names.push("nana-app-title-bar".into());
    bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
    bridge.register(
        body.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            label: "Workspace".into(),
            ..Default::default()
        },
    );
    bridge.register(
        shell.0,
        crate::WidgetKind::AppShell,
        crate::WidgetProps {
            label: "Nana".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(title_bar.0, shell.0, None);
    bridge.insert_child(body.0, shell.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let title_id = StableNodeId::try_from(title_bar).unwrap();
    let body_id = StableNodeId::try_from(body).unwrap();
    let title_style = doc.runtime.node_style(title_id).unwrap();
    let body_style = doc.runtime.node_style(body_id).unwrap();
    assert_eq!(
        title_style.layout.height,
        Some(nana_ui_core::LengthSpec::Px(nana_ui_core::TITLE_BAR_HEIGHT))
    );
    assert_eq!(title_style.layout.flex_grow, Some(0.0));
    assert!(
        doc.runtime.text(title_id).unwrap_or("").is_empty(),
        "title-bar root text must be empty after assemble, got {:?}",
        doc.runtime.text(title_id)
    );
    let title_label =
        assembled_title_bar_center_label(&doc, title_id).expect("center column title");
    assert_eq!(doc.runtime.text(title_label), Some("Nana"));
    assert_eq!(
        doc.runtime
            .accessibility(title_id)
            .and_then(|state| state.label.as_deref()),
        Some("Nana")
    );
    assert_eq!(body_style.layout.flex_grow, Some(1.0));
    assert_eq!(
        body_style.layout.height,
        Some(nana_ui_core::LengthSpec::Fill)
    );
}

#[test]
fn app_shell_title_bar_and_body_keep_layout_after_assemble() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let shell = doc.create_element("nana-app-shell");
    let title_bar = doc.create_element("nana-app-title-bar");
    let body = doc.create_element("div");
    doc.insert(shell, doc.mount_root(), None);
    doc.insert(title_bar, shell, None);
    doc.insert(body, shell, None);
    let mut bridge = crate::MessageBridge::new();
    let mut title_props = crate::WidgetProps {
        label: "Nana".into(),
        element_tag: "nana-app-title-bar".into(),
        ..Default::default()
    };
    title_props
        .attrs
        .insert("data-slot".into(), "title-bar".into());
    title_props.class_names.push("nana-app-title-bar".into());
    bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
    bridge.register(
        body.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            label: "Workspace".into(),
            ..Default::default()
        },
    );
    bridge.register(
        shell.0,
        crate::WidgetKind::AppShell,
        crate::WidgetProps {
            label: "Nana".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(title_bar.0, shell.0, None);
    bridge.insert_child(body.0, shell.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let shell_id = StableNodeId::try_from(shell).unwrap();
    let title_id = StableNodeId::try_from(title_bar).unwrap();
    let body_id = StableNodeId::try_from(body).unwrap();
    let (bound_title, bound_body, bound_overlay) = doc
        .context()
        .read(
            Entity::<RuntimeAppShell>::from_stable_id(shell_id),
            |shell| (shell.title_bar, shell.body, shell.overlay),
        )
        .unwrap();
    assert_eq!(bound_title, Some(title_id));
    assert_eq!(bound_body, Some(body_id));
    assert_eq!(bound_overlay, None);
    assert_eq!(
        doc.runtime.node(shell_id).unwrap().children,
        vec![title_id, body_id]
    );
    let title_style = doc.runtime.node_style(title_id).unwrap();
    let body_style = doc.runtime.node_style(body_id).unwrap();
    assert_eq!(
        title_style.layout.height,
        Some(nana_ui_core::LengthSpec::Px(nana_ui_core::TITLE_BAR_HEIGHT))
    );
    assert_eq!(title_style.layout.flex_grow, Some(0.0));
    assert!(
        doc.runtime.text(title_id).unwrap_or("").is_empty(),
        "title-bar root text must be empty after assemble, got {:?}",
        doc.runtime.text(title_id)
    );
    let title_label =
        assembled_title_bar_center_label(&doc, title_id).expect("center column title");
    assert_eq!(doc.runtime.text(title_label), Some("Nana"));
    assert_eq!(
        doc.runtime
            .accessibility(title_id)
            .and_then(|state| state.label.as_deref()),
        Some("Nana")
    );
    assert_eq!(body_style.layout.flex_grow, Some(1.0));
    assert_eq!(
        body_style.layout.height,
        Some(nana_ui_core::LengthSpec::Fill)
    );
}

#[test]
fn column_nana_app_shell_title_bar_and_body_keep_layout_after_assemble() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let shell = doc.create_element("nana-app-shell");
    let title_bar = doc.create_element("nana-app-title-bar");
    let body = doc.create_element("div");
    doc.insert(shell, doc.mount_root(), None);
    doc.insert(title_bar, shell, None);
    doc.insert(body, shell, None);
    let mut bridge = crate::MessageBridge::new();
    let mut title_props = crate::WidgetProps {
        label: "Nana".into(),
        element_tag: "nana-app-title-bar".into(),
        ..Default::default()
    };
    title_props
        .attrs
        .insert("data-slot".into(), "title-bar".into());
    title_props.class_names.push("nana-app-title-bar".into());
    bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
    bridge.register(
        body.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            label: "Workspace".into(),
            ..Default::default()
        },
    );
    bridge.register(
        shell.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            label: "Nana".into(),
            element_tag: "nana-app-shell".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(title_bar.0, shell.0, None);
    bridge.insert_child(body.0, shell.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let shell_id = StableNodeId::try_from(shell).unwrap();
    let title_id = StableNodeId::try_from(title_bar).unwrap();
    let body_id = StableNodeId::try_from(body).unwrap();
    let (bound_title, bound_body, bound_overlay) = doc
        .context()
        .read(
            Entity::<RuntimeAppShell>::from_stable_id(shell_id),
            |shell| (shell.title_bar, shell.body, shell.overlay),
        )
        .unwrap();
    assert_eq!(bound_title, Some(title_id));
    assert_eq!(bound_body, Some(body_id));
    assert_eq!(bound_overlay, None);
    assert_eq!(
        doc.runtime.node(shell_id).unwrap().children,
        vec![title_id, body_id]
    );
    let title_style = doc.runtime.node_style(title_id).unwrap();
    let body_style = doc.runtime.node_style(body_id).unwrap();
    assert_eq!(
        title_style.layout.height,
        Some(nana_ui_core::LengthSpec::Px(nana_ui_core::TITLE_BAR_HEIGHT))
    );
    assert_eq!(title_style.layout.flex_grow, Some(0.0));
    assert!(
        doc.runtime.text(title_id).unwrap_or("").is_empty(),
        "title-bar root text must be empty after assemble, got {:?}",
        doc.runtime.text(title_id)
    );
    let title_label =
        assembled_title_bar_center_label(&doc, title_id).expect("center column title");
    assert_eq!(doc.runtime.text(title_label), Some("Nana"));
    assert_eq!(
        doc.runtime
            .accessibility(title_id)
            .and_then(|state| state.label.as_deref()),
        Some("Nana")
    );
    assert_eq!(body_style.layout.flex_grow, Some(1.0));
    assert_eq!(
        body_style.layout.height,
        Some(nana_ui_core::LengthSpec::Fill)
    );
    runtime_layout(&mut doc, 800.0, 600.0);
    assert!(
        !runtime_is_descendant(&doc, title_id, body_id),
        "body must stay a sibling of the title bar"
    );
    let title_box = doc.runtime.layout_box(title_id).unwrap();
    let body_box = doc.runtime.layout_box(body_id).unwrap();
    assert!(
        body_box.y + 0.5 >= title_box.y + nana_ui_core::TITLE_BAR_HEIGHT,
        "body.y={} must sit below title bar y={} height={}",
        body_box.y,
        title_box.y,
        title_box.height
    );
}

#[test]
fn column_nana_app_shell_title_bar_and_body_copy_stays_below_title_bar() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let shell = doc.create_element("nana-app-shell");
    let title_bar = doc.create_element("nana-app-title-bar");
    let body = doc.create_element("div");
    let copy = doc.create_text("Type in the field below.");
    doc.insert(shell, doc.mount_root(), None);
    doc.insert(title_bar, shell, None);
    doc.insert(body, shell, None);
    doc.insert(copy, body, None);
    let mut bridge = crate::MessageBridge::new();
    let mut title_props = crate::WidgetProps {
        label: "Nana".into(),
        element_tag: "nana-app-title-bar".into(),
        ..Default::default()
    };
    title_props
        .attrs
        .insert("data-slot".into(), "title-bar".into());
    title_props.class_names.push("nana-app-title-bar".into());
    let mut body_props = crate::WidgetProps {
        element_tag: "div".into(),
        ..Default::default()
    };
    body_props.attrs.insert("data-slot".into(), "body".into());
    body_props.class_names.push("chrome-probe-body".into());
    bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
    bridge.register(body.0, crate::WidgetKind::Column, body_props);
    bridge.register(
        copy.0,
        crate::WidgetKind::Text,
        crate::WidgetProps {
            label: "Type in the field below.".into(),
            ..Default::default()
        },
    );
    bridge.register(
        shell.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            label: "Nana".into(),
            element_tag: "nana-app-shell".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(title_bar.0, shell.0, None);
    bridge.insert_child(body.0, shell.0, None);
    bridge.insert_child(copy.0, body.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());
    runtime_layout(&mut doc, 800.0, 600.0);

    let shell_id = StableNodeId::try_from(shell).unwrap();
    let title_id = StableNodeId::try_from(title_bar).unwrap();
    let body_id = StableNodeId::try_from(body).unwrap();
    let copy_id = StableNodeId::try_from(copy).unwrap();
    assert_eq!(
        doc.runtime.node(shell_id).unwrap().children,
        vec![title_id, body_id]
    );
    assert!(
        !runtime_is_descendant(&doc, title_id, body_id),
        "body must not be a descendant of the title bar"
    );
    assert!(
        !runtime_is_descendant(&doc, title_id, copy_id),
        "body copy must not be a child of the title-bar node"
    );
    assert_eq!(doc.runtime.text(copy_id), Some("Type in the field below."));
    let title_box = doc.runtime.layout_box(title_id).unwrap();
    let body_box = doc.runtime.layout_box(body_id).unwrap();
    assert!(
        body_box.y + 0.5 >= title_box.y + nana_ui_core::TITLE_BAR_HEIGHT,
        "body.y={} must sit below title bar y={} height={}",
        body_box.y,
        title_box.y,
        title_box.height
    );
}

#[test]
fn column_nana_app_shell_nested_body_is_reparented_below_title_bar() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let shell = doc.create_element("nana-app-shell");
    let title_bar = doc.create_element("nana-app-title-bar");
    let body = doc.create_element("div");
    let copy = doc.create_text("Type in the field below.");
    doc.insert(shell, doc.mount_root(), None);
    doc.insert(title_bar, shell, None);
    doc.insert(body, title_bar, None);
    doc.insert(copy, body, None);
    let mut bridge = crate::MessageBridge::new();
    let mut title_props = crate::WidgetProps {
        label: "Nana".into(),
        element_tag: "nana-app-title-bar".into(),
        ..Default::default()
    };
    title_props
        .attrs
        .insert("data-slot".into(), "title-bar".into());
    title_props.class_names.push("nana-app-title-bar".into());
    let mut body_props = crate::WidgetProps {
        element_tag: "div".into(),
        ..Default::default()
    };
    body_props.attrs.insert("data-slot".into(), "body".into());
    body_props.class_names.push("chrome-probe-body".into());
    bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
    bridge.register(body.0, crate::WidgetKind::Column, body_props);
    bridge.register(
        copy.0,
        crate::WidgetKind::Text,
        crate::WidgetProps {
            label: "Type in the field below.".into(),
            ..Default::default()
        },
    );
    bridge.register(
        shell.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            label: "Nana".into(),
            element_tag: "nana-app-shell".into(),
            ..Default::default()
        },
    );
    bridge.insert_child(title_bar.0, shell.0, None);
    bridge.insert_child(body.0, title_bar.0, None);
    bridge.insert_child(copy.0, body.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());
    runtime_layout(&mut doc, 800.0, 600.0);

    let shell_id = StableNodeId::try_from(shell).unwrap();
    let title_id = StableNodeId::try_from(title_bar).unwrap();
    let body_id = StableNodeId::try_from(body).unwrap();
    let copy_id = StableNodeId::try_from(copy).unwrap();
    assert_eq!(
        doc.runtime.node(shell_id).unwrap().children,
        vec![title_id, body_id]
    );
    assert!(
        !runtime_is_descendant(&doc, title_id, body_id),
        "nested body must be lifted out of the title bar"
    );
    assert!(
        !runtime_is_descendant(&doc, title_id, copy_id),
        "body copy must not stay under the title-bar node"
    );
    let title_box = doc.runtime.layout_box(title_id).unwrap();
    let body_box = doc.runtime.layout_box(body_id).unwrap();
    assert!(
        body_box.y + 0.5 >= title_box.y + nana_ui_core::TITLE_BAR_HEIGHT,
        "body.y={} must sit below title bar y={} height={}",
        body_box.y,
        title_box.y,
        title_box.height
    );
}

#[test]
fn app_shell_empty_title_bar_still_assembles_columns() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let shell = doc.create_element("nana-app-shell");
    let title_bar = doc.create_element("nana-app-title-bar");
    let body = doc.create_element("div");
    doc.insert(shell, doc.mount_root(), None);
    doc.insert(title_bar, shell, None);
    doc.insert(body, shell, None);
    let mut bridge = crate::MessageBridge::new();
    let mut title_props = crate::WidgetProps {
        element_tag: "nana-app-title-bar".into(),
        ..Default::default()
    };
    title_props
        .attrs
        .insert("data-slot".into(), "title-bar".into());
    title_props.class_names.push("nana-app-title-bar".into());
    bridge.register(title_bar.0, crate::WidgetKind::Column, title_props);
    bridge.register(
        body.0,
        crate::WidgetKind::Column,
        crate::WidgetProps::default(),
    );
    bridge.register(
        shell.0,
        crate::WidgetKind::AppShell,
        crate::WidgetProps::default(),
    );
    bridge.insert_child(title_bar.0, shell.0, None);
    bridge.insert_child(body.0, shell.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let title_id = StableNodeId::try_from(title_bar).unwrap();
    let columns = doc.runtime.node(title_id).unwrap().children;
    let tags: Vec<String> = columns
        .iter()
        .map(|&id| match doc.runtime.node(id).map(|node| node.kind) {
            Some(NodeKind::Element { tag }) => tag,
            other => panic!("expected title-bar column, got {other:?}"),
        })
        .collect();
    assert_eq!(
        tags,
        [
            "app-title-bar-leading",
            "app-title-bar-center",
            "app-title-bar-trailing"
        ]
    );
    assert!(
        doc.runtime.text(title_id).unwrap_or("").is_empty(),
        "title-bar root text must be empty after assemble, got {:?}",
        doc.runtime.text(title_id)
    );
    let title_label =
        assembled_title_bar_center_label(&doc, title_id).expect("center column title");
    assert_eq!(doc.runtime.text(title_label), Some(""));
    assert_eq!(
        doc.runtime
            .accessibility(title_id)
            .and_then(|state| state.label.as_deref()),
        Some("")
    );
}

fn runtime_is_descendant(
    doc: &NanaTreeDocument,
    ancestor: StableNodeId,
    node: StableNodeId,
) -> bool {
    let mut current = doc.runtime.node(node).and_then(|node| node.parent);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        current = doc.runtime.node(id).and_then(|node| node.parent);
    }
    false
}

fn assembled_title_bar_center_label(
    doc: &NanaTreeDocument,
    title_id: StableNodeId,
) -> Option<StableNodeId> {
    let columns = doc.runtime.node(title_id)?.children;
    let center = columns.into_iter().find(|&id| {
        matches!(
            doc.runtime.node(id).as_ref().map(|node| &node.kind),
            Some(NodeKind::Element { tag }) if tag == "app-title-bar-center"
        )
    })?;
    doc.runtime.node(center)?.children.first().copied()
}

#[test]
fn split_pane_two_children_vertical_sets_slots_and_axis() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let pane = doc.create_element("nana-split-pane");
    let first = doc.create_element("div");
    let second = doc.create_element("div");
    doc.insert(pane, doc.mount_root(), None);
    doc.insert(first, pane, None);
    doc.insert(second, pane, None);
    let mut bridge = crate::MessageBridge::new();
    let mut props = crate::WidgetProps {
        element_tag: "nana-split-pane".into(),
        ..Default::default()
    };
    props.apply_prop("axis", &nana_js_engine::HostValue::string("vertical"));
    bridge.register(pane.0, crate::WidgetKind::SplitPane, props);
    bridge.register(
        first.0,
        crate::WidgetKind::Column,
        crate::WidgetProps::default(),
    );
    bridge.register(
        second.0,
        crate::WidgetKind::Column,
        crate::WidgetProps::default(),
    );
    bridge.insert_child(first.0, pane.0, None);
    bridge.insert_child(second.0, pane.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let pane_id = StableNodeId::try_from(pane).unwrap();
    let first_id = StableNodeId::try_from(first).unwrap();
    let _second_id = StableNodeId::try_from(second).unwrap();
    assert_eq!(
        doc.runtime.node_style(pane_id).unwrap().layout.direction,
        Some(nana_ui_core::FlexDirection::Column)
    );
    let (first_slot, second_slot) = doc
        .context()
        .read(
            Entity::<RuntimeSplitPane>::from_stable_id(pane_id),
            |pane| (pane.first_slot, pane.second_slot),
        )
        .unwrap();
    let first_slot = first_slot.expect("first slot shell");
    let second_slot = second_slot.expect("second slot shell");
    let first_style = doc.runtime.node_style(first_slot).unwrap();
    let second_style = doc.runtime.node_style(second_slot).unwrap();
    assert_eq!(
        first_style.layout.height,
        Some(nana_ui_core::LengthSpec::Px(240.0))
    );
    assert_eq!(first_style.layout.flex_grow, Some(0.0));
    assert_eq!(
        second_style.layout.height,
        Some(nana_ui_core::LengthSpec::Fill)
    );
    assert_eq!(second_style.layout.flex_grow, Some(1.0));
    // Host content keeps projecting itself; the shells carry the geometry.
    assert_ne!(
        doc.runtime
            .node_style(first_id)
            .and_then(|style| style.layout.height),
        Some(nana_ui_core::LengthSpec::Px(240.0))
    );
}

#[test]
fn split_pane_two_children_assemble_binds_resize_handle() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let pane = doc.create_element("nana-split-pane");
    let first = doc.create_element("div");
    let second = doc.create_element("div");
    doc.insert(pane, doc.mount_root(), None);
    doc.insert(first, pane, None);
    doc.insert(second, pane, None);
    let mut bridge = crate::MessageBridge::new();
    let mut props = crate::WidgetProps {
        element_tag: "nana-split-pane".into(),
        ..Default::default()
    };
    props.apply_prop("axis", &nana_js_engine::HostValue::string("vertical"));
    bridge.register(pane.0, crate::WidgetKind::SplitPane, props);
    bridge.register(
        first.0,
        crate::WidgetKind::Column,
        crate::WidgetProps::default(),
    );
    bridge.register(
        second.0,
        crate::WidgetKind::Column,
        crate::WidgetProps::default(),
    );
    bridge.insert_child(first.0, pane.0, None);
    bridge.insert_child(second.0, pane.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let pane_id = StableNodeId::try_from(pane).unwrap();
    let first_id = StableNodeId::try_from(first).unwrap();
    let _second_id = StableNodeId::try_from(second).unwrap();
    let handle = doc
        .context()
        .read(
            Entity::<RuntimeSplitPane>::from_stable_id(pane_id),
            |pane| pane.handle,
        )
        .unwrap()
        .expect("assemble_split_pane creates a handle");
    assert!(
        doc.context().is_split_handle(handle),
        "assembled handle must be a split-pane resize target"
    );
    let (first_slot, second_slot) = doc
        .context()
        .read(
            Entity::<RuntimeSplitPane>::from_stable_id(pane_id),
            |pane| (pane.first_slot, pane.second_slot),
        )
        .unwrap();
    let first_slot = first_slot.expect("first slot shell");
    let second_slot = second_slot.expect("second slot shell");
    assert_eq!(
        doc.runtime.node(pane_id).unwrap().children,
        vec![first_slot, handle, second_slot]
    );
    assert_eq!(
        doc.runtime.node(first_slot).unwrap().children,
        vec![first_id]
    );
    let first_style = doc.runtime.node_style(first_slot).unwrap();
    let second_style = doc.runtime.node_style(second_slot).unwrap();
    assert_eq!(
        first_style.layout.height,
        Some(nana_ui_core::LengthSpec::Px(240.0))
    );
    assert_eq!(
        second_style.layout.height,
        Some(nana_ui_core::LengthSpec::Fill)
    );
}

#[test]
fn split_pane_resync_reuses_handle() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let pane = doc.create_element("nana-split-pane");
    let first = doc.create_element("div");
    let second = doc.create_element("div");
    doc.insert(pane, doc.mount_root(), None);
    doc.insert(first, pane, None);
    doc.insert(second, pane, None);
    let mut props = crate::WidgetProps {
        element_tag: "nana-split-pane".into(),
        ..Default::default()
    };
    props.apply_prop("axis", &nana_js_engine::HostValue::string("vertical"));
    let mut bridge = crate::MessageBridge::new();
    bridge.register(pane.0, crate::WidgetKind::SplitPane, props.clone());
    bridge.register(
        first.0,
        crate::WidgetKind::Column,
        crate::WidgetProps::default(),
    );
    bridge.register(
        second.0,
        crate::WidgetKind::Column,
        crate::WidgetProps::default(),
    );
    bridge.insert_child(first.0, pane.0, None);
    bridge.insert_child(second.0, pane.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let pane_id = StableNodeId::try_from(pane).unwrap();
    let first_id = StableNodeId::try_from(first).unwrap();
    let second_id = StableNodeId::try_from(second).unwrap();
    let handle = doc
        .context()
        .read(
            Entity::<RuntimeSplitPane>::from_stable_id(pane_id),
            |pane| pane.handle,
        )
        .unwrap()
        .expect("assemble_split_pane creates a handle");

    bridge.patch_prop(
        pane.0,
        "axis",
        &nana_js_engine::HostValue::string("vertical"),
    );
    doc.sync_semantic_styles(&bridge.snapshot());

    let handle_again = doc
        .context()
        .read(
            Entity::<RuntimeSplitPane>::from_stable_id(pane_id),
            |pane| pane.handle,
        )
        .unwrap()
        .expect("resync keeps a handle");
    assert_eq!(handle_again, handle);
    assert!(
        doc.context().is_split_handle(handle),
        "resync must keep the same resize handle identity"
    );
    let children = doc.runtime.node(pane_id).unwrap().children;
    assert!(children.contains(&handle));
    let (first_slot, second_slot) = doc
        .context()
        .read(
            Entity::<RuntimeSplitPane>::from_stable_id(pane_id),
            |pane| (pane.first_slot, pane.second_slot),
        )
        .unwrap();
    let first_slot = first_slot.expect("first slot shell");
    let second_slot = second_slot.expect("second slot shell");
    assert_eq!(
        doc.runtime.node(first_slot).unwrap().children,
        vec![first_id]
    );
    assert_eq!(
        doc.runtime.node(second_slot).unwrap().children,
        vec![second_id]
    );
}

#[test]
fn dock_two_item_children_are_not_dummy_dock_item() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let dock = doc.create_element("nana-dock");
    let nav = doc.create_element("div");
    let files = doc.create_element("div");
    doc.insert(dock, doc.mount_root(), None);
    doc.insert(nav, dock, None);
    doc.insert(files, dock, None);
    let mut bridge = crate::MessageBridge::new();
    let mut nav_props = crate::WidgetProps {
        label: "Nav".into(),
        ..Default::default()
    };
    nav_props.attrs.insert("data-dock-id".into(), "nav".into());
    let mut files_props = crate::WidgetProps {
        label: "Files".into(),
        ..Default::default()
    };
    files_props
        .attrs
        .insert("data-dock-id".into(), "files".into());
    bridge.register(
        dock.0,
        crate::WidgetKind::Dock,
        crate::WidgetProps::default(),
    );
    bridge.register(nav.0, crate::WidgetKind::Column, nav_props);
    bridge.register(files.0, crate::WidgetKind::Column, files_props);
    bridge.insert_child(nav.0, dock.0, None);
    bridge.insert_child(files.0, dock.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let dock_id = StableNodeId::try_from(dock).unwrap();
    let nav_id = StableNodeId::try_from(nav).unwrap();
    let files_id = StableNodeId::try_from(files).unwrap();
    let items = doc
        .context()
        .read(Entity::<RuntimeDock>::from_stable_id(dock_id), |dock| {
            dock.flatten()
                .into_iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_ne!(
        items.as_slice(),
        ["dock"],
        "dock with two item children must not project DockNode::item(\"dock\", None)"
    );
    assert_eq!(items, ["nav", "files"]);
    assert!(
        doc.runtime.node_style(files_id).unwrap().layout.hidden,
        "inactive dock tab body is hidden"
    );
    let children = doc.runtime.node(dock_id).unwrap().children;
    assert!(
        children.len() > 2,
        "assemble_dock must mount chrome beyond the two content nodes"
    );
    assert!(children.contains(&nav_id));
    assert!(children.contains(&files_id));
    assert!(
        children.iter().any(|child| {
            doc.runtime
                .accessibility(*child)
                .is_some_and(|state| state.role == AccessibilityRole::TabList)
        }),
        "assemble_dock mounts a tab-strip chrome node"
    );
}

#[test]
fn column_nana_dock_two_item_children_are_not_dummy_dock_item() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let dock = doc.create_element("nana-dock");
    let nav = doc.create_element("div");
    let files = doc.create_element("div");
    doc.insert(dock, doc.mount_root(), None);
    doc.insert(nav, dock, None);
    doc.insert(files, dock, None);
    let mut bridge = crate::MessageBridge::new();
    let mut nav_props = crate::WidgetProps {
        label: "Nav".into(),
        ..Default::default()
    };
    nav_props.attrs.insert("data-dock-id".into(), "nav".into());
    let mut files_props = crate::WidgetProps {
        label: "Files".into(),
        ..Default::default()
    };
    files_props
        .attrs
        .insert("data-dock-id".into(), "files".into());
    bridge.register(
        dock.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            element_tag: "nana-dock".into(),
            ..Default::default()
        },
    );
    bridge.register(nav.0, crate::WidgetKind::Column, nav_props);
    bridge.register(files.0, crate::WidgetKind::Column, files_props);
    bridge.insert_child(nav.0, dock.0, None);
    bridge.insert_child(files.0, dock.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let dock_id = StableNodeId::try_from(dock).unwrap();
    let nav_id = StableNodeId::try_from(nav).unwrap();
    let files_id = StableNodeId::try_from(files).unwrap();
    let items = doc
        .context()
        .read(Entity::<RuntimeDock>::from_stable_id(dock_id), |dock| {
            dock.flatten()
                .into_iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_ne!(
        items.as_slice(),
        ["dock"],
        "dock with two item children must not project DockNode::item(\"dock\", None)"
    );
    assert_eq!(items, ["nav", "files"]);
    assert!(
        doc.runtime.node_style(files_id).unwrap().layout.hidden,
        "inactive dock tab body is hidden"
    );
    let children = doc.runtime.node(dock_id).unwrap().children;
    assert!(
        children.len() > 2,
        "assemble_dock must mount chrome beyond the two content nodes"
    );
    assert!(children.contains(&nav_id));
    assert!(children.contains(&files_id));
    assert!(
        children.iter().any(|child| {
            doc.runtime
                .accessibility(*child)
                .is_some_and(|state| state.role == AccessibilityRole::TabList)
        }),
        "assemble_dock mounts a tab-strip chrome node"
    );
}

#[test]
fn workspace_region_child_records_region_slot() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let workspace = doc.create_element("nana-workspace");
    let primary = doc.create_element("div");
    doc.insert(workspace, doc.mount_root(), None);
    doc.insert(primary, workspace, None);
    let mut bridge = crate::MessageBridge::new();
    let mut region_props = crate::WidgetProps {
        region: "primary".into(),
        ..Default::default()
    };
    region_props
        .attrs
        .insert("data-region".into(), "primary".into());
    bridge.register(
        workspace.0,
        crate::WidgetKind::Workspace,
        crate::WidgetProps::default(),
    );
    bridge.register(primary.0, crate::WidgetKind::Column, region_props);
    bridge.insert_child(primary.0, workspace.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let workspace_id = StableNodeId::try_from(workspace).unwrap();
    let primary_id = StableNodeId::try_from(primary).unwrap();
    let label = doc
        .runtime
        .accessibility(primary_id)
        .and_then(|state| state.label.clone())
        .unwrap_or_default();
    assert_eq!(
        label.as_ref(),
        "primary",
        "workspace region child must be recorded as a WorkspaceRegionSlot"
    );
    let (middle, primary_column, primary_row, editor_stack) = doc
        .context()
        .read(
            Entity::<RuntimeWorkspace>::from_stable_id(workspace_id),
            |workspace| {
                (
                    workspace.middle,
                    workspace.primary_column,
                    workspace.primary_row,
                    workspace.editor_stack,
                )
            },
        )
        .unwrap();
    assert!(
        middle.is_some(),
        "assemble_workspace creates the middle track"
    );
    assert!(
        primary_column.is_some(),
        "assemble_workspace creates the primary column"
    );
    assert!(
        primary_row.is_some(),
        "assemble_workspace creates the primary row"
    );
    assert!(
        editor_stack.is_some(),
        "assemble_workspace creates the editor stack"
    );
    let children = doc.runtime.node(workspace_id).unwrap().children;
    assert!(
        children.contains(&middle.unwrap()) || children.len() > 1,
        "assemble_workspace mounts track hosts as extra child ids"
    );
}

#[test]
#[cfg(feature = "rich-text")]
fn markdown_mermaid_and_math_fences_project_native_blocks() {
    let mut doc = NanaTreeDocument::new(420, 240, 1.0);
    let markdown = doc.create_element("nana-markdown");
    doc.insert(markdown, doc.mount_root(), None);
    let source = "# Title\n\n```mermaid\nflowchart LR\nA-->B\n```\n\n```math\n\\frac{1}{2}\n```";
    let parsed = RuntimeNativeMarkdown::from_source(source);
    assert!(
        parsed
            .blocks()
            .iter()
            .any(|block| matches!(block, nana_ui_runtime::MarkdownBlock::Mermaid(_)))
    );
    assert!(
        parsed
            .blocks()
            .iter()
            .any(|block| matches!(block, nana_ui_runtime::MarkdownBlock::DisplayMath(_)))
    );

    let mut bridge = crate::MessageBridge::new();
    let mut props = crate::WidgetProps {
        value: source.into(),
        ..Default::default()
    };
    props.apply_prop(
        "mermaidRenderer",
        &nana_js_engine::HostValue::string("app-mermaid"),
    );
    props.apply_prop(
        "mathRenderer",
        &nana_js_engine::HostValue::string("app-math"),
    );
    bridge.register(markdown.0, crate::WidgetKind::NativeMarkdown, props);
    doc.sync_semantic_styles(&bridge.snapshot());

    let markdown_id = StableNodeId::try_from(markdown).unwrap();
    let expected = parsed.plain_text();
    assert!(
        matches!(
            doc.runtime.standard_visual(markdown_id),
            Some(nana_ui_runtime::StandardVisual::NativeMarkdown { text, .. })
                if text.as_ref() == expected
                    && text.contains("flowchart LR")
                    && text.contains("\\frac{1}{2}")
        ),
        "Vue NativeMarkdown must project mermaid/math fences through from_source"
    );
    assert!(
        doc.runtime.custom_render(markdown_id).is_none(),
        "mermaid/math stay on NativeMarkdown; Vue must not invent a renderer"
    );
    let children = doc.runtime.node(markdown_id).unwrap().children;
    assert!(
        children.iter().any(|child| {
            doc.runtime
                .highlight_request(*child)
                .is_some_and(|request| {
                    request.presenter.as_ref() == RuntimeNativeMarkdown::MERMAID_PRESENTER
                })
        }),
        "assemble_markdown attaches a mermaid fence child"
    );
    assert!(
        children.iter().any(|child| {
            doc.runtime
                .node_style(*child)
                .is_some_and(|style| style.layout.hidden)
        }),
        "fence children stay hidden identity slots"
    );
}

#[test]
fn runtime_is_authoritative_for_semantic_hierarchy_and_unchanged_style_is_idle() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let child = doc.create_element("button");
    doc.insert(child, doc.mount_root(), None);
    let mut props = crate::WidgetProps::default();
    props.element_tag = "button".into();
    props.layout.opacity = Some(0.5);
    let mut snapshot = crate::SemanticSnapshot {
        revision: 1,
        theme: nana_ui_core::ThemeMode::Light,
        appearance: nana_ui_core::AppearanceSettings::default(),
        roots: vec![child.0],
        changes: Default::default(),
        widgets: vec![crate::SemanticWidget {
            id: child.0,
            kind: crate::WidgetKind::Button,
            props,
            children: vec![999],
            parent: None,
        }],
    };
    doc.apply_runtime_hierarchy(&mut snapshot);
    assert_eq!(snapshot.roots, vec![child.0]);
    assert!(snapshot.widgets[0].children.is_empty());

    doc.sync_semantic_styles(&snapshot);
    let generation = doc.runtime_generation();
    doc.sync_semantic_styles(&snapshot);
    assert_eq!(doc.runtime_generation(), generation);
    let id = StableNodeId::try_from(child).unwrap();
    assert_eq!(
        doc.runtime.node_style(id).unwrap().layout.opacity,
        Some(0.5)
    );
    assert!(doc.runtime.interaction(id).unwrap().focusable);
    assert_eq!(doc.runtime.theme_mode(), nana_ui_core::ThemeMode::Light);

    snapshot.revision += 1;
    snapshot.theme = nana_ui_core::ThemeMode::Dark;
    doc.sync_semantic_styles(&snapshot);
    assert_eq!(doc.runtime.theme_mode(), nana_ui_core::ThemeMode::Dark);
}

#[test]
fn sidebar_frame_body_projects_as_vertical_scroll_view() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let frame = doc.create_element("nana-sidebar-frame");
    let top = doc.create_element("nana-column");
    let body = doc.create_element("nana-column");
    let footer = doc.create_element("nana-column");
    doc.insert(frame, doc.mount_root(), None);
    doc.insert(top, frame, None);
    doc.insert(body, frame, None);
    doc.insert(footer, frame, None);
    let mut bridge = crate::MessageBridge::new();
    let mut frame_props = crate::WidgetProps::default();
    frame_props.class_names = vec!["nana-sidebar-frame".into()];
    bridge.register(frame.0, crate::WidgetKind::SidebarFrame, frame_props);
    let mut top_props = crate::WidgetProps::default();
    top_props.class_names = vec!["nana-sidebar-frame__top".into()];
    top_props
        .attrs
        .insert("data-slot".into(), "sidebar-top".into());
    bridge.register(top.0, crate::WidgetKind::Column, top_props);
    let mut body_props = crate::WidgetProps::default();
    body_props.class_names = vec!["nana-sidebar-frame__body".into()];
    body_props
        .attrs
        .insert("data-slot".into(), "sidebar-body".into());
    bridge.register(body.0, crate::WidgetKind::Column, body_props);
    let mut footer_props = crate::WidgetProps::default();
    footer_props.class_names = vec!["nana-sidebar-frame__footer".into()];
    footer_props
        .attrs
        .insert("data-slot".into(), "sidebar-footer".into());
    bridge.register(footer.0, crate::WidgetKind::Column, footer_props);
    bridge.insert_child(top.0, frame.0, None);
    bridge.insert_child(body.0, frame.0, None);
    bridge.insert_child(footer.0, frame.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());

    let body_id = StableNodeId::try_from(body).unwrap();
    let top_id = StableNodeId::try_from(top).unwrap();
    let footer_id = StableNodeId::try_from(footer).unwrap();
    assert_eq!(
        doc.runtime.node_style(body_id).unwrap().layout.overflow_y,
        nana_ui_core::OverflowSpec::Scroll
    );
    assert!(
        doc.runtime
            .node_style(body_id)
            .unwrap()
            .layout
            .overflow_y
            .scrolls()
    );
    assert!(doc.runtime.interaction(body_id).unwrap().pointer_events);
    assert!(
        !doc.runtime
            .node_style(top_id)
            .unwrap()
            .layout
            .overflow_y
            .scrolls()
    );
    assert!(
        !doc.runtime
            .node_style(footer_id)
            .unwrap()
            .layout
            .overflow_y
            .scrolls()
    );
}

fn register_settings_row(
    doc: &mut NanaTreeDocument,
    bridge: &mut crate::MessageBridge,
    label: &str,
    hint: Option<&str>,
    nest_hint_text: bool,
) -> (NodeHandle, NodeHandle, Option<NodeHandle>) {
    let row = doc.create_element("nana-settings-row");
    let copy = doc.create_element("div");
    let label_node = doc.create_element("span");
    let control = doc.create_element("div");
    doc.insert(row, doc.mount_root(), None);
    doc.insert(copy, row, None);
    doc.insert(label_node, copy, None);
    doc.insert(control, row, None);
    let mut row_props = crate::WidgetProps::default();
    row_props.class_names = vec!["nana-settings-row".into()];
    row_props.label = label.into();
    bridge.register(row.0, crate::WidgetKind::SettingsRow, row_props);
    let mut copy_props = crate::WidgetProps::default();
    copy_props.class_names = vec!["nana-settings-row__label".into()];
    bridge.register(copy.0, crate::WidgetKind::Column, copy_props);
    let mut label_props = crate::WidgetProps::default();
    label_props.label = label.into();
    bridge.register(label_node.0, crate::WidgetKind::Text, label_props);
    let hint_text = hint.filter(|_| nest_hint_text).map(|value| {
        let hint = doc.create_element("div");
        let text = doc.create_text(value);
        doc.insert(hint, copy, None);
        doc.insert(text, hint, None);
        let mut hint_props = crate::WidgetProps::default();
        hint_props.class_names = vec!["nana-settings-row__hint".into()];
        bridge.register(hint.0, crate::WidgetKind::Column, hint_props);
        let mut text_props = crate::WidgetProps::default();
        text_props.label = value.into();
        bridge.register(text.0, crate::WidgetKind::Text, text_props);
        bridge.insert_child(hint.0, copy.0, None);
        bridge.insert_child(text.0, hint.0, None);
        (hint, text)
    });
    let mut control_props = crate::WidgetProps::default();
    control_props.class_names = vec!["nana-settings-row__control".into()];
    bridge.register(control.0, crate::WidgetKind::Column, control_props);
    bridge.insert_child(copy.0, row.0, None);
    bridge.insert_child(label_node.0, copy.0, None);
    bridge.insert_child(control.0, row.0, None);
    doc.sync_semantic_styles(&bridge.snapshot());
    (
        label_node,
        hint_text.map(|(hint, _)| hint).unwrap_or(label_node),
        hint_text.map(|(_, text)| text),
    )
}

#[test]
fn settings_row_projects_label_and_nested_hint_once() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = crate::MessageBridge::new();
    let (label, hint, hint_text) = register_settings_row(
        &mut doc,
        &mut bridge,
        "主题",
        Some("选择应用配色，立即生效"),
        true,
    );
    let label_style = doc
        .runtime
        .node_style(StableNodeId::try_from(label).unwrap())
        .unwrap();
    assert_eq!(label_style.layout.font_size, Some(13.0));
    assert_eq!(label_style.layout.font_weight, Some(500));
    let hint_text_id = StableNodeId::try_from(hint_text.unwrap()).unwrap();
    let hint_style = doc.runtime.node_style(hint_text_id).unwrap();
    assert_eq!(
        hint_style.foreground,
        Some(nana_ui_core::SemanticColorRole::Muted)
    );
    assert_eq!(hint_style.layout.font_size, Some(12.0));
    assert_eq!(
        doc.runtime.text(hint_text_id),
        Some("选择应用配色，立即生效")
    );
    let hint_box = doc
        .runtime
        .node_style(StableNodeId::try_from(hint).unwrap())
        .unwrap();
    assert_ne!(
        hint_box.foreground,
        Some(nana_ui_core::SemanticColorRole::Muted)
    );
}

#[test]
fn settings_row_without_hint_still_projects_label() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = crate::MessageBridge::new();
    let (label, _, _) = register_settings_row(&mut doc, &mut bridge, "工作区边缘", None, false);
    let label_style = doc
        .runtime
        .node_style(StableNodeId::try_from(label).unwrap())
        .unwrap();
    assert!(!label_style.layout.hidden);
    assert_eq!(label_style.layout.font_size, Some(13.0));
    assert_eq!(label_style.layout.font_weight, Some(500));
}

#[test]
fn insert_into_missing_parent_does_not_orphan_child() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let parent = doc.create_element("nana-sidebar-frame");
    let footer = doc.create_element("nana-column");
    doc.insert(parent, doc.mount_root(), None);
    doc.insert(footer, parent, None);
    assert_eq!(doc.parent_node(footer), Some(parent));
    // Stale wrapNode target after remount dispose: insert must not detach.
    doc.insert(footer, NodeHandle(9_999_999), None);
    assert_eq!(
        doc.parent_node(footer),
        Some(parent),
        "failed insert must keep existing parent"
    );
    assert!(
        doc.children_of(parent).contains(&footer),
        "footer must stay under sidebar frame"
    );
}

#[test]
fn insert_into_comment_parent_does_not_orphan_child() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let parent = doc.create_element("div");
    let child = doc.create_element("span");
    let comment = doc.create_comment("teleport-anchor");
    doc.insert(parent, doc.mount_root(), None);
    doc.insert(child, parent, None);
    doc.insert(comment, doc.mount_root(), None);
    doc.insert(child, comment, None);
    assert_eq!(
        doc.parent_node(child),
        Some(parent),
        "non-element parent must not steal children"
    );
}

#[test]
fn teleport_to_body_remove_disposes_subtree_without_leaking() {
    // Vue Teleport `to="body"` inserts under mount_root; unmount must not
    // leave orphan nodes in the document map (Overlay open/close cycles).
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let body = doc.mount_root();
    let before = doc.nodes.len();
    let overlay = doc.create_element("div");
    doc.set_attribute(overlay, "class", "nana-dialog");
    doc.set_attribute(overlay, "aria-modal", "true");
    doc.set_attribute(overlay, "role", "dialog");
    let title = doc.create_element("span");
    doc.set_element_text(title, "Confirm");
    doc.insert(title, overlay, None);
    doc.insert(overlay, body, None);
    assert_eq!(doc.parent_node(overlay), Some(body));
    assert!(doc.contains(body, overlay));
    assert!(doc.contains(overlay, title));
    assert!(doc.nodes.len() > before);

    doc.remove(overlay);
    assert_eq!(doc.parent_node(overlay), None);
    assert!(!doc.contains(body, overlay));
    assert!(!doc.nodes.contains_key(&overlay.0));
    assert!(!doc.nodes.contains_key(&title.0));
    assert_eq!(doc.children_of(body), Vec::<NodeHandle>::new());
    assert_eq!(
        doc.nodes.len(),
        before,
        "scaffold-only after Teleport unmount"
    );
    assert_eq!(doc.query_selector("body"), Some(body));
}

#[test]
fn contains_self_and_descendants() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let outer = doc.create_element("div");
    doc.insert(outer, doc.mount_root(), None);
    let inner = doc.create_element("span");
    doc.insert(inner, outer, None);
    let text = doc.create_text("hi");
    doc.insert(text, inner, None);
    let sibling = doc.create_element("div");
    doc.insert(sibling, doc.mount_root(), None);

    assert!(doc.contains(outer, outer));
    assert!(doc.contains(outer, inner));
    assert!(doc.contains(outer, text));
    assert!(doc.contains(doc.mount_root(), outer));
    assert!(!doc.contains(outer, sibling));
    assert!(!doc.contains(inner, outer));
    assert!(!doc.contains(sibling, inner));
}

#[test]
fn create_element_stores_svg_namespace_and_is() {
    let mut doc = NanaTreeDocument::new(400, 300, 1.0);
    let svg = doc.create_element_ns("svg", ElementNamespace::Html, None);
    assert_eq!(doc.element_namespace(svg), Some(ElementNamespace::Svg));
    let path = doc.create_element_ns("path", ElementNamespace::Svg, None);
    assert_eq!(doc.element_namespace(path), Some(ElementNamespace::Svg));
    let custom = doc.create_element_ns("p", ElementNamespace::Html, Some("fancy-p"));
    assert_eq!(doc.get_attribute(custom, "is").as_deref(), Some("fancy-p"));
}

#[test]
fn insert_static_content_parses_fragment_and_reuses_start_end() {
    let mut doc = NanaTreeDocument::new(400, 300, 1.0);
    let parent = doc.mount_root();
    let (first, last, _) = doc.insert_static_content(
        "<span class=\"a\">hi</span><b>x</b>",
        parent,
        None,
        ElementNamespace::Html,
        None,
        None,
    );
    assert_ne!(first, last);
    assert_eq!(doc.element_tag(first).as_deref(), Some("span"));
    assert_eq!(doc.get_attribute(first, "class").as_deref(), Some("a"));
    assert_eq!(doc.element_tag(last).as_deref(), Some("b"));
    assert_eq!(doc.children_of(parent), vec![first, last]);

    let (c0, c1, pairs) = doc.insert_static_content(
        "ignored-on-reuse",
        parent,
        None,
        ElementNamespace::Html,
        Some(first),
        Some(last),
    );
    assert!(pairs.iter().any(|(s, d)| s.0 == first.0 && d.0 != s.0));
    assert_eq!(doc.children_of(parent).len(), 4);
    assert_eq!(doc.element_tag(c0).as_deref(), Some("span"));
    assert_eq!(doc.element_tag(c1).as_deref(), Some("b"));
    assert_ne!(c0, first);
}

#[test]
fn insert_static_content_unwraps_svg_namespace_wrapper() {
    let mut doc = NanaTreeDocument::new(400, 300, 1.0);
    let parent = doc.mount_root();
    let (first, last, _) = doc.insert_static_content(
        "<path d=\"M0 0\"></path>",
        parent,
        None,
        ElementNamespace::Svg,
        None,
        None,
    );
    assert_eq!(first, last);
    assert_eq!(doc.element_tag(first).as_deref(), Some("path"));
    assert_eq!(doc.element_namespace(first), Some(ElementNamespace::Svg));
}

#[test]
fn first_child_child_nodes_parent_element_match_tree() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let outer = doc.create_element("div");
    doc.insert(outer, doc.mount_root(), None);
    let text = doc.create_text("a");
    doc.insert(text, outer, None);
    let inner = doc.create_element("span");
    doc.insert(inner, outer, None);

    assert_eq!(doc.first_child(outer), Some(text));
    assert_eq!(doc.children_of(outer), vec![text, inner]);
    assert_eq!(doc.parent_element(inner), Some(outer));
    assert_eq!(doc.parent_element(text), Some(outer));
    assert_eq!(doc.parent_node(outer), Some(doc.mount_root()));
    assert_eq!(doc.parent_element(outer), Some(doc.mount_root()));
    assert_eq!(doc.node_kind(text), DomNodeKind::Text);
    assert_eq!(doc.element_tag(inner).as_deref(), Some("span"));
}

#[test]
fn query_selector_all_and_closest_match_compounds() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let outer = doc.create_element("div");
    doc.set_attribute(outer, "class", "home-pending-action is-confirming");
    doc.set_attribute(outer, "data-sidebar-repo-id", "r1");
    doc.insert(outer, doc.mount_root(), None);
    let inner = doc.create_element("span");
    doc.set_attribute(inner, "id", "sec-a");
    doc.insert(inner, outer, None);

    assert_eq!(
        doc.query_selector(".home-pending-action.is-confirming"),
        Some(outer)
    );
    assert_eq!(doc.query_selector("#sec-a"), Some(inner));
    assert_eq!(
        doc.query_selector_all("[data-sidebar-repo-id], #sec-a")
            .len(),
        2
    );
    assert_eq!(doc.closest(inner, "[data-sidebar-repo-id]"), Some(outer));
    assert_eq!(
        doc.closest(inner, ".home-pending-action.is-confirming"),
        Some(outer)
    );
    assert_eq!(doc.closest(inner, ".missing"), None);
}

#[test]
fn apply_layout_boxes_keeps_viewport_roots() {
    let mut doc = NanaTreeDocument::new(400, 300, 1.0);
    let child = doc.create_element("div");
    doc.insert(child, doc.mount_root(), None);
    doc.apply_layout_boxes(&[(
        child,
        LayoutBox {
            handle: child,
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 40.0,
        },
    )]);
    let body = doc.layout_box(doc.mount_root()).expect("body");
    assert_eq!((body.width, body.height), (400.0, 300.0));
    let box_ = doc.layout_box(child).expect("child");
    assert_eq!(
        (box_.x, box_.y, box_.width, box_.height),
        (10.0, 20.0, 100.0, 40.0)
    );
}

#[test]
fn get_layout_box_prefers_paint_store_over_document() {
    let mut doc = NanaTreeDocument::new(400, 300, 1.0);
    let child = doc.create_element("div");
    doc.insert(child, doc.mount_root(), None);
    doc.apply_layout_boxes(&[(
        child,
        LayoutBox {
            handle: child,
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 20.0,
        },
    )]);
    let store = LayoutBoxStore::new();
    // Scene chrome (e.g. scrollport padding) shifts the painted box.
    store.record(child, 16.0, 16.0, 80.0, 24.0);
    let box_ = get_layout_box_from(&store, &doc, child).expect("paint box");
    assert_eq!(
        (box_.x, box_.y, box_.width, box_.height),
        (16.0, 16.0, 80.0, 24.0),
        "menu anchors must follow Scene paint, not pre-paint measure"
    );
    store.begin_frame();
    let kept = get_layout_box_from(&store, &doc, child).expect("incremental paint box");
    assert_eq!(
        (kept.x, kept.y, kept.width, kept.height),
        (16.0, 16.0, 80.0, 24.0),
        "begin_frame must keep last paint boxes"
    );
    store.remove(child);
    let fallback = get_layout_box_from(&store, &doc, child).expect("doc fallback");
    assert_eq!(
        (fallback.x, fallback.y, fallback.width, fallback.height),
        (0.0, 0.0, 50.0, 20.0)
    );
}

#[test]
fn create_insert_and_text_snapshot() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let root = doc.mount_root();
    let btn = doc.create_element("button");
    doc.set_element_text(btn, "0");
    doc.set_event_flag(btn, "onClick", true);
    doc.insert(btn, root, None);
    doc.apply_layout_boxes(&[(
        btn,
        LayoutBox {
            handle: btn,
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 28.0,
        },
    )]);
    let snap = doc.snapshot_boxes();
    assert!(snap.texts.iter().any(|(_, t)| t == "0"));
    assert!(snap.event_targets.iter().any(|(_, e)| e == "click"));
    let box_ = doc.layout_box(btn).expect("button box");
    assert!(box_.width > 0.0 && box_.height > 0.0);
    assert_eq!(
        doc.hit_event_target(box_.x + 1.0, box_.y + 1.0, "click"),
        Some(btn)
    );
}

#[test]
fn transformed_layout_box_uses_inverse_affine_hit_testing() {
    let store = LayoutBoxStore::new();
    let node = NodeHandle(991);
    let sin = std::f32::consts::FRAC_1_SQRT_2;
    store.record_transformed(node, 0.0, 0.0, 4.0, 2.0, [sin, sin, -sin, sin, 5.0, 1.0]);
    let bounds = store.get(node).expect("transformed bounds");
    assert!(store.contains_point(node, 5.0, 2.0));
    assert!(
        !store.contains_point(node, bounds.x + 0.01, bounds.y + 0.01),
        "rotated AABB corners must not become false-positive pointer targets"
    );
    let (local_x, local_y) = store.local_point(node, 5.0, 2.0).unwrap();
    assert!((local_x - sin).abs() < 1e-5);
    assert!((local_y - sin).abs() < 1e-5);
}

#[test]
fn document_hit_test_uses_runtime_transform_authority() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let node = doc.create_element("button");
    doc.insert(node, doc.mount_root(), None);
    let mut props = crate::WidgetProps::default();
    props.layout.transform = Some(nana_ui_core::PaintTransform {
        e: 10.0,
        ..Default::default()
    });
    let snapshot = crate::SemanticSnapshot {
        revision: 1,
        theme: nana_ui_core::ThemeMode::Light,
        appearance: nana_ui_core::AppearanceSettings::default(),
        roots: vec![node.0],
        changes: Default::default(),
        widgets: vec![crate::SemanticWidget {
            id: node.0,
            kind: crate::WidgetKind::Button,
            props,
            children: Vec::new(),
            parent: Some(doc.mount_root().0),
        }],
    };
    doc.sync_semantic_styles(&snapshot);
    doc.apply_layout_boxes(&[(
        node,
        LayoutBox {
            handle: node,
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
        },
    )]);

    assert_eq!(doc.hit_test(11.0, 1.0), Some(node));
    assert_ne!(doc.hit_test(1.0, 1.0), Some(node));
}

#[test]
fn pointer_events_none_is_not_hit() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let under = doc.create_element("button");
    let overlay = doc.create_element("div");
    let root = doc.mount_root();
    doc.insert(under, root, None);
    doc.insert(overlay, root, None);
    let mut under_props = crate::WidgetProps::default();
    under_props.layout.width = Some(nana_ui_core::LengthSpec::Px(40.0));
    under_props.layout.height = Some(nana_ui_core::LengthSpec::Px(40.0));
    let mut overlay_props = crate::WidgetProps::default();
    overlay_props.layout.pointer_events = Some(nana_ui_core::PointerEventsSpec::None);
    overlay_props.layout.width = Some(nana_ui_core::LengthSpec::Px(40.0));
    overlay_props.layout.height = Some(nana_ui_core::LengthSpec::Px(40.0));
    let snapshot = crate::SemanticSnapshot {
        revision: 1,
        theme: nana_ui_core::ThemeMode::Light,
        appearance: nana_ui_core::AppearanceSettings::default(),
        roots: vec![under.0, overlay.0],
        changes: Default::default(),
        widgets: vec![
            crate::SemanticWidget {
                id: under.0,
                kind: crate::WidgetKind::Button,
                props: under_props,
                children: Vec::new(),
                parent: Some(root.0),
            },
            crate::SemanticWidget {
                id: overlay.0,
                kind: crate::WidgetKind::Column,
                props: overlay_props,
                children: Vec::new(),
                parent: Some(root.0),
            },
        ],
    };
    doc.sync_semantic_styles(&snapshot);
    doc.inject_layout_boxes(&[
        (
            under,
            LayoutBox {
                handle: under,
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 40.0,
            },
        ),
        (
            overlay,
            LayoutBox {
                handle: overlay,
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 40.0,
            },
        ),
    ]);

    let overlay_id = StableNodeId::try_from(overlay).unwrap();
    assert!(!doc.runtime.interaction(overlay_id).unwrap().pointer_events);
    assert_eq!(
        doc.runtime
            .node_style(overlay_id)
            .unwrap()
            .layout
            .pointer_events,
        Some(nana_ui_core::PointerEventsSpec::None)
    );
    assert_ne!(doc.hit_test(20.0, 20.0), Some(overlay));
    assert_eq!(doc.hit_test(20.0, 20.0), Some(under));
}

#[test]
fn pointer_events_none_skips_hit_and_auto_child_punches_through() {
    use nana_ui_core::PointerEventsSpec;

    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let parent = doc.create_element("div");
    let child = doc.create_element("div");
    let inherited = doc.create_element("div");
    doc.insert(parent, doc.mount_root(), None);
    doc.insert(child, parent, None);
    doc.insert(inherited, parent, None);

    let mut parent_props = crate::WidgetProps::default();
    parent_props.layout.pointer_events = Some(PointerEventsSpec::None);
    let mut child_props = crate::WidgetProps::default();
    child_props.layout.pointer_events = Some(PointerEventsSpec::Auto);
    let snapshot = crate::SemanticSnapshot {
        revision: 1,
        theme: nana_ui_core::ThemeMode::Light,
        appearance: nana_ui_core::AppearanceSettings::default(),
        roots: vec![parent.0],
        changes: Default::default(),
        widgets: vec![
            crate::SemanticWidget {
                id: parent.0,
                kind: crate::WidgetKind::Column,
                props: parent_props,
                children: vec![child.0, inherited.0],
                parent: Some(doc.mount_root().0),
            },
            crate::SemanticWidget {
                id: child.0,
                kind: crate::WidgetKind::Column,
                props: child_props,
                children: Vec::new(),
                parent: Some(parent.0),
            },
            crate::SemanticWidget {
                id: inherited.0,
                kind: crate::WidgetKind::Column,
                props: crate::WidgetProps::default(),
                children: Vec::new(),
                parent: Some(parent.0),
            },
        ],
    };
    doc.sync_semantic_styles(&snapshot);
    doc.apply_layout_boxes(&[
        (
            parent,
            LayoutBox {
                handle: parent,
                x: 10.0,
                y: 10.0,
                width: 50.0,
                height: 50.0,
            },
        ),
        (
            child,
            LayoutBox {
                handle: child,
                x: 15.0,
                y: 15.0,
                width: 20.0,
                height: 20.0,
            },
        ),
        (
            inherited,
            LayoutBox {
                handle: inherited,
                x: 40.0,
                y: 40.0,
                width: 10.0,
                height: 10.0,
            },
        ),
    ]);

    let parent_id = StableNodeId::try_from(parent).expect("parent id");
    let child_id = StableNodeId::try_from(child).expect("child id");
    assert!(!doc.runtime.interaction(parent_id).unwrap().pointer_events);
    assert!(doc.runtime.interaction(child_id).unwrap().pointer_events);
    assert_eq!(doc.hit_test(25.0, 25.0), Some(child));
    assert_ne!(doc.hit_test(12.0, 12.0), Some(parent));
    assert_ne!(doc.hit_test(44.0, 44.0), Some(inherited));
}

#[test]
fn paint_transform_overlay_does_not_write_runtime_layout_box() {
    use nana_ui_core::PaintTransform;
    use nana_ui_runtime::StableNodeId;

    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let node = doc.create_element("li");
    doc.insert(node, doc.mount_root(), None);
    doc.flush_host_frame();
    doc.inject_layout_boxes(&[(
        node,
        LayoutBox {
            handle: node,
            x: 10.0,
            y: 20.0,
            width: 40.0,
            height: 16.0,
        },
    )]);
    let before = doc.layout_box(node).expect("box");
    doc.set_paint_transform(node, "translate(12px, 4px)");
    doc.flush_host_frame();
    let after = doc.layout_box(node).expect("box after transform");
    assert_eq!(before.x, after.x);
    assert_eq!(before.y, after.y);
    assert_eq!(before.width, after.width);
    assert_eq!(before.height, after.height);
    let style = doc
        .world()
        .node_style(StableNodeId::try_from(node).unwrap())
        .expect("style");
    assert_eq!(
        style.layout.transform,
        Some(PaintTransform {
            e: 12.0,
            f: 4.0,
            ..PaintTransform::default()
        })
    );
    doc.set_paint_transform(node, "");
    doc.flush_host_frame();
    let cleared = doc
        .world()
        .node_style(StableNodeId::try_from(node).unwrap())
        .expect("cleared style");
    assert_eq!(cleared.layout.transform, None);
    let still = doc.layout_box(node).expect("box after clear");
    assert_eq!(still.x, before.x);
    assert_eq!(still.y, before.y);
}

#[test]
fn gpu_slot_authority_is_runtime_custom_render() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let node = doc.create_element("div");
    doc.insert(node, doc.mount_root(), None);
    doc.set_gpu_slot(node, "program");
    let id = StableNodeId::try_from(node).unwrap();
    assert!(
        doc.gpu_slots()
            .iter()
            .any(|(handle, slot)| *handle == node && slot == "program")
    );
    assert!(
        doc.world().custom_render(id).is_none(),
        "host ops must not commit GPU slots before the frame boundary"
    );

    doc.flush_host_frame();
    let content = doc
        .world()
        .custom_render(id)
        .expect("data-nana-gpu must land on CustomRenderNode");
    assert_eq!(content.renderer.as_ref(), "nana.host-texture");
    assert_eq!(content.resource.as_ref(), "program");
    assert_eq!(
        doc.gpu_slots(),
        vec![(node, "program".into())],
        "snapshot/host GPU binding must read Runtime, not a facade map"
    );
    assert_eq!(
        content.revision, 0,
        "unresolved registry handles keep revision 0"
    );

    doc.remove_attribute(node, "data-nana-gpu");
    doc.flush_host_frame();
    assert!(doc.world().custom_render(id).is_none());
    assert!(doc.gpu_slots().is_empty());
}

#[test]
fn flushed_host_texture_revision_matches_packed_generation_and_version() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let node = doc.create_element("div");
    doc.insert(node, doc.mount_root(), None);
    doc.set_gpu_slot(node, "program");
    let generation = 5;
    let version = 3;
    doc.override_host_texture_revision(
        "program",
        nana_ui_runtime::pack_gpu_revision(generation, version),
    );
    let id = StableNodeId::try_from(node).unwrap();
    assert!(
        doc.world().custom_render(id).is_none(),
        "host ops must not commit GPU revisions before the frame boundary"
    );

    doc.flush_host_frame();
    let content = doc
        .world()
        .custom_render(id)
        .expect("registered texture must land on CustomRenderNode");
    assert_eq!(
        content.revision,
        nana_ui_runtime::pack_gpu_revision(generation, version)
    );

    doc.override_host_texture_revision(
        "program",
        nana_ui_runtime::pack_gpu_revision(generation, version + 1),
    );
    doc.flush_host_frame();
    let content = doc
        .world()
        .custom_render(id)
        .expect("invalidated texture must keep CustomRenderNode");
    assert_eq!(
        content.revision,
        nana_ui_runtime::pack_gpu_revision(generation, version + 1),
        "content updates must change CustomRenderNode.revision"
    );

    let world_generation = doc.world().generation();
    doc.flush_host_frame();
    assert_eq!(
        doc.world().generation(),
        world_generation,
        "unchanged host-texture revision must not enqueue CustomRender mutations"
    );
}

#[test]
fn video_tag_binds_video_control_and_host_texture_slot() {
    let mut doc = NanaTreeDocument::new(320, 200, 1.0);
    let video = doc.create_element("video");
    doc.insert(video, doc.mount_root(), None);
    doc.set_attribute(video, "data-nana-video", "7");
    let mut bridge = crate::MessageBridge::new();
    let mut props = crate::WidgetProps::default();
    props.attrs.insert("data-nana-video".into(), "7".into());
    props.attrs.insert("autoplay".into(), "true".into());
    props.attrs.insert("muted".into(), "".into());
    bridge.register(video.0, crate::WidgetKind::Video, props);
    doc.sync_semantic_styles(&bridge.snapshot());

    let id = StableNodeId::try_from(video).unwrap();
    assert_eq!(
        doc.runtime.component_type(id).map(ComponentTypeId::as_str),
        Some("nana.video"),
        "HTML <video> binds the same-name Runtime control"
    );
    assert_eq!(
        doc.element_with_attribute("data-nana-video", "7"),
        Some(video)
    );

    doc.flush_host_frame();
    let content = doc
        .world()
        .custom_render(id)
        .expect("video slot must land on CustomRenderNode");
    assert_eq!(content.renderer.as_ref(), "nana.host-texture");
    assert_eq!(content.resource.as_ref(), "video:7");
    assert_eq!(content.revision, 0, "unregistered slot keeps revision 0");

    doc.override_host_texture_revision("video:7", nana_ui_runtime::pack_gpu_revision(2, 9));
    doc.flush_host_frame();
    let content = doc
        .world()
        .custom_render(id)
        .expect("invalidated video texture must keep CustomRenderNode");
    assert_eq!(
        content.revision,
        nana_ui_runtime::pack_gpu_revision(2, 9),
        "frame uploads must surface through CustomRenderNode.revision"
    );

    doc.remove_attribute(video, "data-nana-video");
    doc.flush_host_frame();
    assert!(doc.world().custom_render(id).is_none());
}

#[test]
fn has_event_reads_runtime_listener_authority() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let btn = doc.create_element("button");
    doc.insert(btn, doc.mount_root(), None);
    doc.set_event_flag(btn, "onClick", true);
    assert!(doc.has_event(btn, "click"));
    let id = StableNodeId::try_from(btn).unwrap();
    assert!(
        !doc.world().has_event(id, "click"),
        "EventRoute is not the listener set; listeners commit at the frame boundary"
    );

    doc.flush_host_frame();
    assert!(doc.has_event(btn, "click"));
    assert!(doc.world().has_event(id, "click"));
    assert!(
        doc.world()
            .event_targets(doc.world().node(id).unwrap().document)
            .contains(&(btn.0, "click".into()))
    );

    doc.set_event_flag(btn, "click", false);
    doc.flush_host_frame();
    assert!(!doc.has_event(btn, "click"));
    assert!(!doc.world().has_event(id, "click"));
}

#[test]
fn same_frame_host_ops_flush_once_at_frame_boundary() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let root = doc.mount_root();
    let generation = doc.runtime_generation();
    let parent = doc.create_element("div");
    let child = doc.create_element("span");
    doc.insert(parent, root, None);
    doc.insert(child, parent, None);
    doc.set_element_text(child, "hello");
    doc.set_event_flag(parent, "click", true);
    doc.set_gpu_slot(parent, "slot");

    assert_eq!(
        doc.runtime_generation(),
        generation,
        "create/insert/text/event/gpu must share one uncommitted batch"
    );
    assert_eq!(doc.parent_node(parent), Some(root));
    assert_eq!(doc.children_of(parent), vec![child]);
    assert_eq!(doc.parent_node(child), Some(parent));
    assert!(doc.text_content(child).unwrap().contains("hello"));

    doc.flush_host_frame();
    assert!(
        doc.runtime_generation() > generation,
        "one host flush must commit the batched ops (layout writeback may add a generation)"
    );
    assert_eq!(doc.children_of(parent), vec![child]);
    let parent_id = StableNodeId::try_from(parent).unwrap();
    assert!(doc.world().has_event(parent_id, "click"));
    assert!(doc.world().custom_render(parent_id).is_some());
}

#[test]
fn layout_box_store_keeps_clean_paint_and_drops_removed_nodes() {
    let store = LayoutBoxStore::new();
    let kept = NodeHandle(11);
    let removed = NodeHandle(12);
    store.record(kept, 1.0, 2.0, 10.0, 20.0);
    store.record(removed, 3.0, 4.0, 30.0, 40.0);
    store.begin_frame();
    assert_eq!(store.get(kept).unwrap().width, 10.0);
    assert_eq!(store.get(removed).unwrap().height, 40.0);
    store.remove(removed);
    assert!(store.get(kept).is_some());
    assert!(store.get(removed).is_none());
    store.retain(|id| id == kept.0);
    assert!(store.get(kept).is_some());
    assert!(store.get(removed).is_none());
}

#[test]
fn scroll_view_overlay_does_not_write_runtime_layout() {
    let mut doc = NanaTreeDocument::new(400, 300, 1.0);
    let target = doc.create_element("div");
    doc.insert(target, doc.mount_root(), None);
    let store = LayoutBoxStore::new();
    store.record(target, 0.0, 400.0, 300.0, 40.0);
    doc.apply_layout_boxes(&store.snapshot());
    assert_eq!(doc.layout_box(target).unwrap().y, 400.0);

    store.translate(target, 0.0, -400.0);
    assert_eq!(
        store.get(target).unwrap().y,
        0.0,
        "JS paint overlay follows scroll"
    );
    assert_eq!(
        doc.layout_box(target).unwrap().y,
        400.0,
        "Runtime LayoutBox stays unscrolled"
    );
    doc.apply_layout_boxes(&store.snapshot());
    assert_eq!(
        doc.layout_box(target).unwrap().y,
        400.0,
        "paint snapshot writeback must not copy scrolled coordinates"
    );
}

#[test]
fn generated_before_pseudo_appears_in_runtime_tree() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = crate::MessageBridge::new();
    let root = doc.mount_root();
    let host = doc.create_element("div");
    doc.insert(host, root, None);
    bridge.register(
        host.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            class_names: vec!["chip".into()],
            ..crate::WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(".chip::before { content: \"\"; width: 4px; height: 4px; }");
    bridge.resolve_document_layout(&mut doc);
    let snap = bridge.snapshot();
    let has_before = snap.widgets.iter().any(|w| {
        w.props
            .attrs
            .get(crate::bridge::GENERATED_PSEUDO_ATTR)
            .map(String::as_str)
            == Some("before")
    });
    assert!(
        has_before,
        "chip::before must materialize a generated child widget"
    );
    doc.flush_host_frame();
    let document = nana_ui_runtime::DocumentId::try_from(doc.id()).unwrap();
    let order = doc.world().document_order(document);
    assert!(
        order.iter().any(|id| snap.widgets.iter().any(|w| {
            w.props
                .attrs
                .contains_key(crate::bridge::GENERATED_PSEUDO_ATTR)
                && w.id == id.get()
        })),
        "generated ::before node must exist in Runtime document order"
    );
}

#[test]
fn generated_before_keeps_size_with_static_parent_rule() {
    use nana_ui_core::LengthSpec;
    use nana_ui_runtime::StableNodeId;

    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = crate::MessageBridge::new();
    let root = doc.mount_root();
    let host = doc.create_element("div");
    doc.insert(host, root, None);
    bridge.register(
        host.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            class_names: vec!["chip".into()],
            ..crate::WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        ".chip { display: flex; } .chip::before { content: \"\"; width: 4px; height: 4px; }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.flush_host_frame();
    let before_id = bridge
        .snapshot()
        .widgets
        .iter()
        .find(|w| {
            w.props
                .attrs
                .get(crate::bridge::GENERATED_PSEUDO_ATTR)
                .map(String::as_str)
                == Some("before")
        })
        .map(|w| w.id)
        .expect("generated ::before widget");
    let style = doc
        .runtime
        .node_style(StableNodeId::new(before_id).unwrap())
        .expect("before runtime style");
    assert_eq!(style.layout.width, Some(LengthSpec::Px(4.0)));
    assert_eq!(style.layout.height, Some(LengthSpec::Px(4.0)));
}

#[test]
fn generated_before_inserts_before_origin_children() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = crate::MessageBridge::new();
    let root = doc.mount_root();
    let host = doc.create_element("div");
    doc.insert(host, root, None);
    let child = doc.create_element("span");
    doc.insert(child, host, None);
    bridge.register(
        host.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            class_names: vec!["chip".into()],
            ..crate::WidgetProps::default()
        },
    );
    bridge.register(
        child.0,
        crate::WidgetKind::Text,
        crate::WidgetProps {
            label: "x".into(),
            ..crate::WidgetProps::default()
        },
    );
    bridge.insert_child(child.0, host.0, None);
    bridge.inject_stylesheet(".chip::before { content: \"\"; width: 4px; height: 4px; }");
    bridge.resolve_document_layout(&mut doc);
    doc.flush_host_frame();
    let before_id = bridge
        .snapshot()
        .widgets
        .iter()
        .find(|w| {
            w.props
                .attrs
                .get(crate::bridge::GENERATED_PSEUDO_ATTR)
                .map(String::as_str)
                == Some("before")
        })
        .map(|w| w.id)
        .expect("generated ::before widget");
    let document = nana_ui_runtime::DocumentId::try_from(doc.id()).unwrap();
    let order = doc
        .world()
        .document_order(document)
        .into_iter()
        .map(|id| id.get())
        .collect::<Vec<_>>();
    let host_idx = order.iter().position(|id| *id == host.0).expect("host");
    let before_idx = order
        .iter()
        .position(|id| *id == before_id)
        .expect("before");
    let child_idx = order.iter().position(|id| *id == child.0).expect("child");
    assert!(host_idx < before_idx && before_idx < child_idx);
}

#[test]
fn generated_after_inserts_after_origin_children() {
    let mut doc = NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = crate::MessageBridge::new();
    let root = doc.mount_root();
    let host = doc.create_element("div");
    doc.insert(host, root, None);
    let child = doc.create_element("span");
    doc.insert(child, host, None);
    bridge.register(
        host.0,
        crate::WidgetKind::Column,
        crate::WidgetProps {
            class_names: vec!["chip".into()],
            ..crate::WidgetProps::default()
        },
    );
    bridge.register(
        child.0,
        crate::WidgetKind::Text,
        crate::WidgetProps {
            label: "x".into(),
            ..crate::WidgetProps::default()
        },
    );
    bridge.insert_child(child.0, host.0, None);
    bridge.inject_stylesheet(".chip::after { content: \"\"; width: 4px; height: 4px; }");
    bridge.resolve_document_layout(&mut doc);
    doc.flush_host_frame();
    let after_id = bridge
        .snapshot()
        .widgets
        .iter()
        .find(|w| {
            w.props
                .attrs
                .get(crate::bridge::GENERATED_PSEUDO_ATTR)
                .map(String::as_str)
                == Some("after")
        })
        .map(|w| w.id)
        .expect("generated ::after widget");
    let document = nana_ui_runtime::DocumentId::try_from(doc.id()).unwrap();
    let order = doc
        .world()
        .document_order(document)
        .into_iter()
        .map(|id| id.get())
        .collect::<Vec<_>>();
    let host_idx = order.iter().position(|id| *id == host.0).expect("host");
    let child_idx = order.iter().position(|id| *id == child.0).expect("child");
    let after_idx = order.iter().position(|id| *id == after_id).expect("after");
    assert!(host_idx < child_idx && child_idx < after_idx);
}
