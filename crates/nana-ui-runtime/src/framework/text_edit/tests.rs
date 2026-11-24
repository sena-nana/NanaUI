use super::*;

#[cfg(test)]
mod fold_snippet_tests {
    use super::*;
    use crate::TextCodeFold;
    use crate::text_editing::TextCaretIntent;
    use crate::{AppContext, DocumentId, TextSnippet};
    use std::sync::Arc;

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
            .find_next_focused_text_match(
                document,
                "x();",
                crate::TextSearchOptions::default(),
                TextFindScope::Document,
            )
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
                area_view.state.selection = TextSelection::new(13, 17);
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
            vec![TextSelection::new(13, 17), TextSelection::new(33, 37),]
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
                area_view.state.selection = TextSelection::new(2, 6);
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
    use crate::{
        AppContext, DocumentId, Entity, StableNodeId, TextArea, TextCompletion, TextSignatureHelp,
    };
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
    fn signature_showing_is_reported_for_escape_two_stage() {
        let (mut context, document, area, _node) =
            completion_editor("mix(", items(&["mix", "max"]));
        assert!(!context.focused_text_signature_showing(document));
        assert!(context.focused_text_completion_active(document));
        context
            .update_component(area, |view, _| {
                view.signature = Some(TextSignatureHelp::new(
                    "mix",
                    "",
                    vec![("e1".into(), String::new())],
                    0,
                ));
            })
            .unwrap();
        assert!(context.focused_text_signature_showing(document));
        assert!(
            context.focused_text_completion_active(document),
            "签名在场时补全会话仍保持"
        );
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

    #[test]
    fn completion_transaction_inserts_server_text_and_additions_once_with_placeholder_selection() {
        let source = "// header\nfo_tail";
        let edit = crate::TextCompletionEdit {
            snippet: None,
            variables: Default::default(),
            source: Arc::from(source),
            caret: 12,
            range: 10..source.len(),
            text: "foo(value, other)".into(),
            additional: vec![(0..0, "// added\n".into())],
            stops: vec![4..9, 11..16, 17..17],
        };
        let candidate = TextCompletion::new("display label", "fn").edit(edit);
        let (mut context, document, area, node) = completion_editor(source, Arc::from([candidate]));
        caret_to(&mut context, area, 12);
        let changes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let emitted = Arc::clone(&changes);
        context
            .on(area, move |_, event: &crate::TextChanged, _| {
                emitted.lock().unwrap().push(event.value.clone());
            })
            .unwrap();
        assert!(
            context
                .accept_focused_text_completion(document, None)
                .unwrap()
        );
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "// added\n// header\nfoo(value, other)"
        );
        assert_eq!(
            changes.lock().unwrap().len(),
            1,
            "one change produces one host undo transaction"
        );
        assert_eq!(selection_of(&context, node), (23, 28));
        assert!(context.replace_focused_text(document, "x").unwrap());
        assert!(
            context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (26, 31));
    }

    #[test]
    fn snippet_completion_links_utf8_edits_and_choices_with_tab_navigation() {
        let candidate = TextCompletion::new("linked", "snippet").edit(crate::TextCompletionEdit {
            snippet: Some("${1:颜色} + $1 + ${2|red,blue|} + $2$0".into()),
            variables: Default::default(),
            source: Arc::from("fo"),
            caret: 2,
            range: 0..2,
            text: String::new(),
            additional: vec![],
            stops: vec![],
        });
        let (mut context, document, area, node) = completion_editor("fo", Arc::from([candidate]));
        caret_to(&mut context, area, 2);
        let changes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let emitted = Arc::clone(&changes);
        context
            .on(area, move |_, event: &crate::TextChanged, _| {
                emitted.lock().unwrap().push(event.value.clone());
            })
            .unwrap();
        assert!(
            context
                .accept_focused_text_completion(document, None)
                .unwrap()
        );
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "颜色 + 颜色 + red + red"
        );
        assert_eq!(changes.lock().unwrap().len(), 1);
        assert!(context.replace_focused_text(document, "蓝").unwrap());
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "蓝 + 蓝 + red + red"
        );
        assert!(context.commit_ime(document, "色").unwrap());
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "蓝色 + 蓝色 + red + red"
        );
        assert_eq!(
            changes.lock().unwrap().len(),
            3,
            "one event per linked edit"
        );
        assert!(
            context
                .advance_focused_text_snippet(document, false)
                .unwrap()
        );
        context.reproject_component(area).unwrap();
        assert!(context.focused_text_completion_active(document));
        assert!(
            context
                .move_focused_text_completion(document, true)
                .unwrap()
        );
        assert!(
            context
                .accept_focused_text_completion(document, None)
                .unwrap()
        );
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "蓝色 + 蓝色 + blue + blue"
        );
        assert!(
            context
                .advance_focused_text_snippet(document, true)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (0, 6));
        assert!(context.replace_focused_text(document, "x").unwrap());
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "x + x + blue + blue"
        );
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
        let end = context.world().text_input(node).unwrap().value.len();
        assert_eq!(selection_of(&context, node), (end, end));
    }
    #[test]
    fn completion_transaction_rejects_stale_source_caret_and_overlapping_edits() {
        for (source, caret, additional) in [
            ("old", 2, vec![]),
            ("fo", 1, vec![]),
            ("fo", 2, vec![(0..1, "x".into())]),
        ] {
            let candidate = TextCompletion::new("label", "fn").edit(crate::TextCompletionEdit {
                snippet: None,
                variables: Default::default(),
                source: Arc::from(source),
                caret,
                range: 0..2,
                text: "foo()".into(),
                additional,
                stops: vec![],
            });
            let (mut context, document, area, node) =
                completion_editor("fo", Arc::from([candidate]));
            caret_to(&mut context, area, 2);
            assert!(
                !context
                    .accept_focused_text_completion(document, None)
                    .unwrap()
            );
            assert_eq!(context.world().text_input(node).unwrap().value, "fo");
        }
    }
}

#[cfg(test)]
mod minimap_tests {
    use crate::{
        AppContext, DocumentId, Entity, MeasureTextShaper, MutationQueue, StableNodeId, TextArea,
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
            // Narrow enough glyphs that no line wraps beside the minimap: the
            // editor keeps its 30 line breaks, and 30 lines are 300 px.
            layout.font_size = Some(8.0);
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
        match context.world().text_input_geometry(node) {
            Some(crate::TextInputGeometry {
                minimap: Some(minimap),
                ..
            }) => minimap.panel,
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
        let additional_before = state.additional_selections.to_vec();

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
        let text = context.world().text_input_geometry(node).unwrap().text;
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
        let text = context.world().text_input_geometry(node).unwrap().text;
        // 光标在第 0 行（y=0），reveal 后视口回顶部：文本区 y 回到内容顶。
        assert!((text.bounds.y - content.y).abs() < 0.01);
    }

    /// The scroll the editor is drawn at: its text origin above the content
    /// box.
    fn drawn_scroll_y(context: &AppContext, node: StableNodeId) -> f32 {
        let (content, _) = context.world().text_input_pointer_context(node).unwrap();
        let text = context.world().text_input_geometry(node).unwrap().text;
        content.y - text.bounds.y
    }

    fn move_caret(
        context: &mut AppContext,
        area: Entity<TextArea>,
        node: StableNodeId,
        offset: usize,
    ) {
        context
            .update_component(area, |area, _| {
                area.state.selection = crate::TextSelection::caret(offset);
                true
            })
            .unwrap();
        context
            .world_mut()
            .shape_text(&[node], &mut MeasureTextShaper)
            .unwrap();
    }

    /// The caret reveal scrolls a focused editor to its caret; the editor
    /// reports that scroll, so a wheel carries on from what is on screen
    /// instead of from the offset set before the caret moved.
    #[test]
    fn a_wheel_after_the_caret_reveal_scrolls_from_what_is_drawn() {
        let (mut context, _document, area, node) = minimap_editor();
        let end = context.world().text_input(node).unwrap().value.len();
        move_caret(&mut context, area, node, end);
        // 30 lines of 10 px in a 100 px box: the last line is revealed at the
        // bottom, so the editor is drawn scrolled to the end.
        let revealed = drawn_scroll_y(&context, node);
        assert!(
            revealed > 150.0,
            "caret reveal scrolls to the end: {revealed}"
        );
        assert_eq!(scroll_y_of(&context, node), revealed);

        assert!(
            context
                .scroll_node_by(node, crate::ScrollOffset { x: 0.0, y: -10.0 })
                .unwrap()
        );
        assert_eq!(scroll_y_of(&context, node), revealed - 10.0);
        assert_eq!(drawn_scroll_y(&context, node), revealed - 10.0);
    }

    /// Shortening the value leaves the request past the new end; the editor
    /// is drawn clamped, and a wheel moves from there on its first tick.
    #[test]
    fn a_wheel_after_the_value_shrinks_moves_on_its_first_tick() {
        let (mut context, _document, area, node) = minimap_editor();
        move_caret(&mut context, area, node, 0);
        assert_eq!(scroll_y_of(&context, node), 0.0);
        assert!(
            context
                .scroll_node_by(node, crate::ScrollOffset { x: 0.0, y: 1_000.0 })
                .unwrap()
        );
        let bottom = scroll_y_of(&context, node);
        assert!(bottom > 150.0, "scrolled to the end: {bottom}");

        // 15 lines of 10 px: 50 px past a 100 px box. The caret stays on the
        // first line, so the wheel's pin holds.
        let shorter = (0..15)
            .map(|index| format!("line{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        context
            .update_component(area, |area, _| {
                area.state.value = shorter.into();
                true
            })
            .unwrap();
        context
            .world_mut()
            .shape_text(&[node], &mut MeasureTextShaper)
            .unwrap();
        let end = scroll_y_of(&context, node);
        assert_eq!(drawn_scroll_y(&context, node), end);
        assert!(end < bottom, "drawn clamped to the shorter value: {end}");

        assert!(
            context
                .scroll_node_by(node, crate::ScrollOffset { x: 0.0, y: -10.0 })
                .unwrap()
        );
        assert_eq!(drawn_scroll_y(&context, node), end - 10.0);
    }
}

#[cfg(test)]
mod find_scope_smart_select_tests {
    use super::*;
    use crate::text_editing::TextSearchOptions;
    use crate::{AppContext, DocumentId, Entity, StableNodeId, TextArea};

    fn editor(value: &str) -> (AppContext, DocumentId, Entity<TextArea>, StableNodeId) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new(value).code_editor(true))
            .unwrap();
        let node = area.stable_id();
        context.focus_node(document, node).unwrap();
        (context, document, area, node)
    }

    fn set_selection(
        context: &mut AppContext,
        area: Entity<TextArea>,
        selection: TextSelection,
        additional: Vec<TextSelection>,
    ) {
        context
            .update_component(area, |area, _| {
                area.state.selection = selection;
                area.state.additional_selections = additional;
                true
            })
            .unwrap();
    }

    fn selection_span(context: &AppContext, node: StableNodeId) -> (usize, usize) {
        let state = context.world().text_input(node).unwrap();
        let span = state.selection.ordered();
        (span.start, span.end)
    }

    #[test]
    fn find_in_selection_scopes_navigation_and_replace_all() {
        //            0123456789...
        let value = "ab cd ab cd ab";
        let (mut context, document, area, node) = editor(value);
        let options = TextSearchOptions::default();

        // 无选区时 Selection 退化为全局（与 Document 一致）。
        set_selection(&mut context, area, TextSelection::caret(0), Vec::new());
        assert!(
            context
                .find_next_focused_text_match(document, "ab", options, TextFindScope::Selection)
                .unwrap()
        );
        assert_eq!(selection_span(&context, node), (0, 2));

        // 选区 3..12（"cd ab cd"）：范围内只有一个 "ab"，导航反复循环在
        // 6..8，不越界到边界外或范围外的命中。
        set_selection(&mut context, area, TextSelection::new(3, 12), Vec::new());
        assert!(
            context
                .find_next_focused_text_match(document, "ab", options, TextFindScope::Selection)
                .unwrap()
        );
        assert_eq!(selection_span(&context, node), (6, 8));
        // 命中已是当前选区：无移动，返回 false 但选区保持。
        assert!(
            !context
                .find_next_focused_text_match(document, "ab", options, TextFindScope::Selection)
                .unwrap()
        );
        assert_eq!(selection_span(&context, node), (6, 8));
        // 范围内唯一的命中已被选中：再次导航原地不动（返回 false），循环
        // 不越界；上一个同样停在同一命中。
        assert!(
            !context
                .find_next_focused_text_match(document, "ab", options, TextFindScope::Selection)
                .unwrap()
        );
        assert_eq!(selection_span(&context, node), (6, 8));
        assert!(
            !context
                .find_previous_focused_text_match(document, "ab", options, TextFindScope::Selection)
                .unwrap()
        );
        assert_eq!(selection_span(&context, node), (6, 8));

        // Document 范围不受限：从 6..8 起下一个命中是 12..14。
        assert!(
            context
                .find_next_focused_text_match(document, "ab", options, TextFindScope::Document)
                .unwrap()
        );
        assert_eq!(selection_span(&context, node), (12, 14));

        // 替换全部限定在选区内：只有范围内的两个 "cd" 被替换。
        set_selection(&mut context, area, TextSelection::new(3, 12), Vec::new());
        let replaced = context
            .replace_all_focused_text_matches(
                document,
                "cd",
                options,
                "X",
                TextFindScope::Selection,
                false,
            )
            .unwrap();
        assert_eq!(replaced, 2);
        assert_eq!(
            context.world().text_input(node).unwrap().value,
            "ab X ab X ab"
        );
    }

    #[test]
    fn replace_all_preserves_case_shapes() {
        let (mut context, document, _area, node) = editor("foo FOO Foo foo");
        let replaced = context
            .replace_all_focused_text_matches(
                document,
                "foo",
                TextSearchOptions::default(),
                "bar",
                TextFindScope::Document,
                true,
            )
            .unwrap();
        assert_eq!(replaced, 4);
        let state = context.world().text_input(node).unwrap();
        assert_eq!(state.value, "bar BAR Bar bar");
        // 第一个替换文本被选中。
        assert_eq!(selection_span(&context, node), (0, 3));
    }

    #[test]
    fn expand_and_shrink_selection_layers() {
        let value = "fn a() { greeting }";
        let (mut context, document, area, node) = editor(value);
        set_selection(&mut context, area, TextSelection::caret(13), Vec::new());

        // 词 → 括号内部 → 全文档。
        assert!(context.expand_focused_text_selection(document).unwrap());
        assert_eq!(selection_span(&context, node), (9, 17));
        assert!(context.expand_focused_text_selection(document).unwrap());
        assert_eq!(selection_span(&context, node), (8, 18));
        assert!(context.expand_focused_text_selection(document).unwrap());
        assert_eq!(selection_span(&context, node), (0, value.len()));
        // 已在全文档：不能再扩展。
        assert!(!context.expand_focused_text_selection(document).unwrap());

        // 收缩按扩展历史逐层回退，最后无可收缩。
        assert!(context.shrink_focused_text_selection(document).unwrap());
        assert_eq!(selection_span(&context, node), (8, 18));
        assert!(context.shrink_focused_text_selection(document).unwrap());
        assert_eq!(selection_span(&context, node), (9, 17));
        assert!(context.shrink_focused_text_selection(document).unwrap());
        assert_eq!(selection_span(&context, node), (13, 13));
        assert!(!context.shrink_focused_text_selection(document).unwrap());

        // 编辑后历史失效：扩展一次再改值，收缩拒绝。
        assert!(context.expand_focused_text_selection(document).unwrap());
        context
            .update_component(area, |area, _| {
                area.state.value = "fn a() { greeting!! }".into();
                true
            })
            .unwrap();
        assert!(!context.shrink_focused_text_selection(document).unwrap());
    }

    #[test]
    fn expand_selection_is_independent_per_cursor() {
        // "(aa bb) x (cc dd)"：两个光标分别落在两个括号组的词里。
        let value = "(aa bb) x (cc dd)";
        let (mut context, document, area, node) = editor(value);
        set_selection(
            &mut context,
            area,
            TextSelection::caret(1),
            vec![TextSelection::caret(14)],
        );

        let spans = |context: &AppContext, node: StableNodeId| -> Vec<(usize, usize)> {
            let state = context.world().text_input(node).unwrap();
            state
                .selections()
                .iter()
                .map(|s| (s.ordered().start, s.ordered().end))
                .collect()
        };

        // 第一次扩展：各自选中自己的词。
        assert!(context.expand_focused_text_selection(document).unwrap());
        assert_eq!(spans(&context, node), vec![(1, 3), (14, 16)]);
        // 第二次扩展：各自落到自己所在括号对的内部。
        assert!(context.expand_focused_text_selection(document).unwrap());
        assert_eq!(spans(&context, node), vec![(1, 6), (11, 16)]);
        // 第三次扩展：都到全文档，融合为一个选区。
        assert!(context.expand_focused_text_selection(document).unwrap());
        assert_eq!(spans(&context, node), vec![(0, value.len())]);
        // 收缩逐层回退：先回到括号内部双光标，再回词，再回光标。
        assert!(context.shrink_focused_text_selection(document).unwrap());
        assert_eq!(spans(&context, node), vec![(1, 6), (11, 16)]);
        assert!(context.shrink_focused_text_selection(document).unwrap());
        assert_eq!(spans(&context, node), vec![(1, 3), (14, 16)]);
        assert!(context.shrink_focused_text_selection(document).unwrap());
        // 历史恢复的是扩展前的双光标集合。
        let state = context.world().text_input(node).unwrap();
        assert_eq!(state.additional_selections, vec![TextSelection::caret(14)]);
        assert_eq!(selection_span(&context, node), (1, 1));
        assert!(!context.shrink_focused_text_selection(document).unwrap());
    }
}

#[cfg(test)]
mod atom_tests {
    use super::*;
    use crate::text_editing::TextCaretIntent;
    use crate::{AppContext, DocumentId, TextAtomSpan};
    use std::sync::Arc;

    fn focused_editor(value: &str) -> (AppContext, DocumentId, Entity<TextArea>, StableNodeId) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new(value))
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
    fn delete_and_caret_treat_atom_spans_as_one_unit() {
        let value = "ab[chip]cd";
        let atom = TextAtomSpan::new(2, 8);
        let (mut context, document, area, node) = focused_editor(value);
        context
            .update_component(area, |area, _| {
                area.atom_spans = Arc::from([atom.clone()]);
                area.state.selection = TextSelection::caret(8);
            })
            .unwrap();
        assert!(
            context
                .delete_focused_text(document, TextDeleteKind::Backward)
                .unwrap()
        );
        assert_eq!(context.world().text_input(node).unwrap().value, "abcd");
        assert_eq!(selection_of(&context, node), (2, 2));

        context
            .update_component(area, |area, _| {
                area.state.replace_value(value);
                area.atom_spans = Arc::from([atom.clone()]);
                area.state.selection = TextSelection::caret(2);
            })
            .unwrap();
        assert!(
            context
                .move_focused_text_caret(document, TextCaretIntent::Right, false, None)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (8, 8));
        assert!(
            context
                .move_focused_text_caret(document, TextCaretIntent::Left, false, None)
                .unwrap()
        );
        assert_eq!(selection_of(&context, node), (2, 2));

        context
            .update_component(area, |area, _| {
                area.state.selection = TextSelection::caret(5);
            })
            .unwrap();
        assert!(context.replace_focused_text(document, "x").unwrap());
        assert_eq!(context.world().text_input(node).unwrap().value, "abxcd");
    }

    #[test]
    fn cutting_into_an_atom_puts_on_the_pasteboard_what_it_deletes() {
        let value = "Hi [bob]!";
        let (mut context, document, area, node) = focused_editor(value);
        context
            .update_component(area, |area, _| {
                area.atom_spans = Arc::from([TextAtomSpan::new(3, 8)]);
                area.state.selection = TextSelection::new(0, 6);
            })
            .unwrap();
        let cut = context.cut_focused_text(document).unwrap();
        assert_eq!(context.world().text_input(node).unwrap().value, "!");
        assert_eq!(
            cut.as_deref(),
            Some("Hi [bob]"),
            "the whole chip, as deleted"
        );
    }

    #[test]
    fn left_and_right_collapse_onto_a_selection_edge_that_touches_an_atom() {
        // A selection ending on a chip: collapsing lands on the selection's
        // edge, not where a caret stepping off the chip would.
        let value = "Hi [bob]";
        let atom = TextAtomSpan::new(3, 8);
        for (anchor, focus, intent, landing) in [
            (0, 8, TextCaretIntent::Left, 0),
            (8, 0, TextCaretIntent::Right, 8),
        ] {
            let (mut context, document, area, node) = focused_editor(value);
            context
                .update_component(area, |area, _| {
                    area.atom_spans = Arc::from([atom.clone()]);
                    area.state.selection = TextSelection::new(anchor, focus);
                })
                .unwrap();
            assert!(
                context
                    .move_focused_text_caret(document, intent, false, None)
                    .unwrap()
            );
            assert_eq!(
                selection_of(&context, node),
                (landing, landing),
                "{intent:?}"
            );
        }
    }

    #[test]
    fn close_focused_text_atom_deletes_the_labeled_unit() {
        let value = "ab[chip]cd";
        let atom = TextAtomSpan::new(2, 8).label("chip").token("file-1");
        let (mut context, document, area, node) = focused_editor(value);
        let closed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = closed.clone();
        context
            .on(area, move |_, event: &crate::TextAtomClosed, _| {
                sink.lock().unwrap().push(event.token.to_string());
            })
            .unwrap();
        context
            .update_component(area, |area, _| {
                area.atom_spans = Arc::from([atom]);
                area.state.selection = TextSelection::caret(8);
            })
            .unwrap();
        assert!(context.close_focused_text_atom(document, "file-1").unwrap());
        assert_eq!(context.world().text_input(node).unwrap().value, "abcd");
        assert_eq!(*closed.lock().unwrap(), vec!["file-1".to_string()]);
    }
}

#[cfg(test)]
mod read_only_tests {
    use super::*;
    use crate::text_editing::{TextCaretIntent, TextSearchOptions};
    fn editor(value: &str) -> (AppContext, DocumentId, Entity<TextArea>) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new(value).code_editor(true))
            .unwrap();
        context.focus_node(document, area.stable_id()).unwrap();
        (context, document, area)
    }
    #[test]
    fn read_only_text_area_keeps_focus_find_selection_copy_and_other_view_independence() {
        let (mut context, document, area) = editor("猫 and 猫");
        let other = context
            .create_component(document, TextArea::new("猫 and 猫").read_only(true))
            .unwrap();
        context
            .update_component(area, |area, _| area.read_only = true)
            .unwrap();
        assert_eq!(context.world().focused(document), Some(area.stable_id()));
        assert!(
            !context
                .world()
                .accessibility(area.stable_id())
                .unwrap()
                .editable
        );
        assert!(
            !context
                .world()
                .accessibility(area.stable_id())
                .unwrap()
                .disabled
        );
        context
            .move_focused_text_caret(document, TextCaretIntent::DocStart, false, None)
            .unwrap();
        assert!(
            context
                .find_next_focused_text_match(
                    document,
                    "猫",
                    TextSearchOptions::default(),
                    TextFindScope::Document
                )
                .unwrap()
        );
        assert_eq!(
            context.focused_selected_text(document).as_deref(),
            Some("猫")
        );
        assert_eq!(
            context.read(area, |area| area.state.selection).unwrap(),
            TextSelection::new(0, 3)
        );
        assert_eq!(
            context.read(other, |area| area.state.selection).unwrap(),
            TextSelection::caret("猫 and 猫".len())
        );
        assert!(
            context
                .find_previous_focused_text_match(
                    document,
                    "猫",
                    TextSearchOptions::default(),
                    TextFindScope::Document
                )
                .unwrap()
        );
        assert!(context.select_all_focused_text(document).unwrap());
        assert_eq!(
            context.focused_selected_text(document).as_deref(),
            Some("猫 and 猫")
        );
        assert!(!context.replace_text_area_selection(area, "new").unwrap());
        assert!(
            !context
                .delete_focused_text(document, TextDeleteKind::Backward)
                .unwrap()
        );
        assert!(!context.insert_focused_text_newline(document).unwrap());
        assert!(context.cut_focused_text(document).unwrap().is_none());
        assert!(
            !context
                .replace_focused_text_match(
                    document,
                    "猫",
                    TextSearchOptions::default(),
                    "dog",
                    false
                )
                .unwrap()
        );
        assert_eq!(
            context
                .replace_all_focused_text_matches(
                    document,
                    "猫",
                    TextSearchOptions::default(),
                    "dog",
                    TextFindScope::Document,
                    false
                )
                .unwrap(),
            0
        );
        assert_eq!(
            context.read(area, |area| area.state.value.clone()).unwrap(),
            "猫 and 猫"
        );
        context.focus_node(document, other.stable_id()).unwrap();
        assert_eq!(
            context.read(area, |area| area.state.selection).unwrap(),
            TextSelection::new(0, "猫 and 猫".len())
        );
    }
    #[test]
    fn entering_read_only_cancels_preedit_and_pending_text_drop_without_changing_value() {
        let (mut context, document, area) = editor("alpha beta");
        assert!(
            context
                .set_ime_preedit(document, "候選".into(), Some((0, 6)))
                .unwrap()
        );
        context.text_edit.text_selection_drag = Some(TextSelectionDrag {
            pointer_id: 1,
            node: area.stable_id(),
            kind: TextEditorKind::Area,
            press: (0.0, 0.0),
            source: (0, 5),
            active: true,
            copy: false,
            target: Some(10),
        });
        context
            .update_component(area, |area, _| area.read_only = true)
            .unwrap();
        assert!(context.world().ime(area.stable_id()).is_none());
        assert_eq!(context.world().focused(document), Some(area.stable_id()));
        assert!(
            !context
                .set_ime_preedit(document, "new".into(), None)
                .unwrap()
        );
        assert!(!context.commit_ime(document, "候選").unwrap());
        assert!(!context.delete_ime_surrounding(document, 1, 1).unwrap());
        assert!(
            !context
                .text_editor_selection_drop(document, 1, 100.0, 10.0, &mut crate::MeasureTextShaper)
                .unwrap()
        );
        assert!(context.text_edit.text_selection_drag.is_none());
        assert_eq!(
            context.read(area, |area| area.state.value.clone()).unwrap(),
            "alpha beta"
        );
        context
            .update_component(area, |area, _| area.read_only = false)
            .unwrap();
        assert!(context.commit_ime(document, "!").unwrap());
        assert_eq!(
            context.read(area, |area| area.state.value.clone()).unwrap(),
            "alpha beta!"
        );
    }
    #[test]
    fn disabled_text_area_still_loses_focus_and_rejects_selection_and_input() {
        let (mut context, document, area) = editor("alpha");
        context
            .update_component(area, |area, _| {
                area.read_only = true;
                area.disabled = true;
            })
            .unwrap();
        assert_ne!(context.world().focused(document), Some(area.stable_id()));
        context.focus_node(document, area.stable_id()).unwrap();
        assert_ne!(context.world().focused(document), Some(area.stable_id()));
        assert!(
            !context
                .find_next_focused_text_match(
                    document,
                    "alpha",
                    TextSearchOptions::default(),
                    TextFindScope::Document
                )
                .unwrap()
        );
        assert!(!context.select_all_focused_text(document).unwrap());
        assert!(!context.replace_text_area_selection(area, "new").unwrap());
        assert_eq!(
            context.read(area, |area| area.state.value.clone()).unwrap(),
            "alpha"
        );
    }
}
