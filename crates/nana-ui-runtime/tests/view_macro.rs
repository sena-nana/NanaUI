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
                    <Slider min=0 max=1 step=0.05 v-model={volume} on:RangeChanged={|_: &RangeChanged| {}} />
                    <TextInput placeholder="名字" v-show={shown} key="name" />
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
                        .model(volume)
                        .on::<RangeChanged>(|_: &RangeChanged| {}),
                    text_input().placeholder("名字").visible(shown).key("name"),
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
                    <NumberInput v-model={amount} placeholder={"数量"} key="number" />
                    <Progress max=10 value={amount.get()} key="progress" />
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
