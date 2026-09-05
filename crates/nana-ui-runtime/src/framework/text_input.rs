//! AppContext text input operations.

use super::*;

impl AppContext {
    /// Native composition owns its pending text and committed insertion range.
    /// Ordinary key edits must wait for the IME commit/cancel event.
    pub(super) fn has_focused_ime_composition(&self, document: DocumentId) -> bool {
        self.world
            .focused_text_input(document)
            .and_then(|(target, _)| self.world.ime(target))
            .is_some_and(|composition| !composition.text.is_empty())
    }

    pub fn set_ime_preedit(
        &mut self,
        document: DocumentId,
        text: String,
        selection: Option<(usize, usize)>,
    ) -> Result<bool, FrameworkError> {
        let Some((target, _)) = self.world.focused_text_input(document) else {
            return Ok(false);
        };
        if !self
            .world
            .accessibility(target)
            .is_some_and(|state| state.editable)
        {
            return Ok(false);
        }
        let composition = crate::ImeComposition { text, selection };
        if self.world.ime(target) == Some(&composition) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_ime(target, Some(composition));
        self.world.commit(mutations)?;
        Ok(true)
    }

    pub fn clear_ime(&mut self, document: DocumentId) -> Result<bool, FrameworkError> {
        let Some((target, _)) = self.world.focused_text_input(document) else {
            return Ok(false);
        };
        if self.world.ime(target).is_none() {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_ime(target, None);
        self.world.commit(mutations)?;
        Ok(true)
    }

    pub fn commit_ime(&mut self, document: DocumentId, text: &str) -> Result<bool, FrameworkError> {
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.commit_editable_ime(entity, text);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.commit_editable_ime(entity, text);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.commit_editable_ime(entity, text);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.commit_editable_ime(entity, text);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.commit_editable_ime(entity, text);
        }
        self.commit_world_text_input_ime(document, text)
    }

    /// Delete UTF-8 bytes surrounding the focused editor's selection.
    ///
    /// Leaves IME preedit in place. Returns `Ok(false)` when no focused
    /// editable field can apply the requested span.
    pub fn delete_ime_surrounding(
        &mut self,
        document: DocumentId,
        before_bytes: usize,
        after_bytes: usize,
    ) -> Result<bool, FrameworkError> {
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.delete_editable_surrounding(entity, before_bytes, after_bytes);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.delete_editable_surrounding(entity, before_bytes, after_bytes);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.delete_editable_surrounding(entity, before_bytes, after_bytes);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.delete_editable_surrounding(entity, before_bytes, after_bytes);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.delete_editable_surrounding(entity, before_bytes, after_bytes);
        }
        self.delete_world_text_input_surrounding(document, before_bytes, after_bytes)
    }

    pub(super) fn commit_world_text_input_ime(
        &mut self,
        document: DocumentId,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        let Some((target, state)) = self.world.focused_text_input(document) else {
            return Ok(false);
        };
        let mut next = state.clone();
        if !self
            .world
            .accessibility(target)
            .is_some_and(|state| state.editable)
        {
            return Ok(false);
        }
        if !next.replace_selection(text) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_ime(target, None);
        mutations.set_text_input(target, Some(next));
        self.world.commit(mutations)?;
        Ok(true)
    }

    pub(super) fn delete_world_text_input_surrounding(
        &mut self,
        document: DocumentId,
        before_bytes: usize,
        after_bytes: usize,
    ) -> Result<bool, FrameworkError> {
        let Some((target, state)) = self.world.focused_text_input(document) else {
            return Ok(false);
        };
        if !self
            .world
            .accessibility(target)
            .is_some_and(|state| state.editable)
        {
            return Ok(false);
        }
        let mut next = state.clone();
        if !next.delete_surrounding(before_bytes, after_bytes) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_text_input(target, Some(next));
        self.world.commit(mutations)?;
        Ok(true)
    }

    pub(super) fn delete_editable_surrounding<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        before_bytes: usize,
        after_bytes: usize,
    ) -> Result<bool, FrameworkError> {
        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            if !editable.delete_surrounding(before_bytes, after_bytes) {
                return false;
            }
            cx.emit(editable.change());
            true
        })
    }

    pub fn replace_focused_text(
        &mut self,
        document: DocumentId,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        if self.has_focused_ime_composition(document) {
            return Ok(false);
        }
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.replace_editable_selection(entity, text);
        }
        if let Some(entity) = self.focused_editor::<NumberInput>(document) {
            return self.replace_editable_selection(entity, text);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.replace_editable_selection(entity, text);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.replace_editable_selection(entity, text);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.replace_editable_selection(entity, text);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.replace_editable_selection(entity, text);
        }
        Ok(false)
    }

    pub fn delete_focused_text_backward(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        if self.has_focused_ime_composition(document) {
            return Ok(false);
        }
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.delete_editable_backward(entity);
        }
        if let Some(entity) = self.focused_editor::<NumberInput>(document) {
            return self.delete_editable_backward(entity);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.delete_editable_backward(entity);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.delete_editable_backward(entity);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.delete_editable_backward(entity);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.delete_editable_backward(entity);
        }
        Ok(false)
    }

    /// Text currently selected in the focused editor, or in the focused
    /// rich-text block.
    ///
    /// An empty selection reports `None` so a host copy request never replaces
    /// the pasteboard with an empty string. The Runtime does not touch the OS
    /// pasteboard; the host writes what this returns.
    pub fn focused_selected_text(&self, document: DocumentId) -> Option<String> {
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.editable_selected_text(entity);
        }
        if let Some(entity) = self.focused_editor::<NumberInput>(document) {
            return self.editable_selected_text(entity);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.editable_selected_text(entity);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.editable_selected_text(entity);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.editable_selected_text(entity);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.editable_selected_text(entity);
        }
        #[cfg(feature = "rich-text")]
        let focused = self.world.focused(document)?;
        #[cfg(feature = "rich-text")]
        if let Some(entity) = self.view_entity::<crate::SelectableRichText>(focused) {
            return self
                .read(entity, |text| text.copy_snapshot())
                .ok()
                .flatten()
                .map(|snapshot| snapshot.text);
        }
        #[cfg(feature = "rich-text")]
        if let Some(entity) = self.view_entity::<crate::NativeMarkdown>(focused) {
            return self
                .read(entity, |markdown| markdown.copy_snapshot())
                .ok()
                .flatten()
                .map(|snapshot| snapshot.text);
        }
        None
    }

    /// Remove the focused editor's selection and report what it held.
    ///
    /// Reports `None` without editing when the selection is empty or the field
    /// rejects input, so a cut on a read-only field leaves both the value and
    /// the pasteboard alone.
    pub fn cut_focused_text(
        &mut self,
        document: DocumentId,
    ) -> Result<Option<String>, FrameworkError> {
        let Some(text) = self.focused_selected_text(document) else {
            return Ok(None);
        };
        if !self.replace_focused_text(document, "")? {
            return Ok(None);
        }
        Ok(Some(text))
    }

    /// Select the whole value of the focused editor.
    ///
    /// Read-only and disabled fields still select, so their text can be copied.
    pub fn select_all_focused_text(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        if let Some(entity) = self.focused_editor::<TextInput>(document) {
            return self.select_all_editable(entity);
        }
        if let Some(entity) = self.focused_editor::<NumberInput>(document) {
            return self.select_all_editable(entity);
        }
        if let Some(entity) = self.focused_editor::<TextArea>(document) {
            return self.select_all_editable(entity);
        }
        if let Some(entity) = self.focused_editor::<SearchDropdown>(document) {
            return self.select_all_editable(entity);
        }
        if let Some(entity) = self.focused_editor::<CommandPalette>(document) {
            return self.select_all_editable(entity);
        }
        if let Some(entity) = self.focused_editor::<ContextMenu>(document) {
            return self.select_all_editable(entity);
        }
        Ok(false)
    }

    pub(super) fn editable_selected_text<C: EditableText>(
        &self,
        entity: Entity<C>,
    ) -> Option<String> {
        self.read(entity, |editable| {
            let state = editable.state();
            // Zed copy semantics: every selection's text, in document order,
            // joined with newlines. An empty set (bare carets only) reports
            // None so a copy never blanks the pasteboard.
            let mut parts: Vec<&str> = Vec::new();
            for selection in state.selections().iter() {
                if !selection.is_valid_for(&state.value) {
                    continue;
                }
                let range = selection.ordered();
                if !range.is_empty() {
                    parts.push(&state.value[range]);
                }
            }
            (!parts.is_empty()).then(|| parts.join("\n"))
        })
        .ok()
        .flatten()
    }

    pub(super) fn select_all_editable<C: EditableText>(
        &mut self,
        entity: Entity<C>,
    ) -> Result<bool, FrameworkError> {
        self.update_component(entity, |editable, cx| {
            let selection = TextSelection {
                anchor: 0,
                focus: editable.state().value.len(),
            };
            if editable.state().selection == selection
                && !editable.state().has_additional_selections()
            {
                return false;
            }
            let state = editable.state_mut();
            state.selection = selection;
            // Select-all is a wholesale replacement of the selection set.
            state.additional_selections.clear();
            cx.emit(TextChanged {
                value: editable.state().value.clone(),
                selection,
            });
            true
        })
    }

    pub(super) fn delete_editable_backward<C: EditableText>(
        &mut self,
        entity: Entity<C>,
    ) -> Result<bool, FrameworkError> {
        use unicode_segmentation::UnicodeSegmentation;

        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            {
                let state = editable.state_mut();
                if state.selection.anchor == state.selection.focus {
                    let caret = state.selection.focus;
                    let Some(previous) = state.value[..caret]
                        .grapheme_indices(true)
                        .next_back()
                        .map(|(index, _)| index)
                    else {
                        return false;
                    };
                    state.selection = TextSelection {
                        anchor: previous,
                        focus: caret,
                    };
                }
            }
            if !editable.replace_selection("") {
                return false;
            }
            cx.emit(editable.change());
            true
        })
    }

    pub(super) fn commit_editable_ime<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            cx.mutations().set_ime(entity.stable_id(), None);
            if !editable.commit_ime_text(text) {
                return false;
            }
            cx.emit(editable.change());
            true
        })
    }

    pub(super) fn set_editable_value<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        value: String,
    ) -> Result<bool, FrameworkError> {
        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            if !editable.set_value(value) {
                return false;
            }
            cx.emit(editable.change());
            true
        })
    }

    pub(super) fn set_editable_selection<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        selection: TextSelection,
    ) -> Result<bool, FrameworkError> {
        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            if editable.state().selection == selection
                || !selection.is_valid_for(&editable.state().value)
            {
                return false;
            }
            editable.state_mut().selection = selection;
            cx.emit(TextChanged {
                value: editable.state().value.clone(),
                selection,
            });
            true
        })
    }

    /// Replace the active UTF-8 selection and notify typed observers without
    /// requiring an application-wide message enum.
    pub fn replace_text_input_selection(
        &mut self,
        entity: Entity<TextInput>,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        self.replace_editable_selection(entity, text)
    }

    pub fn replace_text_area_selection(
        &mut self,
        entity: Entity<TextArea>,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        self.replace_editable_selection(entity, text)
    }

    pub(super) fn replace_editable_selection<C: EditableText>(
        &mut self,
        entity: Entity<C>,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        if !self.read(entity, EditableText::accepts_input)? {
            return Ok(false);
        }
        self.update_component(entity, |editable, cx| {
            let atoms =
                crate::text_editing::atoms_in(&editable.state().value, editable.text_atoms());
            if !atoms.is_empty() {
                let range = editable.state().selection.ordered();
                let expanded = crate::text_editing::expand_range_over_atoms(range.clone(), &atoms);
                if expanded != range {
                    editable.state_mut().selection = crate::TextSelection {
                        anchor: expanded.start,
                        focus: expanded.end,
                    };
                }
            }
            if !editable.replace_selection(text) {
                return false;
            }
            cx.emit(editable.change());
            true
        })
    }
}

#[cfg(test)]
mod composition_tests {
    use super::*;
    #[test]
    fn preedit_owns_committed_range_until_commit_for_single_and_multiline_editors() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let field = context
            .create_component(document, TextInput::new("ab"))
            .unwrap();
        let area = context
            .create_component(document, TextArea::new("ab"))
            .unwrap();
        for node in [field.stable_id(), area.stable_id()] {
            assert!(context.focus_node(document, node).unwrap());
            context
                .set_ime_preedit(document, "ni".into(), None)
                .unwrap();
            let before = context.world().text_input(node).unwrap().clone();
            let preedit = context.world().ime(node).unwrap().clone();
            assert!(
                !context
                    .move_focused_text_caret(document, crate::TextCaretIntent::Left, false, None)
                    .unwrap()
            );
            assert!(
                !context
                    .delete_focused_text(document, crate::TextDeleteKind::Backward)
                    .unwrap()
            );
            assert!(
                !context
                    .delete_focused_text(document, crate::TextDeleteKind::Forward)
                    .unwrap()
            );
            assert!(!context.delete_focused_text_backward(document).unwrap());
            assert!(!context.replace_focused_text(document, "raw-key").unwrap());
            assert_eq!(context.world().text_input(node), Some(&before));
            assert_eq!(context.world().ime(node), Some(&preedit));
            assert!(context.commit_ime(document, "你").unwrap());
            assert_eq!(context.world().text_input(node).unwrap().value, "ab你");
            assert!(context.world().ime(node).is_none());
            assert!(context.delete_focused_text_backward(document).unwrap());
            assert_eq!(context.world().text_input(node).unwrap().value, "ab");
        }
    }
    #[test]
    fn ime_surrounding_delete_and_empty_preedit_keep_their_explicit_edit_contracts() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let field = context
            .create_component(document, TextInput::new("ab"))
            .unwrap();
        context.focus_node(document, field.stable_id()).unwrap();
        context
            .set_ime_preedit(document, "ni".into(), None)
            .unwrap();
        assert!(context.delete_ime_surrounding(document, 1, 0).unwrap());
        assert_eq!(
            context.world().text_input(field.stable_id()).unwrap().value,
            "a"
        );
        assert_eq!(context.world().ime(field.stable_id()).unwrap().text, "ni");
        context
            .set_ime_preedit(document, String::new(), None)
            .unwrap();
        assert!(context.replace_focused_text(document, "c").unwrap());
        assert_eq!(
            context.world().text_input(field.stable_id()).unwrap().value,
            "ac"
        );
    }
}
