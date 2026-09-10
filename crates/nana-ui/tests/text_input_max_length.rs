use nana_ui::{RuntimeInputAdapter, runtime::*};
use nana_ui_platform::{ImeEvent, InputEvent, InputModifiers, MemoryClipboard, shared_clipboard};
use std::sync::Arc;

fn key(value: &str, shortcut: bool) -> InputEvent {
    InputEvent::Keyboard {
        pressed: true,
        key: value.into(),
        code: value.into(),
        text: (!shortcut && value.chars().count() == 1).then(|| value.into()),
        repeat: false,
        modifiers: InputModifiers {
            control: shortcut,
            ..Default::default()
        },
    }
}

#[test]
fn maxlength_normal_keyboard_paste_selection_and_ime_commit_share_utf16_budget() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let input = cx
        .create_component(doc, TextInput::new("ab").max_length(5))
        .unwrap();
    cx.focus_node(doc, input.stable_id()).unwrap();
    let clipboard = shared_clipboard(MemoryClipboard::new());
    let mut adapter = RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
    adapter.dispatch(&mut cx, doc, &key("😀", false)).unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("ab😀"));
    clipboard.lock().unwrap().write_text("😀z");
    adapter.dispatch(&mut cx, doc, &key("v", true)).unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("ab😀"));
    clipboard.lock().unwrap().write_text("Z😀");
    adapter.dispatch(&mut cx, doc, &key("v", true)).unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("ab😀Z"));
    adapter.dispatch(&mut cx, doc, &key("a", true)).unwrap();
    clipboard.lock().unwrap().write_text("😀😀😀");
    adapter.dispatch(&mut cx, doc, &key("v", true)).unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("😀😀"));
    adapter
        .dispatch_ime(
            &mut cx,
            doc,
            &ImeEvent::Preedit {
                text: "你好世界".into(),
                selection: None,
            },
        )
        .unwrap();
    assert_eq!(cx.world().ime(input.stable_id()).unwrap().text, "你好世界");
    assert_eq!(cx.world().text(input.stable_id()), Some("😀😀"));
    adapter
        .dispatch_ime(&mut cx, doc, &ImeEvent::Commit("你好世界".into()))
        .unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("😀😀你"));
    assert!(cx.world().ime(input.stable_id()).is_none());
}

#[test]
fn maxlength_programmatic_overlong_values_are_preserved_but_user_growth_is_blocked() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let input = cx
        .create_component(doc, TextInput::new("oversized").max_length(3))
        .unwrap();
    cx.focus_node(doc, input.stable_id()).unwrap();
    let mut adapter = RuntimeInputAdapter::default();
    adapter.dispatch(&mut cx, doc, &key("X", false)).unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("oversized"));
    adapter
        .dispatch(&mut cx, doc, &key("Backspace", false))
        .unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("oversize"));
    cx.update_component(input, |view, _| {
        view.state = TextInputState::new("loaded 😀 value")
    })
    .unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("loaded 😀 value"));
    adapter.dispatch(&mut cx, doc, &key("a", true)).unwrap();
    adapter.dispatch(&mut cx, doc, &key("好", false)).unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("好"));
}

#[test]
fn maxlength_multiple_cursors_share_total_budget_and_ime_only_replaces_primary() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let mut view = TextInput::new("ab").max_length(7);
    view.state.selection = TextSelection::caret(0);
    view.state.additional_selections = vec![TextSelection::caret(2)];
    let input = cx.create_component(doc, view).unwrap();
    cx.focus_node(doc, input.stable_id()).unwrap();
    let mut adapter = RuntimeInputAdapter::default();
    adapter.dispatch(&mut cx, doc, &key("😀", false)).unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("😀ab😀"));
    adapter.dispatch(&mut cx, doc, &key("X", false)).unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("😀ab😀"));
    adapter
        .dispatch_ime(&mut cx, doc, &ImeEvent::Commit("你好".into()))
        .unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("😀你ab😀"));
    assert_eq!(
        cx.read(input, |view| view.state.additional_selections.len())
            .unwrap(),
        1
    );
}

#[test]
fn maxlength_advanced_replace_rejects_growth_without_changing_selection_and_accepts_shortening() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let input = cx
        .create_component(doc, TextInput::new("ab ab").max_length(5))
        .unwrap();
    cx.focus_node(doc, input.stable_id()).unwrap();
    cx.update_component(input, |view, _| {
        view.state.selection = TextSelection {
            anchor: 0,
            focus: 2,
        }
    })
    .unwrap();
    let before = cx.read(input, |view| view.state.clone()).unwrap();
    assert!(
        !cx.replace_focused_text_match(doc, "ab", TextSearchOptions::default(), "😀ab", false)
            .unwrap()
    );
    assert_eq!(cx.read(input, |view| view.state.clone()).unwrap(), before);
    assert_eq!(
        cx.replace_all_focused_text_matches(
            doc,
            "ab",
            TextSearchOptions::default(),
            "abcdef",
            TextFindScope::Document,
            false
        )
        .unwrap(),
        0
    );
    assert_eq!(cx.read(input, |view| view.state.clone()).unwrap(), before);
    assert_eq!(
        cx.replace_all_focused_text_matches(
            doc,
            "ab",
            TextSearchOptions::default(),
            "x",
            TextFindScope::Document,
            false
        )
        .unwrap(),
        2
    );
    assert_eq!(cx.world().text(input.stable_id()), Some("x x"));
}

#[test]
fn maxlength_identical_paste_and_ime_still_collapse_the_replaced_selection() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let input = cx
        .create_component(doc, TextInput::new("abc").max_length(3))
        .unwrap();
    cx.focus_node(doc, input.stable_id()).unwrap();
    let clipboard = shared_clipboard(MemoryClipboard::new());
    clipboard.lock().unwrap().write_text("abc");
    let mut adapter = RuntimeInputAdapter::default().with_clipboard(clipboard);
    adapter.dispatch(&mut cx, doc, &key("a", true)).unwrap();
    adapter.dispatch(&mut cx, doc, &key("v", true)).unwrap();
    assert_eq!(
        cx.read(input, |view| view.state.selection).unwrap(),
        TextSelection::caret(3)
    );
    adapter.dispatch(&mut cx, doc, &key("a", true)).unwrap();
    adapter
        .dispatch_ime(
            &mut cx,
            doc,
            &ImeEvent::Preedit {
                text: "abc".into(),
                selection: None,
            },
        )
        .unwrap();
    adapter
        .dispatch_ime(&mut cx, doc, &ImeEvent::Commit("abc".into()))
        .unwrap();
    assert_eq!(cx.world().text(input.stable_id()), Some("abc"));
    assert_eq!(
        cx.read(input, |view| view.state.selection).unwrap(),
        TextSelection::caret(3)
    );
    assert!(cx.world().ime(input.stable_id()).is_none());
}
