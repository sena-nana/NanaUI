//! GameTextInput buffer → [`ImeEvent`] mapping (host-testable).
//!
//! GameActivity's InputConnection owns a full text buffer. Nana's editor
//! authority stays on Runtime `TextInput`; this module only diffs the buffer
//! into [`RuntimeInputAdapter::dispatch_ime`] events. It is not a second
//! text state machine.

use nana_ui_platform::ImeEvent;

/// Android `InputType` / `EditorInfo` bits mirrored for host tests.
pub const TYPE_CLASS_TEXT: u32 = 1;
pub const TYPE_TEXT_VARIATION_PASSWORD: u32 = 0x00000080;
pub const TYPE_TEXT_FLAG_MULTI_LINE: u32 = 0x00020000;
pub const TYPE_TEXT_FLAG_IME_MULTI_LINE: u32 = 0x00040000;
pub const IME_ACTION_NONE: i32 = 1;
pub const IME_ACTION_DONE: i32 = 6;
pub const IME_FLAG_NO_FULLSCREEN: u32 = 0x02000000;

/// Snapshot of GameTextInput / InputConnection state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlotImeBuffer {
    pub text: String,
    pub selection_start: usize,
    pub selection_end: usize,
    pub compose: Option<(usize, usize)>,
}

/// `EditorInfo` subset mirrored from [`nana_ui_platform::TextInputRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotEditorInfo {
    pub input_type: u32,
    pub action: i32,
    pub ime_options: u32,
}

/// Password / multiline flags from the Runtime focus mirror.
pub fn editor_info_from_request(password: bool, multiline: bool) -> SlotEditorInfo {
    let mut input_type = TYPE_CLASS_TEXT;
    if password {
        input_type |= TYPE_TEXT_VARIATION_PASSWORD;
    }
    if multiline {
        input_type |= TYPE_TEXT_FLAG_MULTI_LINE | TYPE_TEXT_FLAG_IME_MULTI_LINE;
    }
    SlotEditorInfo {
        input_type,
        action: if multiline {
            IME_ACTION_NONE
        } else {
            IME_ACTION_DONE
        },
        ime_options: IME_FLAG_NO_FULLSCREEN,
    }
}

/// Diff `previous` → `next` into desktop IME events.
///
/// Composing spans become [`ImeEvent::Preedit`]; a composition that ends
/// becomes [`ImeEvent::Commit`]; committed-only edits become
/// [`ImeEvent::DeleteSurrounding`] and/or [`ImeEvent::Commit`]. Selection-only
/// updates produce no events.
pub fn ime_events_from_buffer_delta(
    previous: &SlotImeBuffer,
    next: &SlotImeBuffer,
) -> Vec<ImeEvent> {
    let previous = previous.clamped();
    let next = next.clamped();
    if previous == next {
        return Vec::new();
    }

    if let Some(preedit) = next.compose_text() {
        let mut events = Vec::new();
        let next_committed = without_compose(&next);
        if previous.compose.is_some() {
            let prev_committed = without_compose(&previous);
            if prev_committed.text != next_committed.text {
                match composition_replacement(&previous, &next_committed) {
                    Some(committed) => events.push(ImeEvent::Commit(committed)),
                    None => events.extend(committed_edit_events(&prev_committed, &next_committed)),
                }
            }
        } else if previous.text != next_committed.text {
            events.extend(committed_edit_events(&previous, &next_committed));
        }
        let selection = next.compose.map(|(start, _)| {
            let rel_start = next
                .selection_start
                .saturating_sub(start)
                .min(preedit.len());
            let rel_end = next.selection_end.saturating_sub(start).min(preedit.len());
            let (rel_start, rel_end) = order(rel_start, rel_end);
            (
                floor_boundary(&preedit, rel_start),
                floor_boundary(&preedit, rel_end),
            )
        });
        events.push(ImeEvent::Preedit {
            text: preedit,
            selection,
        });
        return events;
    }

    if previous.compose.is_some() {
        return match composition_replacement(&previous, &next) {
            Some(committed) => vec![ImeEvent::Commit(committed)],
            // Nothing in `next` lines up with the text that surrounded the
            // composition, so nothing identifies what replaced it. Committing
            // the old preedit here would put back text the IME just abandoned;
            // diff from the composition already removed instead.
            None => committed_edit_events(&without_compose(&previous), &next),
        };
    }

    committed_edit_events(&previous, &next)
}

impl SlotImeBuffer {
    fn clamped(&self) -> Self {
        let selection_start = floor_boundary(&self.text, self.selection_start.min(self.text.len()));
        let selection_end = floor_boundary(&self.text, self.selection_end.min(self.text.len()));
        let compose = self.compose.and_then(|(start, end)| {
            let start = floor_boundary(&self.text, start.min(self.text.len()));
            let end = floor_boundary(&self.text, end.min(self.text.len()));
            if start == end {
                None
            } else {
                Some(order(start, end))
            }
        });
        Self {
            text: self.text.clone(),
            selection_start,
            selection_end,
            compose,
        }
    }

    fn compose_text(&self) -> Option<String> {
        let (start, end) = self.compose?;
        Some(self.text[start..end].to_string())
    }
}

/// What replaced `previous`'s composing span, read off the text that surrounded
/// it. `None` when that surrounding text moved as well and the span can no
/// longer be located.
fn composition_replacement(previous: &SlotImeBuffer, next: &SlotImeBuffer) -> Option<String> {
    let (start, end) = previous.compose?;
    let before = &previous.text[..start];
    let after = &previous.text[end..];
    if !next.text.starts_with(before) || !next.text.ends_with(after) {
        return None;
    }
    let mid_end = next.text.len().saturating_sub(after.len());
    (mid_end >= before.len()).then(|| next.text[before.len()..mid_end].to_string())
}

fn without_compose(buffer: &SlotImeBuffer) -> SlotImeBuffer {
    let Some((start, end)) = buffer.compose else {
        return buffer.clone();
    };
    let mut text =
        String::with_capacity(buffer.text.len().saturating_sub(end.saturating_sub(start)));
    text.push_str(&buffer.text[..start]);
    text.push_str(&buffer.text[end..]);
    let adjust = |index: usize| {
        if index <= start {
            index
        } else if index >= end {
            index - (end - start)
        } else {
            start
        }
    };
    SlotImeBuffer {
        text,
        selection_start: adjust(buffer.selection_start),
        selection_end: adjust(buffer.selection_end),
        compose: None,
    }
}

fn committed_edit_events(previous: &SlotImeBuffer, next: &SlotImeBuffer) -> Vec<ImeEvent> {
    if previous.text == next.text {
        return Vec::new();
    }
    let prefix = common_prefix_len(&previous.text, &next.text);
    let suffix = common_suffix_len(&previous.text, &next.text, prefix);
    let prev_mid_end = previous.text.len().saturating_sub(suffix);
    let next_mid_end = next.text.len().saturating_sub(suffix);
    let deleted = if prefix <= prev_mid_end {
        &previous.text[prefix..prev_mid_end]
    } else {
        ""
    };
    let inserted = if prefix <= next_mid_end {
        &next.text[prefix..next_mid_end]
    } else {
        ""
    };

    let mut events = Vec::new();
    if !deleted.is_empty() {
        let caret = previous.selection_end.min(previous.text.len());
        let before_bytes = caret.saturating_sub(prefix).min(deleted.len());
        let after_bytes = deleted.len().saturating_sub(before_bytes);
        events.push(ImeEvent::DeleteSurrounding {
            before_bytes,
            after_bytes,
        });
    }
    if !inserted.is_empty() {
        events.push(ImeEvent::Commit(inserted.to_string()));
    }
    events
}

fn floor_boundary(text: &str, index: usize) -> usize {
    let index = index.min(text.len());
    if text.is_char_boundary(index) {
        return index;
    }
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        if text.is_char_boundary(cursor) {
            return cursor;
        }
    }
    0
}

fn order(a: usize, b: usize) -> (usize, usize) {
    if a <= b { (a, b) } else { (b, a) }
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    let mut i = 0;
    let max = a.len().min(b.len());
    while i < max && a.as_bytes()[i] == b.as_bytes()[i] {
        i += 1;
    }
    while i > 0 && (!a.is_char_boundary(i) || !b.is_char_boundary(i)) {
        i -= 1;
    }
    i
}

fn common_suffix_len(a: &str, b: &str, prefix: usize) -> usize {
    let a = &a[prefix..];
    let b = &b[prefix..];
    let mut i = 0;
    let max = a.len().min(b.len());
    while i < max && a.as_bytes()[a.len() - 1 - i] == b.as_bytes()[b.len() - 1 - i] {
        i += 1;
    }
    while i > 0 && (!a.is_char_boundary(a.len() - i) || !b.is_char_boundary(b.len() - i)) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(text: &str, caret: usize, compose: Option<(usize, usize)>) -> SlotImeBuffer {
        SlotImeBuffer {
            text: text.into(),
            selection_start: caret,
            selection_end: caret,
            compose,
        }
    }

    #[test]
    fn password_and_multiline_mirror_editor_info() {
        let normal = editor_info_from_request(false, false);
        assert_eq!(normal.input_type, TYPE_CLASS_TEXT);
        assert_eq!(normal.action, IME_ACTION_DONE);
        let password = editor_info_from_request(true, false);
        assert_eq!(
            password.input_type,
            TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_PASSWORD
        );
        let multiline = editor_info_from_request(false, true);
        assert_eq!(
            multiline.input_type,
            TYPE_CLASS_TEXT | TYPE_TEXT_FLAG_MULTI_LINE | TYPE_TEXT_FLAG_IME_MULTI_LINE
        );
        assert_eq!(multiline.action, IME_ACTION_NONE);
    }

    /// An IME that abandons a composition while also editing the text around
    /// it leaves nothing that locates the old span. Committing the preedit
    /// anyway would put back the very characters the user just discarded.
    #[test]
    fn an_abandoned_composition_is_not_committed_back() {
        let prev = buffer("ab\u{4e16}", 5, Some((2, 5)));
        let next = buffer("b", 1, None);
        let events = ime_events_from_buffer_delta(&prev, &next);
        assert!(
            !events.iter().any(|event| matches!(
                event,
                ImeEvent::Commit(text) if text.contains('\u{4e16}')
            )),
            "the discarded preedit must not come back: {events:?}"
        );
        assert_eq!(
            events,
            committed_edit_events(&buffer("ab", 2, None), &next),
            "the edit must be read off the committed text instead"
        );
    }

    #[test]
    fn composing_span_becomes_preedit() {
        let prev = buffer("", 0, None);
        let next = buffer("你", 3, Some((0, 3)));
        assert_eq!(
            ime_events_from_buffer_delta(&prev, &next),
            vec![ImeEvent::Preedit {
                text: "你".into(),
                selection: Some((3, 3)),
            }]
        );
    }

    #[test]
    fn ending_composition_commits_replacement() {
        let prev = buffer("你", 3, Some((0, 3)));
        let next = buffer("你", 3, None);
        assert_eq!(
            ime_events_from_buffer_delta(&prev, &next),
            vec![ImeEvent::Commit("你".into())]
        );
    }

    #[test]
    fn ascii_insert_commits_without_a_second_buffer() {
        let prev = buffer("na", 2, None);
        let next = buffer("nan", 3, None);
        assert_eq!(
            ime_events_from_buffer_delta(&prev, &next),
            vec![ImeEvent::Commit("n".into())]
        );
    }

    #[test]
    fn backspace_deletes_surrounding_committed_text() {
        let prev = buffer("nan", 3, None);
        let next = buffer("na", 2, None);
        assert_eq!(
            ime_events_from_buffer_delta(&prev, &next),
            vec![ImeEvent::DeleteSurrounding {
                before_bytes: 1,
                after_bytes: 0,
            }]
        );
    }

    #[test]
    fn selection_only_change_is_silent() {
        let prev = buffer("nana", 4, None);
        let next = SlotImeBuffer {
            text: "nana".into(),
            selection_start: 0,
            selection_end: 4,
            compose: None,
        };
        assert!(ime_events_from_buffer_delta(&prev, &next).is_empty());
    }

    #[test]
    fn identical_buffers_are_silent() {
        let buf = buffer("nana", 2, None);
        assert!(ime_events_from_buffer_delta(&buf, &buf).is_empty());
    }

    #[test]
    fn committing_then_starting_the_next_compose_emits_commit_before_preedit() {
        let prev = buffer("你", 3, Some((0, 3)));
        let next = buffer("你好", 6, Some((3, 6)));
        assert_eq!(
            ime_events_from_buffer_delta(&prev, &next),
            vec![
                ImeEvent::Commit("你".into()),
                ImeEvent::Preedit {
                    text: "好".into(),
                    selection: Some((3, 3)),
                }
            ]
        );
    }

    #[test]
    fn compose_replacement_without_a_committed_change_is_preedit_only() {
        let prev = buffer("你", 3, Some((0, 3)));
        let next = buffer("您", 3, Some((0, 3)));
        assert_eq!(
            ime_events_from_buffer_delta(&prev, &next),
            vec![ImeEvent::Preedit {
                text: "您".into(),
                selection: Some((3, 3)),
            }]
        );
    }
}
