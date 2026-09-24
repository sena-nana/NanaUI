//! Selection and IME composition: editor state that is not text.

use crate::edit::Affinity;
use std::borrow::Cow;
use std::ops::Range;

/// A selection in committed byte offsets.
///
/// `anchor` stays where the selection started, `focus` moves. Equal ends are a
/// caret. `affinity` says which side of an ambiguous position the focus draws
/// on: the end of a soft-wrapped line or the start of the next, and the run
/// before or after a BiDi boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EditSelection {
    pub anchor: usize,
    pub focus: usize,
    pub affinity: Affinity,
}

impl EditSelection {
    pub const fn caret(offset: usize) -> Self {
        Self {
            anchor: offset,
            focus: offset,
            affinity: Affinity::Downstream,
        }
    }

    pub const fn new(anchor: usize, focus: usize) -> Self {
        Self {
            anchor,
            focus,
            affinity: Affinity::Downstream,
        }
    }

    #[must_use]
    pub const fn with_affinity(mut self, affinity: Affinity) -> Self {
        self.affinity = affinity;
        self
    }

    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    /// The selected bytes, low end first.
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.focus)..self.anchor.max(self.focus)
    }

    /// [`Self::range`], by value.
    pub fn ordered(self) -> Range<usize> {
        self.range()
    }

    /// Whether both ends lie on grapheme cluster boundaries of `text`.
    ///
    /// Scans the logical line around each end, not the whole text: this runs
    /// on every edit and caret move.
    pub fn is_valid_for(self, text: &str) -> bool {
        self.anchor <= text.len()
            && self.focus <= text.len()
            && text.is_char_boundary(self.anchor)
            && text.is_char_boundary(self.focus)
            && super::navigation::is_grapheme_boundary(text, self.anchor)
            && super::navigation::is_grapheme_boundary(text, self.focus)
    }
}

/// Restores the multi-selection invariants over a primary selection and any
/// number of others: sorted by span, overlapping or touching spans fused into
/// one forward selection, and the primary's identity carried through a fusion
/// — the span a fusion with the primary produces is the primary.
///
/// Returns the primary and the remaining spans in document order, none of
/// which overlaps or touches another or the primary.
pub fn normalize_selections(
    primary: EditSelection,
    others: impl IntoIterator<Item = EditSelection>,
) -> (EditSelection, Vec<EditSelection>) {
    let mut flagged: Vec<(EditSelection, bool)> = std::iter::once((primary, true))
        .chain(others.into_iter().map(|selection| (selection, false)))
        .collect();
    if flagged.len() == 1 {
        return (primary, Vec::new());
    }
    flagged.sort_by_key(|(selection, _)| {
        let range = selection.range();
        (range.start, range.end)
    });
    let mut merged: Vec<(EditSelection, bool)> = Vec::with_capacity(flagged.len());
    for (next, is_primary) in flagged {
        match merged.last_mut() {
            Some((last, last_is_primary)) if last.range().end >= next.range().start => {
                let start = last.range().start;
                let end = last.range().end.max(next.range().end);
                *last = EditSelection::new(start, end);
                *last_is_primary |= is_primary;
            }
            _ => merged.push((next, is_primary)),
        }
    }
    let mut primary = None;
    let mut others = Vec::with_capacity(merged.len().saturating_sub(1));
    for (selection, is_primary) in merged {
        if is_primary && primary.is_none() {
            primary = Some(selection);
        } else {
            others.push(selection);
        }
    }
    (primary.unwrap_or_default(), others)
}

/// Where an offset lands after `removed` bytes at `start` became `inserted`
/// bytes: before the edit it stays, after it it shifts, and inside the
/// replaced span it moves to the end of what was inserted — a cursor inside
/// deleted text ends up where the edit left off.
pub fn remap_offset(offset: usize, start: usize, removed: usize, inserted: usize) -> usize {
    let end = start + removed;
    if offset <= start {
        offset
    } else if offset >= end {
        offset - removed + inserted
    } else {
        start + inserted
    }
}

/// [`remap_offset`] for a selection. A focus the edit moved no longer knows
/// which side of a soft wrap it was resolved on, so its affinity resets.
pub fn remap_selection(
    selection: EditSelection,
    start: usize,
    removed: usize,
    inserted: usize,
) -> EditSelection {
    let focus = remap_offset(selection.focus, start, removed, inserted);
    EditSelection {
        anchor: remap_offset(selection.anchor, start, removed, inserted),
        focus,
        affinity: if focus == selection.focus {
            selection.affinity
        } else {
            Affinity::Downstream
        },
    }
}

/// An IME composition (preedit): transient text standing in for a committed
/// range until the IME commits or cancels it.
///
/// The committed text is never touched while composing. What a user sees is
/// the *display* text — the committed text with [`Self::replaced`] swapped for
/// [`Self::text`] — so cancelling restores exactly what was there, and a
/// preedit keystroke is not an edit that has to be undone.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Composition {
    /// Committed bytes the preedit stands in for: the selection when the
    /// composition started, moved by any surrounding-text deletion since.
    pub replaced: Range<usize>,
    /// Preedit text.
    pub text: String,
    /// The IME's own cursor or selection inside [`Self::text`], in bytes.
    /// `None` puts the cursor at its end.
    pub selection: Option<Range<usize>>,
}

impl Composition {
    /// Where the preedit sits in the display text.
    pub fn display_range(&self) -> Range<usize> {
        self.replaced.start..self.replaced.start + self.text.len()
    }

    /// The preedit's cursor, as a display offset.
    pub fn display_cursor(&self) -> usize {
        self.replaced.start
            + self
                .selection
                .as_ref()
                .map_or(self.text.len(), |selection| selection.end)
    }

    /// The segment the IME is converting, as a display range, when it names
    /// a non-empty one.
    pub fn display_target(&self) -> Option<Range<usize>> {
        let selection = self.selection.as_ref().filter(|range| !range.is_empty())?;
        Some(self.replaced.start + selection.start..self.replaced.start + selection.end)
    }

    /// A committed offset in display space. Offsets inside the replaced range
    /// have no display position of their own and land on the preedit's start.
    pub fn display_offset(&self, committed: usize) -> usize {
        if committed <= self.replaced.start {
            committed
        } else if committed >= self.replaced.end {
            committed - self.replaced.len() + self.text.len()
        } else {
            self.replaced.start
        }
    }

    /// A display offset in committed space. Offsets inside the preedit land on
    /// the replaced range's start.
    pub fn committed_offset(&self, display: usize) -> usize {
        let preedit = self.display_range();
        if display <= preedit.start {
            display
        } else if display >= preedit.end {
            display - self.text.len() + self.replaced.len()
        } else {
            self.replaced.start
        }
    }

    /// The committed text as displayed: the preedit spliced over `replaced`.
    pub fn display_text<'a>(&self, committed: &'a str) -> Cow<'a, str> {
        if self.replaced.is_empty() && self.text.is_empty() {
            return Cow::Borrowed(committed);
        }
        let mut display = String::with_capacity(committed.len() + self.text.len());
        display.push_str(&committed[..self.replaced.start]);
        display.push_str(&self.text);
        display.push_str(&committed[self.replaced.end..]);
        Cow::Owned(display)
    }
}

/// How many times each kind of editor state has changed. Text has its own
/// [`TextRevision`](crate::TextRevision) on [`EditableText`](super::EditableText).
///
/// Separate counters are the contract that lets a consumer skip work: a caret
/// move bumps `selection` only, so nothing keyed on the text or the
/// composition is invalidated by it, and a caret blink bumps nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EditRevisions {
    /// Which session these revisions count. Two sessions' revisions are not
    /// comparable, so geometry synced from one is never taken as current for
    /// another.
    pub session: u64,
    pub text: crate::TextRevision,
    pub selection: u64,
    pub composition: u64,
}
