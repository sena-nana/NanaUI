//! Text-editor caret, selection, and code-editing commands on [`AppContext`].
//!
//! Keyboard and pointer adapters resolve intents and hand them here; this
//! module owns which focused component is editable, how offsets map onto the
//! committed value, and when change events fire. Geometry-dependent queries
//! receive the host shaper so layout stays backend-owned.

use super::{AppContext, DocumentId, EditableText, Entity, FrameworkError, StableNodeId};
use super::{TextArea, TextInput, TextInputState, TextSelection};
use crate::components::TextSnippetSession;
use crate::text_editing::{
    CursorEdit, TextCaretIntent, TextLineDirection, TextReplacement, TextSearchOptions,
    apply_cursor_edits, apply_replacement, auto_indent_newline, auto_pair_edit, caret_focus,
    caret_offset_at_point, clamp_boundary, delete_backward, delete_forward, delete_lines,
    delete_to_line_end, delete_to_line_start, delete_word_backward, delete_word_forward,
    duplicate_lines, find_matches, find_next_match, find_previous_match, indent_selection,
    join_lines, logical_line_range, matching_bracket_pair, move_lines, moved_selection,
    outdent_selection, page_caret_focus, page_caret_focus_logical, replace_all_matches, sort_lines,
    toggle_line_comment, transform_selection_case, vertical_caret_focus,
    vertical_caret_focus_logical, word_range_at,
};
use crate::{
    CodeEditing, MutationQueue, ScrollOffset, TextCodeFold, TextContent, TextShapeConstraints,
    TextSnippet,
};
use std::sync::Arc;

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
pub(crate) enum TextEditorKind {
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
    ///
    /// 折叠语义：存在折叠态区间时，移动意图在显示视图上解析（探针与
    /// 换行结构与渲染一致），结果映射回值空间——落在折叠隐藏区间上的
    /// 偏移自然钳到折叠起始行行尾。LineStart/LineEnd/Up/Down/PageUp/
    /// PageDown/DocStart/DocEnd 因此跳过折叠区间。
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
        // 折叠视图：无折叠态区间时为 None，全部按原始值解析（零成本）。
        let fold_view = self.world.text_display_view(focused.node);
        let probe_value: &str = fold_view.as_ref().map_or(&state.value, |view| &view.value);
        let to_display = |selection: TextSelection| -> TextSelection {
            match &fold_view {
                Some(view) => TextSelection {
                    anchor: view.display_of(selection.anchor),
                    focus: view.display_of(selection.focus),
                },
                None => selection,
            }
        };
        let to_value = |selection: TextSelection| -> TextSelection {
            match &fold_view {
                Some(view) => TextSelection {
                    anchor: view.value_of(selection.anchor),
                    focus: view.value_of(selection.focus),
                },
                None => selection,
            }
        };
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
                            value: probe_value.to_owned(),
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
                    probe_value,
                    to_display(selection),
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
                        next_selections.push(to_value(moved));
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
        let selection = to_display(state.selection);
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
                        value: probe_value.to_owned(),
                    },
                    style,
                    constraints,
                };
                let moved = if matches!(intent, TextCaretIntent::Up | TextCaretIntent::Down) {
                    vertical_caret_focus(
                        probe_value,
                        selection,
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
                        probe_value,
                        selection,
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
                    vertical_caret_focus_logical(probe_value, selection, intent, extend)
                } else {
                    page_caret_focus_logical(probe_value, selection, intent, extend)
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
            match caret_focus(probe_value, selection, mapped) {
                Some(focus) => moved_selection(selection, focus, extend),
                None => return Ok(false),
            }
        } else {
            self.caret_goal_x = None;
            match caret_focus(probe_value, selection, intent) {
                Some(focus) => moved_selection(selection, focus, extend),
                None => return Ok(false),
            }
        };
        self.write_editor_selection(focused.node, focused.kind, to_value(selection))
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
        let state = self.editor_state(focused.node, focused.kind)?;
        let start = clamp_focus(&state.value, start);
        let end = clamp_focus(&state.value, end);
        self.caret_goal_x = None;
        // 区间两端都参与 reveal（光标导航只看 focus；宿主跳转的起点同样
        // 必须可见）。write_editor_selections 内部还会按 focus 再对账一次，
        // 幂等无害。
        self.unfold_text_folds_containing(focused.node, &[start, end])?;
        self.write_editor_selections(
            focused.node,
            focused.kind,
            TextSelection {
                anchor: start,
                focus: end,
            },
            Vec::new(),
        )
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
        // 折叠视图：命中测试与双击/三击语义都按显示视图解析。
        let fold_view = self.world.text_display_view(node);
        let fold_view_value = fold_view.as_ref().map(|view| view.value.clone());
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
        // 补全弹层交互优先于折叠：弹层绘制在全部编辑器覆盖层之上，
        // 命中候选行即接受该行（单击且无修饰时），不落光标。
        if count == 1
            && !extend
            && !add_cursor
            && let Some(row) = self.world.text_completion_hit(node, x, y)
        {
            self.text_pointer_drag = None;
            return self.accept_focused_text_completion(document, Some(row));
        }
        // minimap 竖条交互：条内按下即消费（任何修饰键与连击计数）——
        // 视口滚动到点击行居中的位置并进入拖拽跟随，不移动光标、不产生
        // 选区；滚轮不经过本路径，落在条上仍按编辑器常规滚动。
        if let Some(target) = self.world.text_minimap_scroll_target(node, x, y) {
            self.text_pointer_drag = None;
            return self.text_editor_minimap_navigate(node, focused.kind, pointer_id, target);
        }
        // 折叠交互优先：gutter 箭头与折叠摘要标记的点击切换折叠态，不落
        // 光标（单击且无修饰时）。
        if count == 1
            && !extend
            && !add_cursor
            && let Some(fold) = self.world.text_fold_hit(node, x, y)
        {
            self.text_pointer_drag = None;
            let collapsed = self
                .world
                .text_fold_view_state(node)
                .map(|state| state.collapsed.contains(&fold))
                .unwrap_or(false);
            return self.set_node_text_fold(node, fold, !collapsed);
        }
        let mut geometry = EditorGeometry {
            shaper,
            node,
            // 折叠态下命中测试按显示视图解析，再映射回值空间。
            text: TextContent {
                value: fold_view_value.as_ref().unwrap_or(&state.value).clone(),
            },
            style,
            constraints,
        };
        let (local_x, local_y) = EditorGeometry::localize(content, scroll, x, y);
        let hit = caret_offset_at_point(
            fold_view_value.as_ref().unwrap_or(&state.value),
            local_x,
            local_y,
            geometry.probe(),
        );
        let offset = match &fold_view {
            Some(view) => view.value_of(hit),
            None => hit,
        };
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

    /// minimap 导航：把目标滚动写进世界与组件（组件字段是投影权威，
    /// 不同步会在下一帧被投影改写回去），并钉住视口使光标 reveal 让位。
    fn text_editor_minimap_navigate(
        &mut self,
        node: StableNodeId,
        kind: TextEditorKind,
        pointer_id: u64,
        target: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_offset(node, target);
        self.world.commit(mutations)?;
        // 钉住提交后的实际偏移（世界侧可能再钳制），保证与几何层读取的
        // 请求滚动逐位相等。
        let applied = self.world.scroll_offset(node).unwrap_or(target);
        self.world.set_text_viewport_pin(node, Some(applied));
        if kind == TextEditorKind::Area {
            self.update_component(Entity::<TextArea>::from_stable_id(node), |area, _| {
                area.scroll_offset = applied;
                false
            })?;
        }
        self.text_minimap_drag = Some((pointer_id, node, kind));
        Ok(true)
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
        // minimap 拖拽优先：按下进入的导航拖拽连续跟随指针换算视口，
        // 不做选区延伸。指针拖出条外时保持最后视口（仍消费本次拖拽）。
        if let Some((drag_id, drag_node, drag_kind)) = self.text_minimap_drag {
            if drag_id != pointer_id {
                return Ok(false);
            }
            return match self.world.text_minimap_scroll_target(drag_node, x, y) {
                Some(target) => {
                    self.text_editor_minimap_navigate(drag_node, drag_kind, pointer_id, target)
                }
                None => Ok(true),
            };
        }
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
        let fold_view = self.world.text_display_view(node);
        let probe_value: &str = fold_view.as_ref().map_or(&state.value, |view| &view.value);
        let mut geometry = EditorGeometry {
            shaper,
            node,
            text: TextContent {
                value: probe_value.to_owned(),
            },
            style,
            constraints,
        };
        let (local_x, local_y) = EditorGeometry::localize(content, scroll, x, y);
        let hit = caret_offset_at_point(probe_value, local_x, local_y, geometry.probe());
        let offset = match &fold_view {
            Some(view) => view.value_of(hit),
            None => hit,
        };
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
        if self
            .text_minimap_drag
            .is_some_and(|(drag_id, _, _)| drag_id == pointer_id)
        {
            self.text_minimap_drag = None;
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
        // 光标落在折叠隐藏区间内 → 该折叠自动展开（reveal 语义；查找导航
        // 跳转也经由此路径展开）。
        self.unfold_text_folds_containing(node, &[selection.focus])?;
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
        // 任一光标落在折叠隐藏区间内 → 该折叠自动展开（附加光标进入折叠
        // 的强制展开语义；跨越折叠的范围选择不展开）。
        let focuses: Vec<usize> = std::iter::once(selection.focus)
            .chain(additional.iter().map(|selection| selection.focus))
            .collect();
        self.unfold_text_folds_containing(node, &focuses)?;
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

/// 展开 snippet body：`$N` 占位从文本移除，`$1..$N` 按序号记录跳位偏移，
/// `$0` 记录初始光标位置（缺省为插入文本末尾）。`$` 后不跟数字时保持
/// 字面量。
fn expand_snippet_body(body: &str, base: usize) -> (String, Vec<usize>, usize) {
    let mut text = String::with_capacity(body.len());
    let mut stops: Vec<(u32, usize)> = Vec::new();
    let mut final_caret: Option<usize> = None;
    let mut characters = body.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '$' && characters.peek().is_some_and(|next| next.is_ascii_digit()) {
            let mut number = characters.next().unwrap().to_digit(10).unwrap_or(0);
            while let Some(next) = characters.peek()
                && next.is_ascii_digit()
            {
                number = number * 10 + next.to_digit(10).unwrap_or(0);
                characters.next();
            }
            let offset = base + text.len();
            if number == 0 {
                final_caret = Some(offset);
            } else {
                stops.push((number, offset));
            }
            continue;
        }
        text.push(character);
    }
    stops.sort_by_key(|(number, _)| *number);
    let caret = final_caret.unwrap_or(base + text.len());
    (
        text,
        stops.into_iter().map(|(_, offset)| offset).collect(),
        caret,
    )
}

/// 补全接受时被替换的词前缀字符（`[A-Za-z0-9_]`，Zed 风格标识符词）。
fn is_word_prefix_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// 从 `caret` 向前的词前缀起点（连续 `[A-Za-z0-9_]` 游程的开头）。
fn word_prefix_start(value: &str, caret: usize) -> usize {
    let bytes = value.as_bytes();
    let caret = caret.min(bytes.len());
    let mut start = caret;
    while start > 0 && is_word_prefix_byte(bytes[start - 1]) {
        start -= 1;
    }
    start
}

impl AppContext {
    /// 聚焦多行编辑器当前活跃（非空且未关闭）的补全会话与节点。
    fn focused_completion_session(
        &self,
        document: DocumentId,
    ) -> Option<(StableNodeId, crate::store::TextCompletionViewState)> {
        let focused = self.focused_text_editor(document)?;
        if !focused.multiline || !focused.accepts_input {
            return None;
        }
        let state = self.world.text_completion_view(focused.node)?;
        (state.items.len() > 0 && !state.dismissed).then_some((focused.node, state))
    }

    /// 聚焦多行编辑器当前是否激活补全弹层（非空且未关闭的会话）。
    /// 路由层用它决定编辑键是否被弹层消费（边界上导航无可做仍消费）。
    pub fn focused_text_completion_active(&self, document: DocumentId) -> bool {
        self.focused_completion_session(document).is_some()
    }

    /// 补全弹层的上下键导航：在候选间移动选中项（编辑器选区不动），
    /// 滚动窗口跟随选中项。弹层未激活或已在边界时返回 `Ok(false)`——
    /// 边界仍消费事件由路由层决定（弹层激活期间一律消费）。
    pub fn move_focused_text_completion(
        &mut self,
        document: DocumentId,
        down: bool,
    ) -> Result<bool, FrameworkError> {
        let Some((node, state)) = self.focused_completion_session(document) else {
            return Ok(false);
        };
        let len = state.items.len();
        let selected = if down {
            (state.selected + 1).min(len - 1)
        } else {
            state.selected.saturating_sub(1)
        };
        let scroll = completion_scroll_follow(selected, state.scroll, len);
        if selected == state.selected && scroll == state.scroll {
            return Ok(false);
        }
        let mut mutations = crate::MutationQueue::new();
        mutations.set_text_input_completion_view(node, selected, scroll);
        self.world.commit(mutations)?;
        Ok(true)
    }

    /// 接受补全候选：把主光标前的词前缀（`[A-Za-z0-9_]` 游程）替换为
    /// 候选 `label`，发一次 TextChanged。`row` 为 `Some` 时接受指定候选
    /// （弹层点击路径），`None` 接受当前选中项。多光标时只作用主光标，
    /// 附加光标经偏移重映射保留。会话本身不动：弹层的后续过滤由宿主
    /// 在 TextChanged 后重喂决定。
    pub fn accept_focused_text_completion(
        &mut self,
        document: DocumentId,
        row: Option<usize>,
    ) -> Result<bool, FrameworkError> {
        let Some((_, state)) = self.focused_completion_session(document) else {
            return Ok(false);
        };
        let index = row.unwrap_or(state.selected);
        let Some(item) = state.items.get(index) else {
            return Ok(false);
        };
        let label = item.label.clone();
        let focused = self
            .focused_text_editor(document)
            .expect("session implies editor");
        self.caret_goal_x = None;
        self.edit_editor_multi(
            focused.node,
            focused.kind,
            |value, selection, is_primary| {
                if !is_primary {
                    return None;
                }
                let caret = clamp_boundary(value, selection.focus);
                let start = word_prefix_start(value, caret);
                Some(CursorEdit::Span(TextReplacement {
                    range: start..caret,
                    insert: label.clone(),
                    caret: start + label.len(),
                }))
            },
        )
    }

    /// Esc 关闭补全弹层。会话数据保留：宿主重喂相同列表不复活弹层，
    /// 换新列表（打字后过滤结果变化）重新打开。无活跃弹层时返回
    /// `Ok(false)`，Esc 落到后续优先级（多光标塌缩等）。
    pub fn dismiss_focused_text_completion(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        let Some(state) = self.world.text_completion_view(focused.node) else {
            return Ok(false);
        };
        if state.dismissed {
            return Ok(false);
        }
        let mut mutations = crate::MutationQueue::new();
        mutations.set_text_input_completion_dismissed(focused.node);
        self.world.commit(mutations)?;
        Ok(true)
    }

    /// 重开补全弹层（宿主显式重触发，如 Esc 关闭后再次 Ctrl+Space）：
    /// 清除关闭标记并把选中归零，候选列表保持不变。弹层未激活或未处
    /// 于关闭态时返回 `Ok(false)`。
    pub fn reopen_focused_text_completion(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        let Some(state) = self.world.text_completion_view(focused.node) else {
            return Ok(false);
        };
        if !state.dismissed {
            return Ok(false);
        }
        let mut mutations = crate::MutationQueue::new();
        mutations.set_text_input_completion_reopened(focused.node);
        self.world.commit(mutations)?;
        Ok(true)
    }

    /// 滚轮滚动补全弹层（`rows` 为正表示向列表末尾方向）。滚动位置钳在
    /// 候选范围内；弹层未激活时返回 `Ok(false)`。
    pub fn scroll_focused_text_completion(
        &mut self,
        document: DocumentId,
        rows: isize,
    ) -> Result<bool, FrameworkError> {
        let Some((node, state)) = self.focused_completion_session(document) else {
            return Ok(false);
        };
        let len = state.items.len();
        let scroll = clamp_scroll(state.scroll as isize + rows, len);
        if scroll == state.scroll {
            return Ok(true);
        }
        let mut mutations = crate::MutationQueue::new();
        mutations.set_text_input_completion_view(node, state.selected, scroll);
        self.world.commit(mutations)?;
        Ok(true)
    }

    /// 指针位置命中的补全候选（弹层点击接受路径的查询入口）。未聚焦
    /// 编辑器或未命中时为 `None`。
    pub fn completion_hit_at(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> Result<Option<usize>, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(None);
        };
        if !focused.multiline {
            return Ok(None);
        }
        Ok(self.world.text_completion_hit(focused.node, x, y))
    }

    /// 滚轮落在锚定浮层内时的路由：优先滚动补全弹层（绘制在最上，会话
    /// 锚定聚焦编辑器），其次滚动 hover 浮窗正文。hover 命中测试驱动：
    /// hover 显示不要求焦点，滚轮落点命中任意编辑器的浮窗面板即滚动该
    /// 面板。都不在浮层上时返回 `Ok(false)`，滚轮落回编辑器/文档滚动。
    pub fn scroll_text_overlay_at(
        &mut self,
        document: DocumentId,
        x: f32,
        y: f32,
        rows: isize,
    ) -> Result<bool, FrameworkError> {
        if let Some(focused) = self.focused_text_editor(document)
            && self.world.text_completion_panel_hit(focused.node, x, y)
        {
            return self.scroll_focused_text_completion(document, rows);
        }
        let Some(node) = self.world.text_hover_panel_at(document, x, y) else {
            return Ok(false);
        };
        let Some(state) = self.world.text_hover_view(node) else {
            return Ok(false);
        };
        let line_count = state.doc.body.lines().count();
        let scroll = clamp_scroll(state.scroll as isize + rows, line_count);
        if scroll == state.scroll {
            return Ok(true);
        }
        let mut mutations = crate::MutationQueue::new();
        mutations.set_text_input_hover_scroll(node, scroll);
        self.world.commit(mutations)?;
        Ok(true)
    }
}

/// 键盘导航的滚动窗口跟随：选中项离开可见窗口时把窗口滑到包含它的
/// 最近位置。
fn completion_scroll_follow(selected: usize, scroll: usize, len: usize) -> usize {
    let visible = crate::components::TEXT_COMPLETION_VISIBLE_ROWS.min(len);
    if selected < scroll {
        selected
    } else if selected >= scroll + visible {
        selected + 1 - visible
    } else {
        scroll
    }
}

/// 滚动位置钳制：非空列表钳在 `[0, len - 1]`，空列表恒为 0。
fn clamp_scroll(scroll: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    scroll.clamp(0, (len - 1) as isize) as usize
}

impl AppContext {
    /// 光标落点所在的折叠自动展开。`offsets` 里的偏移严格落在某个折叠
    /// 的隐藏区间内部（折叠起始行行尾不算）时，该折叠展开。
    fn unfold_text_folds_containing(
        &mut self,
        node: StableNodeId,
        offsets: &[usize],
    ) -> Result<(), FrameworkError> {
        let Some(view) = self.world.text_display_view(node) else {
            return Ok(());
        };
        let to_unfold: Vec<TextCodeFold> = view
            .spans
            .iter()
            .filter(|span| offsets.iter().any(|offset| view.span_hides(span, *offset)))
            .map(|span| span.fold)
            .collect();
        if to_unfold.is_empty() {
            return Ok(());
        }
        let mut state = self.world.text_fold_view_state(node).unwrap_or_default();
        state.collapsed.retain(|fold| !to_unfold.contains(fold));
        let mut mutations = crate::MutationQueue::new();
        mutations.set_text_input_fold_collapsed(node, state.collapsed.into());
        self.world.commit(mutations)?;
        Ok(())
    }

    /// 写回单个节点的折叠态（toggle 命令与点击交互共用）。
    fn set_node_text_fold(
        &mut self,
        node: StableNodeId,
        fold: TextCodeFold,
        collapse: bool,
    ) -> Result<bool, FrameworkError> {
        let mut state = self.world.text_fold_view_state(node).unwrap_or_default();
        let changed = if collapse {
            if state.collapsed.contains(&fold) {
                false
            } else {
                state.collapsed.push(fold);
                state.collapsed.sort_by_key(|fold| (fold.start, fold.end));
                true
            }
        } else {
            let before = state.collapsed.len();
            state.collapsed.retain(|entry| *entry != fold);
            state.collapsed.len() != before
        };
        if !changed {
            return Ok(false);
        }
        let mut mutations = crate::MutationQueue::new();
        mutations.set_text_input_fold_collapsed(node, state.collapsed.into());
        self.world.commit(mutations)?;
        Ok(true)
    }

    /// 切换聚焦代码编辑器的一个折叠区间。`offset` 为 `Some` 时取包含该
    /// 偏移的最内层折叠，`None` 时取主光标所在的；光标不在任何折叠上时
    /// 返回 `Ok(false)`。折叠是纯视图状态：不改值、不发 change 事件。
    pub fn toggle_focused_text_fold(
        &mut self,
        document: DocumentId,
        offset: Option<usize>,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        let state = self.editor_state(focused.node, focused.kind)?;
        let view = self.world.text_fold_view_state(focused.node);
        let offered = view.as_ref().map_or(&[][..], |state| &state.offered);
        let at = offset.unwrap_or_else(|| state.selection.focus);
        let at = clamp_boundary(&state.value, at);
        // 包含该偏移的最内层折叠：区间覆盖，或偏移落在折叠起始行上
        // （Zed 语义：光标在 `{` 所在行即可切换该折叠）。
        let fold = offered
            .iter()
            .filter(|fold| {
                let line_start = logical_line_range(&state.value, fold.start).0;
                line_start <= at && at <= fold.end
            })
            .max_by_key(|fold| fold.start)
            .copied();
        let Some(fold) = fold else {
            return Ok(false);
        };
        let collapsed = view.is_some_and(|state| state.collapsed.contains(&fold));
        self.set_node_text_fold(focused.node, fold, !collapsed)
    }

    /// 展开聚焦编辑器的全部折叠。返回是否发生了变化。
    pub fn unfold_all_focused_text_folds(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        let Some(state) = self.world.text_fold_view_state(focused.node) else {
            return Ok(false);
        };
        if state.collapsed.is_empty() {
            return Ok(false);
        }
        let mut mutations = crate::MutationQueue::new();
        mutations.set_text_input_fold_collapsed(focused.node, Arc::from([]));
        self.world.commit(mutations)?;
        Ok(true)
    }

    /// 查询聚焦编辑器当前的折叠态区间（值空间，按 `start` 排序）。宿主
    /// 测试与状态面板的只读入口；无折叠时为空表。
    pub fn focused_text_collapsed_folds(&self, document: DocumentId) -> Vec<TextCodeFold> {
        self.focused_text_editor(document)
            .and_then(|focused| self.world.text_fold_view_state(focused.node))
            .map(|state| state.collapsed)
            .unwrap_or_default()
    }

    /// 在聚焦多行编辑器的当前主选区处插入 snippet。占位 `$N` 从文本中
    /// 移除，`$1..$N` 按序号成为 Tab 跳位，`$0`（缺省为插入文本末尾）是
    /// 插入后的光标位置并开启会话；无占位的 snippet 不开启会话。
    ///
    /// 多光标限制：snippet 只作用于主光标（附加光标随编辑平移保留），
    /// 不做占位镜显。
    pub fn insert_focused_text_snippet(
        &mut self,
        document: DocumentId,
        snippet: &TextSnippet,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        let node = focused.node;
        self.caret_goal_x = None;
        let state = self.editor_state(node, focused.kind)?;
        let (insert, stops, final_caret) =
            expand_snippet_body(&snippet.body, state.selection.ordered().start);
        let mut next = state.clone();
        // 主光标独占：与 IME 提交同款路径，附加光标经偏移重映射保留。
        if !next.replace_primary_selection(&insert) {
            return Ok(false);
        }
        let session = (!stops.is_empty()).then(|| TextSnippetSession { stops, index: 0 });
        // 选区落在 $0（缺省为插入文本末尾）。
        next.selection = TextSelection::caret(final_caret);
        match focused.kind {
            TextEditorKind::Area => {
                let entity = Entity::<TextArea>::from_stable_id(node);
                self.update_component(entity, |area: &mut TextArea, cx| {
                    area.state = next.clone();
                    cx.emit(area.change());
                    true
                })?;
            }
            TextEditorKind::Field => {
                let entity = Entity::<TextInput>::from_stable_id(node);
                self.update_component(entity, |field: &mut TextInput, cx| {
                    field.state = next.clone();
                    cx.emit(field.change());
                    true
                })?;
            }
        }
        let mut mutations = crate::MutationQueue::new();
        mutations.set_text_input_snippet(node, session);
        self.world.commit(mutations)?;
        Ok(true)
    }

    /// snippet 会话内 Tab/Shift+Tab 跳位：前进到下一个 `$N`、后退到上一
    /// 个已访问跳位；越过最后一个跳位（或后退越过第一个）时结束会话并
    /// 放行 Tab（返回 `Ok(false)`，缩进等既有行为接手）。无会话时返回
    /// `Ok(false)`。
    pub fn advance_focused_text_snippet(
        &mut self,
        document: DocumentId,
        reverse: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if !focused.multiline || !focused.accepts_input {
            return Ok(false);
        }
        let node = focused.node;
        let session = self.world.text_snippet_session(node);
        let Some(mut session) = session else {
            return Ok(false);
        };
        let mut ended = false;
        // 不变式：`index` 指向下一个前进跳位（前进落到 `stops[index]` 后
        // 加一）。后退先回退两步取"上一个已访问跳位"，再恢复不变式。
        let caret = if reverse {
            if session.index <= 1 {
                ended = true;
                None
            } else {
                session.index -= 2;
                let caret = session.stops[session.index];
                session.index += 1;
                Some(caret)
            }
        } else if session.index >= session.stops.len() {
            ended = true;
            None
        } else {
            let caret = session.stops[session.index];
            session.index += 1;
            Some(caret)
        };
        if let Some(caret) = caret {
            let state = self.editor_state(node, focused.kind)?;
            if clamp_boundary(&state.value, caret) != caret {
                // 跳位失效（文本边界已不在字符边界上）：结束会话。
                ended = true;
            } else {
                self.write_editor_selection(node, focused.kind, TextSelection::caret(caret))?;
            }
        }
        let session = (!ended).then_some(session);
        let mut mutations = crate::MutationQueue::new();
        mutations.set_text_input_snippet(node, session);
        self.world.commit(mutations)?;
        Ok(true)
    }

    /// 结束聚焦编辑器的 snippet 会话（Esc）。返回是否存在活跃会话。
    pub fn cancel_focused_text_snippet(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if self.world.text_snippet_session(focused.node).is_none() {
            return Ok(false);
        }
        let mut mutations = crate::MutationQueue::new();
        mutations.set_text_input_snippet(focused.node, None);
        self.world.commit(mutations)?;
        Ok(true)
    }
}

#[cfg(test)]
mod fold_snippet_tests {
    use super::*;
    use crate::{AppContext, DocumentId, TextSnippet};

    /// "fn a() {\n    x();\n    y();\n}\nfn b() {}"，块折叠区间
    /// `{`（7）到 `}` 之后（28），隐藏三行。
    fn fold_value() -> &'static str {
        "fn a() {\n    x();\n    y();\n}\nfn b() {}"
    }

    const FOLD_BLOCK: TextCodeFold = TextCodeFold { start: 7, end: 28 };

    fn focused_editor(value: &str) -> (AppContext, DocumentId, Entity<TextArea>, StableNodeId) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new(value).code_editor(true))
            .unwrap();
        let node = area.stable_id();
        context.focus_node(document, node).unwrap();
        (context, document, area, node)
    }

    fn offered_editor(
        value: &str,
        folds: &[TextCodeFold],
    ) -> (AppContext, DocumentId, Entity<TextArea>, StableNodeId) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new(value)
                    .code_editor(true)
                    .code_folds(Arc::from(folds.to_vec().into_boxed_slice())),
            )
            .unwrap();
        let node = area.stable_id();
        context.focus_node(document, node).unwrap();
        (context, document, area, node)
    }

    fn selection_of(context: &AppContext, node: StableNodeId) -> (usize, usize) {
        let state = context.world().text_input(node).unwrap();
        (state.selection.anchor, state.selection.focus)
    }

    #[test]
    fn toggle_focused_text_fold_switches_and_reports_state() {
        let (mut context, document, _area, node) = offered_editor(fold_value(), &[FOLD_BLOCK]);
        // 光标不在任何折叠上：不消费。
        assert!(!context.toggle_focused_text_fold(document, None).unwrap());
        assert!(context.focused_text_collapsed_folds(document).is_empty());

        // 光标移到折叠起始行：toggle 折叠，查询接口回报状态。
        context
            .update_component(_area, |area, _| {
                area.state.selection = TextSelection::caret(3);
            })
            .unwrap();
        assert!(context.toggle_focused_text_fold(document, None).unwrap());
        assert_eq!(
            context.focused_text_collapsed_folds(document),
            vec![FOLD_BLOCK]
        );
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            fold_value()
        );
        // 折叠是纯视图：值不变、change 事件零条（由宿主值监听保证）。

        // 再次 toggle 展开；指定偏移与光标等价。
        assert!(context.toggle_focused_text_fold(document, None).unwrap());
        assert!(context.focused_text_collapsed_folds(document).is_empty());
        assert!(
            context
                .toggle_focused_text_fold(document, Some(10))
                .unwrap()
        );
        assert_eq!(
            context.focused_text_collapsed_folds(document),
            vec![FOLD_BLOCK]
        );

        // 全部展开。
        assert!(context.unfold_all_focused_text_folds(document).unwrap());
        assert!(context.focused_text_collapsed_folds(document).is_empty());
        assert!(!context.unfold_all_focused_text_folds(document).unwrap());
    }

    #[test]
    fn caret_movement_skips_collapsed_regions() {
        let (mut context, document, _area, node) = offered_editor(fold_value(), &[FOLD_BLOCK]);
        context
            .update_component(_area, |area, _| {
                area.state.selection = TextSelection::caret(0);
            })
            .unwrap();
        assert!(context.toggle_focused_text_fold(document, None).unwrap());
        let mut shaper = crate::MeasureTextShaper;

        // Down：跳过隐藏的三行，落到下一可见行 `fn b()` 行首。
        assert!(
            context
                .move_focused_text_caret(document, TextCaretIntent::Down, false, None)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (29, 29));

        // Up：回到文档首行行首（隐藏行的偏移不可达）。
        assert!(
            context
                .move_focused_text_caret(document, TextCaretIntent::Up, false, None)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (0, 0));

        // LineEnd 在折叠起始行：落到可见行尾（摘要标记之后），随后
        // LineStart 回到行首。
        context
            .update_component(_area, |area, _| {
                area.state.selection = TextSelection::caret(3);
            })
            .unwrap();
        assert!(
            context
                .move_focused_text_caret(document, TextCaretIntent::LineEnd, false, None)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (28, 28));
        assert!(
            context
                .move_focused_text_caret(document, TextCaretIntent::LineStart, false, None)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (0, 0));

        // 带几何探针的垂直移动同样按显示视图解析。
        context
            .update_component(_area, |area, _| {
                area.state.selection = TextSelection::caret(0);
            })
            .unwrap();
        assert!(
            context
                .move_focused_text_caret(document, TextCaretIntent::Down, false, Some(&mut shaper))
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (29, 29));

        // DocEnd / DocStart 始终到达边界。
        assert!(
            context
                .move_focused_text_caret(document, TextCaretIntent::DocEnd, false, None)
                .unwrap()
        );
        assert_eq!(
            selection_of(&context, node),
            (fold_value().len(), fold_value().len())
        );
        assert!(
            context
                .move_focused_text_caret(document, TextCaretIntent::DocStart, false, None)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (0, 0));
    }

    #[test]
    fn find_next_match_unfolds_fold_hiding_the_match() {
        let (mut context, document, _area, node) = offered_editor(fold_value(), &[FOLD_BLOCK]);
        assert!(
            context
                .toggle_focused_text_fold(document, Some(10))
                .unwrap()
        );
        assert_eq!(
            context.focused_text_collapsed_folds(document),
            vec![FOLD_BLOCK]
        );

        // "x();" 只出现在折叠区间内：导航命中后该折叠自动展开（reveal
        // 语义），选区落在匹配上。
        let found = context
            .find_next_focused_text_match(document, "x();", crate::TextSearchOptions::default())
            .unwrap();
        assert!(found);
        assert_eq!(context.focused_text_collapsed_folds(document), vec![]);
        assert_eq!(selection_of(&context, node), (13, 17));
    }

    #[test]
    fn select_all_occurrences_unfolds_fold_containing_new_cursor() {
        let value = "fn a() {\n    x();\n    y();\n}\nlet x();";
        let fold = TextCodeFold { start: 7, end: 28 };
        let (mut context, document, area, node) = offered_editor(value, &[fold]);
        context
            .update_component(area, |area_view, _| {
                area_view.state.selection = TextSelection {
                    anchor: 13,
                    focus: 17,
                };
            })
            .unwrap();
        assert!(
            context
                .select_all_focused_text_occurrences(document)
                .unwrap()
        );
        // 新光标（第二处出现）落在折叠外，但主光标在折叠隐藏区间内——
        // 该折叠自动展开；两个光标都落在真实出现上。
        assert_eq!(context.focused_text_collapsed_folds(document), vec![]);
        let state = context.world().text_input(node).unwrap();
        assert_eq!(
            state.selections().into_owned(),
            vec![
                TextSelection {
                    anchor: 13,
                    focus: 17
                },
                TextSelection {
                    anchor: 33,
                    focus: 37
                },
            ]
        );
        assert_eq!(state.value, value);
    }

    #[test]
    fn snippet_insertion_lands_on_zero_and_tabs_through_stops() {
        let (mut context, document, _area, node) = focused_editor("fn main() {}");
        context
            .update_component(_area, |area, _| {
                area.state.selection = TextSelection::caret(0);
            })
            .unwrap();
        let snippet = TextSnippet::new("let", "let $1 = $2;$0");
        assert!(
            context
                .insert_focused_text_snippet(document, &snippet)
                .unwrap()
        );
        // `$N` 移除后插入原文，光标停在 `$0`（插入文本末尾）。
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "let  = ;fn main() {}"
        );
        assert_eq!(selection_of(&context, node), (8, 8));

        // Tab 依次跳 $1、$2；跳完后会话结束，caret 停在最后跳位。
        assert!(
            context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (4, 4));
        assert!(
            context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (7, 7));
        // 会话仍在：最后一个跳位之后的 Tab 结束会话（消费最后一次）。
        assert!(
            context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (7, 7));
        // 会话已结束：Tab 不再被 snippet 消费。
        assert!(
            !context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
    }

    #[test]
    fn snippet_shift_tab_walks_stops_backwards_and_ends_session() {
        let (mut context, document, _area, node) = focused_editor("");
        let snippet = TextSnippet::new("let", "let $1 = $2;$0");
        assert!(
            context
                .insert_focused_text_snippet(document, &snippet)
                .unwrap()
        );
        // 前进到 $2（第二个跳位）。
        assert!(
            context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
        assert!(
            context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (7, 7));
        // Shift+Tab 回到 $1；在第一个跳位上再后退即结束会话。
        assert!(
            context
                .advance_focused_text_snippet(document, true)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (4, 4));
        assert!(
            context
                .advance_focused_text_snippet(document, true)
                .unwrap()
        );
        assert!(
            !context
                .advance_focused_text_snippet(document, true)
                .unwrap()
        );
    }

    #[test]
    fn snippet_tab_precedes_indent_and_escape_ends_session() {
        let (mut context, document, _area, node) = focused_editor("");
        let snippet = TextSnippet::new("if", "if $1 {\n\t$2\n}$0");
        assert!(
            context
                .insert_focused_text_snippet(document, &snippet)
                .unwrap()
        );
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "if  {\n\t\n}"
        );

        // 会话内 Tab 跳位而不是插入缩进（值不变，只有光标移动）。
        assert!(
            context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (3, 3));
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "if  {\n\t\n}"
        );
        assert!(
            context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (7, 7));

        // Esc 结束会话；之后再 Esc 没有会话可结束。
        assert!(context.cancel_focused_text_snippet(document).unwrap());
        assert!(!context.cancel_focused_text_snippet(document).unwrap());

        // 无会话且代码编辑器：Tab 回到缩进行为。
        assert!(
            !context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
        assert!(context.code_edit_indent(document, false).unwrap());
        assert_ne!(
            context.world().text_input(node).unwrap().value,
            "if  {\n\t\n}"
        );
    }

    #[test]
    fn snippet_insert_keeps_additional_cursors_on_primary_only() {
        let (mut context, document, area, node) = focused_editor("ab\ncd");
        context
            .update_component(area, |area_view, _| {
                area_view.state.selection = TextSelection::caret(2);
                area_view.state.additional_selections = vec![TextSelection::caret(5)];
            })
            .unwrap();
        let snippet = TextSnippet::new("x", "[$1]$0");
        assert!(
            context
                .insert_focused_text_snippet(document, &snippet)
                .unwrap()
        );
        let state = context.world().text_input(node).unwrap();
        // 只作用主光标；附加光标随编辑平移保留（多光标限制，声明行为）。
        assert_eq!(state.value, "ab[]\ncd");
        assert_eq!((state.selection.anchor, state.selection.focus), (4, 4));
        assert_eq!(state.additional_selections, vec![TextSelection::caret(7)]);
    }

    #[test]
    fn external_edit_shifts_snippet_stops_and_ends_session_on_invalidated_stop() {
        let (mut context, document, area, node) = focused_editor("");
        let snippet = TextSnippet::new("let", "let $1 = $2;$0");
        context
            .insert_focused_text_snippet(document, &snippet)
            .unwrap();

        // 在 $0 处打字（插入文本末尾）：跳位不受影响，会话保持。
        context.replace_focused_text(document, "X").unwrap();
        assert!(
            context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (4, 4));

        // 编辑区间严格覆盖 $1 跳位（删除覆盖它的 span）→ 会话失效结束。
        context
            .update_component(area, |area_view, _| {
                area_view.state.selection = TextSelection {
                    anchor: 2,
                    focus: 6,
                };
            })
            .unwrap();
        assert!(
            context
                .delete_focused_text(document, crate::TextDeleteKind::Backward)
                .unwrap()
        );
        assert!(
            !context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
    }

    #[test]
    fn snippet_without_stops_never_opens_session() {
        let (mut context, document, _area, node) = focused_editor("");
        let snippet = TextSnippet::new("fn", "fn main() {}");
        assert!(
            context
                .insert_focused_text_snippet(document, &snippet)
                .unwrap()
        );
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "fn main() {}"
        );
        assert_eq!(selection_of(&context, node), (12, 12));
        assert!(
            !context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
    }

    #[test]
    fn select_range_sets_sole_selection_clamped_and_clears_extra_cursors() {
        // "a中文"：字节 1..4 是"中"，4..7 是"文"。
        let (mut context, document, area, node) = focused_editor("a中文");
        context
            .update_component(area, |area_view, _| {
                area_view.state.selection = TextSelection::caret(0);
                area_view.state.additional_selections = vec![TextSelection::caret(1)];
            })
            .unwrap();

        // 越界偏移钳到值长度；附加光标清除；start == end 为光标。
        assert!(
            context
                .select_focused_text_range(document, 100, 200)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (7, 7));
        assert!(
            context
                .world()
                .text_input(node)
                .unwrap()
                .additional_selections
                .is_empty()
        );

        // 字符边界钳制：落在"中"内部（2）回退到 1，落在"文"内部（5）回退到 4。
        assert!(context.select_focused_text_range(document, 2, 5).unwrap());
        assert_eq!(selection_of(&context, node), (1, 4));

        // 无聚焦编辑器：不消费。
        let mut other = AppContext::new();
        let other_document = DocumentId::new(9).unwrap();
        assert!(
            !other
                .select_focused_text_range(other_document, 0, 1)
                .unwrap()
        );
    }

    #[test]
    fn select_range_unfolds_fold_hiding_endpoint() {
        let (mut context, document, _area, node) = offered_editor(fold_value(), &[FOLD_BLOCK]);
        assert!(context.toggle_focused_text_fold(document, Some(3)).unwrap());
        assert_eq!(
            context.focused_text_collapsed_folds(document),
            vec![FOLD_BLOCK]
        );

        // 选区端点落在隐藏区间内部：该折叠自动展开（reveal 语义）。
        assert!(context.select_focused_text_range(document, 13, 17).unwrap());
        assert_eq!(context.focused_text_collapsed_folds(document), vec![]);
        assert_eq!(selection_of(&context, node), (13, 17));

        // 起点在折叠外、终点落在隐藏区间内：区间覆盖同样展开。
        assert!(context.toggle_focused_text_fold(document, Some(3)).unwrap());
        assert!(context.select_focused_text_range(document, 0, 15).unwrap());
        assert_eq!(context.focused_text_collapsed_folds(document), vec![]);
        assert_eq!(selection_of(&context, node), (0, 15));
    }

    #[test]
    fn select_range_into_nested_folds_unfolds_only_covering_parent() {
        let child = TextCodeFold::new(12, 20);
        let (mut context, document, _area, node) =
            offered_editor(fold_value(), &[FOLD_BLOCK, child]);
        // 先折叠父级，再折叠子级（偏移 15 的最内层折叠）。
        assert!(context.toggle_focused_text_fold(document, Some(3)).unwrap());
        assert!(
            context
                .toggle_focused_text_fold(document, Some(15))
                .unwrap()
        );
        assert_eq!(
            context.focused_text_collapsed_folds(document),
            vec![FOLD_BLOCK, child]
        );

        // display 语义：被父级覆盖的子折叠不进入显示视图，偏移由父级
        // 遮住——只展开父级，子级保持折叠。
        assert!(context.select_focused_text_range(document, 15, 16).unwrap());
        assert_eq!(context.focused_text_collapsed_folds(document), vec![child]);
        assert_eq!(selection_of(&context, node), (15, 16));
    }

    #[test]
    fn select_range_is_selection_only_and_emits_no_change() {
        let (mut context, document, area, node) = offered_editor(fold_value(), &[FOLD_BLOCK]);
        let changes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&changes);
        context
            .on(area, move |_view, event: &crate::TextChanged, _cx| {
                sink.lock().unwrap().push(event.value.clone());
            })
            .unwrap();
        assert!(context.toggle_focused_text_fold(document, Some(3)).unwrap());

        // 选区写入 + 折叠展开都不发 change 事件（同 move 命令）。
        assert!(context.select_focused_text_range(document, 15, 17).unwrap());
        assert_eq!(context.focused_text_collapsed_folds(document), vec![]);
        assert_eq!(selection_of(&context, node), (15, 17));
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            fold_value()
        );
        assert!(changes.lock().unwrap().is_empty());

        // 选区本就一致：返回 false，依旧零事件。
        assert!(!context.select_focused_text_range(document, 15, 17).unwrap());
        assert!(changes.lock().unwrap().is_empty());
    }
}

#[cfg(test)]
mod completion_tests {
    use crate::{AppContext, DocumentId, Entity, StableNodeId, TextArea, TextCompletion};
    use std::sync::Arc;

    fn items(labels: &[&str]) -> Arc<[TextCompletion]> {
        labels
            .iter()
            .map(|label| TextCompletion::new(*label, "fn"))
            .collect::<Vec<_>>()
            .into()
    }

    fn selection_of(context: &AppContext, node: StableNodeId) -> (usize, usize) {
        let state = context.world().text_input(node).unwrap();
        (state.selection.anchor, state.selection.focus)
    }

    fn completion_editor(
        value: &str,
        completions: Arc<[TextCompletion]>,
    ) -> (AppContext, DocumentId, Entity<TextArea>, StableNodeId) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new(value).completions(completions))
            .unwrap();
        let node = area.stable_id();
        context.focus_node(document, node).unwrap();
        (context, document, area, node)
    }

    fn session_of(context: &AppContext, node: StableNodeId) -> Option<(usize, usize, bool, usize)> {
        context.world().text_completion_view(node).map(|state| {
            (
                state.items.len(),
                state.selected,
                state.dismissed,
                state.scroll,
            )
        })
    }

    fn caret_to(context: &mut AppContext, area: Entity<TextArea>, caret: usize) {
        context
            .update_component(area, |view, _| {
                view.state.selection = crate::TextSelection::caret(caret);
            })
            .unwrap();
    }

    #[test]
    fn feed_activates_session_and_same_feed_keeps_selection() {
        let (mut context, document, area, node) =
            completion_editor("fn f", items(&["fn", "format"]));
        // 喂入非空列表激活会话：选中第一条。
        assert_eq!(session_of(&context, node), Some((2, 0, false, 0)));

        // Down：钳在最后一条（导航无可做，返回 false，但弹层仍活跃）。
        assert!(
            context
                .move_focused_text_completion(document, true)
                .unwrap()
        );
        assert_eq!(session_of(&context, node), Some((2, 1, false, 0)));
        assert!(
            !context
                .move_focused_text_completion(document, true)
                .unwrap()
        );
        assert_eq!(session_of(&context, node), Some((2, 1, false, 0)));

        // 重喂相同列表：键盘选中保持。
        context
            .update_component(area, |view, _| {
                view.completions = items(&["fn", "format"]);
            })
            .unwrap();
        assert_eq!(session_of(&context, node), Some((2, 1, false, 0)));

        // 重喂不同列表：视为新会话（选中归零、重新打开）。
        context
            .update_component(area, |view, _| {
                view.completions = items(&["format"]);
            })
            .unwrap();
        assert_eq!(session_of(&context, node), Some((1, 0, false, 0)));

        // 喂空列表：会话移除（弹层关闭）。
        context
            .update_component(area, |view, _| {
                view.completions = Arc::from([]);
            })
            .unwrap();
        assert_eq!(session_of(&context, node), None);
    }

    #[test]
    fn navigation_scrolls_window_and_clamps() {
        let (mut context, document, _area, node) = completion_editor(
            "",
            items(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]),
        );
        // Down 走到第 9 条：第 9 步时窗口滑出一页（scroll = 8 + 1 - 8 = 1）。
        for _ in 0..8 {
            assert!(
                context
                    .move_focused_text_completion(document, true)
                    .unwrap()
            );
        }
        assert_eq!(session_of(&context, node), Some((10, 8, false, 1)));
        // 第 10 条：窗口再次滑动使选中项可见（scroll = 9 + 1 - 8 = 2）。
        assert!(
            context
                .move_focused_text_completion(document, true)
                .unwrap()
        );
        assert_eq!(session_of(&context, node), Some((10, 9, false, 2)));
        // 边界：再 Down 不再移动（导航返回 false 表示无可做）。
        assert!(
            !context
                .move_focused_text_completion(document, true)
                .unwrap()
        );
        // Up 回到第一条：scroll 跟回 0。
        for _ in 0..9 {
            assert!(
                context
                    .move_focused_text_completion(document, false)
                    .unwrap()
            );
        }
        assert_eq!(session_of(&context, node), Some((10, 0, false, 0)));
        assert!(
            !context
                .move_focused_text_completion(document, false)
                .unwrap()
        );
    }

    #[test]
    fn accept_replaces_word_prefix_and_emits_one_change() {
        let (mut context, document, area, node) =
            completion_editor("let x = fo", items(&["food", "foobar"]));
        caret_to(&mut context, area, "let x = fo".len());
        let changes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&changes);
        context
            .on(area, move |_view, event: &crate::TextChanged, _cx| {
                sink.lock().unwrap().push(event.value.clone());
            })
            .unwrap();

        // 接受选中项（第一条）：光标前的词前缀 `fo` 替换为 label，一次
        // TextChanged，光标停在插入末尾。
        assert!(
            context
                .accept_focused_text_completion(document, None)
                .unwrap()
        );
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "let x = food"
        );
        assert_eq!(
            selection_of(&context, node),
            ("let x = food".len(), "let x = food".len())
        );
        assert_eq!(*changes.lock().unwrap(), vec!["let x = food".to_string()]);

        // 点击接受第二行候选：替换同一词前缀，再发一次 TextChanged。
        caret_to(&mut context, area, "let x = food".len());
        assert!(
            context
                .accept_focused_text_completion(document, Some(1))
                .unwrap()
        );
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "let x = foobar"
        );
        assert_eq!(changes.lock().unwrap().len(), 2);
    }

    #[test]
    fn dismiss_sticks_for_same_list_and_reopens_on_new_list() {
        let (mut context, document, area, node) = completion_editor("f", items(&["fn", "format"]));
        assert_eq!(session_of(&context, node), Some((2, 0, false, 0)));

        // Esc 关闭：会话数据保留但 dismissed 置位；重复关闭返回 false
        // （Esc 落到后续优先级）。
        assert!(context.dismiss_focused_text_completion(document).unwrap());
        assert_eq!(session_of(&context, node), Some((2, 0, true, 0)));
        assert!(!context.dismiss_focused_text_completion(document).unwrap());

        // 光标移动触发组件重投影、宿主重喂相同列表：弹层不复活。
        caret_to(&mut context, area, 0);
        assert_eq!(session_of(&context, node), Some((2, 0, true, 0)));

        // 打字后宿主重喂不同列表：新会话（重新打开、选中归零）。
        caret_to(&mut context, area, 1);
        context
            .update_component(area, |view, _| {
                view.completions = items(&["format"]);
            })
            .unwrap();
        assert_eq!(session_of(&context, node), Some((1, 0, false, 0)));

        // 无会话时 dismiss 与导航都返回 false。
        context
            .update_component(area, |view, _| {
                view.completions = Arc::from([]);
            })
            .unwrap();
        assert!(!context.dismiss_focused_text_completion(document).unwrap());
        assert!(
            !context
                .move_focused_text_completion(document, true)
                .unwrap()
        );
        assert!(
            !context
                .accept_focused_text_completion(document, None)
                .unwrap()
        );
    }

    #[test]
    fn completion_wheel_scroll_clamps_into_range() {
        let (mut context, document, _area, node) =
            completion_editor("", items(&["a", "b", "c", "d", "e"]));
        // 向列表末尾方向滚过边界：钳在最后一条可见。
        assert!(context.scroll_focused_text_completion(document, 7).unwrap());
        assert_eq!(session_of(&context, node), Some((5, 0, false, 4)));
        // 回滚到顶。
        assert!(
            context
                .scroll_focused_text_completion(document, -9)
                .unwrap()
        );
        assert_eq!(session_of(&context, node), Some((5, 0, false, 0)));
        // 无会话时不消费。
        context
            .update_component(_area, |view, _| {
                view.completions = Arc::from([]);
            })
            .unwrap();
        assert!(!context.scroll_focused_text_completion(document, 1).unwrap());
    }

    #[test]
    fn accept_only_edits_primary_cursor_and_maps_additional() {
        let (mut context, document, area, node) = completion_editor("fo\nfo", items(&["food"]));
        context
            .update_component(area, |view, _| {
                view.state.selection = crate::TextSelection::caret(2);
                view.state.additional_selections = vec![crate::TextSelection::caret(5)];
            })
            .unwrap();
        assert!(
            context
                .accept_focused_text_completion(document, None)
                .unwrap()
        );
        let state = context.world().text_input(node).unwrap();
        // 只作用主光标；附加光标随编辑平移保留（与 snippet 同款限制）。
        assert_eq!(state.value, "food\nfo");
        assert_eq!(state.selection, crate::TextSelection::caret(4));
        assert_eq!(
            state.additional_selections,
            vec![crate::TextSelection::caret(7)]
        );
    }
}

#[cfg(test)]
mod minimap_tests {
    use crate::{
        AppContext, DocumentId, Entity, MeasureTextShaper, MutationQueue, StableNodeId, TextArea,
        TextSelection,
    };
    use std::time::Duration;

    /// 30 行 minimap 编辑器：行高 10、布局 200×100，已布局并 shape，
    /// 且已聚焦。
    fn minimap_editor() -> (AppContext, DocumentId, Entity<TextArea>, StableNodeId) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let value = (0..30)
            .map(|index| format!("line{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut view = TextArea::new(value).minimap(true);
        {
            use nana_ui_core::{LengthSpec, LineHeightSpec};
            let layout = std::sync::Arc::make_mut(&mut view.style.layout);
            layout.width = Some(LengthSpec::Px(200.0));
            layout.height = Some(LengthSpec::Px(100.0));
            layout.line_height = Some(LineHeightSpec::Absolute(10.0));
            layout.padding_left = Some(LengthSpec::Px(0.0));
            layout.padding_right = Some(LengthSpec::Px(0.0));
            layout.padding_top = Some(LengthSpec::Px(0.0));
            layout.padding_bottom = Some(LengthSpec::Px(0.0));
        }
        let area = context.create_component(document, view).unwrap();
        let node = area.stable_id();
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            node,
            crate::LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
        );
        context.commit_mutations(mutations).unwrap();
        context.world_mut().resolve_styles(&[node]).unwrap();
        context
            .world_mut()
            .shape_text(&[node], &mut MeasureTextShaper)
            .unwrap();
        context.focus_node(document, node).unwrap();
        (context, document, area, node)
    }

    fn minimap_panel(context: &AppContext, node: StableNodeId) -> crate::LayoutBox {
        match context.world().component_geometry(node).unwrap() {
            crate::ComponentGeometry::TextInput {
                minimap: Some(minimap),
                ..
            } => minimap.panel,
            other => panic!("expected minimap geometry, got {other:?}"),
        }
    }

    fn press(
        context: &mut AppContext,
        document: DocumentId,
        node: StableNodeId,
        x: f32,
        y: f32,
    ) -> bool {
        context
            .text_editor_pointer_press(
                document,
                node,
                1,
                x,
                y,
                false,
                false,
                Duration::ZERO,
                &mut MeasureTextShaper,
            )
            .unwrap()
    }

    fn scroll_y_of(context: &AppContext, node: StableNodeId) -> f32 {
        context.world().scroll_offset(node).unwrap().y
    }

    /// 点击条内 `offset_y`（面板内偏移）后的期望滚动：命中行居中并钳到
    /// 文档范围（行高 10，30 行；内容盒从世界侧读取，含边框）。
    fn expected_scroll(context: &AppContext, node: StableNodeId, offset_y: f32) -> f32 {
        let (content, _) = context.world().text_input_pointer_context(node).unwrap();
        let line = (offset_y / 2.0).floor();
        let max_scroll = (30.0 * 10.0 - content.height).max(0.0);
        (line * 10.0 + 5.0 - content.height / 2.0).clamp(0.0, max_scroll)
    }

    #[test]
    fn minimap_press_scrolls_viewport_without_moving_caret_or_selection() {
        let (mut context, document, area, node) = minimap_editor();
        let state = context.world().text_input(node).unwrap();
        let selection_before = state.selection;
        let additional_before = state.additional_selections.clone();

        // 条内中部点击：点击行（面板内 50px → 第 25 行）居中。
        let panel = minimap_panel(&context, node);
        assert!(press(
            &mut context,
            document,
            node,
            panel.x + 32.0,
            panel.y + 50.0
        ));
        let expected = expected_scroll(&context, node, 50.0);
        assert_eq!(scroll_y_of(&context, node), expected);
        // 组件字段同步（投影权威），下一帧不被改写回去。
        assert_eq!(
            context
                .read(area, |view: &TextArea| view.scroll_offset.y)
                .unwrap(),
            expected
        );
        // 按下被消费：光标与选区纹丝不动。
        let state = context.world().text_input(node).unwrap();
        assert_eq!(state.selection, selection_before);
        assert_eq!(state.additional_selections, additional_before);
    }

    #[test]
    fn minimap_drag_follows_pointer_until_release() {
        let (mut context, document, _area, node) = minimap_editor();
        let panel = minimap_panel(&context, node);
        assert!(press(
            &mut context,
            document,
            node,
            panel.x + 32.0,
            panel.y + 10.0
        ));
        assert_eq!(
            scroll_y_of(&context, node),
            expected_scroll(&context, node, 10.0)
        );

        // 按住拖动：视口连续跟随换算（面板内 50px → 第 25 行居中）。
        assert!(
            context
                .text_editor_pointer_drag(
                    document,
                    1,
                    panel.x + 32.0,
                    panel.y + 50.0,
                    &mut MeasureTextShaper
                )
                .unwrap()
        );
        assert_eq!(
            scroll_y_of(&context, node),
            expected_scroll(&context, node, 50.0)
        );

        // 释放后指针移动不再跟随。
        context.text_editor_pointer_release(1);
        assert!(
            !context
                .text_editor_pointer_drag(
                    document,
                    1,
                    panel.x + 32.0,
                    panel.y + 70.0,
                    &mut MeasureTextShaper
                )
                .unwrap()
        );
        assert_eq!(
            scroll_y_of(&context, node),
            expected_scroll(&context, node, 50.0)
        );
    }

    #[test]
    fn minimap_click_outside_strip_places_caret_normally() {
        let (mut context, document, _area, node) = minimap_editor();
        let panel = minimap_panel(&context, node);
        // 条外（内容区左侧）点击：常规 caret 落点，非 minimap 消费语义。
        assert!(press(&mut context, document, node, 30.0, panel.y + 10.0));
        let state = context.world().text_input(node).unwrap();
        assert_ne!(
            state.selection.focus, 0,
            "plain press inside text must move the caret"
        );
    }

    #[test]
    fn minimap_scroll_survives_caret_reveal_until_caret_moves() {
        let (mut context, document, _area, node) = minimap_editor();
        let panel = minimap_panel(&context, node);
        // 光标在第 0 行；点击条内底部 → 视口滚到文档尾部（钳到 max）。
        assert!(press(
            &mut context,
            document,
            node,
            panel.x + 32.0,
            panel.y + 70.0
        ));
        let navigated = scroll_y_of(&context, node);
        assert!(navigated > 150.0, "click near the bottom must scroll far");

        // 重跑几何：视口不回弹到光标处（视口钉住生效）——文本区随滚动
        // 停在导航位置。
        let (content, _) = context.world().text_input_pointer_context(node).unwrap();
        let extracted = &context.world().extract_nodes(&[node])[0];
        let crate::ComponentGeometry::TextInput { text, .. } =
            extracted.component_geometry.as_ref().unwrap()
        else {
            panic!("expected text input geometry");
        };
        assert!((text.bounds.y - (content.y - navigated)).abs() < 0.01);

        // 光标移动（选区写入）：钉住失效，reveal 恢复权威——视口回到光标。
        assert!(
            context
                .select_focused_text_range(document, 0, "line0".len())
                .unwrap()
        );
        context
            .world_mut()
            .shape_text(&[node], &mut MeasureTextShaper)
            .unwrap();
        let extracted = &context.world().extract_nodes(&[node])[0];
        let crate::ComponentGeometry::TextInput { text, .. } =
            extracted.component_geometry.as_ref().unwrap()
        else {
            panic!("expected text input geometry");
        };
        // 光标在第 0 行（y=0），reveal 后视口回顶部：文本区 y 回到内容顶。
        assert!((text.bounds.y - content.y).abs() < 0.01);
    }
}
