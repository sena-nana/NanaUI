//! `view!` expands to the function API: a template and its hand-written
//! equivalent mount the same retained tree and update it the same way.
#![cfg(feature = "view-macro")]

use nana_ui_runtime::view::{
    IntoView, Signal, button, checkbox, column, each, row, signal, slider, text, text_input, when,
    widget,
};
use nana_ui_runtime::{
    AppContext, DocumentId, Entity, RangeChanged, StableNodeId, Stack, Text, view,
};

/// A mounted page's signals and its tree.
trait Probe {
    fn tree(&self) -> nana_ui_runtime::view::AnyView;
    fn poke(&self);
}

/// Every node under `root`: kind, text, children, in document order.
fn dump(cx: &AppContext, root: StableNodeId) -> String {
    fn walk(cx: &AppContext, id: StableNodeId, depth: usize, out: &mut String) {
        let node = cx.world().node(id).unwrap();
        out.push_str(&"  ".repeat(depth));
        out.push_str(&format!("{:?}", node.kind));
        if let Some(text) = cx.world().text(id) {
            out.push_str(&format!(" {text:?}"));
        }
        out.push('\n');
        for child in node.children.iter() {
            walk(cx, *child, depth + 1, out);
        }
    }
    let mut out = String::new();
    walk(cx, root, 0, &mut out);
    out
}

fn assert_same(template: &dyn Fn() -> Box<dyn Probe>, functions: &dyn Fn() -> Box<dyn Probe>) {
    let mut cx_t = AppContext::new();
    let mut cx_f = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let mut probes = Vec::new();
    let t = cx_t
        .mount_view_root(document, || {
            let probe = template();
            let tree = probe.tree();
            probes.push(probe);
            tree
        })
        .unwrap();
    let f = cx_f
        .mount_view_root(document, || {
            let probe = functions();
            let tree = probe.tree();
            probes.push(probe);
            tree
        })
        .unwrap();
    let before = dump(&cx_t, t.roots()[0]);
    assert_eq!(before, dump(&cx_f, f.roots()[0]));
    for probe in &probes {
        probe.poke();
    }
    cx_t.flush_reactive().unwrap();
    cx_f.flush_reactive().unwrap();
    let after = dump(&cx_t, t.roots()[0]);
    assert_eq!(after, dump(&cx_f, f.roots()[0]));
    assert_ne!(before, after, "poking changed something");
}

#[derive(Clone)]
struct Todo {
    id: u32,
    title: &'static str,
}

fn todo_row(todo: Todo, done: bool) -> impl IntoView {
    row(4.0, (text(todo.title), checkbox("完成").checked(done)))
}

struct Page {
    count: Signal<u64>,
    loading: Signal<bool>,
    mode: Signal<u8>,
    todos: Signal<Vec<Todo>>,
    volume: Signal<f64>,
    shown: Signal<bool>,
    template: bool,
}

impl Page {
    fn new(template: bool) -> Self {
        Self {
            count: signal(0),
            loading: signal(true),
            mode: signal(0),
            todos: signal(vec![
                Todo {
                    id: 1, title: "一"
                },
                Todo {
                    id: 2, title: "二"
                },
            ]),
            volume: signal(0.5),
            shown: signal(true),
            template,
        }
    }
}

impl Probe for Page {
    fn tree(&self) -> nana_ui_runtime::view::AnyView {
        let Page {
            count,
            loading,
            mode,
            todos,
            volume,
            shown,
            ..
        } = *self;
        if self.template {
            view! {
                <Column gap=12>
                    <Text>"计数 {count}"</Text>
                    <Button @activate={count.update(|c| *c += 1)} disabled={loading}>"加一"</Button>
                    <Text v-if={loading}>"加载中"</Text>
                    <Text v-else-if={mode.get() == 1}>"模式一"</Text>
                    <Text v-else>"完成"</Text>
                    <Slider min=0 max=1 step=0.05 label="音量" v-model={volume} on:RangeChanged={|_: &RangeChanged| {}} />
                    <TextInput label="名字" placeholder="名字" v-show={shown} key="name" />
                    <TodoRow v-for={t in todos} key={t.id} todo={t} done={false} />
                    <Widget of={Stack::row(2.0)}>
                        <Text>"a"</Text>
                        <Text>{format!("b{}", count.get())}</Text>
                    </Widget>
                    {text("尾")}
                </Column>
            }
            .into_any()
        } else {
            column(
                12.0,
                (
                    nana_ui_runtime::text!("计数 {count}"),
                    button("加一")
                        .on_activate(move || {
                            count.update(|c| *c += 1);
                        })
                        .disabled(loading),
                    when(loading, move || text("加载中")).otherwise(move || {
                        when(move || mode.get() == 1, move || text("模式一"))
                            .otherwise(move || text("完成"))
                    }),
                    slider(0.0, 1.0, 0.05)
                        .label("音量")
                        .model(volume)
                        .on::<RangeChanged>(|_: &RangeChanged| {}),
                    text_input()
                        .label("名字")
                        .placeholder("名字")
                        .visible(shown)
                        .key("name"),
                    each(todos, move |t: &Todo| t.id, move |t| todo_row(t, false)),
                    widget(Stack::row(2.0))
                        .children((text("a"), text(move || format!("b{}", count.get())))),
                    text("尾"),
                ),
            )
            .into_any()
        }
    }

    fn poke(&self) {
        self.count.set(3);
        self.loading.set(false);
        self.mode.set(1);
        self.todos.update(|list| {
            list.reverse();
            list.push(Todo {
                id: 3, title: "三"
            });
        });
        self.volume.set(0.25);
        self.shown.set(false);
    }
}

#[test]
fn a_template_mounts_and_updates_exactly_like_its_function_calls() {
    assert_same(&|| Box::new(Page::new(true)), &|| {
        Box::new(Page::new(false))
    });
}

#[test]
fn more_than_twelve_children_nest_into_tuples() {
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let view = cx
        .mount_view_root(document, || {
            view! {
                <Column>
                    "1" "2" "3" "4" "5" "6" "7" "8" "9" "10" "11" "12" "13" "14"
                </Column>
            }
        })
        .unwrap();
    let texts: Vec<String> = cx
        .world()
        .node(view.roots()[0])
        .unwrap()
        .children
        .iter()
        .map(|id| {
            cx.read(Entity::<Text>::from_stable_id(*id), |t| t.value.clone())
                .unwrap()
        })
        .collect();
    assert_eq!(texts, (1..=14).map(|n| n.to_string()).collect::<Vec<_>>());
}

#[derive(Clone)]
struct Item {
    id: u32,
    title: String,
}

fn item_row(item: Item, list: Signal<Vec<Item>>) -> impl IntoView {
    let id = item.id;
    view! {
        <Row gap=8>
            <Text>{item.title}</Text>
            <Button key="remove" @activate={list.update(|list| list.retain(|t| t.id != id))}>"删除"</Button>
        </Row>
    }
}

#[test]
fn a_template_page_adds_and_removes_rows_through_its_handlers() {
    use nana_ui_runtime::Button;

    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let state = std::cell::Cell::new(None);
    let view = cx
        .mount_view_root(document, || {
            let list: Signal<Vec<Item>> = signal(Vec::new());
            let next = signal(1u32);
            state.set(Some(list));
            let add = move || {
                let id = next.get_untracked();
                next.set(id + 1);
                list.update(|list| {
                    list.push(Item {
                        id,
                        title: format!("任务 {id}"),
                    })
                });
            };
            view! {
                <Column>
                    <Button key="add" @activate={add}>"添加"</Button>
                    <ItemRow v-for={item in list} key={item.id} item={item} list={list} />
                    <Text v-if={list.with(Vec::is_empty)}>"空"</Text>
                    <Text v-else>{format!("共 {} 项", list.with(Vec::len))}</Text>
                </Column>
            }
        })
        .unwrap();
    let root = view.roots()[0];
    let add = cx.resolve_assembly_entity::<Button>(root, "add").unwrap();
    let parts = |cx: &AppContext| cx.world().node(root).unwrap().children.to_vec();
    let text_of = |cx: &AppContext, id: StableNodeId| {
        cx.read(Entity::<Text>::from_stable_id(id), |t| t.value.clone())
            .unwrap()
    };
    let summary = |cx: &AppContext| {
        let block = parts(cx)[2];
        text_of(cx, cx.world().node(block).unwrap().children[0])
    };
    assert_eq!(summary(&cx), "空");
    cx.activate_button(add).unwrap();
    cx.activate_button(add).unwrap();
    cx.flush_reactive().unwrap();
    let rows = cx.world().node(parts(&cx)[1]).unwrap().children.to_vec();
    assert_eq!(rows.len(), 2);
    assert_eq!(summary(&cx), "共 2 项");
    let first_title = cx.world().node(rows[0]).unwrap().children[0];
    assert_eq!(text_of(&cx, first_title), "任务 1");
    let remove = cx
        .resolve_assembly_entity::<Button>(rows[0], "remove")
        .unwrap();
    cx.activate_button(remove).unwrap();
    cx.flush_reactive().unwrap();
    let rows_after = cx.world().node(parts(&cx)[1]).unwrap().children.to_vec();
    assert_eq!(rows_after, vec![rows[1]], "the other row kept its node");
    assert_eq!(summary(&cx), "共 1 项");
    assert_eq!(state.get().unwrap().with_untracked(Vec::len), 1);
}

#[test]
fn table_controls_take_their_attributes_in_templates() {
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let parent = cx.create_component(document, Stack::column(0.0)).unwrap();
    let flags = std::cell::Cell::new(None);
    let mounted = cx
        .mount_view(parent.stable_id(), || {
            let on = signal(true);
            let amount = signal(4.0f64);
            flags.set(Some(amount));
            view! {
                <Column gap=4>
                    <Switch v-model={on} key="switch">"通知"</Switch>
                    <NumberInput v-model={amount} label="数量" placeholder={"数量"} key="number" />
                    <Progress max=10 value={amount.get()} label="进度" key="progress" />
                    <Divider />
                </Column>
            }
        })
        .unwrap();
    let root = mounted.roots()[0];
    let amount = flags.get().unwrap();
    amount.set(6.0);
    cx.flush_reactive().unwrap();
    let progress = cx
        .resolve_assembly_entity::<nana_ui_runtime::Progress>(root, "progress")
        .unwrap();
    assert_eq!(
        cx.read(progress, |p| (p.value, p.max)).unwrap(),
        (6.0, 10.0)
    );
    let switch = cx
        .resolve_assembly_entity::<nana_ui_runtime::Switch>(root, "switch")
        .unwrap();
    assert_eq!(
        cx.read(switch, |s| (s.checked, s.label.clone())).unwrap(),
        (true, "通知".to_owned())
    );
}

#[test]
fn v_virtual_in_a_template_mounts_a_virtual_list() {
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let parent = cx.create_component(document, Stack::column(0.0)).unwrap();
    let mounted = cx
        .mount_view(parent.stable_id(), || {
            let rows: Signal<Vec<u32>> = signal((0..10_000).collect());
            view! {
                <Text v-for={n in rows} key={*n} v-virtual={20}>"行 {n}"</Text>
            }
        })
        .unwrap();
    let scroll = mounted.roots()[0];
    assert!(
        cx.read(
            Entity::<nana_ui_runtime::ScrollView>::from_stable_id(scroll),
            |_| ()
        )
        .is_ok(),
        "the rows live in a ScrollView"
    );
}

#[test]
fn a_virtual_element_in_a_template_sizes_its_scroll_area() {
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let parent = cx.create_component(document, Stack::column(0.0)).unwrap();
    let mounted = cx
        .mount_view(parent.stable_id(), || {
            let rows: Signal<Vec<u32>> = signal((0..10_000).collect());
            view! {
                <Virtual row_height=20 height=200 measured>
                    <Text v-for={n in rows} key={*n}>"行 {n}"</Text>
                </Virtual>
            }
        })
        .unwrap();
    let scroll = mounted.roots()[0];
    cx.layout_document(document, nana_ui_runtime::LayoutViewport::new(320.0, 600.0))
        .unwrap();
    assert_eq!(cx.world().layout_box(scroll).unwrap().height, 200.0);
}

mod store_derive {
    use nana_ui_runtime::view::{Store, StoreList, StorePath, reactive_stats, store, text};
    use nana_ui_runtime::{AppContext, DocumentId, Entity, Stack, Text};

    #[derive(Clone, Store)]
    struct Todo {
        id: u64,
        title: String,
    }

    #[derive(Store)]
    struct App {
        todos: Vec<Todo>,
        heading: String,
    }

    #[test]
    fn derived_accessors_track_each_field() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let parent = cx
            .create_component(document, Stack::column(0.0))
            .unwrap()
            .stable_id();
        let app = std::cell::Cell::new(None);
        let view = cx
            .mount_view(parent, || {
                let state = store(App {
                    todos: vec![
                        Todo {
                            id: 1,
                            title: "一".into(),
                        },
                        Todo {
                            id: 2,
                            title: "二".into(),
                        },
                    ],
                    heading: "标题".into(),
                });
                app.set(Some(state));
                state
                    .todos()
                    .keyed(|todo| todo.id)
                    .each(|todo| text(todo.title()))
            })
            .unwrap();
        let app = app.get().unwrap();
        let list = view.roots()[0];

        let before = reactive_stats();
        app.heading().set("新标题".into());
        app.todos()
            .keyed(|todo| todo.id)
            .at(&2)
            .title()
            .set("贰".into());
        cx.flush_reactive().unwrap();
        let after = reactive_stats();
        assert_eq!(after.effects_run - before.effects_run, 1);
        let rows = cx.world().node(list).unwrap().children.to_vec();
        let second: Entity<Text> = Entity::from_stable_id(rows[1]);
        assert_eq!(cx.read(second, |text| text.value.clone()).unwrap(), "贰");
        assert_eq!(app.heading().get_untracked(), "新标题");
        assert_eq!(app.todos().len(), 2);
    }
}

mod transition_block {
    use std::time::Duration;

    use nana_ui_runtime::view::{Transition, each, signal, text, when};
    use nana_ui_runtime::{AppContext, DocumentId, Stack, view};

    #[test]
    fn a_transition_element_is_the_function_api_with_a_transition() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let parent = cx
            .create_component(document, Stack::column(0.0))
            .unwrap()
            .stable_id();
        cx.advance_animations(Duration::from_secs(1));
        let flag = std::cell::Cell::new(None);
        let view = cx
            .mount_view(parent, || {
                let open = signal(true);
                let items = signal(vec![1u32, 2]);
                flag.set(Some(open));
                view! {
                    <Column>
                        <Transition name="fade" duration=200>
                            <Text v-if={open}>"开"</Text>
                            <Text v-else>"关"</Text>
                        </Transition>
                        <TransitionGroup move=200>
                            <Text v-for={n in items} key={*n}>{n.to_string()}</Text>
                        </TransitionGroup>
                    </Column>
                }
            })
            .unwrap();
        let column = view.roots()[0];
        let block = cx.world().node(column).unwrap().children[0];
        flag.get().unwrap().set(false);
        cx.flush_reactive().unwrap();
        assert_eq!(
            cx.world().node(block).unwrap().children.len(),
            2,
            "the old branch leaves while the new one enters"
        );
        cx.advance_animations(Duration::from_secs(2));
        assert_eq!(cx.world().node(block).unwrap().children.len(), 1);
        // The function API it stands for compiles to the same calls.
        let _ = || {
            when(signal(true), || text("开"))
                .otherwise(|| text("关"))
                .transition(Transition::fade(Duration::from_millis(200)));
            each(signal(vec![1u32]), |n| *n, |n| text(n.to_string())).transition(
                Transition::fade(Duration::from_millis(150)).moves(Duration::from_millis(200)),
            );
        };
    }
}

mod suspense_block {
    use nana_ui_runtime::view::{poll_tasks, resource, text};
    use nana_ui_runtime::{AppContext, DocumentId, Stack, view};

    #[test]
    fn a_suspense_element_shows_content_once_it_resolves() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let parent = cx
            .create_component(document, Stack::column(0.0))
            .unwrap()
            .stable_id();
        let view = cx
            .mount_view(parent, || {
                view! {
                    <Suspense fallback={text("加载中")}>
                        {{
                            let data = resource(|| (), |()| std::future::ready(7u32));
                            text(move || format!("{:?}", data.get()))
                        }}
                    </Suspense>
                }
            })
            .unwrap();
        poll_tasks();
        cx.take_system_work();
        let root = view.roots()[0];
        let children = cx.world().node(root).unwrap().children.to_vec();
        let fallback = cx.world().node(children[1]).unwrap().children.len();
        assert_eq!(fallback, 0, "resolved on the first poll");
    }
}

mod styles {
    use nana_ui_runtime::view::{IntoView, signal, text, widget};
    use nana_ui_runtime::{AppContext, DocumentId, LengthSpec, Stack, css, view};

    fn layout_of(cx: &AppContext, id: nana_ui_runtime::StableNodeId) -> nana_ui_core::LayoutStyle {
        (*cx.world().node_style(id).unwrap().layout).clone()
    }

    #[test]
    fn a_template_style_and_css_compile_at_build_time() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let parent = cx
            .create_component(document, Stack::column(0.0))
            .unwrap()
            .stable_id();
        let flag = std::cell::Cell::new(None);
        let view = cx
            .mount_view(parent, || {
                let open = signal(false);
                flag.set(Some(open));
                view! {
                    style = ".panel { padding: 6px; } .panel.open { opacity: 0.5; }";
                    <Column class="panel" class:open={open}>
                        {widget(Stack::row(0.0)).css(css!("margin: 3px; opacity: 0.25")).into_any()}
                        <Text>"x"</Text>
                    </Column>
                }
            })
            .unwrap();
        let panel = view.roots()[0];
        assert_eq!(layout_of(&cx, panel).padding, Some(LengthSpec::Px(6.0)));
        assert_eq!(layout_of(&cx, panel).opacity, None);
        flag.get().unwrap().set(true);
        cx.flush_reactive().unwrap();
        assert_eq!(layout_of(&cx, panel).opacity, Some(0.5));
        let inline = cx.world().node(panel).unwrap().children[0];
        assert_eq!(layout_of(&cx, inline).margin, Some(LengthSpec::Px(3.0)));
        assert_eq!(layout_of(&cx, inline).opacity, Some(0.25));
        let _ = text("");
    }
}
