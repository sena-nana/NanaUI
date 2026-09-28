use super::*;
use crate::{
    AppContext, Button, Checkbox, DocumentId, Entity, LayoutViewport, LengthSpec, RangeField,
    StableNodeId, Stack, Text, TextChanged, TextInput,
};

fn setup() -> (AppContext, DocumentId, StableNodeId) {
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let parent = cx.create_component(document, Stack::column(0.0)).unwrap();
    (cx, document, parent.stable_id())
}

fn node<C: crate::ComponentView>(cx: &AppContext, root: StableNodeId, path: &str) -> Entity<C> {
    let id = cx
        .resolve_assembly_path(root, path)
        .unwrap_or_else(|| panic!("no node at {path}"));
    Entity::from_stable_id(id)
}

fn text_of(cx: &AppContext, entity: Entity<Text>) -> String {
    cx.read(entity, |text| text.value.clone()).unwrap()
}

fn children(cx: &AppContext, id: StableNodeId) -> Vec<StableNodeId> {
    cx.world().node(id).unwrap().children.to_vec()
}

fn delta(before: ReactiveStats) -> ReactiveStats {
    let now = reactive_stats();
    ReactiveStats {
        signals: now.signals.wrapping_sub(before.signals),
        effects: now.effects.wrapping_sub(before.effects),
        scopes: now.scopes.wrapping_sub(before.scopes),
        signal_writes: now.signal_writes - before.signal_writes,
        effects_run: now.effects_run - before.effects_run,
        flushes: now.flushes - before.flushes,
        nodes_patched: now.nodes_patched - before.nodes_patched,
        commits: now.commits - before.commits,
    }
}

fn counter() -> impl IntoView {
    let count = signal(0u64);
    column(
        12.0,
        (
            crate::text!("计数 {count}").key("value"),
            button("加一")
                .key("inc")
                .on_activate(move || count.update(|c| *c += 1)),
        ),
    )
    .key("counter")
}

#[test]
fn a_click_updates_exactly_the_bound_node_in_one_commit() {
    let (mut cx, _, parent) = setup();
    let view = cx.mount_view(parent, counter).unwrap();
    let root = view.roots()[0];
    let value = node::<Text>(&cx, root, "value");
    let inc = node::<Button>(&cx, root, "inc");
    assert_eq!(text_of(&cx, value), "计数 0");

    let before = reactive_stats();
    cx.activate_button(inc).unwrap();
    cx.flush_reactive().unwrap();
    let delta = delta(before);
    assert_eq!(text_of(&cx, value), "计数 1");
    assert_eq!(delta.nodes_patched, 1);
    assert_eq!(delta.commits, 1);
    assert_eq!(delta.effects_run, 1);
}

#[test]
fn a_frame_flushes_writes_made_outside_input() {
    let (mut cx, _, parent) = setup();
    let count = std::cell::Cell::new(None);
    let view = cx
        .mount_view(parent, || {
            let value = signal(1);
            count.set(Some(value));
            text!("{value}").key("value")
        })
        .unwrap();
    let value = Entity::<Text>::from_stable_id(view.roots()[0]);
    count.get().unwrap().set(2);
    assert!(cx.has_pending_reactive());
    cx.take_system_work();
    assert_eq!(text_of(&cx, value), "2");
    assert!(!cx.has_pending_reactive());
}

#[test]
fn a_flush_with_nothing_written_runs_nothing() {
    let (mut cx, _, parent) = setup();
    cx.mount_view(parent, counter).unwrap();
    let before = reactive_stats();
    cx.flush_reactive().unwrap();
    cx.take_system_work();
    assert_eq!(delta(before).effects_run, 0);
}

#[test]
fn constant_props_create_no_effect() {
    let (mut cx, _, parent) = setup();
    let before = reactive_stats();
    cx.mount_view(parent, || {
        column(4.0, (text("静态"), button("按钮").disabled(true)))
    })
    .unwrap();
    let delta = delta(before);
    assert_eq!(delta.effects, 0);
    assert_eq!(delta.signals, 0);
}

#[test]
fn a_hundred_bound_nodes_commit_once() {
    let (mut cx, _, parent) = setup();
    let shared = std::cell::Cell::new(None);
    cx.mount_view(parent, || {
        let count = signal(0);
        shared.set(Some(count));
        column(
            0.0,
            (0..100)
                .map(move |_| text(count.map(|c| c.to_string())))
                .collect::<Vec<_>>(),
        )
    })
    .unwrap();
    let before = reactive_stats();
    shared.get().unwrap().set(7);
    cx.flush_reactive().unwrap();
    let delta = delta(before);
    assert_eq!(delta.nodes_patched, 100);
    assert_eq!(delta.commits, 1);
}

#[test]
fn three_bindings_on_one_node_stage_it_once() {
    let (mut cx, _, parent) = setup();
    let signals = std::cell::Cell::new(None);
    let view = cx
        .mount_view(parent, || {
            let label = signal(String::from("保存"));
            let disabled = signal(false);
            let loading = signal(false);
            signals.set(Some((label, disabled, loading)));
            button(label).disabled(disabled).loading(loading)
        })
        .unwrap();
    let (label, disabled, loading) = signals.get().unwrap();
    let before = reactive_stats();
    label.set("保存中".into());
    disabled.set(true);
    loading.set(true);
    cx.flush_reactive().unwrap();
    let delta = delta(before);
    assert_eq!(delta.effects_run, 1, "one merged effect per node");
    assert_eq!(delta.nodes_patched, 1);
    assert_eq!(delta.commits, 1);
    let button = Entity::<Button>::from_stable_id(view.roots()[0]);
    let state = cx
        .read(button, |b| (b.label.clone(), b.disabled, b.loading))
        .unwrap();
    assert_eq!(state, ("保存中".into(), true, true));
}

#[derive(Clone)]
struct Todo {
    id: u32,
    title: Signal<String>,
}

#[test]
fn a_keyed_list_keeps_row_identity_through_insert_remove_and_reorder() {
    let (mut cx, _, parent) = setup();
    let items = std::cell::Cell::new(None);
    let view = cx
        .mount_view(parent, || {
            let todos = signal(
                (1..=3)
                    .map(|id| Todo {
                        id,
                        title: signal(format!("任务 {id}")),
                    })
                    .collect::<Vec<_>>(),
            );
            items.set(Some(todos));
            each(todos, |todo| todo.id, |todo| text(todo.title))
        })
        .unwrap();
    let todos = items.get().unwrap();
    let list = view.roots()[0];
    let initial = children(&cx, list);
    assert_eq!(initial.len(), 3);

    // Reorder: same nodes, new order.
    todos.update(|list| list.reverse());
    cx.flush_reactive().unwrap();
    let reversed = children(&cx, list);
    assert_eq!(reversed, initial.iter().rev().copied().collect::<Vec<_>>());

    // Remove the middle row, insert a new one at the front.
    let fresh = view_scope_signal("任务 9");
    todos.update(|list| {
        list.remove(1);
        list.insert(
            0,
            Todo {
                id: 9,
                title: fresh,
            },
        );
    });
    cx.flush_reactive().unwrap();
    let after = children(&cx, list);
    assert_eq!(after.len(), 3);
    assert_eq!(&after[1..], &[reversed[0], reversed[2]]);
    assert!(
        !cx.world().contains(reversed[1]),
        "removed row is despawned"
    );
    assert_eq!(text_of(&cx, Entity::from_stable_id(after[0])), "任务 9");
}

/// A signal owned by no scope, like application state created at startup.
fn view_scope_signal(value: &str) -> Signal<String> {
    signal(value.to_owned())
}

#[test]
fn a_row_field_change_does_not_rerun_the_list() {
    let (mut cx, _, parent) = setup();
    let items = std::cell::Cell::new(None);
    let view = cx
        .mount_view(parent, || {
            let todos = signal(
                (1..=200)
                    .map(|id| Todo {
                        id,
                        title: signal(format!("任务 {id}")),
                    })
                    .collect::<Vec<_>>(),
            );
            items.set(Some(todos));
            each(todos, |todo| todo.id, |todo| text(todo.title))
        })
        .unwrap();
    let todos = items.get().unwrap();
    let before = reactive_stats();
    todos.with_untracked(|list| list[57].title.set("改过".into()));
    cx.flush_reactive().unwrap();
    let delta = delta(before);
    assert_eq!(delta.effects_run, 1, "only the row's text binding ran");
    assert_eq!(delta.nodes_patched, 1);
    let row = children(&cx, view.roots()[0])[57];
    assert_eq!(text_of(&cx, Entity::from_stable_id(row)), "改过");
}

#[test]
fn when_builds_and_drops_its_branch_and_disposes_its_scope() {
    let (mut cx, _, parent) = setup();
    let flag = std::cell::Cell::new(None);
    let view = cx
        .mount_view(parent, || {
            let open = signal(false);
            flag.set(Some(open));
            when(open, || {
                let local = signal(1);
                text!("打开 {local}")
            })
            .otherwise(|| text("关闭"))
        })
        .unwrap();
    let open = flag.get().unwrap();
    let block = view.roots()[0];
    let closed = children(&cx, block);
    assert_eq!(text_of(&cx, Entity::from_stable_id(closed[0])), "关闭");
    let baseline = reactive_stats();

    open.set(true);
    cx.flush_reactive().unwrap();
    let opened = children(&cx, block);
    assert_eq!(opened.len(), 1);
    assert!(!cx.world().contains(closed[0]));
    assert_eq!(text_of(&cx, Entity::from_stable_id(opened[0])), "打开 1");

    open.set(false);
    cx.flush_reactive().unwrap();
    assert!(!cx.world().contains(opened[0]));
    let back = reactive_stats();
    assert_eq!(
        (back.signals, back.effects, back.scopes),
        (baseline.signals, baseline.effects, baseline.scopes),
        "the dropped branch released its signal, effect and scope"
    );
}

#[test]
fn unmount_releases_every_signal_effect_scope_and_handler() {
    let (mut cx, _, parent) = setup();
    let handlers = cx.event_handler_count();
    let before = reactive_stats();
    let view = cx
        .mount_view(parent, || {
            let todos = signal(vec![1u32, 2, 3]);
            let open = signal(true);
            column(
                0.0,
                (
                    counter(),
                    each(todos, |id| *id, |id| text(format!("{id}"))),
                    when(open, || text("x")),
                ),
            )
        })
        .unwrap();
    assert!(cx.event_handler_count() > handlers);
    view.unmount(&mut cx).unwrap();
    let after = reactive_stats();
    assert_eq!(
        (after.signals, after.effects, after.scopes),
        (before.signals, before.effects, before.scopes)
    );
    assert_eq!(cx.event_handler_count(), handlers);
    assert!(children(&cx, parent).is_empty());
}

#[test]
fn despawning_a_mounted_root_disposes_the_view() {
    let (mut cx, _, parent) = setup();
    let before = reactive_stats();
    let view = cx.mount_view(parent, counter).unwrap();
    let mut despawn = crate::MutationQueue::new();
    despawn.despawn_subtree(view.roots()[0]);
    cx.commit_mutations(despawn).unwrap();
    let after = reactive_stats();
    assert_eq!(
        (after.signals, after.effects, after.scopes),
        (before.signals, before.effects, before.scopes)
    );
}

#[test]
fn dropping_the_context_releases_its_views() {
    let before = reactive_stats();
    {
        let (mut cx, _, parent) = setup();
        cx.mount_view(parent, counter).unwrap();
        cx.mount_view(parent, || {
            each(signal(vec![1u8, 2]), |n| *n, |n| text(format!("{n}")))
        })
        .unwrap();
    }
    let after = reactive_stats();
    assert_eq!(
        (after.signals, after.effects, after.scopes),
        (before.signals, before.effects, before.scopes)
    );
}

#[test]
fn models_write_back_through_their_signal() {
    let (mut cx, _, parent) = setup();
    let signals = std::cell::Cell::new(None);
    let view = cx
        .mount_view(parent, || {
            let volume = signal(0.25);
            let name = signal(String::new());
            let agreed = signal(false);
            signals.set(Some((volume, name, agreed)));
            column(
                0.0,
                (
                    slider(0.0, 1.0, 0.05).model(volume).key("volume"),
                    text_input().model(name).key("name"),
                    checkbox("同意").model(agreed).key("agreed"),
                ),
            )
        })
        .unwrap();
    let (volume, name, agreed) = signals.get().unwrap();
    let root = view.roots()[0];
    let range = node::<RangeField>(&cx, root, "volume");
    assert_eq!(cx.read(range, |r| r.value).unwrap(), 0.25);

    cx.set_range_value(range, 0.5).unwrap();
    cx.flush_reactive().unwrap();
    assert_eq!(volume.get_untracked(), 0.5);

    volume.set(0.75);
    cx.flush_reactive().unwrap();
    assert_eq!(cx.read(range, |r| r.value).unwrap(), 0.75);

    let input = node::<TextInput>(&cx, root, "name");
    cx.update_component(input, |field, cx| {
        field.state.replace_value("小明");
        cx.emit(TextChanged {
            value: field.state.value.clone(),
            selection: field.state.selection,
        });
    })
    .unwrap();
    let before = reactive_stats();
    cx.flush_reactive().unwrap();
    assert_eq!(name.get_untracked(), "小明");
    assert_eq!(
        delta(before).nodes_patched,
        0,
        "echoing the edit back leaves the field alone"
    );

    let check = node::<Checkbox>(&cx, root, "agreed");
    cx.toggle_checkbox(check).unwrap();
    cx.flush_reactive().unwrap();
    assert!(agreed.get_untracked());
}

#[test]
fn visible_takes_a_node_out_of_layout_without_dropping_it() {
    let (mut cx, document, parent) = setup();
    let flag = std::cell::Cell::new(None);
    let view = cx
        .mount_view(parent, || {
            let shown = signal(true);
            flag.set(Some(shown));
            column(
                0.0,
                (
                    widget(
                        Stack::column(0.0).with_layout(|l| l.height = Some(LengthSpec::Px(40.0))),
                    )
                    .visible(shown)
                    .key("first"),
                    widget(
                        Stack::column(0.0).with_layout(|l| l.height = Some(LengthSpec::Px(40.0))),
                    )
                    .key("second"),
                ),
            )
        })
        .unwrap();
    let root = view.roots()[0];
    let first = cx.resolve_assembly_path(root, "first").unwrap();
    let second = cx.resolve_assembly_path(root, "second").unwrap();
    let viewport = LayoutViewport::new(320.0, 240.0);
    cx.layout_document(document, viewport).unwrap();
    let top = cx.world().layout_box(first).unwrap().y;
    assert_eq!(cx.world().layout_box(second).unwrap().y, top + 40.0);

    flag.get().unwrap().set(false);
    cx.flush_reactive().unwrap();
    cx.layout_document(document, viewport).unwrap();
    assert!(cx.world().contains(first));
    assert_eq!(cx.world().layout_box(second).unwrap().y, top);
}

#[test]
fn computed_props_follow_their_inputs() {
    let (mut cx, _, parent) = setup();
    let source = std::cell::Cell::new(None);
    let view = cx
        .mount_view(parent, || {
            let items = signal(vec![1, 2, 3]);
            source.set(Some(items));
            let total = computed(move || items.with(|v| v.iter().sum::<i32>()));
            text!("合计 {total}")
        })
        .unwrap();
    let text = Entity::<Text>::from_stable_id(view.roots()[0]);
    assert_eq!(text_of(&cx, text), "合计 6");
    source.get().unwrap().update(|v| v.push(4));
    cx.flush_reactive().unwrap();
    assert_eq!(text_of(&cx, text), "合计 10");
}

#[test]
fn effects_that_requeue_each_other_stop_at_the_round_limit() {
    let (mut cx, _, parent) = setup();
    let view = cx
        .mount_view(parent, || {
            let ping = signal(0u64);
            watch_effect(move || {
                let value = ping.get();
                ping.set(value + 1);
            });
            text("x")
        })
        .unwrap();
    cx.flush_reactive().unwrap();
    assert!(
        !cx.has_pending_reactive(),
        "the rest of the queue was dropped"
    );
    view.unmount(&mut cx).unwrap();
}

#[cfg(feature = "reactive-trace")]
#[test]
fn why_updated_names_the_write_that_caused_a_patch() {
    let (mut cx, _, parent) = setup();
    let view = cx.mount_view(parent, counter).unwrap();
    let root = view.roots()[0];
    let value = node::<Text>(&cx, root, "value");
    let inc = node::<Button>(&cx, root, "inc");
    cx.activate_button(inc).unwrap();
    cx.flush_reactive().unwrap();
    let why = cx
        .why_updated(value.stable_id())
        .expect("value was patched");
    assert_eq!(why.bindings.len(), 1);
    assert_eq!(why.bindings[0].0, "Text.value");
    assert_eq!(why.causes.len(), 1);
    assert_eq!(why.causes[0].written_at.file(), file!());
    assert!(why.causes[0].signal_created.is_some());
}

#[test]
fn a_routed_click_flushes_its_bindings_and_invalidates_the_frame() {
    use nana_ui_input::PointerPhase;

    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let view = cx.mount_view_root(document, counter).unwrap();
    let root = view.roots()[0];
    let value = node::<Text>(&cx, root, "value");
    let inc = node::<Button>(&cx, root, "inc");
    cx.layout_document(document, LayoutViewport::new(320.0, 240.0))
        .unwrap();
    cx.take_system_work();
    cx.rebuild_hit_test(document);
    let target = cx.world().layout_box(inc.stable_id()).unwrap();
    let (x, y) = (
        target.x + target.width / 2.0,
        target.y + target.height / 2.0,
    );

    let mut input = crate::HeadlessInput::bind(&mut cx, document);
    input.pointer(&mut cx, PointerPhase::Down, x, y).unwrap();
    let up = input.pointer(&mut cx, PointerPhase::Up, x, y).unwrap();
    assert_eq!(text_of(&cx, value), "计数 1");
    assert!(up.invalidated_work);
    assert!(!cx.has_pending_reactive());
}

#[derive(Clone, PartialEq, Debug)]
struct Theme(&'static str);

#[test]
fn provided_values_reach_rows_and_branches_built_later() {
    let (mut cx, _, parent) = setup();
    let source = std::cell::Cell::new(None);
    let view = cx
        .mount_view(parent, || {
            provide(Theme("深色"));
            let rows = signal(vec![1u32]);
            let open = signal(false);
            source.set(Some((rows, open)));
            let label = |tag: &str| {
                let theme = use_context::<Theme>().map_or("无", |theme| theme.0);
                text(format!("{tag} {theme}"))
            };
            column(
                0.0,
                (
                    each(rows, |n| *n, move |n| label(&format!("行{n}"))),
                    when(open, move || {
                        provide(Theme("浅色"));
                        column(0.0, (label("分支"),))
                    }),
                ),
            )
        })
        .unwrap();
    let (rows, open) = source.get().unwrap();
    rows.update(|list| list.push(2));
    open.set(true);
    cx.flush_reactive().unwrap();
    let root = view.roots()[0];
    let [list, block] = children(&cx, root)[..] else {
        panic!("two blocks");
    };
    let texts: Vec<String> = children(&cx, list)
        .into_iter()
        .map(|id| text_of(&cx, Entity::from_stable_id(id)))
        .collect();
    assert_eq!(
        texts,
        ["行1 深色", "行2 深色"],
        "a row built later still sees the mount's value"
    );
    let branch = children(&cx, children(&cx, block)[0])[0];
    assert_eq!(
        text_of(&cx, Entity::from_stable_id(branch)),
        "分支 浅色",
        "the nearest provider wins"
    );
    assert_eq!(
        use_context::<Theme>(),
        None,
        "outside any scope nothing is provided"
    );
}
