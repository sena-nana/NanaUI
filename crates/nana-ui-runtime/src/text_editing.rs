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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// One viewport-height up; resolved by [`page_caret_focus`] (or the
    /// logical fallback [`page_caret_focus_logical`]).
    PageUp,
    /// One viewport-height down; see [`TextCaretIntent::PageUp`].
    PageDown,
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
        TextCaretIntent::PageUp | TextCaretIntent::PageDown => None,
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

/// Pick the offset on the visual band starting at `band_y` whose caret x is
/// closest to `goal`, retaining the goal column. `fallback` is used when the
/// backend reports no band at that y (clamped document edges).
fn select_in_visual_band(
    value: &str,
    selection: crate::TextSelection,
    extend: bool,
    goal: f32,
    band_y: f32,
    line_height: f32,
    fallback: usize,
    mut position: impl FnMut(usize) -> (f32, f32, f32),
) -> (crate::TextSelection, f32) {
    let Some((band_start, band_end)) = visual_line_band(value, band_y, line_height, &mut position)
    else {
        return (
            moved_selection(selection, clamp_boundary(value, fallback), extend),
            goal,
        );
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
    (moved_selection(selection, best, extend), goal)
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
    let last_y = position(value.len()).1;
    let (band_y, fallback) = match intent {
        TextCaretIntent::Up => {
            if current_y <= f32::EPSILON {
                return Some((moved_selection(selection, 0, extend), goal));
            }
            (current_y - line_height, 0)
        }
        _ => {
            if current_y + f32::EPSILON >= last_y {
                return Some((moved_selection(selection, value.len(), extend), goal));
            }
            (current_y + line_height, value.len())
        }
    };
    Some(select_in_visual_band(
        value,
        selection,
        extend,
        goal,
        band_y,
        line_height,
        fallback,
        &mut position,
    ))
}

/// Logical lines paged by [`page_caret_focus_logical`].
pub const TEXT_EDITOR_PAGE_LOGICAL_LINES: usize = 15;

/// Resolve a PageUp/PageDown intent against the editor's viewport: the caret
/// moves by `page_height` pixels of visual lines (the content-box height),
/// keeping the horizontal goal column exactly like [`vertical_caret_focus`].
/// Clamps at the document edges.
pub fn page_caret_focus(
    value: &str,
    selection: crate::TextSelection,
    intent: TextCaretIntent,
    extend: bool,
    goal_x: Option<f32>,
    page_height: f32,
    mut position: impl FnMut(usize) -> (f32, f32, f32),
) -> Option<(crate::TextSelection, f32)> {
    let focus = clamp_boundary(value, selection.focus);
    if !matches!(intent, TextCaretIntent::PageUp | TextCaretIntent::PageDown) {
        return None;
    }
    let (current_x, current_y, probed_height) = position(focus);
    let line_height = probed_height.max(1.0);
    let goal = goal_x.unwrap_or(current_x);
    let page = page_height.max(line_height);
    let last_y = position(value.len()).1;
    let (band_y, fallback) = match intent {
        TextCaretIntent::PageUp => {
            if current_y <= f32::EPSILON {
                return Some((moved_selection(selection, 0, extend), goal));
            }
            ((current_y - page).max(0.0), 0)
        }
        _ => {
            if current_y + f32::EPSILON >= last_y {
                return Some((moved_selection(selection, value.len(), extend), goal));
            }
            ((current_y + page).min(last_y), value.len())
        }
    };
    Some(select_in_visual_band(
        value,
        selection,
        extend,
        goal,
        band_y,
        line_height,
        fallback,
        &mut position,
    ))
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

/// Geometry-free PageUp/PageDown fallback for hosts without a shaper:
/// [`vertical_caret_focus_logical`] applied
/// [`TEXT_EDITOR_PAGE_LOGICAL_LINES`] times.
pub fn page_caret_focus_logical(
    value: &str,
    selection: crate::TextSelection,
    intent: TextCaretIntent,
    extend: bool,
) -> Option<crate::TextSelection> {
    let direction = match intent {
        TextCaretIntent::PageUp => TextCaretIntent::Up,
        TextCaretIntent::PageDown => TextCaretIntent::Down,
        _ => return None,
    };
    let mut current = selection;
    for _ in 0..TEXT_EDITOR_PAGE_LOGICAL_LINES {
        current = vertical_caret_focus_logical(value, current, direction, extend)?;
    }
    Some(current)
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

/// Direction for [`move_lines`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextLineDirection {
    Up,
    Down,
}

/// The logical lines a selection touches as `(start, end)` byte ranges
/// (`end` excludes the newline), using the same convention as
/// [`toggle_line_comment`]: a range that ends exactly at a line start does
/// not touch that line.
fn selected_line_ranges(value: &str, range: std::ops::Range<usize>) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let first_line = logical_line_range(value, range.start).0;
    let (last_start, _) = logical_line_range(value, range.end);
    let last_line = if range.end > range.start && range.end == last_start {
        logical_line_range(value, range.end - 1).0
    } else {
        last_start
    };
    let mut cursor = first_line;
    loop {
        let (line_start, line_end) = logical_line_range(value, cursor);
        ranges.push((line_start, line_end));
        if cursor >= last_line || line_end >= value.len() {
            break;
        }
        cursor = line_end + 1;
    }
    ranges
}

/// Move the block of lines the selection touches up or down one line.
///
/// The selection follows the moved text (anchor/focus order preserved).
/// Returns `None` when the block is already at the document edge in that
/// direction.
pub fn move_lines(
    value: &str,
    selection: crate::TextSelection,
    direction: TextLineDirection,
) -> Option<(String, crate::TextSelection)> {
    let lines = selected_line_ranges(value, selection.ordered());
    let block_start = lines.first()?.0;
    let block_end = lines.last()?.1;
    let (next, delta) = match direction {
        TextLineDirection::Up => {
            if block_start == 0 {
                return None;
            }
            // The newline above the block swaps places with the block.
            let previous_end = block_start - 1;
            let previous_start = value[..previous_end]
                .rfind('\n')
                .map_or(0, |index| index + 1);
            let mut next = String::with_capacity(value.len());
            next.push_str(&value[..previous_start]);
            next.push_str(&value[block_start..block_end]);
            next.push('\n');
            next.push_str(&value[previous_start..previous_end]);
            next.push_str(&value[block_end..]);
            let delta = previous_start as isize - block_start as isize;
            (next, delta)
        }
        TextLineDirection::Down => {
            if block_end >= value.len() {
                return None;
            }
            // The newline after the block swaps places with the next line.
            let next_start = block_end + 1;
            let next_end = value[next_start..]
                .find('\n')
                .map_or(value.len(), |index| next_start + index);
            let mut next = String::with_capacity(value.len());
            next.push_str(&value[..block_start]);
            next.push_str(&value[next_start..next_end]);
            next.push('\n');
            next.push_str(&value[block_start..block_end]);
            next.push_str(&value[next_end..]);
            let delta = next_end as isize - block_end as isize;
            (next, delta)
        }
    };
    let map = |offset: usize| clamp_boundary(&next, (offset as isize + delta).max(0) as usize);
    let anchor = map(selection.anchor);
    let focus = map(selection.focus);
    Some((next, crate::TextSelection { anchor, focus }))
}

/// Copy the block of lines the selection touches and insert the copy on the
/// line below, returning a selection that covers the copy (Zed-style
/// duplicate). Always succeeds on a non-empty value; an empty value has no
/// line to duplicate.
pub fn duplicate_lines(
    value: &str,
    selection: crate::TextSelection,
) -> Option<(String, crate::TextSelection)> {
    if value.is_empty() {
        return None;
    }
    let lines = selected_line_ranges(value, selection.ordered());
    let block_start = lines.first()?.0;
    let block_end = lines.last()?.1;
    let mut next = String::with_capacity(value.len() + (block_end - block_start) + 1);
    next.push_str(&value[..block_end]);
    next.push('\n');
    next.push_str(&value[block_start..block_end]);
    next.push_str(&value[block_end..]);
    let delta = (block_end + 1 - block_start) as isize;
    let map = |offset: usize| clamp_boundary(&next, (offset as isize + delta).max(0) as usize);
    let anchor = map(selection.anchor);
    let focus = map(selection.focus);
    Some((next, crate::TextSelection { anchor, focus }))
}

/// Delete the block of lines the selection touches, including one adjacent
/// newline so the surrounding lines join cleanly. The caret lands where the
/// block started (on the previous line's end when the block reached the
/// document end).
pub fn delete_lines(
    value: &str,
    selection: crate::TextSelection,
) -> Option<(String, crate::TextSelection)> {
    if value.is_empty() {
        return None;
    }
    let lines = selected_line_ranges(value, selection.ordered());
    let block_start = lines.first()?.0;
    let block_end = lines.last()?.1;
    let (remove_start, remove_end, caret) = if block_end < value.len() {
        (block_start, block_end + 1, block_start)
    } else if block_start > 0 {
        (block_start - 1, block_end, block_start - 1)
    } else {
        (block_start, block_end, block_start)
    };
    let next = format!("{}{}", &value[..remove_start], &value[remove_end..]);
    let caret = caret.min(next.len());
    Some((next, crate::TextSelection::caret(caret)))
}

/// Join the lines the selection touches into one line.
///
/// With a bare caret this joins the caret line with the following one, like
/// desktop editors. Line breaks and the following lines' leading whitespace
/// are removed; each seam receives a single space unless the left side
/// already ends with whitespace or either joined content is empty. The
/// selection maps onto the joined line (offsets inside removed indentation
/// collapse onto the seam). Returns `None` when there is nothing to join (a
/// single touched line, or the caret on the last line).
pub fn join_lines(
    value: &str,
    selection: crate::TextSelection,
) -> Option<(String, crate::TextSelection)> {
    let range = selection.ordered();
    let lines = if range.start == range.end {
        // 裸光标：与桌面编辑器一致，把光标行和下一行合并。
        let (line_start, line_end) = logical_line_range(value, range.start);
        if line_end >= value.len() {
            return None;
        }
        let next_start = line_end + 1;
        let next_end = value[next_start..]
            .find('\n')
            .map_or(value.len(), |index| next_start + index);
        vec![(line_start, line_end), (next_start, next_end)]
    } else {
        selected_line_ranges(value, range)
    };
    if lines.len() < 2 {
        return None;
    }
    let block_start = lines[0].0;
    let block_end = lines[lines.len() - 1].1;
    let mut next = String::with_capacity(value.len());
    next.push_str(&value[..block_start]);
    // `(line_start, content_start, line_end, new_content_start)` per line.
    let mut mapping: Vec<(usize, usize, usize, usize)> = Vec::with_capacity(lines.len());
    let mut left_content = "";
    for (index, &(line_start, line_end)) in lines.iter().enumerate() {
        let content_start = if index == 0 {
            line_start
        } else {
            line_content_start(value, line_start)
        };
        let content = &value[content_start..line_end];
        if index > 0 {
            let seam = !left_content.is_empty()
                && !content.is_empty()
                && !left_content.ends_with(char::is_whitespace);
            if seam {
                next.push(' ');
            }
        }
        let new_content_start = next.len();
        next.push_str(content);
        mapping.push((line_start, content_start, line_end, new_content_start));
        left_content = content;
    }
    next.push_str(&value[block_end..]);
    let map = |offset: usize| -> usize {
        for &(line_start, content_start, line_end, new_content_start) in &mapping {
            if (line_start..=line_end).contains(&offset) {
                if offset <= content_start {
                    return new_content_start;
                }
                return new_content_start + (offset - content_start);
            }
        }
        offset
    };
    let anchor = clamp_boundary(&next, map(selection.anchor));
    let focus = clamp_boundary(&next, map(selection.focus));
    Some((next, crate::TextSelection { anchor, focus }))
}

/// Uppercase (`upper`) or lowercase the selection. UTF-8 safe: case
/// expansion such as `ß` → `SS` shifts the selection end accordingly, and an
/// empty selection declines.
pub fn transform_selection_case(
    value: &str,
    selection: crate::TextSelection,
    upper: bool,
) -> Option<(String, crate::TextSelection)> {
    let range = selection.ordered();
    if range.is_empty() {
        return None;
    }
    let transformed = if upper {
        value[range.clone()].to_uppercase()
    } else {
        value[range.clone()].to_lowercase()
    };
    let mut next = String::with_capacity(value.len() + transformed.len());
    next.push_str(&value[..range.start]);
    next.push_str(&transformed);
    next.push_str(&value[range.end..]);
    let delta = transformed.len() as isize - range.len() as isize;
    let map = |offset: usize| -> usize {
        if offset <= range.start {
            offset
        } else if offset >= range.end {
            (offset as isize + delta).max(range.start as isize) as usize
        } else {
            range.start
        }
    };
    let anchor = clamp_boundary(&next, map(selection.anchor));
    let focus = clamp_boundary(&next, map(selection.focus));
    Some((next, crate::TextSelection { anchor, focus }))
}

/// Sort the lines the selection touches by byte order (`str` `Ord`, so the
/// result is deterministic across platforms). `descending` reverses the
/// order; `unique` drops repeated rows after sorting. The selection then
/// covers the sorted block. Returns `None` when fewer than two lines are
/// touched (sorting one line cannot change anything).
pub fn sort_lines(
    value: &str,
    selection: crate::TextSelection,
    descending: bool,
    unique: bool,
) -> Option<(String, crate::TextSelection)> {
    let lines = selected_line_ranges(value, selection.ordered());
    if lines.len() < 2 {
        return None;
    }
    let block_start = lines[0].0;
    let block_end = lines[lines.len() - 1].1;
    let mut rows: Vec<&str> = lines
        .iter()
        .map(|&(start, end)| &value[start..end])
        .collect();
    if descending {
        rows.sort_unstable_by(|left, right| right.cmp(left));
    } else {
        rows.sort_unstable();
    }
    if unique {
        rows.dedup();
    }
    let suffix = &value[block_end..];
    let mut next = String::with_capacity(value.len());
    next.push_str(&value[..block_start]);
    for row in &rows {
        next.push_str(row);
        next.push('\n');
    }
    next.pop();
    next.push_str(suffix);
    let block_end_next = next.len() - suffix.len();
    let selection = if selection.anchor <= selection.focus {
        crate::TextSelection {
            anchor: block_start,
            focus: block_end_next,
        }
    } else {
        crate::TextSelection {
            anchor: block_end_next,
            focus: block_start,
        }
    };
    Some((next, selection))
}

fn bracket_closer(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

fn bracket_opener(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

/// Byte offsets `(open, close)` of the bracket pair adjacent to `caret`.
///
/// A caret immediately before an opener pairs forward, immediately after a
/// closer pairs backward, and between a matched pair (for example `(|)`) both
/// brackets pair with each other. Nesting is counted, and unbalanced text
/// yields `None`. Only `()`, `[]`, `{}` pair here — quotes are left to the
/// host's language model.
pub fn matching_bracket_pair(value: &str, caret: usize) -> Option<(usize, usize)> {
    let caret = clamp_boundary(value, caret);
    let after = value[caret..].chars().next();
    let before = value[..caret].chars().next_back();
    if let Some(open) = after.filter(|character| bracket_closer(*character).is_some()) {
        return match_bracket_forward(value, caret, open);
    }
    if let Some(open) = before.filter(|character| bracket_closer(*character).is_some()) {
        let open_offset = caret - open.len_utf8();
        return match_bracket_forward(value, open_offset, open);
    }
    if let Some(close) = before.filter(|character| bracket_opener(*character).is_some()) {
        let close_offset = caret - close.len_utf8();
        return match_bracket_backward(value, close_offset, close);
    }
    if let Some(close) = after.filter(|character| bracket_opener(*character).is_some()) {
        return match_bracket_backward(value, caret, close);
    }
    None
}

fn match_bracket_forward(value: &str, open_offset: usize, open: char) -> Option<(usize, usize)> {
    let close = bracket_closer(open)?;
    let mut depth = 1usize;
    for (offset, character) in value[open_offset + open.len_utf8()..].char_indices() {
        let offset = open_offset + open.len_utf8() + offset;
        if character == open {
            depth += 1;
        } else if character == close {
            depth -= 1;
            if depth == 0 {
                return Some((open_offset, offset));
            }
        }
    }
    None
}

fn match_bracket_backward(value: &str, close_offset: usize, close: char) -> Option<(usize, usize)> {
    let open = bracket_opener(close)?;
    let mut depth = 1usize;
    for (offset, character) in value[..close_offset].char_indices().rev() {
        if character == close {
            depth += 1;
        } else if character == open {
            depth -= 1;
            if depth == 0 {
                return Some((offset, close_offset));
            }
        }
    }
    None
}

/// Literal search options for [`find_matches`] and friends. Regex search is
/// deliberately out of scope for this layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextSearchOptions {
    /// When `false` (the default), ASCII letters match case-insensitively;
    /// non-ASCII characters always compare by exact codepoint.
    pub case_sensitive: bool,
    /// Require the bytes flanking a match to be non-identifier characters
    /// (`[A-Za-z0-9_]`) so `ser` does not match inside `user` or `ser1`.
    pub whole_word: bool,
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn starts_with_query(haystack: &str, query: &str, case_sensitive: bool) -> bool {
    let haystack = haystack.as_bytes();
    let needle = query.as_bytes();
    haystack.len() >= needle.len()
        && haystack[..needle.len()]
            .iter()
            .zip(needle)
            .all(|(value, query)| {
                if case_sensitive {
                    value == query
                } else {
                    value.eq_ignore_ascii_case(query)
                }
            })
}

fn is_word_bounded(value: &str, start: usize, end: usize) -> bool {
    let bytes = value.as_bytes();
    let before = start == 0 || !is_ident_byte(bytes[start - 1]);
    let after = end >= bytes.len() || !is_ident_byte(bytes[end]);
    before && after
}

/// First match starting at or after `from`. `from` is clamped to the nearest
/// char boundary; an empty query never matches.
fn first_match_from(
    value: &str,
    query: &str,
    options: TextSearchOptions,
    from: usize,
) -> Option<std::ops::Range<usize>> {
    if query.is_empty() {
        return None;
    }
    let mut start = clamp_boundary(value, from);
    while start + query.len() <= value.len() {
        if starts_with_query(&value[start..], query, options.case_sensitive) {
            let end = start + query.len();
            if !options.whole_word || is_word_bounded(value, start, end) {
                return Some(start..end);
            }
        }
        start += value[start..].chars().next()?.len_utf8();
    }
    None
}

/// All literal matches of `query` in `value`, left to right and
/// non-overlapping: scanning resumes at the end of each match, so `"aa"` in
/// `"aaa"` yields one match.
pub fn find_matches(
    value: &str,
    query: &str,
    options: TextSearchOptions,
) -> Vec<std::ops::Range<usize>> {
    find_matches_capped(value, query, options, usize::MAX).0
}

/// [`find_matches`] with an early-stop cap: at most `cap` matches are
/// returned, and `truncated` reports that scanning stopped early because a
/// `cap + 1`-th match exists. Results are always a prefix of the uncapped
/// [`find_matches`] result; `cap == 0` returns no matches and only reports
/// whether any match exists.
pub fn find_matches_capped(
    value: &str,
    query: &str,
    options: TextSearchOptions,
    cap: usize,
) -> (Vec<std::ops::Range<usize>>, bool) {
    let mut matches = Vec::new();
    let mut from = 0;
    while let Some(found) = first_match_from(value, query, options, from) {
        if matches.len() == cap {
            return (matches, true);
        }
        from = found.end;
        matches.push(found);
    }
    (matches, false)
}

/// Next match at or after `from`, wrapping to the first match when none
/// follows. Match ranges come from [`find_matches`].
pub fn find_next_match(
    matches: &[std::ops::Range<usize>],
    from: usize,
) -> Option<std::ops::Range<usize>> {
    matches
        .iter()
        .find(|found| found.start >= from)
        .cloned()
        .or_else(|| matches.first().cloned())
}

/// Previous match ending at or before `from`, wrapping to the last match when
/// none precedes. A match ending exactly at `from` counts as previous.
pub fn find_previous_match(
    matches: &[std::ops::Range<usize>],
    from: usize,
) -> Option<std::ops::Range<usize>> {
    matches
        .iter()
        .rev()
        .find(|found| found.end <= from)
        .cloned()
        .or_else(|| matches.last().cloned())
}

/// Occurrence-highlight scan cap: a pathological document (a word repeated
/// thousands of times) stops deriving marks past this many occurrences.
pub const OCCURRENCE_HIGHLIGHT_LIMIT: usize = 200;

/// Query text for cursor-occurrence highlighting.
///
/// - Collapsed caret: the `[A-Za-z0-9_]` word around `caret` (empty result is
///   `None`), matched whole-word.
/// - Non-empty single-line selection: the selected text itself, matched as a
///   plain case-sensitive substring (Zed selection-highlight semantics).
/// - A multi-line selection returns `None`: highlighting every occurrence of
///   a paragraph is noise, not signal.
///
/// Returns the query plus whether matches must be whole-word bounded. The
/// caller feeds the string to [`find_matches_capped`] with
/// `case_sensitive: true`.
pub fn occurrence_query_at(
    value: &str,
    selection: Option<(usize, usize)>,
    caret: usize,
) -> Option<(String, bool)> {
    if let Some((start, end)) = selection {
        let start = start.min(end).min(value.len());
        let end = end.min(value.len());
        if start < end
            && value.is_char_boundary(start)
            && value.is_char_boundary(end)
            && !value[start..end].contains('\n')
        {
            return Some((value[start..end].to_owned(), false));
        }
        return None;
    }
    let caret = clamp_boundary(value, caret);
    let bytes = value.as_bytes();
    let mut start = caret;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = caret;
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    (start < end).then(|| (value[start..end].to_owned(), true))
}

/// Replace every match ([`find_matches`] semantics: left to right,
/// non-overlapping) with `replacement`, returning the new value and the
/// replacement count. Matching runs on the original text, so a replacement
/// containing the query cannot rescan.
pub fn replace_all_matches(
    value: &str,
    query: &str,
    replacement: &str,
    options: TextSearchOptions,
) -> (String, usize) {
    let matches = find_matches(value, query, options);
    if matches.is_empty() {
        return (value.to_owned(), 0);
    }
    let mut next = String::with_capacity(value.len() + replacement.len());
    let mut cursor = 0;
    for found in &matches {
        next.push_str(&value[cursor..found.start]);
        next.push_str(replacement);
        cursor = found.end;
    }
    next.push_str(&value[cursor..]);
    (next, matches.len())
}

/// One cursor's multi-cursor edit, computed against the pre-edit value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorEdit {
    /// Replace `range` with `insert`; `caret` is the post-edit caret when
    /// this edit applies alone.
    Span(TextReplacement),
    /// A whole-value transform: `next` is the transformed document and
    /// `selection` the transform's mapped selection. The pipeline extracts
    /// the replaced span by diffing so batched transforms share one output
    /// string instead of rebuilding the document per cursor.
    Transform {
        next: String,
        selection: crate::TextSelection,
    },
}

/// One accepted cursor edit inside the batch pipeline.
#[derive(Debug, Clone)]
struct CursorSpan {
    /// Replaced range on the pre-edit value.
    range: std::ops::Range<usize>,
    insert: String,
    /// Where the inserted text starts in the batched output.
    output_start: usize,
}

/// Extract the contiguous replacement a whole-value transform applied by
/// stripping the shared prefix/suffix. Line transforms replace one contiguous
/// block, so the diff is exactly that block.
fn transform_span(value: &str, next: &str) -> (std::ops::Range<usize>, String) {
    let prefix = value
        .chars()
        .zip(next.chars())
        .take_while(|(current, candidate)| current == candidate)
        .map(|(character, _)| character.len_utf8())
        .sum::<usize>();
    let suffix = value[prefix..]
        .chars()
        .rev()
        .zip(next[prefix..].chars().rev())
        .take_while(|(current, candidate)| current == candidate)
        .map(|(_, character)| character.len_utf8())
        .sum::<usize>();
    if prefix + suffix > value.len().min(next.len()) {
        // Degenerate diff (equal-length swaps overlap prefix and suffix);
        // fall back to the whole document so the batch stays correct.
        return (0..value.len(), next.to_owned());
    }
    (
        prefix..value.len() - suffix,
        next[prefix..next.len() - suffix].to_owned(),
    )
}

/// Map an offset through the accepted spans. Offsets inside a replaced span
/// land on that span's output start (the surviving edit point); offsets after
/// a span accumulate its length delta.
fn remap_offset(offset: usize, spans: &[CursorSpan]) -> usize {
    let mut delta = 0isize;
    for span in spans {
        if span.range.end <= offset {
            delta += span.insert.len() as isize - span.range.len() as isize;
        } else if span.range.start < offset {
            return span.output_start;
        } else {
            break;
        }
    }
    ((offset as isize + delta).max(0)) as usize
}

/// Translate a cursor's own-edit selection into batched-output coordinates:
/// offsets inside the cursor's inserted text shift by the span's output
/// start; the rest convert back to pre-edit coordinates and remap through
/// every accepted span.
fn translate_selection(
    selection: crate::TextSelection,
    own: &CursorSpan,
    spans: &[CursorSpan],
) -> crate::TextSelection {
    let to_output = |offset: usize| -> usize {
        if offset <= own.range.start {
            remap_offset(offset, spans)
        } else if offset >= own.range.start + own.insert.len() {
            let original = offset - own.insert.len() + own.range.len();
            remap_offset(original, spans)
        } else {
            own.output_start + (offset - own.range.start)
        }
    };
    crate::TextSelection {
        anchor: to_output(selection.anchor),
        focus: to_output(selection.focus),
    }
}

/// Apply every cursor's edit in one pass: edits are computed per selection
/// against the pre-edit snapshot, spliced into a single output string, and
/// every cursor's selection is remapped onto the result. Overlapping edit
/// spans collapse onto the first cursor (duplicates fuse away on normalize).
///
/// Each slot carries the cursor's pre-edit selection; a slot with `None` keeps
/// that selection remapped through its neighbours' edits. Returns `None` when
/// no cursor produced an edit. The returned selections keep the input order;
/// callers split the primary back out and normalize.
pub fn apply_cursor_edits(
    value: &str,
    edits: &[(crate::TextSelection, Option<CursorEdit>)],
) -> Option<(String, Vec<crate::TextSelection>)> {
    let mut accepted: Vec<(CursorSpan, crate::TextSelection)> = Vec::new();
    // Slot -> accepted index; `None` for declined slots and for spans dropped
    // by the overlap guard (those cursors fall back to offset remapping).
    let mut slot_accepted: Vec<Option<usize>> = Vec::with_capacity(edits.len());
    for (_, edit) in edits {
        let Some(edit) = edit else {
            slot_accepted.push(None);
            continue;
        };
        let (span, edited_selection) = match edit {
            CursorEdit::Span(replacement) => (
                CursorSpan {
                    range: replacement.range.clone(),
                    insert: replacement.insert.clone(),
                    output_start: 0,
                },
                crate::TextSelection::caret(replacement.caret),
            ),
            CursorEdit::Transform { next, selection } => {
                let (range, insert) = transform_span(value, next);
                (
                    CursorSpan {
                        range,
                        insert,
                        output_start: 0,
                    },
                    *selection,
                )
            }
        };
        if accepted.iter().any(|(other, _)| {
            span.range.start < other.range.end && other.range.start < span.range.end
        }) {
            slot_accepted.push(None);
            continue;
        }
        accepted.push((span, edited_selection));
        slot_accepted.push(Some(accepted.len() - 1));
    }
    if accepted.is_empty() {
        return None;
    }
    accepted.sort_by_key(|(span, _)| (span.range.start, span.range.end));
    let inserted_bytes: usize = accepted.iter().map(|(span, _)| span.insert.len()).sum();
    let mut next = String::with_capacity(value.len() + inserted_bytes);
    let mut cursor = 0usize;
    for (span, _) in accepted.iter_mut() {
        let end = span.range.end.min(value.len()).max(span.range.start);
        next.push_str(&value[cursor..span.range.start.min(value.len())]);
        span.output_start = next.len();
        next.push_str(&span.insert);
        cursor = end;
    }
    next.push_str(&value[cursor..]);
    let spans: Vec<CursorSpan> = accepted.iter().map(|(span, _)| span.clone()).collect();
    let mut result = Vec::with_capacity(edits.len());
    for (index, (selection, _)) in edits.iter().enumerate() {
        let output = match slot_accepted[index] {
            Some(accepted_index) => {
                let (span, edited) = &accepted[accepted_index];
                translate_selection(*edited, span, &spans)
            }
            None => crate::TextSelection {
                anchor: remap_offset(selection.anchor, &spans),
                focus: remap_offset(selection.focus, &spans),
            },
        };
        result.push(output);
    }
    Some((next, result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_edits_splice_in_one_pass_and_remap_carets() {
        let value = "ab_cd";
        let edits = vec![
            (
                crate::TextSelection::caret(2),
                Some(CursorEdit::Span(TextReplacement {
                    range: 2..2,
                    insert: "X".into(),
                    caret: 3,
                })),
            ),
            (
                crate::TextSelection::caret(5),
                Some(CursorEdit::Span(TextReplacement {
                    range: 5..5,
                    insert: "X".into(),
                    caret: 6,
                })),
            ),
        ];
        let (next, selections) = apply_cursor_edits(value, &edits).unwrap();
        // 一次拼接：既不是逐光标重写字符串，也不是从前往后反复搬移。
        assert_eq!(next, "abX_cdX");
        assert_eq!(
            selections,
            vec![
                crate::TextSelection::caret(3),
                // 末尾插入后光标停在新串末尾（偏移 7）。
                crate::TextSelection::caret(7)
            ]
        );
    }

    #[test]
    fn cursor_edits_remap_slots_that_declined_to_edit() {
        let value = "abcd";
        let edits = vec![
            // 末尾光标没有编辑（例如无可删除内容），随其他编辑平移。
            (crate::TextSelection::caret(4), None),
            (
                crate::TextSelection::caret(1),
                Some(CursorEdit::Span(TextReplacement {
                    range: 0..1,
                    insert: String::new(),
                    caret: 0,
                })),
            ),
        ];
        let (next, selections) = apply_cursor_edits(value, &edits).unwrap();
        assert_eq!(next, "bcd");
        assert_eq!(selections[0], crate::TextSelection::caret(3));
        assert_eq!(selections[1], crate::TextSelection::caret(0));
    }

    #[test]
    fn cursor_edits_drop_overlapping_spans_onto_the_first_cursor() {
        let value = "abcdef";
        // 两个光标同行 delete-to-line-end：后者的范围被前者覆盖，丢弃。
        let edits = vec![
            (
                crate::TextSelection::caret(1),
                Some(CursorEdit::Span(TextReplacement {
                    range: 1..6,
                    insert: String::new(),
                    caret: 1,
                })),
            ),
            (
                crate::TextSelection::caret(3),
                Some(CursorEdit::Span(TextReplacement {
                    range: 3..6,
                    insert: String::new(),
                    caret: 3,
                })),
            ),
        ];
        let (next, selections) = apply_cursor_edits(value, &edits).unwrap();
        assert_eq!(next, "a");
        assert_eq!(selections[0], crate::TextSelection::caret(1));
        // 被丢弃的光标映射到幸存编辑点，随后在 normalize 中合并。
        assert_eq!(selections[1], crate::TextSelection::caret(1));
    }

    #[test]
    fn cursor_edits_extract_transform_spans_by_diffing() {
        let value = "a\nb";
        let edits = vec![(
            crate::TextSelection::caret(3),
            Some(CursorEdit::Transform {
                next: "a\nb\nc".to_string(),
                selection: crate::TextSelection::caret(5),
            }),
        )];
        let (next, selections) = apply_cursor_edits(value, &edits).unwrap();
        assert_eq!(next, "a\nb\nc");
        assert_eq!(selections, vec![crate::TextSelection::caret(5)]);

        // 多光标 transform：后一个光标的位移叠加在前一个编辑之后。
        let value = "a\nb";
        let edits = vec![
            (
                crate::TextSelection::caret(1),
                Some(CursorEdit::Transform {
                    next: "a\nx\nb".to_string(),
                    selection: crate::TextSelection::caret(3),
                }),
            ),
            (
                crate::TextSelection::caret(3),
                Some(CursorEdit::Span(TextReplacement {
                    range: 3..3,
                    insert: "y".into(),
                    caret: 4,
                })),
            ),
        ];
        let (next, selections) = apply_cursor_edits(value, &edits).unwrap();
        assert_eq!(next, "a\nx\nby");
        assert_eq!(
            selections,
            vec![
                crate::TextSelection::caret(3),
                crate::TextSelection::caret(6)
            ]
        );
    }

    #[test]
    fn cursor_edits_return_none_when_no_cursor_edits() {
        let value = "abc";
        let edits = vec![(crate::TextSelection::caret(1), None)];
        assert!(apply_cursor_edits(value, &edits).is_none());
    }

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

    #[test]
    fn occurrence_query_takes_caret_word_selection_text_or_nothing() {
        let value = "fn count() { count }";
        // 收起光标：光标处 [A-Za-z0-9_] 词，全词匹配。
        let caret = "fn cou".len();
        assert_eq!(
            occurrence_query_at(value, None, caret),
            Some(("count".to_owned(), true))
        );
        // 词尾光标取同一个词。
        assert_eq!(
            occurrence_query_at(value, None, "fn count".len()),
            Some(("count".to_owned(), true))
        );
        // 下划线属于词字符。
        assert_eq!(
            occurrence_query_at("snake_case_x", None, "snake".len() + 1),
            Some(("snake_case_x".to_owned(), true))
        );
        // 光标不在词内（括号之间）：返回 None。
        assert_eq!(
            occurrence_query_at("fn () { count }", None, "fn (".len()),
            None
        );
        assert_eq!(occurrence_query_at("", None, 0), None);
        // 非空单行选区：选中文本本身，子串匹配（不加全词约束）。
        assert_eq!(
            occurrence_query_at(value, Some(("fn ".len(), "fn count".len())), 0),
            Some(("count".to_owned(), false))
        );
        // 多行选区：None。
        let multiline = "ab\ncd";
        assert_eq!(
            occurrence_query_at(multiline, Some((0, multiline.len())), 0),
            None
        );
    }

    #[test]
    fn find_matches_respects_case_and_whole_word_options() {
        let value = "Ser ser user SER ser1 _ser";
        let all = find_matches(value, "ser", TextSearchOptions::default());
        // 默认不区分大小写、不做全词约束：包含 "user"、"ser1"、"_ser" 的字面子串。
        assert_eq!(
            all,
            vec![
                0..3,   // "Ser"
                4..7,   // "ser"
                9..12,  // "user"
                13..16, // "SER"
                17..20, // "ser1"
                23..26, // "_ser"
            ]
        );
        let sensitive = find_matches(
            value,
            "ser",
            TextSearchOptions {
                case_sensitive: true,
                ..TextSearchOptions::default()
            },
        );
        assert_eq!(sensitive, vec![4..7, 9..12, 17..20, 23..26]);
        let words = find_matches(
            value,
            "ser",
            TextSearchOptions {
                whole_word: true,
                ..TextSearchOptions::default()
            },
        );
        // 全词边界按 [A-Za-z0-9_] 判定："user"、"ser1"、"_ser" 都不算；
        // "Ser"、"ser"、"SER" 是独立词。
        assert_eq!(words, vec![0..3, 4..7, 13..16]);
        let sensitive_words = find_matches(
            value,
            "Ser",
            TextSearchOptions {
                case_sensitive: true,
                whole_word: true,
            },
        );
        assert_eq!(sensitive_words, vec![0..3]);
    }

    #[test]
    fn find_matches_is_utf8_safe_and_ignores_empty_queries() {
        let value = "界界 héllo 界";
        let found = find_matches(value, "界", TextSearchOptions::default());
        assert_eq!(
            found,
            vec![
                0.."界".len(),
                "界".len().."界界".len(),
                "界界 héllo ".len().."界界 héllo 界".len(),
            ]
        );
        for range in &found {
            assert!(value.is_char_boundary(range.start) && value.is_char_boundary(range.end));
        }
        // 查询与周围文本都是多字节字符时不产生跨字符的假匹配；大小写折叠
        // 只作用于 ASCII，"HÉLLO" 不会匹配 "é"。
        let accented = find_matches("héllo HÉLLO", "é", TextSearchOptions::default());
        assert_eq!(accented, vec![1..3]);
        assert!(find_matches("abc", "", TextSearchOptions::default()).is_empty());
        assert!(find_matches("", "abc", TextSearchOptions::default()).is_empty());
    }

    #[test]
    fn find_matches_are_left_to_right_and_non_overlapping() {
        assert_eq!(
            find_matches("aaaa", "aa", TextSearchOptions::default()),
            vec![0..2, 2..4]
        );
        assert_eq!(
            find_matches("aaa", "aa", TextSearchOptions::default()),
            vec![0..2]
        );
    }

    #[test]
    fn find_matches_capped_stops_early_and_reports_truncation() {
        let options = TextSearchOptions::default();
        let value = "ab ab ab ab";
        let all = find_matches(value, "ab", options);
        assert_eq!(all, vec![0..2, 3..5, 6..8, 9..11]);

        // 达到 cap 即停：只保留前缀，truncated 标记还有更多匹配。
        let (capped, truncated) = find_matches_capped(value, "ab", options, 2);
        assert_eq!(capped, vec![0..2, 3..5]);
        assert!(truncated);

        // cap 覆盖全部匹配：结果与 find_matches 一致且不标记截断。
        let (complete, truncated) = find_matches_capped(value, "ab", options, all.len());
        assert_eq!(complete, all);
        assert!(!truncated);

        // cap = 0：不返回匹配，仅在存在匹配时报告截断。
        let (empty, truncated) = find_matches_capped(value, "ab", options, 0);
        assert!(empty.is_empty());
        assert!(truncated);
        let (none, truncated) = find_matches_capped(value, "xy", options, 0);
        assert!(none.is_empty());
        assert!(!truncated);

        // 任意 cap 下都是全量结果的前缀，截断语义一致（UTF-8 文本同验）。
        let cjk = find_matches("界ab界 ab", "ab", options);
        assert_eq!(cjk, vec![3..5, 9..11]);
        for cap in 0..=all.len() + 1 {
            let (prefix, truncated) = find_matches_capped(value, "ab", options, cap);
            assert_eq!(prefix, all.iter().take(cap).cloned().collect::<Vec<_>>());
            assert_eq!(truncated, cap < all.len());
        }
    }

    #[test]
    fn match_navigation_wraps_around_the_document() {
        let value = "ab ab ab";
        let matches = find_matches(value, "ab", TextSearchOptions::default());
        // 下一个从选区末尾起找。
        assert_eq!(find_next_match(&matches, 0), Some(0..2));
        assert_eq!(find_next_match(&matches, 2), Some(3..5));
        // 越过最后一个匹配后环绕到开头。
        assert_eq!(find_next_match(&matches, 7), Some(0..2));
        // 上一个从选区起点起找，匹配恰好结束在起点也算。
        assert_eq!(find_previous_match(&matches, 0), Some(6..8)); // 环绕
        assert_eq!(find_previous_match(&matches, 3), Some(0..2));
        assert_eq!(find_previous_match(&matches, 8), Some(6..8));
        assert!(find_next_match(&[], 0).is_none());
        assert!(find_previous_match(&[], 0).is_none());
    }

    #[test]
    fn replace_all_counts_left_to_right_replacements() {
        let (replaced, count) =
            replace_all_matches("a界b a界b", "界", "CJK", TextSearchOptions::default());
        assert_eq!(replaced, "aCJKb aCJKb");
        assert_eq!(count, 2);
        // 替换文本包含查询时不重扫。
        let (grown, grown_count) =
            replace_all_matches("aa", "a", "aa", TextSearchOptions::default());
        assert_eq!(grown, "aaaa");
        assert_eq!(grown_count, 2);
        let (untouched, zero) =
            replace_all_matches("abc", "xyz", "q", TextSearchOptions::default());
        assert_eq!(untouched, "abc");
        assert_eq!(zero, 0);
        let (empty, empty_count) =
            replace_all_matches("abc", "", "q", TextSearchOptions::default());
        assert_eq!(empty, "abc");
        assert_eq!(empty_count, 0);
    }

    #[test]
    fn lines_move_up_and_down_with_their_selection() {
        let value = "ab\ncd\nef";
        let selection = crate::TextSelection {
            anchor: 3,
            focus: 5,
        };
        // "cd" 与上一行交换，选区跟随移动后的文本。
        let (up, up_selection) = move_lines(value, selection, TextLineDirection::Up).unwrap();
        assert_eq!(up, "cd\nab\nef");
        assert_eq!(
            up_selection,
            crate::TextSelection {
                anchor: 0,
                focus: 2
            }
        );
        // "cd" 再与下一行交换并回到原位。
        let (down, _) = move_lines(&up, up_selection, TextLineDirection::Down).unwrap();
        assert_eq!(down, value);
        // 多行块整体移动。
        let block = crate::TextSelection {
            anchor: 0,
            focus: 5,
        };
        let (down, down_selection) = move_lines(value, block, TextLineDirection::Down).unwrap();
        assert_eq!(down, "ef\nab\ncd");
        assert_eq!(
            down_selection,
            crate::TextSelection {
                anchor: 3,
                focus: 8
            }
        );
        // 已移到顶部后继续上移是空操作；末行继续下移同样是空操作。
        assert!(move_lines(&up, up_selection, TextLineDirection::Up).is_none());
        let last = crate::TextSelection {
            anchor: 6,
            focus: 8,
        };
        assert!(move_lines(value, last, TextLineDirection::Down).is_none());
    }

    #[test]
    fn move_lines_handles_edge_line_shapes() {
        // 无结尾换行：最后一行下移是空操作；光标行（无选区）按整行移动。
        let value = "a\nb";
        let caret_on_b = crate::TextSelection::caret(3);
        assert!(move_lines(value, caret_on_b, TextLineDirection::Down).is_none());
        let (up, up_selection) = move_lines(value, caret_on_b, TextLineDirection::Up).unwrap();
        assert_eq!(up, "b\na");
        assert_eq!(up_selection, crate::TextSelection::caret(1));
        // 结尾换行后的幻影空行可以上移。
        let value = "a\n";
        let phantom = crate::TextSelection::caret(2);
        let (up, _) = move_lines(value, phantom, TextLineDirection::Up).unwrap();
        assert_eq!(up, "\na");
        // 选区结束在行首时该行不算触碰（与注释切换同一约定）。
        let value = "a\nb\nc";
        let boundary = crate::TextSelection {
            anchor: 0,
            focus: 2,
        };
        let (moved, _) = move_lines(value, boundary, TextLineDirection::Down).unwrap();
        assert_eq!(moved, "b\na\nc");
    }

    #[test]
    fn duplicate_lines_copies_below_and_selects_the_copy() {
        let value = "ab\ncd";
        let selection = crate::TextSelection {
            anchor: 3,
            focus: 5,
        };
        let (next, duplicated) = duplicate_lines(value, selection).unwrap();
        assert_eq!(next, "ab\ncd\ncd");
        // 选区落在副本上。
        assert_eq!(
            duplicated,
            crate::TextSelection {
                anchor: 6,
                focus: 8
            }
        );
        // 纯光标：复制光标所在整行，光标停在副本同一列。
        let value = "xy";
        let (next, duplicated) = duplicate_lines(value, crate::TextSelection::caret(1)).unwrap();
        assert_eq!(next, "xy\nxy");
        assert_eq!(duplicated, crate::TextSelection::caret(4));
        // 空文本没有可复制的行。
        assert!(duplicate_lines("", crate::TextSelection::caret(0)).is_none());
    }

    #[test]
    fn delete_lines_removes_the_block_and_one_adjacent_newline() {
        // 中间行：换行随行一起删除，前后两行拼合。
        let value = "a\nb\nc";
        let middle = crate::TextSelection {
            anchor: 2,
            focus: 3,
        };
        let (next, deleted) = delete_lines(value, middle).unwrap();
        assert_eq!(next, "a\nc");
        assert_eq!(deleted, crate::TextSelection::caret(2));
        // 首行：从块起点删除到下一行起点。
        let first = crate::TextSelection::caret(0);
        let (next, _) = delete_lines(value, first).unwrap();
        assert_eq!(next, "b\nc");
        // 末行（无结尾换行）：连同前导换行一起删除，光标落在上一行末尾。
        let last = crate::TextSelection {
            anchor: 4,
            focus: 5,
        };
        let (next, deleted) = delete_lines(value, last).unwrap();
        assert_eq!(next, "a\nb");
        assert_eq!(deleted, crate::TextSelection::caret(3));
        // 只剩一行：清空文本。
        let (next, deleted) = delete_lines("only", crate::TextSelection::caret(1)).unwrap();
        assert_eq!(next, "");
        assert_eq!(deleted, crate::TextSelection::caret(0));
        // 空文本无可删除行。
        assert!(delete_lines("", crate::TextSelection::caret(0)).is_none());
    }

    #[test]
    fn join_lines_merges_touched_lines_with_single_space_seams() {
        // 相邻行合并插入单个空格。
        let value = "ab\ncd\nef";
        let selection = crate::TextSelection {
            anchor: 1,
            focus: 8,
        };
        let (next, joined) = join_lines(value, selection).unwrap();
        assert_eq!(next, "ab cd ef");
        // 选区映射到合并后的行（起点不动，终点收缩到行尾）。
        assert_eq!(
            joined,
            crate::TextSelection {
                anchor: 1,
                focus: 8
            }
        );
        // 下一行的前导空白被移除；左侧已有尾随空白时不再补空格。
        let value = "a \n  b";
        let (next, _) = join_lines(
            value,
            crate::TextSelection {
                anchor: 0,
                focus: 5,
            },
        )
        .unwrap();
        assert_eq!(next, "a b");
        // 空行参与合并不产生空格。
        let value = "a\n\nb";
        let (next, _) = join_lines(
            value,
            crate::TextSelection {
                anchor: 0,
                focus: 3,
            },
        )
        .unwrap();
        assert_eq!(next, "a\nb");
        // 裸光标：把光标行和下一行合并。
        let value = "ab\ncd";
        let (next, joined) = join_lines(value, crate::TextSelection::caret(1)).unwrap();
        assert_eq!(next, "ab cd");
        // 光标在未移动的首行上，列保持不变。
        assert_eq!(joined, crate::TextSelection::caret(1));
        // 最后一行的光标没有下一行可合并。
        assert!(join_lines("ab\ncd", crate::TextSelection::caret(4)).is_none());
        // 单行文本合并是空操作。
        assert!(join_lines("ab", crate::TextSelection::caret(1)).is_none());
    }

    #[test]
    fn join_lines_keeps_utf8_offsets_valid() {
        // 多字节字符行参与合并时选区映射仍落在字符边界。
        let value = "界界\nhéllo";
        let selection = crate::TextSelection {
            anchor: "界".len(),
            focus: value.len(),
        };
        let (next, joined) = join_lines(value, selection).unwrap();
        assert_eq!(next, "界界 héllo");
        for offset in [joined.anchor, joined.focus] {
            assert!(next.is_char_boundary(offset));
        }
        assert_eq!(joined.anchor, "界".len());
        assert_eq!(joined.focus, next.len());
    }

    #[test]
    fn case_transform_is_utf8_safe_and_declines_empty_selections() {
        // ASCII 直接变换。
        let value = "ab CD";
        let selection = crate::TextSelection {
            anchor: 0,
            focus: 5,
        };
        let (upper, upper_selection) = transform_selection_case(value, selection, true).unwrap();
        assert_eq!(upper, "AB CD");
        assert_eq!(upper_selection, selection);
        let (lower, _) = transform_selection_case(&upper, selection, false).unwrap();
        assert_eq!(lower, "ab cd");
        // ﬁ → FI 的字节收缩把选区终点平移到有效边界。
        let value = "aﬁb";
        let selection = crate::TextSelection {
            anchor: 1,
            focus: 4,
        };
        let (upper, upper_selection) = transform_selection_case(value, selection, true).unwrap();
        assert_eq!(upper, "aFIb");
        assert_eq!(
            upper_selection,
            crate::TextSelection {
                anchor: 1,
                focus: 3
            }
        );
        // İ → i + 组合点 的字节膨胀同样保持边界有效。
        let value = "aİb";
        let selection = crate::TextSelection {
            anchor: 1,
            focus: 3,
        };
        let (lower, lower_selection) = transform_selection_case(value, selection, false).unwrap();
        assert_eq!(lower, "ai\u{307}b");
        assert_eq!(
            lower_selection,
            crate::TextSelection {
                anchor: 1,
                focus: 4
            }
        );
        // 空选区拒绝。
        assert!(transform_selection_case(value, crate::TextSelection::caret(1), true).is_none());
    }

    #[test]
    fn sort_lines_orders_reverses_and_dedups() {
        let value = "pear\napple\npear\nbanana";
        let whole = crate::TextSelection {
            anchor: 0,
            focus: value.len(),
        };
        let (asc, asc_selection) = sort_lines(value, whole, false, false).unwrap();
        assert_eq!(asc, "apple\nbanana\npear\npear");
        assert_eq!(
            asc_selection,
            crate::TextSelection {
                anchor: 0,
                focus: asc.len()
            }
        );
        let (desc, _) = sort_lines(value, whole, true, false).unwrap();
        assert_eq!(desc, "pear\npear\nbanana\napple");
        let (unique, _) = sort_lines(value, whole, false, true).unwrap();
        assert_eq!(unique, "apple\nbanana\npear");
        // 逆序选区（anchor > focus）保持方向并覆盖排序后的块。
        let reversed = crate::TextSelection {
            anchor: value.len(),
            focus: 0,
        };
        let (_, sorted_selection) = sort_lines(value, reversed, false, false).unwrap();
        assert_eq!(
            sorted_selection,
            crate::TextSelection {
                anchor: 22,
                focus: 0
            }
        );
        // 单行排序无变化。
        assert!(sort_lines("a\nb", crate::TextSelection::caret(2), false, false).is_none());
    }

    #[test]
    fn matching_bracket_pairs_forward_backward_and_nested() {
        // 光标在 opener 之前。
        assert_eq!(matching_bracket_pair("fn()", 2), Some((2, 3)));
        // 光标紧跟 closer 之后。
        assert_eq!(matching_bracket_pair("fn()", 4), Some((2, 3)));
        // 夹在配对中间（(|)）：两端互相配对。
        assert_eq!(matching_bracket_pair("fn()", 3), Some((2, 3)));
        // 光标在 closer 之前。
        assert_eq!(matching_bracket_pair("(a)", 2), Some((0, 2)));
        // 嵌套计数：内层括号不干扰外层匹配。
        let value = "f(g(h), x)";
        assert_eq!(matching_bracket_pair(value, 1), Some((1, 9)));
        assert_eq!(matching_bracket_pair(value, 3), Some((3, 5)));
        assert_eq!(matching_bracket_pair(value, 9), Some((1, 9)));
        // 三种括号各自独立配对。
        assert_eq!(matching_bracket_pair("{[()]}", 0), Some((0, 5)));
        assert_eq!(matching_bracket_pair("{[()]}", 1), Some((1, 4)));
        // 未闭合返回 None，且引号不参与。
        assert_eq!(matching_bracket_pair("(abc", 0), None);
        assert_eq!(matching_bracket_pair("a\"b\"", 1), None);
        // 邻近没有括号时返回 None。
        assert_eq!(matching_bracket_pair("abc", 1), None);
        assert_eq!(matching_bracket_pair("", 0), None);
    }

    #[test]
    fn page_focus_moves_a_viewport_of_visual_lines() {
        let value = "l0\nl1\nl2\nl3\nl4\nl5\nl6";
        // 行高 20px，视口 60px = 3 行。PageDown 从第 0 行跳到第 3 行。
        let caret = crate::TextSelection::caret(1);
        let (down, goal) = page_caret_focus(
            value,
            caret,
            TextCaretIntent::PageDown,
            false,
            None,
            60.0,
            monospace_probe(value),
        )
        .unwrap();
        assert_eq!(down.focus, value.find("l3").unwrap() + 1);
        assert_eq!(goal, 10.0);
        // 保持目标列：连续 PageDown 后 PageUp 回到同一列。
        let (down_again, goal) = page_caret_focus(
            value,
            down,
            TextCaretIntent::PageDown,
            false,
            Some(goal),
            60.0,
            monospace_probe(value),
        )
        .unwrap();
        assert_eq!(down_again.focus, value.find("l6").unwrap() + 1);
        let (up, _) = page_caret_focus(
            value,
            down_again,
            TextCaretIntent::PageUp,
            false,
            Some(goal),
            60.0,
            monospace_probe(value),
        )
        .unwrap();
        assert_eq!(up.focus, value.find("l3").unwrap() + 1);
        // 文档边缘钳制：第一行 PageUp 到文档起点，最后一行 PageDown 到终点。
        let (top, _) = page_caret_focus(
            value,
            caret,
            TextCaretIntent::PageUp,
            false,
            None,
            60.0,
            monospace_probe(value),
        )
        .unwrap();
        assert_eq!(top.focus, 0);
        let last = crate::TextSelection::caret(value.len());
        let (bottom, _) = page_caret_focus(
            value,
            last,
            TextCaretIntent::PageDown,
            false,
            None,
            60.0,
            monospace_probe(value),
        )
        .unwrap();
        assert_eq!(bottom.focus, value.len());
        // 其它意图不受支持。
        assert!(
            page_caret_focus(
                value,
                caret,
                TextCaretIntent::Up,
                false,
                None,
                60.0,
                monospace_probe(value),
            )
            .is_none()
        );
    }

    #[test]
    fn logical_page_fallback_moves_fixed_lines_and_clamps() {
        // 20 行文档：固定 15 行页幅下移动并保持列，再移回来。
        let value = (0..20)
            .map(|index| format!("l{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let caret = crate::TextSelection::caret(1);
        let down =
            page_caret_focus_logical(&value, caret, TextCaretIntent::PageDown, false).unwrap();
        let down_line_start = value.find("l15").unwrap();
        assert_eq!(down.focus, down_line_start + 1);
        let up = page_caret_focus_logical(&value, down, TextCaretIntent::PageUp, false).unwrap();
        assert_eq!(up.focus, caret.focus);
        // 短文档钳制到文档末尾。
        let short = "l0\nl1\nl2\nl3";
        let bottom =
            page_caret_focus_logical(short, caret, TextCaretIntent::PageDown, false).unwrap();
        assert_eq!(bottom.focus, short.len());
        assert!(page_caret_focus_logical(short, caret, TextCaretIntent::Left, false).is_none());
    }
}
