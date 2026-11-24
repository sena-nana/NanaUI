use super::caret::{EditorGeometry, clamp_focus};
use super::{AppContext, DocumentId, EditorEdit, FrameworkError, TextFindScope, TextSelection};
use crate::TextContent;
use crate::text_editing::{
    CursorEdit, TextCaretIntent, TextSearchOptions, expanded_selection, find_matches,
    find_matches_in_range, find_next_match, find_previous_match, logical_line_range,
    preserve_case_replacement, replace_all_matches_in_range, vertical_caret_focus,
    vertical_caret_focus_logical, word_range_at,
};

/// The search span for a [`TextFindScope`], when one exists.
fn find_scope_range(selection: &TextSelection) -> Option<(usize, usize)> {
    let range = selection.ordered();
    (range.start < range.end).then_some((range.start, range.end))
}

/// The match list for a find scope on the editor's value: an empty
/// selection under [`TextFindScope::Selection`] falls back to the whole
/// document.
fn find_matches_in_scope(
    value: &str,
    query: &str,
    options: TextSearchOptions,
    scope: TextFindScope,
    selection: &TextSelection,
) -> Vec<std::ops::Range<usize>> {
    match scope {
        TextFindScope::Document => find_matches(value, query, options),
        TextFindScope::Selection => match find_scope_range(selection) {
            Some(range) => find_matches_in_range(value, query, options, range),
            None => find_matches(value, query, options),
        },
    }
}

/// The word/selection text occurrence search runs on: the primary selection
/// itself, or the word at its caret when the selection is bare.
fn occurrence_query(value: &str, selection: TextSelection) -> Option<(usize, usize)> {
    let range = selection.ordered();
    if range.start != range.end {
        return Some((range.start, range.end));
    }
    let (start, end) = word_range_at(value, selection.focus);
    (start != end).then_some((start, end))
}

/// Whether a candidate match intersects any active selection. Bare carets
/// never block an occurrence: a caret inside a word still lets that word be
/// selected (and fuses with it on normalize).
fn is_covered(candidate: &std::ops::Range<usize>, selections: &[TextSelection]) -> bool {
    selections.iter().any(|selection| {
        let span = selection.ordered();
        !span.is_empty() && candidate.start < span.end && span.start < candidate.end
    })
}

/// The next (or previous, wrapping) occurrence of the primary selection's
/// query that no existing cursor covers.
fn next_occurrence(
    value: &str,
    selections: &[TextSelection],
    primary: TextSelection,
    previous: bool,
) -> Option<std::ops::Range<usize>> {
    let (query_start, query_end) = occurrence_query(value, primary)?;
    let query = &value[query_start..query_end];
    if query.chars().all(char::is_whitespace) {
        return None;
    }
    let matches = find_matches(
        value,
        query,
        TextSearchOptions {
            whole_word: true,
            ..TextSearchOptions::default()
        },
    );
    if matches.is_empty() {
        return None;
    }
    let last_end = selections
        .iter()
        .map(|selection| selection.ordered().end)
        .max()
        .unwrap_or(0);
    let first_start = selections
        .iter()
        .map(|selection| selection.ordered().start)
        .min()
        .unwrap_or(0);
    let start_index = if previous {
        matches
            .iter()
            .rposition(|found| found.end <= first_start)
            .unwrap_or(matches.len() - 1)
    } else {
        matches
            .iter()
            .position(|found| found.start >= last_end)
            .unwrap_or(0)
    };
    for step in 0..matches.len() {
        let index = if previous {
            (start_index + matches.len() - step) % matches.len()
        } else {
            (start_index + step) % matches.len()
        };
        if !is_covered(&matches[index], selections) {
            return Some(matches[index].clone());
        }
    }
    None
}

impl AppContext {
    /// Collapse every selection onto the primary cursor (the exit from
    /// multi-cursor mode; hosts decide when to invoke it — Escape is
    /// deliberately not bound). Selection-only move: no change event.
    /// `Ok(false)` means there is no focused editor or only one cursor.
    pub fn collapse_focused_text_selections(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        let state = self.editor_state(focused.node)?;
        if !state.has_additional_selections() {
            return Ok(false);
        }
        self.write_editor_selections(focused.node, state.selection, Vec::new())
    }

    /// 把聚焦文本编辑器的选区设为唯一选区 `start..end`（`start == end`
    /// 为光标；附加光标清除）。偏移先钳到最近的字符边界。区间任一端点
    /// 严格落在折叠隐藏区间内部时该折叠自动展开（与光标导航同源的
    /// reveal 语义，复用框架 display 判定：嵌套折叠只展开当前真正遮住
    /// 端点的父级）。纯选区变更：不发 change 事件（同 move 命令）。
    /// `Ok(false)` 表示没有聚焦编辑器，或选区本就一致。
    pub fn select_focused_text_range(
        &mut self,
        document: DocumentId,
        start: usize,
        end: usize,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        let state = self.editor_state(focused.node)?;
        let start = clamp_focus(&state.value, start);
        let end = clamp_focus(&state.value, end);
        self.text_edit.caret_goal_x = None;
        // 区间两端都参与 reveal（光标导航只看 focus；宿主跳转的起点同样
        // 必须可见）。write_editor_selections 内部还会按 focus 再对账一次，
        // 幂等无害。
        self.unfold_text_folds_containing(focused.node, &[start, end])?;
        self.write_editor_selections(focused.node, TextSelection::new(start, end), Vec::new())
    }

    /// Add one cursor on the visual line above (`above`) or below every
    /// active selection, keeping each cursor's column. Multiline editors
    /// only; hosts without a shaper fall back to logical-line columns.
    /// Targets that already hold a cursor, and document-edge clamps that
    /// would land on the same line, are skipped. Selection-only move.
    pub fn add_focused_text_cursor(
        &mut self,
        document: DocumentId,
        above: bool,
        shaper: Option<&mut dyn crate::TextShaper>,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_selection {
            return Ok(false);
        }
        let state = self.editor_state(focused.node)?;
        let intent = if above {
            TextCaretIntent::Up
        } else {
            TextCaretIntent::Down
        };
        let value = state.value.clone();
        let selections = state.selections().into_owned();
        let mut candidates: Vec<TextSelection> = Vec::with_capacity(selections.len());
        match (shaper, self.world.text_input_shape_context(focused.node)) {
            (Some(shaper), Some((style, constraints))) => {
                let mut geometry = EditorGeometry {
                    shaper,
                    node: focused.node,
                    text: TextContent {
                        value: self.editor_probe_text(focused.node, None),
                    },
                    style,
                    constraints,
                };
                let mut probe = geometry.probe();
                for &selection in &selections {
                    let Some((moved, _)) =
                        vertical_caret_focus(&value, selection, intent, false, None, &mut probe)
                    else {
                        continue;
                    };
                    // Same visual line means there is no line in that
                    // direction; skip instead of stacking on the source.
                    let (_, source_y, _) = probe(clamp_focus(&value, selection.focus));
                    let (_, moved_y, _) = probe(clamp_focus(&value, moved.focus));
                    let distinct = if above {
                        moved_y + f32::EPSILON < source_y
                    } else {
                        moved_y > source_y + f32::EPSILON
                    };
                    if distinct {
                        candidates.push(moved);
                    }
                }
            }
            _ => {
                for &selection in &selections {
                    let Some(moved) =
                        vertical_caret_focus_logical(&value, selection, intent, false)
                    else {
                        continue;
                    };
                    let source_line = logical_line_range(&value, selection.focus).0;
                    let moved_line = logical_line_range(&value, moved.focus).0;
                    let distinct = if above {
                        moved_line < source_line
                    } else {
                        moved_line > source_line
                    };
                    if distinct {
                        candidates.push(moved);
                    }
                }
            }
        }
        if candidates.is_empty() {
            return Ok(false);
        }
        let mut next = state;
        if !next.add_selections(&candidates) {
            return Ok(false);
        }
        self.write_editor_selections(focused.node, next.selection, next.additional_selections)
    }

    /// Select the next (`previous = false`) or previous literal occurrence of
    /// the primary selection's text — or of the word at the primary caret
    /// when it is bare — skipping spans that already hold a cursor and
    /// wrapping around the document. Multiline editors only.
    /// Selection-only move: no change event.
    pub fn select_focused_text_occurrence(
        &mut self,
        document: DocumentId,
        previous: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_selection {
            return Ok(false);
        }
        let state = self.editor_state(focused.node)?;
        let Some(found) =
            next_occurrence(&state.value, &state.selections(), state.selection, previous)
        else {
            return Ok(false);
        };
        let mut next = state;
        if !next.add_selections(&[TextSelection::new(found.start, found.end)]) {
            return Ok(false);
        }
        self.write_editor_selections(focused.node, next.selection, next.additional_selections)
    }

    /// Select every literal occurrence of the primary selection's text (or of
    /// the word at the bare primary caret) that does not already hold a
    /// cursor. Multiline editors only. Selection-only move: no change event.
    pub fn select_all_focused_text_occurrences(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_selection {
            return Ok(false);
        }
        let state = self.editor_state(focused.node)?;
        let selections = state.selections().into_owned();
        let Some((query_start, query_end)) = occurrence_query(&state.value, state.selection) else {
            return Ok(false);
        };
        let query = &state.value[query_start..query_end];
        let matches = find_matches(
            &state.value,
            query,
            TextSearchOptions {
                whole_word: true,
                ..TextSearchOptions::default()
            },
        );
        let candidates: Vec<TextSelection> = matches
            .iter()
            .filter(|found| !is_covered(found, &selections))
            .map(|found| TextSelection::new(found.start, found.end))
            .collect();
        if candidates.is_empty() {
            return Ok(false);
        }
        let mut next = state;
        if !next.add_selections(&candidates) {
            return Ok(false);
        }
        self.write_editor_selections(focused.node, next.selection, next.additional_selections)
    }

    /// Expand every selection of the focused editor one level — collapsed
    /// caret to its word, then to the interior of the innermost enclosing
    /// bracket pair, outward layer by layer, finally the whole document (see
    /// [`expanded_selection`]). Applied per cursor; overlapping results fuse
    /// through the normal selection write. Selection-only move. The prior
    /// selection set is remembered for
    /// [`AppContext::shrink_focused_text_selection`] and invalidated by any
    /// value edit. `Ok(false)` means no focused editor or nothing grew.
    pub fn expand_focused_text_selection(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_selection {
            return Ok(false);
        }
        let state = self.editor_state(focused.node)?;
        let selections = state.selections().into_owned();
        let primary_index = selections
            .iter()
            .position(|selection| *selection == state.selection)
            .unwrap_or(0);
        let mut next_selections = Vec::with_capacity(selections.len());
        let mut grew = false;
        for &selection in &selections {
            let next = expanded_selection(&state.value, selection).unwrap_or(selection);
            grew |= next != selection;
            next_selections.push(next);
        }
        if !grew {
            return Ok(false);
        }
        let entry = (state.selection, state.additional_selections.clone());
        match &mut self.text_edit.selection_expansions {
            Some((node, history, value)) if *node == focused.node && *value == state.value => {
                history.push(entry);
            }
            _ => {
                self.text_edit.selection_expansions =
                    Some((focused.node, vec![entry], state.value.to_string()));
            }
        }
        self.text_edit.caret_goal_x = None;
        let primary = next_selections[primary_index];
        let additional = next_selections
            .into_iter()
            .enumerate()
            .filter(|(index, _)| *index != primary_index)
            .map(|(_, selection)| selection)
            .collect();
        self.write_editor_selections(focused.node, primary, additional)
    }

    /// Undo one [`AppContext::expand_focused_text_selection`] step: restore
    /// the selection set from before the last expansion. The expansion record
    /// dies when the editor's value changes (any edit), when a different
    /// editor is focused, or once every recorded step is consumed.
    /// Selection-only move. `Ok(false)` means there is nothing to shrink to.
    pub fn shrink_focused_text_selection(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        let state = self.editor_state(focused.node)?;
        match &mut self.text_edit.selection_expansions {
            Some((node, history, value))
                if *node == focused.node && **value == state.value && !history.is_empty() =>
            {
                let (primary, additional) = history.pop().expect("non-empty checked above");
                self.text_edit.caret_goal_x = None;
                self.write_editor_selections(focused.node, primary, additional)
            }
            _ => {
                self.text_edit.selection_expansions = None;
                Ok(false)
            }
        }
    }

    /// Select the next literal match of `query` in the focused text editor,
    /// searching from the selection's end and wrapping to the first in-scope
    /// match. With [`TextFindScope::Selection`] the navigation loop stays
    /// inside the primary selection (an empty selection searches the whole
    /// document). Wrapping never leaves the scope: the match list only
    /// contains in-scope matches.
    ///
    /// Selection-only move: like [`AppContext::move_focused_text_caret`], no
    /// change event is emitted. `Ok(false)` means there is no focused editor
    /// or no match.
    pub fn find_next_focused_text_match(
        &mut self,
        document: DocumentId,
        query: &str,
        options: TextSearchOptions,
        scope: TextFindScope,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_selection {
            return Ok(false);
        }
        let state = self.editor_state(focused.node)?;
        let matches = find_matches_in_scope(&state.value, query, options, scope, &state.selection);
        let Some(found) = find_next_match(&matches, state.selection.ordered().end) else {
            return Ok(false);
        };
        self.text_edit.caret_goal_x = None;
        self.write_editor_selection(focused.node, TextSelection::new(found.start, found.end))
    }

    /// Select the previous literal match of `query` in the focused text
    /// editor, searching from the selection's start and wrapping to the last
    /// in-scope match; see [`AppContext::find_next_focused_text_match`] for
    /// scoping. Selection-only move.
    pub fn find_previous_focused_text_match(
        &mut self,
        document: DocumentId,
        query: &str,
        options: TextSearchOptions,
        scope: TextFindScope,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_selection {
            return Ok(false);
        }
        let state = self.editor_state(focused.node)?;
        let matches = find_matches_in_scope(&state.value, query, options, scope, &state.selection);
        let Some(found) = find_previous_match(&matches, state.selection.ordered().start) else {
            return Ok(false);
        };
        self.text_edit.caret_goal_x = None;
        self.write_editor_selection(focused.node, TextSelection::new(found.start, found.end))
    }

    /// Replace the focused editor's selection with `replacement` when the
    /// selection is exactly a match of `query` under `options`, then select
    /// the inserted text (so a following replace targets the same span
    /// semantics) and emit the change event. Advancing to the next match is
    /// the host's call — compose with
    /// [`AppContext::find_next_focused_text_match`]. With `preserve_case`
    /// the replacement text follows the matched text's case shape (see
    /// [`preserve_case_replacement`]); `Ok(false)` means there is no focused
    /// editor or the selection is not a match.
    ///
    /// Primary-cursor semantics: only the primary selection replaces; other
    /// cursors survive through offset remapping.
    pub fn replace_focused_text_match(
        &mut self,
        document: DocumentId,
        query: &str,
        options: TextSearchOptions,
        replacement: &str,
        preserve_case: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_input {
            return Ok(false);
        }
        self.text_edit.caret_goal_x = None;
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Structural,
            |value, selection, is_primary| {
                if !is_primary {
                    return None;
                }
                let range = selection.ordered();
                let is_match = find_matches(value, query, options).contains(&range);
                if !is_match {
                    return None;
                }
                let text = if preserve_case {
                    preserve_case_replacement(replacement, &value[range.clone()])
                } else {
                    replacement.to_owned()
                };
                let mut next =
                    String::with_capacity(value.len() - range.len().min(value.len()) + text.len());
                next.push_str(&value[..range.start]);
                next.push_str(&text);
                next.push_str(&value[range.end..]);
                Some(CursorEdit::Transform {
                    next,
                    selection: TextSelection::new(range.start, range.start + text.len()),
                })
            },
        )
    }

    /// Replace every literal match of `query` in the focused editor with
    /// `replacement` (left-to-right, non-overlapping on the original text),
    /// select the first replacement, emit one change event, and return the
    /// replacement count. [`TextFindScope::Selection`] restricts matching to
    /// the primary selection (an empty selection replaces document-wide);
    /// `preserve_case` follows [`AppContext::replace_focused_text_match`].
    /// `Ok(0)` leaves the editor untouched.
    pub fn replace_all_focused_text_matches(
        &mut self,
        document: DocumentId,
        query: &str,
        options: TextSearchOptions,
        replacement: &str,
        scope: TextFindScope,
        preserve_case: bool,
    ) -> Result<usize, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(0);
        };
        if !focused.accepts_input {
            return Ok(0);
        }
        self.text_edit.caret_goal_x = None;
        let mut replaced = 0usize;
        let applied =
            self.edit_editor(focused.node, crate::TextEditOrigin::Structural, |state| {
                let matches =
                    find_matches_in_scope(&state.value, query, options, scope, &state.selection);
                if matches.is_empty() {
                    return None;
                }
                let first = matches[0].clone();
                let first_replacement = if preserve_case {
                    preserve_case_replacement(replacement, &state.value[first.clone()])
                } else {
                    replacement.to_owned()
                };
                let (value, count) = replace_all_matches_in_range(
                    &state.value,
                    query,
                    replacement,
                    options,
                    match scope {
                        TextFindScope::Document => None,
                        TextFindScope::Selection => find_scope_range(&state.selection),
                    },
                    preserve_case,
                );
                replaced = count;
                Some(EditorEdit {
                    value,
                    selection: TextSelection::new(
                        first.start,
                        first.start + first_replacement.len(),
                    ),
                })
            })?;
        Ok(if applied { replaced } else { 0 })
    }
}
