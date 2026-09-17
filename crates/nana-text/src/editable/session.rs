//! Editing commands over [`EditableText`] + [`EditState`]: typing, deleting,
//! caret motion, selection and IME composition.

use super::geometry::{CaretRect, EditorGeometry};
use super::ime;
use super::navigation;
use super::state::{Composition, EditRevisions, EditSelection};
use super::text::{EditableText, TextEdit};
use crate::counters::TextWorkCounters;
use crate::edit::Affinity;
use std::borrow::Cow;
use std::ops::Range;

/// A caret motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    /// One grapheme cluster back in logical order.
    GraphemeBackward,
    /// One grapheme cluster forward in logical order.
    GraphemeForward,
    /// One caret position left on screen, through mixed-direction text in
    /// visual order. Needs geometry; without it this is
    /// [`Self::GraphemeBackward`].
    Left,
    /// One caret position right on screen. See [`Self::Left`].
    Right,
    WordBackward,
    WordForward,
    /// Start of the visual line; of the logical line without geometry.
    LineStart,
    /// End of the visual line; of the logical line without geometry.
    LineEnd,
    /// One visual line up, keeping the goal column. Without geometry, one
    /// logical line up keeping the grapheme column.
    LineUp,
    LineDown,
    ParagraphStart,
    ParagraphEnd,
    DocumentStart,
    DocumentEnd,
}

impl Motion {
    fn is_vertical(self) -> bool {
        matches!(self, Self::LineUp | Self::LineDown)
    }
}

/// What a command changed. Each kind invalidates something different: only
/// [`Self::Text`] and [`Self::Composition`] change what is laid out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditChange {
    /// Nothing: the command did not apply, or changed nothing.
    None,
    /// The selection or caret moved.
    Selection,
    /// The preedit changed; committed text did not.
    Composition,
    /// Committed text changed.
    Text(TextEdit),
}

impl EditChange {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// Committed text around the selection, as a platform IME asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurroundingText<'a> {
    pub text: &'a str,
    /// Byte offset of `text` in the committed text.
    pub offset: usize,
    /// The selection, relative to `text`.
    pub selection: (usize, usize),
}

/// Editor state that is not text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EditState {
    pub selection: EditSelection,
    pub composition: Option<Composition>,
    /// The x a run of vertical moves keeps returning to, in geometry space.
    pub goal_x_px: Option<f32>,
}

/// One editable text and its state.
///
/// The only way to change either: every command bumps exactly the revision of
/// what it changed and records itself in the editable counters, which is what
/// lets a consumer tell a caret move from an edit without comparing strings.
///
/// While a composition is active the IME owns the text around the caret:
/// ordinary edits and caret motion are refused until it commits or cancels,
/// and [`Self::blur`] cancels it.
#[derive(Debug)]
pub struct EditSession {
    /// Process-unique; see [`EditRevisions::session`].
    id: u64,
    text: EditableText,
    state: EditState,
    selection_revision: u64,
    composition_revision: u64,
    work: TextWorkCounters,
}

impl EditSession {
    /// A session with the caret at the end of `text`.
    pub fn new(text: impl Into<String>) -> Self {
        let text = EditableText::new(text);
        let end = text.len();
        Self {
            id: next_session_id(),
            text,
            state: EditState {
                selection: EditSelection::caret(end),
                ..EditState::default()
            },
            selection_revision: 0,
            composition_revision: 0,
            work: TextWorkCounters::default(),
        }
    }

    pub fn text(&self) -> &EditableText {
        &self.text
    }

    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }

    pub fn state(&self) -> &EditState {
        &self.state
    }

    pub fn selection(&self) -> EditSelection {
        self.state.selection
    }

    pub fn composition(&self) -> Option<&Composition> {
        self.state.composition.as_ref()
    }

    pub fn is_composing(&self) -> bool {
        self.state.composition.is_some()
    }

    pub fn revisions(&self) -> EditRevisions {
        EditRevisions {
            session: self.id,
            text: self.text.revision(),
            selection: self.selection_revision,
            composition: self.composition_revision,
        }
    }

    /// Editable work since the last call.
    pub fn take_work(&mut self) -> TextWorkCounters {
        std::mem::take(&mut self.work)
    }

    /// What is laid out and drawn: the committed text with the preedit in
    /// place of the range it stands in for.
    pub fn display_text(&self) -> Cow<'_, str> {
        match &self.state.composition {
            Some(composition) => composition.display_text(self.text.as_str()),
            None => Cow::Borrowed(self.text.as_str()),
        }
    }

    pub fn display_offset(&self, committed: usize) -> usize {
        self.state
            .composition
            .as_ref()
            .map_or(committed, |composition| {
                composition.display_offset(committed)
            })
    }

    pub fn committed_offset(&self, display: usize) -> usize {
        self.state
            .composition
            .as_ref()
            .map_or(display, |composition| composition.committed_offset(display))
    }

    /// The selected committed text, when the selection is not empty.
    pub fn selected_text(&self) -> Option<&str> {
        let selection = self.state.selection;
        (!selection.is_collapsed())
            .then(|| self.text.slice(selection.range()))
            .flatten()
    }

    /// The caret a platform places its candidate window at: the preedit's
    /// cursor while composing, the selection's focus otherwise. Display space.
    pub fn ime_cursor(&self) -> (usize, Affinity) {
        match &self.state.composition {
            Some(composition) => (composition.display_cursor(), Affinity::Downstream),
            None => (self.state.selection.focus, self.state.selection.affinity),
        }
    }

    /// Where to put the IME candidate window, from geometry synced to this
    /// session.
    pub fn candidate_rect(&self, geometry: &EditorGeometry) -> Option<CaretRect> {
        let (offset, affinity) = self.ime_cursor();
        geometry.caret_rect(offset, affinity)
    }

    /// Moves the selection as a consequence of a text edit: its revision moves,
    /// but it is not a selection-only update.
    fn place_selection(&mut self, selection: EditSelection) {
        if self.state.selection != selection {
            self.state.selection = selection;
            self.selection_revision += 1;
        }
    }

    fn set_selection_state(&mut self, selection: EditSelection) -> EditChange {
        if self.state.selection == selection {
            return EditChange::None;
        }
        self.state.selection = selection;
        self.selection_revision += 1;
        if selection.is_collapsed() {
            self.work.caret_only_updates += 1;
        } else {
            self.work.selection_only_updates += 1;
        }
        EditChange::Selection
    }

    fn record_edit(&mut self, edit: &TextEdit) {
        self.work.editable_mutations += 1;
        self.work.editable_bytes_inserted += edit.inserted_len;
        self.work.editable_bytes_deleted += edit.deleted_len();
    }

    fn bump_composition(&mut self) {
        self.composition_revision += 1;
        self.work.composition_updates += 1;
    }

    /// Selects `anchor..focus`, each snapped back onto a grapheme boundary.
    /// Refused while composing.
    pub fn set_selection(&mut self, anchor: usize, focus: usize, affinity: Affinity) -> EditChange {
        if self.is_composing() {
            return EditChange::None;
        }
        self.state.goal_x_px = None;
        let selection = EditSelection {
            anchor: self.text.snap_to_grapheme(anchor, false),
            focus: self.text.snap_to_grapheme(focus, false),
            affinity,
        };
        self.set_selection_state(selection)
    }

    pub fn select_all(&mut self) -> EditChange {
        self.set_selection(0, self.text.len(), Affinity::Downstream)
    }

    /// Selects the word at `offset`, as a double click does.
    pub fn select_word_at(&mut self, offset: usize) -> EditChange {
        let (start, end) = navigation::word_range_at(self.text.as_str(), offset);
        self.set_selection(start, end, Affinity::Downstream)
    }

    /// Geometry is only used when it was synced from this session's current
    /// text and composition; stale geometry would place the caret in text that
    /// is no longer there.
    fn usable<'g>(&self, geometry: Option<&'g EditorGeometry>) -> Option<&'g EditorGeometry> {
        geometry.filter(|geometry| geometry.synced_from(self.revisions()))
    }

    fn target(
        &self,
        motion: Motion,
        from: (usize, Affinity),
        geometry: Option<&EditorGeometry>,
    ) -> Option<(usize, Affinity)> {
        let text = self.text.as_str();
        let (offset, affinity) = from;
        let downstream = |offset: usize| Some((offset, Affinity::Downstream));
        let moved = |target: usize| (target != offset).then_some((target, Affinity::Downstream));
        match motion {
            Motion::GraphemeBackward => {
                navigation::prev_grapheme(text, offset).and_then(downstream)
            }
            Motion::GraphemeForward => navigation::next_grapheme(text, offset).and_then(downstream),
            Motion::Left | Motion::Right => match geometry {
                Some(geometry) => geometry.visual_move(offset, affinity, motion == Motion::Right),
                None if motion == Motion::Left => {
                    navigation::prev_grapheme(text, offset).and_then(downstream)
                }
                None => navigation::next_grapheme(text, offset).and_then(downstream),
            },
            Motion::WordBackward => moved(navigation::word_start_before(text, offset)),
            Motion::WordForward => moved(navigation::word_end_after(text, offset)),
            Motion::LineStart | Motion::LineEnd => {
                let bounds = geometry.and_then(|geometry| geometry.line_bounds(offset, affinity));
                let (start, end) = bounds.unwrap_or_else(|| {
                    let (start, end) = navigation::logical_line_range(text, offset);
                    ((start, Affinity::Downstream), (end, Affinity::Downstream))
                });
                let target = if motion == Motion::LineStart {
                    start
                } else {
                    end
                };
                (target != (offset, affinity)).then_some(target)
            }
            Motion::ParagraphStart => moved(navigation::logical_line_range(text, offset).0),
            Motion::ParagraphEnd => moved(navigation::logical_line_range(text, offset).1),
            Motion::DocumentStart => moved(0),
            Motion::DocumentEnd => moved(text.len()),
            Motion::LineUp | Motion::LineDown => {
                let lines = if motion == Motion::LineUp { -1 } else { 1 };
                match geometry {
                    Some(geometry) => {
                        let goal = self.state.goal_x_px.or_else(|| {
                            geometry
                                .caret_rect(offset, affinity)
                                .map(|caret| caret.x_px)
                        })?;
                        let target = geometry.vertical(offset, affinity, lines, goal)?;
                        (target != from).then_some(target)
                    }
                    None => moved(logical_vertical(text, offset, lines)),
                }
            }
        }
    }

    /// Moves the caret, or the selection's focus when `extend`.
    ///
    /// Without `extend`, a horizontal motion over a non-empty selection
    /// collapses it onto the edge in that direction instead of moving from the
    /// focus. Refused while composing.
    pub fn move_caret(
        &mut self,
        motion: Motion,
        extend: bool,
        geometry: Option<&EditorGeometry>,
    ) -> EditChange {
        if self.is_composing() {
            return EditChange::None;
        }
        let geometry = self.usable(geometry);
        let selection = self.state.selection;
        if !extend && !selection.is_collapsed() {
            let range = selection.range();
            let visual_edge = |rightwards: bool| {
                // On screen the left edge of a selection is not always its
                // logical start: in RTL text it is the end.
                // Across lines x says nothing about which end is which: the
                // logical order is the reading order then.
                let caret = |offset: usize, affinity| {
                    geometry.and_then(|geometry| geometry.caret_rect(offset, affinity))
                };
                let (start, end) = (
                    caret(range.start, Affinity::Downstream),
                    caret(range.end, Affinity::Upstream),
                );
                match (start, end) {
                    (Some(start), Some(end))
                        if (start.y_px - end.y_px).abs() <= f32::EPSILON
                            && (end.x_px < start.x_px) == rightwards =>
                    {
                        range.start
                    }
                    (Some(start), Some(end)) if (start.y_px - end.y_px).abs() <= f32::EPSILON => {
                        range.end
                    }
                    // Reading order: forwards is rightwards unless the
                    // paragraph reads right to left.
                    _ => {
                        let rtl = geometry
                            .and_then(|geometry| {
                                geometry.line_direction(range.start, Affinity::Downstream)
                            })
                            .is_some_and(|direction| direction.is_rtl());
                        if rightwards != rtl {
                            range.end
                        } else {
                            range.start
                        }
                    }
                }
            };
            let collapsed = match motion {
                Motion::GraphemeBackward => Some(range.start),
                Motion::GraphemeForward => Some(range.end),
                Motion::Left => Some(visual_edge(false)),
                Motion::Right => Some(visual_edge(true)),
                _ => None,
            };
            if let Some(offset) = collapsed {
                self.state.goal_x_px = None;
                return self.set_selection_state(EditSelection::caret(offset));
            }
        }
        let from = (selection.focus, selection.affinity);
        if motion.is_vertical() {
            if self.state.goal_x_px.is_none()
                && let Some(caret) =
                    geometry.and_then(|geometry| geometry.caret_rect(from.0, from.1))
            {
                self.state.goal_x_px = Some(caret.x_px);
            }
        } else {
            self.state.goal_x_px = None;
        }
        let Some((focus, affinity)) = self.target(motion, from, geometry) else {
            return EditChange::None;
        };
        let next = EditSelection {
            anchor: if extend { selection.anchor } else { focus },
            focus,
            affinity,
        };
        self.set_selection_state(next)
    }

    /// The caret after inserting `text` at `start`. It belongs to what was
    /// just typed: at the end of RTL text typed into an LTR line, or at a soft
    /// wrap, downstream would draw it somewhere else.
    fn caret_after(start: usize, text: &str) -> EditSelection {
        let caret = EditSelection::caret(start + text.len());
        if text.is_empty() || text.ends_with('\n') {
            caret
        } else {
            caret.with_affinity(Affinity::Upstream)
        }
    }

    fn replace(&mut self, range: Range<usize>, text: &str) -> EditChange {
        match self.text.replace(range.clone(), text) {
            Ok(Some(edit)) => {
                self.record_edit(&edit);
                self.state.goal_x_px = None;
                self.place_selection(Self::caret_after(range.start, text));
                EditChange::Text(edit)
            }
            Ok(None) => self.set_selection_state(Self::caret_after(range.start, text)),
            Err(_) => EditChange::None,
        }
    }

    /// Replaces the selection with `text` and puts the caret after it: typing
    /// and pasting. Refused while composing.
    pub fn insert(&mut self, text: &str) -> EditChange {
        if self.is_composing() {
            return EditChange::None;
        }
        self.replace(self.state.selection.range(), text)
    }

    /// Deletes the selection, or when it is empty the text between the caret
    /// and where `motion` would move it — so [`Motion::LineStart`] with
    /// geometry deletes to the start of the visual line. Left and right
    /// delete logically. Refused while composing.
    pub fn delete(&mut self, motion: Motion, geometry: Option<&EditorGeometry>) -> EditChange {
        if self.is_composing() {
            return EditChange::None;
        }
        let geometry = self.usable(geometry);
        let selection = self.state.selection;
        if !selection.is_collapsed() {
            return self.replace(selection.range(), "");
        }
        let motion = match motion {
            Motion::Left => Motion::GraphemeBackward,
            Motion::Right => Motion::GraphemeForward,
            other => other,
        };
        let Some((target, _)) =
            self.target(motion, (selection.focus, selection.affinity), geometry)
        else {
            return EditChange::None;
        };
        let range = selection.focus.min(target)..selection.focus.max(target);
        self.replace(range, "")
    }

    /// The selected text, removed.
    pub fn cut(&mut self) -> Option<String> {
        let text = self.selected_text()?.to_owned();
        (!self.insert("").is_none()).then_some(text)
    }

    /// Replaces the whole text. Cancels any composition; the selection is
    /// clamped into the new text.
    pub fn set_text(&mut self, text: &str) -> EditChange {
        let cancelled = self.state.composition.take().is_some();
        if cancelled {
            self.bump_composition();
        }
        let Some(edit) = self.text.set_text(text) else {
            return if cancelled {
                EditChange::Composition
            } else {
                EditChange::None
            };
        };
        self.record_edit(&edit);
        let clamp = |offset: usize| self.text.snap_to_grapheme(offset, false);
        let selection = EditSelection::new(
            clamp(self.state.selection.anchor),
            clamp(self.state.selection.focus),
        );
        self.state.goal_x_px = None;
        self.place_selection(selection);
        EditChange::Text(edit)
    }

    /// Starts or updates a composition. It stands in for the selection it
    /// started over; `selection` is the IME's own cursor or selection inside
    /// `text`. An empty `text` ends the composition, as cancelling does.
    pub fn set_preedit(&mut self, text: &str, selection: Option<Range<usize>>) -> EditChange {
        let selection = selection.filter(|range| {
            range.start <= range.end
                && range.end <= text.len()
                && text.is_char_boundary(range.start)
                && text.is_char_boundary(range.end)
        });
        if text.is_empty() {
            return self.cancel_composition();
        }
        let next = Composition {
            replaced: match &self.state.composition {
                Some(composition) => composition.replaced.clone(),
                None => self.state.selection.range(),
            },
            text: text.to_owned(),
            selection,
        };
        if self.state.composition.as_ref() == Some(&next) {
            return EditChange::None;
        }
        self.state.composition = Some(next);
        self.state.goal_x_px = None;
        self.bump_composition();
        EditChange::Composition
    }

    /// Commits `text` in place of the composition (or of the selection when
    /// nothing is composing) and ends the composition.
    pub fn commit(&mut self, text: &str) -> EditChange {
        let composed = self.state.composition.take();
        let ended = composed.is_some();
        let range = match composed {
            Some(composition) => {
                self.bump_composition();
                composition.replaced
            }
            None => self.state.selection.range(),
        };
        match self.replace(range, text) {
            EditChange::None | EditChange::Selection if ended => {
                // The committed text was what the composition replaced: no
                // byte changed, but the preedit is gone.
                EditChange::Composition
            }
            change => change,
        }
    }

    /// Ends the composition without committing. The committed text and the
    /// selection are exactly what they were before it started.
    pub fn cancel_composition(&mut self) -> EditChange {
        if self.state.composition.take().is_none() {
            return EditChange::None;
        }
        self.bump_composition();
        EditChange::Composition
    }

    /// Focus left the editor: an unfinished composition is cancelled.
    pub fn blur(&mut self) -> EditChange {
        self.cancel_composition()
    }

    /// Deletes `before` bytes before and `after` bytes after the selection —
    /// and the selection itself, the Runtime editor contract. While composing
    /// the anchor is the range the preedit stands in for, and it is kept: see
    /// [`ime::surrounding_deletion`]. Refused when either end is out of bounds
    /// or splits a character.
    pub fn delete_surrounding(&mut self, before: usize, after: usize) -> EditChange {
        let (anchor, keep) = match &self.state.composition {
            Some(composition) => (composition.replaced.clone(), true),
            None => (self.state.selection.range(), false),
        };
        let Some(deletion) =
            ime::surrounding_deletion(self.text.as_str(), anchor.clone(), before, after, keep)
        else {
            return EditChange::None;
        };
        // One edit spanning both sides, the kept anchor written back in place:
        // what a consumer diffing paragraphs sees is the same either way.
        let span = deletion.before.start..deletion.after.end.max(deletion.before.end);
        let kept = if keep {
            self.text.as_str()[anchor.clone()].to_owned()
        } else {
            String::new()
        };
        let Ok(Some(edit)) = self.text.replace(span, &kept) else {
            return EditChange::None;
        };
        self.record_edit(&edit);
        self.work.editable_bytes_inserted -= kept.len();
        self.work.editable_bytes_deleted -= kept.len();
        match &mut self.state.composition {
            Some(composition) => {
                composition.replaced =
                    deletion.map_offset(anchor.start)..deletion.map_offset(anchor.end);
                self.composition_revision += 1;
                let selection =
                    EditSelection::new(composition.replaced.start, composition.replaced.end);
                self.place_selection(selection);
            }
            None => self.place_selection(EditSelection::caret(deletion.before.start)),
        }
        self.state.goal_x_px = None;
        EditChange::Text(edit)
    }

    /// Up to `before` and `after` bytes of committed text around the selection,
    /// cut on character boundaries.
    pub fn surrounding_text(&self, before: usize, after: usize) -> SurroundingText<'_> {
        let text = self.text.as_str();
        let range = self.state.selection.range();
        let window = ime::surrounding_window(text, range.clone(), before, after);
        SurroundingText {
            text: &text[window.clone()],
            offset: window.start,
            selection: (range.start - window.start, range.end - window.start),
        }
    }
}

impl Default for EditSession {
    fn default() -> Self {
        Self::new(String::new())
    }
}

/// A copy is a new session: its edits diverge from the original's under the
/// same revision numbers.
impl Clone for EditSession {
    fn clone(&self) -> Self {
        Self {
            id: next_session_id(),
            text: self.text.clone(),
            state: self.state.clone(),
            selection_revision: self.selection_revision,
            composition_revision: self.composition_revision,
            work: self.work,
        }
    }
}

fn next_session_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl EditorGeometry {
    /// Whether this geometry was last synced from a session at these
    /// revisions' text and composition.
    pub fn synced_from(&self, revisions: EditRevisions) -> bool {
        self.synced_revisions() == Some((revisions.session, revisions.text, revisions.composition))
    }
}

/// One logical line up or down, keeping the grapheme column.
fn logical_vertical(text: &str, offset: usize, lines: isize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    let (line_start, line_end) = navigation::logical_line_range(text, offset);
    let column = text[line_start..offset.min(line_end)]
        .graphemes(true)
        .count();
    let (target_start, target_end) = if lines < 0 {
        if line_start == 0 {
            return 0;
        }
        navigation::logical_line_range(text, line_start - 1)
    } else {
        if line_end >= text.len() {
            return text.len();
        }
        navigation::logical_line_range(text, line_end + 1)
    };
    text[target_start..target_end]
        .grapheme_indices(true)
        .map(|(index, grapheme)| target_start + index + grapheme.len())
        .take(column)
        .last()
        .unwrap_or(target_start)
}
