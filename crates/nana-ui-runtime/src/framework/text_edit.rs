//! Text-editor caret, selection, and code-editing commands on [`AppContext`].
//!
//! Keyboard and pointer adapters resolve intents and hand them here; this
//! module owns which focused component is editable, how offsets map onto the
//! committed value, and when change events fire. Geometry-dependent queries
//! receive the host shaper so layout stays backend-owned.

use super::{AppContext, DocumentId, EditableText, Entity, FrameworkError, StableNodeId};
use super::{TextArea, TextInput, TextInputState, TextSelection};
use crate::text_editing::{
    CursorEdit, TextCaretIntent, TextLineDirection, TextReplacement, TextSearchOptions,
    apply_cursor_edits, apply_replacement, auto_indent_newline, auto_pair_edit, caret_focus,
    caret_offset_at_point, delete_backward, delete_forward, delete_lines, delete_to_line_end,
    delete_to_line_start, delete_word_backward, delete_word_forward, duplicate_lines, find_matches,
    find_next_match, find_previous_match, indent_selection, join_lines, logical_line_range,
    matching_bracket_pair, move_lines, moved_selection, outdent_selection, page_caret_focus,
    page_caret_focus_logical, replace_all_matches, sort_lines, toggle_line_comment,
    transform_selection_case, vertical_caret_focus, vertical_caret_focus_logical, word_range_at,
};
use crate::{CodeEditing, TextContent, TextShapeConstraints};

/// Delete semantics for [`AppContext::delete_focused_text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDeleteKind {
    Backward,
    Forward,
    WordBackward,
    WordForward,
    LineStart,
    LineEnd,
}

/// The focused plain text editor, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct FocusedTextEditor {
    pub node: StableNodeId,
    pub multiline: bool,
    pub accepts_input: bool,
    pub code_editing: Option<CodeEditing>,
    kind: TextEditorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextEditorKind {
    Area,
    Field,
}

/// A [`crate::TextShaper`] probe bound to one editor node's shaped layout.
struct EditorGeometry<'a> {
    shaper: &'a mut dyn crate::TextShaper,
    node: StableNodeId,
    text: TextContent,
    style: crate::ComputedStyle,
    constraints: TextShapeConstraints,
}

impl EditorGeometry<'_> {
    fn probe(&mut self) -> impl FnMut(usize) -> (f32, f32, f32) + '_ {
        move |offset: usize| {
            self.shaper
                .text_position(self.node, &self.text, offset, &self.style, self.constraints)
        }
    }

    /// Document-local pointer coordinates as a content-local point.
    fn localize(
        content: crate::LayoutBox,
        scroll: crate::ScrollOffset,
        x: f32,
        y: f32,
    ) -> (f32, f32) {
        (x - content.x + scroll.x, y - content.y + scroll.y)
    }
}

/// Nearest char boundary at or below `offset` (clamped to the value length).
fn clamp_focus(value: &str, offset: usize) -> usize {
    crate::text_editing::clamp_boundary(value, offset)
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

/// One committed value edit: the next value and where the caret lands.
struct EditorEdit {
    value: String,
    selection: TextSelection,
}

impl From<(String, usize)> for EditorEdit {
    fn from((value, caret): (String, usize)) -> Self {
        EditorEdit {
            value,
            selection: TextSelection::caret(caret),
        }
    }
}

impl From<(String, TextSelection)> for EditorEdit {
    fn from((value, selection): (String, TextSelection)) -> Self {
        EditorEdit { value, selection }
    }
}

impl AppContext {
    /// Identify the focused plain text editor (`TextArea` or `TextInput`).
    ///
    /// Composite search surfaces (palettes, menus, dropdowns) own their
    /// navigation and are deliberately not plain editors.
    pub fn focused_text_editor(&self, document: DocumentId) -> Option<FocusedTextEditor> {
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.editor_info(entity, TextEditorKind::Area);
        }
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.editor_info(entity, TextEditorKind::Field);
        }
        None
    }

    fn editor_info<C: EditableText>(
        &self,
        entity: Entity<C>,
        kind: TextEditorKind,
    ) -> Option<FocusedTextEditor> {
        let node = entity.id;
        self.read(entity, |editable: &C| FocusedTextEditor {
            node,
            multiline: editable.is_multiline(),
            accepts_input: editable.accepts_input(),
            code_editing: editable.code_editing().cloned(),
            kind,
        })
        .ok()
    }

    /// Resolve one caret move for one selection. Vertical intents use the
    /// host shaper when available and fall back to logical-line column
    /// preservation otherwise; single-line fields map vertical intents onto
    /// the line boundaries. Returns the next selection plus the goal column
    /// to retain for chained vertical moves.
    #[allow(clippy::too_many_arguments)]
    fn resolve_caret_move(
        value: &str,
        selection: TextSelection,
        intent: TextCaretIntent,
        extend: bool,
        multiline: bool,
        mut geometry: Option<&mut EditorGeometry<'_>>,
        page_height: f32,
    ) -> Option<(TextSelection, Option<f32>)> {
        let vertical = matches!(
            intent,
            TextCaretIntent::Up
                | TextCaretIntent::Down
                | TextCaretIntent::PageUp
                | TextCaretIntent::PageDown
        );
        if vertical && multiline {
            if let Some(geometry) = geometry.as_deref_mut() {
                let moved = if matches!(intent, TextCaretIntent::Up | TextCaretIntent::Down) {
                    vertical_caret_focus(value, selection, intent, extend, None, geometry.probe())
                } else {
                    // One viewport height of visual lines: the content box is
                    // the editor's viewport.
                    page_caret_focus(
                        value,
                        selection,
                        intent,
                        extend,
                        None,
                        page_height,
                        geometry.probe(),
                    )
                };
                return moved.map(|(selection, goal)| (selection, Some(goal)));
            }
            let moved = if matches!(intent, TextCaretIntent::Up | TextCaretIntent::Down) {
                vertical_caret_focus_logical(value, selection, intent, extend)
            } else {
                page_caret_focus_logical(value, selection, intent, extend)
            };
            return moved.map(|selection| (selection, None));
        }
        if vertical {
            // Single-line fields map Up/Down and PageUp/PageDown onto the
            // line boundaries.
            let mapped = match intent {
                TextCaretIntent::Up | TextCaretIntent::PageUp => TextCaretIntent::LineStart,
                _ => TextCaretIntent::LineEnd,
            };
            return caret_focus(value, selection, mapped)
                .map(|focus| (moved_selection(selection, focus, extend), None));
        }
        caret_focus(value, selection, intent)
            .map(|focus| (moved_selection(selection, focus, extend), None))
    }

    /// Move the caret or selection edge of the focused text editor.
    ///
    /// With multiple cursors every selection moves by the same intent; the
    /// shared goal column only tracks the primary cursor.
    pub fn move_focused_text_caret(
        &mut self,
        document: DocumentId,
        intent: TextCaretIntent,
        extend: bool,
        mut shaper: Option<&mut dyn crate::TextShaper>,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_input {
            return Ok(false);
        }
        let state = self.editor_state(focused.node, focused.kind)?;
        let vertical = matches!(
            intent,
            TextCaretIntent::Up
                | TextCaretIntent::Down
                | TextCaretIntent::PageUp
                | TextCaretIntent::PageDown
        );
        if state.has_additional_selections() {
            self.caret_goal_x = None;
            let page_height = self
                .world
                .text_input_pointer_context(focused.node)
                .map_or(0.0, |(content, _)| content.height);
            let shape_context = if vertical && focused.multiline && shaper.is_some() {
                self.world.text_input_shape_context(focused.node)
            } else {
                None
            };
            let mut geometry =
                shape_context
                    .zip(shaper.as_deref_mut())
                    .map(|((style, constraints), shaper)| EditorGeometry {
                        shaper,
                        node: focused.node,
                        text: TextContent {
                            value: state.value.clone(),
                        },
                        style,
                        constraints,
                    });
            let selections = state.selections().into_owned();
            let primary_index = selections
                .iter()
                .position(|selection| *selection == state.selection)
                .unwrap_or(0);
            let mut next_selections = Vec::with_capacity(selections.len());
            let mut next_goal = None;
            let mut moved_any = false;
            for (index, &selection) in selections.iter().enumerate() {
                match Self::resolve_caret_move(
                    &state.value,
                    selection,
                    intent,
                    extend,
                    focused.multiline,
                    geometry.as_mut(),
                    page_height,
                ) {
                    Some((moved, goal)) => {
                        moved_any = true;
                        if index == primary_index {
                            next_goal = goal;
                        }
                        next_selections.push(moved);
                    }
                    None => next_selections.push(selection),
                }
            }
            if !moved_any {
                return Ok(false);
            }
            if let Some(goal) = next_goal {
                self.caret_goal_x = Some((focused.node, goal));
            }
            let primary = next_selections[primary_index];
            let additional = next_selections
                .into_iter()
                .enumerate()
                .filter(|(index, _)| *index != primary_index)
                .map(|(_, selection)| selection)
                .collect();
            return self.write_editor_selections(focused.node, focused.kind, primary, additional);
        }
        let selection = if vertical && focused.multiline {
            let goal = self
                .caret_goal_x
                .take()
                .filter(|(id, _)| *id == focused.node)
                .map(|(_, x)| x);
            if let (Some(shaper), Some((style, constraints))) = (
                shaper.as_deref_mut(),
                self.world.text_input_shape_context(focused.node),
            ) {
                let mut geometry = EditorGeometry {
                    shaper,
                    node: focused.node,
                    text: TextContent {
                        value: state.value.clone(),
                    },
                    style,
                    constraints,
                };
                let moved = if matches!(intent, TextCaretIntent::Up | TextCaretIntent::Down) {
                    vertical_caret_focus(
                        &state.value,
                        state.selection,
                        intent,
                        extend,
                        goal,
                        geometry.probe(),
                    )
                } else {
                    // One viewport height of visual lines: the content box
                    // is the editor's viewport.
                    let page_height = self
                        .world
                        .text_input_pointer_context(focused.node)
                        .map_or(0.0, |(content, _)| content.height);
                    page_caret_focus(
                        &state.value,
                        state.selection,
                        intent,
                        extend,
                        goal,
                        page_height,
                        geometry.probe(),
                    )
                };
                match moved {
                    Some((selection, goal)) => {
                        self.caret_goal_x = Some((focused.node, goal));
                        selection
                    }
                    None => return Ok(false),
                }
            } else {
                let moved = if matches!(intent, TextCaretIntent::Up | TextCaretIntent::Down) {
                    vertical_caret_focus_logical(&state.value, state.selection, intent, extend)
                } else {
                    page_caret_focus_logical(&state.value, state.selection, intent, extend)
                };
                match moved {
                    Some(selection) => selection,
                    None => return Ok(false),
                }
            }
        } else if vertical {
            // Single-line fields map Up/Down and PageUp/PageDown onto the
            // line boundaries.
            let mapped = match intent {
                TextCaretIntent::Up | TextCaretIntent::PageUp => TextCaretIntent::LineStart,
                _ => TextCaretIntent::LineEnd,
            };
            match caret_focus(&state.value, state.selection, mapped) {
                Some(focus) => moved_selection(state.selection, focus, extend),
                None => return Ok(false),
            }
        } else {
            self.caret_goal_x = None;
            match caret_focus(&state.value, state.selection, intent) {
                Some(focus) => moved_selection(state.selection, focus, extend),
                None => return Ok(false),
            }
        };
        self.write_editor_selection(focused.node, focused.kind, selection)
    }

    /// Delete around the caret(s) of the focused text editor. With multiple
    /// cursors every cursor deletes by the same kind.
    pub fn delete_focused_text(
        &mut self,
        document: DocumentId,
        kind: TextDeleteKind,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        self.edit_editor_multi(focused.node, focused.kind, |value, selection, _| {
            let replacement = match kind {
                TextDeleteKind::Backward => delete_backward(value, selection)?,
                TextDeleteKind::Forward => delete_forward(value, selection)?,
                TextDeleteKind::WordBackward => delete_word_backward(value, selection)?,
                TextDeleteKind::WordForward => delete_word_forward(value, selection)?,
                TextDeleteKind::LineStart => delete_to_line_start(value, selection)?,
                TextDeleteKind::LineEnd => delete_to_line_end(value, selection)?,
            };
            Some(CursorEdit::Span(replacement))
        })
    }

    /// Insert a newline into the focused `TextArea`, with auto-indent when
    /// code editing is enabled. Single-line fields never accept newlines.
    pub fn insert_focused_text_newline(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        let indent_unit = focused
            .code_editing
            .as_ref()
            .map(|code| code.indent_unit.to_string());
        self.edit_editor_multi(focused.node, focused.kind, |value, selection, _| {
            let replacement = match &indent_unit {
                Some(unit) => auto_indent_newline(value, selection, unit),
                None => {
                    let range = selection.ordered();
                    let caret = range.start + 1;
                    TextReplacement {
                        range,
                        insert: "\n".into(),
                        caret,
                    }
                }
            };
            Some(CursorEdit::Span(replacement))
        })
    }

    /// Apply code-editor auto-pairing for a typed character. `Ok(false)`
    /// means the character was not consumed and should type normally.
    pub fn code_edit_typed(
        &mut self,
        document: DocumentId,
        typed: char,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if focused.code_editing.is_none() || !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        self.edit_editor_multi(focused.node, focused.kind, |value, selection, _| {
            auto_pair_edit(value, selection, typed).map(CursorEdit::Span)
        })
    }

    /// Indent (`Tab`) or outdent (`Shift+Tab`) the focused code editor.
    pub fn code_edit_indent(
        &mut self,
        document: DocumentId,
        outdent: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        let Some(code) = focused.code_editing else {
            return Ok(false);
        };
        if !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        let unit = code.indent_unit.to_string();
        self.edit_editor_multi(focused.node, focused.kind, |value, selection, _| {
            let (next, selection) = if outdent {
                outdent_selection(value, selection, &unit)?
            } else {
                indent_selection(value, selection, &unit)?
            };
            Some(CursorEdit::Transform { next, selection })
        })
    }

    /// Toggle line comments across the selections of the focused code editor.
    pub fn code_edit_toggle_comment(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        let Some(code) = focused.code_editing else {
            return Ok(false);
        };
        if !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        let prefix = code.comment_prefix.to_string();
        self.edit_editor_multi(focused.node, focused.kind, |value, selection, _| {
            let (next, selection) = toggle_line_comment(value, selection, &prefix)?;
            Some(CursorEdit::Transform { next, selection })
        })
    }

    /// Move the block of lines each selection touches up or down one line,
    /// with each selection following its moved text. `Ok(false)` means there
    /// is no focused multiline editor or no block can move.
    pub fn move_focused_text_lines(
        &mut self,
        document: DocumentId,
        direction: TextLineDirection,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        self.edit_editor_multi(focused.node, focused.kind, |value, selection, _| {
            let (next, selection) = move_lines(value, selection, direction)?;
            Some(CursorEdit::Transform { next, selection })
        })
    }

    /// Duplicate the block of lines each selection touches on the line below
    /// and select each copy.
    pub fn duplicate_focused_text_lines(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        self.edit_editor_multi(focused.node, focused.kind, |value, selection, _| {
            let (next, selection) = duplicate_lines(value, selection)?;
            Some(CursorEdit::Transform { next, selection })
        })
    }

    /// Delete the block of lines each selection touches, including one
    /// adjacent newline.
    pub fn delete_focused_text_lines(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        self.edit_editor_multi(focused.node, focused.kind, |value, selection, _| {
            let (next, selection) = delete_lines(value, selection)?;
            Some(CursorEdit::Transform { next, selection })
        })
    }

    /// Join the lines each selection touches into one line (single-space
    /// seams, following lines' indentation removed).
    pub fn join_focused_text_lines(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        self.edit_editor_multi(focused.node, focused.kind, |value, selection, _| {
            let (next, selection) = join_lines(value, selection)?;
            Some(CursorEdit::Transform { next, selection })
        })
    }

    /// Uppercase (`upper`) or lowercase every selection of the focused editor.
    pub fn transform_focused_text_case(
        &mut self,
        document: DocumentId,
        upper: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        self.edit_editor_multi(focused.node, focused.kind, |value, selection, _| {
            let (next, selection) = transform_selection_case(value, selection, upper)?;
            Some(CursorEdit::Transform { next, selection })
        })
    }

    /// Sort the lines each selection touches by byte order, optionally
    /// descending and dropping repeated rows. Sorting is a host command-panel
    /// action; no default key binding routes here.
    pub fn sort_focused_text_lines(
        &mut self,
        document: DocumentId,
        descending: bool,
        unique: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        self.edit_editor_multi(focused.node, focused.kind, |value, selection, _| {
            let (next, selection) = sort_lines(value, selection, descending, unique)?;
            Some(CursorEdit::Transform { next, selection })
        })
    }

    /// Move the caret of the focused editor onto the bracket matching the
    /// one adjacent to the caret (`()[]{}`, nesting-aware).
    /// Selection-only move: no change event is emitted. `Ok(false)` means
    /// there is no focused editor or no adjacent bracket pair.
    pub fn goto_focused_text_matching_bracket(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_input {
            return Ok(false);
        }
        let state = self.editor_state(focused.node, focused.kind)?;
        let Some((open, close)) = matching_bracket_pair(&state.value, state.selection.focus) else {
            return Ok(false);
        };
        let target = if state.selection.focus <= open {
            close
        } else {
            open
        };
        self.caret_goal_x = None;
        self.write_editor_selection(
            focused.node,
            focused.kind,
            moved_selection(state.selection, target, false),
        )
    }

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
        let state = self.editor_state(focused.node, focused.kind)?;
        if !state.has_additional_selections() {
            return Ok(false);
        }
        self.write_editor_selections(focused.node, focused.kind, state.selection, Vec::new())
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
        mut shaper: Option<&mut dyn crate::TextShaper>,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        let state = self.editor_state(focused.node, focused.kind)?;
        let intent = if above {
            TextCaretIntent::Up
        } else {
            TextCaretIntent::Down
        };
        let value = state.value.clone();
        let selections = state.selections().into_owned();
        let mut candidates: Vec<TextSelection> = Vec::with_capacity(selections.len());
        match (
            shaper.as_deref_mut(),
            self.world.text_input_shape_context(focused.node),
        ) {
            (Some(shaper), Some((style, constraints))) => {
                let mut geometry = EditorGeometry {
                    shaper,
                    node: focused.node,
                    text: TextContent {
                        value: value.clone(),
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
        self.write_editor_selections(
            focused.node,
            focused.kind,
            next.selection,
            next.additional_selections,
        )
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
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        let state = self.editor_state(focused.node, focused.kind)?;
        let Some(found) =
            next_occurrence(&state.value, &state.selections(), state.selection, previous)
        else {
            return Ok(false);
        };
        let mut next = state;
        if !next.add_selections(&[TextSelection {
            anchor: found.start,
            focus: found.end,
        }]) {
            return Ok(false);
        }
        self.write_editor_selections(
            focused.node,
            focused.kind,
            next.selection,
            next.additional_selections,
        )
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
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        let state = self.editor_state(focused.node, focused.kind)?;
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
            .map(|found| TextSelection {
                anchor: found.start,
                focus: found.end,
            })
            .collect();
        if candidates.is_empty() {
            return Ok(false);
        }
        let mut next = state;
        if !next.add_selections(&candidates) {
            return Ok(false);
        }
        self.write_editor_selections(
            focused.node,
            focused.kind,
            next.selection,
            next.additional_selections,
        )
    }

    /// Select the next literal match of `query` in the focused text editor,
    /// searching from the selection's end and wrapping to the document start.
    ///
    /// Selection-only move: like [`AppContext::move_focused_text_caret`], no
    /// change event is emitted. `Ok(false)` means there is no focused editor
    /// or no match.
    pub fn find_next_focused_text_match(
        &mut self,
        document: DocumentId,
        query: &str,
        options: TextSearchOptions,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_input {
            return Ok(false);
        }
        let state = self.editor_state(focused.node, focused.kind)?;
        let matches = find_matches(&state.value, query, options);
        let Some(found) = find_next_match(&matches, state.selection.ordered().end) else {
            return Ok(false);
        };
        self.caret_goal_x = None;
        self.write_editor_selection(
            focused.node,
            focused.kind,
            TextSelection {
                anchor: found.start,
                focus: found.end,
            },
        )
    }

    /// Select the previous literal match of `query` in the focused text
    /// editor, searching from the selection's start and wrapping to the
    /// document end. Selection-only move; see
    /// [`AppContext::find_next_focused_text_match`].
    pub fn find_previous_focused_text_match(
        &mut self,
        document: DocumentId,
        query: &str,
        options: TextSearchOptions,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_input {
            return Ok(false);
        }
        let state = self.editor_state(focused.node, focused.kind)?;
        let matches = find_matches(&state.value, query, options);
        let Some(found) = find_previous_match(&matches, state.selection.ordered().start) else {
            return Ok(false);
        };
        self.caret_goal_x = None;
        self.write_editor_selection(
            focused.node,
            focused.kind,
            TextSelection {
                anchor: found.start,
                focus: found.end,
            },
        )
    }

    /// Replace the focused editor's selection with `replacement` when the
    /// selection is exactly a match of `query` under `options`, then select
    /// the inserted text (so a following replace targets the same span
    /// semantics) and emit the change event. Advancing to the next match is
    /// the host's call — compose with
    /// [`AppContext::find_next_focused_text_match`]. `Ok(false)` means there
    /// is no focused editor or the selection is not a match.
    ///
    /// Primary-cursor semantics: only the primary selection replaces; other
    /// cursors survive through offset remapping.
    pub fn replace_focused_text_match(
        &mut self,
        document: DocumentId,
        query: &str,
        options: TextSearchOptions,
        replacement: &str,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.accepts_input {
            return Ok(false);
        }
        self.caret_goal_x = None;
        self.edit_editor_multi(
            focused.node,
            focused.kind,
            |value, selection, is_primary| {
                if !is_primary {
                    return None;
                }
                let range = selection.ordered();
                let is_match = find_matches(value, query, options).contains(&range);
                if !is_match {
                    return None;
                }
                let mut next = String::with_capacity(
                    value.len() - range.len().min(value.len()) + replacement.len(),
                );
                next.push_str(&value[..range.start]);
                next.push_str(replacement);
                next.push_str(&value[range.end..]);
                Some(CursorEdit::Transform {
                    next,
                    selection: TextSelection {
                        anchor: range.start,
                        focus: range.start + replacement.len(),
                    },
                })
            },
        )
    }

    /// Replace every literal match of `query` in the focused editor with
    /// `replacement` (left-to-right, non-overlapping on the original text),
    /// select the first replacement, emit one change event, and return the
    /// replacement count. `Ok(0)` leaves the editor untouched.
    pub fn replace_all_focused_text_matches(
        &mut self,
        document: DocumentId,
        query: &str,
        options: TextSearchOptions,
        replacement: &str,
    ) -> Result<usize, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(0);
        };
        if !focused.accepts_input {
            return Ok(0);
        }
        self.caret_goal_x = None;
        let mut replaced = 0usize;
        self.edit_editor(focused.node, focused.kind, |state| {
            let matches = find_matches(&state.value, query, options);
            if matches.is_empty() {
                return None;
            }
            let first_start = matches[0].start;
            let (value, count) = replace_all_matches(&state.value, query, replacement, options);
            replaced = count;
            Some(EditorEdit {
                value,
                selection: TextSelection {
                    anchor: first_start,
                    focus: first_start + replacement.len(),
                },
            })
        })?;
        Ok(replaced)
    }

    /// Place or extend the selection of the focused editor from a pointer
    /// press. Single presses set the caret, Shift extends, double clicks
    /// select the word, triple clicks select the line. An Alt+click
    /// (`add_cursor`) on a multiline editor adds or removes a cursor instead:
    /// clicking an existing additional cursor removes it, any other spot gets
    /// one. Plain presses collapse back to the single primary selection.
    #[allow(clippy::too_many_arguments)]
    pub fn text_editor_pointer_press(
        &mut self,
        document: DocumentId,
        node: StableNodeId,
        pointer_id: u64,
        x: f32,
        y: f32,
        extend: bool,
        add_cursor: bool,
        now: std::time::Duration,
        shaper: &mut dyn crate::TextShaper,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if focused.node != node || !focused.accepts_input {
            self.text_pointer_drag = None;
            return Ok(false);
        }
        let Some((content, scroll)) = self.world.text_input_pointer_context(node) else {
            return Ok(false);
        };
        let Some((style, constraints)) = self.world.text_input_shape_context(node) else {
            return Ok(false);
        };
        let state = self.editor_state(node, focused.kind)?;
        const DOUBLE_CLICK_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);
        const DOUBLE_CLICK_SLOP: f32 = 4.0;
        let count = match &self.text_pointer_click {
            Some(click)
                if click.pointer_id == pointer_id
                    && click.node == node
                    && now.saturating_sub(click.at) <= DOUBLE_CLICK_WINDOW
                    && (click.x - x).abs() < DOUBLE_CLICK_SLOP
                    && (click.y - y).abs() < DOUBLE_CLICK_SLOP =>
            {
                click.count % 3 + 1
            }
            _ => 1,
        };
        self.text_pointer_click = Some(super::TextPointerClick {
            pointer_id,
            node,
            at: now,
            x,
            y,
            count,
        });
        let mut geometry = EditorGeometry {
            shaper,
            node,
            text: TextContent {
                value: state.value.clone(),
            },
            style,
            constraints,
        };
        let (local_x, local_y) = EditorGeometry::localize(content, scroll, x, y);
        let offset = caret_offset_at_point(&state.value, local_x, local_y, geometry.probe());
        // Alt+click toggles an extra cursor on multiline editors; single-line
        // fields keep their plain click semantics.
        if add_cursor && focused.multiline && count == 1 {
            return self.text_editor_toggle_cursor(node, focused.kind, &state, offset);
        }
        let (selection, additional) = match count {
            2 => {
                let (start, end) = word_range_at(&state.value, offset);
                (
                    TextSelection {
                        anchor: start,
                        focus: end,
                    },
                    Vec::new(),
                )
            }
            3 => {
                let (start, end) = logical_line_range(&state.value, offset);
                (
                    TextSelection {
                        anchor: start,
                        focus: end,
                    },
                    Vec::new(),
                )
            }
            _ if extend => (
                moved_selection(state.selection, offset, true),
                state.additional_selections.clone(),
            ),
            // A plain click naturally collapses to one primary cursor.
            _ => (moved_selection(state.selection, offset, false), Vec::new()),
        };
        if count != 1 {
            self.text_pointer_drag = None;
        } else {
            let anchor = if extend { selection.anchor } else { offset };
            self.text_pointer_drag = Some((pointer_id, node, anchor));
        }
        self.write_editor_selections(node, focused.kind, selection, additional)
    }

    /// Toggle an Alt+click cursor at `offset`: clicking an existing additional
    /// cursor removes it, clicking anywhere else adds one (the primary
    /// selection is never removed this way).
    fn text_editor_toggle_cursor(
        &mut self,
        node: StableNodeId,
        kind: TextEditorKind,
        state: &TextInputState,
        offset: usize,
    ) -> Result<bool, FrameworkError> {
        let mut next = state.clone();
        let before = next.additional_selections.len();
        next.additional_selections.retain(|selection| {
            let range = selection.ordered();
            !(range.start <= offset && offset <= range.end)
        });
        if next.additional_selections.len() == before {
            let primary = state.selection.ordered();
            let on_primary = primary.start <= offset && offset <= primary.end;
            if !on_primary {
                next.additional_selections
                    .push(TextSelection::caret(offset));
            }
        }
        next.normalize_selections();
        self.write_editor_selections(node, kind, next.selection, next.additional_selections)
    }

    /// Extend a live drag selection to the pointer position.
    pub fn text_editor_pointer_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        shaper: &mut dyn crate::TextShaper,
    ) -> Result<bool, FrameworkError> {
        let Some((drag_id, node, anchor)) = self.text_pointer_drag else {
            return Ok(false);
        };
        if drag_id != pointer_id {
            return Ok(false);
        }
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if focused.node != node {
            return Ok(false);
        }
        let Some((content, scroll)) = self.world.text_input_pointer_context(node) else {
            return Ok(false);
        };
        let Some((style, constraints)) = self.world.text_input_shape_context(node) else {
            return Ok(false);
        };
        let state = self.editor_state(node, focused.kind)?;
        let mut geometry = EditorGeometry {
            shaper,
            node,
            text: TextContent {
                value: state.value.clone(),
            },
            style,
            constraints,
        };
        let (local_x, local_y) = EditorGeometry::localize(content, scroll, x, y);
        let offset = caret_offset_at_point(&state.value, local_x, local_y, geometry.probe());
        if offset == state.selection.focus {
            return Ok(false);
        }
        self.write_editor_selection(
            node,
            focused.kind,
            TextSelection {
                anchor,
                focus: offset,
            },
        )
    }

    /// End any drag selection started by this pointer.
    pub fn text_editor_pointer_release(&mut self, pointer_id: u64) {
        if self
            .text_pointer_drag
            .is_some_and(|(drag_id, _, _)| drag_id == pointer_id)
        {
            self.text_pointer_drag = None;
        }
    }

    fn editor_state(
        &self,
        node: StableNodeId,
        kind: TextEditorKind,
    ) -> Result<TextInputState, FrameworkError> {
        match kind {
            TextEditorKind::Area => self
                .view_entity::<TextArea>(node)
                .and_then(|entity| self.read(entity, |area: &TextArea| area.state.clone()).ok())
                .ok_or(FrameworkError::MissingView(node)),
            TextEditorKind::Field => self
                .view_entity::<TextInput>(node)
                .and_then(|entity| {
                    self.read(entity, |field: &TextInput| field.state.clone())
                        .ok()
                })
                .ok_or(FrameworkError::MissingView(node)),
        }
    }

    fn write_editor_selection(
        &mut self,
        node: StableNodeId,
        kind: TextEditorKind,
        selection: TextSelection,
    ) -> Result<bool, FrameworkError> {
        match kind {
            TextEditorKind::Area => {
                let entity = Entity::<TextArea>::from_stable_id(node);
                self.update_component(entity, |area: &mut TextArea, _| {
                    if area.state.selection == selection {
                        return false;
                    }
                    area.state.selection = selection;
                    true
                })
            }
            TextEditorKind::Field => {
                let entity = Entity::<TextInput>::from_stable_id(node);
                self.update_component(entity, |field: &mut TextInput, _| {
                    if field.state.selection == selection {
                        return false;
                    }
                    field.state.selection = selection;
                    true
                })
            }
        }
    }

    /// Write the complete selection set (primary plus additional cursors),
    /// restoring the invariants in place — cursors that moved onto the same
    /// offset fuse here. Selection-only move: no change event is emitted.
    fn write_editor_selections(
        &mut self,
        node: StableNodeId,
        kind: TextEditorKind,
        selection: TextSelection,
        additional: Vec<TextSelection>,
    ) -> Result<bool, FrameworkError> {
        match kind {
            TextEditorKind::Area => {
                let entity = Entity::<TextArea>::from_stable_id(node);
                self.update_component(entity, |area: &mut TextArea, _| {
                    if area.state.selection == selection
                        && area.state.additional_selections == additional
                    {
                        return false;
                    }
                    area.state.selection = selection;
                    area.state.additional_selections = additional;
                    area.state.normalize_selections();
                    true
                })
            }
            TextEditorKind::Field => {
                let entity = Entity::<TextInput>::from_stable_id(node);
                self.update_component(entity, |field: &mut TextInput, _| {
                    if field.state.selection == selection
                        && field.state.additional_selections == additional
                    {
                        return false;
                    }
                    field.state.selection = selection;
                    field.state.additional_selections = additional;
                    field.state.normalize_selections();
                    true
                })
            }
        }
    }

    /// Apply a committed value edit and emit the editor's change event.
    /// `Ok(false)` means the edit declined or changed nothing.
    fn edit_editor(
        &mut self,
        node: StableNodeId,
        kind: TextEditorKind,
        edit: impl FnOnce(&TextInputState) -> Option<EditorEdit>,
    ) -> Result<bool, FrameworkError> {
        let state = self.editor_state(node, kind)?;
        let Some(edited) = edit(&state) else {
            return Ok(false);
        };
        if edited.value == state.value {
            return Ok(false);
        }
        let EditorEdit { value, selection } = edited;
        match kind {
            TextEditorKind::Area => {
                let entity = Entity::<TextArea>::from_stable_id(node);
                self.update_component(entity, |area: &mut TextArea, cx| {
                    area.state.value = value;
                    area.state.selection = selection;
                    cx.emit(area.change());
                    true
                })
            }
            TextEditorKind::Field => {
                let entity = Entity::<TextInput>::from_stable_id(node);
                self.update_component(entity, |field: &mut TextInput, cx| {
                    field.state.value = value;
                    field.state.selection = selection;
                    cx.emit(field.change());
                    true
                })
            }
        }
    }

    /// Apply a per-cursor edit closure to every active selection and commit
    /// the combined result as one edit.
    ///
    /// Single-cursor fast path: the closure runs once and the write is the
    /// historical single-selection commit. Multi-cursor path: the closure
    /// runs per selection against the pre-edit snapshot,
    /// [`apply_cursor_edits`] splices everything into one output string, and
    /// the selection set is rebuilt and normalized — one intermediate String,
    /// never one per cursor.
    fn edit_editor_multi(
        &mut self,
        node: StableNodeId,
        kind: TextEditorKind,
        mut edit: impl FnMut(&str, TextSelection, bool) -> Option<CursorEdit>,
    ) -> Result<bool, FrameworkError> {
        let state = self.editor_state(node, kind)?;
        if !state.has_additional_selections() {
            let Some(edited) = edit(&state.value, state.selection, true) else {
                return Ok(false);
            };
            let (value, selection) = match edited {
                CursorEdit::Span(replacement) => {
                    let (value, caret) = apply_replacement(&state.value, &replacement);
                    (value, TextSelection::caret(caret))
                }
                CursorEdit::Transform { next, selection } => (next, selection),
            };
            if value == state.value {
                return Ok(false);
            }
            return self.commit_editor_value(node, kind, value, selection, Vec::new());
        }
        let selections = state.selections().into_owned();
        let primary_index = selections
            .iter()
            .position(|selection| *selection == state.selection)
            .unwrap_or(0);
        let mut any_edit = false;
        let edits: Vec<(TextSelection, Option<CursorEdit>)> = selections
            .iter()
            .enumerate()
            .map(|(index, &selection)| {
                let edited = edit(&state.value, selection, index == primary_index);
                any_edit |= edited.is_some();
                (selection, edited)
            })
            .collect();
        if !any_edit {
            return Ok(false);
        }
        let Some((value, edited_selections)) = apply_cursor_edits(&state.value, &edits) else {
            return Ok(false);
        };
        if value == state.value {
            return Ok(false);
        }
        let primary = edited_selections
            .get(primary_index)
            .copied()
            .unwrap_or_else(|| TextSelection::caret(value.len()));
        let additional = edited_selections
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != primary_index)
            .map(|(_, selection)| *selection)
            .collect();
        self.commit_editor_value(node, kind, value, primary, additional)
    }

    /// Commit a value edit together with the rebuilt selection set. The set
    /// is normalized here so overlapping or touching cursors fuse in the
    /// same pass that emits the change event.
    fn commit_editor_value(
        &mut self,
        node: StableNodeId,
        kind: TextEditorKind,
        value: String,
        selection: TextSelection,
        additional: Vec<TextSelection>,
    ) -> Result<bool, FrameworkError> {
        let mut next = TextInputState {
            value,
            selection,
            additional_selections: additional,
        };
        next.normalize_selections();
        let (value, selection, additional) =
            (next.value, next.selection, next.additional_selections);
        match kind {
            TextEditorKind::Area => {
                let entity = Entity::<TextArea>::from_stable_id(node);
                self.update_component(entity, |area: &mut TextArea, cx| {
                    if area.state.value == value {
                        return false;
                    }
                    area.state.value = value;
                    area.state.selection = selection;
                    area.state.additional_selections = additional;
                    cx.emit(area.change());
                    true
                })
            }
            TextEditorKind::Field => {
                let entity = Entity::<TextInput>::from_stable_id(node);
                self.update_component(entity, |field: &mut TextInput, cx| {
                    if field.state.value == value {
                        return false;
                    }
                    field.state.value = value;
                    field.state.selection = selection;
                    field.state.additional_selections = additional;
                    cx.emit(field.change());
                    true
                })
            }
        }
    }
}
