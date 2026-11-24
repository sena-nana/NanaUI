//! Text editor and IME mutation application.

use super::super::*;

impl UiWorld {
    pub(super) fn apply_editor_mutation(&mut self, mutation: &UiMutation) -> bool {
        match mutation {
            UiMutation::SetIme { id, composition } => {
                if let Some(editor) = self.nodes.editor_mut(*id) {
                    let before = editor.ime().map(|ime| ime.to_composition());
                    match composition {
                        Some(composition) if !composition.text.is_empty() => {
                            editor.empty_preedit = None;
                            editor.session.set_preedit(
                                &composition.text,
                                composition.selection.map(|(start, end)| start..end),
                            );
                        }
                        Some(composition) => {
                            editor.session.cancel_composition();
                            editor.empty_preedit = Some(composition.selection);
                        }
                        None => {
                            editor.session.cancel_composition();
                            editor.empty_preedit = None;
                        }
                    }
                    // One update per change of what the IME shows, the
                    // empty preedit included: the session counts only its
                    // compositions.
                    let mut work = editor.session.take_work();
                    work.composition_updates = 0;
                    let changed = editor.ime().map(|ime| ime.to_composition()) != before;
                    self.text.pending_edit_work.accumulate(work);
                    if changed {
                        self.text.pending_edit_work.composition_updates += 1;
                    }
                }
                self.nodes
                    .invalidate_text(*id, crate::text_node::TextDirty::EDIT_STATE);
                self.mark(
                    *id,
                    DirtyMask::TEXT | DirtyMask::FOCUS_IME | DirtyMask::RENDER,
                );
            }
            UiMutation::SetTextInput { id, state } => {
                let Some(state) = state else {
                    self.remove_ime(*id);
                    self.nodes.set_editor(*id, None);
                    self.nodes.set_text_content(*id, TextContent::default());
                    self.invalidate_text_content(*id);
                    self.nodes.set_text_fold_view(*id, None);
                    self.nodes.set_text_inlays(*id, None);
                    self.nodes.set_text_snippet_session(*id, None);
                    self.nodes.set_text_completion_view(*id, None);
                    self.nodes.set_text_hover_view(*id, None);
                    self.nodes.set_text_signature(*id, None);
                    self.mark(
                        *id,
                        DirtyMask::TEXT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::RENDER
                            | DirtyMask::ACCESSIBILITY,
                    );
                    return true;
                };
                // The component's own buffer: the session adopts it, so both
                // hold one copy of the text.
                let value = state.value.clone();
                // The text before, for the fold and snippet remaps: a shared
                // copy, so taking it costs nothing.
                let previous = self
                    .nodes
                    .editor(*id)
                    .map(|editor| editor.session.snapshot());
                match self.nodes.editor_mut(*id) {
                    // Mounting an editor puts its value in place; nobody
                    // edited it.
                    None => self.nodes.set_editor(
                        *id,
                        Some(crate::store::EditorRecord::new(
                            nana_text::EditSession::with_selections(
                                value,
                                state.selection,
                                state.additional_selections.iter().copied(),
                            ),
                        )),
                    ),
                    Some(editor) => {
                        let additional = state.additional_selections.clone();
                        let work = edit_keeping_preedit(editor, |session| {
                            session.assign(&value, Some((state.selection, additional)));
                        });
                        self.text.pending_edit_work.accumulate(work);
                    }
                }
                let next = self
                    .nodes
                    .editor(*id)
                    .map(|editor| editor.session.snapshot())
                    .unwrap_or_default();
                // The same text (a selection-only write, or a component's copy
                // of these bytes) leaves the content alone: only what the
                // edit state draws changed. A `SetText` since then wrote the
                // node something else, and this write puts the value back.
                if previous
                    .as_ref()
                    .is_some_and(|previous| previous.same_identity(&next))
                    && self.nodes.text_content(*id).value.same_identity(&next)
                {
                    self.nodes
                        .invalidate_text(*id, crate::text_node::TextDirty::EDIT_STATE);
                } else {
                    self.nodes.set_text_content(
                        *id,
                        TextContent {
                            value: next.clone(),
                        },
                    );
                    self.invalidate_text_content(*id);
                }
                // 值变化后重映射折叠态与 snippet 会话：受影响的折叠自动
                // 展开，跳位失效即结束会话。
                if let Some(previous) = previous
                    && (self.nodes.text_fold_view(*id).is_some()
                        || self.nodes.text_snippet_session(*id).is_some())
                    && previous != next
                {
                    self.reconcile_text_view_state(*id, &previous, &next);
                }
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetTextSelection { id, selection } => {
                let editor = self
                    .nodes
                    .editor_mut(*id)
                    .expect("entity must have runtime component");
                let work = edit_keeping_preedit(editor, |session| {
                    session.set_primary_selection(*selection);
                });
                self.text.pending_edit_work.accumulate(work);
                self.nodes
                    .invalidate_text(*id, crate::text_node::TextDirty::EDIT_STATE);
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::ReplaceTextSelection { id, text } => {
                let editor = self
                    .nodes
                    .editor_mut(*id)
                    .expect("entity must have runtime component");
                let work = edit_keeping_preedit(editor, |session| {
                    replace_every_selection(session, text);
                });
                let value = editor.session.snapshot();
                self.text.pending_edit_work.accumulate(work);
                self.nodes.set_text_content(*id, TextContent { value });
                self.invalidate_text_content(*id);
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetHighlightRequest { id, request } => {
                self.nodes.set_highlight(*id, request.as_deref().cloned());
                if request.is_none() {
                    self.nodes.set_text_presentation(*id, None);
                }
                self.mark(*id, DirtyMask::TEXT | DirtyMask::RENDER);
            }
            UiMutation::SetTextInputFoldCollapsed { id, folds } => {
                // 规范化：仅保留仍在宿主喂入区间内、且确实可折叠的条目。
                let offered = self
                    .text_input_visual_ref(*id)
                    .map(|v| Arc::clone(&v.folds))
                    .unwrap_or_else(|| Arc::from([]));
                let mut collapsed: Vec<crate::TextCodeFold> = folds
                    .iter()
                    .copied()
                    .filter(|fold| offered.contains(fold))
                    .collect();
                collapsed.compact_sort_by_key(|fold| (fold.start, fold.end));
                collapsed.dedup();
                let changed = self
                    .nodes
                    .text_fold_view(*id)
                    .map(|entry| entry.collapsed.as_slice() != collapsed.as_slice())
                    .unwrap_or(!collapsed.is_empty());
                if collapsed.is_empty() && offered.is_empty() {
                    self.nodes.set_text_fold_view(*id, None);
                } else {
                    self.nodes.set_text_fold_view(
                        *id,
                        Some(crate::store::TextFoldViewState { offered, collapsed }),
                    );
                }
                if changed {
                    self.mark(*id, DirtyMask::TEXT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputInlays { id, inlays } => {
                // 世界校验（钳除风格，同折叠先例）：锚点 char boundary 且
                // 不越界、文本非空且不含 '\n'，按 (offset, label) 排序去
                // 重。空集（含全部非法被钳除）移除条目，零分配待机。
                let value = self
                    .nodes
                    .text_input(*id)
                    .map(|state| state.value_shared())
                    .unwrap_or_default();
                let normalized = normalize_text_inlays(&value, inlays);
                let changed = self
                    .nodes
                    .text_inlays(*id)
                    .map(|fed| fed.as_ref() != normalized.as_slice())
                    .unwrap_or(!normalized.is_empty());
                if changed {
                    if normalized.is_empty() {
                        self.nodes.set_text_inlays(*id, None);
                    } else {
                        self.nodes.set_text_inlays(*id, Some(normalized.into()));
                    }
                    self.mark(*id, DirtyMask::TEXT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputSnippet { id, session } => {
                if self.nodes.text_snippet_session(*id) != session.as_deref() {
                    let previous_choices = self
                        .nodes
                        .text_snippet_session(*id)
                        .and_then(|s| s.choice_items());
                    let next_choices = session.as_deref().and_then(|s| s.choice_items());
                    if let Some(items) = next_choices {
                        self.nodes.set_text_completion_view(
                            *id,
                            Some(crate::store::TextCompletionViewState {
                                items,
                                selected: 0,
                                scroll: 0,
                                dismissed: false,
                            }),
                        );
                    } else if previous_choices.is_some() {
                        self.nodes.set_text_completion_view(*id, None);
                    }
                    self.nodes
                        .set_text_snippet_session(*id, session.as_deref().cloned());
                    self.mark(*id, DirtyMask::TEXT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputCompletions { id, items } => {
                if items.is_empty() {
                    // 空列表关闭弹层：条目移除，零分配待机。
                    if self.nodes.text_completion_view(*id).is_some() {
                        self.nodes.set_text_completion_view(*id, None);
                        self.mark(*id, DirtyMask::RENDER);
                    }
                    return true;
                }
                let next = match self.nodes.text_completion_view(*id) {
                    // 相同列表：无操作，键盘选中、滚动与 Esc 关闭态保持。
                    // 组件投影已过滤未变的喂入，这里的内容比较服务于直接
                    // 下发变更的调用方：内容相等即同一会话，不能降为指针
                    // 比较（换 Arc 重喂相同列表会被误判成新会话）。
                    Some(state) if state.items == *items => None,
                    // 不同列表：视为新会话（选中归零、重新打开）。
                    Some(_) | None => Some(crate::store::TextCompletionViewState {
                        items: Arc::clone(items),
                        selected: 0,
                        scroll: 0,
                        dismissed: false,
                    }),
                };
                if let Some(next) = next {
                    self.nodes.set_text_completion_view(*id, Some(next));
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputCompletionView {
                id,
                selected,
                scroll,
            } => {
                let changed = self
                    .nodes
                    .text_completion_view(*id)
                    .is_some_and(|state| state.selected != *selected || state.scroll != *scroll);
                if changed {
                    if let Some(state) = self.nodes.text_completion_view_mut(*id) {
                        state.selected = *selected;
                        state.scroll = *scroll;
                    }
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputCompletionDismissed { id } => {
                let changed = self
                    .nodes
                    .text_completion_view(*id)
                    .is_some_and(|state| !state.dismissed);
                if changed {
                    if let Some(state) = self.nodes.text_completion_view_mut(*id) {
                        state.dismissed = true;
                    }
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputCompletionReopened { id } => {
                let changed = self
                    .nodes
                    .text_completion_view(*id)
                    .is_some_and(|state| state.dismissed);
                if changed {
                    if let Some(state) = self.nodes.text_completion_view_mut(*id) {
                        state.dismissed = false;
                        state.selected = 0;
                        state.scroll = 0;
                    }
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputHover { id, hover } => {
                let changed = match (hover.as_deref(), self.nodes.text_hover_view(*id)) {
                    (None, None) => false,
                    (None, Some(_)) => true,
                    (Some(doc), Some(state)) => &state.doc != doc || state.scroll != 0,
                    (Some(_), None) => true,
                };
                if !changed {
                    return true;
                }
                match hover.as_deref() {
                    Some(doc) => {
                        self.nodes.set_text_hover_view(
                            *id,
                            Some(crate::store::TextHoverViewState {
                                doc: doc.clone(),
                                scroll: 0,
                                diagnostic: false,
                            }),
                        );
                    }
                    None => self.nodes.set_text_hover_view(*id, None),
                }
                self.mark(*id, DirtyMask::RENDER);
            }
            UiMutation::SetTextInputDiagnosticHover { id, hover } => {
                let current = self.nodes.text_hover_view(*id);
                let changed = match (hover.as_deref(), current) {
                    (None, Some(state)) => state.diagnostic,
                    (None, None) => false,
                    (Some(doc), Some(state)) => !state.diagnostic || &state.doc != doc,
                    (Some(_), None) => true,
                };
                if !changed {
                    return true;
                }
                match hover.as_deref() {
                    Some(doc) => {
                        let scroll = current
                            .filter(|state| state.diagnostic && state.doc == *doc)
                            .map(|state| state.scroll)
                            .unwrap_or(0);
                        self.nodes.set_text_hover_view(
                            *id,
                            Some(crate::store::TextHoverViewState {
                                doc: doc.clone(),
                                scroll,
                                diagnostic: true,
                            }),
                        );
                    }
                    None => {
                        if current.is_some_and(|state| state.diagnostic) {
                            self.nodes.set_text_hover_view(*id, None);
                        }
                    }
                }
                self.mark(*id, DirtyMask::RENDER);
            }
            UiMutation::SetTextInputSignature { id, help } => {
                let changed = match (help.as_deref(), self.nodes.text_signature(*id)) {
                    (None, None) => false,
                    (None, Some(_)) => true,
                    (Some(next), Some(fed)) => next != fed,
                    (Some(_), None) => true,
                };
                if changed {
                    self.nodes.set_text_signature(*id, help.as_deref().cloned());
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputHoverScroll { id, scroll } => {
                let changed = self
                    .nodes
                    .text_hover_view(*id)
                    .is_some_and(|state| state.scroll != *scroll);
                if changed {
                    if let Some(state) = self.nodes.text_hover_view_mut(*id) {
                        state.scroll = *scroll;
                    }
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            _ => return false,
        }
        true
    }
}

fn replace_every_selection(session: &mut nana_text::EditSession, text: &str) {
    let primary = session.selection();
    let selections = session.selections().into_owned();
    let mut edits = Vec::with_capacity(selections.len());
    let mut carets = Vec::with_capacity(selections.len());
    let mut primary_caret = None;
    let mut shift = 0isize;
    for selection in selections {
        let range = selection.range();
        let start = (range.start as isize + shift) as usize;
        let caret = crate::TextSelection::caret(start + text.len());
        if selection == primary && primary_caret.is_none() {
            primary_caret = Some(caret);
        } else {
            carets.push(caret);
        }
        shift += text.len() as isize - range.len() as isize;
        edits.push((range, text));
    }
    let primary = primary_caret.unwrap_or_else(|| carets.remove(0));
    session.splice(&edits, primary, carets);
}

/// Applies `edit` to a composing session as Runtime's contract has it: the
/// preedit stands over the editor's current primary selection, so a write of
/// the text or selection moves it along instead of being refused (the
/// session refuses selection changes while composing) or cancelling it. The
/// preedit is lifted, the write applied, and the same preedit put back over
/// the resulting selection. Its lifting and restoring are not composition
/// updates: what the IME shows did not change.
///
/// Returns the session's work for the whole write.
fn edit_keeping_preedit(
    editor: &mut crate::store::EditorRecord,
    edit: impl FnOnce(&mut nana_text::EditSession),
) -> nana_text::TextWorkCounters {
    let preedit = editor
        .session
        .composition()
        .map(|composition| (composition.text.clone(), composition.selection.clone()));
    if preedit.is_some() {
        editor.session.cancel_composition();
    }
    edit(&mut editor.session);
    let restored = preedit.is_some();
    if let Some((text, selection)) = preedit {
        editor.session.set_preedit(&text, selection);
    }
    let mut work = editor.session.take_work();
    if restored {
        work.composition_updates = 0;
    }
    work
}
