//! 默认样式输入与文字背景的真实 Runtime/Scene 回归，覆盖规范输入和 retained 排版。

#![cfg(feature = "rich-text")]

use std::sync::{Arc, Mutex};

use nana_text::NativeTextEngine;
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_ui_core::{LengthSpec, PaintColor, RichSpanStyle, RichText, WhiteSpaceSpec};
use nana_ui_input::{
    CompositionInput, HostServices, InputModifiers, InputPayload, KeyInput, KeyState,
};
use nana_ui_runtime::{
    DocumentId, Entity, HeadlessInput, LayoutViewport, NanaTextEngineShaper, RichAttrMask,
    RichEditCommand, RichTextEditor, RichTypingStylePolicy,
};
use nana_ui_scene::{RuntimeDocument, ScenePrimitiveKind};

const RED: PaintColor = PaintColor::srgb([1.0, 0.0, 0.0, 0.4]);
const BLUE: PaintColor = PaintColor::srgb([0.0, 0.0, 1.0, 1.0]);

/// 使用产品文本引擎和输入路由，便于同时验证文档与绘制结果。
fn editor(
    value: RichText,
    policy: RichTypingStylePolicy,
) -> (
    RuntimeDocument,
    NanaTextEngineShaper,
    HeadlessInput,
    Entity<RichTextEditor>,
) {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let mut view = RichTextEditor::new(value)
        .typing_style_policy(policy)
        .width(LengthSpec::Px(120.0));
    Arc::make_mut(&mut view.style.layout).white_space = WhiteSpaceSpec::PreWrap;
    let entity = runtime
        .context_mut()
        .create_component(document, view)
        .unwrap();
    let mut fonts_policy = FallbackPolicy::empty();
    fonts_policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(fonts_policy);
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
    (runtime, shaper, input, entity)
}

fn settle(runtime: &mut RuntimeDocument, shaper: &mut NanaTextEngineShaper) {
    for _ in 0..4 {
        runtime
            .flush(LayoutViewport::new(400.0, 300.0), shaper)
            .unwrap();
    }
}

fn value(runtime: &RuntimeDocument, entity: Entity<RichTextEditor>) -> RichText {
    runtime
        .context()
        .read(entity, |view| view.value.clone())
        .unwrap()
}

fn shortcut(runtime: &mut RuntimeDocument, input: &mut HeadlessInput, key: &'static str) {
    input
        .press(
            runtime.context_mut(),
            KeyInput::named(key, key, KeyState::Pressed, InputModifiers::primary()),
            None,
            None,
        )
        .unwrap();
}

#[test]
fn plain_typing_preserves_existing_spans_and_undo_while_observing_caret_style() {
    let style = RichSpanStyle::new()
        .bold()
        .color(BLUE)
        .background_color(RED)
        .effect(3);
    let original = RichText::new("ab").with_span(0..2, style.clone());
    let (mut runtime, _, mut input, entity) =
        editor(original.clone(), RichTypingStylePolicy::Plain);
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::Select(1..1))
        .unwrap();
    assert_eq!(
        runtime
            .context()
            .read(entity, |view| view.selection_attrs().style)
            .unwrap(),
        style
    );
    input.text(runtime.context_mut(), "中文").unwrap();
    let rich = value(&runtime, entity);
    assert_eq!(rich.text(), "a中文b");
    assert_eq!(rich.style_at(0), Some(&style));
    assert_eq!(rich.style_at(7), Some(&style));
    assert!(rich.style_at(1).is_none());
    assert!(rich.style_at(4).is_none());
    shortcut(&mut runtime, &mut input, "z");
    assert_eq!(value(&runtime, entity), original);
    shortcut(&mut runtime, &mut input, "y");
    assert_eq!(value(&runtime, entity), rich);
}

#[test]
fn plain_preedit_and_commit_use_the_same_default_style() {
    let style = RichSpanStyle::new().bold().background_color(RED).effect(7);
    let original = RichText::new("ab").with_span(0..2, style.clone());
    let (mut runtime, mut shaper, mut input, entity) =
        editor(original.clone(), RichTypingStylePolicy::Plain);
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::Select(1..1))
        .unwrap();
    input
        .composition(runtime.context_mut(), CompositionInput::Start)
        .unwrap();
    input
        .composition(
            runtime.context_mut(),
            CompositionInput::Update {
                text: "中文".into(),
                selection: Some((6, 6)),
            },
        )
        .unwrap();
    settle(&mut runtime, &mut shaper);
    let display = runtime
        .context()
        .world()
        .rich_text(entity.stable_id())
        .unwrap();
    let preedit = display.style_at(1).unwrap();
    assert!(preedit.shape.is_empty());
    assert_eq!(preedit.paint.background_color, None);
    assert_eq!(preedit.effect, None);
    assert!(preedit.paint.decoration.unwrap().underline);
    assert_eq!(value(&runtime, entity), original);
    input
        .composition(
            runtime.context_mut(),
            CompositionInput::Commit("中文".into()),
        )
        .unwrap();
    assert!(value(&runtime, entity).style_at(1).is_none());
    assert_eq!(value(&runtime, entity).style_at(7), Some(&style));
}

#[test]
fn plain_paste_uses_defaults_and_rich_clipboard_keeps_styles_and_ruby() {
    let mut original =
        RichText::new("AB").with_span(0..2, RichSpanStyle::new().bold().background_color(RED));
    original.set_ruby(0..2, "annotation");
    let (mut runtime, _, mut input, entity) =
        editor(original.clone(), RichTypingStylePolicy::Plain);
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::SelectAll)
        .unwrap();
    shortcut(&mut runtime, &mut input, "c");
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::Select(2..2))
        .unwrap();
    shortcut(&mut runtime, &mut input, "v");
    let pasted = value(&runtime, entity);
    assert_eq!(pasted.text(), "ABAB");
    assert_eq!(pasted.style_at(2), original.style_at(0));
    assert_eq!(&**pasted.ruby_at(2).unwrap().1, "annotation");
    // 外部纯文本重写相同正文也会更换剪贴板身份。
    input.services_mut().write_clipboard("AB").unwrap();
    shortcut(&mut runtime, &mut input, "v");
    let plain = value(&runtime, entity);
    assert_eq!(plain.text(), "ABABAB");
    assert!(plain.style_at(4).is_none());
    assert!(plain.ruby_at(4).is_none());
    input
        .services_mut()
        .write_clipboard("plain clipboard")
        .unwrap();
    shortcut(&mut runtime, &mut input, "v");
    assert!(value(&runtime, entity).style_at(6).is_none());
}

/// 同正文的两个私有宿主各自保留来源；外部纯文本重写后采用默认样式。
#[test]
fn identical_clipboard_text_keeps_each_hosts_own_rich_source() {
    let bold = RichSpanStyle::new().bold();
    let italic = RichSpanStyle::new().italic(true);
    let (mut first, _, mut first_input, first_editor) = editor(
        RichText::new("same").with_span(0..4, bold.clone()),
        RichTypingStylePolicy::Plain,
    );
    let (mut second, _, mut second_input, second_editor) = editor(
        RichText::new("same").with_span(0..4, italic.clone()),
        RichTypingStylePolicy::Plain,
    );
    for (runtime, input, entity, style) in [
        (&mut first, &mut first_input, first_editor, &bold),
        (&mut second, &mut second_input, second_editor, &italic),
    ] {
        runtime
            .context_mut()
            .rich_edit(entity, RichEditCommand::SelectAll)
            .unwrap();
        shortcut(runtime, input, "c");
        assert_eq!(value(runtime, entity).style_at(0), Some(style));
    }
    for (runtime, input, entity, style) in [
        (&mut first, &mut first_input, first_editor, &bold),
        (&mut second, &mut second_input, second_editor, &italic),
    ] {
        runtime
            .context_mut()
            .rich_edit(entity, RichEditCommand::Select(4..4))
            .unwrap();
        shortcut(runtime, input, "v");
        assert_eq!(value(runtime, entity).style_at(4), Some(style));
        input.services_mut().write_clipboard("same").unwrap();
        shortcut(runtime, input, "v");
        assert!(value(runtime, entity).style_at(8).is_none());
    }
}

#[test]
fn inherit_remains_default_and_policy_reconciliation_preserves_selection() {
    let original = RichText::new("ab").with_span(0..2, RichSpanStyle::new().bold());
    let (mut runtime, _, mut input, entity) =
        editor(original.clone(), RichTypingStylePolicy::Inherit);
    assert_eq!(
        RichTextEditor::new("").typing_style_policy,
        RichTypingStylePolicy::Inherit
    );
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::Select(1..1))
        .unwrap();
    runtime
        .context_mut()
        .set_component(
            entity,
            RichTextEditor::new(original).typing_style_policy(RichTypingStylePolicy::Plain),
        )
        .unwrap();
    assert_eq!(
        runtime
            .context()
            .read(entity, RichTextEditor::selection)
            .unwrap(),
        (1, 1)
    );
    input.text(runtime.context_mut(), "X").unwrap();
    assert!(value(&runtime, entity).style_at(1).is_none());
}

#[test]
fn background_quads_follow_multiline_geometry_and_stay_below_selection_and_text() {
    let rich =
        RichText::new("Hello\nworld").with_span(0..11, RichSpanStyle::new().background_color(RED));
    let (mut runtime, mut shaper, _, entity) = editor(rich, RichTypingStylePolicy::Inherit);
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::SelectAll)
        .unwrap();
    settle(&mut runtime, &mut shaper);
    let primitives: Vec<_> = runtime
        .scene()
        .primitives()
        .filter(|p| p.node == entity.stable_id())
        .collect();
    let backgrounds: Vec<_> = primitives.iter().enumerate().filter(|(_, p)| matches!(p.kind, ScenePrimitiveKind::Quad { background: Some(color), .. } if color == RED)).collect();
    assert_eq!(backgrounds.len(), 2);
    assert!(backgrounds[0].1.bounds.width > 5.0);
    assert!(backgrounds[1].1.bounds.y > backgrounds[0].1.bounds.y);
    let (_, layout) = runtime
        .context()
        .world()
        .text_layout(entity.stable_id())
        .unwrap();
    let rects = layout.selection_rects(0..11);
    assert_eq!(rects.len(), backgrounds.len());
    for ((_, primitive), rect) in backgrounds.iter().zip(&rects) {
        assert_eq!(primitive.bounds.width, rect.width);
        assert_eq!(primitive.bounds.height, rect.height);
    }
    let text_index = primitives
        .iter()
        .position(|p| matches!(p.kind, ScenePrimitiveKind::Text { .. }))
        .unwrap();
    let selection_index = primitives
        .iter()
        .position(|p| p.id.slot >> 32 == 6)
        .unwrap();
    assert!(
        backgrounds
            .iter()
            .all(|(index, _)| *index < selection_index)
    );
    assert!(selection_index < text_index);
}

#[test]
fn background_only_changes_keep_retained_layout_and_report_mixed_state() {
    let original = RichText::new("abcd");
    let (mut runtime, mut shaper, _, entity) = editor(original, RichTypingStylePolicy::Inherit);
    let (handle, before) = runtime
        .context()
        .world()
        .text_layout(entity.stable_id())
        .unwrap();
    let before = Arc::clone(before);
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::Select(0..2))
        .unwrap();
    runtime
        .context_mut()
        .rich_edit(
            entity,
            RichEditCommand::SetAttrs(RichSpanStyle::new().background_color(RED)),
        )
        .unwrap();
    runtime
        .flush(LayoutViewport::new(400.0, 300.0), &mut shaper)
        .unwrap();
    let work = runtime.context().world().last_text_work_counters();
    assert_eq!(work.text_nodes_shaped, 0);
    assert_eq!(work.layouts_created, 0);
    let (after_handle, after) = runtime
        .context()
        .world()
        .text_layout(entity.stable_id())
        .unwrap();
    assert_eq!(handle, after_handle);
    assert!(Arc::ptr_eq(&before, after));
    let observed = Arc::new(Mutex::new(None));
    let captured = observed.clone();
    runtime
        .context_mut()
        .on(
            entity,
            move |_: &mut RichTextEditor, event: &nana_ui_runtime::RichTextEditorEvent, _| {
                if let nana_ui_runtime::RichTextEditorEvent::SelectionChanged { attrs, .. } = event
                {
                    *captured.lock().unwrap() = Some(attrs.clone());
                }
            },
        )
        .unwrap();
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::SelectAll)
        .unwrap();
    let attrs = observed.lock().unwrap().clone().unwrap();
    assert!(attrs.mixed.contains(RichAttrMask::BACKGROUND_COLOR));
    assert_eq!(attrs.style.paint.background_color, None);
}

#[test]
fn wrapped_background_is_removed_without_relayout_when_formatting_clears() {
    let text = "自动换行背景矩形".repeat(10);
    let rich = RichText::new(text.clone())
        .with_span(0..text.len(), RichSpanStyle::new().background_color(RED));
    let (mut runtime, mut shaper, _, entity) = editor(rich, RichTypingStylePolicy::Plain);
    let count = runtime
        .scene()
        .primitives()
        .filter(|p| {
            p.node == entity.stable_id()
                && matches!(p.kind, ScenePrimitiveKind::Quad { background: Some(color), .. } if color == RED)
        })
        .count();
    let (before, layout) = runtime
        .context()
        .world()
        .text_layout(entity.stable_id())
        .unwrap();
    assert!(count > 2);
    assert_eq!(count, layout.selection_rects(0..text.len()).len());
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::SelectAll)
        .unwrap();
    runtime
        .context_mut()
        .rich_edit(entity, RichEditCommand::ClearFormatting)
        .unwrap();
    runtime
        .flush(LayoutViewport::new(400.0, 300.0), &mut shaper)
        .unwrap();
    assert_eq!(
        runtime
            .context()
            .world()
            .text_layout(entity.stable_id())
            .unwrap()
            .0,
        before
    );
    assert_eq!(
        runtime
            .context()
            .world()
            .last_text_work_counters()
            .text_nodes_shaped,
        0
    );
    assert_eq!(
        runtime
            .scene()
            .primitives()
            .filter(|p| {
                p.node == entity.stable_id()
                    && matches!(p.kind, ScenePrimitiveKind::Quad { background: Some(color), .. } if color == RED)
            })
            .count(),
        0
    );
}
