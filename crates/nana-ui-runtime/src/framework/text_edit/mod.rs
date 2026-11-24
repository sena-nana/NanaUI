//! Text-editor caret, selection, and code-editing commands on [`AppContext`].
//!
//! Keyboard and pointer adapters resolve intents and hand them here; this
//! module owns which focused component is editable, how offsets map onto the
//! committed value, and when change events fire. Geometry-dependent queries
//! receive the host shaper so layout stays backend-owned.

use super::{AppContext, DocumentId, EditableText, Entity, FrameworkError, StableNodeId};
use super::{TextArea, TextInputState, TextSelection};
use crate::text_editing::{CursorEdit, apply_cursor_edits, apply_replacement};
use crate::{CodeEditing, MutationQueue, TextAtomSpan};

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

/// Where a find/replace command searches in the focused editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextFindScope {
    /// The whole editor value (default; historical behavior).
    #[default]
    Document,
    /// Find in selection: only matches fully inside the primary selection.
    /// An empty primary selection falls back to the whole document.
    Selection,
}

mod caret;
mod code_intel;
mod find;
#[cfg(test)]
mod tests;

/// The focused plain text editor, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct FocusedTextEditor {
    pub node: StableNodeId,
    pub multiline: bool,
    pub accepts_input: bool,
    pub accepts_selection: bool,
    pub code_editing: Option<CodeEditing>,
    pub(crate) kind: TextEditorKind,
}

impl FocusedTextEditor {
    /// A [`crate::NumberInput`]: plain ArrowUp/ArrowDown step its value and
    /// Enter commits its draft, so a host routes those keys to the numeric
    /// field instead of the editor's caret and submit handling.
    pub fn is_numeric(&self) -> bool {
        self.kind == TextEditorKind::Number
    }
}

/// Text editing reached through the edited node's type
/// ([`crate::TypeBehavior::editable`]), so a program links the editing
/// paths of only the editors it creates.
#[doc(hidden)]
pub struct EditableHooks {
    /// A plain editor's own operations; `None` for a composite's search
    /// field.
    pub(super) plain: Option<&'static PlainEditorHooks>,
    pub(super) commit_ime: fn(&mut AppContext, StableNodeId, &str) -> Result<bool, FrameworkError>,
    pub(super) delete_surrounding:
        fn(&mut AppContext, StableNodeId, usize, usize) -> Result<bool, FrameworkError>,
    pub(super) replace_selection: fn(
        &mut AppContext,
        StableNodeId,
        &str,
        crate::TextEditOrigin,
    ) -> Result<bool, FrameworkError>,
    pub(super) delete_backward: fn(&mut AppContext, StableNodeId) -> Result<bool, FrameworkError>,
    pub(super) selected_text: fn(&AppContext, StableNodeId) -> Option<String>,
    pub(super) select_all: fn(&mut AppContext, StableNodeId) -> Result<bool, FrameworkError>,
    /// The whole value and the selection set from outside (an assistive
    /// technology's "set value" and "set selection").
    pub(crate) set_value: fn(&mut AppContext, StableNodeId, String) -> Result<bool, FrameworkError>,
    pub(crate) set_selection:
        fn(&mut AppContext, StableNodeId, crate::TextSelection) -> Result<bool, FrameworkError>,
}

/// What only a plain editor (`TextArea`, `TextInput`, `NumberInput`) does:
/// caret moves, whole-state edits, undo.
pub(super) struct PlainEditorHooks {
    pub(super) kind: TextEditorKind,
    pub(super) info: fn(&AppContext, StableNodeId, TextEditorKind) -> Option<FocusedTextEditor>,
    pub(super) state: fn(&AppContext, StableNodeId) -> Result<TextInputState, FrameworkError>,
    pub(super) update_state: fn(
        &mut AppContext,
        StableNodeId,
        &mut dyn FnMut(&mut TextInputState) -> bool,
    ) -> Result<bool, FrameworkError>,
    pub(super) replace_state: fn(
        &mut AppContext,
        StableNodeId,
        crate::TextEditOrigin,
        TextInputState,
    ) -> Result<bool, FrameworkError>,
    pub(super) restore: fn(
        &mut AppContext,
        StableNodeId,
        super::text_history::EditorSnapshot,
    ) -> Result<bool, FrameworkError>,
    pub(super) set_state:
        fn(&mut AppContext, StableNodeId, TextInputState) -> Result<(), FrameworkError>,
}

/// The editing tables of the editor type `C`.
pub(crate) struct Editable<C>(std::marker::PhantomData<C>);

impl<C: EditableText> Editable<C> {
    /// A composite's search field.
    pub(crate) const HOOKS: EditableHooks = Self::with_plain(None);

    /// A plain editor (`C::PLAIN` names which).
    pub(crate) const EDITOR_HOOKS: EditableHooks = Self::with_plain(Some(&Self::PLAIN));

    const PLAIN: PlainEditorHooks = PlainEditorHooks {
        kind: C::PLAIN.expect("a plain editor names its kind"),
        info: |cx: &AppContext, node, kind| cx.editor_info(Entity::<C>::from_stable_id(node), kind),
        state: |cx: &AppContext, node| cx.editor_state_of::<C>(node),
        update_state: |cx: &mut AppContext, node, update| {
            cx.update_editor_state_of::<C>(node, update)
        },
        replace_state: |cx: &mut AppContext, node, origin, next| {
            cx.replace_editor_state_of::<C>(node, origin, next)
        },
        restore: |cx: &mut AppContext, node, snapshot| {
            cx.restore_editor_snapshot_of::<C>(node, snapshot)
        },
        set_state: |cx: &mut AppContext, node, state| {
            cx.update_component(Entity::<C>::from_stable_id(node), |editor: &mut C, _| {
                *editor.state_mut() = state
            })
        },
    };

    const fn with_plain(plain: Option<&'static PlainEditorHooks>) -> EditableHooks {
        EditableHooks {
            plain,
            commit_ime: |cx: &mut AppContext, node, text: &str| {
                cx.commit_editable_ime(Entity::<C>::from_stable_id(node), text)
            },
            delete_surrounding: |cx: &mut AppContext, node, before, after| {
                cx.delete_editable_surrounding(Entity::<C>::from_stable_id(node), before, after)
            },
            replace_selection: |cx: &mut AppContext, node, text: &str, origin| {
                cx.replace_editable_selection(Entity::<C>::from_stable_id(node), text, origin)
            },
            delete_backward: |cx: &mut AppContext, node| {
                cx.delete_editable_backward(Entity::<C>::from_stable_id(node))
            },
            selected_text: |cx: &AppContext, node| {
                cx.editable_selected_text(Entity::<C>::from_stable_id(node))
            },
            select_all: |cx: &mut AppContext, node| {
                cx.select_all_editable(Entity::<C>::from_stable_id(node))
            },
            set_value: |cx: &mut AppContext, node, value| {
                cx.set_editable_value(Entity::<C>::from_stable_id(node), value)
            },
            set_selection: |cx: &mut AppContext, node, selection| {
                cx.set_editable_selection(Entity::<C>::from_stable_id(node), selection)
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextEditorKind {
    Area,
    Field,
    /// A [`crate::NumberInput`]'s draft. Edits change the draft only; the
    /// committed number is parsed from it on Enter or blur.
    Number,
}

/// 拖拽移动选中文本的状态机（见
/// [`AppContext::text_editor_pointer_press`]）。`pending → active → 落点
/// 执行`：按下落在主选区内部时进入 `pending`（不塌缩选区），指针移动
/// 超过 [`TEXT_SELECTION_DRAG_THRESHOLD`] 进入 `active` 并在世界侧发布
/// 落点指示线；释放时 `active` 执行移动/复制（单次拼接修订，一步撤销），
/// `pending` 回落为普通点击落 caret。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextSelectionDrag {
    pub pointer_id: u64,
    pub node: StableNodeId,
    pub kind: TextEditorKind,
    /// 按下点（世界坐标），用于阈值判定。
    pub press: (f32, f32),
    /// 源选区（按下时主选区，值空间，已有序）。
    pub source: (usize, usize),
    /// 已超过阈值进入拖拽态。
    pub active: bool,
    /// Alt 按住 = 复制（macOS 惯例；普通拖拽为移动）。
    pub copy: bool,
    /// 当前落点（值空间；落在源选区内部时为 `None`，指示无效落点）。
    pub target: Option<usize>,
}

/// 进入拖拽态的指针位移阈值（像素）。
pub(crate) const TEXT_SELECTION_DRAG_THRESHOLD: f32 = 4.0;

/// One committed value edit: the next value and where the caret lands.
pub(super) struct EditorEdit {
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
    /// Identify the focused plain text editor (`TextArea`, `TextInput`, or a
    /// `NumberInput`'s draft).
    ///
    /// Composite search surfaces (palettes, menus, dropdowns) own their
    /// navigation and are deliberately not plain editors.
    pub fn focused_text_editor(&self, document: DocumentId) -> Option<FocusedTextEditor> {
        if self.has_focused_ime_composition(document) {
            return None;
        }
        self.focused_plain_editor(document)
    }

    /// Whether a plain text editor is focused with an IME composition in
    /// progress, which hides it from [`Self::focused_text_editor`]. Its
    /// navigation keys then belong to the composition: a host keeps them
    /// from reaching enclosing navigation (tables, trees). Composite search
    /// surfaces are not plain editors and keep their own list navigation.
    pub fn focused_text_editor_composing(&self, document: DocumentId) -> bool {
        self.has_focused_ime_composition(document)
            && self.focused_plain_editor_node(document).is_some()
    }

    /// The focused plain editor's node, composition or not.
    pub(super) fn focused_plain_editor_node(&self, document: DocumentId) -> Option<StableNodeId> {
        self.focused_plain_editor_kind(document)
            .map(|(node, _)| node)
    }

    fn focused_plain_editor(&self, document: DocumentId) -> Option<FocusedTextEditor> {
        let (node, kind) = self.focused_plain_editor_kind(document)?;
        (self.plain_editor(node)?.info)(self, node, kind)
    }

    /// The one probe for which plain editor is focused, composition or not.
    /// Reads no component state.
    pub(super) fn focused_plain_editor_kind(
        &self,
        document: DocumentId,
    ) -> Option<(StableNodeId, TextEditorKind)> {
        let (target, _) = self.world.focused_text_input(document)?;
        Some((target, self.plain_editor_kind(target)?))
    }

    /// The plain editor component behind `node`, if it is one.
    pub(super) fn plain_editor_kind(&self, node: StableNodeId) -> Option<TextEditorKind> {
        Some(self.plain_editor(node)?.kind)
    }

    /// The plain editor operations of the editor type behind `node`.
    pub(super) fn plain_editor(&self, node: StableNodeId) -> Option<&'static PlainEditorHooks> {
        self.editable_hooks(node)?.plain
    }

    /// The editing table of the editor type behind `node`.
    pub(super) fn editable_hooks(&self, node: StableNodeId) -> Option<&'static EditableHooks> {
        self.behavior(node)?.editable
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
            accepts_selection: editable.accepts_selection(),
            code_editing: editable.code_editing().cloned(),
            kind,
        })
        .ok()
    }

    pub(super) fn editor_text_atoms(
        &self,
        node: StableNodeId,
        kind: TextEditorKind,
    ) -> Vec<TextAtomSpan> {
        match kind {
            TextEditorKind::Area => self
                .view_entity::<TextArea>(node)
                .and_then(|entity| {
                    self.read(entity, |area: &TextArea| area.atom_spans.to_vec())
                        .ok()
                })
                .unwrap_or_default(),
            TextEditorKind::Field | TextEditorKind::Number => Vec::new(),
        }
    }

    /// The text an editor's geometry probes ask about: the fold / inlay
    /// view's when there is one, the committed text otherwise — as the shared,
    /// stamped values the presentation pass laid out, so the host answers a
    /// probe without comparing the text it already holds geometry for.
    pub(super) fn editor_probe_text(
        &self,
        node: StableNodeId,
        fold_view: Option<&crate::world::TextDisplayView>,
    ) -> crate::TextValue {
        match fold_view {
            Some(view) => view.value.clone(),
            None => self
                .world
                .text_input(node)
                .map(|input| input.value_shared())
                .unwrap_or_default(),
        }
    }

    pub(super) fn editor_state(
        &self,
        node: StableNodeId,
    ) -> Result<TextInputState, FrameworkError> {
        let hooks = self
            .plain_editor(node)
            .ok_or(FrameworkError::MissingView(node))?;
        (hooks.state)(self, node)
    }

    fn editor_state_of<C: EditableText>(
        &self,
        node: StableNodeId,
    ) -> Result<TextInputState, FrameworkError> {
        self.view_entity::<C>(node)
            .and_then(|entity| {
                self.read(entity, |editable: &C| editable.state().clone())
                    .ok()
            })
            .ok_or(FrameworkError::MissingView(node))
    }

    /// Change an editor's state in place without an edit: selection moves.
    /// `update` reports whether anything changed.
    fn update_editor_state(
        &mut self,
        node: StableNodeId,
        update: impl FnOnce(&mut TextInputState) -> bool,
    ) -> Result<bool, FrameworkError> {
        let hooks = self
            .plain_editor(node)
            .ok_or(FrameworkError::MissingView(node))?;
        let mut update = Some(update);
        (hooks.update_state)(self, node, &mut |state| {
            update.take().is_some_and(|update| update(state))
        })
    }

    fn update_editor_state_of<C: EditableText>(
        &mut self,
        node: StableNodeId,
        update: &mut dyn FnMut(&mut TextInputState) -> bool,
    ) -> Result<bool, FrameworkError> {
        self.update_component(Entity::<C>::from_stable_id(node), |editable: &mut C, _| {
            update(editable.state_mut())
        })
    }

    /// Replace an editor's whole state as one journaled edit. Editors that
    /// refuse input, or the value (a length limit), decline it.
    pub(super) fn replace_editor_state(
        &mut self,
        node: StableNodeId,
        origin: crate::TextEditOrigin,
        next: TextInputState,
    ) -> Result<bool, FrameworkError> {
        let hooks = self
            .plain_editor(node)
            .ok_or(FrameworkError::MissingView(node))?;
        (hooks.replace_state)(self, node, origin, next)
    }

    fn replace_editor_state_of<C: EditableText>(
        &mut self,
        node: StableNodeId,
        origin: crate::TextEditOrigin,
        next: TextInputState,
    ) -> Result<bool, FrameworkError> {
        self.commit_editor_edit(
            Entity::<C>::from_stable_id(node),
            origin,
            move |editable: &mut C, _| {
                if !editable.accepts_input() || !editable.accepts_edit_value(&next.value) {
                    return false;
                }
                *editable.state_mut() = next;
                true
            },
        )
    }

    /// Put an editor back to a journaled snapshot (undo/redo): its text
    /// state and, for a `NumberInput`, the committed number with it. No
    /// value check: the journal only holds states the editor already took.
    pub(super) fn restore_editor_snapshot(
        &mut self,
        node: StableNodeId,
        snapshot: super::text_history::EditorSnapshot,
    ) -> Result<bool, FrameworkError> {
        let hooks = self
            .plain_editor(node)
            .ok_or(FrameworkError::MissingView(node))?;
        (hooks.restore)(self, node, snapshot)
    }

    fn restore_editor_snapshot_of<C: EditableText>(
        &mut self,
        node: StableNodeId,
        snapshot: super::text_history::EditorSnapshot,
    ) -> Result<bool, FrameworkError> {
        let super::text_history::EditorSnapshot { state, number } = snapshot;
        // `History` keeps the restore from becoming a step of its own.
        self.commit_editor_edit(
            Entity::<C>::from_stable_id(node),
            crate::TextEditOrigin::History,
            move |editable: &mut C, cx| {
                *editable.state_mut() = state;
                if let Some(number) = number {
                    editable.restore_committed_number(number, cx);
                }
                true
            },
        )
    }

    pub(super) fn write_editor_selection(
        &mut self,
        node: StableNodeId,
        selection: TextSelection,
    ) -> Result<bool, FrameworkError> {
        // 光标落在折叠隐藏区间内 → 该折叠自动展开（reveal 语义；查找导航
        // 跳转也经由此路径展开）。
        self.unfold_text_folds_containing(node, &[selection.focus])?;
        let changed = self.update_editor_state(node, |state| {
            if state.selection == selection {
                return false;
            }
            state.selection = selection;
            // The session fuses a primary that lands on another cursor; so
            // does the component, or the two disagree.
            state.normalize_selections();
            true
        })?;
        if changed {
            self.seal_editor_history(node);
        }
        Ok(changed)
    }

    /// Write the complete selection set (primary plus additional cursors),
    /// restoring the invariants in place — cursors that moved onto the same
    /// offset fuse here. Selection-only move: no change event is emitted.
    pub(super) fn write_editor_selections(
        &mut self,
        node: StableNodeId,
        selection: TextSelection,
        additional: Vec<TextSelection>,
    ) -> Result<bool, FrameworkError> {
        self.component_lifecycle.wheel_scrolls.remove(&node);
        // 任一光标落在折叠隐藏区间内 → 该折叠自动展开（附加光标进入折叠
        // 的强制展开语义；跨越折叠的范围选择不展开）。
        let focuses: Vec<usize> = std::iter::once(selection.focus)
            .chain(additional.iter().map(|selection| selection.focus))
            .collect();
        self.unfold_text_folds_containing(node, &focuses)?;
        let changed = self.update_editor_state(node, |state| {
            if state.selection == selection && state.additional_selections == additional {
                return false;
            }
            let previous = (state.selection, state.additional_selections.clone());
            state.selection = selection;
            state.additional_selections = additional;
            state.normalize_selections();
            previous.0 != state.selection || previous.1 != state.additional_selections
        })?;
        if changed {
            self.seal_editor_history(node);
        }
        Ok(changed)
    }

    /// Apply a committed value edit and emit the editor's change event.
    /// `Ok(false)` means the edit declined or changed nothing.
    pub(super) fn edit_editor(
        &mut self,
        node: StableNodeId,
        origin: crate::TextEditOrigin,
        edit: impl FnOnce(&TextInputState) -> Option<EditorEdit>,
    ) -> Result<bool, FrameworkError> {
        let state = self.editor_state(node)?;
        let Some(edited) = edit(&state) else {
            return Ok(false);
        };
        if edited.value == state.value {
            return Ok(false);
        }
        let EditorEdit { value, selection } = edited;
        self.commit_editor_value(node, value.into(), selection, Vec::new(), origin)
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
    pub(super) fn edit_editor_multi(
        &mut self,
        node: StableNodeId,
        origin: crate::TextEditOrigin,
        mut edit: impl FnMut(&str, TextSelection, bool) -> Option<CursorEdit>,
    ) -> Result<bool, FrameworkError> {
        let state = self.editor_state(node)?;
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
            return self.commit_editor_value(node, value.into(), selection, Vec::new(), origin);
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
        self.commit_editor_value(node, value.into(), primary, additional, origin)
    }

    /// Commit a value edit together with the rebuilt selection set. The set
    /// is normalized here so overlapping or touching cursors fuse in the
    /// same pass that emits the change event.
    pub(super) fn commit_editor_value(
        &mut self,
        node: StableNodeId,
        value: crate::TextValue,
        selection: TextSelection,
        additional: Vec<TextSelection>,
        origin: crate::TextEditOrigin,
    ) -> Result<bool, FrameworkError> {
        let linked = self.world.text_snippet_session(node).and_then(|session| {
            let old = self.editor_state(node).ok()?;
            session.linked_edit(&old.value, &value, selection)
        });
        let (value, selection, linked_session) = if let Some((value, selection, session)) = linked {
            (value.into(), selection, Some(session))
        } else {
            (value, selection, None)
        };
        let mut next = TextInputState {
            value,
            selection,
            additional_selections: additional,
        };
        next.normalize_selections();
        let changed = self.replace_editor_state(node, origin, next)?;
        if changed && let Some(session) = linked_session {
            let mut mutations = MutationQueue::new();
            mutations.set_text_input_snippet(node, Some(session));
            self.world.commit(mutations)?;
        }
        Ok(changed)
    }
}

#[derive(Default)]
pub(super) struct TextEditSession {
    /// Live text drag-selection: pointer id, node, and the anchor offset.
    pub(super) text_pointer_drag: Option<(u64, StableNodeId, usize)>,
    /// Document-level `user-select` drag: pointer, document, node, anchor.
    pub(super) document_select_drag: Option<(u64, DocumentId, StableNodeId, usize)>,
    /// Live minimap navigation drag: pointer id, node, and editor kind.
    pub(super) text_minimap_drag: Option<(u64, StableNodeId, TextEditorKind)>,
    /// 拖拽移动选中文本的状态机（多行编辑器、单选区、非 IME；按下落在
    /// 主选区内部时进入，超过阈值激活，释放执行移动/复制或回落为点击）。
    pub(super) text_selection_drag: Option<crate::framework::text_edit::TextSelectionDrag>,
    /// Last press inside a text editor for double/triple click counting.
    pub(super) text_pointer_click: Option<super::TextPointerClick>,
    /// The editor a press is focusing, for that press only: it hits the
    /// text as drawn before the focus moved.
    pub(super) unfocused_press: Option<StableNodeId>,
    /// Horizontal goal column retained across chained vertical moves.
    pub(super) caret_goal_x: Option<(StableNodeId, f32)>,
    /// Expand/shrink selection record: prior selection sets per node, valid
    /// only while the editor's value is unchanged (any edit clears it).
    pub(super) selection_expansions: Option<(
        StableNodeId,
        Vec<(crate::TextSelection, Vec<crate::TextSelection>)>,
        String,
    )>,
}
