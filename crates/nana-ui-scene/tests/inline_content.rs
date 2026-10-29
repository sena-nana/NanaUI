use nana_text::{
    NativeTextEngine,
    font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily},
};
use nana_ui_runtime::{
    Button, DocumentId, FormattingContextKind, InlineContent, InlineFlow, LayoutStyle,
    LayoutViewport, LengthSpec, NanaTextEngineShaper, ParticipationKind,
};
use nana_ui_scene::RuntimeDocument;
use std::sync::{Arc, Mutex};

fn shaper() -> NanaTextEngineShaper {
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../nana-ui/assets/fonts/NotoSansSC-Variable.ttf"
            ),
            &FaceDescriptor::default(),
        )
        .unwrap();
    NanaTextEngineShaper::new(Arc::new(Mutex::new(NativeTextEngine::new(fonts))))
}

#[test]
fn inline_text_and_an_ordinary_button_share_the_native_paragraph() {
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let child = doc
        .context_mut()
        .create_detached_component(
            document,
            Button::new("Details").layout(Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(60.0)),
                height: Some(LengthSpec::Px(24.0)),
                ..Default::default()
            })),
        )
        .unwrap();
    let content = InlineContent::builder()
        .text("CPU ")
        .child(child.stable_id())
        .text(" 42% and more text to wrap")
        .build();
    let paragraph = doc
        .context_mut()
        .create_component(
            document,
            InlineFlow::new(content).width(LengthSpec::Px(120.0)),
        )
        .unwrap();
    let mut shaper = shaper();
    let viewport = LayoutViewport::new(400.0, 300.0);
    let update = doc.flush(viewport, &mut shaper).unwrap();
    assert!(update.passes <= 8);
    let world = doc.context().world();
    assert_eq!(world.len(), 2);
    assert_eq!(
        world
            .layout_node(paragraph.stable_id())
            .unwrap()
            .established,
        Some(FormattingContextKind::Inline)
    );
    assert_eq!(
        world.layout_node(child.stable_id()).unwrap().participation,
        Some(ParticipationKind::AtomicInline)
    );
    let (_, layout) = world.text_layout(paragraph.stable_id()).unwrap();
    assert!(
        layout.lines.len() > 1,
        "text and the object must wrap together"
    );
    assert_eq!(layout.objects.len(), 1);
    let object = layout.page_rect(layout.objects[0].rect, 120.0);
    let actual = world.layout_box(child.stable_id()).unwrap();
    assert_eq!(
        (actual.x, actual.y, actual.width, actual.height),
        (object.x, object.y, object.width, object.height)
    );
    assert_eq!(
        world.inline_text(paragraph.stable_id(), false).as_deref(),
        Some("CPU Details 42% and more text to wrap")
    );
    assert_eq!(
        world.hit_test(document, actual.x + 5.0, actual.y + 5.0),
        Some(child.stable_id())
    );
    assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
}

#[test]
fn mixed_replacement_preserves_children_and_returns_to_plain_without_residue() {
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let child = doc
        .context_mut()
        .create_detached_component(document, Button::new("live"))
        .unwrap();
    let paragraph = doc
        .context_mut()
        .create_component(
            document,
            InlineFlow::new(
                InlineContent::builder()
                    .text("before ")
                    .child(child.stable_id())
                    .text(" after")
                    .build(),
            )
            .width(LengthSpec::Px(150.0)),
        )
        .unwrap();
    let mut shaper = shaper();
    let viewport = LayoutViewport::new(400.0, 300.0);
    doc.flush(viewport, &mut shaper).unwrap();
    let before = doc
        .context()
        .world()
        .layout_result(paragraph.stable_id())
        .unwrap()
        .clone();
    let content = doc
        .context()
        .world()
        .inline_content(paragraph.stable_id())
        .unwrap()
        .replace(0..7, "new ");
    doc.context_mut()
        .set_inline_content(paragraph.stable_id(), content)
        .unwrap();
    doc.flush(viewport, &mut shaper).unwrap();
    assert!(doc.context().world().contains(child.stable_id()));
    assert_eq!(
        doc.context()
            .world()
            .inline_text(paragraph.stable_id(), false)
            .as_deref(),
        Some("new live after")
    );
    assert_ne!(
        before,
        *doc.context()
            .world()
            .layout_result(paragraph.stable_id())
            .unwrap()
    );
    doc.context_mut()
        .set_inline_content(paragraph.stable_id(), "plain")
        .unwrap();
    doc.flush(viewport, &mut shaper).unwrap();
    let world = doc.context().world();
    assert_eq!(world.len(), 1);
    assert!(!world.contains(child.stable_id()));
    assert!(matches!(
        world.inline_content(paragraph.stable_id()),
        Some(InlineContent::PlainText(_))
    ));
    assert!(world.rich_text(paragraph.stable_id()).is_none());
    assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
}

#[test]
fn button_content_uses_native_objects_and_rejects_nested_controls_atomically() {
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let glyph = doc
        .context_mut()
        .create_detached_component(document, nana_ui_runtime::Text::new("★"))
        .unwrap();
    let control = doc
        .context_mut()
        .create_component(
            document,
            Button::new("").content(
                InlineContent::builder()
                    .text("Run ")
                    .child(glyph.stable_id())
                    .build(),
            ),
        )
        .unwrap();
    let mut shaper = shaper();
    let viewport = LayoutViewport::new(400.0, 300.0);
    doc.flush(viewport, &mut shaper).unwrap();
    assert_eq!(
        doc.context()
            .world()
            .inline_text(control.stable_id(), true)
            .as_deref(),
        Some("Run ★")
    );
    let other = doc
        .context_mut()
        .create_detached_component(document, Button::new("Nested"))
        .unwrap();
    let old = doc
        .context()
        .world()
        .inline_content(control.stable_id())
        .unwrap();
    assert!(
        doc.context_mut()
            .set_inline_content(
                control.stable_id(),
                InlineContent::atomic(other.stable_id())
            )
            .is_err()
    );
    assert_eq!(
        doc.context().world().inline_content(control.stable_id()),
        Some(old)
    );
    assert!(doc.context().world().contains(glyph.stable_id()));
    // Updating a child's role later must also validate its inline owner.
    let mut queue = nana_ui_runtime::MutationQueue::new();
    queue.set_accessibility(
        glyph.stable_id(),
        nana_ui_runtime::AccessibilityState {
            role: nana_ui_runtime::AccessibilityRole::Button,
            ..Default::default()
        },
    );
    assert!(doc.context_mut().commit_mutations(queue).is_err());
}

#[test]
fn atomic_size_changes_reflow_the_paragraph_and_publish_source_fragments() {
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let child = doc
        .context_mut()
        .create_detached_component(document, Button::new("x"))
        .unwrap();
    let paragraph = doc
        .context_mut()
        .create_component(
            document,
            InlineFlow::new(
                InlineContent::builder()
                    .text("prefix ")
                    .child(child.stable_id())
                    .text(" 尾部 text")
                    .build(),
            )
            .width(LengthSpec::Px(140.0)),
        )
        .unwrap();
    let mut shaper = shaper();
    let viewport = LayoutViewport::new(400.0, 300.0);
    doc.flush(viewport, &mut shaper).unwrap();
    let before = doc.context().world().layout_box(child.stable_id()).unwrap();
    doc.context_mut()
        .update_component(child, |button, _| {
            button.label = "longer atomic button".into()
        })
        .unwrap();
    doc.flush(viewport, &mut shaper).unwrap();
    let world = doc.context().world();
    let after = world.layout_box(child.stable_id()).unwrap();
    assert!(after.width > before.width);
    let result = world.layout_result(paragraph.stable_id()).unwrap();
    assert!(
        result
            .fragments
            .iter()
            .any(|f| f.node == Some(child.stable_id()) && f.source == Some(7..10))
    );
    assert!(
        result
            .fragments
            .iter()
            .filter(|f| f.kind == nana_ui_runtime::LayoutFragmentKind::TextRun)
            .all(|f| f.source.is_some())
    );
    assert_eq!(world.len(), 2);
    assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
}

#[test]
fn rtl_and_vertical_objects_follow_the_text_engine_and_width_changes_reuse_shape() {
    use nana_ui_core::{DirSpec, WritingModeSpec};
    use nana_ui_runtime::{NodeStyle, Text};
    for (dir, mode) in [
        (DirSpec::Ltr, WritingModeSpec::HorizontalTb),
        (DirSpec::Rtl, WritingModeSpec::HorizontalTb),
        (DirSpec::Ltr, WritingModeSpec::VerticalRl),
        (DirSpec::Rtl, WritingModeSpec::VerticalLr),
    ] {
        let document = DocumentId::new(1).unwrap();
        let mut doc = RuntimeDocument::new(document);
        let child = doc
            .context_mut()
            .create_detached_component(
                document,
                Text::new("中").style(NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(20.0)),
                        height: Some(LengthSpec::Px(24.0)),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            )
            .unwrap();
        let paragraph = doc
            .context_mut()
            .create_component(
                document,
                InlineFlow::new(
                    InlineContent::builder()
                        .text("字文 ")
                        .child(child.stable_id())
                        .text(" 尾部 text")
                        .build(),
                )
                .style(NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(160.0)),
                        height: Some(LengthSpec::Px(150.0)),
                        dir: Some(dir),
                        writing_mode: Some(mode),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            )
            .unwrap();
        let mut shaper = shaper();
        let viewport = LayoutViewport::new(400.0, 300.0);
        doc.flush(viewport, &mut shaper).unwrap();
        let world = doc.context().world();
        let (_, layout) = world.text_layout(paragraph.stable_id()).unwrap();
        let object = layout.page_rect(layout.objects[0].rect, 160.0);
        let actual = world.layout_box(child.stable_id()).unwrap();
        assert_eq!(
            (actual.x, actual.y, actual.width, actual.height),
            (object.x, object.y, 20.0, 24.0),
            "{mode:?}/{dir:?}"
        );
        doc.context_mut()
            .update_component(paragraph, |flow, _| {
                Arc::make_mut(&mut flow.style.layout).width = Some(LengthSpec::Px(120.0))
            })
            .unwrap();
        doc.flush(viewport, &mut shaper).unwrap();
        let world = doc.context().world();
        let layout = world.text_layout(paragraph.stable_id()).unwrap().1;
        let object = layout.page_rect(layout.objects[0].rect, 120.0);
        let actual = world.layout_box(child.stable_id()).unwrap();
        assert_eq!(
            (actual.x, actual.y),
            (object.x, object.y),
            "resized {mode:?}/{dir:?}"
        );
        assert_eq!(doc.context().last_work_counters().text_shaped_runs, 0);
        assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
    }
}

#[test]
fn ellipsis_omits_atomic_subtrees_without_destructing_or_reflow_oscillation() {
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let child = doc
        .context_mut()
        .create_detached_component(document, Button::new("Details"))
        .unwrap();
    let paragraph = doc
        .context_mut()
        .create_component(
            document,
            InlineFlow::new(
                InlineContent::builder()
                    .text("enough words before ")
                    .child(child.stable_id())
                    .text(" after")
                    .build(),
            )
            .style(nana_ui_runtime::NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(55.0)),
                    line_clamp: Some(1),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
        .unwrap();
    let mut shaper = shaper();
    let viewport = LayoutViewport::new(400.0, 300.0);
    let update = doc.flush(viewport, &mut shaper).unwrap();
    assert!(update.passes <= 8);
    let world = doc.context().world();
    assert!(
        world
            .text_layout(paragraph.stable_id())
            .unwrap()
            .1
            .objects
            .is_empty()
    );
    assert_eq!(world.layout_box(child.stable_id()).unwrap().width, 0.0);
    assert!(!world.extract_nodes(&[child.stable_id()])[0].style.visible);
    assert_ne!(world.hit_test(document, 2.0, 2.0), Some(child.stable_id()));
    assert!(world.contains(child.stable_id()));
    assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
    doc.context_mut()
        .update_component(paragraph, |flow, _| {
            Arc::make_mut(&mut flow.style.layout).width = Some(LengthSpec::Px(400.0))
        })
        .unwrap();
    doc.flush(viewport, &mut shaper).unwrap();
    assert!(
        doc.context()
            .world()
            .layout_box(child.stable_id())
            .unwrap()
            .width
            > 0.0
    );
    assert!(
        doc.context()
            .world()
            .extract_nodes(&[child.stable_id()])
            .len()
            == 1
    );
}

#[test]
fn paint_and_equal_atomic_metrics_do_not_reflow_the_inline_parent() {
    use nana_ui_runtime::{MutationQueue, SemanticColorRole};
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let child = doc
        .context_mut()
        .create_detached_component(
            document,
            Button::new("one").layout(Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(80.0)),
                height: Some(LengthSpec::Px(30.0)),
                ..Default::default()
            })),
        )
        .unwrap();
    let paragraph = doc
        .context_mut()
        .create_component(
            document,
            InlineFlow::new(
                InlineContent::builder()
                    .text("CPU ")
                    .child(child.stable_id())
                    .text(" 42%")
                    .build(),
            )
            .width(LengthSpec::Px(220.0)),
        )
        .unwrap();
    let mut shaper = shaper();
    let viewport = LayoutViewport::new(400.0, 300.0);
    doc.flush(viewport, &mut shaper).unwrap();
    let result = doc
        .context()
        .world()
        .layout_result(paragraph.stable_id())
        .unwrap() as *const _ as usize;
    let mut queue = MutationQueue::new();
    let mut style = doc
        .context()
        .world()
        .node_style(child.stable_id())
        .unwrap()
        .clone();
    style.foreground = Some(SemanticColorRole::Danger);
    queue.set_style(child.stable_id(), style);
    doc.context_mut().commit_mutations(queue).unwrap();
    doc.flush(viewport, &mut shaper).unwrap();
    let work = doc.context().last_work_counters();
    assert_eq!(
        (
            work.inline_reflows,
            work.inline_atomic_measure_requests,
            work.layout_measure_nodes,
            work.layout_placement_nodes
        ),
        (0, 0, 0, 0)
    );
    doc.context_mut()
        .update_component(child, |button, _| button.label = "two".into())
        .unwrap();
    doc.flush(viewport, &mut shaper).unwrap();
    let work = doc.context().last_work_counters();
    assert_eq!(
        work.inline_reflows, 0,
        "fixed metrics stop at the child: {work:?}"
    );
    assert_eq!(
        doc.context()
            .world()
            .layout_result(paragraph.stable_id())
            .unwrap() as *const _ as usize,
        result
    );
    assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
}

#[test]
fn physical_margins_are_exported_as_outer_metrics_in_both_writing_axes() {
    use nana_ui_core::WritingModeSpec;
    for mode in [WritingModeSpec::HorizontalTb, WritingModeSpec::VerticalRl] {
        let document = DocumentId::new(1).unwrap();
        let mut doc = RuntimeDocument::new(document);
        let child = doc
            .context_mut()
            .create_detached_component(
                document,
                Button::new("x").layout(Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(30.0)),
                    height: Some(LengthSpec::Px(24.0)),
                    margin_left: Some(LengthSpec::Px(5.0)),
                    margin_right: Some(LengthSpec::Px(7.0)),
                    margin_top: Some(LengthSpec::Px(3.0)),
                    margin_bottom: Some(LengthSpec::Px(4.0)),
                    ..Default::default()
                })),
            )
            .unwrap();
        let flow = doc
            .context_mut()
            .create_component(
                document,
                InlineFlow::new(
                    InlineContent::builder()
                        .text("字")
                        .child(child.stable_id())
                        .text("尾")
                        .build(),
                )
                .style(nana_ui_runtime::NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(180.0)),
                        height: Some(LengthSpec::Px(180.0)),
                        writing_mode: Some(mode),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            )
            .unwrap();
        let mut shaper = shaper();
        let viewport = LayoutViewport::new(400.0, 300.0);
        doc.flush(viewport, &mut shaper).unwrap();
        let world = doc.context().world();
        let layout = world.text_layout(flow.stable_id()).unwrap().1;
        let outer = layout.page_rect(layout.objects[0].rect, 180.0);
        let actual = world.layout_box(child.stable_id()).unwrap();
        assert_eq!((outer.width, outer.height), (42.0, 31.0));
        assert_eq!((actual.x, actual.y), (outer.x + 5.0, outer.y + 3.0));
        assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
    }
}

#[test]
fn incremental_inline_content_matches_a_cold_world_and_keeps_focus() {
    fn fixture(
        prefix: &str,
        label: &str,
    ) -> (
        RuntimeDocument,
        nana_ui_runtime::Entity<Button>,
        nana_ui_runtime::Entity<InlineFlow>,
    ) {
        let document = DocumentId::new(1).unwrap();
        let mut doc = RuntimeDocument::new(document);
        let child = doc
            .context_mut()
            .create_detached_component(document, Button::new(label))
            .unwrap();
        let flow = doc
            .context_mut()
            .create_component(
                document,
                InlineFlow::new(
                    InlineContent::builder()
                        .text(prefix)
                        .child(child.stable_id())
                        .text(" 中文 tail")
                        .build(),
                )
                .width(LengthSpec::Px(170.0)),
            )
            .unwrap();
        (doc, child, flow)
    }
    let (mut actual, child, flow) = fixture("before ", "x");
    let viewport = LayoutViewport::new(400.0, 300.0);
    let mut a = shaper();
    actual.flush(viewport, &mut a).unwrap();
    actual
        .context_mut()
        .focus_node(DocumentId::new(1).unwrap(), child.stable_id())
        .unwrap();
    actual
        .context_mut()
        .update_component(child, |button, _| button.label = "Details".into())
        .unwrap();
    actual
        .context_mut()
        .update_component(flow, |flow, _| {
            flow.content = flow.content.replace(0..7, "after ")
        })
        .unwrap();
    actual.flush(viewport, &mut a).unwrap();
    let (mut cold, cold_child, cold_flow) = fixture("after ", "Details");
    let mut b = shaper();
    cold.flush(viewport, &mut b).unwrap();
    for (left, right) in [
        (child.stable_id(), cold_child.stable_id()),
        (flow.stable_id(), cold_flow.stable_id()),
    ] {
        assert!(
            actual
                .context()
                .world()
                .layout_result(left)
                .unwrap()
                .geometry_eq(cold.context().world().layout_result(right).unwrap()),
            "{left:?} differs from cold layout"
        );
    }
    assert_eq!(
        actual
            .context()
            .world()
            .focused(DocumentId::new(1).unwrap()),
        Some(child.stable_id())
    );
}

#[test]
fn native_inline_work_is_bounded_by_atomic_entries_and_idle_is_empty() {
    for count in [64, 1024] {
        let document = DocumentId::new(1).unwrap();
        let mut doc = RuntimeDocument::new(document);
        let mut content = InlineContent::builder();
        for _ in 0..count {
            let child = doc
                .context_mut()
                .create_detached_component(document, nana_ui_runtime::Text::new("x"))
                .unwrap();
            content = content.text(" ").child(child.stable_id());
        }
        doc.context_mut()
            .create_component(
                document,
                InlineFlow::new(content.build()).width(LengthSpec::Px(200.0)),
            )
            .unwrap();
        let mut shaper = shaper();
        let viewport = LayoutViewport::new(400.0, 300.0);
        doc.flush(viewport, &mut shaper).unwrap();
        let counters = doc.context().last_work_counters();
        assert!(counters.inline_reflows <= 4, "{count}: {counters:?}");
        assert!(counters.inline_atomic_measure_requests <= 4 * count);
        assert!(counters.layout_children_measured <= 16 * count);
        assert_eq!(doc.context().world().len(), count + 1);
        assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
    }
}

#[test]
fn a_plain_single_run_keeps_box_fragments_when_only_its_advance_changes() {
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let label = doc
        .context_mut()
        .create_component(
            document,
            nana_ui_runtime::Text::new("iii").style(nana_ui_runtime::NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(120.0)),
                    height: Some(LengthSpec::Px(24.0)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
        .unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    let mut shaper = shaper();
    doc.flush(viewport, &mut shaper).unwrap();
    let before = doc
        .context()
        .world()
        .layout_result(label.stable_id())
        .unwrap() as *const _ as usize;
    let old_text = doc
        .context()
        .world()
        .text_layout(label.stable_id())
        .unwrap()
        .0;
    doc.context_mut()
        .update_component(label, |label, _| label.value = "WWW".into())
        .unwrap();
    doc.flush(viewport, &mut shaper).unwrap();
    let world = doc.context().world();
    assert_ne!(world.text_layout(label.stable_id()).unwrap().0, old_text);
    assert_eq!(
        world.layout_result(label.stable_id()).unwrap() as *const _ as usize,
        before
    );
    assert_eq!(
        world
            .layout_result(label.stable_id())
            .unwrap()
            .fragments
            .len(),
        2
    );
    doc.context_mut()
        .update_component(label, |label, _| label.value = "WWWW".into())
        .unwrap();
    doc.flush(viewport, &mut shaper).unwrap();
    assert!(
        doc.context()
            .world()
            .layout_result(label.stable_id())
            .unwrap()
            .fragments
            .iter()
            .all(|fragment| fragment.source == Some(0..4))
    );
}

#[test]
fn secure_atomic_fields_never_become_a_paragraph_name_or_copy_source() {
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let secret = doc
        .context_mut()
        .create_detached_component(
            document,
            nana_ui_runtime::TextInput::new("private value").secure(true),
        )
        .unwrap();
    let paragraph = doc
        .context_mut()
        .create_component(
            document,
            InlineFlow::new(
                InlineContent::builder()
                    .text("Value: ")
                    .child(secret.stable_id())
                    .build(),
            )
            .width(LengthSpec::Px(240.0)),
        )
        .unwrap();
    let world = doc.context().world();
    assert_eq!(
        world.inline_text(paragraph.stable_id(), true).as_deref(),
        Some("Value: ")
    );
    assert_eq!(
        world.inline_text(paragraph.stable_id(), false).as_deref(),
        Some("Value: ")
    );
}

#[test]
fn canonical_control_content_overrides_legacy_label_and_can_return_to_plain() {
    use nana_ui_core::{RichSpanStyle, RichText};
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let button = doc
        .context_mut()
        .create_component(
            document,
            Button::new("fallback")
                .content(RichText::new("current").with_span(0..7, RichSpanStyle::new().size(18.0))),
        )
        .unwrap();
    doc.context_mut()
        .update_component(button, |button, _| button.label = "stale label".into())
        .unwrap();
    let world = doc.context().world();
    assert_eq!(world.text(button.stable_id()), Some("current"));
    assert!(
        matches!(world.standard_visual(button.stable_id()),Some(nana_ui_runtime::StandardVisual::Button {label,..}) if label.as_ref()=="current")
    );
    doc.context_mut()
        .update_component(button, |button, _| {
            *button = button.clone().content("plain")
        })
        .unwrap();
    assert_eq!(
        doc.context().world().text(button.stable_id()),
        Some("plain")
    );
    assert!(
        doc.context()
            .world()
            .rich_text(button.stable_id())
            .is_none()
    );
}

#[test]
fn literal_inline_content_ends_localization_even_when_the_text_is_equal() {
    use nana_ui_runtime::{LocalizedText, Text};
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let text = doc
        .context_mut()
        .create_component(document, Text::localized(LocalizedText::new("caption")))
        .unwrap();
    assert!(
        doc.context()
            .world()
            .localized_text(text.stable_id())
            .is_some()
    );
    let current = doc
        .context()
        .world()
        .text(text.stable_id())
        .unwrap()
        .to_owned();
    doc.context_mut()
        .set_inline_content(text.stable_id(), current)
        .unwrap();
    assert!(
        doc.context()
            .world()
            .localized_text(text.stable_id())
            .is_none()
    );
}

#[test]
fn advanced_menu_primary_uses_its_text_part_budget_and_hint_keeps_its_own_layout() {
    use nana_ui_core::{RichSpanStyle, RichText};
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let menu = doc
        .context_mut()
        .create_component(
            document,
            nana_ui_runtime::ActionMenuItem::new("fallback")
                .content(RichText::new("same").with_span(0..4, RichSpanStyle::new().size(20.0)))
                .hint("same")
                .style(nana_ui_runtime::NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(180.0)),
                        dir: Some(nana_ui_core::DirSpec::Rtl),
                        height: Some(LengthSpec::Px(40.0)),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
        )
        .unwrap();
    let mut shaper = shaper();
    doc.flush(LayoutViewport::new(400.0, 300.0), &mut shaper)
        .unwrap();
    let world = doc.context().world();
    let text_box = world
        .layout_result(menu.stable_id())
        .unwrap()
        .parts
        .iter()
        .find(|part| part.kind == nana_ui_runtime::LayoutPartKind::TextContent)
        .unwrap()
        .bounds;
    assert_eq!(
        world
            .text_layout(menu.stable_id())
            .unwrap()
            .1
            .constraints
            .max_width_px,
        Some(text_box.width)
    );
    let primary = doc
        .scene()
        .primitives()
        .find(|p| p.node == menu.stable_id() && p.id.slot == 2)
        .unwrap();
    let hint = doc
        .scene()
        .primitives()
        .find(|p| p.node == menu.stable_id() && p.id.slot == 4)
        .unwrap();
    assert_eq!(primary.bounds.x, text_box.x);
    assert!(matches!(
        &primary.kind,
        nana_ui_scene::ScenePrimitiveKind::Text {
            layout: Some(_),
            ..
        }
    ));
    assert!(matches!(
        &hint.kind,
        nana_ui_scene::ScenePrimitiveKind::Text { layout: None, .. }
    ));
    doc.context_mut()
        .update_component(menu, |item, _| item.hint = Some(Arc::from("a wider hint")))
        .unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    doc.flush(viewport, &mut shaper).unwrap();
    let world = doc.context().world();
    let width = world
        .layout_result(menu.stable_id())
        .unwrap()
        .parts
        .iter()
        .find(|part| part.kind == nana_ui_runtime::LayoutPartKind::TextContent)
        .unwrap()
        .bounds
        .width;
    assert!(width < text_box.width);
    assert_eq!(
        world
            .text_layout(menu.stable_id())
            .unwrap()
            .1
            .constraints
            .max_width_px,
        Some(width)
    );
    assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
}

#[test]
fn control_paint_spans_keep_plain_geometry_and_native_budget() {
    use nana_ui_core::{PaintColor, RichSpanStyle};
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let button = doc
        .context_mut()
        .create_component(
            document,
            Button::new("Save").layout(Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(160.0)),
                height: Some(LengthSpec::Px(40.0)),
                ..Default::default()
            })),
        )
        .unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    let mut shaper = shaper();
    doc.flush(viewport, &mut shaper).unwrap();
    let before = doc
        .context()
        .world()
        .layout_result(button.stable_id())
        .unwrap()
        .clone();
    let layout = doc
        .context()
        .world()
        .text_layout(button.stable_id())
        .unwrap()
        .0;
    let styled = InlineContent::from("Save").with_span(
        0..4,
        RichSpanStyle::new().color(PaintColor::Srgb {
            rgba: [1.0, 0.0, 0.0, 1.0],
        }),
    );
    for content in [styled.clone(), InlineContent::from("Save")] {
        doc.context_mut()
            .update_component(button, |button, _| {
                *button = button.clone().content(content);
            })
            .unwrap();
        doc.flush(viewport, &mut shaper).unwrap();
        let world = doc.context().world();
        assert_eq!(world.text_layout(button.stable_id()).unwrap().0, layout);
        assert!(before.geometry_eq(world.layout_result(button.stable_id()).unwrap()));
        let work = doc.context().last_work_counters();
        assert_eq!(
            (
                work.text_shaped_runs,
                work.text_wrap_layouts,
                work.inline_reflows,
                work.layout_measure_nodes,
                work.layout_placement_nodes
            ),
            (0, 0, 0, 0, 0)
        );
        assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
    }
    let cold = doc
        .context_mut()
        .create_component(
            document,
            Button::new("ignored")
                .content(styled)
                .layout(Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(160.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    ..Default::default()
                })),
        )
        .unwrap();
    doc.flush(viewport, &mut shaper).unwrap();
    assert!(
        before.geometry_eq(
            doc.context()
                .world()
                .layout_result(cold.stable_id())
                .unwrap()
        )
    );
    let budget = doc
        .context()
        .world()
        .text_layout(button.stable_id())
        .unwrap()
        .1
        .constraints
        .max_width_px
        .unwrap();
    doc.context_mut()
        .update_component(button, |button, _| button.loading = true)
        .unwrap();
    doc.flush(viewport, &mut shaper).unwrap();
    assert!(
        doc.context()
            .world()
            .text_layout(button.stable_id())
            .unwrap()
            .1
            .constraints
            .max_width_px
            .unwrap()
            < budget
    );
    assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
    let short = doc
        .context_mut()
        .create_component(
            document,
            Button::new("Save")
                .loading(true)
                .icon_size(20.0)
                .icon_gap(3.0)
                .layout(Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(100.0)),
                    height: Some(LengthSpec::Px(8.0)),
                    ..Default::default()
                })),
        )
        .unwrap();
    doc.flush(viewport, &mut shaper).unwrap();
    let world = doc.context().world();
    let result = world.layout_result(short.stable_id()).unwrap();
    let spinner = result
        .parts
        .iter()
        .find(|part| part.kind == nana_ui_runtime::LayoutPartKind::Spinner)
        .unwrap();
    assert!(spinner.bounds.width < 20.0);
    let expected = (result.content_box.width - spinner.bounds.width - 3.0).max(0.0);
    assert_eq!(
        world
            .text_layout(short.stable_id())
            .unwrap()
            .1
            .constraints
            .max_width_px,
        Some(expected)
    );
    assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
}
