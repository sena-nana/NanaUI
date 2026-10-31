//! 富文本光标在真实 retained layout、场景抽取及规范输入链上的回归测试。

#![cfg(feature = "rich-text")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nana_text::NativeTextEngine;
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_ui_core::RichText;
use nana_ui_input::{CompositionInput, InputModifiers, InputPayload, KeyInput, KeyState};
use nana_ui_runtime::{
    DocumentId, Entity, HeadlessInput, LayoutViewport, MutationQueue, NanaTextEngineShaper,
    RichEditCommand, RichTextEditor,
};
use nana_ui_scene::{RuntimeDocument, ScenePrimitiveKind};

/// 创建产品使用的文本引擎与富文本编辑器，返回规范输入适配器。
fn editor() -> (
    RuntimeDocument,
    NanaTextEngineShaper,
    HeadlessInput,
    Entity<RichTextEditor>,
) {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let entity = runtime
        .context_mut()
        .create_component(document, RichTextEditor::new(""))
        .unwrap();
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_bytes(
            font_blob(nana_ui_core::fonts::UI_FONT_REGULAR),
            &FaceDescriptor::default(),
        )
        .unwrap();
    let mut shaper = NanaTextEngineShaper::new(Arc::new(Mutex::new(NativeTextEngine::new(fonts))));
    settle(&mut runtime, &mut shaper);
    let mut input = HeadlessInput::bind(runtime.context_mut(), document);
    input
        .route(runtime.context_mut(), InputPayload::Focus { focused: true })
        .unwrap();
    runtime
        .context_mut()
        .focus_node(document, entity.stable_id())
        .unwrap();
    settle(&mut runtime, &mut shaper);
    runtime
        .context_mut()
        .refresh_input_effects(HeadlessInput::SOURCE, input.services_mut());
    (runtime, shaper, input, entity)
}

fn settle(runtime: &mut RuntimeDocument, shaper: &mut NanaTextEngineShaper) {
    for _ in 0..4 {
        runtime
            .flush(LayoutViewport::new(400.0, 300.0), shaper)
            .unwrap();
    }
}

fn caret_count(runtime: &RuntimeDocument, entity: Entity<RichTextEditor>) -> usize {
    runtime
        .scene()
        .primitives()
        .filter(|primitive| {
            primitive.node == entity.stable_id()
                && matches!(primitive.kind, ScenePrimitiveKind::Quad { .. })
                && primitive.bounds.width < 2.0
                && primitive.bounds.height > 5.0
        })
        .count()
}

#[test]
fn empty_editor_keeps_caret_layout_after_typing_deleting_and_external_clear() {
    let (mut runtime, mut shaper, mut input, entity) = editor();
    assert_eq!(caret_count(&runtime, entity), 1);
    let (_, layout) = runtime
        .context()
        .world()
        .text_layout(entity.stable_id())
        .unwrap();
    assert_eq!(layout.lines.len(), 1);
    assert!(nana_ui_runtime::rich_editor_caret_box(layout, 0).unwrap().2 > 0.0);
    input.text(runtime.context_mut(), "短消息").unwrap();
    settle(&mut runtime, &mut shaper);
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::SelectAll)
        .unwrap();
    input
        .press(
            runtime.context_mut(),
            KeyInput::named(
                "Backspace",
                "Backspace",
                KeyState::Pressed,
                InputModifiers::default(),
            ),
            None,
            None,
        )
        .unwrap();
    settle(&mut runtime, &mut shaper);
    assert_eq!(runtime.context().world().text(entity.stable_id()), Some(""));
    assert_eq!(caret_count(&runtime, entity), 1);
    input.text(runtime.context_mut(), "发送后清空").unwrap();
    settle(&mut runtime, &mut shaper);
    runtime
        .context_mut()
        .set_component(entity, RichTextEditor::new(RichText::new("")))
        .unwrap();
    settle(&mut runtime, &mut shaper);
    assert_eq!(caret_count(&runtime, entity), 1);
}

#[test]
fn caret_blinks_on_deadlines_without_shaping_and_keeps_ime_anchor() {
    let (mut runtime, mut shaper, mut input, entity) = editor();
    assert_eq!(
        runtime.context().next_animation_deadline(),
        Some(Duration::from_millis(500))
    );
    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(499));
    settle(&mut runtime, &mut shaper);
    assert_eq!(caret_count(&runtime, entity), 1);
    let ime_before = input.services().text_input().cloned();
    assert!(
        ime_before
            .as_ref()
            .and_then(|state| state.cursor_area)
            .is_some()
    );
    let (_, before) = runtime
        .context()
        .world()
        .text_layout(entity.stable_id())
        .unwrap();
    let before = Arc::clone(before);
    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(500));
    runtime
        .flush(LayoutViewport::new(400.0, 300.0), &mut shaper)
        .unwrap();
    assert_eq!(caret_count(&runtime, entity), 0);
    runtime
        .context_mut()
        .set_component(entity, RichTextEditor::new(""))
        .unwrap();
    runtime.context_mut().reproject_component(entity).unwrap();
    assert_eq!(
        runtime.context().next_animation_deadline(),
        Some(Duration::from_millis(1000))
    );
    settle(&mut runtime, &mut shaper);
    assert_eq!(caret_count(&runtime, entity), 0);
    assert_eq!(
        runtime
            .context()
            .world()
            .rich_editor_marks(entity.stable_id())
            .unwrap()
            .caret,
        Some(0)
    );
    assert_eq!(
        runtime
            .context()
            .world()
            .last_text_work_counters()
            .text_nodes_shaped,
        0
    );
    let (_, after) = runtime
        .context()
        .world()
        .text_layout(entity.stable_id())
        .unwrap();
    assert!(Arc::ptr_eq(&before, after));
    runtime
        .context_mut()
        .refresh_input_effects(HeadlessInput::SOURCE, input.services_mut());
    assert_eq!(input.services().text_input().cloned(), ime_before);
    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(1000));
    settle(&mut runtime, &mut shaper);
    assert_eq!(caret_count(&runtime, entity), 1);
}

#[test]
fn input_selection_and_composition_reset_the_visible_phase_from_input_time() {
    let (mut runtime, mut shaper, mut input, entity) = editor();
    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(500));
    input.set_now(Duration::from_millis(700));
    input.text(runtime.context_mut(), "字").unwrap();
    settle(&mut runtime, &mut shaper);
    assert_eq!(caret_count(&runtime, entity), 1);
    assert_eq!(
        runtime.context().next_animation_deadline(),
        Some(Duration::from_millis(1200))
    );
    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(1200));
    input.set_now(Duration::from_millis(1400));
    input
        .composition(runtime.context_mut(), CompositionInput::Start)
        .unwrap();
    input
        .composition(
            runtime.context_mut(),
            CompositionInput::Update {
                text: "输入".into(),
                selection: Some((6, 6)),
            },
        )
        .unwrap();
    settle(&mut runtime, &mut shaper);
    assert_eq!(caret_count(&runtime, entity), 1);
    assert_eq!(
        runtime.context().next_animation_deadline(),
        Some(Duration::from_millis(1900))
    );
    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(1900));
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::Select(0..0))
        .unwrap();
    settle(&mut runtime, &mut shaper);
    assert_eq!(caret_count(&runtime, entity), 1);
    assert_eq!(
        runtime.context().next_animation_deadline(),
        Some(Duration::from_millis(2400))
    );
}

#[test]
fn blur_disable_and_unload_cancel_blink_and_window_refocus_restores_it() {
    let (mut runtime, mut shaper, mut input, entity) = editor();
    input.set_now(Duration::from_millis(200));
    input
        .route(
            runtime.context_mut(),
            InputPayload::Focus { focused: false },
        )
        .unwrap();
    settle(&mut runtime, &mut shaper);
    assert_eq!(caret_count(&runtime, entity), 0);
    assert_eq!(runtime.context().next_animation_deadline(), None);
    assert_eq!(
        runtime
            .context()
            .world()
            .focused(DocumentId::new(1).unwrap()),
        Some(entity.stable_id())
    );
    input.set_now(Duration::from_millis(800));
    input
        .route(runtime.context_mut(), InputPayload::Focus { focused: true })
        .unwrap();
    settle(&mut runtime, &mut shaper);
    assert_eq!(caret_count(&runtime, entity), 1);
    assert_eq!(
        runtime.context().next_animation_deadline(),
        Some(Duration::from_millis(1300))
    );
    runtime
        .context_mut()
        .update_component(entity, |view, _| view.disabled = true)
        .unwrap();
    assert_eq!(runtime.context().next_animation_deadline(), None);
    runtime
        .context_mut()
        .update_component(entity, |view, _| view.disabled = false)
        .unwrap();
    let document = DocumentId::new(1).unwrap();
    runtime
        .context_mut()
        .focus_node(document, entity.stable_id())
        .unwrap();
    assert!(runtime.context().next_animation_deadline().is_some());
    let mut queue = MutationQueue::new();
    queue.despawn_subtree(entity.stable_id());
    runtime.context_mut().commit_mutations(queue).unwrap();
    assert_eq!(runtime.context().next_animation_deadline(), None);
}
