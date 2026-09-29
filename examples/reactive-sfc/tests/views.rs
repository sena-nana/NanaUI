//! The compiled `.vue` views, driven headless.

use nana_ui::runtime::view::{
    AnyView, IntoView, Signal, button, column, reactive_stats, row, signal, text,
};
use nana_ui::runtime::{
    AppContext, Button, DocumentId, Entity, StableNodeId, Text, TextChanged, TextInput,
};
use reactive_sfc::views;

fn text_of(cx: &AppContext, id: StableNodeId) -> String {
    cx.read(Entity::<Text>::from_stable_id(id), |t| t.value.clone())
        .unwrap()
}

fn children(cx: &AppContext, id: StableNodeId) -> Vec<StableNodeId> {
    cx.world().node(id).unwrap().children.to_vec()
}

#[test]
fn the_counter_and_the_todo_list_work_as_written() {
    let before = reactive_stats();
    let mut cx = AppContext::typed();
    let document = DocumentId::new(1).unwrap();
    let page = cx.mount_view_root(document, views::app).unwrap().roots()[0];

    // Counter.vue
    let value = cx.resolve_assembly_path(page, "counter/value").unwrap();
    let inc = cx
        .resolve_assembly_entity::<Button>(page, "counter/inc")
        .unwrap();
    assert_eq!(text_of(&cx, value), "计数 0（双倍 0）");
    assert_eq!(cx.read(inc, |b| b.label.clone()).unwrap(), "加 1");
    cx.activate_button(inc).unwrap();
    cx.flush_reactive().unwrap();
    assert_eq!(text_of(&cx, value), "计数 1（双倍 2）");

    // TodoList.vue: type, add, remove.
    let draft = cx
        .resolve_assembly_entity::<TextInput>(page, "todo-section/todos/draft")
        .unwrap();
    let add = cx
        .resolve_assembly_entity::<Button>(page, "todo-section/todos/add")
        .unwrap();
    assert!(cx.read(add, |b| b.disabled).unwrap(), "empty draft");
    // Section.vue's `#header` slot, and TodoList.vue's `on_mount` focus.
    let section = cx.resolve_assembly_path(page, "todo-section").unwrap();
    assert_eq!(text_of(&cx, children(&cx, section)[0]), "待办");
    assert_eq!(cx.world().focused(document), Some(draft.stable_id()));
    let todos = cx
        .resolve_assembly_path(page, "todo-section/todos")
        .unwrap();
    let list = children(&cx, todos)[2];
    let summary = children(&cx, todos)[3];
    assert_eq!(text_of(&cx, children(&cx, summary)[0]), "还没有任务");
    for title in ["买菜", "写代码"] {
        cx.update_component(draft, |field, cx| {
            field.state.replace_value(title);
            cx.emit(TextChanged {
                value: field.state.value.clone(),
                selection: field.state.selection,
            });
        })
        .unwrap();
        cx.flush_reactive().unwrap();
        assert!(!cx.read(add, |b| b.disabled).unwrap());
        cx.activate_button(add).unwrap();
        cx.flush_reactive().unwrap();
    }
    let rows = children(&cx, list);
    assert_eq!(rows.len(), 2);
    assert_eq!(text_of(&cx, children(&cx, rows[0])[0]), "买菜");
    assert_eq!(text_of(&cx, children(&cx, summary)[0]), "共 2 项");
    assert!(
        cx.read(add, |b| b.disabled).unwrap(),
        "the draft was cleared"
    );
    let remove = cx
        .resolve_assembly_entity::<Button>(rows[0], "remove")
        .unwrap();
    cx.activate_button(remove).unwrap();
    cx.flush_reactive().unwrap();
    assert_eq!(
        children(&cx, list),
        vec![rows[1]],
        "the other row kept its node"
    );
    assert_eq!(text_of(&cx, children(&cx, summary)[0]), "共 1 项");

    // Every dependency the compiler declared matched what the bindings read.
    if cfg!(debug_assertions) {
        assert_eq!(
            reactive_stats().static_deps_mismatches - before.static_deps_mismatches,
            0
        );
    }
}

/// The same counter written with the function API and no analysis: the
/// button's `加 {step}` reads a live signal, so it needs an effect.
fn counter_by_hand() -> impl IntoView {
    let count = signal(0u64);
    let step = signal(1u64);
    let doubled = nana_ui::runtime::view::computed(move || count.get() * 2);
    row(
        8.0,
        (
            text(move || format!("计数 {count}（双倍 {doubled}）")),
            button(move || format!("加 {step}"))
                .on_activate(move || count.update(|c| *c += step.get())),
        ),
    )
}

#[test]
fn folding_saves_the_effects_constants_would_have_needed() {
    let effects_of = |view: fn() -> nana_ui::runtime::view::AnyView| {
        let mut cx = AppContext::typed();
        let before = reactive_stats().effects;
        let _mounted = cx
            .mount_view_root(DocumentId::new(1).unwrap(), view)
            .unwrap();
        reactive_stats().effects - before
    };
    let compiled = effects_of(|| views::counter().into_any());
    let by_hand = effects_of(|| counter_by_hand().into_any());
    assert_eq!((compiled, by_hand), (1, 2));
}

/// A debug build compiles its views in hot mode: static text swaps in the
/// running tree when the view's shape is unchanged, and a change beyond
/// the text is refused.
#[test]
fn static_text_is_swapped_in_the_running_tree() {
    let mut files: Vec<_> = std::fs::read_dir("views")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "vue"))
        .collect();
    files.sort();
    let sources: Vec<(String, String)> = files
        .iter()
        .map(|path| {
            (
                path.display().to_string(),
                std::fs::read_to_string(path).unwrap(),
            )
        })
        .collect();
    let compiler = nana_ui_sfc::Compiler::new("::nana_ui::runtime");
    let hot = compiler.hot_views(&sources).unwrap();
    let list = hot.iter().find(|view| view.name == "TodoList").unwrap();
    let at = list
        .literals
        .iter()
        .position(|text| text == "添加")
        .unwrap();

    let mut cx = AppContext::typed();
    let document = DocumentId::new(1).unwrap();
    let page = cx.mount_view_root(document, views::app).unwrap().roots()[0];
    let add = cx
        .resolve_assembly_entity::<Button>(page, "todo-section/todos/add")
        .unwrap();
    let mut literals = list.literals.clone();
    literals[at] = "新增".into();
    nana_ui::runtime::view::apply_hot_literals("TodoList", list.shape, literals).unwrap();
    cx.flush_reactive().unwrap();
    assert_eq!(cx.read(add, |b| b.label.clone()).unwrap(), "新增");

    // Editing more than text changes the shape.
    let edited: Vec<(String, String)> = sources
        .iter()
        .map(|(file, text)| {
            (
                file.clone(),
                text.replace("key=\"add\"", "key=\"add\" :disabled=\"true\""),
            )
        })
        .collect();
    let changed = compiler.hot_views(&edited).unwrap();
    let shape = changed
        .iter()
        .find(|view| view.name == "TodoList")
        .unwrap()
        .shape;
    assert_ne!(shape, list.shape);
    assert!(matches!(
        nana_ui::runtime::view::apply_hot_literals("TodoList", shape, list.literals.clone()),
        Err(nana_ui::runtime::view::HotReloadError::ShapeChanged(_))
    ));
}

/// `<style>` compiled at build time: static classes, a conditional class,
/// and a transition that animates the change.
#[test]
fn view_styles_apply_and_follow_their_classes() {
    use std::time::Duration;
    let start = Duration::from_secs(5);
    let mut cx = AppContext::typed();
    let document = DocumentId::new(1).unwrap();
    cx.advance_animations(start);
    let page = cx.mount_view_root(document, views::app).unwrap().roots()[0];
    let todos = cx
        .resolve_assembly_path(page, "todo-section/todos")
        .unwrap();
    let opacity = |cx: &AppContext| {
        cx.world()
            .node_style(todos)
            .and_then(|style| style.layout.opacity)
    };
    assert_eq!(
        opacity(&cx),
        Some(0.6),
        "`.todos.empty` while there is none"
    );

    let draft = cx
        .resolve_assembly_entity::<TextInput>(page, "todo-section/todos/draft")
        .unwrap();
    cx.update_component(draft, |field, cx| {
        field.state.replace_value("样式");
        cx.emit(TextChanged {
            value: field.state.value.clone(),
            selection: field.state.selection,
        });
    })
    .unwrap();
    cx.flush_reactive().unwrap();
    let add = cx
        .resolve_assembly_entity::<Button>(page, "todo-section/todos/add")
        .unwrap();
    cx.activate_button(add).unwrap();
    cx.flush_reactive().unwrap();
    assert_eq!(opacity(&cx), Some(1.0), "no longer empty");
    let shown = |at: u64| match cx.world().presentation_motion_value(
        todos,
        nana_ui::runtime::AnimatableProperty::Opacity,
        start + Duration::from_millis(at),
    ) {
        Some(nana_ui::runtime::MotionValue::Scalar(value)) => value,
        other => panic!("{other:?}"),
    };
    let halfway = shown(60);
    assert!(
        halfway > 0.6 && halfway < 1.0,
        "the change animates: {halfway}"
    );
    assert_eq!(shown(200), 1.0);

    // TodoItem.vue's rows take `.item`, their titles `.title`.
    let row = children(&cx, children(&cx, todos)[2])[0];
    let layout = &cx.world().node_style(row).unwrap().layout;
    assert_eq!(
        layout.padding_left,
        Some(nana_ui::runtime::LengthSpec::Px(8.0))
    );
    let title = children(&cx, row)[0];
    assert_eq!(
        cx.world().node_style(title).unwrap().layout.flex_grow,
        Some(1.0)
    );
}

/// Mount `view` and list its nodes in tree order: kind, text, layout.
fn tree(view: impl FnOnce() -> AnyView) -> Vec<String> {
    fn walk(cx: &AppContext, id: StableNodeId, out: &mut Vec<String>) {
        let node = cx.world().node(id).unwrap();
        // CSS `padding` also records which edges were declared physically;
        // `Stack::padding_xy` sets the same edges without that record.
        let mut layout = (*cx.world().node_style(id).unwrap().layout).clone();
        layout.logical_padding = Default::default();
        out.push(format!(
            "{:?} {:?} {layout:?}",
            node.kind,
            cx.world().text(id)
        ));
        for child in node.children {
            walk(cx, child, out);
        }
    }
    let mut cx = AppContext::typed();
    let root = cx
        .mount_view_root(DocumentId::new(1).unwrap(), view)
        .unwrap()
        .roots()[0];
    let mut out = Vec::new();
    walk(&cx, root, &mut out);
    out
}

/// `sfc-benchmark` compares the `bench/` views with functions written by
/// hand; its numbers mean something only if they build the same tree.
#[test]
fn the_benchmark_views_build_what_their_hand_written_twins_build() {
    use reactive_sfc::bench::{Item, hot, idiomatic, inline_css, naive, views as bench};

    let rows = |row: fn(usize) -> AnyView| {
        tree(move || column(0.0, (0..3).map(row).collect::<Vec<_>>()).into_any())
    };
    let compiled = rows(|i| bench::static_row(i).into_any());
    assert_eq!(compiled, rows(|i| idiomatic::static_row(i).into_any()));
    assert_eq!(compiled, rows(|i| naive::static_row(i).into_any()));

    // Row 1 carries the conditional class.
    let styled = |row: fn(usize, Signal<usize>) -> AnyView| {
        tree(move || {
            let selected = signal(1);
            column(0.0, (0..3).map(|i| row(i, selected)).collect::<Vec<_>>()).into_any()
        })
    };
    let compiled = styled(|i, s| bench::styled_row(i, s).into_any());
    assert!(
        compiled
            .iter()
            .any(|node| node.contains("opacity: Some(0.6)"))
    );
    assert_eq!(
        compiled,
        styled(|i, s| idiomatic::styled_row(i, s).into_any())
    );
    assert_eq!(
        compiled,
        styled(|i, s| inline_css::styled_row(i, s).into_any())
    );
    assert_eq!(compiled, styled(|i, s| hot::styled_row(i, s).into_any()));

    let list = |view: fn(Signal<Vec<Item>>) -> AnyView| {
        tree(move || {
            let items = (0..3)
                .map(|id| Item {
                    id,
                    title: signal(format!("任务 {id}")),
                })
                .collect();
            view(signal(items))
        })
    };
    let compiled = list(|l| bench::row_list(l).into_any());
    assert_eq!(compiled, list(|l| idiomatic::row_list(l).into_any()));
    assert_eq!(compiled, list(|l| hot::row_list(l).into_any()));
}
