//! `RichTextEditor` on the product path: it lays out exactly as the
//! `RichTextView` it edits for, takes typing, IME and the clipboard through
//! the canonical input route, and draws its caret and selection from the
//! layout its text is drawn from.

use std::sync::{Arc, Mutex};

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_text::{NativeTextEngine, SharedTextEngine};
use nana_ui_core::{
    FlexDirection, LayoutStyle, LengthSpec, PaintColor, RichObject, RichSpanStyle, RichText,
};
use nana_ui_input::{CompositionInput, InputModifiers, KeyInput, KeyState, PointerPhase};
use nana_ui_runtime::{
    DocumentId, Entity, HeadlessInput, LayoutViewport, MutationQueue, NanaTextEngineShaper,
    NodeKind, NodeStyle, RichEditCommand, RichTextEditor, RichTextView, StableNodeId, TextShaper,
};
use nana_ui_scene::{RuntimeDocument, ScenePrimitiveKind};

const WIDTH: f32 = 180.0;

fn engine() -> SharedTextEngine {
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_bytes(
            font_blob(nana_ui_core::fonts::UI_FONT_REGULAR),
            &FaceDescriptor::default(),
        )
        .expect("the bundled UI face registers");
    Arc::new(Mutex::new(NativeTextEngine::new(fonts)))
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(400.0, 600.0)
}

fn settle(runtime: &mut RuntimeDocument, shaper: &mut impl TextShaper) {
    for _ in 0..5 {
        runtime.flush(viewport(), shaper).unwrap();
    }
}

fn document_id() -> DocumentId {
    DocumentId::new(1).unwrap()
}

/// A column holding a display and an editor of the same document, both
/// `WIDTH` wide at the same size.
fn document(value: RichText) -> (RuntimeDocument, StableNodeId, Entity<RichTextEditor>) {
    let document = document_id();
    let mut runtime = RuntimeDocument::new(document);
    let mut queue = MutationQueue::new();
    let root = StableNodeId::new(1).unwrap();
    queue.create(root, document, NodeKind::Document);
    let column = StableNodeId::new(2).unwrap();
    queue.create(column, document, NodeKind::Element { tag: "div".into() });
    queue.insert(root, column, None);
    queue.set_style(
        column,
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(400.0)),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    runtime.context_mut().commit_mutations(queue).unwrap();
    let cx = runtime.context_mut();
    let view = cx
        .create_component(
            document,
            RichTextView::new(value.clone())
                .font_size(18.0)
                .width(LengthSpec::Px(WIDTH)),
        )
        .unwrap();
    let editor = cx
        .create_component(
            document,
            RichTextEditor::new(value)
                .font_size(18.0)
                .width(LengthSpec::Px(WIDTH)),
        )
        .unwrap();
    let mut queue = MutationQueue::new();
    queue.insert(column, view.stable_id(), None);
    queue.insert(column, editor.stable_id(), None);
    cx.commit_mutations(queue).unwrap();
    (runtime, view.stable_id(), editor)
}

fn dialogue() -> RichText {
    RichText::builder()
        .plain("今天的直播")
        .push("非常", RichSpanStyle::new().size(30.0).bold())
        .plain("开心，")
        .object(RichObject::image(1, "file:///s/heart.png", 24.0, 24.0))
        .object(RichObject::chip(2, "wait 500", 0))
        .push(
            "谢谢大家的支持和陪伴",
            RichSpanStyle::new().color(PaintColor::srgb([1.0, 0.3, 0.4, 1.0])),
        )
        .plain("！")
        .build()
}

fn lines(runtime: &RuntimeDocument, id: StableNodeId) -> Vec<(std::ops::Range<usize>, u32, u32)> {
    let (_, layout) = runtime
        .context()
        .world()
        .text_layout(id)
        .expect("laid out through the rich path");
    layout
        .lines
        .iter()
        .map(|line| {
            (
                line.source.clone(),
                line.metrics.width_px.to_bits(),
                line.metrics.height_px.to_bits(),
            )
        })
        .collect()
}

fn editor_value(runtime: &RuntimeDocument, editor: Entity<RichTextEditor>) -> RichText {
    runtime
        .context()
        .read(editor, |view| view.value.clone())
        .unwrap()
}

fn key(name: &'static str, modifiers: InputModifiers) -> KeyInput {
    KeyInput::named(name, name, KeyState::Pressed, modifiers)
}

fn primary() -> InputModifiers {
    InputModifiers {
        control: true,
        ..InputModifiers::default()
    }
}

/// Click into the editor: a press and release at its first line.
fn focus(runtime: &mut RuntimeDocument, input: &mut HeadlessInput, editor: Entity<RichTextEditor>) {
    let bounds = runtime
        .context()
        .world()
        .layout_box(editor.stable_id())
        .unwrap();
    let (x, y) = (bounds.x + 2.0, bounds.y + 4.0);
    input
        .pointer(runtime.context_mut(), PointerPhase::Down, x, y)
        .unwrap();
    input
        .pointer(runtime.context_mut(), PointerPhase::Up, x, y)
        .unwrap();
}

#[test]
fn the_editor_breaks_its_lines_exactly_where_the_display_does() {
    let (mut runtime, view, editor) = document(dialogue());
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let shown = lines(&runtime, view);
    assert!(shown.len() >= 3, "the dialogue wraps: {shown:?}");
    assert_eq!(
        lines(&runtime, editor.stable_id()),
        shown,
        "same document, same width: the same lines, to the bit"
    );
    let chips = runtime
        .scene()
        .primitives()
        .filter(|primitive| primitive.node == view)
        .filter(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Quad { .. }))
        .filter(|primitive| primitive.bounds.width <= 2.0)
        .count();
    assert_eq!(chips, 0, "the display draws no marker");
}

#[test]
fn typing_ime_and_undo_reach_the_document_through_the_input_route() {
    let (mut runtime, _, editor) = document(RichText::new("ab"));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let mut input = HeadlessInput::bind(runtime.context_mut(), document_id());
    focus(&mut runtime, &mut input, editor);
    assert_eq!(
        runtime.context().focused_rich_editor(document_id()),
        Some(editor),
        "a click focuses the editor"
    );
    runtime
        .context_mut()
        .rich_edit(editor, RichEditCommand::Select(2..2))
        .unwrap();
    input
        .press(
            runtime.context_mut(),
            key("KeyC", InputModifiers::default()),
            Some("c"),
            None,
        )
        .unwrap();
    assert_eq!(editor_value(&runtime, editor).text(), "abc");
    assert!(
        input.services().text_input().is_some(),
        "the IME is told where the editor's caret is"
    );

    input
        .composition(runtime.context_mut(), CompositionInput::Start)
        .unwrap();
    input
        .composition(
            runtime.context_mut(),
            CompositionInput::Update {
                text: "にほ".into(),
                selection: None,
            },
        )
        .unwrap();
    settle(&mut runtime, &mut shaper);
    assert_eq!(
        runtime.context().world().text(editor.stable_id()),
        Some("abcにほ")
    );
    assert_eq!(
        editor_value(&runtime, editor).text(),
        "abc",
        "a preedit is shown, not committed"
    );
    input
        .composition(
            runtime.context_mut(),
            CompositionInput::Commit("日本".into()),
        )
        .unwrap();
    assert_eq!(editor_value(&runtime, editor).text(), "abc日本");

    input
        .press(
            runtime.context_mut(),
            key("Backspace", InputModifiers::default()),
            None,
            None,
        )
        .unwrap();
    assert_eq!(editor_value(&runtime, editor).text(), "abc日");
    input
        .press(runtime.context_mut(), key("z", primary()), None, None)
        .unwrap();
    assert_eq!(
        editor_value(&runtime, editor).text(),
        "abc日本",
        "undo the delete"
    );
}

#[test]
fn a_rich_copy_pastes_back_with_its_styling() {
    let bold = RichSpanStyle::new().bold();
    let value = RichText::new("hello world").with_span(0..5, bold.clone());
    let (mut runtime, _, editor) = document(value);
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let mut input = HeadlessInput::bind(runtime.context_mut(), document_id());
    focus(&mut runtime, &mut input, editor);
    runtime
        .context_mut()
        .rich_edit(editor, RichEditCommand::Select(0..5))
        .unwrap();
    input
        .press(runtime.context_mut(), key("c", primary()), None, None)
        .unwrap();
    assert_eq!(input.services().clipboard(), Some("hello"));
    let end = "hello world".len();
    runtime
        .context_mut()
        .rich_edit(editor, RichEditCommand::Select(end..end))
        .unwrap();
    input
        .press(runtime.context_mut(), key("v", primary()), None, None)
        .unwrap();
    let pasted = editor_value(&runtime, editor);
    assert_eq!(pasted.text(), "hello worldhello");
    assert_eq!(pasted.style_at(end), Some(&bold), "the piece kept its bold");
}

#[test]
fn the_caret_and_selection_are_drawn_from_the_editors_own_layout() {
    let (mut runtime, _, editor) = document(RichText::new("select me"));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let mut input = HeadlessInput::bind(runtime.context_mut(), document_id());
    focus(&mut runtime, &mut input, editor);
    runtime
        .context_mut()
        .rich_edit(editor, RichEditCommand::Select(0..6))
        .unwrap();
    settle(&mut runtime, &mut shaper);
    let quads: Vec<_> = runtime
        .scene()
        .primitives()
        .filter(|primitive| primitive.node == editor.stable_id())
        .filter(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Quad { .. }))
        .map(|primitive| primitive.bounds)
        .collect();
    let caret = quads.iter().find(|bounds| bounds.width < 2.0);
    let selection = quads.iter().find(|bounds| bounds.width > 10.0);
    assert!(
        caret.is_some(),
        "a focused editor draws its caret: {quads:?}"
    );
    let selection = selection.expect("and its selection");
    let (_, layout) = runtime
        .context()
        .world()
        .text_layout(editor.stable_id())
        .unwrap();
    let expected = layout.selection_rects(0..6)[0];
    assert!((selection.width - expected.width).abs() < 0.01);
}

#[test]
fn toolbar_attributes_restyle_the_selection_without_reflowing_a_colour() {
    let (mut runtime, _, editor) = document(RichText::new("make it red"));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let before = lines(&runtime, editor.stable_id());
    let red = PaintColor::srgb([1.0, 0.0, 0.0, 1.0]);
    runtime
        .context_mut()
        .rich_edit(editor, RichEditCommand::Select(8..11))
        .unwrap();
    assert!(
        runtime
            .context_mut()
            .rich_edit(
                editor,
                RichEditCommand::SetAttrs(RichSpanStyle::new().color(red))
            )
            .unwrap()
    );
    runtime.flush(viewport(), &mut shaper).unwrap();
    let work = runtime.context().world().last_text_work_counters();
    assert_eq!(
        work.text_nodes_shaped, 0,
        "a colour shapes nothing: {work:?}"
    );
    assert_eq!(lines(&runtime, editor.stable_id()), before);
    let value = editor_value(&runtime, editor);
    assert_eq!(
        value.style_at(9).and_then(|style| style.paint.color),
        Some(red)
    );
    let summary = runtime
        .context()
        .read(editor, RichTextEditor::selection_attrs)
        .unwrap();
    assert_eq!(summary.style.paint.color, Some(red));
}

#[test]
fn arrows_move_and_extend_by_character_and_line_edges() {
    let (mut runtime, _, editor) = document(RichText::new("one two"));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let mut input = HeadlessInput::bind(runtime.context_mut(), document_id());
    focus(&mut runtime, &mut input, editor);
    let selection = |runtime: &RuntimeDocument| {
        runtime
            .context()
            .read(editor, RichTextEditor::selection)
            .unwrap()
    };
    runtime
        .context_mut()
        .rich_edit(editor, RichEditCommand::Select(0..0))
        .unwrap();
    input
        .press(
            runtime.context_mut(),
            key("End", InputModifiers::default()),
            None,
            None,
        )
        .unwrap();
    assert_eq!(selection(&runtime), (7, 7), "End reaches the line's end");
    let shift = InputModifiers {
        shift: true,
        ..InputModifiers::default()
    };
    input
        .press(runtime.context_mut(), key("ArrowLeft", shift), None, None)
        .unwrap();
    assert_eq!(selection(&runtime), (7, 6), "Shift extends from the anchor");
    input
        .press(
            runtime.context_mut(),
            key("ArrowLeft", primary()),
            None,
            None,
        )
        .unwrap();
    assert_eq!(selection(&runtime), (4, 4), "a word back");
    input
        .press(
            runtime.context_mut(),
            key("Home", InputModifiers::default()),
            None,
            None,
        )
        .unwrap();
    assert_eq!(selection(&runtime), (0, 0));
}

#[test]
fn a_ruby_set_from_the_editor_lays_out_as_the_display_lays_it_out() {
    let plain = RichText::new("今天的直播非常开心");
    let (mut runtime, view, editor) = document(plain.clone());
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let flat = lines(&runtime, editor.stable_id());
    runtime
        .context_mut()
        .rich_edit(editor, RichEditCommand::Select(15..21))
        .unwrap();
    assert!(
        runtime
            .context_mut()
            .rich_edit(editor, RichEditCommand::SetRuby(Some("fēicháng".into())))
            .unwrap()
    );
    let annotated = editor_value(&runtime, editor);
    assert_eq!(annotated.rubies().len(), 1);
    assert_eq!(annotated.rubies()[0].0, 15..21);
    runtime
        .context_mut()
        .update_component(Entity::<RichTextView>::from_stable_id(view), |shown, _| {
            shown.value = annotated.clone();
        })
        .unwrap();
    settle(&mut runtime, &mut shaper);
    let (_, layout) = runtime
        .context()
        .world()
        .text_layout(editor.stable_id())
        .expect("laid out");
    assert_eq!(layout.rubies.len(), 1, "the annotation is placed");
    assert!(
        lines(&runtime, editor.stable_id())[0].2 > flat[0].2,
        "its line grows to hold it"
    );
    assert_eq!(
        lines(&runtime, editor.stable_id()),
        lines(&runtime, view),
        "the editor and the display agree with the annotation in"
    );
    assert!(
        runtime
            .context_mut()
            .rich_edit(editor, RichEditCommand::SetRuby(None))
            .unwrap()
    );
    assert!(editor_value(&runtime, editor).rubies().is_empty());
    assert!(
        runtime
            .context_mut()
            .rich_edit(editor, RichEditCommand::Undo)
            .unwrap()
    );
    assert_eq!(
        editor_value(&runtime, editor).rubies().len(),
        1,
        "undo brings it back"
    );
}
