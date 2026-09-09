//! The one place a text editor's value changes, and the undo journal built on
//! it.
//!
//! Every path that writes editor text — typing, deletion, paste, an IME
//! commit, a line transform, a snippet, the application setting a value —
//! goes through [`AppContext::commit_editor_edit`]. Concerns that apply to
//! *all* edits live there once instead of in each caller: emitting the change
//! event, refusing a read-only editor, and recording undo.
//!
//! Composition is deliberately invisible here. IME preedit lives in the
//! world's `ime` slot, not in the editor's value, so the journal never sees
//! the half-typed states of a composition — only the commit, as one step.

use std::collections::HashMap;

use crate::{StableNodeId, TextInputState};

/// Why an edit happened. Decides whether it extends the previous undo step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEditOrigin {
    /// Inserted text as the user typed. A run of typing collapses into one
    /// undo step, so undo does not walk back a character at a time.
    Typing,
    /// Removed text. Consecutive deletions collapse the same way.
    Delete,
    /// One paste is one step, however much it inserted.
    Paste,
    /// One committed composition is one step.
    Ime,
    /// A transform over lines or selections: move, sort, case, comment,
    /// snippet. Always its own step.
    Structural,
    /// The application wrote the value. Not the user's edit, so it clears the
    /// journal rather than becoming a step the user can undo into.
    Program,
    /// Undo or redo itself. Never recorded.
    History,
}

impl TextEditOrigin {
    /// Whether a new edit of this origin continues the previous step instead
    /// of starting a new one.
    fn merges_with(self, previous: Self) -> bool {
        matches!(
            (previous, self),
            (Self::Typing, Self::Typing) | (Self::Delete, Self::Delete)
        )
    }
}

/// One undoable step: the state before and after.
///
/// Stores whole states rather than diffs. An editor's undo depth is bounded
/// and its documents are the size a person edits by hand; a diff
/// representation would buy little and has to be re-derived on every merge.
#[derive(Debug, Clone, PartialEq)]
struct TextEditStep {
    before: TextInputState,
    after: TextInputState,
    origin: TextEditOrigin,
}

/// Undo journal for one editor.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct TextHistory {
    steps: Vec<TextEditStep>,
    /// Steps before this index are undoable; steps from it on are redoable.
    cursor: usize,
}

impl TextHistory {
    /// Undo depth per editor. Deep enough that a person does not hit it in a
    /// session, bounded so a long-lived editor cannot grow without limit.
    const CAPACITY: usize = 200;

    fn record(&mut self, before: TextInputState, after: TextInputState, origin: TextEditOrigin) {
        if origin == TextEditOrigin::History {
            return;
        }
        // A programmatic write is not something the user performed, so there
        // is nothing meaningful to undo back through.
        if origin == TextEditOrigin::Program {
            self.steps.clear();
            self.cursor = 0;
            return;
        }
        // Anything after the cursor was undone; a fresh edit replaces it.
        self.steps.truncate(self.cursor);
        if let Some(last) = self.steps.last_mut()
            && origin.merges_with(last.origin)
        {
            last.after = after;
            return;
        }
        self.steps.push(TextEditStep {
            before,
            after,
            origin,
        });
        if self.steps.len() > Self::CAPACITY {
            self.steps.remove(0);
        }
        self.cursor = self.steps.len();
    }

    /// Ends the current merge run, so the next edit starts a new step even if
    /// it is the same origin. Moving the caret or changing focus does this:
    /// typing, arrowing away, then typing again is two steps.
    fn seal(&mut self) {
        if let Some(last) = self.steps.last_mut()
            && matches!(last.origin, TextEditOrigin::Typing | TextEditOrigin::Delete)
        {
            last.origin = TextEditOrigin::Structural;
        }
    }

    fn undo(&mut self) -> Option<TextInputState> {
        let index = self.cursor.checked_sub(1)?;
        self.cursor = index;
        Some(self.steps[index].before.clone())
    }

    fn redo(&mut self) -> Option<TextInputState> {
        let step = self.steps.get(self.cursor)?;
        let after = step.after.clone();
        self.cursor += 1;
        Some(after)
    }

    fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    fn can_redo(&self) -> bool {
        self.cursor < self.steps.len()
    }
}

/// Per-editor journals.
#[derive(Debug, Default)]
pub(super) struct TextHistories {
    entries: HashMap<StableNodeId, TextHistory>,
}

impl TextHistories {
    pub(super) fn record(
        &mut self,
        node: StableNodeId,
        before: TextInputState,
        after: TextInputState,
        origin: TextEditOrigin,
    ) {
        self.entries
            .entry(node)
            .or_default()
            .record(before, after, origin);
    }

    pub(super) fn seal(&mut self, node: StableNodeId) {
        if let Some(history) = self.entries.get_mut(&node) {
            history.seal();
        }
    }

    pub(super) fn undo(&mut self, node: StableNodeId) -> Option<TextInputState> {
        self.entries.get_mut(&node)?.undo()
    }

    pub(super) fn redo(&mut self, node: StableNodeId) -> Option<TextInputState> {
        self.entries.get_mut(&node)?.redo()
    }

    pub(super) fn can_undo(&self, node: StableNodeId) -> bool {
        self.entries.get(&node).is_some_and(TextHistory::can_undo)
    }

    pub(super) fn can_redo(&self, node: StableNodeId) -> bool {
        self.entries.get(&node).is_some_and(TextHistory::can_redo)
    }

    /// Releases the journal of a node that no longer exists.
    pub(super) fn forget(&mut self, node: StableNodeId) {
        self.entries.remove(&node);
    }
}

impl crate::AppContext {
    /// The single place an editor's text state changes.
    ///
    /// `apply` mutates the component; everything that must happen for *every*
    /// edit happens here: the change event, and the undo journal. Callers pass
    /// why the edit happened so consecutive typing or deletion collapses into
    /// one undo step.
    ///
    /// Returns whether the state actually changed.
    pub(super) fn commit_editor_edit<C: super::EditableText>(
        &mut self,
        entity: crate::Entity<C>,
        origin: TextEditOrigin,
        apply: impl FnOnce(&mut C, &mut crate::ViewContext<'_, C>) -> bool,
    ) -> Result<bool, crate::FrameworkError> {
        let before = self.read(entity, |editable: &C| editable.state().clone())?;
        let changed = self.update_component(entity, |editable: &mut C, cx| {
            if !apply(editable, cx) {
                return false;
            }
            cx.emit(editable.change());
            true
        })?;
        if !changed {
            return Ok(false);
        }
        let after = self.read(entity, |editable: &C| editable.state().clone())?;
        if after.value != before.value {
            self.text_histories
                .record(entity.stable_id(), before, after, origin);
        }
        Ok(true)
    }

    /// Undoes the focused editor's last edit. Returns whether anything moved.
    ///
    /// Restores the selection the edit started from, so undoing puts the caret
    /// back where the user was working rather than where the edit ended.
    /// A composition in progress is left alone: undo during preedit would fight
    /// the IME.
    pub fn undo_focused_text(
        &mut self,
        document: crate::DocumentId,
    ) -> Result<bool, crate::FrameworkError> {
        self.step_focused_text_history(document, true)
    }

    /// Redoes the edit the last [`Self::undo_focused_text`] took back.
    pub fn redo_focused_text(
        &mut self,
        document: crate::DocumentId,
    ) -> Result<bool, crate::FrameworkError> {
        self.step_focused_text_history(document, false)
    }

    fn step_focused_text_history(
        &mut self,
        document: crate::DocumentId,
        undo: bool,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_input {
            return Ok(false);
        }
        let node = focused.node;
        let Some(target) = (if undo {
            self.text_histories.undo(node)
        } else {
            self.text_histories.redo(node)
        }) else {
            return Ok(false);
        };
        // `History` keeps the restore from becoming a step of its own.
        let restore = move |state: &mut crate::TextInputState| *state = target;
        match focused.kind {
            super::text_edit::TextEditorKind::Area => self.commit_editor_edit(
                crate::Entity::<crate::TextArea>::from_stable_id(node),
                TextEditOrigin::History,
                move |area: &mut crate::TextArea, _| {
                    restore(&mut area.state);
                    true
                },
            ),
            super::text_edit::TextEditorKind::Field => self.commit_editor_edit(
                crate::Entity::<crate::TextInput>::from_stable_id(node),
                TextEditOrigin::History,
                move |field: &mut crate::TextInput, _| {
                    restore(&mut field.state);
                    true
                },
            ),
        }
    }

    /// Ends the current typing or deletion run for an editor, so the next edit
    /// starts a new undo step. Caret moves and focus changes call this.
    pub(super) fn seal_editor_history(&mut self, node: StableNodeId) {
        self.text_histories.seal(node);
    }

    /// Whether the editor has an edit to undo.
    pub fn can_undo_text(&self, node: StableNodeId) -> bool {
        self.text_histories.can_undo(node)
    }

    /// Whether the editor has an undone edit to redo.
    pub fn can_redo_text(&self, node: StableNodeId) -> bool {
        self.text_histories.can_redo(node)
    }
}

#[cfg(test)]
mod editor_tests {
    use crate::{AppContext, DocumentId, TextArea, TextDeleteKind, TextInput};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn focused_area(cx: &mut AppContext, value: &str) -> crate::Entity<TextArea> {
        let area = cx
            .create_component(document(), TextArea::new(value))
            .unwrap();
        cx.focus_node(document(), area.stable_id()).unwrap();
        area
    }

    fn value_of(cx: &AppContext, area: crate::Entity<TextArea>) -> String {
        cx.read(area, |area| area.state.value.clone()).unwrap()
    }

    #[test]
    fn a_composition_is_one_undo_step_and_its_preedit_is_never_a_step() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");

        // Preedit lives in the world's IME slot, not in the value, so these
        // intermediate states must not become undoable steps.
        cx.set_ime_preedit(document(), "ni".to_owned(), None)
            .unwrap();
        cx.set_ime_preedit(document(), "nih".to_owned(), None)
            .unwrap();
        cx.set_ime_preedit(document(), "niha".to_owned(), None)
            .unwrap();
        assert!(
            !cx.can_undo_text(area.stable_id()),
            "composition in flight is not an edit yet"
        );

        cx.commit_ime(document(), "你好").unwrap();
        assert_eq!(value_of(&cx, area), "你好");

        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "", "one commit undoes as one step");
        assert!(!cx.can_undo_text(area.stable_id()));

        assert!(cx.redo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "你好");
    }

    #[test]
    fn a_typing_run_undoes_as_one_step_and_a_caret_move_splits_it() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");

        for character in ["a", "b", "c"] {
            cx.replace_focused_text(document(), character).unwrap();
        }
        assert_eq!(value_of(&cx, area), "abc");

        // Arrow away, then type again: the second run is a separate step.
        cx.move_focused_text_caret(document(), crate::TextCaretIntent::Left, false, None)
            .unwrap();
        cx.replace_focused_text(document(), "X").unwrap();

        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "abc", "only the second run came back");
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "");
        assert!(!cx.can_undo_text(area.stable_id()));
    }

    #[test]
    fn undo_restores_the_multi_cursor_selection_the_edit_started_from() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "one\ntwo\nthree");

        // Two cursors, then one edit across both.
        cx.select_focused_text_range(document(), 0, 0).unwrap();
        cx.add_focused_text_cursor(document(), false, None).unwrap();
        let before = cx
            .read(area, |area| area.state.additional_selections.len())
            .unwrap();
        assert_eq!(before, 1, "the second cursor is live");

        cx.replace_focused_text(document(), "#").unwrap();
        assert_eq!(value_of(&cx, area), "#one\n#two\nthree");

        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "one\ntwo\nthree");
        assert_eq!(
            cx.read(area, |area| area.state.additional_selections.len())
                .unwrap(),
            before,
            "the cursor set the edit started from is restored"
        );
    }

    #[test]
    fn an_application_write_is_not_something_the_user_can_undo_into() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "draft").unwrap();
        assert!(cx.can_undo_text(area.stable_id()));

        // Loading a different document is not the user's edit.
        cx.apply_accessibility_action(
            document(),
            crate::AccessibilityActionRequest {
                target: area.stable_id(),
                action: crate::AccessibilityAction::SetValue("loaded from disk".to_owned()),
            },
        )
        .unwrap();
        assert!(
            !cx.can_undo_text(area.stable_id()),
            "undo must not walk back into the previous document"
        );
        assert_eq!(value_of(&cx, area), "loaded from disk");
    }

    #[test]
    fn a_read_only_editor_has_nothing_to_undo() {
        let mut cx = AppContext::new();
        let field = cx
            .create_component(document(), TextInput::new("fixed").read_only(true))
            .unwrap();
        cx.focus_node(document(), field.stable_id()).unwrap();

        cx.replace_focused_text(document(), "x").unwrap();
        assert!(!cx.can_undo_text(field.stable_id()));
        assert!(!cx.undo_focused_text(document()).unwrap());
    }

    #[test]
    fn deletion_collapses_into_its_own_run_separate_from_typing() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "abcdef").unwrap();
        for _ in 0..3 {
            cx.delete_focused_text(document(), TextDeleteKind::Backward)
                .unwrap();
        }
        assert_eq!(value_of(&cx, area), "abc");

        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "abcdef", "three deletes, one step");
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(value: &str) -> TextInputState {
        TextInputState::new(value)
    }

    fn record(history: &mut TextHistory, from: &str, to: &str, origin: TextEditOrigin) {
        history.record(state(from), state(to), origin);
    }

    #[test]
    fn a_run_of_typing_is_one_undo_step() {
        let mut history = TextHistory::default();
        record(&mut history, "", "a", TextEditOrigin::Typing);
        record(&mut history, "a", "ab", TextEditOrigin::Typing);
        record(&mut history, "ab", "abc", TextEditOrigin::Typing);

        assert_eq!(history.undo().map(|state| state.value), Some(String::new()));
        assert!(!history.can_undo(), "the run collapsed into one step");
        assert_eq!(
            history.redo().map(|state| state.value),
            Some("abc".to_owned())
        );
    }

    #[test]
    fn a_different_origin_starts_a_new_step() {
        let mut history = TextHistory::default();
        record(&mut history, "", "abc", TextEditOrigin::Typing);
        record(&mut history, "abc", "ab", TextEditOrigin::Delete);
        record(&mut history, "ab", "ab!", TextEditOrigin::Paste);

        assert_eq!(history.undo().map(|s| s.value), Some("ab".to_owned()));
        assert_eq!(history.undo().map(|s| s.value), Some("abc".to_owned()));
        assert_eq!(history.undo().map(|s| s.value), Some(String::new()));
        assert!(!history.can_undo());
    }

    #[test]
    fn sealing_breaks_a_typing_run() {
        let mut history = TextHistory::default();
        record(&mut history, "", "ab", TextEditOrigin::Typing);
        history.seal();
        record(&mut history, "ab", "abcd", TextEditOrigin::Typing);

        assert_eq!(history.undo().map(|s| s.value), Some("ab".to_owned()));
        assert_eq!(history.undo().map(|s| s.value), Some(String::new()));
    }

    #[test]
    fn editing_after_undo_drops_the_redo_tail() {
        let mut history = TextHistory::default();
        record(&mut history, "", "one", TextEditOrigin::Paste);
        record(&mut history, "one", "two", TextEditOrigin::Paste);
        history.undo();
        assert!(history.can_redo());

        record(&mut history, "one", "three", TextEditOrigin::Paste);
        assert!(!history.can_redo(), "the abandoned branch is gone");
        assert_eq!(history.undo().map(|s| s.value), Some("one".to_owned()));
    }

    #[test]
    fn an_application_write_clears_what_the_user_could_undo_into() {
        let mut history = TextHistory::default();
        record(&mut history, "", "typed", TextEditOrigin::Typing);
        record(&mut history, "typed", "loaded", TextEditOrigin::Program);

        assert!(
            !history.can_undo(),
            "a document swap is not the user's edit"
        );
        assert!(!history.can_redo());
    }

    #[test]
    fn undo_and_redo_are_never_recorded_as_steps() {
        let mut history = TextHistory::default();
        record(&mut history, "", "a", TextEditOrigin::Paste);
        record(&mut history, "a", "", TextEditOrigin::History);
        assert_eq!(history.steps.len(), 1);
    }

    #[test]
    fn the_journal_is_bounded() {
        let mut history = TextHistory::default();
        for index in 0..(TextHistory::CAPACITY + 50) {
            record(
                &mut history,
                &index.to_string(),
                &(index + 1).to_string(),
                TextEditOrigin::Paste,
            );
        }
        assert_eq!(history.steps.len(), TextHistory::CAPACITY);
        assert!(history.can_undo());
    }
}
