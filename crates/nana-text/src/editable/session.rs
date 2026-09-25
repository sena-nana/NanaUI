//! Editing commands over [`EditableText`] + [`EditState`]: typing, deleting,
//! caret motion, selection and IME composition.

use super::geometry::{CaretRect, EditorGeometry};
use super::ime;
use super::navigation;
use super::state::{
    Composition, EditRevisions, EditSelection, normalize_selections, remap_selection,
};
use super::text::{EditableText, TextEdit};
use crate::counters::TextWorkCounters;
use crate::edit::Affinity;
use crate::shared::SharedText;
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
///
/// `selection` is the primary selection: the one a platform IME composes
/// over, accessibility reports and a vertical run keeps its goal column for.
/// `additional` holds any further cursors or selections, in document order;
/// together with the primary they never overlap or touch
/// ([`normalize_selections`](super::normalize_selections) keeps it so).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EditState {
    pub selection: EditSelection,
    pub additional: Vec<EditSelection>,
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

    /// A session over `text` adopted without a copy, with `selection` as its
    /// primary selection and `additional` as further cursors. Every offset is
    /// snapped back onto a grapheme boundary and the set is normalized.
    pub fn with_selections(
        text: SharedText,
        selection: EditSelection,
        additional: impl IntoIterator<Item = EditSelection>,
    ) -> Self {
        let mut session = Self {
            id: next_session_id(),
            text: EditableText::from_shared(text),
            state: EditState::default(),
            selection_revision: 0,
            composition_revision: 0,
            work: TextWorkCounters::default(),
        };
        let (primary, additional) = session.normalized(selection, additional);
        session.state.selection = primary;
        session.state.additional = additional;
        session
    }

    pub fn text(&self) -> &EditableText {
        &self.text
    }

    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }

    /// An O(1) copy of the committed text; see [`EditableText::snapshot`].
    pub fn snapshot(&self) -> SharedText {
        self.text.snapshot()
    }

    pub fn state(&self) -> &EditState {
        &self.state
    }

    /// The primary selection.
    pub fn selection(&self) -> EditSelection {
        self.state.selection
    }

    /// The selections besides the primary, in document order.
    pub fn additional_selections(&self) -> &[EditSelection] {
        &self.state.additional
    }

    pub fn has_additional_selections(&self) -> bool {
        !self.state.additional.is_empty()
    }

    /// Every selection in document order, the primary among them.
    pub fn selections(&self) -> Cow<'_, [EditSelection]> {
        if self.state.additional.is_empty() {
            return Cow::Borrowed(std::slice::from_ref(&self.state.selection));
        }
        let mut all = Vec::with_capacity(self.state.additional.len() + 1);
        all.push(self.state.selection);
        all.extend_from_slice(&self.state.additional);
        all.sort_by_key(|selection| {
            let range = selection.range();
            (range.start, range.end)
        });
        Cow::Owned(all)
    }

    /// Position of the primary within [`Self::selections`].
    pub fn primary_index(&self) -> usize {
        let start = self.state.selection.range().start;
        self.state
            .additional
            .iter()
            .filter(|selection| selection.range().start < start)
            .count()
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

    /// The text of every non-empty selection in document order, joined with
    /// line feeds — what a copy puts on the pasteboard with several cursors.
    /// `None` when every selection is a bare caret.
    pub fn selected_texts(&self) -> Option<String> {
        let parts: Vec<&str> = self
            .selections()
            .iter()
            .filter(|selection| !selection.is_collapsed())
            .filter_map(|selection| self.text.slice(selection.range()))
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    /// The selected committed text, when the primary selection is not empty.
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

    /// `selection` with both ends snapped back onto grapheme boundaries of
    /// the committed text. A focus the snap moved loses its affinity.
    fn snapped(&self, selection: EditSelection) -> EditSelection {
        let focus = self.text.snap_to_grapheme(selection.focus, false);
        EditSelection {
            anchor: self.text.snap_to_grapheme(selection.anchor, false),
            focus,
            affinity: if focus == selection.focus {
                selection.affinity
            } else {
                Affinity::Downstream
            },
        }
    }

    fn normalized(
        &self,
        primary: EditSelection,
        additional: impl IntoIterator<Item = EditSelection>,
    ) -> (EditSelection, Vec<EditSelection>) {
        normalize_selections(
            self.snapped(primary),
            additional
                .into_iter()
                .map(|selection| self.snapped(selection)),
        )
    }

    /// Moves the selections as a consequence of a text edit: their revision
    /// moves, but it is not a selection-only update.
    fn place_selections(&mut self, primary: EditSelection, additional: Vec<EditSelection>) {
        let (primary, additional) = normalize_selections(primary, additional);
        if self.state.selection != primary || self.state.additional != additional {
            self.state.selection = primary;
            self.state.additional = additional;
            self.selection_revision += 1;
        }
    }

    fn place_selection(&mut self, selection: EditSelection) {
        self.place_selections(selection, Vec::new());
    }

    /// A selection-only change. Counted as a caret update only when every
    /// cursor is a bare caret, so a live multi-cursor selection is never
    /// reported as a caret move.
    fn set_selections_state(
        &mut self,
        primary: EditSelection,
        additional: Vec<EditSelection>,
    ) -> EditChange {
        let (primary, additional) = normalize_selections(primary, additional);
        if self.state.selection == primary && self.state.additional == additional {
            return EditChange::None;
        }
        self.state.selection = primary;
        self.state.additional = additional;
        self.selection_revision += 1;
        let collapsed = self.state.selection.is_collapsed()
            && self
                .state
                .additional
                .iter()
                .all(EditSelection::is_collapsed);
        if collapsed {
            self.work.caret_only_updates += 1;
        } else {
            self.work.selection_only_updates += 1;
        }
        EditChange::Selection
    }

    fn set_selection_state(&mut self, selection: EditSelection) -> EditChange {
        self.set_selections_state(selection, Vec::new())
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

    /// Selects `anchor..focus`, each snapped back onto a grapheme boundary,
    /// as the only selection: any further cursors go. Refused while
    /// composing.
    pub fn set_selection(&mut self, anchor: usize, focus: usize, affinity: Affinity) -> EditChange {
        if self.is_composing() {
            return EditChange::None;
        }
        self.state.goal_x_px = None;
        let selection = self.snapped(EditSelection {
            anchor,
            focus,
            affinity,
        });
        self.set_selection_state(selection)
    }

    /// Replaces the whole selection set: `primary` plus `additional`, snapped
    /// onto grapheme boundaries and normalized. Refused while composing.
    pub fn set_selections(
        &mut self,
        primary: EditSelection,
        additional: impl IntoIterator<Item = EditSelection>,
    ) -> EditChange {
        if self.is_composing() {
            return EditChange::None;
        }
        self.state.goal_x_px = None;
        let (primary, additional) = self.normalized(primary, additional);
        self.set_selections_state(primary, additional)
    }

    /// Moves the primary selection, keeping the other cursors (fusing any the
    /// primary now overlaps or touches). Only the primary is snapped: the
    /// others already are, so a caret move with many cursors on long lines
    /// does not rescan every cursor's line. Refused while composing.
    pub fn set_primary_selection(&mut self, selection: EditSelection) -> EditChange {
        if self.is_composing() {
            return EditChange::None;
        }
        self.state.goal_x_px = None;
        let primary = self.snapped(selection);
        let additional = self.state.additional.clone();
        self.set_selections_state(primary, additional)
    }

    /// Adds cursors or selections to the set, fusing any that overlap or
    /// touch. Refused while composing.
    pub fn add_selections(
        &mut self,
        candidates: impl IntoIterator<Item = EditSelection>,
    ) -> EditChange {
        if self.is_composing() {
            return EditChange::None;
        }
        let additional: Vec<EditSelection> = self
            .state
            .additional
            .iter()
            .copied()
            .chain(
                candidates
                    .into_iter()
                    .map(|selection| self.snapped(selection)),
            )
            .collect();
        self.set_selections_state(self.state.selection, additional)
    }

    /// Drops every selection but the primary.
    pub fn collapse_selections(&mut self) -> EditChange {
        if self.state.additional.is_empty() {
            return EditChange::None;
        }
        self.set_selections_state(self.state.selection, Vec::new())
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
        goal_x_px: Option<f32>,
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
                        let goal = goal_x_px.or_else(|| {
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

    /// Moves the caret, or the selection's focus when `extend` — every
    /// selection by the same motion, the fused result normalized.
    ///
    /// Without `extend`, a horizontal motion over a non-empty selection
    /// collapses it onto the edge in that direction instead of moving from the
    /// focus ([`collapse_edge`]). A vertical run keeps the primary's goal
    /// column; the other cursors aim at their own. Refused while composing.
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
        let primary = self.state.selection;
        if motion.is_vertical() {
            if self.state.goal_x_px.is_none()
                && let Some(caret) = geometry
                    .and_then(|geometry| geometry.caret_rect(primary.focus, primary.affinity))
            {
                self.state.goal_x_px = Some(caret.x_px);
            }
        } else {
            self.state.goal_x_px = None;
        }
        let goal = self.state.goal_x_px;
        let moved_primary = self.moved(primary, motion, extend, geometry, goal);
        let mut any = moved_primary.is_some();
        let additional: Vec<EditSelection> = self
            .state
            .additional
            .iter()
            .map(
                |&selection| match self.moved(selection, motion, extend, geometry, None) {
                    Some(next) => {
                        any = true;
                        next
                    }
                    None => selection,
                },
            )
            .collect();
        if !any {
            return EditChange::None;
        }
        self.set_selections_state(moved_primary.unwrap_or(primary), additional)
    }

    /// Where one selection goes under `motion`; `None` when it stays.
    fn moved(
        &self,
        selection: EditSelection,
        motion: Motion,
        extend: bool,
        geometry: Option<&EditorGeometry>,
        goal_x_px: Option<f32>,
    ) -> Option<EditSelection> {
        if !extend && !selection.is_collapsed() {
            let range = selection.range();
            // Columns: x and y say nothing about which end reads first.
            let horizontal = geometry.filter(|geometry| !geometry.is_vertical());
            let caret = |offset: usize, affinity| {
                horizontal.and_then(|geometry| geometry.caret_rect(offset, affinity))
            };
            let visual = |rightwards: bool| {
                collapse_edge(
                    range.clone(),
                    rightwards,
                    caret(range.start, Affinity::Downstream).map(|caret| (caret.x_px, caret.y_px)),
                    caret(range.end, Affinity::Upstream).map(|caret| (caret.x_px, caret.y_px)),
                    geometry
                        .and_then(|geometry| {
                            geometry.line_direction(range.start, Affinity::Downstream)
                        })
                        .is_some_and(|direction| direction.is_rtl()),
                )
            };
            let collapsed = match motion {
                Motion::GraphemeBackward => Some(range.start),
                Motion::GraphemeForward => Some(range.end),
                Motion::Left => Some(visual(false)),
                Motion::Right => Some(visual(true)),
                _ => None,
            };
            if let Some(offset) = collapsed {
                // On the focus, the caret keeps the side it was drawn on; on
                // the anchor, the side of the selection it ends (a selection
                // ending at a soft wrap ends on the line it covers).
                let caret = EditSelection::caret(offset);
                return Some(if offset == selection.focus {
                    caret.with_affinity(selection.affinity)
                } else if offset == range.end {
                    caret.with_affinity(Affinity::Upstream)
                } else {
                    caret
                });
            }
        }
        let from = (selection.focus, selection.affinity);
        let (focus, affinity) = self.target(motion, from, geometry, goal_x_px)?;
        Some(EditSelection {
            anchor: if extend { selection.anchor } else { focus },
            focus,
            affinity,
        })
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

    /// Replaces the primary's `range` with `text` and puts the primary caret
    /// after it; every other cursor moves through the edit.
    fn replace(&mut self, range: Range<usize>, text: &str) -> EditChange {
        let caret = Self::caret_after(range.start, text);
        match self.text.replace(range.clone(), text) {
            Ok(Some(edit)) => {
                self.record_edit(&edit);
                self.state.goal_x_px = None;
                let additional = self
                    .state
                    .additional
                    .iter()
                    .map(|&selection| {
                        remap_selection(selection, range.start, range.len(), text.len())
                    })
                    .collect();
                self.place_selections(caret, additional);
                EditChange::Text(edit)
            }
            Ok(None) => self.set_selections_state(caret, self.state.additional.clone()),
            Err(_) => EditChange::None,
        }
    }

    /// Replaces every one of `ranges` with `text` in one edit, a caret after
    /// each insertion. Overlapping ranges fuse (the primary flag rides along),
    /// so two cursors deleting into each other delete the union once.
    fn replace_each(&mut self, mut ranges: Vec<(Range<usize>, bool)>, text: &str) -> EditChange {
        ranges.sort_by_key(|(range, _)| (range.start, range.end));
        let mut fused: Vec<(Range<usize>, bool)> = Vec::with_capacity(ranges.len());
        for (range, primary) in ranges {
            match fused.last_mut() {
                Some((last, last_primary)) if range.start < last.end => {
                    last.end = last.end.max(range.end);
                    *last_primary |= primary;
                }
                _ => fused.push((range, primary)),
            }
        }
        let mut primary = None;
        let mut additional = Vec::with_capacity(fused.len());
        let mut shift = 0isize;
        for (range, is_primary) in &fused {
            let start = (range.start as isize + shift) as usize;
            let caret = Self::caret_after(start, text);
            if *is_primary && primary.is_none() {
                primary = Some(caret);
            } else {
                additional.push(caret);
            }
            shift += text.len() as isize - range.len() as isize;
        }
        let primary = primary.unwrap_or_else(|| additional.remove(0));
        let edits: Vec<(Range<usize>, &str)> = fused
            .iter()
            .map(|(range, _)| (range.clone(), text))
            .collect();
        let changing = self.changing(&edits);
        match self.text.splice(&edits) {
            Ok(Some(edit)) => {
                self.record_splice(&changing);
                self.state.goal_x_px = None;
                self.place_selections(primary, additional);
                EditChange::Text(edit)
            }
            Ok(None) => self.set_selections_state(primary, additional),
            Err(_) => EditChange::None,
        }
    }

    /// The edits of `edits` that would change a byte: the rest rewrite what
    /// is already there, and are neither work nor a reach into a preedit.
    fn changing<'e>(&self, edits: &[(Range<usize>, &'e str)]) -> Vec<(Range<usize>, &'e str)> {
        edits
            .iter()
            .filter(|(range, text)| self.text.slice(range.clone()) != Some(*text))
            .cloned()
            .collect()
    }

    /// Counts a batch of edits as one mutation of their summed bytes.
    fn record_splice(&mut self, edits: &[(Range<usize>, &str)]) {
        self.work.editable_mutations += 1;
        for (range, text) in edits {
            self.work.editable_bytes_inserted += text.len();
            self.work.editable_bytes_deleted += range.len();
        }
    }

    /// Replaces every selection with `text` and puts a caret after each
    /// insertion: typing and pasting. Refused while composing.
    pub fn insert(&mut self, text: &str) -> EditChange {
        if self.is_composing() {
            return EditChange::None;
        }
        if self.state.additional.is_empty() {
            return self.replace(self.state.selection.range(), text);
        }
        let primary = self.state.selection;
        let ranges = self
            .selections()
            .iter()
            .map(|selection| (selection.range(), *selection == primary))
            .collect();
        self.replace_each(ranges, text)
    }

    /// Deletes each selection, or for a bare caret the text between it and
    /// where `motion` would move it — so [`Motion::LineStart`] with geometry
    /// deletes to the start of the visual line. Left and right delete
    /// logically. Refused while composing.
    pub fn delete(&mut self, motion: Motion, geometry: Option<&EditorGeometry>) -> EditChange {
        if self.is_composing() {
            return EditChange::None;
        }
        let geometry = self.usable(geometry);
        let motion = match motion {
            Motion::Left => Motion::GraphemeBackward,
            Motion::Right => Motion::GraphemeForward,
            other => other,
        };
        let primary = self.state.selection;
        let span = |selection: EditSelection| -> Option<Range<usize>> {
            if !selection.is_collapsed() {
                return Some(selection.range());
            }
            // A vertical run's goal column is the primary's.
            let goal = (selection == primary)
                .then_some(self.state.goal_x_px)
                .flatten();
            let (target, _) = self.target(
                motion,
                (selection.focus, selection.affinity),
                geometry,
                goal,
            )?;
            Some(selection.focus.min(target)..selection.focus.max(target))
        };
        if self.state.additional.is_empty() {
            let Some(range) = span(self.state.selection) else {
                return EditChange::None;
            };
            return self.replace(range, "");
        }
        let mut any = false;
        let ranges: Vec<(Range<usize>, bool)> = self
            .selections()
            .iter()
            .map(|&selection| {
                let range = span(selection).unwrap_or(selection.focus..selection.focus);
                any |= !range.is_empty();
                (range, selection == primary)
            })
            .collect();
        if !any {
            return EditChange::None;
        }
        self.replace_each(ranges, "")
    }

    /// The selected text of every selection, removed. Several selections
    /// come back joined with line feeds, as [`Self::selected_texts`].
    pub fn cut(&mut self) -> Option<String> {
        let text = self.selected_texts()?;
        (!self.insert("").is_none()).then_some(text)
    }

    /// Replaces the whole text. Cancels any composition; the selection is
    /// clamped into the new text and further cursors go.
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

    /// Whether any of `edits` (committed ranges) reaches into the text a
    /// preedit stands in for: overlaps it, or inserts strictly inside it.
    /// An insertion at its start lands before the preedit and one at its end
    /// after it; a deletion next to it leaves it alone.
    fn edits_reach_composition(&self, edits: &[(Range<usize>, &str)]) -> bool {
        let Some(composition) = &self.state.composition else {
            return false;
        };
        let replaced = &composition.replaced;
        edits.iter().any(|(range, _)| {
            (range.start < replaced.end && range.end > replaced.start)
                || (range.is_empty() && range.start > replaced.start && range.start < replaced.end)
        })
    }

    /// Moves the composition's replaced range through `edits` that stay clear
    /// of it ([`Self::edits_reach_composition`]).
    fn shift_composition(&mut self, edits: &[(Range<usize>, &str)]) {
        let Some(composition) = &mut self.state.composition else {
            return;
        };
        let replaced = composition.replaced.clone();
        let delta = |before: &dyn Fn(&Range<usize>) -> bool| -> isize {
            edits
                .iter()
                .filter(|(range, _)| before(range))
                .map(|(range, text)| text.len() as isize - range.len() as isize)
                .sum()
        };
        // Everything ending at or before the start comes before the preedit,
        // an insertion at its start included.
        let start_shift = delta(&|range| range.end <= replaced.start);
        // An empty preedit is one point: what goes before its start goes
        // before its end too. Otherwise an insertion at the end lands after.
        let end_shift = if replaced.is_empty() {
            start_shift
        } else {
            delta(&|range| range.end <= replaced.end && range.start < replaced.end)
        };
        let next = (replaced.start as isize + start_shift) as usize
            ..(replaced.end as isize + end_shift) as usize;
        if next != composition.replaced {
            composition.replaced = next;
            self.composition_revision += 1;
        }
    }

    /// Applies `edits` — committed ranges of the current text, sorted and
    /// disjoint — as one edit, and leaves the selection set at `primary` plus
    /// `additional` (offsets of the text after the edit).
    ///
    /// The caller computed the edit and where its cursors go; a multi-cursor
    /// transform is exactly that. While composing, edits clear of the
    /// preedit's range keep the composition (it moves with them, and so does
    /// the primary selection it stands in for, whatever `primary` says); one
    /// that reaches into it cancels the composition first.
    pub fn splice(
        &mut self,
        edits: &[(Range<usize>, &str)],
        primary: EditSelection,
        additional: impl IntoIterator<Item = EditSelection>,
    ) -> EditChange {
        if !self.valid_edits(edits) {
            return EditChange::None;
        }
        let changing = self.changing(edits);
        let cancelled = self.edits_reach_composition(&changing);
        if cancelled {
            self.state.composition = None;
            self.bump_composition();
        }
        // `Some` exactly while composing.
        let before = self.composed_primary();
        match self.text.splice(edits) {
            Ok(Some(edit)) => {
                self.record_splice(&changing);
                if before.is_some() {
                    self.shift_composition(&changing);
                }
                self.state.goal_x_px = None;
                let (primary, additional) = self.normalized(primary, additional);
                let primary = match &before {
                    Some(before) => self.primary_on_composition(before, primary),
                    None => primary,
                };
                self.place_selections(primary, additional);
                EditChange::Text(edit)
            }
            _ if cancelled => EditChange::Composition,
            Ok(None) if before.is_some() => EditChange::None,
            Ok(None) => {
                let (primary, additional) = self.normalized(primary, additional);
                self.set_selections_state(primary, additional)
            }
            Err(_) => EditChange::None,
        }
    }

    /// Whether `edits` are in bounds, on character boundaries, sorted and
    /// disjoint: what [`EditableText::splice`] accepts.
    fn valid_edits(&self, edits: &[(Range<usize>, &str)]) -> bool {
        let mut previous_end = 0;
        edits.iter().all(|(range, _)| {
            let valid = range.start >= previous_end
                && range.start <= range.end
                && self.text.slice(range.clone()).is_some();
            previous_end = range.end;
            valid
        })
    }

    /// Makes the committed text `text` as the one edit that differs from it,
    /// so offsets and layouts outside the change survive. A [`SharedText`]
    /// snapshot of these very bytes is recognised by its stamp in O(1).
    ///
    /// The selection set becomes `selections` when given; otherwise every
    /// cursor moves through the edit ([`remap_selection`](super::remap_selection)),
    /// and one left inside a cluster the edit completed goes after it.
    /// A composition clear of the change survives it, as with [`Self::splice`]:
    /// an ambiguous change is placed clear of it where the text allows, and
    /// the primary selection stays the range the preedit stands in for,
    /// given `selections` or not.
    pub fn assign(
        &mut self,
        text: &SharedText,
        selections: Option<(EditSelection, Vec<EditSelection>)>,
    ) -> EditChange {
        let changed = if self.text.snapshot().same_identity(text) {
            None
        } else {
            let changed = match &self.state.composition {
                // Where the change is ambiguous, not where it ends the
                // composition.
                Some(composition) => super::diff::changed_range_clear_of(
                    self.text.as_str(),
                    text,
                    composition.replaced.clone(),
                ),
                None => super::diff::changed_range(self.text.as_str(), text),
            };
            if changed.is_none() {
                // The same bytes in another buffer (a component's copy):
                // share it, so the next comparison is a pointer check.
                self.text.adopt_equal(text);
            }
            changed
        };
        let Some((start, old_end, new_end)) = changed else {
            return match selections {
                Some((primary, additional)) if !self.is_composing() => {
                    let (primary, additional) = self.normalized(primary, additional);
                    self.set_selections_state(primary, additional)
                }
                _ => EditChange::None,
            };
        };
        let inserted = &text[start..new_end];
        let edits = [(start..old_end, inserted)];
        if self.edits_reach_composition(&edits) {
            self.state.composition = None;
            self.bump_composition();
        }
        let before = self.composed_primary();
        let edit = self.text.adopt(text, start, old_end, new_end);
        self.record_edit(&edit);
        self.shift_composition(&edits);
        self.state.goal_x_px = None;
        let (primary, additional) = match selections {
            Some((primary, additional)) => self.normalized(primary, additional),
            None => {
                // An offset the edit left in place can now sit inside a
                // cluster the insertion completed ("e" + U+0301): it was
                // after that base, so it goes after the whole cluster.
                let remap = |selection| {
                    let moved = remap_selection(selection, start, old_end - start, new_end - start);
                    let focus = self.text.snap_to_grapheme(moved.focus, true);
                    EditSelection {
                        anchor: self.text.snap_to_grapheme(moved.anchor, true),
                        focus,
                        affinity: if focus == moved.focus {
                            moved.affinity
                        } else {
                            Affinity::Downstream
                        },
                    }
                };
                let primary = remap(self.state.selection);
                let additional: Vec<EditSelection> = self
                    .state
                    .additional
                    .iter()
                    .map(|&selection| remap(selection))
                    .collect();
                self.normalized(primary, additional)
            }
        };
        let primary = match &before {
            Some(before) => self.primary_on_composition(before, primary),
            None => primary,
        };
        self.place_selections(primary, additional);
        EditChange::Text(edit)
    }

    /// While composing, the primary selection is the range the preedit
    /// stands in for. An edit that moved the composition moves the primary
    /// with it rather than by its own offset rule: an insertion at the
    /// preedit's start lands before the preedit, so before its selection too.
    fn primary_on_composition(
        &self,
        before: &(EditSelection, Range<usize>),
        primary: EditSelection,
    ) -> EditSelection {
        let (previous, was) = before;
        let Some(now) = &self.state.composition else {
            return primary;
        };
        if previous.range() != *was {
            return primary;
        }
        let range = now.replaced.clone();
        let selection = if previous.focus < previous.anchor {
            EditSelection::new(range.end, range.start)
        } else {
            EditSelection::new(range.start, range.end)
        };
        selection.with_affinity(previous.affinity)
    }

    /// What [`Self::primary_on_composition`] reads of the state before an
    /// edit: the primary selection and the range the preedit stands in for,
    /// while composing. Not the whole state: the preedit and every other
    /// cursor would be copied for nothing.
    fn composed_primary(&self) -> Option<(EditSelection, Range<usize>)> {
        let composition = self.state.composition.as_ref()?;
        Some((self.state.selection, composition.replaced.clone()))
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
        let additional: Vec<EditSelection> = self
            .state
            .additional
            .iter()
            .map(|selection| {
                let focus = deletion.map_offset(selection.focus);
                EditSelection {
                    anchor: deletion.map_offset(selection.anchor),
                    focus,
                    affinity: if focus == selection.focus {
                        selection.affinity
                    } else {
                        Affinity::Downstream
                    },
                }
            })
            .collect();
        match &mut self.state.composition {
            Some(composition) => {
                composition.replaced =
                    deletion.map_offset(anchor.start)..deletion.map_offset(anchor.end);
                self.composition_revision += 1;
                let selection =
                    EditSelection::new(composition.replaced.start, composition.replaced.end);
                self.place_selections(selection, additional);
            }
            None => self.place_selections(EditSelection::caret(deletion.before.start), additional),
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

/// Where a non-extending Left or Right lands on a non-empty selection: the
/// selection's edge on that side of the screen, not a step from its focus.
///
/// `start` and `end` are the carets drawn at the selection's logical start
/// (downstream) and end (upstream), as `(x, y)`. On one line x decides — in
/// RTL text the left edge is the logical end. Across lines, or without
/// geometry, x says nothing about which end is which, so the reading order
/// does: forwards is rightwards unless the paragraph reads right to left.
///
/// One rule for [`EditSession::move_caret`] and for hosts that keep their own
/// motion code and read carets through probes.
pub fn collapse_edge(
    range: Range<usize>,
    rightwards: bool,
    start: Option<(f32, f32)>,
    end: Option<(f32, f32)>,
    paragraph_rtl: bool,
) -> usize {
    match (start, end) {
        (Some((start_x, start_y)), Some((end_x, end_y)))
            if (start_y - end_y).abs() <= f32::EPSILON =>
        {
            if (end_x < start_x) == rightwards {
                range.start
            } else {
                range.end
            }
        }
        _ if rightwards != paragraph_rtl => range.end,
        _ => range.start,
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
