//! The one place a text editor's value changes, and the undo journal built on
//! it.
//!
//! Every framework path that writes editor text — typing, deletion, paste,
//! an IME commit, a line transform, a snippet, assistive technology setting a
//! value, an application edit made on the user's behalf — goes through
//! [`AppContext::commit_editor_edit`]. Concerns that apply to
//! *all* edits live there once instead of in each caller: emitting the change
//! event and recording undo. Whether the editor accepts input at all is the
//! caller's check, made before it builds the edit.
//!
//! Any other write that changes an editor's text replaces its value, and
//! clears its journal as a [`TextEditOrigin::Program`] edit does: the
//! application writing the component (`update_component`, `set_component`, a
//! keyed `mount`) or committing `SetTextInput` to the world directly. Undo
//! after loading another document therefore does not walk back into the
//! previous one. The journal notices by the text's [`TextStamp`], which
//! changes only with the bytes: each journal keeps the stamp of the text the
//! last write it saw left. A commit through [`AppContext::commit_mutations`]
//! checks the editors it wrote as soon as it lands, so a replaced document's
//! snapshots are freed at once; a write straight to the world is caught on
//! the journal's next use. A write that leaves the bytes as they were — the
//! value the editor reported, or the same text rebuilt — keeps the stamp and
//! the journal.
//!
//! A composite component that writes a child editor's text (a color field's
//! hex input, a path field's text) replaces that child's value the same way,
//! and undo could not bring back text the composite's own state no longer
//! matches. Such a component writes the child only when the value really
//! changes: [`crate::ColorField`] leaves text that already names its color
//! as the user typed it.
//!
//! An application edit meant to be undone like the user's own — a format
//! shortcut, a completion — goes through
//! [`crate::AppContext::edit_text_area`] or
//! [`crate::AppContext::edit_text_input`], which replace a range the way the
//! user's own edit would, as a step of its own. Rebinding an editor to another object
//! whose text happens to be identical changes no byte; call
//! [`crate::AppContext::clear_text_history`] for that.
//!
//! Only a [`crate::TextArea`] or [`crate::TextInput`] keeps a journal: their
//! whole state is their text, so a snapshot of it restores them. A number
//! field's committed value, or a picker's query and filtered list, would not
//! follow a restored text, so those editors record nothing (and
//! `can_undo_text` never offers them an undo).
//!
//! Composition is deliberately invisible here. IME preedit lives in the
//! world's `ime` slot, not in the editor's value, so the journal never sees
//! the half-typed states of a composition — only the commit, as one step.

use std::collections::HashMap;

use nana_text::TextStamp;

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
    /// Removing the selection to the pasteboard. Its own step, which typing
    /// after it does not extend.
    Cut,
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

impl TextEditStep {
    /// Keeps a later edit of the same origin from merging into this step.
    fn seal(&mut self) {
        if matches!(self.origin, TextEditOrigin::Typing | TextEditOrigin::Delete) {
            self.origin = TextEditOrigin::Structural;
        }
    }
}

/// Undo journal for one editor.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct TextHistory {
    steps: Vec<TextEditStep>,
    /// Steps before this index are undoable; steps from it on are redoable.
    cursor: usize,
    /// The stamp of the editor's text after the last write the journal saw
    /// (see [`TextHistories::witness`]). Reads never move it. A stamp changes
    /// only with the bytes, so a different one means something outside the
    /// journal wrote the text since.
    held: Option<TextStamp>,
}

impl TextHistory {
    /// Undo depth per editor. Deep enough that a person does not hit it in a
    /// session, bounded so a long-lived editor cannot grow without limit.
    const CAPACITY: usize = 200;

    /// Text bytes the journal of one editor may hold, across every step.
    ///
    /// [`Self::CAPACITY`] alone bounds the journal only for documents of a
    /// hand-written size: a step keeps the value before AND after it, so 200
    /// steps of a 300 KB document would be 60–120 MB per editor. Deep undo is
    /// worth memory, but not that much of it -- past this the oldest steps go,
    /// which is what a step-count overflow does too.
    const CAPACITY_BYTES: usize = 8 * 1024 * 1024;

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
        while self.steps.len() > Self::CAPACITY
            || (self.steps.len() > 1 && self.bytes() > Self::CAPACITY_BYTES)
        {
            self.steps.remove(0);
        }
        self.cursor = self.steps.len();
    }

    /// Text bytes the steps hold. Both ends of every step: undo restores the
    /// `before`, redo the `after`.
    ///
    /// Counted per buffer, not per snapshot: a step's `after` and the next
    /// step's `before` are usually one shared buffer (Issue #182), and it
    /// is memory once.
    fn bytes(&self) -> usize {
        let mut seen = std::collections::HashSet::with_capacity(self.steps.len() * 2);
        self.steps
            .iter()
            .flat_map(|step| [&step.before.value, &step.after.value])
            .filter(|value| seen.insert((value.as_ptr() as usize, value.len())))
            .map(|value| value.len())
            .sum()
    }

    /// Ends the current merge run, so the next edit starts a new step even if
    /// it is the same origin. Moving the caret or changing focus does this:
    /// typing, arrowing away, then typing again is two steps.
    fn seal(&mut self) {
        if let Some(last) = self.steps.last_mut() {
            last.seal();
        }
    }

    /// Moves the cursor one step back (`undo`) or forward, once the state
    /// [`Self::peek`] showed has been restored.
    ///
    /// Undo and redo end a merge run too: typing after an undo is a new
    /// step, not more of the one the undo stepped back onto.
    fn step(&mut self, undo: bool) {
        if undo {
            if self.cursor == 0 {
                return;
            }
            self.cursor -= 1;
        } else {
            if self.cursor == self.steps.len() {
                return;
            }
            self.cursor += 1;
        }
        self.seal_before_cursor();
    }

    /// The state [`Self::undo`] or [`Self::redo`] would restore, without
    /// moving the cursor or sealing anything.
    fn peek(&self, undo: bool) -> Option<&TextInputState> {
        if undo {
            Some(&self.steps.get(self.cursor.checked_sub(1)?)?.before)
        } else {
            Some(&self.steps.get(self.cursor)?.after)
        }
    }

    /// Seals the step the next edit would merge into.
    fn seal_before_cursor(&mut self) {
        if let Some(step) = self
            .cursor
            .checked_sub(1)
            .and_then(|index| self.steps.get_mut(index))
        {
            step.seal();
        }
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
    /// Editors a journaled write is in flight for. The edit's own commit
    /// lands before the write can witness it, so a commit leaves these to
    /// the write, which settles them when it finishes (see
    /// [`crate::AppContext::commit_editor_edit`]).
    writing: Vec<StableNodeId>,
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

    pub(super) fn peek(&self, node: StableNodeId, undo: bool) -> Option<TextInputState> {
        self.entries.get(&node)?.peek(undo).cloned()
    }

    pub(super) fn step(&mut self, node: StableNodeId, undo: bool) {
        if let Some(history) = self.entries.get_mut(&node) {
            history.step(undo);
        }
    }

    /// `node`'s journal, if the editor still holds the text it left there.
    fn current(&self, node: StableNodeId, held: Option<TextStamp>) -> Option<&TextHistory> {
        self.entries
            .get(&node)
            .filter(|history| history.held == held)
    }

    pub(super) fn can_undo(&self, node: StableNodeId, held: Option<TextStamp>) -> bool {
        self.current(node, held).is_some_and(TextHistory::can_undo)
    }

    pub(super) fn can_redo(&self, node: StableNodeId, held: Option<TextStamp>) -> bool {
        self.current(node, held).is_some_and(TextHistory::can_redo)
    }

    /// Drops `node`'s journal when its text changed outside the journal: a
    /// value the application wrote is not a step the user took, and what it
    /// replaced is not something to undo back into.
    pub(super) fn follow(&mut self, node: StableNodeId, held: Option<TextStamp>) {
        if self.current(node, held).is_none() {
            self.entries.remove(&node);
        }
    }

    /// Notes the text an edit the journal saw left the editor holding.
    pub(super) fn witness(&mut self, node: StableNodeId, held: Option<TextStamp>) {
        if let Some(history) = self.entries.get_mut(&node) {
            history.held = held;
        }
    }

    /// Ends the journaled write of `node` [`crate::AppContext::commit_editor_edit`]
    /// began. By position rather than by popping the top, so a write that
    /// never finished (a caught unwind) cannot leave another node marked.
    fn finish_writing(&mut self, node: StableNodeId) {
        if let Some(index) = self.writing.iter().rposition(|writing| *writing == node) {
            self.writing.remove(index);
        }
    }

    /// Releases the journal of a node that no longer exists.
    pub(super) fn forget(&mut self, node: StableNodeId) {
        self.entries.remove(&node);
    }
}

impl crate::AppContext {
    /// Start an independent undo session when an existing editor is rebound to
    /// another business object, even if the new text is identical.
    pub fn clear_text_history(&mut self, node: StableNodeId) -> Result<(), crate::FrameworkError> {
        if !self.world.contains(node) {
            return Err(crate::FrameworkError::MissingView(node));
        }
        if self.world.text_input(node).is_none() {
            return Err(crate::FrameworkError::InvalidInput);
        }
        self.text_histories.forget(node);
        Ok(())
    }

    /// Replaces `range` of a [`crate::TextArea`]'s text with `text` on the
    /// user's behalf -- a format shortcut, an auto-completion, an application
    /// command -- as an undo step of its own, where writing the component
    /// would clear the journal. The step neither extends the typing before it
    /// nor is extended by the typing after it.
    ///
    /// Refused, like the user's own edits, while the editor is read-only or
    /// disabled or the user is composing with an IME. An atom the range
    /// reaches into is replaced whole; every cursor, the user's own included,
    /// moves through the edit rather than to it. It is not typing into a
    /// snippet: linked placeholders do not mirror it, and an active snippet
    /// session is remapped through it (or ends) as for any value change.
    /// Emits the
    /// editor's change event. Returns whether the text changed; a range
    /// outside the text or off a character boundary is an error.
    pub fn edit_text_area(
        &mut self,
        entity: crate::Entity<crate::TextArea>,
        range: std::ops::Range<usize>,
        text: &str,
    ) -> Result<bool, crate::FrameworkError> {
        self.replace_editable_range(entity, range, text)
    }

    /// [`Self::edit_text_area`] for a [`crate::TextInput`]. An edit that would
    /// take the field past its length limit is refused, as it is from the
    /// keyboard.
    pub fn edit_text_input(
        &mut self,
        entity: crate::Entity<crate::TextInput>,
        range: std::ops::Range<usize>,
        text: &str,
    ) -> Result<bool, crate::FrameworkError> {
        self.replace_editable_range(entity, range, text)
    }

    /// The stamp of the text the world holds for an editor.
    fn editor_text_stamp(&self, node: StableNodeId) -> Option<TextStamp> {
        self.world
            .text_input(node)
            .map(|input| input.session().text().stamp())
    }

    /// Editors a batch writes the text of, whose journals the commit checks
    /// once it lands ([`Self::verify_text_histories`]). An editor with an edit
    /// in flight is left to that edit.
    pub(super) fn text_histories_written_by(
        &self,
        mutations: &crate::MutationQueue,
    ) -> Vec<StableNodeId> {
        // Nothing to look at unless the batch writes editor text and some
        // editor keeps a journal: layout, style, animation and selection
        // batches pass in O(1).
        if !mutations.writes_text() || self.text_histories.entries.is_empty() {
            return Vec::new();
        }
        mutations
            .as_slice()
            .iter()
            .filter_map(|mutation| match mutation {
                // Removing an editor's text (`state: None`) frees its journal
                // too: nothing would use it again to notice.
                crate::UiMutation::SetTextInput { id, .. }
                | crate::UiMutation::ReplaceTextSelection { id, .. }
                    if self.text_histories.entries.contains_key(id)
                        && !self.text_histories.writing.contains(id) =>
                {
                    Some(*id)
                }
                _ => None,
            })
            .collect()
    }

    /// Drops, as soon as the write lands, the journals an outside write left
    /// stale, rather than holding their snapshots until the editor is next
    /// edited -- which a document loaded for reading never is.
    pub(super) fn verify_text_histories(&mut self, written: Vec<StableNodeId>) {
        for node in written {
            self.text_histories
                .follow(node, self.editor_text_stamp(node));
        }
    }

    /// The single place an editor's text state changes.
    ///
    /// `apply` mutates the component; everything that must happen for *every*
    /// edit happens here: the change event, and the undo journal. Callers pass
    /// why the edit happened so consecutive typing or deletion collapses into
    /// one undo step.
    ///
    /// Returns whether the state actually changed.
    ///
    /// Not unwind-safe, like [`Self::update_component`] it runs on (which
    /// takes the view out of the context around user code): a handler that
    /// panics leaves the context inconsistent, journal bookkeeping included.
    pub(super) fn commit_editor_edit<C: super::EditableText>(
        &mut self,
        entity: crate::Entity<C>,
        origin: TextEditOrigin,
        apply: impl FnOnce(&mut C, &mut crate::ViewContext<'_, C>) -> bool,
    ) -> Result<bool, crate::FrameworkError> {
        // The text the edit itself produced, before observers of its change
        // event run: a handler given the editor may rewrite it.
        let mut edited = None;
        let update = |cx: &mut Self, edited: &mut Option<crate::TextValue>| {
            cx.update_component(entity, |editable: &mut C, view| {
                if !apply(editable, view) {
                    return false;
                }
                if C::JOURNALED {
                    *edited = Some(editable.state().value.clone());
                }
                view.emit(editable.change());
                true
            })
        };
        if !C::JOURNALED {
            return update(self, &mut edited);
        }
        let node = entity.stable_id();
        // Undo and redo followed already, before looking at the journal.
        if origin != TextEditOrigin::History {
            self.follow_text_history(node);
        }
        let before = self.read(entity, |editable: &C| editable.state().clone())?;
        self.text_histories.writing.push(node);
        let written = update(self, &mut edited);
        self.text_histories.finish_writing(node);
        if matches!(written, Ok(false)) {
            return written;
        }
        let after = self.read(entity, |editable: &C| editable.state().clone())?;
        // Someone else wrote the text during the edit: the world holds text
        // the journal did not produce, so it goes rather than adopt it.
        // Likewise an observer that rewrote the text in the change event's
        // own delivery (a clear after send, a formatter): it replaced the
        // value the user produced, an application write. A commit that
        // failed and rolled the component back (text as before) is neither.
        let rewritten =
            after.value != before.value && edited.is_some_and(|edited| edited != after.value);
        // The world takes the component's own buffer as the edit lands, so
        // text it still holds from this edit compares by identity; a
        // mismatch means another batch wrote it meanwhile.
        let foreign = written.is_ok()
            && self
                .world
                .text_input(node)
                .is_some_and(|input| input.value_shared() != after.value);
        if foreign || rewritten {
            self.text_histories.forget(node);
            return written;
        }
        // A failed write still counts if the component took the new text:
        // the world may hold it already, and a journal that never saw it
        // would take it for an outside write and drop every step. A write
        // the component rolled back is left unwitnessed, so a world that
        // moved anyway reads as the outside write it now is.
        if written.is_ok() || after.value != before.value {
            if after.value != before.value {
                self.text_histories.record(node, before, after, origin);
            }
            self.text_histories
                .witness(node, self.editor_text_stamp(node));
        }
        written
    }

    /// Drops `node`'s journal if its text changed since the journal saw it.
    fn follow_text_history(&mut self, node: StableNodeId) {
        self.text_histories
            .follow(node, self.editor_text_stamp(node));
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
        // `None` during a composition too: undo during preedit would fight
        // the IME. The two kinds it knows are the journaled editors.
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        match focused.kind {
            super::text_edit::TextEditorKind::Area => self.step_text_history(
                crate::Entity::<crate::TextArea>::from_stable_id(focused.node),
                undo,
            ),
            super::text_edit::TextEditorKind::Field => self.step_text_history(
                crate::Entity::<crate::TextInput>::from_stable_id(focused.node),
                undo,
            ),
        }
    }

    fn step_text_history<C: super::EditableText>(
        &mut self,
        entity: crate::Entity<C>,
        undo: bool,
    ) -> Result<bool, crate::FrameworkError> {
        if !self.read(entity, super::EditableText::accepts_input)? {
            return Ok(false);
        }
        let node = entity.stable_id();
        // Before reading the journal: one an outside write left stale has
        // nothing to undo.
        self.follow_text_history(node);
        // Looked at, not taken: the cursor moves only once the restore has
        // landed, so an undo that fails leaves the journal as it was.
        let Some(target) = self.text_histories.peek(node, undo) else {
            return Ok(false);
        };
        let target_value = target.value.clone();
        // `History` keeps the restore from becoming a step of its own.
        let restored =
            self.commit_editor_edit(entity, TextEditOrigin::History, move |editable, _| {
                *editable.state_mut() = target;
                true
            });
        // A restore that failed may still have landed (the component took
        // the text before a follow-up failed): then the cursor moves too.
        let landed = match &restored {
            Ok(restored) => *restored,
            Err(_) => self
                .read(entity, |editable: &C| {
                    editable.state().value == target_value
                })
                .unwrap_or(false),
        };
        if landed {
            self.text_histories.step(node, undo);
        }
        restored
    }

    /// Ends the current typing or deletion run for an editor, so the next edit
    /// starts a new undo step. Caret moves and focus changes call this.
    pub(super) fn seal_editor_history(&mut self, node: StableNodeId) {
        self.text_histories.seal(node);
    }

    pub(super) fn seal_blurred_editor_history(
        &mut self,
        document: crate::DocumentId,
        previous: Option<StableNodeId>,
    ) {
        if previous != self.world.focused(document)
            && let Some(previous) = previous
        {
            self.seal_editor_history(previous);
        }
    }

    /// Whether the editor has an edit to undo.
    pub fn can_undo_text(&self, node: StableNodeId) -> bool {
        self.text_histories
            .can_undo(node, self.editor_text_stamp(node))
    }

    /// Whether the editor has an undone edit to redo.
    pub fn can_redo_text(&self, node: StableNodeId) -> bool {
        self.text_histories
            .can_redo(node, self.editor_text_stamp(node))
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
        cx.read(area, |area| area.state.value.to_string()).unwrap()
    }

    #[test]
    fn a_cut_is_its_own_undo_step_that_typing_does_not_extend() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "abc").unwrap();
        cx.select_all_focused_text(document()).unwrap();
        assert_eq!(
            cx.cut_focused_text(document()).unwrap().as_deref(),
            Some("abc")
        );
        cx.replace_focused_text(document(), "x").unwrap();
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "", "undo takes back the typing alone");
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "abc", "and then the cut");
    }

    #[test]
    fn a_snippet_insertion_is_one_undo_step() {
        let mut cx = AppContext::new();
        let area = cx
            .create_component(document(), TextArea::new("tail").code_editor(true))
            .unwrap();
        cx.focus_node(document(), area.stable_id()).unwrap();
        cx.select_focused_text_range(document(), 0, 0).unwrap();
        let snippet = crate::TextSnippet::new("let", "let $1 = $2;$0");
        assert!(
            cx.insert_focused_text_snippet(document(), &snippet)
                .unwrap()
        );
        assert_eq!(value_of(&cx, area), "let  = ;tail");
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "tail");
        assert!(cx.redo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "let  = ;tail");
    }

    #[test]
    fn a_number_inputs_ime_commit_stays_after_the_next_keystroke() {
        let mut cx = AppContext::new();
        let input = cx
            .create_component(document(), crate::NumberInput::new(1.0))
            .unwrap();
        let node = input.stable_id();
        cx.focus_node(document(), node).unwrap();
        cx.select_all_focused_text(document()).unwrap();
        cx.set_ime_preedit(document(), "２".into(), None).unwrap();
        assert!(cx.commit_ime(document(), "2").unwrap());
        cx.replace_focused_text(document(), "5").unwrap();
        let component = cx
            .read(input, |input| input.state.value.to_string())
            .unwrap();
        assert_eq!(component, "25", "the commit reached the component");
        assert_eq!(cx.world().text_input(node).unwrap().value, "25");
    }

    #[test]
    fn left_and_right_collapse_a_selection_onto_its_edge() {
        // Right lands on the anchor, the selection's end: on the side of the
        // text it selected, as the collapse probed it.
        for (intent, landing) in [
            (crate::TextCaretIntent::Left, crate::TextSelection::caret(1)),
            (
                crate::TextCaretIntent::Right,
                crate::TextSelection::caret(4).with_affinity(crate::TextAffinity::Upstream),
            ),
        ] {
            let mut cx = AppContext::new();
            let input = cx
                .create_component(document(), TextInput::new("abcdef"))
                .unwrap();
            cx.focus_node(document(), input.stable_id()).unwrap();
            // Focus at the far end from where Left lands, and the near end
            // for Right: a step from the focus would land elsewhere.
            cx.select_focused_text_range(document(), 4, 1).unwrap();
            assert!(
                cx.move_focused_text_caret(document(), intent, false, None)
                    .unwrap()
            );
            assert_eq!(
                cx.world().text_input(input.stable_id()).unwrap().selection,
                landing,
                "{intent:?}"
            );
        }
    }

    #[test]
    fn focus_changes_separate_typing_runs_but_refocusing_the_same_editor_does_not() {
        for clear in [false, true] {
            let mut cx = AppContext::new();
            let area = focused_area(&mut cx, "");
            cx.replace_focused_text(document(), "first").unwrap();
            assert!(!cx.focus_node(document(), area.stable_id()).unwrap());
            cx.replace_focused_text(document(), " run").unwrap();
            if clear {
                cx.clear_focus(document()).unwrap();
            } else {
                let other = cx.create_component(document(), TextInput::new("")).unwrap();
                cx.focus_node(document(), other.stable_id()).unwrap();
            }
            cx.focus_node(document(), area.stable_id()).unwrap();
            cx.replace_focused_text(document(), " second").unwrap();
            assert!(cx.undo_focused_text(document()).unwrap());
            assert_eq!(value_of(&cx, area), "first run");
            assert!(cx.undo_focused_text(document()).unwrap());
            assert_eq!(value_of(&cx, area), "");
            assert!(cx.redo_focused_text(document()).unwrap());
            assert_eq!(value_of(&cx, area), "first run");
        }
    }

    #[test]
    fn select_all_and_accessibility_selection_separate_replacement_from_typing() {
        for accessibility in [false, true] {
            let mut cx = AppContext::new();
            let area = focused_area(&mut cx, "");
            cx.replace_focused_text(document(), "first").unwrap();
            if accessibility {
                assert!(
                    cx.apply_accessibility_action(
                        document(),
                        crate::AccessibilityActionRequest {
                            target: area.stable_id(),
                            action: crate::AccessibilityAction::SetSelection(
                                crate::TextSelection::new(0, 5)
                            ),
                        }
                    )
                    .unwrap()
                );
            } else {
                cx.select_all_focused_text(document()).unwrap();
            }
            cx.replace_focused_text(document(), "second").unwrap();
            assert!(cx.undo_focused_text(document()).unwrap());
            assert_eq!(value_of(&cx, area), "first");
            assert!(cx.redo_focused_text(document()).unwrap());
            assert_eq!(value_of(&cx, area), "second");
        }
    }

    #[test]
    fn rejected_focus_and_selection_leave_the_typing_run_intact() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "first").unwrap();
        let structure = cx
            .create_component(document(), crate::Stack::column(0.0))
            .unwrap();
        assert!(!cx.focus_node(document(), structure.stable_id()).unwrap());
        assert!(
            !cx.apply_accessibility_action(
                document(),
                crate::AccessibilityActionRequest {
                    target: area.stable_id(),
                    action: crate::AccessibilityAction::SetSelection(crate::TextSelection::new(
                        0, 999
                    )),
                }
            )
            .unwrap()
        );
        assert!(
            !cx.move_focused_text_caret(document(), crate::TextCaretIntent::Right, false, None)
                .unwrap()
        );
        cx.replace_focused_text(document(), " second").unwrap();
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "");
    }

    #[test]
    fn pointer_and_public_range_selection_split_history_in_the_same_editor() {
        for pointer in [false, true] {
            let mut cx = AppContext::new();
            let area = focused_area(&mut cx, "");
            cx.replace_focused_text(document(), "first").unwrap();
            if pointer {
                let mut queue = crate::MutationQueue::new();
                queue.write_layout(
                    area.stable_id(),
                    crate::LayoutBox {
                        x: 0.0,
                        y: 0.0,
                        width: 200.0,
                        height: 80.0,
                    },
                );
                cx.commit_mutations(queue).unwrap();
                cx.resolve_styles(&[area.stable_id()]).unwrap();
                cx.shape_text(&[area.stable_id()], &mut crate::MeasureTextShaper)
                    .unwrap();
                let (content, _) = cx
                    .world()
                    .text_input_pointer_context(area.stable_id())
                    .unwrap();
                assert!(
                    cx.text_editor_pointer_press(
                        document(),
                        area.stable_id(),
                        1,
                        content.x + 1.0,
                        content.y + 1.0,
                        false,
                        false,
                        std::time::Duration::ZERO,
                        &mut crate::MeasureTextShaper
                    )
                    .unwrap()
                );
                cx.text_editor_pointer_release(1);
            } else {
                assert!(cx.select_focused_text_range(document(), 0, 0).unwrap());
            }
            assert_eq!(
                cx.read(area, |area| area.state.selection).unwrap(),
                crate::TextSelection::caret(0)
            );
            cx.replace_focused_text(document(), "second ").unwrap();
            assert!(cx.undo_focused_text(document()).unwrap());
            assert_eq!(value_of(&cx, area), "first");
            assert!(cx.redo_focused_text(document()).unwrap());
            assert_eq!(value_of(&cx, area), "second first");
        }
    }

    #[test]
    fn modal_focus_round_trip_seals_history_but_rejected_focus_batch_does_not() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        let host = cx
            .create_component(document(), crate::OverlayHost::new())
            .unwrap();
        let dialog = cx
            .create_component(document(), crate::Dialog::new("Dialog"))
            .unwrap();
        cx.append_child(host, dialog).unwrap();
        cx.replace_focused_text(document(), "first").unwrap();
        let mut rejected = crate::MutationQueue::new();
        rejected.request_focus(document(), Some(host.stable_id()));
        assert!(cx.commit_mutations(rejected).is_err());
        assert_eq!(cx.world().focused(document()), Some(area.stable_id()));
        cx.replace_focused_text(document(), " run").unwrap();
        cx.activate_overlay(host, dialog).unwrap();
        assert_eq!(cx.world().focused(document()), Some(dialog.stable_id()));
        cx.dismiss_overlay(host).unwrap();
        assert_eq!(cx.world().focused(document()), Some(area.stable_id()));
        cx.replace_focused_text(document(), " second").unwrap();
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "first run");
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "");
    }

    #[test]
    fn style_resolution_clearing_focus_seals_history() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "first").unwrap();
        let initial = cx.read(area, |area| area.style.clone()).unwrap();
        let mut hidden = initial.clone();
        std::sync::Arc::make_mut(&mut hidden.layout).display =
            Some(nana_ui_core::DisplaySpec::None);
        cx.update_component(area, |area, _| area.style = hidden)
            .unwrap();
        cx.resolve_styles(&[area.stable_id()]).unwrap();
        assert_eq!(cx.world().focused(document()), None);
        cx.update_component(area, |area, _| area.style = initial)
            .unwrap();
        cx.resolve_styles(&[area.stable_id()]).unwrap();
        cx.focus_node(document(), area.stable_id()).unwrap();
        cx.replace_focused_text(document(), " second").unwrap();
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "first");
    }

    #[test]
    fn identity_rebind_clears_undo_and_redo_without_changing_editor_state() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "first").unwrap();
        assert!(cx.can_undo_text(area.stable_id()));
        cx.clear_text_history(area.stable_id()).unwrap();
        assert_eq!(value_of(&cx, area), "first");
        assert!(!cx.undo_focused_text(document()).unwrap());
        cx.replace_focused_text(document(), "second").unwrap();
        assert!(cx.undo_focused_text(document()).unwrap());
        assert!(cx.can_redo_text(area.stable_id()));
        let state = cx.read(area, |area| area.state.clone()).unwrap();
        cx.set_ime_preedit(document(), "ni".to_owned(), None)
            .unwrap();
        cx.clear_text_history(area.stable_id()).unwrap();
        assert!(!cx.can_redo_text(area.stable_id()));
        assert_eq!(cx.read(area, |area| area.state.clone()).unwrap(), state);
        assert!(cx.world().ime(area.stable_id()).is_some());
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
    fn an_application_writing_a_new_value_into_the_component_clears_the_journal() {
        for write in ["set_component", "update_component", "mount"] {
            let mut cx = AppContext::new();
            let card = cx.create_component(document(), crate::Card::new()).unwrap();
            let mount = |cx: &mut AppContext, value: &str| {
                let mut area = None;
                cx.mount(card, |ui| {
                    area = Some(ui.child("editor", TextArea::new(value))?);
                    Ok(())
                })
                .unwrap();
                area.unwrap()
            };
            let area = mount(&mut cx, "");
            cx.focus_node(document(), area.stable_id()).unwrap();
            cx.replace_focused_text(document(), "draft").unwrap();
            cx.move_focused_text_caret(document(), crate::TextCaretIntent::Left, false, None)
                .unwrap();
            cx.replace_focused_text(document(), "!").unwrap();
            assert!(cx.undo_focused_text(document()).unwrap());
            assert!(cx.can_undo_text(area.stable_id()));
            assert!(cx.can_redo_text(area.stable_id()));

            // Loading another document by rebuilding the component.
            match write {
                "set_component" => cx
                    .set_component(area, TextArea::new("loaded from disk"))
                    .unwrap(),
                "update_component" => cx
                    .update_component(area, |area, _| {
                        area.state = crate::TextInputState::new("loaded from disk");
                    })
                    .unwrap(),
                _ => assert_eq!(mount(&mut cx, "loaded from disk"), area),
            }
            assert!(!cx.can_undo_text(area.stable_id()), "{write}");
            assert!(!cx.can_redo_text(area.stable_id()), "{write}");
            assert!(!cx.undo_focused_text(document()).unwrap(), "{write}");
            assert!(!cx.redo_focused_text(document()).unwrap(), "{write}");
            assert_eq!(value_of(&cx, area), "loaded from disk", "{write}");

            // The new document's own edits undo as usual.
            cx.replace_focused_text(document(), "x").unwrap();
            assert!(cx.undo_focused_text(document()).unwrap(), "{write}");
            assert_eq!(value_of(&cx, area), "loaded from disk", "{write}");
            assert!(!cx.can_undo_text(area.stable_id()), "{write}");
        }
    }

    #[test]
    fn a_composition_after_typing_is_not_mistaken_for_a_replacement() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "a").unwrap();
        cx.set_ime_preedit(document(), "ni".to_owned(), None)
            .unwrap();
        assert!(cx.can_undo_text(area.stable_id()), "preedit is not a write");
        cx.commit_ime(document(), "你").unwrap();
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "a");
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "");
    }

    #[test]
    fn a_journal_an_application_write_replaced_is_freed_at_once() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "draft").unwrap();
        assert!(cx.text_histories.entries.contains_key(&area.stable_id()));
        cx.set_component(area, TextArea::new("loaded")).unwrap();
        assert!(
            !cx.text_histories.entries.contains_key(&area.stable_id()),
            "the old document's snapshots are not kept until the next edit"
        );
    }

    #[test]
    fn a_number_field_offers_no_undo_it_could_not_take() {
        let mut cx = AppContext::new();
        let input = cx
            .create_component(document(), crate::NumberInput::new(1.0))
            .unwrap();
        let node = input.stable_id();
        cx.focus_node(document(), node).unwrap();
        cx.select_all_focused_text(document()).unwrap();
        cx.replace_focused_text(document(), "12").unwrap();
        assert!(cx.step_focused_number_input(document(), 1).unwrap());
        assert!(!cx.can_undo_text(node));
        assert!(!cx.undo_focused_text(document()).unwrap());
        assert!(!cx.text_histories.entries.contains_key(&node));
    }

    #[test]
    fn a_selection_replaced_through_the_world_frees_the_journal_at_commit() {
        let mut cx = AppContext::new();
        let field = cx.create_component(document(), TextInput::new("")).unwrap();
        let node = field.stable_id();
        cx.focus_node(document(), node).unwrap();
        cx.replace_focused_text(document(), "draft").unwrap();
        assert!(cx.text_histories.entries.contains_key(&node));
        let mut queue = crate::MutationQueue::new();
        queue.replace_text_selection(node, "X");
        cx.commit_mutations(queue).unwrap();
        assert!(!cx.text_histories.entries.contains_key(&node));
    }

    #[test]
    fn a_change_handler_rewriting_its_editor_is_an_application_write() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "hello").unwrap();
        // Clear after send: the handler is given the editor itself.
        cx.on(area, |area, event: &crate::TextChanged, _| {
            if event.value.ends_with('\n') {
                area.state = crate::TextInputState::new("");
            }
        })
        .unwrap();
        cx.insert_focused_text_newline(document()).unwrap();
        assert_eq!(value_of(&cx, area), "");
        assert!(
            !cx.can_undo_text(area.stable_id()),
            "the sent draft is not undone back into"
        );
        assert!(!cx.undo_focused_text(document()).unwrap());
    }

    #[test]
    fn removing_an_editors_text_frees_its_journal_at_commit() {
        let mut cx = AppContext::new();
        let field = cx.create_component(document(), TextInput::new("")).unwrap();
        let node = field.stable_id();
        cx.focus_node(document(), node).unwrap();
        cx.replace_focused_text(document(), "draft").unwrap();
        let mut queue = crate::MutationQueue::new();
        queue.set_text_input(node, None);
        cx.commit_mutations(queue).unwrap();
        assert!(!cx.text_histories.entries.contains_key(&node));
    }

    #[test]
    fn an_edit_whose_commit_fails_leaves_the_journal_as_it_was() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "draft").unwrap();
        // The world refuses a selection past the text: the commit fails and
        // the component is rolled back, text as it was.
        let failed = cx.commit_editor_edit(area, crate::TextEditOrigin::Structural, |area, _| {
            area.state.value = "other".into();
            area.state.selection = crate::TextSelection::caret(999);
            true
        });
        assert!(failed.is_err());
        assert_eq!(value_of(&cx, area), "draft");
        assert!(
            cx.can_undo_text(area.stable_id()),
            "the typing is still undoable"
        );
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "");
    }

    #[test]
    fn the_world_holds_the_editors_own_buffer_once_an_edit_lands() {
        // What tells an edit's own text from another batch's write during it:
        // the world adopts the component's buffer, so the two compare by
        // pointer, and another write would not.
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        for text in ["a", "b"] {
            cx.replace_focused_text(document(), text).unwrap();
            let component = cx.read(area, |area| area.state.value.clone()).unwrap();
            let world = cx
                .world()
                .text_input(area.stable_id())
                .unwrap()
                .value_shared();
            assert!(world.same_identity(&component), "after typing {text:?}");
        }
        assert!(cx.undo_focused_text(document()).unwrap());
        let component = cx.read(area, |area| area.state.value.clone()).unwrap();
        let world = cx
            .world()
            .text_input(area.stable_id())
            .unwrap()
            .value_shared();
        assert!(world.same_identity(&component), "after undo");
    }

    #[test]
    fn an_edit_made_on_the_users_behalf_is_an_undo_step_of_its_own() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "fo").unwrap();
        // A completion: not merged into the prefix the user typed, and the
        // typing after it is not merged into it.
        assert!(cx.edit_text_area(area, 0..2, "foo()").unwrap());
        cx.replace_focused_text(document(), ";").unwrap();
        assert_eq!(value_of(&cx, area), "foo();");
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "foo()");
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "fo", "the completion came off alone");
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "");
    }

    #[test]
    fn an_edit_on_the_users_behalf_is_refused_where_the_users_would_be() {
        let changes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "ab").unwrap();
        let counted = std::sync::Arc::clone(&changes);
        cx.on(area, move |_area, _: &crate::TextChanged, _cx| {
            counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })
        .unwrap();
        let count = || changes.load(std::sync::atomic::Ordering::Relaxed);
        // A range outside the text is the caller's error.
        assert!(matches!(
            cx.edit_text_area(area, 1..9, "x"),
            Err(crate::FrameworkError::InvalidInput)
        ));
        // Replacing text with itself is no edit, and leaves the typing run
        // whole: "c" still joins "ab".
        assert!(!cx.edit_text_area(area, 0..1, "a").unwrap());
        assert_eq!(count(), 0);
        cx.replace_focused_text(document(), "c").unwrap();
        // While the user composes, the editor is theirs.
        cx.set_ime_preedit(document(), "ni".to_owned(), None)
            .unwrap();
        assert!(!cx.edit_text_area(area, 0..0, "x").unwrap());
        assert!(
            cx.world().ime(area.stable_id()).is_some(),
            "composition intact"
        );
        cx.commit_ime(document(), "").unwrap();
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "", "one run, one step");
        // Read-only: refused, and nothing recorded.
        cx.update_component(area, |area, _| area.read_only = true)
            .unwrap();
        let before = count();
        assert!(!cx.edit_text_area(area, 0..0, "x").unwrap());
        assert_eq!(value_of(&cx, area), "");
        assert_eq!(count(), before);
        // A field's length limit holds as it does for typing.
        let field = cx
            .create_component(document(), TextInput::new("abc").max_length(3))
            .unwrap();
        assert!(!cx.edit_text_input(field, 3..3, "d").unwrap());
        assert_eq!(
            cx.read(field, |field| field.state.value.to_string())
                .unwrap(),
            "abc"
        );
    }

    #[test]
    fn an_edit_on_the_users_behalf_leaves_the_users_caret_where_they_work() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "hello world");
        cx.select_focused_text_range(document(), 6, 11).unwrap();
        // A format command at the top of the document.
        assert!(cx.edit_text_area(area, 0..5, "HELLO!").unwrap());
        assert_eq!(value_of(&cx, area), "HELLO! world");
        assert_eq!(
            cx.read(area, |area| area.state.selection).unwrap(),
            crate::TextSelection::new(7, 12),
            "the user's selection moved with the text, not to the edit"
        );
    }

    #[test]
    fn an_edit_on_a_limited_field_is_refused_whole_and_touches_only_its_range() {
        let text = |cx: &AppContext, field: crate::Entity<TextInput>| {
            cx.read(field, |field| field.state.value.to_string())
                .unwrap()
        };
        let mut cx = AppContext::new();
        let field = cx
            .create_component(document(), TextInput::new("ab").max_length(4))
            .unwrap();
        // Half a completion is not the edit asked for.
        assert!(!cx.edit_text_input(field, 2..2, "foo()").unwrap());
        assert_eq!(text(&cx, field), "ab");

        let field = cx
            .create_component(document(), TextInput::new("abcdefgh").max_length(10))
            .unwrap();
        cx.update_component(field, |field, _| {
            field.state.selection = crate::TextSelection::caret(8);
            field.state.additional_selections = vec![crate::TextSelection::new(3, 6)];
        })
        .unwrap();
        // A further cursor overlapping the range is not fused into it: only
        // 0..4 is replaced.
        assert!(cx.edit_text_input(field, 0..4, "X").unwrap());
        assert_eq!(text(&cx, field), "Xefgh");
    }

    #[test]
    fn a_completion_inserted_at_the_caret_leaves_the_caret_after_it() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "foo").unwrap();
        assert!(cx.edit_text_area(area, 3..3, "()").unwrap());
        cx.replace_focused_text(document(), ";").unwrap();
        assert_eq!(value_of(&cx, area), "foo();", "typing goes on after it");

        // Replaced text with the caret at its start: the caret stays in front.
        cx.select_focused_text_range(document(), 0, 0).unwrap();
        assert!(cx.edit_text_area(area, 0..3, "bar").unwrap());
        assert_eq!(
            cx.read(area, |area| area.state.selection).unwrap(),
            crate::TextSelection::caret(0)
        );
    }

    #[test]
    fn an_edit_on_the_users_behalf_leaves_the_editor_and_world_on_one_caret() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "e");
        cx.select_focused_text_range(document(), 1, 1).unwrap();
        // A combining mark after the caret's base: the caret, now inside the
        // cluster, snaps the same way in the component and in the world.
        assert!(cx.edit_text_area(area, 1..1, "\u{301}").unwrap());
        let component = cx.read(area, |area| area.state.selection).unwrap();
        let world = cx.world().text_input(area.stable_id()).unwrap().selection;
        assert_eq!(component, world);
        assert!(
            cx.world()
                .text_input(area.stable_id())
                .unwrap()
                .value
                .is_char_boundary(component.focus)
        );
    }

    #[test]
    fn an_edit_on_the_users_behalf_takes_an_atom_whole() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "Hi [bob]!");
        cx.update_component(area, |area, _| {
            area.atom_spans = std::sync::Arc::from([crate::TextAtomSpan::new(3, 8)]);
        })
        .unwrap();
        // 5..9 reaches into the chip: it goes whole.
        assert!(cx.edit_text_area(area, 5..9, "x").unwrap());
        assert_eq!(value_of(&cx, area), "Hi x");

        // At a chip's edge is beside it, not in it, even where the text
        // inserted repeats the chip's own ("[" before "[bob]").
        let area = focused_area(&mut cx, "x[bob]");
        cx.update_component(area, |area, _| {
            area.atom_spans = std::sync::Arc::from([crate::TextAtomSpan::new(1, 6)]);
        })
        .unwrap();
        assert!(cx.edit_text_area(area, 1..1, "[").unwrap());
        assert_eq!(value_of(&cx, area), "x[[bob]");
    }

    #[test]
    fn an_application_writing_back_the_editors_own_value_keeps_the_journal() {
        let mut cx = AppContext::new();
        let area = focused_area(&mut cx, "");
        cx.replace_focused_text(document(), "draft").unwrap();
        // The component rebuilt from what the editor reported: its own buffer,
        // then the same text in a fresh one.
        let reported = cx.read(area, |area| area.state.value.clone()).unwrap();
        cx.update_component(area, |area, _| {
            area.state.value = reported;
            area.placeholder = "Notes".into();
        })
        .unwrap();
        cx.set_component(area, TextArea::new("draft")).unwrap();
        assert!(cx.undo_focused_text(document()).unwrap());
        assert_eq!(value_of(&cx, area), "");
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

    /// What the editor's undo does: look at the step, then move once the
    /// restore landed (here it always does).
    fn undo(history: &mut TextHistory) -> Option<TextInputState> {
        let state = history.peek(true).cloned()?;
        history.step(true);
        Some(state)
    }

    fn redo(history: &mut TextHistory) -> Option<TextInputState> {
        let state = history.peek(false).cloned()?;
        history.step(false);
        Some(state)
    }

    /// The step count alone bounds the journal only for small documents: a
    /// step holds the value before AND after, so a deep journal of a large
    /// document would be hundreds of megabytes. Past the byte budget the
    /// oldest steps go -- and the newest edit stays undoable, however large.
    #[test]
    fn a_large_document_journal_stays_inside_its_byte_budget() {
        let big = "x".repeat(512 * 1024);
        let mut history = TextHistory::default();
        for step in 0..40 {
            // Each step is its own (Structural never merges), and each end is
            // half a megabyte.
            record(
                &mut history,
                &format!("{big}{step}"),
                &format!("{big}{step}!"),
                TextEditOrigin::Structural,
            );
            assert!(
                history.bytes() <= TextHistory::CAPACITY_BYTES + 2 * big.len(),
                "step {step}: {} bytes",
                history.bytes()
            );
        }
        assert!(history.steps.len() < 40, "old steps went");
        assert!(history.can_undo(), "the newest edit is still undoable");
        assert_eq!(
            undo(&mut history).map(|state| state.value.len()),
            Some(big.len() + 2),
            "and it undoes to the value that edit started from"
        );

        // One step bigger than the whole budget is still kept: an editor with
        // no undo at all would be worse than one over budget.
        let huge = "y".repeat(TextHistory::CAPACITY_BYTES * 2);
        let mut history = TextHistory::default();
        record(
            &mut history,
            &huge,
            &format!("{huge}!"),
            TextEditOrigin::Paste,
        );
        assert_eq!(history.steps.len(), 1);
        assert!(history.can_undo());
    }

    #[test]
    fn a_run_of_typing_is_one_undo_step() {
        let mut history = TextHistory::default();
        record(&mut history, "", "a", TextEditOrigin::Typing);
        record(&mut history, "a", "ab", TextEditOrigin::Typing);
        record(&mut history, "ab", "abc", TextEditOrigin::Typing);

        assert_eq!(
            undo(&mut history).map(|state| state.value.to_string()),
            Some(String::new())
        );
        assert!(!history.can_undo(), "the run collapsed into one step");
        assert_eq!(
            redo(&mut history).map(|state| state.value.to_string()),
            Some("abc".to_owned())
        );
    }

    #[test]
    fn a_different_origin_starts_a_new_step() {
        let mut history = TextHistory::default();
        record(&mut history, "", "abc", TextEditOrigin::Typing);
        record(&mut history, "abc", "ab", TextEditOrigin::Delete);
        record(&mut history, "ab", "ab!", TextEditOrigin::Paste);

        assert_eq!(
            undo(&mut history).map(|s| s.value.to_string()),
            Some("ab".to_owned())
        );
        assert_eq!(
            undo(&mut history).map(|s| s.value.to_string()),
            Some("abc".to_owned())
        );
        assert_eq!(
            undo(&mut history).map(|s| s.value.to_string()),
            Some(String::new())
        );
        assert!(!history.can_undo());
    }

    #[test]
    fn sealing_breaks_a_typing_run() {
        let mut history = TextHistory::default();
        record(&mut history, "", "ab", TextEditOrigin::Typing);
        history.seal();
        record(&mut history, "ab", "abcd", TextEditOrigin::Typing);

        assert_eq!(
            undo(&mut history).map(|s| s.value.to_string()),
            Some("ab".to_owned())
        );
        assert_eq!(
            undo(&mut history).map(|s| s.value.to_string()),
            Some(String::new())
        );
    }

    #[test]
    fn typing_after_an_undo_is_a_step_of_its_own() {
        let mut history = TextHistory::default();
        record(&mut history, "", "abc", TextEditOrigin::Typing);
        record(&mut history, "abc", "ab", TextEditOrigin::Delete);
        undo(&mut history);
        record(&mut history, "abc", "abcd", TextEditOrigin::Typing);
        assert_eq!(
            undo(&mut history).map(|s| s.value.to_string()),
            Some("abc".to_owned()),
            "not back through the typing before the undo"
        );

        // After a redo, likewise.
        let mut history = TextHistory::default();
        record(&mut history, "", "ab", TextEditOrigin::Typing);
        undo(&mut history);
        redo(&mut history);
        record(&mut history, "ab", "abc", TextEditOrigin::Typing);
        assert_eq!(
            undo(&mut history).map(|s| s.value.to_string()),
            Some("ab".to_owned())
        );
    }

    #[test]
    fn editing_after_undo_drops_the_redo_tail() {
        let mut history = TextHistory::default();
        record(&mut history, "", "one", TextEditOrigin::Paste);
        record(&mut history, "one", "two", TextEditOrigin::Paste);
        undo(&mut history);
        assert!(history.can_redo());

        record(&mut history, "one", "three", TextEditOrigin::Paste);
        assert!(!history.can_redo(), "the abandoned branch is gone");
        assert_eq!(
            undo(&mut history).map(|s| s.value.to_string()),
            Some("one".to_owned())
        );
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
