//! Text-editor caret, selection, and code-editing commands on [`AppContext`].
//!
//! Keyboard and pointer adapters resolve intents and hand them here; this
//! module owns which focused component is editable, how offsets map onto the
//! committed value, and when change events fire. Geometry-dependent queries
//! receive the host shaper so layout stays backend-owned.

use super::{AppContext, DocumentId, EditableText, Entity, FrameworkError, StableNodeId};
use super::{TextArea, TextInput, TextInputState, TextSelection};
use crate::text_editing::{
    TextCaretIntent, TextReplacement, apply_replacement, auto_indent_newline, auto_pair_edit,
    caret_focus, caret_offset_at_point, delete_backward, delete_forward, delete_to_line_end,
    delete_to_line_start, delete_word_backward, delete_word_forward, indent_selection,
    logical_line_range, moved_selection, outdent_selection, toggle_line_comment,
    vertical_caret_focus, vertical_caret_focus_logical, word_range_at,
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

    /// Move the caret or selection edge of the focused text editor.
    ///
    /// Vertical intents use the host shaper when available and fall back to
    /// logical-line column preservation otherwise.
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
        let vertical = matches!(intent, TextCaretIntent::Up | TextCaretIntent::Down);
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
                match vertical_caret_focus(
                    &state.value,
                    state.selection,
                    intent,
                    extend,
                    goal,
                    geometry.probe(),
                ) {
                    Some((selection, goal)) => {
                        self.caret_goal_x = Some((focused.node, goal));
                        selection
                    }
                    None => return Ok(false),
                }
            } else {
                match vertical_caret_focus_logical(&state.value, state.selection, intent, extend) {
                    Some(selection) => selection,
                    None => return Ok(false),
                }
            }
        } else if vertical {
            // Single-line fields map Up/Down onto the line boundaries.
            let mapped = match intent {
                TextCaretIntent::Up => TextCaretIntent::LineStart,
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

    /// Delete around the caret of the focused text editor.
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
        self.edit_editor(focused.node, focused.kind, |state| {
            let replacement = match kind {
                TextDeleteKind::Backward => delete_backward(&state.value, state.selection)?,
                TextDeleteKind::Forward => delete_forward(&state.value, state.selection)?,
                TextDeleteKind::WordBackward => {
                    delete_word_backward(&state.value, state.selection)?
                }
                TextDeleteKind::WordForward => delete_word_forward(&state.value, state.selection)?,
                TextDeleteKind::LineStart => delete_to_line_start(&state.value, state.selection)?,
                TextDeleteKind::LineEnd => delete_to_line_end(&state.value, state.selection)?,
            };
            Some(apply_replacement(&state.value, &replacement).into())
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
        self.edit_editor(focused.node, focused.kind, |state| {
            let replacement = match &indent_unit {
                Some(unit) => auto_indent_newline(&state.value, state.selection, unit),
                None => {
                    let range = state.selection.ordered();
                    TextReplacement {
                        range: range.clone(),
                        insert: "\n".into(),
                        caret: range.start + 1,
                    }
                }
            };
            Some(apply_replacement(&state.value, &replacement).into())
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
        self.edit_editor(focused.node, focused.kind, |state| {
            let replacement = auto_pair_edit(&state.value, state.selection, typed)?;
            Some(apply_replacement(&state.value, &replacement).into())
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
        self.edit_editor(focused.node, focused.kind, |state| {
            let (value, selection) = if outdent {
                outdent_selection(&state.value, state.selection, &unit)?
            } else {
                indent_selection(&state.value, state.selection, &unit)?
            };
            Some((value, selection).into())
        })
    }

    /// Toggle line comments across the selection of the focused code editor.
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
        self.edit_editor(focused.node, focused.kind, |state| {
            let (value, selection) = toggle_line_comment(&state.value, state.selection, &prefix)?;
            Some((value, selection).into())
        })
    }

    /// Place or extend the selection of the focused editor from a pointer
    /// press. Single presses set the caret, Shift extends, double clicks
    /// select the word, triple clicks select the line.
    #[allow(clippy::too_many_arguments)]
    pub fn text_editor_pointer_press(
        &mut self,
        document: DocumentId,
        node: StableNodeId,
        pointer_id: u64,
        x: f32,
        y: f32,
        extend: bool,
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
        let selection = match count {
            2 => {
                let (start, end) = word_range_at(&state.value, offset);
                TextSelection {
                    anchor: start,
                    focus: end,
                }
            }
            3 => {
                let (start, end) = logical_line_range(&state.value, offset);
                TextSelection {
                    anchor: start,
                    focus: end,
                }
            }
            _ => {
                let anchor = if extend {
                    state.selection.anchor
                } else {
                    offset
                };
                self.text_pointer_drag = Some((pointer_id, node, anchor));
                moved_selection(state.selection, offset, extend)
            }
        };
        if count != 1 {
            self.text_pointer_drag = None;
        }
        self.write_editor_selection(node, focused.kind, selection)
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
}
