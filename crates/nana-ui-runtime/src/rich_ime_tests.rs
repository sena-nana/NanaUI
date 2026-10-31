//! 富文本输入法快照、组合周边删除与只读隔离回归。
use super::*;
use crate::RichEditCommand;

#[test]
fn rich_ime_snapshot_keeps_preview_cursor_and_original_selection() {
    let mut editor = RichTextEditor::new("前中文后");
    editor.apply_command(RichEditCommand::Select(3..9));
    editor.state.preedit = Some(("ni😀".into(), 2));
    assert_eq!(
        editor.ime_snapshot(),
        Some(RichImeSnapshot {
            text: "前ni😀后".into(),
            selection: (5, 5),
            composition: Some(3..9),
        })
    );
    assert!(editor.delete_ime_surrounding(3, 3));
    assert_eq!(editor.value.text(), "中文");
    assert_eq!(editor.selection(), (0, 6));
    assert_eq!(editor.ime_snapshot().unwrap().text, "ni😀");
    assert!(editor.apply_command(RichEditCommand::Undo));
    assert_eq!(editor.value.text(), "前中文后");
}

#[test]
fn rich_ime_rejects_partial_characters_and_read_only_snapshots() {
    let mut editor = RichTextEditor::new("中😀文");
    editor.apply_command(RichEditCommand::Select(7..7));
    assert!(!editor.delete_ime_surrounding(1, 0));
    assert_eq!(editor.value.text(), "中😀文");
    editor.read_only = true;
    assert_eq!(editor.ime_snapshot(), None);
    editor.read_only = false;
    editor.disabled = true;
    assert_eq!(editor.ime_snapshot(), None);
}
