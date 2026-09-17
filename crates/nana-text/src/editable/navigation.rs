//! Offset navigation over UTF-8 text: graphemes, words and logical lines.
//!
//! Pure functions of `&str` and a byte offset. None of them moves by a byte:
//! every answer is a grapheme cluster boundary (UAX #29), so a caret can never
//! land inside an emoji ZWJ sequence, a base + combining mark, or a Hangul
//! syllable spelled in jamo.
//!
//! Word and grapheme scans are bounded to the logical line around the offset.
//! UAX #29 always breaks after a line feed (WB3a / GB5), so a scan that starts
//! just after one agrees with a scan from the start of the text, and moving a
//! word in a large document costs the line rather than the document.

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// Clamps `offset` onto the nearest char boundary at or below it.
pub fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Whether `offset` is a grapheme cluster boundary of `text`. Both ends of the
/// text are.
pub fn is_grapheme_boundary(text: &str, offset: usize) -> bool {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return false;
    }
    if offset == 0 || offset == text.len() {
        return true;
    }
    let (start, window) = line_window_inclusive(text, offset);
    GraphemeCursor::new(offset - start, window.len(), true)
        .is_boundary(window, 0)
        .unwrap_or(false)
}

/// The grapheme boundary after `offset`, or `None` at the end of the text or
/// on a non-char boundary.
pub fn next_grapheme(text: &str, offset: usize) -> Option<usize> {
    if offset >= text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let (start, window) = line_window_inclusive(text, offset);
    GraphemeCursor::new(offset - start, window.len(), true)
        .next_boundary(window, 0)
        .ok()
        .flatten()
        .map(|next| start + next)
}

/// The grapheme boundary before `offset`, or `None` at the start of the text
/// or on a non-char boundary.
pub fn prev_grapheme(text: &str, offset: usize) -> Option<usize> {
    if offset == 0 || offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    // The window has to hold the cluster that ends at `offset`, and a line
    // feed at `offset - 1` ends the line before the one `offset` opens.
    let (start, window) = line_window_inclusive(text, offset - 1);
    GraphemeCursor::new(offset - start, window.len(), true)
        .prev_boundary(window, 0)
        .ok()
        .flatten()
        .map(|previous| start + previous)
}

/// Snaps `offset` onto a grapheme boundary: the one at or before it
/// (`forward == false`) or at or after it.
pub fn snap_to_grapheme(text: &str, offset: usize, forward: bool) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    if is_grapheme_boundary(text, offset) {
        return offset;
    }
    if forward {
        next_grapheme(text, offset).unwrap_or(text.len())
    } else {
        prev_grapheme(text, offset).unwrap_or(0)
    }
}

/// Byte range of the logical line containing `offset`; the end excludes the
/// line feed.
pub fn logical_line_range(text: &str, offset: usize) -> (usize, usize) {
    let offset = clamp_to_char_boundary(text, offset);
    let start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
    let end = text[start..]
        .find('\n')
        .map_or(text.len(), |index| start + index);
    (start, end)
}

/// Byte range of the word containing `offset`, as a double click selects it.
///
/// A caret at a word's start selects that word; a caret between a word's end
/// and whitespace selects the word it ends; inside whitespace nothing is
/// selected.
pub fn word_range_at(text: &str, offset: usize) -> (usize, usize) {
    let offset = clamp_to_char_boundary(text, offset);
    let (start, window) = line_window_inclusive(text, offset);
    let mut ends_here: Option<(usize, usize)> = None;
    for (index, word) in window.split_word_bound_indices() {
        let index = start + index;
        let end = index + word.len();
        let whitespace = word.chars().next().is_some_and(char::is_whitespace);
        if index < offset && offset < end {
            if !whitespace {
                return (index, end);
            }
            continue;
        }
        if index == offset && end > offset {
            if !whitespace {
                return (index, end);
            }
            continue;
        }
        if end == offset && !word.chars().all(char::is_whitespace) {
            ends_here = Some((index, end));
        }
    }
    ends_here.unwrap_or((offset, offset))
}

/// Start of the word at or before `offset`, skipping whitespace and line
/// breaks. Inside or at the end of a word it is that word's start.
pub fn word_start_before(text: &str, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    let mut line_start = logical_line_range(text, offset).0;
    loop {
        // The whole line is segmented, not the prefix up to `offset`: a word
        // like `one.two` only stays one word while the segmenter can see past
        // the `.`.
        let (_, window) = line_window_inclusive(text, line_start);
        let mut candidate = None;
        for (index, word) in window.split_word_bound_indices() {
            let index = line_start + index;
            if index >= offset {
                break;
            }
            if word.chars().all(char::is_whitespace) {
                continue;
            }
            if offset <= index + word.len() {
                return index;
            }
            candidate = Some(index);
        }
        if let Some(candidate) = candidate {
            return candidate;
        }
        if line_start == 0 {
            return 0;
        }
        line_start = logical_line_range(text, line_start - 1).0;
    }
}

/// End of the word at or after `offset`, skipping whitespace and line breaks.
/// Inside a word it is that word's end; at a word's end it is the next one's.
pub fn word_end_after(text: &str, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    let mut fallback = offset;
    let mut line_start = logical_line_range(text, offset).0;
    loop {
        let (_, window) = line_window_inclusive(text, line_start);
        for (index, word) in window.split_word_bound_indices() {
            let end = line_start + index + word.len();
            if end <= offset {
                continue;
            }
            if word.chars().all(char::is_whitespace) {
                fallback = end;
                continue;
            }
            return end;
        }
        let next = line_start + window.len();
        if next >= text.len() {
            return fallback;
        }
        line_start = next;
    }
}

/// The logical line around `offset` with its line feed, and where it starts.
///
/// The line feed stays in: `CR LF` is one grapheme cluster, and word
/// segmentation has to see the break as the whitespace segment a whole-text
/// scan would.
fn line_window_inclusive(text: &str, offset: usize) -> (usize, &str) {
    let (start, end) = logical_line_range(text, offset);
    let end = if end < text.len() { end + 1 } else { end };
    (start, &text[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The unbounded reference: segment the whole text.
    fn whole_next(text: &str, offset: usize) -> Option<usize> {
        if offset >= text.len() || !text.is_char_boundary(offset) {
            return None;
        }
        text.grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .find(|boundary| *boundary > offset)
    }

    fn whole_word_start(text: &str, offset: usize) -> usize {
        let mut candidate = 0;
        for (index, word) in text.split_word_bound_indices() {
            let end = index + word.len();
            if index >= offset {
                break;
            }
            if word.chars().all(char::is_whitespace) {
                continue;
            }
            if offset <= end {
                return index;
            }
            candidate = index;
        }
        candidate
    }

    fn whole_word_end(text: &str, offset: usize) -> usize {
        let mut fallback = offset;
        for (index, word) in text.split_word_bound_indices() {
            let end = index + word.len();
            if end <= offset {
                continue;
            }
            if word.chars().all(char::is_whitespace) {
                fallback = end;
                continue;
            }
            return end;
        }
        fallback
    }

    const SAMPLES: &[&str] = &[
        "",
        "hello world",
        "  lead\n\ntrail  ",
        "e\u{301}\u{302}x\n👩‍👩‍👧 end",
        "a\r\nb\rc\n",
        "中文 输入\n한국어 입력",
        "مرحبا hello عالم",
        "one.two, three\n\n\nfour",
    ];

    #[test]
    fn line_bounded_scans_agree_with_whole_text_segmentation() {
        for text in SAMPLES {
            for offset in (0..=text.len()).filter(|offset| text.is_char_boundary(*offset)) {
                assert_eq!(
                    next_grapheme(text, offset),
                    whole_next(text, offset),
                    "next grapheme of {text:?} at {offset}"
                );
                assert_eq!(
                    word_start_before(text, offset),
                    whole_word_start(text, offset),
                    "word start of {text:?} at {offset}"
                );
                assert_eq!(
                    word_end_after(text, offset),
                    whole_word_end(text, offset),
                    "word end of {text:?} at {offset}"
                );
                let boundary = text
                    .grapheme_indices(true)
                    .any(|(index, _)| index == offset)
                    || offset == text.len();
                assert_eq!(
                    is_grapheme_boundary(text, offset),
                    boundary,
                    "{text:?} {offset}"
                );
                let whole_previous = text
                    .grapheme_indices(true)
                    .map(|(index, _)| index)
                    .take_while(|index| *index < offset)
                    .last();
                assert_eq!(
                    prev_grapheme(text, offset),
                    whole_previous.filter(|_| offset > 0),
                    "previous grapheme of {text:?} at {offset}"
                );
            }
        }
    }

    #[test]
    fn grapheme_steps_never_split_a_cluster() {
        let text = "e\u{301}👍🏽";
        assert_eq!(next_grapheme(text, 0), Some(3));
        assert_eq!(next_grapheme(text, 3), Some(text.len()));
        assert_eq!(prev_grapheme(text, text.len()), Some(3));
        assert_eq!(prev_grapheme(text, 3), Some(0));
        assert_eq!(snap_to_grapheme(text, 1, false), 0);
        assert_eq!(snap_to_grapheme(text, 1, true), 3);
        assert_eq!(
            prev_grapheme("a\nb", 2),
            Some(1),
            "a line feed is its own cluster"
        );
        assert_eq!(prev_grapheme("a\r\nb", 3), Some(1), "CR LF is one cluster");
    }

    #[test]
    fn word_ranges_select_words_not_whitespace() {
        let text = "hello  world";
        assert_eq!(word_range_at(text, 0), (0, 5));
        assert_eq!(word_range_at(text, 5), (0, 5));
        assert_eq!(word_range_at(text, 6), (6, 6));
        assert_eq!(word_range_at(text, 7), (7, 12));
        assert_eq!(word_range_at("a\nbc", 2), (2, 4));
    }
}
