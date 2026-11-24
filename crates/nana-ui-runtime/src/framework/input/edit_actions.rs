//! Shared editor commands for menus and host integrations. Clipboard actions
//! use the same handlers and HostServices as keyboard editing.
use super::*;
use crate::Entity;
use nana_ui_input::InputModifiers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEditAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl TextEditAction {
    pub const ALL: [Self; 6] = [
        Self::Undo,
        Self::Redo,
        Self::Cut,
        Self::Copy,
        Self::Paste,
        Self::SelectAll,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Undo => "z",
            Self::Redo => "y",
            Self::Cut => "x",
            Self::Copy => "c",
            Self::Paste => "v",
            Self::SelectAll => "a",
        }
    }
}

type RichDragScroll = (DocumentId, StableNodeId, u64, f32, f32, Duration);

impl AppContext {
    pub(crate) fn rich_drag_scroll(&self) -> Option<RichDragScroll> {
        self.input.rich_drag_scroll
    }
    pub(crate) fn set_rich_drag_scroll(&mut self, state: Option<RichDragScroll>) {
        self.input.rich_drag_scroll = state;
    }
    pub fn editor_composing(&self, document: DocumentId) -> bool {
        self.focused_text_editor_composing(document)
            || self.focused_rich_editor(document).is_some_and(|editor| {
                self.read(editor, |e| e.preedit().is_some())
                    .unwrap_or(false)
            })
    }

    pub fn text_edit_action_enabled(&self, node: StableNodeId, action: TextEditAction) -> bool {
        if let Some(editor) = self.view_entity::<crate::RichTextEditor>(node) {
            return self
                .read(editor, |e| {
                    if e.disabled || e.preedit().is_some() {
                        return false;
                    }
                    let selection = e.selection();
                    match action {
                        TextEditAction::Undo => e.can_undo(),
                        TextEditAction::Redo => e.can_redo(),
                        TextEditAction::Copy => selection.0 != selection.1,
                        TextEditAction::Cut => !e.read_only && selection.0 != selection.1,
                        TextEditAction::Paste => !e.read_only,
                        TextEditAction::SelectAll => !e.value.text().is_empty(),
                    }
                })
                .unwrap_or(false);
        }
        let Some(state) = self.world().text_input(node) else {
            return false;
        };
        let Some(accessibility) = self.world().accessibility(node) else {
            return false;
        };
        if accessibility.disabled || self.world().ime(node).is_some() {
            return false;
        }
        match action {
            TextEditAction::Undo => accessibility.editable && self.can_undo_text(node),
            TextEditAction::Redo => accessibility.editable && self.can_redo_text(node),
            TextEditAction::Copy => !state.selection.range().is_empty(),
            TextEditAction::Cut => accessibility.editable && !state.selection.range().is_empty(),
            TextEditAction::Paste => accessibility.editable,
            TextEditAction::SelectAll => !state.value.is_empty(),
        }
    }

    pub fn edit_text_action(
        &mut self,
        document: DocumentId,
        node: StableNodeId,
        action: TextEditAction,
        services: &mut dyn HostServices,
    ) -> Result<bool, FrameworkError> {
        if self.world().document_of(node) != Some(document)
            || !self.text_edit_action_enabled(node, action)
        {
            return Ok(false);
        }
        self.focus_node(document, node)?;
        if let Some(rich_editor) = self.capabilities.rich_editor
            && let Some(handled) = (rich_editor.key)(
                self,
                document,
                action.key(),
                None,
                InputModifiers {
                    control: true,
                    ..InputModifiers::default()
                },
                services,
            )?
        {
            return Ok(handled);
        }
        match action {
            TextEditAction::Undo => self.undo_focused_text(document),
            TextEditAction::Redo => self.redo_focused_text(document),
            _ => self.dispatch_clipboard_shortcut(document, action.key(), services),
        }
    }

    /// An application toolbar edit is one undo step, unlike rebinding a field
    /// to a different document. It still emits the normal TextChanged event.
    pub fn edit_text_area_value(
        &mut self,
        area: Entity<crate::TextArea>,
        value: String,
        selection: crate::TextSelection,
    ) -> Result<bool, FrameworkError> {
        self.commit_editor_edit(area, crate::TextEditOrigin::Structural, |area, _| {
            if area.read_only || area.disabled {
                return false;
            }
            if area.state.value.as_str() == value && area.state.selection == selection {
                return false;
            }
            area.state.replace_value(value);
            area.state.selection = selection;
            area.state.normalize_selections();
            true
        })
    }
}
