use super::{
    AppContext, DocumentId, Entity, FrameworkError, StableNodeId, TEXT_SELECTION_DRAG_THRESHOLD,
    TextArea, TextDeleteKind, TextEditorKind, TextInputState, TextSelection, TextSelectionDrag,
};
use crate::text_editing::{
    CursorEdit, TextCaretIntent, TextLineDirection, TextReplacement, atoms_in, auto_indent_newline,
    auto_pair_edit, caret_focus, caret_offset_at_point, delete_backward_atoms,
    delete_forward_atoms, delete_lines, delete_to_line_end_atoms, delete_to_line_start_atoms,
    delete_word_backward_atoms, delete_word_forward_atoms, duplicate_lines,
    expand_range_over_atoms, indent_selection, join_lines, logical_line_range,
    matching_bracket_pair, move_lines, moved_selection, outdent_selection, page_caret_focus,
    page_caret_focus_logical, snap_moved_caret, snap_pointer_caret, sort_lines,
    toggle_line_comment, transform_selection_case, vertical_caret_focus,
    vertical_caret_focus_logical, word_range_at,
};
use crate::{MutationQueue, ScrollOffset, TextAtomSpan, TextContent, TextShapeConstraints};
use std::sync::Arc;

/// A [`crate::TextShaper`] probe bound to one editor node's shaped layout.
pub(super) struct EditorGeometry<'a> {
    pub(super) shaper: &'a mut dyn crate::TextShaper,
    pub(super) node: StableNodeId,
    pub(super) text: TextContent,
    pub(super) style: crate::ComputedStyle,
    pub(super) constraints: TextShapeConstraints,
}

impl EditorGeometry<'_> {
    pub(super) fn probe(&mut self) -> impl FnMut(usize) -> (f32, f32, f32) + '_ {
        move |offset: usize| {
            self.shaper
                .text_position(self.node, &self.text, offset, &self.style, self.constraints)
        }
    }

    /// A vertical or page move answered by the backend's own point hit test:
    /// the caret's position, the last line's, and one hit — rather than a
    /// search over position probes. `None` when the backend cannot hit test,
    /// or the intent is not vertical.
    fn vertical_by_point(
        &mut self,
        value: &str,
        selection: TextSelection,
        intent: TextCaretIntent,
        extend: bool,
        goal_x: Option<f32>,
        page_height: f32,
    ) -> Option<(TextSelection, f32)> {
        let page = match intent {
            TextCaretIntent::Up | TextCaretIntent::Down => None,
            TextCaretIntent::PageUp | TextCaretIntent::PageDown => Some(page_height),
            _ => return None,
        };
        // Asked first: a backend that cannot hit test gets its probes spent on
        // the search fallback, not twice.
        if !self.supports_point_hits() {
            return None;
        }
        let focus = clamp_focus(value, selection.focus);
        let (x, y, height) = self.shaper.text_caret_position(
            self.node,
            &self.text,
            focus,
            selection.affinity,
            &self.style,
            self.constraints,
        );
        let height = height.max(1.0);
        let goal = goal_x.unwrap_or(x);
        let upwards = matches!(intent, TextCaretIntent::Up | TextCaretIntent::PageUp);
        let target_y = if upwards {
            if y <= f32::EPSILON {
                return Some((moved_selection(selection, 0, extend), goal));
            }
            // Just inside the line above's bottom, whatever its height.
            match page {
                Some(page) => (y - page.max(height)).max(0.0) + 0.5,
                None => y - 0.5,
            }
        } else {
            let last_y = self
                .shaper
                .text_position(
                    self.node,
                    &self.text,
                    value.len(),
                    &self.style,
                    self.constraints,
                )
                .1;
            if y + f32::EPSILON >= last_y {
                return Some((moved_selection(selection, value.len(), extend), goal));
            }
            // Just inside the line below's top.
            match page {
                Some(page) => (y + page.max(height)).min(last_y) + 0.5,
                None => y + height + 0.5,
            }
        };
        let hit = self.shaper.text_hit_at_point(
            self.node,
            &self.text,
            goal,
            target_y,
            &self.style,
            self.constraints,
        )?;
        Some((
            moved_selection(selection, hit.offset, extend).with_affinity(hit.affinity),
            goal,
        ))
    }

    /// Where a non-extending Left or Right lands on a non-empty selection:
    /// its edge on that side of the screen, by `nana-text`'s rule
    /// ([`nana_text::editable::collapse_edge`]), read off the carets the
    /// backend draws at the selection's two ends.
    fn collapse_edge(&mut self, selection: TextSelection, rightwards: bool) -> usize {
        let range = selection.ordered();
        let (start_x, start_y, _) = self.shaper.text_caret_position(
            self.node,
            &self.text,
            range.start,
            crate::TextAffinity::Downstream,
            &self.style,
            self.constraints,
        );
        let (end_x, end_y, _) = self.shaper.text_caret_position(
            self.node,
            &self.text,
            range.end,
            crate::TextAffinity::Upstream,
            &self.style,
            self.constraints,
        );
        let rtl = matches!(self.style.direction, nana_ui_core::DirSpec::Rtl);
        // Across columns x says nothing about the reading order, and one
        // column's y is not "the same line": vertical text goes by order.
        let horizontal = matches!(
            self.style.writing_mode,
            nana_ui_core::WritingModeSpec::HorizontalTb
        );
        nana_text::editable::collapse_edge(
            range,
            rightwards,
            horizontal.then_some((start_x, start_y)),
            horizontal.then_some((end_x, end_y)),
            rtl,
        )
    }

    /// One visual step left or right, from the backend's own layout: the
    /// caret an arrow key lands on. `None` when the backend has no geometry,
    /// or there is nothing in that direction.
    fn visual_step(&mut self, selection: TextSelection, rightwards: bool) -> Option<TextSelection> {
        let hit = self.shaper.text_caret_visual_step(
            self.node,
            &self.text,
            selection.focus,
            selection.affinity,
            rightwards,
            &self.style,
            self.constraints,
        )?;
        Some(TextSelection {
            anchor: selection.anchor,
            focus: hit.offset,
            affinity: hit.affinity,
        })
    }

    fn supports_point_hits(&mut self) -> bool {
        self.shaper
            .text_hit_at_point(
                self.node,
                &self.text,
                0.0,
                0.0,
                &self.style,
                self.constraints,
            )
            .is_some()
    }
}

/// Whether an editor's drawn text is its value, so a geometry probe of the
/// value asks about the layout its caret is drawn in.
///
/// A secure field draws bullets and an empty one draws its placeholder.
/// Probing with the value there would answer from a layout that is not on
/// screen, and would push the value -- a password among it -- into the host's
/// retained geometry and its shape cache. Those cases step by grapheme
/// instead, which is what "left" means in a column of identical bullets
/// anyway.
///
/// A live preedit also draws text the value does not contain, but no caret
/// move can run then: [`AppContext::focused_text_editor`] refuses while
/// composing. An ENDED composition leaves an empty preedit in place, and the
/// display text is the value again then, so it must not disable this.
pub(super) fn editor_draws_its_value(
    world: &crate::UiWorld,
    node: StableNodeId,
    value: &str,
) -> bool {
    !value.is_empty() && !world.text_input_is_secure(node)
}

/// Whether this intent is the horizontal arrow, and which way it points on
/// screen. `None` for every other intent: only Left/Right follow visual
/// order -- word and line intents are logical by definition, and the vertical
/// ones have their own geometry path.
fn horizontal_rightwards(intent: TextCaretIntent) -> Option<bool> {
    match intent {
        TextCaretIntent::Left => Some(false),
        TextCaretIntent::Right => Some(true),
        _ => None,
    }
}

/// Whether this move collapses a non-empty selection onto its edge: Left or
/// Right without extending. The landing is an edge of what was selected,
/// not a step from anywhere, so the atom rules for a caret leaving a chip do
/// not apply to it.
fn collapses_selection(selection: TextSelection, intent: TextCaretIntent, extend: bool) -> bool {
    !extend && selection.anchor != selection.focus && horizontal_rightwards(intent).is_some()
}

/// A caret at `edge` of `selection`. On the focus it keeps the side the focus
/// was drawn on (the end of a soft-wrapped line stays on that line); on the
/// anchor, the side of the selection it bounds -- the end of a selection
/// that stops at a soft wrap is on the line it covers, as
/// [`EditorGeometry::collapse_edge`] probed it. `nana-text` lands the same.
fn caret_at_edge(selection: TextSelection, edge: usize) -> TextSelection {
    let caret = TextSelection::caret(edge);
    if edge == selection.focus {
        caret.with_affinity(selection.affinity)
    } else if edge == selection.ordered().end {
        caret.with_affinity(crate::TextAffinity::Upstream)
    } else {
        caret
    }
}

/// Nearest char boundary at or below `offset` (clamped to the value length).
pub(super) fn clamp_focus(value: &str, offset: usize) -> usize {
    crate::text_editing::clamp_boundary(value, offset)
}

fn snap_selection_over_atoms(
    previous: TextSelection,
    next: TextSelection,
    extend: bool,
    atoms: &[TextAtomSpan],
) -> TextSelection {
    if atoms.is_empty() {
        return next;
    }
    let snapped = snap_moved_caret(previous.focus, next.focus, atoms);
    if extend {
        let selection = moved_selection(previous, snapped, true);
        let range = expand_range_over_atoms(selection.ordered(), atoms);
        if selection.focus >= selection.anchor {
            TextSelection::new(range.start, range.end)
        } else {
            TextSelection::new(range.end, range.start)
        }
    } else {
        TextSelection::caret(snapped)
    }
}

impl AppContext {
    /// An arrow key as the focused editor's line space names it (#59).
    ///
    /// Caret intents are in line space, where Left/Right step along a line and
    /// Up/Down cross to the neighbouring one. A host names the key it saw, so
    /// it asks this **before** it folds modifiers into intents: in a vertical
    /// writing mode the line runs down the page and the physical keys trade
    /// places — Up/Down walk the column, Left/Right cross columns, towards
    /// the next one on the left in `vertical-rl` and on the right in
    /// `vertical-lr`. Translating the key rather than the finished intent is
    /// what carries every modifier along: Cmd+Up becomes the column's start
    /// and Cmd+Left in `vertical-rl` the document's end, Alt+Up/Down move by
    /// word, and the line-block gestures move to the keys that cross columns.
    ///
    /// Anything that is not an arrow key, and every key of a horizontal
    /// editor, comes back unchanged.
    pub fn focused_text_line_space_key<'k>(&self, document: DocumentId, key: &'k str) -> &'k str {
        let Some(writing) = self
            .focused_text_editor(document)
            .and_then(|focused| self.world.computed_style(focused.node))
            .map(crate::ComputedStyle::writing_context)
            .filter(|writing| writing.is_vertical())
        else {
            return key;
        };
        let next_column_is_left = writing.block_reversed();
        match key {
            "ArrowUp" => "ArrowLeft",
            "ArrowDown" => "ArrowRight",
            "ArrowLeft" if next_column_is_left => "ArrowDown",
            "ArrowLeft" => "ArrowUp",
            "ArrowRight" if next_column_is_left => "ArrowUp",
            "ArrowRight" => "ArrowDown",
            other => other,
        }
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
        geometry: Option<&mut EditorGeometry<'_>>,
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
            if let Some(geometry) = geometry {
                if let Some((selection, goal)) =
                    geometry.vertical_by_point(value, selection, intent, extend, None, page_height)
                {
                    return Some((selection, Some(goal)));
                }
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
        // Left/Right over a non-empty selection collapse it onto its edge
        // on that side instead of stepping from the focus: the visual edge
        // with geometry, the logical one without.
        if !extend
            && selection.anchor != selection.focus
            && let Some(rightwards) = horizontal_rightwards(intent)
        {
            let edge = match geometry {
                Some(geometry) => geometry.collapse_edge(selection, rightwards),
                None => nana_text::editable::collapse_edge(
                    selection.ordered(),
                    rightwards,
                    None,
                    None,
                    false,
                ),
            };
            return Some((caret_at_edge(selection, edge), None));
        }
        if let Some(geometry) = geometry
            && let Some(rightwards) = horizontal_rightwards(intent)
            && let Some(stepped) = geometry.visual_step(selection, rightwards)
        {
            let moved = if extend {
                stepped
            } else {
                TextSelection::caret(stepped.focus).with_affinity(stepped.affinity)
            };
            return Some((moved, None));
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
        if !focused.accepts_selection {
            return Ok(false);
        }
        let state = self.editor_state(focused.node)?;
        // 折叠视图：无折叠态区间时为 None，全部按原始值解析（零成本）。
        let fold_view = self.world.text_display_view(focused.node);
        let probe_value: &str = fold_view.as_ref().map_or(&state.value, |view| &view.value);
        let probe_text = self.editor_probe_text(focused.node, fold_view.as_ref());
        let to_display = |selection: TextSelection| -> TextSelection {
            match &fold_view {
                Some(view) => TextSelection {
                    anchor: view.display_of(selection.anchor),
                    focus: view.display_of(selection.focus),
                    affinity: selection.affinity,
                },
                None => selection,
            }
        };
        let to_value = |selection: TextSelection| -> TextSelection {
            match &fold_view {
                Some(view) => TextSelection {
                    anchor: view.value_of(selection.anchor),
                    focus: view.value_of(selection.focus),
                    affinity: selection.affinity,
                },
                None => selection,
            }
        };
        // 水平移动的显示目标被覆盖区间（折叠摘要 / inlay 插入文本）钳回
        // 起点值偏移时（区间内部无 caret 边界，逐字符右移会变成空操作），
        // 前进到区间末端再重映射：一次按键跨过整个覆盖区间，值偏移步进
        // 为一。折叠摘要上的同源既有卡死一并修复；点击命中与垂直移动
        // 保持钳制语义（to_value）。
        let stepping = matches!(intent, TextCaretIntent::Right | TextCaretIntent::WordRight);
        let to_value_moved = |previous_focus: usize, moved: TextSelection| -> TextSelection {
            match &fold_view {
                Some(view) => {
                    let crossed = view.crossed_cover(moved.focus, stepping);
                    let word_into_label = matches!(intent, TextCaretIntent::WordRight)
                        && crossed.is_some_and(|span| span.fold().is_none());
                    let focus = match crossed {
                        Some(span)
                            if view.value_of(moved.focus) == previous_focus || word_into_label =>
                        {
                            view.value_of_forward(span, || {
                                crate::text_editing::caret_focus(
                                    &state.value,
                                    TextSelection::caret(previous_focus),
                                    intent,
                                )
                                .unwrap_or(previous_focus)
                            })
                        }
                        _ => view.value_of(moved.focus),
                    };
                    // A caret stays a caret: its anchor goes where its focus
                    // went, not back across the fold it just crossed (that
                    // left an invisible selection over the hidden lines).
                    let anchor = if moved.anchor == moved.focus {
                        focus
                    } else {
                        view.value_of(moved.anchor)
                    };
                    TextSelection {
                        anchor,
                        focus,
                        affinity: moved.affinity,
                    }
                }
                None => moved,
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
            self.text_edit.caret_goal_x = None;
            let page_height = self.world.text_input_page_extent(focused.node);
            let geometric = (vertical && focused.multiline)
                || (horizontal_rightwards(intent).is_some()
                    && editor_draws_its_value(&self.world, focused.node, probe_value));
            let shape_context = if geometric && shaper.is_some() {
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
                            value: probe_text.clone(),
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
                        let mapped = if vertical {
                            to_value(moved)
                        } else {
                            to_value_moved(selection.focus, moved)
                        };
                        next_selections.push(mapped);
                    }
                    None => next_selections.push(selection),
                }
            }
            if !moved_any {
                return Ok(false);
            }
            if let Some(goal) = next_goal {
                self.text_edit.caret_goal_x = Some((focused.node, goal));
            }
            let atoms = atoms_in(
                &state.value,
                &self.editor_text_atoms(focused.node, focused.kind),
            );
            if !atoms.is_empty() {
                for (index, next) in next_selections.iter_mut().enumerate() {
                    let previous = if collapses_selection(selections[index], intent, extend) {
                        *next
                    } else {
                        selections[index]
                    };
                    *next = snap_selection_over_atoms(previous, *next, extend, &atoms);
                }
            }
            let primary = next_selections[primary_index];
            let additional = next_selections
                .into_iter()
                .enumerate()
                .filter(|(index, _)| *index != primary_index)
                .map(|(_, selection)| selection)
                .collect();
            return self.write_editor_selections(focused.node, primary, additional);
        }
        let selection = to_display(state.selection);
        let selection = if vertical && focused.multiline {
            let goal = self
                .text_edit
                .caret_goal_x
                .take()
                .filter(|(id, _)| *id == focused.node)
                .map(|(_, x)| x);
            if let (Some(shaper), Some((style, constraints))) =
                (shaper, self.world.text_input_shape_context(focused.node))
            {
                let mut geometry = EditorGeometry {
                    shaper,
                    node: focused.node,
                    text: TextContent {
                        value: probe_text.clone(),
                    },
                    style,
                    constraints,
                };
                let page_height = self.world.text_input_page_extent(focused.node);
                let by_point = geometry.vertical_by_point(
                    probe_value,
                    selection,
                    intent,
                    extend,
                    goal,
                    page_height,
                );
                let moved = if by_point.is_some() {
                    by_point
                } else if matches!(intent, TextCaretIntent::Up | TextCaretIntent::Down) {
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
                    let page_height = self.world.text_input_page_extent(focused.node);
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
                        self.text_edit.caret_goal_x = Some((focused.node, goal));
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
            self.text_edit.caret_goal_x = None;
            let draws_value = editor_draws_its_value(&self.world, focused.node, probe_value);
            // Left/Right over a non-empty selection collapse it onto its
            // edge on that side of the screen — the one `EditSession` picks —
            // rather than stepping from the focus. Logical edge without
            // geometry (a masked or empty field draws something else).
            let collapse = horizontal_rightwards(intent)
                .filter(|_| !extend && selection.anchor != selection.focus);
            let visual = if let Some(rightwards) = collapse {
                let geometric = draws_value
                    .then_some(())
                    .zip(shaper)
                    .and_then(|((), shaper)| {
                        let (style, constraints) =
                            self.world.text_input_shape_context(focused.node)?;
                        Some(EditorGeometry {
                            shaper,
                            node: focused.node,
                            text: TextContent {
                                value: probe_text.clone(),
                            },
                            style,
                            constraints,
                        })
                    });
                let edge = match geometric {
                    Some(mut geometry) => geometry.collapse_edge(selection, rightwards),
                    None => nana_text::editable::collapse_edge(
                        selection.ordered(),
                        rightwards,
                        None,
                        None,
                        false,
                    ),
                };
                Some(caret_at_edge(selection, edge))
            } else {
                // Left/Right follow visual order wherever the backend's
                // layout can say what "left" is; without geometry they are
                // grapheme steps in logical order, which is all "left" could
                // mean then.
                horizontal_rightwards(intent)
                    .filter(|_| draws_value)
                    .zip(shaper)
                    .and_then(|(rightwards, shaper)| {
                        let (style, constraints) =
                            self.world.text_input_shape_context(focused.node)?;
                        let mut geometry = EditorGeometry {
                            shaper,
                            node: focused.node,
                            text: TextContent {
                                value: probe_text.clone(),
                            },
                            style,
                            constraints,
                        };
                        geometry.visual_step(selection, rightwards)
                    })
            };
            match visual {
                Some(stepped) if extend => stepped,
                Some(stepped) => {
                    TextSelection::caret(stepped.focus).with_affinity(stepped.affinity)
                }
                None => match caret_focus(probe_value, selection, intent) {
                    Some(focus) => moved_selection(selection, focus, extend),
                    None => return Ok(false),
                },
            }
        };
        let moved = if vertical {
            to_value(selection)
        } else {
            to_value_moved(state.selection.focus, selection)
        };
        let atoms = atoms_in(
            &state.value,
            &self.editor_text_atoms(focused.node, focused.kind),
        );
        let previous = if collapses_selection(state.selection, intent, extend) {
            moved
        } else {
            state.selection
        };
        let moved = snap_selection_over_atoms(previous, moved, extend, &atoms);
        self.write_editor_selection(focused.node, moved)
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
        self.text_edit.caret_goal_x = None;
        let atoms = self.editor_text_atoms(focused.node, focused.kind);
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Delete,
            |value, selection, _| {
                let atoms = atoms_in(value, &atoms);
                let replacement = match kind {
                    TextDeleteKind::Backward => delete_backward_atoms(value, selection, &atoms)?,
                    TextDeleteKind::Forward => delete_forward_atoms(value, selection, &atoms)?,
                    TextDeleteKind::WordBackward => {
                        delete_word_backward_atoms(value, selection, &atoms)?
                    }
                    TextDeleteKind::WordForward => {
                        delete_word_forward_atoms(value, selection, &atoms)?
                    }
                    TextDeleteKind::LineStart => {
                        delete_to_line_start_atoms(value, selection, &atoms)?
                    }
                    TextDeleteKind::LineEnd => delete_to_line_end_atoms(value, selection, &atoms)?,
                };
                Some(CursorEdit::Span(replacement))
            },
        )
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
        self.text_edit.caret_goal_x = None;
        let indent_unit = focused
            .code_editing
            .as_ref()
            .map(|code| code.indent_unit.to_string());
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Typing,
            |value, selection, _| {
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
            },
        )
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
        self.text_edit.caret_goal_x = None;
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Typing,
            |value, selection, _| auto_pair_edit(value, selection, typed).map(CursorEdit::Span),
        )
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
        self.text_edit.caret_goal_x = None;
        let unit = code.indent_unit.to_string();
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Structural,
            |value, selection, _| {
                let (next, selection) = if outdent {
                    outdent_selection(value, selection, &unit)?
                } else {
                    indent_selection(value, selection, &unit)?
                };
                Some(CursorEdit::Transform { next, selection })
            },
        )
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
        self.text_edit.caret_goal_x = None;
        let prefix = code.comment_prefix.to_string();
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Structural,
            |value, selection, _| {
                let (next, selection) = toggle_line_comment(value, selection, &prefix)?;
                Some(CursorEdit::Transform { next, selection })
            },
        )
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
        self.text_edit.caret_goal_x = None;
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Structural,
            |value, selection, _| {
                let (next, selection) = move_lines(value, selection, direction)?;
                Some(CursorEdit::Transform { next, selection })
            },
        )
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
        self.text_edit.caret_goal_x = None;
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Structural,
            |value, selection, _| {
                let (next, selection) = duplicate_lines(value, selection)?;
                Some(CursorEdit::Transform { next, selection })
            },
        )
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
        self.text_edit.caret_goal_x = None;
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Delete,
            |value, selection, _| {
                let (next, selection) = delete_lines(value, selection)?;
                Some(CursorEdit::Transform { next, selection })
            },
        )
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
        self.text_edit.caret_goal_x = None;
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Structural,
            |value, selection, _| {
                let (next, selection) = join_lines(value, selection)?;
                Some(CursorEdit::Transform { next, selection })
            },
        )
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
        self.text_edit.caret_goal_x = None;
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Structural,
            |value, selection, _| {
                let (next, selection) = transform_selection_case(value, selection, upper)?;
                Some(CursorEdit::Transform { next, selection })
            },
        )
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
        self.text_edit.caret_goal_x = None;
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Structural,
            |value, selection, _| {
                let (next, selection) = sort_lines(value, selection, descending, unique)?;
                Some(CursorEdit::Transform { next, selection })
            },
        )
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
        if !focused.accepts_selection {
            return Ok(false);
        }
        let state = self.editor_state(focused.node)?;
        let Some((open, close)) = matching_bracket_pair(&state.value, state.selection.focus) else {
            return Ok(false);
        };
        let target = if state.selection.focus <= open {
            close
        } else {
            open
        };
        self.text_edit.caret_goal_x = None;
        self.write_editor_selection(
            focused.node,
            moved_selection(state.selection, target, false),
        )
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
        if focused.node != node || !focused.accepts_selection {
            self.text_edit.text_pointer_drag = None;
            return Ok(false);
        }
        // Everything below compares against the editor's layout box: bring
        // the window point through the scroll offsets and transforms above.
        let (x, y) = self.editor_point(node, x, y);
        // A press on a numeric field's spinner steps the value
        // ([`Self::press_number_stepper`]). The editor still owns the press,
        // so nothing else starts a selection there, but it places no caret.
        if focused.is_numeric() && self.on_number_stepper(node, x, y) {
            self.text_edit.text_pointer_drag = None;
            // A spinner press between two text presses breaks their click
            // run: they are two single clicks, not a double click.
            self.text_edit.text_pointer_click = None;
            return Ok(true);
        }
        let state = self.editor_state(node)?;
        const DOUBLE_CLICK_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);
        const DOUBLE_CLICK_SLOP: f32 = 4.0;
        let count = match &self.text_edit.text_pointer_click {
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
        self.text_edit.text_pointer_click = Some(crate::framework::TextPointerClick {
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
            self.text_edit.text_pointer_drag = None;
            return self.accept_focused_text_completion(document, Some(row));
        }
        // minimap 竖条交互：条内按下即消费（任何修饰键与连击计数）——
        // 视口滚动到点击行居中的位置并进入拖拽跟随，不移动光标、不产生
        // 选区；滚轮不经过本路径，落在条上仍按编辑器常规滚动。
        if let Some(target) = self.world.text_minimap_scroll_target(node, x, y) {
            self.text_edit.text_pointer_drag = None;
            return match self
                .behavior(node)
                .and_then(|b| b.pointer_ops)
                .and_then(|p| p.minimap_navigate)
            {
                Some(nav) => nav(self, node, focused.kind, pointer_id, target),
                None => Ok(false),
            };
        }
        if count == 1
            && !extend
            && !add_cursor
            && let Some(token) = self.world.text_atom_close_hit(node, x, y)
        {
            self.text_edit.text_pointer_drag = None;
            return match self
                .behavior(node)
                .and_then(|b| b.pointer_ops)
                .and_then(|p| p.close_text_atom)
            {
                Some(close) => close(self, document, &token),
                None => Ok(false),
            };
        }
        // 折叠交互优先：gutter 箭头与折叠摘要标记的点击切换折叠态，不落
        // 光标（单击且无修饰时）。
        if count == 1
            && !extend
            && !add_cursor
            && let Some(fold) = self.world.text_fold_hit(node, x, y)
        {
            self.text_edit.text_pointer_drag = None;
            let collapsed = self
                .world
                .text_fold_view_state(node)
                .map(|state| state.collapsed.contains(&fold))
                .unwrap_or(false);
            return self.set_node_text_fold(node, fold, !collapsed);
        }
        // 命中换算与拖选共用同一条 offset 路径（折叠视图按显示空间命中
        // 后映射回值空间）。
        let Some(hit) = self.text_editor_hit_offset(node, focused.kind, x, y, shaper)? else {
            return Ok(false);
        };
        let offset = hit.offset;
        // 拖拽移动选中文本：普通按下落在主选区内部（多行、单选区、非
        // IME 组合期）时不落光标，进入拖拽状态机；Alt 按住为复制（macOS
        // 惯例；取舍：选区内的 Alt+click 由多光标添加让位给拖拽复制）。
        if self
            .text_edit
            .text_selection_drag
            .is_some_and(|existing| existing.pointer_id != pointer_id || existing.node != node)
        {
            self.clear_text_selection_drag();
        }
        if count == 1
            && !extend
            && focused.multiline
            && focused.accepts_input
            && state.additional_selections.is_empty()
            && self.world.ime(node).is_none()
            && self.begin_text_selection_drag(
                node,
                focused.kind,
                pointer_id,
                x,
                y,
                offset,
                add_cursor,
                &state,
            )
        {
            self.text_edit.text_pointer_drag = None;
            return Ok(true);
        }
        // Alt+click toggles an extra cursor on multiline editors; single-line
        // fields keep their plain click semantics.
        if add_cursor && focused.multiline && count == 1 {
            return self.text_editor_toggle_cursor(node, &state, hit);
        }
        let atoms = atoms_in(&state.value, &self.editor_text_atoms(node, focused.kind));
        let (selection, additional) = match count {
            2 => {
                let (start, end) = word_range_at(&state.value, offset);
                let range = expand_range_over_atoms(start..end, &atoms);
                (TextSelection::new(range.start, range.end), Vec::new())
            }
            3 => {
                let (start, end) = logical_line_range(&state.value, offset);
                let range = expand_range_over_atoms(start..end, &atoms);
                (TextSelection::new(range.start, range.end), Vec::new())
            }
            // 单击落点带着命中的 affinity：软换行行尾点在左侧就留在这一行。
            _ if extend => (
                moved_selection(state.selection, offset, true).with_affinity(hit.affinity),
                state.additional_selections.clone(),
            ),
            // A plain click naturally collapses to one primary cursor.
            _ => (
                moved_selection(state.selection, offset, false).with_affinity(hit.affinity),
                Vec::new(),
            ),
        };
        // A multi-click starts a selection too. Keep capture so dragging the
        // selected word/paragraph past the editor can extend and auto-scroll.
        let anchor = if extend || count != 1 {
            selection.anchor
        } else {
            offset
        };
        self.text_edit.text_pointer_drag = Some((pointer_id, node, anchor));
        if self.text_edit.text_pointer_drag.is_some() {
            let mut mutations = MutationQueue::new();
            mutations.capture_pointer(pointer_id, node);
            self.commit_mutations(mutations)?;
        }
        self.write_editor_selections(node, selection, additional)
    }

    /// minimap 导航：把目标滚动写进世界与组件（组件字段是投影权威，
    /// 不同步会在下一帧被投影改写回去），并钉住视口使光标 reveal 让位。
    pub(crate) fn text_editor_minimap_navigate(
        &mut self,
        node: StableNodeId,
        kind: TextEditorKind,
        pointer_id: u64,
        target: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        self.component_lifecycle.wheel_scrolls.remove(&node);
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_offset(node, target);
        self.world.commit(mutations)?;
        // 钉住提交后的实际偏移（世界侧可能再钳制），保证与几何层读取的
        // 请求滚动逐位相等。
        let applied = self.world.scroll_request(node).unwrap_or(target);
        self.world.set_text_viewport_pin(node, Some(applied));
        if kind == TextEditorKind::Area {
            self.update_component(Entity::<TextArea>::from_stable_id(node), |area, _| {
                area.scroll_offset = applied;
                false
            })?;
        }
        self.text_edit.text_minimap_drag = Some((pointer_id, node, kind));
        Ok(true)
    }

    /// Toggle an Alt+click cursor at `offset`: clicking an existing additional
    /// cursor removes it, clicking anywhere else adds one (the primary
    /// selection is never removed this way).
    fn text_editor_toggle_cursor(
        &mut self,
        node: StableNodeId,
        state: &TextInputState,
        hit: crate::TextHit,
    ) -> Result<bool, FrameworkError> {
        let offset = hit.offset;
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
                    .push(TextSelection::caret(offset).with_affinity(hit.affinity));
            }
        }
        next.normalize_selections();
        self.write_editor_selections(node, next.selection, next.additional_selections)
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
        if let Some((drag_id, drag_node, drag_kind)) = self.text_edit.text_minimap_drag {
            if drag_id != pointer_id {
                return Ok(false);
            }
            let (x, y) = self.editor_point(drag_node, x, y);
            return match self.world.text_minimap_scroll_target(drag_node, x, y) {
                Some(target) => match self
                    .behavior(drag_node)
                    .and_then(|b| b.pointer_ops)
                    .and_then(|p| p.minimap_navigate)
                {
                    Some(nav) => nav(self, drag_node, drag_kind, pointer_id, target),
                    None => Ok(false),
                },
                None => Ok(true),
            };
        }
        // 拖拽移动选中文本优先：pending/active 状态机消费本次移动，不做
        // 选区延伸（拖拽期间既有点击选词等竞争语义全部让位）。
        if let Some(drag) = self.text_edit.text_selection_drag {
            if drag.pointer_id != pointer_id {
                return Ok(false);
            }
            return self.update_text_selection_drag(document, drag, x, y, shaper);
        }
        let Some((drag_id, node, anchor)) = self.text_edit.text_pointer_drag else {
            return Ok(false);
        };
        let window_point = (x, y);
        let (x, y) = self.editor_point(node, x, y);
        if drag_id != pointer_id {
            return Ok(false);
        }
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if focused.node != node {
            return Ok(false);
        }
        let state = self.editor_state(node)?;
        if let Some(content) = self.world.component_content_box(node) {
            let delta = |p: f32, start: f32, extent: f32| {
                if p < start {
                    (p - start).max(-32.0)
                } else if p > start + extent {
                    (p - start - extent).min(32.0)
                } else {
                    0.0
                }
            };
            let offset = crate::ScrollOffset {
                x: delta(x, content.x, content.width),
                y: delta(y, content.y, content.height),
            };
            if (offset.x != 0.0 || offset.y != 0.0) && self.scroll_node_by(node, offset)? {
                // Keep the host's text engine: timer ticks use the very same
                // shaping authority as the pointer that started the drag.
                self.input.plain_drag_scroll = shaper.text_engine().map(|engine| {
                    (
                        document,
                        node,
                        pointer_id,
                        window_point.0,
                        window_point.1,
                        self.component_lifecycle.now + std::time::Duration::from_millis(16),
                        crate::NanaTextEngineShaper::new(engine),
                    )
                });
            } else {
                self.input.plain_drag_scroll = None;
            }
        }
        // 命中换算与点击共用同一条 offset 路径。
        let Some(hit) = self.text_editor_hit_offset(node, focused.kind, x, y, shaper)? else {
            return Ok(false);
        };
        if hit.offset == state.selection.focus && hit.affinity == state.selection.affinity {
            return Ok(false);
        }
        self.write_editor_selection(
            node,
            TextSelection::new(anchor, hit.offset).with_affinity(hit.affinity),
        )
    }

    /// End any drag selection started by this pointer.
    pub fn text_editor_pointer_release(&mut self, pointer_id: u64) {
        if self
            .input
            .plain_drag_scroll
            .as_ref()
            .is_some_and(|state| state.2 == pointer_id)
        {
            self.input.plain_drag_scroll = None;
        }
        if let Some((drag_id, node, _)) = self.text_edit.text_pointer_drag
            && drag_id == pointer_id
            && let Some(document) = self.world.document_of(node)
        {
            self.release_pointer_capture(document, pointer_id);
        }
        if self
            .text_edit
            .text_pointer_drag
            .is_some_and(|(drag_id, _, _)| drag_id == pointer_id)
        {
            self.text_edit.text_pointer_drag = None;
        }
        if self
            .text_edit
            .text_minimap_drag
            .is_some_and(|(drag_id, _, _)| drag_id == pointer_id)
        {
            self.text_edit.text_minimap_drag = None;
        }
        // 落点未经理由 `text_editor_selection_drop` 消费（指针路径未带
        // 文档/坐标等场景）时按取消处理，一并清除落点指示线。
        if self
            .text_edit
            .text_selection_drag
            .is_some_and(|drag| drag.pointer_id == pointer_id)
        {
            self.clear_text_selection_drag();
        }
    }

    pub(crate) fn plain_drag_deadline(&self) -> Option<std::time::Duration> {
        let (doc, node, pointer, _, _, deadline, _) = self.input.plain_drag_scroll.as_ref()?;
        (self.world.focused(*doc) == Some(*node)
            && self.world.is_mounted(*node)
            && self
                .text_edit
                .text_pointer_drag
                .is_some_and(|(p, n, _)| p == *pointer && n == *node))
        .then_some(*deadline)
    }

    pub(crate) fn advance_plain_drag(&mut self, now: std::time::Duration) -> Option<StableNodeId> {
        let deadline = self.plain_drag_deadline()?;
        if deadline > now {
            return None;
        }
        let (doc, node, pointer, x, y, _, mut shaper) = self.input.plain_drag_scroll.take()?;
        let changed = self
            .text_editor_pointer_drag(doc, pointer, x, y, &mut shaper)
            .ok()?;
        if let Some(state) = &mut self.input.plain_drag_scroll {
            state.6 = shaper;
        }
        changed.then_some(node)
    }

    /// 进入拖拽移动选中的待定态。要求：多行、单选区、非 IME 组合期，
    /// 按下点严格落在非空主选区内部。返回是否已接管本次按下。
    #[allow(clippy::too_many_arguments)]
    fn begin_text_selection_drag(
        &mut self,
        node: StableNodeId,
        kind: TextEditorKind,
        pointer_id: u64,
        x: f32,
        y: f32,
        offset: usize,
        copy: bool,
        state: &TextInputState,
    ) -> bool {
        let range = state.selection.ordered();
        if range.start == range.end || offset < range.start || offset >= range.end {
            return false;
        }
        if self.text_edit.text_selection_drag.is_some() {
            return false;
        }
        self.text_edit.text_selection_drag = Some(TextSelectionDrag {
            pointer_id,
            node,
            kind,
            press: (x, y),
            source: (range.start, range.end),
            active: false,
            copy,
            target: None,
        });
        true
    }

    /// 拖拽态移动：pending 态先做阈值判定；active 态换算落点偏移（按
    /// 折叠/软换行的既有 offset 命中换算），把落点指示线发布到世界侧。
    /// 落点在源选区内部视为无效落点（指示线消失，释放时取消）。
    fn update_text_selection_drag(
        &mut self,
        document: DocumentId,
        mut drag: TextSelectionDrag,
        x: f32,
        y: f32,
        shaper: &mut dyn crate::TextShaper,
    ) -> Result<bool, FrameworkError> {
        let (x, y) = self.editor_point(drag.node, x, y);
        let Some(focused) = self.focused_text_editor(document) else {
            self.clear_text_selection_drag();
            return Ok(false);
        };
        if focused.node != drag.node {
            self.clear_text_selection_drag();
            return Ok(false);
        }
        if !drag.active {
            let (dx, dy) = (x - drag.press.0, y - drag.press.1);
            if dx * dx + dy * dy <= TEXT_SELECTION_DRAG_THRESHOLD * TEXT_SELECTION_DRAG_THRESHOLD {
                self.text_edit.text_selection_drag = Some(drag);
                return Ok(true);
            }
            drag.active = true;
        }
        let Some(target) = self
            .text_editor_hit_offset(drag.node, drag.kind, x, y, shaper)?
            .map(|hit| hit.offset)
        else {
            self.text_edit.text_selection_drag = Some(drag);
            return Ok(true);
        };
        // 源选区区间内（含边界，落点为退化 no-op）不显示指示线。
        drag.target = (!(drag.source.0..=drag.source.1).contains(&target)).then_some(target);
        if let (Some(target), Some((style, constraints))) =
            (drag.target, self.world.text_input_shape_context(drag.node))
        {
            // 折叠探测用显示值（无折叠时为编辑器原值）。
            let value =
                self.editor_probe_text(drag.node, self.world.text_display_view(drag.node).as_ref());
            let mut geometry = EditorGeometry {
                shaper,
                node: drag.node,
                text: TextContent { value },
                style,
                constraints,
            };
            let (indicator_x, indicator_y, indicator_height) = (geometry.probe())(target);
            self.world.set_text_drop_indicator(
                drag.node,
                Some(crate::LayoutBox {
                    x: indicator_x,
                    y: indicator_y,
                    width: nana_ui_core::space::XXS,
                    height: indicator_height,
                }),
            );
        } else {
            self.world.set_text_drop_indicator(drag.node, None);
        }
        self.text_edit.text_selection_drag = Some(drag);
        Ok(true)
    }

    /// 指针释放的落点执行：pending 态回落为普通点击（caret 塌缩到落
    /// 点）；active 态执行移动/复制——删除+插入拼成一次值修订（单个
    /// change 事件，一步撤销），落点在源选区内部则取消（选区保持原
    /// 状）。返回是否消费本次释放。
    pub fn text_editor_selection_drop(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        shaper: &mut dyn crate::TextShaper,
    ) -> Result<bool, FrameworkError> {
        let Some(drag) = self.text_edit.text_selection_drag else {
            return Ok(false);
        };
        if drag.pointer_id != pointer_id {
            return Ok(false);
        }
        let (x, y) = self.editor_point(drag.node, x, y);
        self.clear_text_selection_drag();
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if focused.node != drag.node || !focused.accepts_input {
            return Ok(false);
        }
        if !drag.active {
            // 低于阈值：按原点击语义处理（落 caret）。
            let Some(hit) = self.text_editor_hit_offset(drag.node, drag.kind, x, y, shaper)? else {
                return Ok(false);
            };
            return self.write_editor_selections(
                drag.node,
                TextSelection::caret(hit.offset).with_affinity(hit.affinity),
                Vec::new(),
            );
        }
        let Some(target) = self
            .text_editor_hit_offset(drag.node, drag.kind, x, y, shaper)?
            .map(|hit| hit.offset)
        else {
            return Ok(true);
        };
        let state = self.editor_state(drag.node)?;
        if state.selection.ordered() != (drag.source.0..drag.source.1) {
            // 拖拽期间选区被外部改动：取消，不落文本。
            return Ok(true);
        }
        let (start, end) = drag.source;
        let length = end - start;
        // 源选区边界（含两端）都是退化落点：移动/复制均为 no-op，取消。
        if (start..=end).contains(&target) {
            return Ok(true);
        }
        let value = &state.value;
        let (next, insert_at) = if drag.copy {
            // 复制：目标点插入选中文本，原文本保留。
            (
                format!(
                    "{}{}{}",
                    &value[..target],
                    &value[start..end],
                    &value[target..]
                ),
                target,
            )
        } else if target > end {
            // 向后移动：删除源区间，剩余中段让位，文本插到目标前。
            (
                format!(
                    "{}{}{}{}",
                    &value[..start],
                    &value[end..target],
                    &value[start..end],
                    &value[target..]
                ),
                target - length,
            )
        } else {
            // 向前移动（target < start）。
            (
                format!(
                    "{}{}{}{}",
                    &value[..target],
                    &value[start..end],
                    &value[target..start],
                    &value[end..]
                ),
                target,
            )
        };
        let selection = TextSelection::new(insert_at, insert_at + length);
        self.commit_editor_value(
            drag.node,
            next.into(),
            selection,
            Vec::new(),
            crate::TextEditOrigin::Structural,
        )
    }

    /// 取消拖拽移动选中（Esc）。仅当拖拽属于该文档的聚焦编辑器时消费。
    pub fn cancel_focused_text_selection_drag(&mut self, document: DocumentId) -> bool {
        let Some(drag) = &self.text_edit.text_selection_drag else {
            return false;
        };
        let node = drag.node;
        let owns = self
            .focused_text_editor(document)
            .is_some_and(|focused| focused.node == node);
        self.clear_text_selection_drag();
        owns
    }

    fn clear_text_selection_drag(&mut self) {
        if let Some(drag) = self.text_edit.text_selection_drag.take() {
            self.world.set_text_drop_indicator(drag.node, None);
        }
    }

    /// 指针位置 → 值空间 caret（折叠视图按显示空间命中后映射回值空间；
    /// 与点击/拖选共用同一换算）。无法解析时返回 `None`。
    ///
    /// affinity 由命中点决定，随选区一路写进编辑器状态：CJK 这类没有悬挂
    /// 空白的软换行行尾即下一行行首，只有 `Upstream` 才把 caret 留在被点
    /// 中的那一行。宿主答不出命中时按 downstream 的探针搜索退化。
    /// A window point in `node`'s layout space: through the scroll offsets
    /// and transforms above it, as hit testing sees it.
    pub(super) fn editor_point(&self, node: StableNodeId, x: f32, y: f32) -> (f32, f32) {
        self.world
            .pointer_layout_position(node, x, y)
            .unwrap_or((x, y))
    }

    fn text_editor_hit_offset(
        &self,
        node: StableNodeId,
        kind: TextEditorKind,
        x: f32,
        y: f32,
        shaper: &mut dyn crate::TextShaper,
    ) -> Result<Option<crate::TextHit>, FrameworkError> {
        // The press that focused the field lands where the user saw the
        // text: unfocused, a single-line field shows its start, not the end
        // its caret was left at.
        let frame = if self.text_edit.unfocused_press == Some(node) {
            self.world.editor_frame_as(node, false)
        } else {
            self.world.editor_frame(node)
        };
        let Some(frame) = frame else {
            return Ok(None);
        };
        let Some((style, constraints)) = self.world.text_input_shape_context(node) else {
            return Ok(None);
        };
        let state = self.editor_state(node)?;
        let fold_view = self.world.text_display_view(node);
        let probe_value: &str = fold_view.as_ref().map_or(&state.value, |view| &view.value);
        let mut geometry = EditorGeometry {
            shaper,
            node,
            text: TextContent {
                value: self.editor_probe_text(node, fold_view.as_ref()),
            },
            style,
            constraints,
        };
        // The point in the editor's text space, through the frame its caret
        // and selection are drawn with: the same scroll, centring and
        // inline-start anchoring, in every writing mode.
        let (local_x, local_y) = frame.text_point(x, y);
        let hit = match geometry.shaper.text_hit_at_point(
            node,
            &geometry.text,
            local_x,
            local_y,
            &geometry.style,
            geometry.constraints,
        ) {
            Some(hit) => hit,
            None => crate::TextHit::downstream(caret_offset_at_point(
                probe_value,
                local_x,
                local_y,
                geometry.probe(),
            )),
        };
        let offset = match &fold_view {
            Some(view) => view.value_of(hit.offset),
            None => hit.offset,
        };
        let atoms = atoms_in(&state.value, &self.editor_text_atoms(node, kind));
        let snapped = snap_pointer_caret(offset, &atoms);
        // 吸附到原子边界后落点不再是被点中的那个位置，affinity 随之作废。
        let affinity = if snapped == offset {
            hit.affinity
        } else {
            crate::TextAffinity::Downstream
        };
        Ok(Some(crate::TextHit {
            offset: snapped,
            affinity,
        }))
    }

    /// The byte offset a document point lands on in the focused plain
    /// editor's value, as a tap there places the caret. For a host whose
    /// platform text system drives the loupe, selection handles and edit
    /// menu (UIKit `UITextInteraction`); `None` without a focused editor.
    pub fn focused_text_offset_at(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
        shaper: &mut dyn crate::TextShaper,
    ) -> Result<Option<usize>, FrameworkError> {
        let Some(focused) = self.focused_plain_editor(document) else {
            return Ok(None);
        };
        let (x, y) = self.editor_point(focused.node, x, y);
        Ok(self
            .text_editor_hit_offset(focused.node, focused.kind, x, y, shaper)?
            .map(|hit| hit.offset))
    }

    /// The caret box before byte `offset` of the focused plain editor's
    /// value, in document coordinates where it shows (the field's scroll and
    /// the transforms above it applied): what
    /// [`Self::focused_text_offset_at`] inverts. `None` without a focused
    /// editor, and for an editor showing a folded view of its value.
    pub fn focused_text_caret_rect(
        &self,
        document: DocumentId,
        offset: usize,
        shaper: &mut dyn crate::TextShaper,
    ) -> Result<Option<crate::LayoutBox>, FrameworkError> {
        let Some(focused) = self.focused_plain_editor(document) else {
            return Ok(None);
        };
        let node = focused.node;
        if self.world.text_display_view(node).is_some() {
            return Ok(None);
        }
        let (Some(frame), Some((style, constraints))) = (
            self.world.editor_frame(node),
            self.world.text_input_shape_context(node),
        ) else {
            return Ok(None);
        };
        let offset = offset.min(self.editor_state(node)?.value.len());
        let mut geometry = EditorGeometry {
            shaper,
            node,
            text: TextContent {
                value: self.editor_probe_text(node, None),
            },
            style,
            constraints,
        };
        let (x, y, line) = geometry.probe()(offset);
        let caret = frame.field_rect(crate::LayoutBox {
            x,
            y,
            width: 0.0,
            height: line,
        });
        Ok(Some(
            self.world
                .project_input_bounds(node, caret)
                .unwrap_or(caret),
        ))
    }

    /// Delete the atom identified by `token` through the normal editor history
    /// and notify [`crate::TextAtomClosed`].
    pub fn close_focused_text_atom(
        &mut self,
        document: DocumentId,
        token: &str,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            return Ok(false);
        };
        if focused.kind != TextEditorKind::Area || !focused.accepts_input {
            return Ok(false);
        }
        let atoms = atoms_in(
            &self.editor_state(focused.node)?.value,
            &self.editor_text_atoms(focused.node, focused.kind),
        );
        let Some(atom) = atoms
            .iter()
            .find(|atom| atom.token.as_ref() == token)
            .cloned()
        else {
            return Ok(false);
        };
        self.write_editor_selection(focused.node, TextSelection::new(atom.start, atom.end))?;
        if !self.delete_focused_text(document, TextDeleteKind::Backward)? {
            return Ok(false);
        }
        let entity = Entity::<TextArea>::from_stable_id(focused.node);
        self.update_component(entity, |_, cx| {
            cx.emit(crate::TextAtomClosed {
                token: Arc::clone(&atom.token),
                start: atom.start,
                end: atom.end,
            });
            false
        })?;
        Ok(true)
    }
}
