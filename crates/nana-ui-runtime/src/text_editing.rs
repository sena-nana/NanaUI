//! Offset-space editing primitives shared by keyboard, pointer, and IME
//! editing paths.
//!
//! All functions operate on byte offsets of committed [`TextInputState`]
//! values and never mutate directly: callers apply the returned offsets or
//! replacement values through [`crate::TextInputState`] so IME preedit,
//! change events, and validation stay in one place.

use unicode_segmentation::UnicodeSegmentation;

/// Move `offset` to the next grapheme boundary. `None` at the end of value.
pub fn next_grapheme(value: &str, offset: usize) -> Option<usize> {
    if offset >= value.len() || !value.is_char_boundary(offset) {
        return None;
    }
    value[offset..]
        .grapheme_indices(true)
        .nth(1)
        .map(|(index, _)| offset + index)
        .or(Some(value.len()))
}

/// Move `offset` to the previous grapheme boundary. `None` at the start.
pub fn prev_grapheme(value: &str, offset: usize) -> Option<usize> {
    if offset == 0 || !value.is_char_boundary(offset) {
        return None;
    }
    value[..offset]
        .grapheme_indices(true)
        .next_back()
        .map(|(index, _)| index)
}

/// Byte range of the word containing `offset`.
///
/// Word boundaries come from Unicode word segmentation, matching editor
/// double-click behavior. A caret sitting at a word's start selects that
/// word; a caret between segments (after a word, before whitespace) selects
/// the whitespace segment ahead of it.
pub fn word_range_at(value: &str, offset: usize) -> (usize, usize) {
    let offset = clamp_boundary(value, offset);
    let mut ends_here: Option<(usize, usize)> = None;
    for (index, word) in value.split_word_bound_indices() {
        let end = index + word.len();
        if index < offset && offset < end {
            if !word.chars().next().is_some_and(char::is_whitespace) {
                return (index, end);
            }
            // The interior of a whitespace run selects nothing.
            continue;
        }
        if index == offset && end > offset {
            if !word.chars().next().is_some_and(char::is_whitespace) {
                return (index, end);
            }
            // A whitespace segment starting here defers to a word that ends
            // here, so a double click at a word's end selects that word.
            continue;
        }
        if end == offset && !word.chars().all(char::is_whitespace) {
            ends_here = Some((index, end));
        }
    }
    ends_here.unwrap_or((offset, offset))
}

/// Word start at or before `offset`, skipping whitespace segments.
///
/// A caret inside or at the end of a word moves to that word's start; a caret
/// in whitespace moves to the start of the word before it.
pub fn word_start_before(value: &str, offset: usize) -> usize {
    let offset = clamp_boundary(value, offset);
    let mut candidate = 0;
    for (index, word) in value.split_word_bound_indices() {
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

/// Word end at or after `offset`, skipping whitespace segments.
///
/// A caret inside a word moves to that word's end; a caret at a word's end or
/// in whitespace moves to the end of the next word.
pub fn word_end_after(value: &str, offset: usize) -> usize {
    let offset = clamp_boundary(value, offset);
    let mut fallback = offset;
    for (index, word) in value.split_word_bound_indices() {
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

/// Byte range of the logical line containing `offset`; `end` excludes `\n`.
pub fn logical_line_range(value: &str, offset: usize) -> (usize, usize) {
    let offset = clamp_boundary(value, offset);
    let start = value[..offset].rfind('\n').map_or(0, |index| index + 1);
    let end = value[start..]
        .find('\n')
        .map_or(value.len(), |index| start + index);
    (start, end)
}

/// Byte offset of the first non-whitespace character on the logical line
/// containing `offset` (the line end when the line is all whitespace).
pub fn line_content_start(value: &str, offset: usize) -> usize {
    let (start, end) = logical_line_range(value, offset);
    let indent = value[start..end]
        .find(|character: char| character != ' ' && character != '\t')
        .unwrap_or(end - start);
    start + indent
}

/// Clamp onto the nearest char boundary at or below `offset`.
pub fn clamp_boundary(value: &str, offset: usize) -> usize {
    let mut offset = offset.min(value.len());
    while offset > 0 && !value.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Normalize platform line endings to `\n` so multiline edits never inject
/// `\r` characters from platform key text or pasteboards.
pub fn normalize_newlines(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_owned();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Apply a caret move to a selection. Extending keeps the anchor; otherwise
/// both ends collapse onto the new focus.
pub fn moved_selection(
    selection: crate::TextSelection,
    focus: usize,
    extend: bool,
) -> crate::TextSelection {
    if extend {
        crate::TextSelection {
            anchor: selection.anchor,
            focus,
        }
    } else {
        crate::TextSelection::caret(focus)
    }
}

/// Replacement produced by a delete or transform.
pub struct TextReplacement {
    /// Byte range to remove.
    pub range: std::ops::Range<usize>,
    /// Text inserted in place of the range.
    pub insert: String,
    /// Caret offset after applying the replacement (in the new value).
    pub caret: usize,
}

/// Delete backward from the caret: the selection, or one grapheme before it.
pub fn delete_backward(value: &str, selection: crate::TextSelection) -> Option<TextReplacement> {
    let range = selection.ordered();
    if range.start != range.end {
        return Some(TextReplacement {
            range: range.clone(),
            insert: String::new(),
            caret: range.start,
        });
    }
    let start = prev_grapheme(value, range.start)?;
    Some(TextReplacement {
        range: start..range.start,
        insert: String::new(),
        caret: start,
    })
}

/// Delete forward from the caret: the selection, or one grapheme after it.
pub fn delete_forward(value: &str, selection: crate::TextSelection) -> Option<TextReplacement> {
    let range = selection.ordered();
    if range.start != range.end {
        return Some(TextReplacement {
            range: range.clone(),
            insert: String::new(),
            caret: range.start,
        });
    }
    let end = next_grapheme(value, range.start)?;
    Some(TextReplacement {
        range: range.start..end,
        insert: String::new(),
        caret: range.start,
    })
}

/// Delete from the caret back to the start of the previous word.
pub fn delete_word_backward(
    value: &str,
    selection: crate::TextSelection,
) -> Option<TextReplacement> {
    let range = selection.ordered();
    if range.start != range.end {
        return Some(TextReplacement {
            range: range.clone(),
            insert: String::new(),
            caret: range.start,
        });
    }
    let start = word_start_before(value, range.start);
    Some(TextReplacement {
        range: start..range.start,
        insert: String::new(),
        caret: start,
    })
}

/// Delete from the caret forward to the end of the next word.
pub fn delete_word_forward(
    value: &str,
    selection: crate::TextSelection,
) -> Option<TextReplacement> {
    let range = selection.ordered();
    if range.start != range.end {
        return Some(TextReplacement {
            range: range.clone(),
            insert: String::new(),
            caret: range.start,
        });
    }
    let end = word_end_after(value, range.start);
    Some(TextReplacement {
        range: range.start..end,
        insert: String::new(),
        caret: range.start,
    })
}

/// Delete from the caret back to the start of its logical line.
pub fn delete_to_line_start(
    value: &str,
    selection: crate::TextSelection,
) -> Option<TextReplacement> {
    let range = selection.ordered();
    let (line_start, _) = logical_line_range(value, range.start);
    if range.start == range.end && range.start == line_start {
        return None;
    }
    Some(TextReplacement {
        range: line_start..range.end.max(line_start),
        insert: String::new(),
        caret: line_start,
    })
}

/// Delete from the caret forward to the end of its logical line (excluding
/// the newline).
pub fn delete_to_line_end(value: &str, selection: crate::TextSelection) -> Option<TextReplacement> {
    let range = selection.ordered();
    let (_, line_end) = logical_line_range(value, range.start);
    if range.start == range.end && range.end >= line_end {
        return None;
    }
    Some(TextReplacement {
        range: range.start..line_end.max(range.start),
        insert: String::new(),
        caret: range.start,
    })
}

/// Apply a replacement to a value, returning the new value and caret.
pub fn apply_replacement(value: &str, replacement: &TextReplacement) -> (String, usize) {
    let range = replacement.range.start..replacement.range.end.min(value.len());
    let mut next = String::with_capacity(value.len() + replacement.insert.len());
    next.push_str(&value[..range.start]);
    next.push_str(&replacement.insert);
    next.push_str(&value[range.end..]);
    let caret = replacement.caret.min(next.len());
    (next, caret)
}

/// One caret or selection movement step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextCaretIntent {
    Left,
    Right,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    Up,
    Down,
}

/// Resolve a geometry-free caret intent against the moving end of the
/// selection. Vertical intents need layout and are rejected here.
pub fn caret_focus(
    value: &str,
    selection: crate::TextSelection,
    intent: TextCaretIntent,
) -> Option<usize> {
    let focus = clamp_boundary(value, selection.focus);
    match intent {
        TextCaretIntent::Left => prev_grapheme(value, focus),
        TextCaretIntent::Right => next_grapheme(value, focus),
        TextCaretIntent::WordLeft => {
            let start = word_start_before(value, focus);
            (start < focus).then_some(start)
        }
        TextCaretIntent::WordRight => {
            let end = word_end_after(value, focus);
            (end > focus).then_some(end)
        }
        TextCaretIntent::LineStart => {
            let (start, _) = logical_line_range(value, focus);
            (start < focus).then_some(start)
        }
        TextCaretIntent::LineEnd => {
            let (_, end) = logical_line_range(value, focus);
            (end > focus).then_some(end)
        }
        TextCaretIntent::DocStart => (focus > 0).then_some(0),
        TextCaretIntent::DocEnd => (focus < value.len()).then_some(value.len()),
        TextCaretIntent::Up | TextCaretIntent::Down => None,
    }
}

/// Probed visual position of a byte offset: `(x, line_top, line_height)` in
/// content-local pixels. Backed by [`crate::TextShaper::text_position`].
pub type PositionProbe<'a> = &'a mut dyn FnMut(usize) -> (f32, f32, f32);

/// Byte range `(start, end)` of the visual line whose top is `band_y`.
///
/// Offsets are binary searched through the probe, so wrapped backends pay
/// O(log n) probes per query and explicit-line backends match exactly.
fn visual_line_band(
    value: &str,
    band_y: f32,
    line_height: f32,
    position: PositionProbe<'_>,
) -> Option<(usize, usize)> {
    let mut y_of = |offset: usize| position(clamp_boundary(value, offset)).1;
    let mut lower_bound = |threshold: f32| -> usize {
        let (mut low, mut high) = (0usize, value.len());
        while low < high {
            let mid = low + (high - low) / 2;
            if y_of(mid) + f32::EPSILON >= threshold {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        low
    };
    let start = lower_bound(band_y);
    if start >= value.len() {
        return None;
    }
    let end = lower_bound(band_y + line_height).max(next_grapheme(value, start).unwrap_or(start));
    Some((start, end.min(value.len())))
}

/// Resolve a vertical intent onto the visual line above or below the caret,
/// keeping the horizontal goal column in content-local pixels.
///
/// Returns the new selection and the goal column to retain for chained
/// vertical moves. Clamp semantics match desktop editors: moving up from the
/// first visual line goes to the document start, moving down from the last
/// goes to the document end.
pub fn vertical_caret_focus(
    value: &str,
    selection: crate::TextSelection,
    intent: TextCaretIntent,
    extend: bool,
    goal_x: Option<f32>,
    mut position: impl FnMut(usize) -> (f32, f32, f32),
) -> Option<(crate::TextSelection, f32)> {
    let focus = clamp_boundary(value, selection.focus);
    if !matches!(intent, TextCaretIntent::Up | TextCaretIntent::Down) {
        return None;
    }
    let (current_x, current_y, probed_height) = position(focus);
    let line_height = probed_height.max(1.0);
    let goal = goal_x.unwrap_or(current_x);
    let clamp_to = |offset: usize| {
        Some((
            moved_selection(selection, clamp_boundary(value, offset), extend),
            goal,
        ))
    };
    let last_y = position(value.len()).1;
    let band_y = match intent {
        TextCaretIntent::Up => {
            if current_y <= f32::EPSILON {
                return clamp_to(0);
            }
            current_y - line_height
        }
        _ => {
            if current_y + f32::EPSILON >= last_y {
                return clamp_to(value.len());
            }
            current_y + line_height
        }
    };
    let Some((band_start, band_end)) = visual_line_band(value, band_y, line_height, &mut position)
    else {
        return clamp_to(if intent == TextCaretIntent::Up {
            0
        } else {
            value.len()
        });
    };
    let mut best = band_start;
    let mut best_distance = f32::INFINITY;
    let mut cursor = band_start;
    loop {
        let (x, _, _) = position(cursor);
        let distance = (x - goal).abs();
        if distance < best_distance {
            best_distance = distance;
            best = cursor;
        }
        match next_grapheme(value, cursor) {
            Some(next) if next < band_end || band_end == value.len() => cursor = next,
            _ => break,
        }
    }
    Some((moved_selection(selection, best, extend), goal))
}

/// Byte offset whose caret renders closest to a content-local point.
///
/// Clicks below the last visual line land on that line, and clicks past a
/// line's end collapse onto the line end.
pub fn caret_offset_at_point(
    value: &str,
    x: f32,
    y: f32,
    mut position: impl FnMut(usize) -> (f32, f32, f32),
) -> usize {
    if value.is_empty() {
        return 0;
    }
    let (_, _, line_height) = position(0);
    let line_height = line_height.max(1.0);
    let last_y = position(value.len()).1;
    let mut band_y = (y / line_height).floor() * line_height;
    if band_y > last_y {
        band_y = last_y;
    }
    let Some((band_start, band_end)) = visual_line_band(value, band_y, line_height, &mut position)
    else {
        return value.len();
    };
    let mut best = band_start;
    let mut best_distance = f32::INFINITY;
    let mut cursor = band_start;
    loop {
        let (cursor_x, _, _) = position(cursor);
        let distance = (cursor_x - x).abs();
        if distance < best_distance {
            best_distance = distance;
            best = cursor;
        }
        match next_grapheme(value, cursor) {
            Some(next) if next < band_end || band_end == value.len() => cursor = next,
            _ => break,
        }
    }
    best
}

/// Column-free vertical fallback for hosts without a shaper: move across
/// logical lines keeping the grapheme column index.
pub fn vertical_caret_focus_logical(
    value: &str,
    selection: crate::TextSelection,
    intent: TextCaretIntent,
    extend: bool,
) -> Option<crate::TextSelection> {
    let focus = clamp_boundary(value, selection.focus);
    let (line_start, line_end) = logical_line_range(value, focus);
    let column = value[line_start..focus].graphemes(true).count();
    let (target_start, target_end) = match intent {
        TextCaretIntent::Up => {
            if line_start == 0 {
                return Some(moved_selection(selection, 0, extend));
            }
            let previous_end = line_start - 1;
            let start = value[..previous_end]
                .rfind('\n')
                .map_or(0, |index| index + 1);
            (start, previous_end)
        }
        TextCaretIntent::Down => {
            if line_end >= value.len() {
                return Some(moved_selection(selection, value.len(), extend));
            }
            let next_start = line_end + 1;
            let end = value[next_start..]
                .find('\n')
                .map_or(value.len(), |index| next_start + index);
            (next_start, end)
        }
        _ => return None,
    };
    let mut offset = target_start;
    for (index, grapheme) in value[target_start..target_end].grapheme_indices(true) {
        if index >= column {
            break;
        }
        offset = target_start + index + grapheme.len();
    }
    Some(moved_selection(selection, offset, extend))
}

/// Bracket and quote pairs completed by code-editor auto-pairing.
fn auto_pair_close(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

fn is_closer(character: char) -> bool {
    matches!(character, ')' | ']' | '}' | '"' | '\'' | '`')
}

/// Resolve a typed character in code-editor mode.
///
/// Openers wrap the selection or complete a pair when the following character
/// cannot continue a token; typing a closer that is already next skips over
/// it. `None` means the character types normally.
pub fn auto_pair_edit(
    value: &str,
    selection: crate::TextSelection,
    typed: char,
) -> Option<TextReplacement> {
    let range = selection.ordered();
    if let Some(close) = auto_pair_close(typed) {
        if range.start != range.end {
            let selected = &value[range.clone()];
            let mut insert = String::with_capacity(selected.len() + 2);
            insert.push(typed);
            insert.push_str(selected);
            insert.push(close);
            let caret = range.start + 1 + selected.len();
            return Some(TextReplacement {
                range,
                insert,
                caret,
            });
        }
        let continues_token = value[range.end..]
            .chars()
            .next()
            .is_some_and(|next| next.is_alphanumeric() || next == '_');
        if !continues_token {
            let caret = range.start + typed.len_utf8();
            return Some(TextReplacement {
                range,
                insert: format!("{typed}{close}"),
                caret,
            });
        }
        return None;
    }
    if is_closer(typed) && range.start == range.end && value[range.end..].starts_with(typed) {
        // Skip over the existing closer without typing a second one.
        return Some(TextReplacement {
            range: range.start..range.start,
            insert: String::new(),
            caret: range.start + typed.len_utf8(),
        });
    }
    None
}

/// Replacement that starts a new line at the caret, copying indentation and
/// extending it after an open brace.
pub fn auto_indent_newline(
    value: &str,
    selection: crate::TextSelection,
    indent_unit: &str,
) -> TextReplacement {
    let range = selection.ordered();
    let before = &value[..range.start];
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let indent_end = before[line_start..]
        .find(|character: char| character != ' ' && character != '\t')
        .map_or(before.len(), |index| line_start + index);
    let mut insert = String::from("\n");
    insert.push_str(&before[line_start..indent_end]);
    let mut caret = range.start + insert.len();
    let opens_block = before.ends_with('{');
    let closes_block = value[range.end..].starts_with('}');
    if opens_block {
        insert.push_str(indent_unit);
        caret += indent_unit.len();
    }
    if opens_block && closes_block {
        insert.push('\n');
        insert.push_str(&before[line_start..indent_end]);
    }
    TextReplacement {
        range,
        insert,
        caret,
    }
}

/// Toggle `//`-style comments across the lines the selection touches.
///
/// Blank lines are left alone. Returns the next value and the selection
/// remapped onto it, preserving anchor/focus order.
pub fn toggle_line_comment(
    value: &str,
    selection: crate::TextSelection,
    prefix: &str,
) -> Option<(String, crate::TextSelection)> {
    if prefix.is_empty() {
        return None;
    }
    let range = selection.ordered();
    let first_line = logical_line_range(value, range.start).0;
    let (last_start, _) = logical_line_range(value, range.end);
    let last_line = if range.end > range.start && range.end == last_start {
        logical_line_range(value, range.end - 1).0
    } else {
        last_start
    };
    let mut lines = Vec::new();
    let mut cursor = first_line;
    loop {
        lines.push(logical_line_range(value, cursor));
        let (_, line_end) = *lines.last()?;
        if cursor >= last_line || line_end >= value.len() {
            break;
        }
        cursor = line_end + 1;
    }
    let line_is_commented = |&(start, end): &(usize, usize)| {
        let content_start = line_content_start(value, start);
        content_start < end && value[content_start..end].starts_with(prefix)
    };
    let all_commented = lines
        .iter()
        .filter(|&&(start, end)| start < end)
        .all(&line_is_commented);
    let mut edits: Vec<(usize, usize, Option<String>)> = Vec::new();
    for &(start, end) in &lines {
        if start == end {
            continue;
        }
        let content_start = line_content_start(value, start);
        if all_commented {
            if line_is_commented(&(start, end)) {
                edits.push((content_start, content_start + prefix.len(), None));
            }
        } else {
            edits.push((content_start, content_start, Some(prefix.to_owned())));
        }
    }
    if edits.is_empty() {
        return None;
    }
    let mut next = String::with_capacity(value.len());
    let mut output_cursor = 0usize;
    for (edit_start, edit_end, insert) in &edits {
        if output_cursor < *edit_start {
            next.push_str(&value[output_cursor..*edit_start]);
        }
        if let Some(text) = insert {
            next.push_str(text);
        }
        output_cursor = *edit_end;
    }
    if output_cursor < value.len() {
        next.push_str(&value[output_cursor..]);
    }
    let map = |source: usize| -> usize {
        let mut offset = source;
        for (edit_start, edit_end, insert) in &edits {
            if *edit_end <= source {
                offset = match insert {
                    Some(text) => offset + text.len() - (edit_end - edit_start),
                    None => offset - (edit_end - edit_start),
                };
            }
        }
        clamp_boundary(&next, offset)
    };
    let anchor = map(selection.anchor);
    let focus = map(selection.focus);
    Some((next, crate::TextSelection { anchor, focus }))
}

/// Insert one indentation unit at the caret, or indent every touched line
/// when the selection spans text.
pub fn indent_selection(
    value: &str,
    selection: crate::TextSelection,
    indent_unit: &str,
) -> Option<(String, crate::TextSelection)> {
    let range = selection.ordered();
    if range.start == range.end {
        let mut next = String::with_capacity(value.len() + indent_unit.len());
        next.push_str(&value[..range.start]);
        next.push_str(indent_unit);
        next.push_str(&value[range.start..]);
        let caret = range.start + indent_unit.len();
        return Some((
            next,
            crate::TextSelection {
                anchor: caret,
                focus: caret,
            },
        ));
    }
    shift_selected_lines(value, selection, indent_unit, 1)
}

/// Remove one indentation unit from every touched line.
pub fn outdent_selection(
    value: &str,
    selection: crate::TextSelection,
    indent_unit: &str,
) -> Option<(String, crate::TextSelection)> {
    shift_selected_lines(value, selection, indent_unit, -1)
}

fn shift_selected_lines(
    value: &str,
    selection: crate::TextSelection,
    indent_unit: &str,
    direction: i32,
) -> Option<(String, crate::TextSelection)> {
    let range = selection.ordered();
    let first_line = logical_line_range(value, range.start).0;
    let (last_start, _) = logical_line_range(value, range.end);
    let last_line = if range.end > range.start && range.end == last_start {
        logical_line_range(value, range.end - 1).0
    } else {
        last_start
    };
    let mut next = String::with_capacity(value.len());
    let mut output_cursor = 0usize;
    let mut deltas: Vec<(usize, isize)> = Vec::new();
    let mut cursor = first_line;
    loop {
        let (line_start, line_end) = logical_line_range(value, cursor);
        if output_cursor < line_start {
            next.push_str(&value[output_cursor..line_start]);
        }
        if line_start != line_end {
            if direction > 0 {
                next.push_str(indent_unit);
                deltas.push((line_start, indent_unit.len() as isize));
                next.push_str(&value[line_start..line_end]);
            } else {
                let content_start = line_content_start(value, line_start);
                let removed = remove_indent(&value[line_start..content_start], indent_unit);
                if removed > 0 {
                    deltas.push((line_start, -(removed as isize)));
                }
                next.push_str(&value[line_start + removed..line_end]);
            }
        }
        output_cursor = line_end;
        if cursor >= last_line || line_end >= value.len() {
            break;
        }
        cursor = line_end + 1;
    }
    if output_cursor < value.len() {
        next.push_str(&value[output_cursor..]);
    }
    if deltas.is_empty() {
        return None;
    }
    let map = |source: usize| -> usize {
        let mut offset = source as isize;
        for &(edit_start, delta) in &deltas {
            if edit_start <= source {
                offset += delta;
            }
        }
        clamp_boundary(&next, offset.max(0) as usize)
    };
    let anchor = map(selection.anchor);
    let focus = map(selection.focus);
    Some((next, crate::TextSelection { anchor, focus }))
}

fn remove_indent(text: &str, indent_unit: &str) -> usize {
    if indent_unit.starts_with('\t') {
        return text.chars().filter(|&character| character == '\t').count();
    }
    let unit = indent_unit
        .chars()
        .filter(|&character| character == ' ')
        .count()
        .max(1);
    let spaces = text
        .chars()
        .take_while(|&character| character == ' ')
        .count();
    (spaces / unit) * unit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grapheme_steps_never_split_clusters() {
        let value = "a👩‍💻b";
        assert_eq!(prev_grapheme(value, value.len()), Some("a👩‍💻".len()));
        assert_eq!(next_grapheme(value, 0), Some(1));
        assert_eq!(next_grapheme(value, value.len()), None);
        assert_eq!(prev_grapheme(value, 0), None);
    }

    #[test]
    fn word_boundaries_follow_unicode_words() {
        let value = "first second\nthird";
        // "first"(0,5) " "(5,6) "second"(6,12) "\n"(12,13) "third"(13,18)
        assert_eq!(word_range_at(value, 2), (0, 5));
        // A caret parked at a word's end still selects that word.
        assert_eq!(word_range_at(value, 12), (6, 12));
        assert_eq!(word_range_at(value, value.len()), (13, 18));
        // A word start selects the word ahead of it.
        assert_eq!(word_range_at("hello world", 6), (6, 11));
        // A caret between whitespace segments selects nothing.
        assert_eq!(word_range_at("ab  cd", 3), (3, 3));
        assert_eq!(word_start_before(value, 2), 0);
        assert_eq!(word_start_before(value, 6), 0);
        assert_eq!(word_end_after(value, 6), 12);
        assert_eq!(word_start_before(value, 12), 6);
        assert_eq!(word_end_after(value, 12), 18);
        assert_eq!(word_start_before(value, 0), 0);
        assert_eq!(word_end_after(value, value.len()), value.len());
    }

    #[test]
    fn logical_lines_exclude_the_newline() {
        let value = "one\ntwo\n";
        assert_eq!(logical_line_range(value, 1), (0, 3));
        assert_eq!(logical_line_range(value, 4), (4, 7));
        // The offset on the terminating newline still belongs to line two.
        assert_eq!(logical_line_range(value, 7), (4, 7));
        assert_eq!(logical_line_range(value, 8), (8, 8));
        assert_eq!(line_content_start(value, 5), 4);
    }

    #[test]
    fn deletes_move_the_caret_to_the_edit_point() {
        let value = "abc def";
        let caret = crate::TextSelection::caret(3);
        let backward = delete_backward(value, caret).unwrap();
        assert_eq!(apply_replacement(value, &backward), ("ab def".into(), 2));
        let forward = delete_forward(value, caret).unwrap();
        assert_eq!(apply_replacement(value, &forward), ("abcdef".into(), 3));
        let word = delete_word_backward(value, caret).unwrap();
        assert_eq!(apply_replacement(value, &word), (" def".into(), 0));
        let word_forward = delete_word_forward(value, caret).unwrap();
        assert_eq!(apply_replacement(value, &word_forward), ("abc".into(), 3));
        let to_end = delete_to_line_end(value, caret).unwrap();
        assert_eq!(apply_replacement(value, &to_end), ("abc".into(), 3));
        let to_start = delete_to_line_start(value, caret).unwrap();
        assert_eq!(apply_replacement(value, &to_start), (" def".into(), 0));
    }

    #[test]
    fn selection_delete_collapses_onto_the_range_start() {
        let value = "abcdef";
        let selection = crate::TextSelection {
            anchor: 2,
            focus: 5,
        };
        let replacement = delete_backward(value, selection).unwrap();
        assert_eq!(apply_replacement(value, &replacement), ("abf".into(), 2));
    }

    #[test]
    fn newlines_normalize_to_lf() {
        assert_eq!(normalize_newlines("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(normalize_newlines("abc"), "abc");
    }

    #[test]
    fn moved_selection_extends_or_collapses() {
        let selection = crate::TextSelection {
            anchor: 2,
            focus: 5,
        };
        assert_eq!(
            moved_selection(selection, 8, true),
            crate::TextSelection {
                anchor: 2,
                focus: 8
            }
        );
        assert_eq!(
            moved_selection(selection, 1, false),
            crate::TextSelection::caret(1)
        );
    }

    #[test]
    fn horizontal_intents_stop_at_boundaries() {
        let value = "ab cd";
        let caret = crate::TextSelection::caret(3);
        assert_eq!(caret_focus(value, caret, TextCaretIntent::Left), Some(2));
        assert_eq!(caret_focus(value, caret, TextCaretIntent::Right), Some(4));
        assert_eq!(
            caret_focus(value, caret, TextCaretIntent::WordLeft),
            Some(0)
        );
        assert_eq!(caret_focus(value, caret, TextCaretIntent::LineEnd), Some(5));
        assert_eq!(
            caret_focus(value, caret, TextCaretIntent::LineStart),
            Some(0)
        );
        assert_eq!(
            caret_focus(value, crate::TextSelection::caret(0), TextCaretIntent::Left),
            None
        );
    }

    #[test]
    fn vertical_logical_moves_keep_the_grapheme_column() {
        let value = "abcd\nef\nghijk";
        let caret = crate::TextSelection::caret(2);
        let down =
            vertical_caret_focus_logical(value, caret, TextCaretIntent::Down, false).unwrap();
        // Column 2 clamps to the end of "ef".
        assert_eq!(down.focus, 7);
        let down_again =
            vertical_caret_focus_logical(value, down, TextCaretIntent::Down, false).unwrap();
        assert_eq!(down_again.focus, 10);
        let up =
            vertical_caret_focus_logical(value, down_again, TextCaretIntent::Up, false).unwrap();
        assert_eq!(up.focus, 7);
        let top = vertical_caret_focus_logical(value, caret, TextCaretIntent::Up, false).unwrap();
        assert_eq!(top.focus, 0);
        let bottom =
            vertical_caret_focus_logical(value, caret, TextCaretIntent::Down, false).is_some();
        assert!(bottom);
    }

    /// Uniform-width probe: column index × 10px, line top = line index × 20px.
    fn monospace_probe(value: &str) -> impl FnMut(usize) -> (f32, f32, f32) + '_ {
        move |offset: usize| {
            let offset = clamp_boundary(value, offset);
            let line_index = value[..offset].matches('\n').count();
            let line_start = if offset == 0 {
                0
            } else {
                value[..offset].rfind('\n').map_or(0, |index| index + 1)
            };
            let column = value[line_start..offset].chars().count();
            (column as f32 * 10.0, line_index as f32 * 20.0, 20.0)
        }
    }

    #[test]
    fn vertical_geometry_moves_between_visual_lines() {
        let value = "abcd\nef\nghijk";
        let caret = crate::TextSelection::caret(2);
        let (down, goal) = vertical_caret_focus(
            value,
            caret,
            TextCaretIntent::Down,
            false,
            None,
            monospace_probe(value),
        )
        .unwrap();
        // Goal 20px is past "ef"; the caret parks on the line end.
        assert_eq!(down.focus, 7);
        assert_eq!(goal, 20.0);
        let (down_again, _) = vertical_caret_focus(
            value,
            down,
            TextCaretIntent::Down,
            false,
            Some(goal),
            monospace_probe(value),
        )
        .unwrap();
        // Goal column 20px lands on the third grapheme of "ghijk".
        assert_eq!(down_again.focus, 10);
        let (up, _) = vertical_caret_focus(
            value,
            caret,
            TextCaretIntent::Up,
            false,
            None,
            monospace_probe(value),
        )
        .unwrap();
        // Up from the first line clamps to the document start.
        assert_eq!(up.focus, 0);
    }

    #[test]
    fn point_lookup_resolves_lines_and_columns() {
        let value = "abcd\nef\nghijk";
        let probe = monospace_probe(value);
        assert_eq!(caret_offset_at_point(value, 15.0, 5.0, probe), 1);
        let probe = monospace_probe(value);
        assert_eq!(caret_offset_at_point(value, 15.0, 25.0, probe), 6);
        let probe = monospace_probe(value);
        // Past the line end collapses onto the line end.
        assert_eq!(caret_offset_at_point(value, 95.0, 25.0, probe), 7);
        let probe = monospace_probe(value);
        // Below the last line lands on the last line.
        assert_eq!(caret_offset_at_point(value, 5.0, 95.0, probe), 8);
    }

    #[test]
    fn auto_pair_wraps_selection_and_skips_closers() {
        let value = "fn()";
        // Typing '(' before ')' completes the pair, nesting like desktop
        // editors.
        let caret = crate::TextSelection::caret(3);
        let pair = auto_pair_edit(value, caret, '(').unwrap();
        assert_eq!(apply_replacement(value, &pair), ("fn(())".into(), 4));
        // Typing ')' when ')' is next skips over it.
        let replacement = auto_pair_edit(value, caret, ')').unwrap();
        assert_eq!(apply_replacement(value, &replacement), ("fn()".into(), 4));
        // Wrapping a selection.
        let value = "x + y";
        let selection = crate::TextSelection {
            anchor: 0,
            focus: 5,
        };
        let wrap = auto_pair_edit(value, selection, '(').unwrap();
        assert_eq!(apply_replacement(value, &wrap), ("(x + y)".into(), 6));
        // Next to an identifier no pair completes.
        let value = "ab";
        assert!(auto_pair_edit(value, crate::TextSelection::caret(1), '(').is_none());
    }

    #[test]
    fn auto_indent_copies_indentation_and_extends_blocks() {
        // Enter right after '{' deepens one level.
        let value = "{x}";
        let caret = crate::TextSelection::caret(1);
        let replacement = auto_indent_newline(value, caret, "\t");
        let (next, _) = apply_replacement(value, &replacement);
        assert_eq!(next, "{\n\tx}");
        assert_eq!(replacement.caret, 3);
        // Enter between braces opens a middle line at the deeper level and
        // leaves the closer on the original indentation.
        let value = "{}";
        let caret = crate::TextSelection::caret(1);
        let replacement = auto_indent_newline(value, caret, "\t");
        let (next, _) = apply_replacement(value, &replacement);
        assert_eq!(next, "{\n\t\n}");
        assert_eq!(replacement.caret, 3);
        // Enter mid-line copies the line's leading whitespace.
        let value = "  abc";
        let caret = crate::TextSelection::caret(4);
        let replacement = auto_indent_newline(value, caret, "  ");
        let (next, _) = apply_replacement(value, &replacement);
        assert_eq!(next, "  ab\n  c");
        // Enter at the end of an indented line copies that indentation.
        let value = "{\n  x: 1\n}";
        let caret = crate::TextSelection::caret(8);
        let replacement = auto_indent_newline(value, caret, "  ");
        let (next, _) = apply_replacement(value, &replacement);
        assert_eq!(next, "{\n  x: 1\n  \n}");
        assert_eq!(replacement.caret, 11);
    }

    #[test]
    fn comment_toggle_inserts_and_removes() {
        let value = "let a = 1;\nlet b = 2;";
        let selection = crate::TextSelection {
            anchor: 4,
            focus: 17,
        };
        let (commented, commented_selection) = toggle_line_comment(value, selection, "//").unwrap();
        assert_eq!(commented, "//let a = 1;\n//let b = 2;");
        assert_eq!(commented_selection.anchor, 6);
        assert_eq!(commented_selection.focus, 21);
        let (restored, _) = toggle_line_comment(&commented, commented_selection, "//").unwrap();
        assert_eq!(restored, value);
    }

    #[test]
    fn comment_toggle_skips_blank_lines() {
        let value = "a\n\nb";
        let selection = crate::TextSelection {
            anchor: 0,
            focus: 4,
        };
        let (commented, _) = toggle_line_comment(value, selection, "//").unwrap();
        assert_eq!(commented, "//a\n\n//b");
    }

    #[test]
    fn indent_shifts_lines_and_outdent_restores() {
        let value = "a\nb\nc";
        let selection = crate::TextSelection {
            anchor: 0,
            focus: 4,
        };
        let (indented, indented_selection) = indent_selection(value, selection, "  ").unwrap();
        assert_eq!(indented, "  a\n  b\nc");
        assert_eq!(indented_selection.focus, 8);
        let shifted = crate::TextSelection {
            anchor: indented_selection.anchor,
            focus: indented_selection.focus,
        };
        let (restored, _) = outdent_selection(&indented, shifted, "  ").unwrap();
        assert_eq!(restored, value);
        // Caret indent inserts the unit at the caret.
        let (inserted, inserted_selection) =
            indent_selection(value, crate::TextSelection::caret(1), "  ").unwrap();
        assert_eq!(inserted, "a  \nb\nc");
        assert_eq!(inserted_selection.focus, 3);
    }
}
