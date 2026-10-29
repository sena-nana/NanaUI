use super::*;
use crate::{
    Entity, LayoutBox, MutationQueue, NanaTextEngineShaper, RichClipboardCommand, RichEditCommand,
    RichTextEditor, TextEditAction,
};
use nana_ui_core::{RichSpanStyle, RichText};
use nana_ui_input::{HostServiceError, HostServices};
use std::sync::{Arc, Mutex};

/// The rich clipboard is one per process: tests that copy out of a rich
/// editor take turns.
static RICH_CLIPBOARD_TESTS: Mutex<()> = Mutex::new(());

fn fixture(
    value: &str,
) -> (
    AppContext,
    DocumentId,
    Entity<RichTextEditor>,
    NanaTextEngineShaper,
) {
    let mut fonts = nana_text::font::FontSystem::hermetic();
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../nana-ui/assets/fonts/NotoSansSC-Variable.ttf"
            ),
            &nana_text::font::FaceDescriptor::default(),
        )
        .unwrap();
    let engine = Arc::new(Mutex::new(nana_text::NativeTextEngine::new(fonts)));
    let mut shaper = NanaTextEngineShaper::new(engine);
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let editor = cx
        .create_component(doc, RichTextEditor::new(value))
        .unwrap();
    let mut q = MutationQueue::new();
    q.write_layout(
        editor.stable_id(),
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        },
    );
    cx.commit_mutations(q).unwrap();
    let work = cx.take_system_work();
    cx.resolve_styles(&work.style).unwrap();
    cx.shape_text(&work.text, &mut shaper).unwrap();
    cx.rebuild_hit_test(doc);
    (cx, doc, editor, shaper)
}

fn key(name: &str, modifiers: InputModifiers) -> Gesture {
    Gesture::Event(InputPayload::Key(KeyInput::named(
        "Unidentified",
        match name {
            "End" => "End",
            "Home" => "Home",
            "ArrowDown" => "ArrowDown",
            "ArrowUp" => "ArrowUp",
            "PageDown" => "PageDown",
            "PageUp" => "PageUp",
            "Enter" => "Enter",
            "Escape" => "Escape",
            "ArrowLeft" => "ArrowLeft",
            "Backspace" => "Backspace",
            _ => panic!("test key"),
        },
        KeyState::Pressed,
        modifiers,
    )))
}

fn pointer(phase: PointerPhase, x: f32, y: f32) -> Gesture {
    Gesture::Event(InputPayload::Pointer(PointerInput::mouse(phase, x, y)))
}

#[test]
fn rich_editor_wheel_pins_the_visible_viewport_and_clicks_use_its_offset() {
    let value = "first line\n".repeat(50);
    let (mut cx, doc, editor, _) = fixture(&value);
    let node = editor.stable_id();
    let mut input = TestInput::default();
    let wheel = wheel_fixture! { x:40.0,y:40.0,delta_x:0.0,delta_y:-120.0,line_delta:false,modifiers:Default::default() };
    assert!(input.dispatch(&mut cx, doc, &wheel).unwrap().handled);
    assert_eq!(cx.world().scroll_offset(node).unwrap().y, 120.0);
    assert_eq!(cx.read(editor, |e| e.selection()).unwrap(), (0, 0));
    cx.focus_node(doc, node).unwrap();
    input
        .dispatch(
            &mut cx,
            doc,
            &key(
                "End",
                InputModifiers {
                    control: true,
                    ..Default::default()
                },
            ),
        )
        .unwrap();
    let bottom = cx.world().scroll_offset(node).unwrap().y;
    let up = wheel_fixture! { x:40.0,y:40.0,delta_x:0.0,delta_y:60.0,line_delta:false,modifiers:Default::default() };
    input.dispatch(&mut cx, doc, &up).unwrap();
    let visible = cx.world().scroll_offset(node).unwrap().y;
    assert_eq!(visible, bottom - 60.0);
    cx.update_component(editor, |e, _| e.read_only = false)
        .unwrap();
    assert_eq!(
        cx.world().scroll_offset(node).unwrap().y,
        visible,
        "projection must not reveal a stationary caret"
    );
    let (_, layout) = cx.world().text_layout(node).unwrap();
    let expected = layout
        .hit_test_text(&value, 40.0, visible + 40.0)
        .caret
        .byte;
    input
        .dispatch(&mut cx, doc, &pointer(PointerPhase::Down, 40.0, 40.0))
        .unwrap();
    input
        .dispatch(&mut cx, doc, &pointer(PointerPhase::Up, 40.0, 40.0))
        .unwrap();
    assert_eq!(
        cx.read(editor, |e| e.selection()).unwrap(),
        (expected, expected)
    );
    assert_eq!(
        cx.read(editor, |e| e.value.text().to_owned()).unwrap(),
        value
    );
}

#[test]
fn rich_editor_vertical_and_page_selection_preserve_the_preferred_column() {
    let (mut cx, doc, editor, _) = fixture(&"abcdefghij\nx\nabcdefghij\n".repeat(8));
    let mut input = TestInput::default();
    cx.focus_node(doc, editor.stable_id()).unwrap();
    cx.rich_edit(editor, RichEditCommand::Select(8..8)).unwrap();
    input
        .dispatch(&mut cx, doc, &key("ArrowDown", Default::default()))
        .unwrap();
    input
        .dispatch(&mut cx, doc, &key("ArrowDown", Default::default()))
        .unwrap();
    assert_eq!(cx.read(editor, |e| e.selection().1).unwrap(), 21);
    let start = cx.read(editor, |e| e.selection().1).unwrap();
    input
        .dispatch(
            &mut cx,
            doc,
            &key(
                "PageDown",
                InputModifiers {
                    shift: true,
                    ..Default::default()
                },
            ),
        )
        .unwrap();
    let (anchor, focus) = cx.read(editor, |e| e.selection()).unwrap();
    assert_eq!(anchor, start);
    assert!(focus > start);
    input
        .dispatch(
            &mut cx,
            doc,
            &key(
                "PageUp",
                InputModifiers {
                    shift: true,
                    ..Default::default()
                },
            ),
        )
        .unwrap();
    assert_eq!(cx.read(editor, |e| e.selection().0).unwrap(), start);
}

#[test]
fn rich_editor_multi_click_and_stationary_edge_drag_extend_the_selection() {
    let (mut cx, doc, editor, _) = fixture(&"hello world\n".repeat(40));
    let mut input = TestInput::default();
    for tick in [0, 100] {
        input
            .dispatch_at(
                &mut cx,
                doc,
                &pointer(PointerPhase::Down, 12.0, 8.0),
                Duration::from_millis(tick),
            )
            .unwrap();
        input
            .dispatch_at(
                &mut cx,
                doc,
                &pointer(PointerPhase::Up, 12.0, 8.0),
                Duration::from_millis(tick + 1),
            )
            .unwrap();
    }
    assert_eq!(cx.read(editor, |e| e.selection()).unwrap(), (0, 5));
    input
        .dispatch_at(
            &mut cx,
            doc,
            &pointer(PointerPhase::Down, 12.0, 8.0),
            Duration::from_millis(200),
        )
        .unwrap();
    assert_eq!(cx.read(editor, |e| e.selection()).unwrap(), (0, 12));
    input
        .dispatch_at(
            &mut cx,
            doc,
            &pointer(PointerPhase::Move, 12.0, 120.0),
            Duration::from_millis(210),
        )
        .unwrap();
    let scroll = cx.world().scroll_offset(editor.stable_id()).unwrap().y;
    assert!(cx.rich_drag_deadline().is_some());
    cx.advance_animations(Duration::from_millis(240));
    assert!(cx.world().scroll_offset(editor.stable_id()).unwrap().y > scroll);
    input
        .dispatch_at(
            &mut cx,
            doc,
            &pointer(PointerPhase::Up, 12.0, 120.0),
            Duration::from_millis(250),
        )
        .unwrap();
    assert!(cx.rich_drag_deadline().is_none());
}

#[test]
fn rich_editor_composition_bypasses_application_shortcuts_and_commits_once() {
    let (mut cx, doc, editor, mut shaper) = fixture("hello ");
    cx.focus_node(doc, editor.stable_id()).unwrap();
    cx.rich_edit(editor, RichEditCommand::Select(6..6)).unwrap();
    let calls = Arc::new(Mutex::new(0));
    let observed = calls.clone();
    cx.on_key(editor, move |_| {
        *observed.lock().unwrap() += 1;
        true
    })
    .unwrap();
    let mut input = TestInput::default();
    for composition in [
        CompositionInput::Start,
        CompositionInput::Update {
            text: "ni".into(),
            selection: None,
        },
    ] {
        input
            .dispatch(
                &mut cx,
                doc,
                &Gesture::Event(InputPayload::Composition(composition)),
            )
            .unwrap();
    }
    for name in ["Enter", "Escape", "ArrowUp", "ArrowDown"] {
        assert!(
            input
                .dispatch(&mut cx, doc, &key(name, Default::default()))
                .unwrap()
                .handled
        );
    }
    assert_eq!(*calls.lock().unwrap(), 0);
    input
        .dispatch(
            &mut cx,
            doc,
            &Gesture::Event(InputPayload::Composition(CompositionInput::Commit(
                "你".into(),
            ))),
        )
        .unwrap();
    let work = cx.take_system_work();
    cx.shape_text(&work.text, &mut shaper).unwrap();
    assert_eq!(
        cx.read(editor, |e| e.value.text().to_owned()).unwrap(),
        "hello 你"
    );
}

#[test]
fn editor_actions_share_clipboard_and_undo_for_rich_and_source() {
    let _turn = RICH_CLIPBOARD_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (mut cx, doc, rich, _) = fixture("a中文👨‍👩‍👧‍👦b");
    let mut services = HeadlessHostServices::new();
    cx.rich_edit(rich, RichEditCommand::SelectAll).unwrap();
    for action in [
        TextEditAction::Copy,
        TextEditAction::Cut,
        TextEditAction::Paste,
        TextEditAction::Undo,
        TextEditAction::Redo,
    ] {
        assert!(
            cx.edit_text_action(doc, rich.stable_id(), action, &mut services)
                .unwrap()
        );
    }
    assert_eq!(
        cx.read(rich, |e| e.value.text().to_owned()).unwrap(),
        "a中文👨‍👩‍👧‍👦b"
    );
    let source = cx
        .create_component(doc, crate::TextArea::new("abc"))
        .unwrap();
    cx.edit_text_area_value(source, "[b]abc[/b]".into(), crate::TextSelection::caret(10))
        .unwrap();
    assert!(
        cx.edit_text_action(doc, source.stable_id(), TextEditAction::Undo, &mut services)
            .unwrap()
    );
    assert_eq!(
        cx.read(source, |e| e.state.value.to_string()).unwrap(),
        "abc"
    );
    assert!(
        cx.edit_text_action(doc, source.stable_id(), TextEditAction::Redo, &mut services)
            .unwrap()
    );
    assert_eq!(
        cx.read(source, |e| e.state.value.to_string()).unwrap(),
        "[b]abc[/b]"
    );
}

/// A host that keeps no clipboard change count, as X11 and Wayland do.
struct TextOnlyClipboard(HeadlessHostServices);

impl HostServices for TextOnlyClipboard {
    fn set_cursor(&mut self, cursor: nana_ui_input::CursorIcon) {
        self.0.set_cursor(cursor);
    }
    fn set_text_input(&mut self, state: Option<&nana_ui_input::TextInputContext>) {
        self.0.set_text_input(state);
    }
    fn read_clipboard(&mut self) -> Result<Option<String>, HostServiceError> {
        self.0.read_clipboard()
    }
    fn write_clipboard(&mut self, text: &str) -> Result<(), HostServiceError> {
        self.0.write_clipboard(text)
    }
}

/// A paste takes the editor's own copy back with its styling only while the
/// clipboard still holds that copy's write; the same text written by anyone
/// else pastes as plain text. Without a change count the text decides. The
/// editor types plain, so a plain paste shows as unstyled next to bold.
#[test]
fn a_rich_paste_needs_the_clipboard_to_still_hold_the_copy() {
    let _turn = RICH_CLIPBOARD_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let bold = RichSpanStyle::new().bold();
    let (mut cx, _, rich, _) = fixture("hello world");
    cx.update_component(rich, |e, _| {
        e.typing_style_policy = crate::RichTypingStylePolicy::Plain;
    })
    .unwrap();
    cx.rich_edit(rich, RichEditCommand::Select(0..5)).unwrap();
    cx.rich_edit(rich, RichEditCommand::SetAttrs(bold.clone()))
        .unwrap();
    let value = |cx: &AppContext| cx.read(rich, |e| e.value.clone()).unwrap();
    let paste_at_end =
        |cx: &mut AppContext, command: RichClipboardCommand, services: &mut dyn HostServices| {
            let end = value(cx).text().len();
            cx.rich_edit(rich, RichEditCommand::Select(end..end))
                .unwrap();
            assert!(cx.rich_clipboard(rich, command, services).unwrap());
            let pasted = value(cx);
            assert_eq!(&pasted.text()[end..], "hello");
            pasted.style_at(end).cloned()
        };
    let copy = |cx: &mut AppContext, services: &mut dyn HostServices| {
        cx.rich_edit(rich, RichEditCommand::Select(0..5)).unwrap();
        assert!(
            cx.rich_clipboard(rich, RichClipboardCommand::Copy, services)
                .unwrap()
        );
    };

    let mut services = HeadlessHostServices::new();
    copy(&mut cx, &mut services);
    assert_eq!(services.clipboard(), Some("hello"), "plain text only");
    assert_eq!(
        paste_at_end(&mut cx, RichClipboardCommand::Paste, &mut services),
        Some(bold.clone()),
        "the editor's own copy keeps its bold"
    );
    assert_eq!(
        paste_at_end(&mut cx, RichClipboardCommand::PastePlainText, &mut services),
        None,
        "pasting plain text drops it"
    );
    services.set_clipboard(Some("hello".into()));
    assert_eq!(
        paste_at_end(&mut cx, RichClipboardCommand::Paste, &mut services),
        None,
        "the same text from another program is plain"
    );

    let mut text_only = TextOnlyClipboard(HeadlessHostServices::new());
    copy(&mut cx, &mut text_only);
    text_only.0.set_clipboard(Some("hello".into()));
    assert_eq!(
        paste_at_end(&mut cx, RichClipboardCommand::Paste, &mut text_only),
        Some(bold),
        "without a change count the matching text stands in"
    );
    text_only.0.set_clipboard(Some("other".into()));
    cx.rich_edit(rich, RichEditCommand::SelectAll).unwrap();
    assert!(
        cx.rich_clipboard(rich, RichClipboardCommand::Paste, &mut text_only)
            .unwrap()
    );
    assert_eq!(value(&cx), RichText::new("other"), "other text is plain");
}

/// Copy reads a read-only editor; cut and paste leave it, and nothing
/// selected copies nothing.
#[test]
fn rich_clipboard_commands_respect_the_editor_state() {
    let _turn = RICH_CLIPBOARD_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (mut cx, _, rich, _) = fixture("keep");
    let mut services = HeadlessHostServices::new();
    services.set_clipboard(Some("before".into()));
    assert!(
        !cx.rich_clipboard(rich, RichClipboardCommand::Copy, &mut services)
            .unwrap(),
        "nothing selected"
    );
    assert_eq!(services.clipboard(), Some("before"));
    cx.update_component(rich, |e, _| e.read_only = true)
        .unwrap();
    cx.rich_edit(rich, RichEditCommand::SelectAll).unwrap();
    for command in [
        RichClipboardCommand::Cut,
        RichClipboardCommand::Paste,
        RichClipboardCommand::PastePlainText,
    ] {
        assert!(
            !cx.rich_clipboard(rich, command, &mut services).unwrap(),
            "{command:?}"
        );
    }
    assert!(
        cx.rich_clipboard(rich, RichClipboardCommand::Copy, &mut services)
            .unwrap()
    );
    assert_eq!(services.clipboard(), Some("keep"));
    assert_eq!(
        cx.read(rich, |e| e.value.text().to_owned()).unwrap(),
        "keep"
    );
}

#[test]
fn source_editor_edge_drag_repeats_until_release_and_ime_owns_shortcuts() {
    let (mut cx, doc, _, mut shaper) = fixture("");
    let mut area = crate::TextArea::new("hello world\n".repeat(100));
    area.state.selection = crate::TextSelection::caret(0);
    let source = cx.create_component(doc, area).unwrap();
    let mut q = MutationQueue::new();
    q.write_layout(
        source.stable_id(),
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        },
    );
    cx.commit_mutations(q).unwrap();
    let work = cx.take_system_work();
    cx.resolve_styles(&work.style).unwrap();
    cx.shape_text(&work.text, &mut shaper).unwrap();
    cx.rebuild_hit_test(doc);
    let mut input = TestInput::default();
    input
        .dispatch_with_shaper(
            &mut cx,
            doc,
            &pointer(PointerPhase::Down, 12.0, 20.0),
            Duration::ZERO,
            Some(&mut shaper),
        )
        .unwrap();
    input
        .dispatch_with_shaper(
            &mut cx,
            doc,
            &pointer(PointerPhase::Up, 12.0, 20.0),
            Duration::from_millis(1),
            Some(&mut shaper),
        )
        .unwrap();
    input
        .dispatch_with_shaper(
            &mut cx,
            doc,
            &pointer(PointerPhase::Down, 12.0, 20.0),
            Duration::from_millis(10),
            Some(&mut shaper),
        )
        .unwrap();
    assert!(
        !cx.read(source, |area| area.state.selection.range().is_empty())
            .unwrap()
    );
    input
        .dispatch_with_shaper(
            &mut cx,
            doc,
            &pointer(PointerPhase::Move, 12.0, 130.0),
            Duration::from_millis(20),
            Some(&mut shaper),
        )
        .unwrap();
    let before = cx.world().scroll_offset(source.stable_id()).unwrap().y;
    assert!(cx.plain_drag_deadline().is_some());
    cx.advance_animations(Duration::from_millis(60));
    assert!(cx.world().scroll_offset(source.stable_id()).unwrap().y > before);
    input
        .dispatch_with_shaper(
            &mut cx,
            doc,
            &pointer(PointerPhase::Up, 12.0, 130.0),
            Duration::from_millis(80),
            Some(&mut shaper),
        )
        .unwrap();
    assert!(cx.plain_drag_deadline().is_none());
    let called = Arc::new(Mutex::new(0));
    let observed = called.clone();
    cx.on_key(source, move |_| {
        *observed.lock().unwrap() += 1;
        true
    })
    .unwrap();
    input
        .dispatch(
            &mut cx,
            doc,
            &Gesture::Event(InputPayload::Composition(CompositionInput::Start)),
        )
        .unwrap();
    for name in ["Enter", "Escape", "ArrowUp"] {
        input
            .dispatch(&mut cx, doc, &key(name, Default::default()))
            .unwrap();
    }
    assert_eq!(*called.lock().unwrap(), 0);
}

#[test]
fn rich_editor_shift_drag_keeps_the_original_anchor_and_right_click_keeps_selection() {
    let (mut cx, doc, editor, _) = fixture("abcdefghijklmnop");
    cx.focus_node(doc, editor.stable_id()).unwrap();
    cx.rich_edit(editor, RichEditCommand::Select(2..2)).unwrap();
    let (_, layout) = cx.world().text_layout(editor.stable_id()).unwrap();
    let (x10, y, height) = crate::rich_text_editor::caret_box(layout, 10).unwrap();
    let (x5, _, _) = crate::rich_text_editor::caret_box(layout, 5).unwrap();
    let y = y + height / 2.0;
    let mut down = PointerInput::mouse(PointerPhase::Down, x10, y);
    down.modifiers.shift = true;
    let mut input = TestInput::default();
    input
        .dispatch(&mut cx, doc, &Gesture::Event(InputPayload::Pointer(down)))
        .unwrap();
    input
        .dispatch(&mut cx, doc, &pointer(PointerPhase::Move, x5, y))
        .unwrap();
    input
        .dispatch(&mut cx, doc, &pointer(PointerPhase::Up, x5, y))
        .unwrap();
    assert_eq!(cx.read(editor, |e| e.selection()).unwrap(), (2, 5));
    let secondary = PointerInput {
        button: 2,
        buttons: 2,
        ..PointerInput::mouse(PointerPhase::Down, x10, y)
    };
    input
        .dispatch(
            &mut cx,
            doc,
            &Gesture::Event(InputPayload::Pointer(secondary)),
        )
        .unwrap();
    assert_eq!(cx.read(editor, |e| e.selection()).unwrap(), (2, 5));
}

#[test]
fn rich_editor_paragraph_selection_and_grapheme_deletion_use_text_boundaries() {
    let (mut cx, doc, editor, _) = fixture("abc\n甲👨‍👩‍👧‍👦e\u{301}乙");
    let mut input = TestInput::default();
    cx.focus_node(doc, editor.stable_id()).unwrap();
    cx.rich_edit(editor, RichEditCommand::Select(1..1)).unwrap();
    input
        .dispatch(
            &mut cx,
            doc,
            &key(
                "ArrowDown",
                InputModifiers {
                    control: true,
                    shift: true,
                    ..Default::default()
                },
            ),
        )
        .unwrap();
    assert_eq!(cx.read(editor, |e| e.selection()).unwrap(), (1, 4));
    input
        .dispatch(
            &mut cx,
            doc,
            &key(
                "End",
                InputModifiers {
                    control: true,
                    ..Default::default()
                },
            ),
        )
        .unwrap();
    for name in ["ArrowLeft", "Backspace", "Backspace"] {
        input
            .dispatch(&mut cx, doc, &key(name, Default::default()))
            .unwrap();
    }
    assert_eq!(
        cx.read(editor, |e| e.value.text().to_owned()).unwrap(),
        "abc\n甲乙"
    );
    cx.rich_edit(editor, RichEditCommand::SelectAll).unwrap();
    assert!(
        !cx.edit_text_action(
            doc,
            editor.stable_id(),
            TextEditAction::Cut,
            &mut nana_ui_input::UnsupportedHostServices
        )
        .unwrap()
    );
    assert_eq!(
        cx.read(editor, |e| e.value.text().to_owned()).unwrap(),
        "abc\n甲乙"
    );
}

#[test]
fn ctrl_backspace_deletes_one_word_in_both_editor_types() {
    let (mut cx, doc, rich, _) = fixture("one two three");
    let source = cx
        .create_component(doc, crate::TextArea::new("one two three"))
        .unwrap();
    for node in [rich.stable_id(), source.stable_id()] {
        cx.focus_node(doc, node).unwrap();
        let mut input = TestInput::default();
        let ctrl = InputModifiers {
            control: true,
            ..Default::default()
        };
        for name in ["End", "Backspace"] {
            input.dispatch(&mut cx, doc, &key(name, ctrl)).unwrap();
        }
    }
    assert_eq!(
        cx.read(rich, |e| e.value.text().to_owned()).unwrap(),
        "one two "
    );
    assert_eq!(
        cx.read(source, |e| e.state.value.to_string()).unwrap(),
        "one two "
    );
}
