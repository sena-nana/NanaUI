//! IME surrounding-text rules, as plain functions of the committed text so a
//! host that keeps its own storage applies exactly the rules
//! [`EditSession`](super::EditSession) does.

use super::navigation;
use std::ops::Range;

/// What a surrounding-text deletion removes: the bytes before the anchor and
/// the bytes after it, in committed offsets. When the anchor is deleted too the
/// two touch and read as one range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurroundingDeletion {
    pub before: Range<usize>,
    pub after: Range<usize>,
}

impl SurroundingDeletion {
    /// Bytes removed in total.
    pub fn len(&self) -> usize {
        self.before.len() + self.after.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Where an offset of the text before the deletion lands after it.
    /// Offsets inside a deleted range collapse onto its start.
    pub fn map_offset(&self, offset: usize) -> usize {
        let shift = |offset: usize, range: &Range<usize>| {
            if offset <= range.start {
                offset
            } else if offset >= range.end {
                offset - range.len()
            } else {
                range.start
            }
        };
        // `after` lies after `before`: shift through it first, while its
        // offsets still mean the original text.
        shift(shift(offset, &self.after), &self.before)
    }
}

/// The deletion an IME's `delete_surrounding(before, after)` asks for, around
/// `anchor` — the selection, or the range a preedit stands in for.
///
/// While composing the anchor is kept (`keep_anchor`): the preedit is not
/// committed text, and deleting what it replaces would make cancelling the
/// composition lose text. Otherwise the selection goes with the deletion, the
/// contract Runtime editors have always had.
///
/// `None` when either end falls outside the text or splits a character, or
/// nothing would be deleted.
pub fn surrounding_deletion(
    text: &str,
    anchor: Range<usize>,
    before: usize,
    after: usize,
    keep_anchor: bool,
) -> Option<SurroundingDeletion> {
    let start = anchor.start.checked_sub(before)?;
    let end = anchor.end.checked_add(after)?;
    if anchor.start > anchor.end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return None;
    }
    let deletion = if keep_anchor {
        SurroundingDeletion {
            before: start..anchor.start,
            after: anchor.end..end,
        }
    } else {
        SurroundingDeletion {
            before: start..end,
            after: end..end,
        }
    };
    (!deletion.is_empty()).then_some(deletion)
}

/// A window of at most `before` bytes before and `after` bytes after
/// `range`, shrunk onto character boundaries so it never reports half a
/// character. Returns the window; `range` is inside it.
pub fn surrounding_window(
    text: &str,
    range: Range<usize>,
    before: usize,
    after: usize,
) -> Range<usize> {
    let mut start = range.start.min(text.len()).saturating_sub(before);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let end = navigation::clamp_to_char_boundary(text, range.end.saturating_add(after));
    start..end.max(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_anchor_survives_the_deletion_around_it() {
        let text = "hello foo bar";
        let deletion = surrounding_deletion(text, 6..9, 1, 1, true).unwrap();
        assert_eq!(deletion.before, 5..6);
        assert_eq!(deletion.after, 9..10);
        assert_eq!(deletion.map_offset(6), 5);
        assert_eq!(deletion.map_offset(9), 8);
        assert_eq!(deletion.map_offset(13), 11);

        let whole = surrounding_deletion(text, 6..9, 1, 1, false).unwrap();
        assert_eq!(whole.before, 5..10);
        assert!(whole.after.is_empty());
        assert!(surrounding_deletion(text, 6..6, 0, 0, false).is_none());
        assert!(surrounding_deletion(text, 6..9, 7, 0, false).is_none());
        assert!(surrounding_deletion("中", 3..3, 1, 0, false).is_none());
    }

    #[test]
    fn windows_never_split_a_character() {
        let text = "中文abc";
        assert_eq!(
            surrounding_window(text, 6..6, 2, 1),
            6..7,
            "half of 文 is not reported"
        );
        assert_eq!(surrounding_window(text, 6..6, 3, 1), 3..7);
        assert_eq!(surrounding_window(text, 0..0, 5, 100), 0..text.len());
    }
}
