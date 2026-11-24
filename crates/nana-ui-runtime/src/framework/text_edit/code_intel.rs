use super::{AppContext, DocumentId, FrameworkError, StableNodeId, TextSelection};
use crate::TextSnippet;
use crate::components::TextSnippetSession;
use crate::text_editing::{CursorEdit, TextReplacement, clamp_boundary, logical_line_range};
use crate::{MutationQueue, TextCodeFold};
use nana_ui_core::sort::CompactSort as _;
use std::sync::Arc;

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
    stops.compact_sort_by_key(|(number, _)| *number);
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
        (!state.items.is_empty() && !state.dismissed).then_some((focused.node, state))
    }

    /// 聚焦多行编辑器当前是否激活补全弹层（非空且未关闭的会话）。
    /// 路由层用它决定编辑键是否被弹层消费（边界上导航无可做仍消费）。
    pub fn focused_text_completion_active(&self, document: DocumentId) -> bool {
        self.focused_completion_session(document).is_some()
    }

    /// 聚焦编辑器当前是否显示签名帮助浮窗。Esc 两段式：签名在场时
    /// 路由层先消费 Esc（不关补全），由宿主撤掉签名会话。
    pub fn focused_text_signature_showing(&self, document: DocumentId) -> bool {
        self.focused_text_editor(document)
            .is_some_and(|focused| self.world.text_signature_help(focused.node).is_some())
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
        self.text_edit.caret_goal_x = None;
        if let Some(session) = self.world.text_snippet_session(focused.node)
            && let Some(group) = session
                .index
                .checked_sub(1)
                .and_then(|i| session.placeholders.get(i))
            && group.choices.iter().any(|choice| choice == &label)
            && let Some(range) = group.ranges.first()
        {
            let mut next = self.editor_state(focused.node)?;
            next.selection = TextSelection::new(range.start, range.end);
            next.replace_primary_selection(&label);
            let changed = self.commit_editor_value(
                focused.node,
                next.value,
                next.selection,
                Vec::new(),
                crate::TextEditOrigin::Structural,
            )?;
            self.dismiss_focused_text_completion(document)?;
            return Ok(changed);
        }
        if let Some(raw_edit) = &item.edit {
            let mut edit = raw_edit.clone();
            let expansion = if let Some(body) = &edit.snippet {
                let variables = crate::snippet::context_variables(
                    &edit.source,
                    TextSelection::caret(edit.caret),
                    &edit.variables,
                );
                let Some(expansion) = crate::expand_text_snippet_with_variables(body, &variables)
                else {
                    return Ok(false);
                };
                edit.text = expansion.text.clone();
                edit.stops = expansion
                    .placeholders
                    .iter()
                    .map(|g| g.ranges[0].clone())
                    .collect();
                Some(expansion)
            } else {
                None
            };
            let current = self.editor_state(focused.node)?;
            if current.value != edit.source.as_ref()
                || current.selection != TextSelection::caret(edit.caret)
            {
                return Ok(false);
            }
            let mut replacements = edit.additional.clone();
            replacements.push((edit.range.clone(), edit.text.clone()));
            replacements.compact_sort_by_key(|(range, _)| (range.start, range.end));
            if replacements.iter().any(|(range, _)| {
                range.start > range.end
                    || range.end > current.value.len()
                    || !current.value.is_char_boundary(range.start)
                    || !current.value.is_char_boundary(range.end)
            }) || replacements
                .windows(2)
                .any(|pair| pair[0].0.end > pair[1].0.start || pair[0].0.start == pair[1].0.start)
                || edit.stops.iter().any(|stop| {
                    stop.start > stop.end
                        || !edit.text.is_char_boundary(stop.start)
                        || !edit.text.is_char_boundary(stop.end)
                })
            {
                return Ok(false);
            }
            let shift: isize = edit
                .additional
                .iter()
                .filter(|(range, _)| range.end <= edit.range.start)
                .map(|(range, text)| text.len() as isize - range.len() as isize)
                .sum();
            let base = edit
                .range
                .start
                .checked_add_signed(shift)
                .expect("validated disjoint edits");
            let mut next = current;
            for (range, text) in replacements.into_iter().rev() {
                next.selection = TextSelection::new(range.start, range.end);
                next.replace_primary_selection(&text);
            }
            next.selection = edit
                .stops
                .first()
                .map(|stop| TextSelection::new(base + stop.start, base + stop.end))
                .unwrap_or_else(|| TextSelection::caret(base + edit.text.len()));
            let changed = self.commit_editor_value(
                focused.node,
                next.value,
                next.selection,
                next.additional_selections,
                crate::TextEditOrigin::Structural,
            )?;
            if changed {
                let stops = edit
                    .stops
                    .iter()
                    .map(|range| base + range.start)
                    .collect::<Vec<_>>();
                let selection_ends = edit.stops.iter().map(|range| base + range.end).collect();
                let session = (!stops.is_empty()
                    && expansion.as_ref().is_none_or(|e| e.placeholders.len() > 1))
                .then_some(TextSnippetSession {
                    exit_on_last: true,
                    stops,
                    selection_ends,
                    placeholders: expansion
                        .map(|e| {
                            e.placeholders
                                .into_iter()
                                .map(|mut g| {
                                    for r in &mut g.ranges {
                                        r.start += base;
                                        r.end += base;
                                    }
                                    for (r, _) in &mut g.transforms {
                                        r.start += base;
                                        r.end += base;
                                    }
                                    g
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    index: 1,
                });
                let mut mutations = MutationQueue::new();
                mutations.set_text_input_snippet(focused.node, session);
                self.world.commit(mutations)?;
            }
            return Ok(changed);
        }
        self.edit_editor_multi(
            focused.node,
            crate::TextEditOrigin::Structural,
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

    /// 指针落在诊断 squiggle 或行尾文案上时打开诊断 hover；离开则撤掉
    /// 诊断占用的浮窗（不撤宿主符号 hover）。停在已打开的诊断浮窗上保持。
    pub fn update_text_diagnostic_hover(
        &mut self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> Result<(), FrameworkError> {
        if self.diagnostic_hover_panel_holds(document, x, y) {
            return Ok(());
        }
        let target = self.world.hit_test(document, x, y);
        self.update_text_diagnostic_hover_hit(document, x, y, target)
    }

    /// The pointer rests on an open diagnostic hover panel, which keeps it.
    fn diagnostic_hover_panel_holds(&self, document: DocumentId, x: f32, y: f32) -> bool {
        self.world
            .text_hover_panel_at(document, x, y)
            .is_some_and(|node| {
                self.world
                    .text_hover_view(node)
                    .is_some_and(|state| state.diagnostic)
            })
    }

    /// [`Self::update_text_diagnostic_hover`] with the point already
    /// hit-tested: `target` is the topmost node there.
    pub fn update_text_diagnostic_hover_hit(
        &mut self,
        document: DocumentId,
        x: f32,
        y: f32,
        target: Option<StableNodeId>,
    ) -> Result<(), FrameworkError> {
        if self.diagnostic_hover_panel_holds(document, x, y) {
            return Ok(());
        }
        // Nothing to open and nothing open: the common move over plain UI.
        if target.is_none_or(|id| self.world.editor_frame(id).is_none())
            && self.world.diagnostic_hover_ids().is_empty()
        {
            return Ok(());
        }
        let hit = target.and_then(|id| {
            let (x, y) = self.editor_point(id, x, y);
            let (local_x, local_y) = self.world.editor_frame(id)?.text_point(x, y);
            self.world
                .text_diagnostic_hit(id, local_x, local_y)
                .map(|hover| (id, hover))
        });
        let mut mutations = crate::MutationQueue::new();
        let mut dirty = false;
        if let Some((id, hover)) = hit.as_ref() {
            mutations.set_text_input_diagnostic_hover(*id, Some(hover.clone()));
            dirty = true;
        }
        for id in self.world.diagnostic_hover_ids() {
            if hit.as_ref().is_some_and(|(hit_id, _)| *hit_id == id) {
                continue;
            }
            mutations.set_text_input_diagnostic_hover(id, None);
            dirty = true;
        }
        if dirty {
            self.world.commit(mutations)?;
        }
        Ok(())
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
    pub(super) fn unfold_text_folds_containing(
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
            .filter_map(|span| span.fold())
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
    pub(super) fn set_node_text_fold(
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
                state
                    .collapsed
                    .compact_sort_by_key(|fold| (fold.start, fold.end));
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
        if !focused.multiline || !focused.accepts_selection {
            return Ok(false);
        }
        let state = self.editor_state(focused.node)?;
        let view = self.world.text_fold_view_state(focused.node);
        let offered = view.as_ref().map_or(&[][..], |state| &state.offered);
        let at = offset.unwrap_or(state.selection.focus);
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
        self.text_edit.caret_goal_x = None;
        let state = self.editor_state(node)?;
        let (insert, stops, final_caret) =
            expand_snippet_body(&snippet.body, state.selection.ordered().start);
        let mut next = state.clone();
        // 主光标独占：与 IME 提交同款路径，附加光标经偏移重映射保留。
        if !next.replace_primary_selection(&insert) {
            return Ok(false);
        }
        let session = (!stops.is_empty()).then_some(TextSnippetSession {
            exit_on_last: false,
            stops,
            selection_ends: Vec::new(),
            placeholders: Vec::new(),
            index: 0,
        });
        // 选区落在 $0（缺省为插入文本末尾）。
        next.selection = TextSelection::caret(final_caret);
        // 走与其它编辑同一个入口：一次插入是一个撤销步，只读与长度上限
        // 照常把关。旧会话先结束——新插入不是旧占位的联动编辑；插入被拒
        // 时原样恢复。
        let previous = self.world.text_snippet_session(node);
        if previous.is_some() {
            let mut mutations = crate::MutationQueue::new();
            mutations.set_text_input_snippet(node, None);
            self.world.commit(mutations)?;
        }
        if !self.commit_editor_value(
            node,
            next.value,
            next.selection,
            next.additional_selections,
            crate::TextEditOrigin::Structural,
        )? {
            if previous.is_some() {
                let mut mutations = crate::MutationQueue::new();
                mutations.set_text_input_snippet(node, previous);
                self.world.commit(mutations)?;
            }
            return Ok(false);
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
        if !focused.multiline || !focused.accepts_selection {
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
            let state = self.editor_state(node)?;
            if clamp_boundary(&state.value, caret) != caret {
                // 跳位失效（文本边界已不在字符边界上）：结束会话。
                ended = true;
            } else {
                let end = session
                    .selection_ends
                    .get(session.index.saturating_sub(1))
                    .copied()
                    .unwrap_or(caret);
                self.write_editor_selection(node, TextSelection::new(caret, end))?;
            }
        }
        if session.exit_on_last && session.index == session.stops.len() {
            ended = true;
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
