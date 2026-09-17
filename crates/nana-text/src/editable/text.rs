//! Editable storage: committed text behind a revision and a narrow edit API.

use super::navigation;
use crate::id::TextRevision;
use std::ops::Range;

/// Why an edit was refused. A refused edit changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// The range ends past the text or starts after it ends.
    OutOfBounds,
    /// An end of the range splits a UTF-8 character.
    NotCharBoundary,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::OutOfBounds => "edit range is outside the text",
            Self::NotCharBoundary => "edit range splits a UTF-8 character",
        })
    }
}

impl std::error::Error for EditError {}

/// One applied replacement: `range` of the text before it became
/// `inserted_len` bytes.
///
/// Enough to move any offset of the old text into the new one, and to tell a
/// consumer holding per-paragraph results which of them the edit touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    /// Replaced bytes, in the text before the edit.
    pub range: Range<usize>,
    /// Bytes inserted at `range.start`.
    pub inserted_len: usize,
    /// The text's revision after the edit.
    pub revision: TextRevision,
}

impl TextEdit {
    pub fn deleted_len(&self) -> usize {
        self.range.end - self.range.start
    }

    /// The inserted bytes, in the text after the edit.
    pub fn inserted_range(&self) -> Range<usize> {
        self.range.start..self.range.start + self.inserted_len
    }

    /// Where an offset of the old text lands in the new one.
    ///
    /// Offsets before the edit stay, offsets after it shift by the length
    /// change, and offsets inside the replaced range collapse onto its start
    /// or, with `after_insertion`, onto the end of the inserted text.
    pub fn map_offset(&self, offset: usize, after_insertion: bool) -> usize {
        if offset < self.range.start || (offset == self.range.start && !after_insertion) {
            offset
        } else if offset >= self.range.end && offset > self.range.start {
            offset - self.deleted_len() + self.inserted_len
        } else if after_insertion {
            self.range.start + self.inserted_len
        } else {
            self.range.start
        }
    }
}

/// Committed editable text.
///
/// The storage is private and every change goes through [`Self::replace`],
/// which bumps the revision and reports the [`TextEdit`]. Callers read the
/// text as `&str` and never name the storage type, so it can become a rope or
/// a piece table without an API change once a large-document benchmark asks
/// for one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditableText {
    storage: String,
    revision: TextRevision,
}

impl EditableText {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            storage: text.into(),
            revision: TextRevision::INITIAL,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.storage
    }

    pub fn len(&self) -> usize {
        self.storage.len()
    }

    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Bumped by every edit that changed a byte, and only by those.
    pub fn revision(&self) -> TextRevision {
        self.revision
    }

    /// The bytes of `range`, or `None` if it is out of bounds or splits a
    /// character.
    pub fn slice(&self, range: Range<usize>) -> Option<&str> {
        self.storage.get(range)
    }

    /// Replaces `range` with `text`.
    ///
    /// `Ok(None)` when the replacement is the bytes already there: nothing
    /// changed, so neither the revision nor any derived layout moves.
    pub fn replace(
        &mut self,
        range: Range<usize>,
        text: &str,
    ) -> Result<Option<TextEdit>, EditError> {
        self.check(&range)?;
        if &self.storage[range.clone()] == text {
            return Ok(None);
        }
        self.storage.replace_range(range.clone(), text);
        self.revision = self.revision.next();
        Ok(Some(TextEdit {
            range,
            inserted_len: text.len(),
            revision: self.revision,
        }))
    }

    pub fn insert(&mut self, at: usize, text: &str) -> Result<Option<TextEdit>, EditError> {
        self.replace(at..at, text)
    }

    pub fn delete(&mut self, range: Range<usize>) -> Result<Option<TextEdit>, EditError> {
        self.replace(range, "")
    }

    /// Replaces the whole text.
    pub fn set_text(&mut self, text: &str) -> Option<TextEdit> {
        self.replace(0..self.len(), text)
            .expect("the whole text is always a valid range")
    }

    pub fn is_char_boundary(&self, offset: usize) -> bool {
        self.storage.is_char_boundary(offset)
    }

    pub fn is_grapheme_boundary(&self, offset: usize) -> bool {
        navigation::is_grapheme_boundary(&self.storage, offset)
    }

    /// See [`navigation::snap_to_grapheme`].
    pub fn snap_to_grapheme(&self, offset: usize, forward: bool) -> usize {
        navigation::snap_to_grapheme(&self.storage, offset, forward)
    }

    /// How many grapheme clusters precede `offset`, after snapping it back
    /// onto a cluster boundary.
    pub fn grapheme_index(&self, offset: usize) -> usize {
        use unicode_segmentation::UnicodeSegmentation;
        let offset = self.snap_to_grapheme(offset, false);
        self.storage[..offset].graphemes(true).count()
    }

    /// Byte offset of the `index`-th grapheme cluster boundary; the text's
    /// length for `index == cluster count`, `None` past it.
    pub fn offset_of_grapheme(&self, index: usize) -> Option<usize> {
        use unicode_segmentation::UnicodeSegmentation;
        self.storage
            .grapheme_indices(true)
            .map(|(offset, _)| offset)
            .chain(std::iter::once(self.len()))
            .nth(index)
    }

    /// UTF-16 code units before `offset` — what platform IME and
    /// accessibility APIs count in — after clamping it onto a char boundary.
    pub fn utf16_offset(&self, offset: usize) -> usize {
        let offset = navigation::clamp_to_char_boundary(&self.storage, offset);
        self.storage[..offset].encode_utf16().count()
    }

    /// Byte offset of a UTF-16 offset, or `None` when it lies past the text or
    /// between the two halves of a surrogate pair.
    pub fn offset_of_utf16(&self, utf16: usize) -> Option<usize> {
        let mut units = 0;
        for (offset, character) in self.storage.char_indices() {
            if units == utf16 {
                return Some(offset);
            }
            units += character.len_utf16();
            if units > utf16 {
                return None;
            }
        }
        (units == utf16).then_some(self.len())
    }

    /// The logical paragraph around `offset`, without its line feed.
    pub fn paragraph_range(&self, offset: usize) -> Range<usize> {
        let (start, end) = navigation::logical_line_range(&self.storage, offset);
        start..end
    }

    fn check(&self, range: &Range<usize>) -> Result<(), EditError> {
        if range.start > range.end || range.end > self.storage.len() {
            return Err(EditError::OutOfBounds);
        }
        if !self.storage.is_char_boundary(range.start) || !self.storage.is_char_boundary(range.end)
        {
            return Err(EditError::NotCharBoundary);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_real_change_bumps_the_revision() {
        let mut text = EditableText::new("hello");
        let start = text.revision();
        assert_eq!(text.replace(0..1, "h"), Ok(None));
        assert_eq!(text.revision(), start);
        let edit = text.replace(0..1, "J").unwrap().unwrap();
        assert_eq!(text.as_str(), "Jello");
        assert_eq!(edit.revision, text.revision());
        assert!(text.revision() > start);
        assert_eq!(text.replace(4..9, ""), Err(EditError::OutOfBounds));
        assert_eq!(
            EditableText::new("中").replace(1..2, ""),
            Err(EditError::NotCharBoundary)
        );
    }

    #[test]
    fn an_edit_maps_old_offsets_into_the_new_text() {
        let mut text = EditableText::new("abcdef");
        let edit = text.replace(2..4, "XYZ").unwrap().unwrap();
        assert_eq!(text.as_str(), "abXYZef");
        assert_eq!(edit.map_offset(1, false), 1);
        assert_eq!(edit.map_offset(2, false), 2);
        assert_eq!(edit.map_offset(2, true), 5);
        assert_eq!(edit.map_offset(3, false), 2);
        assert_eq!(edit.map_offset(3, true), 5);
        assert_eq!(edit.map_offset(4, false), 5);
        assert_eq!(edit.map_offset(6, false), 7);
        assert_eq!(edit.inserted_range(), 2..5);
    }

    #[test]
    fn offsets_convert_between_bytes_graphemes_and_utf16() {
        let text = EditableText::new("a😀e\u{301}中");
        assert_eq!(text.grapheme_index(5), 2);
        assert_eq!(text.grapheme_index(6), 2, "inside e + mark snaps back");
        assert_eq!(text.offset_of_grapheme(2), Some(5));
        assert_eq!(text.offset_of_grapheme(4), Some(text.len()));
        assert_eq!(text.offset_of_grapheme(5), None);
        assert_eq!(text.utf16_offset(5), 3, "the emoji is a surrogate pair");
        assert_eq!(text.offset_of_utf16(3), Some(5));
        assert_eq!(text.offset_of_utf16(2), None, "between two surrogates");
        assert_eq!(text.offset_of_utf16(6), Some(text.len()));
        assert_eq!(text.offset_of_utf16(7), None);
    }
}
