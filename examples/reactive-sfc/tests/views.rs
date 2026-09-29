//! The compiled `.vue` views, driven headless.

use nana_ui::runtime::view::{IntoView, button, reactive_stats, row, signal, text};
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
