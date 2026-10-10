//! The keyboard's menu patterns: the context-menu request (`ContextMenu`,
//! Shift+F10, the arrows on a popup trigger) and roving focus groups.

use std::sync::{Arc, Mutex};

use super::*;
use crate::view::{column, entity_ref, list_item, row, signal, when, widget, with_refs};
use crate::{
    Entity, LayoutViewport, ListItem, ListItemRole, NumberInput, RovingEdge, RovingFocusEdge,
    RovingFocusGroup, SecondaryPress, Stack, View,
};

struct Fixture {
    cx: AppContext,
    input: TestInput,
    doc: DocumentId,
    root: StableNodeId,
}

impl Fixture {
    fn new() -> Self {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let root = cx
            .create_component(doc, Stack::column(0.0))
            .unwrap()
            .stable_id();
        Self {
            cx,
            input: TestInput::default(),
            doc,
            root,
        }
    }

    fn layout(&mut self) {
        self.cx
            .layout_document(self.doc, LayoutViewport::new(800.0, 600.0))
            .unwrap();
        self.cx.rebuild_hit_test(self.doc);
    }

    fn focus(&mut self, id: StableNodeId) {
        assert!(self.cx.focus_node(self.doc, id).unwrap());
    }

    fn focused(&self) -> Option<StableNodeId> {
        self.cx.world().focused(self.doc)
    }

    fn key_with(&mut self, key: &str, modifiers: InputModifiers, repeat: bool) -> bool {
        let disposition = self
            .input
            .dispatch(
                &mut self.cx,
                self.doc,
                &key_fixture! {
                    pressed: true,
                    key: key.into(),
                    code: key.into(),
                    text: None,
                    repeat,
                    modifiers,
                },
            )
            .unwrap();
        self.layout();
        disposition.prevent_default
    }

    fn key(&mut self, key: &str) -> bool {
        self.key_with(key, InputModifiers::default(), false)
    }

    fn shift(&mut self, key: &str) -> bool {
        self.key_with(
            key,
            InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
            false,
        )
    }

    fn presses<V: View>(&mut self, entity: Entity<V>) -> Arc<Mutex<Vec<SecondaryPress>>> {
        let presses = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&presses);
        self.cx
            .on(entity, move |_, press: &SecondaryPress, _| {
                heard.lock().unwrap().push(*press);
            })
            .unwrap();
        presses
    }

    fn edges<V: View>(&mut self, entity: Entity<V>) -> Arc<Mutex<Vec<RovingFocusEdge>>> {
        let edges = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&edges);
        self.cx
            .on(entity, move |_, edge: &RovingFocusEdge, _| {
                heard.lock().unwrap().push(*edge);
            })
            .unwrap();
        edges
    }
}

/// Four capsules, the third disabled, in a group `group` declares.
fn capsules(f: &mut Fixture, group: RovingFocusGroup) -> (Entity<Stack>, [Entity<ListItem>; 4]) {
    let (_, (column_ref, a, b, c, d)) =
        f.cx.mount_view(f.root, || {
            let group_ref = entity_ref::<Stack>();
            let items = [
                entity_ref::<ListItem>(),
                entity_ref::<ListItem>(),
                entity_ref::<ListItem>(),
                entity_ref::<ListItem>(),
            ];
            let capsule = |label: &'static str, item| {
                list_item(label).role(ListItemRole::Button).entity_ref(item)
            };
            let container = if group.orientation == crate::SelectionOrientation::Vertical {
                column()
            } else {
                row()
            };
            let view = container
                .roving_focus(group)
                .entity_ref(group_ref)
                .children((
                    capsule("Hiyori", items[0]),
                    capsule("Mao", items[1]),
                    capsule("Haru", items[2]).disabled(true),
                    capsule("Mark", items[3]),
                ));
            with_refs(view, (group_ref, items[0], items[1], items[2], items[3]))
        })
        .unwrap();
    f.layout();
    (column_ref, [a, b, c, d])
}

#[test]
fn the_context_menu_key_and_shift_f10_request_the_focused_nodes_menu() {
    let mut f = Fixture::new();
    let (_, (card, button)) =
        f.cx.mount_view(f.root, || {
            let card = entity_ref::<Stack>();
            let button = entity_ref::<crate::Button>();
            let view = column()
                .entity_ref(card)
                .children(crate::view::button("构建").entity_ref(button));
            with_refs(view, (card, button))
        })
        .unwrap();
    f.layout();
    // The card listens; the focused button inside it does not.
    let presses = f.presses(card);
    f.focus(button.stable_id());
    let bounds = f.cx.world().layout_box(button.stable_id()).unwrap();

    assert!(f.key("ContextMenu"));
    assert!(f.shift("F10"));
    let heard = presses.lock().unwrap().clone();
    assert_eq!(heard.len(), 2, "{heard:?}");
    for press in heard {
        assert!(press.keyboard);
        assert_eq!(press.focus, None, "the application picks the item");
        assert_eq!(press.target, button.stable_id());
        assert_eq!(
            (press.x, press.y),
            (
                bounds.x + bounds.width / 2.0,
                bounds.y + bounds.height / 2.0
            )
        );
    }

    // Plain F10, a held key's repeats and a modified key raise nothing.
    presses.lock().unwrap().clear();
    assert!(!f.key("F10"));
    assert!(!f.key_with("ContextMenu", InputModifiers::default(), true));
    assert!(!f.key_with(
        "ContextMenu",
        InputModifiers {
            control: true,
            ..InputModifiers::default()
        },
        false
    ));
    assert!(presses.lock().unwrap().is_empty());
    assert_eq!(f.focused(), Some(button.stable_id()), "focus stays");
}

#[test]
fn a_key_the_focused_control_takes_raises_no_menu() {
    let mut f = Fixture::new();
    let field =
        f.cx.mount_view(f.root, || {
            let field = entity_ref::<NumberInput>();
            let view = widget(NumberInput::new(1.0)).entity_ref(field);
            with_refs(view, field)
        })
        .unwrap()
        .1;
    f.layout();
    let presses = f.presses(field);
    // The application's key policy for this field claims the menu key.
    f.cx.on_key(field, |key| key.logical.0 == "ContextMenu")
        .unwrap();
    f.focus(field.stable_id());
    assert!(f.key("ContextMenu"));
    assert!(presses.lock().unwrap().is_empty(), "the field kept the key");
    // Shift+F10 it does not claim.
    assert!(f.shift("F10"));
    assert_eq!(presses.lock().unwrap().len(), 1);
}

#[test]
fn the_arrows_on_a_popup_trigger_request_its_menu() {
    let mut f = Fixture::new();
    let (_, (entry, plain)) =
        f.cx.mount_view(f.root, || {
            let entry = entity_ref::<ListItem>();
            let plain = entity_ref::<ListItem>();
            let view = row().children((
                list_item("模型")
                    .role(ListItemRole::ToggleButton)
                    .has_popup(true)
                    .entity_ref(entry),
                list_item("设置")
                    .role(ListItemRole::ToggleButton)
                    .entity_ref(plain),
            ));
            with_refs(view, (entry, plain))
        })
        .unwrap();
    f.layout();
    let entry_presses = f.presses(entry);
    let plain_presses = f.presses(plain);

    // Assistive technology hears a menu button.
    let world = f.cx.world();
    assert!(world.accessibility(entry.stable_id()).unwrap().has_popup);
    assert!(!world.accessibility(plain.stable_id()).unwrap().has_popup);
    let projected = world
        .project_accessibility_nodes(&[entry.stable_id()])
        .pop()
        .unwrap();
    assert!(projected.has_popup);

    f.focus(entry.stable_id());
    assert!(f.key("ArrowUp"));
    assert!(f.key("ArrowDown"));
    assert!(!f.key_with("ArrowUp", InputModifiers::default(), true));
    assert!(!f.shift("ArrowUp"));
    assert!(f.key("ContextMenu"));
    let heard = entry_presses.lock().unwrap().clone();
    assert_eq!(heard.len(), 3, "{heard:?}");
    assert!(heard.iter().all(|press| press.keyboard));
    // ArrowUp opens onto the last item, ArrowDown onto the first; the menu
    // key leaves it to the application.
    assert_eq!(
        heard.iter().map(|press| press.focus).collect::<Vec<_>>(),
        [Some(RovingEdge::End), Some(RovingEdge::Start), None]
    );

    // A control that does not open a menu keeps its arrows to itself.
    f.focus(plain.stable_id());
    assert!(!f.key("ArrowUp"));
    assert!(plain_presses.lock().unwrap().is_empty());
}

#[test]
fn the_arrows_walk_a_vertical_group_skipping_disabled_items() {
    let mut f = Fixture::new();
    let (group, [a, b, c, d]) = capsules(&mut f, RovingFocusGroup::vertical());
    assert_eq!(
        f.cx.roving_focus_group(group.stable_id()),
        Some(RovingFocusGroup::vertical())
    );
    let edges = f.edges(group);
    f.focus(a.stable_id());

    assert!(f.key("ArrowDown"));
    assert_eq!(f.focused(), Some(b.stable_id()));
    assert!(f.key("ArrowDown"));
    assert_eq!(
        f.focused(),
        Some(d.stable_id()),
        "the disabled one is skipped"
    );
    assert_ne!(f.focused(), Some(c.stable_id()));
    assert!(edges.lock().unwrap().is_empty());

    // Past the end: focus stays and the group hears it.
    assert!(f.key("ArrowDown"));
    assert_eq!(f.focused(), Some(d.stable_id()));
    assert_eq!(
        *edges.lock().unwrap(),
        [RovingFocusEdge {
            edge: RovingEdge::End,
            item: d.stable_id(),
        }]
    );

    assert!(f.key("Home"));
    assert_eq!(f.focused(), Some(a.stable_id()));
    assert!(f.key("ArrowUp"));
    assert_eq!(f.focused(), Some(a.stable_id()));
    assert_eq!(
        edges.lock().unwrap().last(),
        Some(&RovingFocusEdge {
            edge: RovingEdge::Start,
            item: a.stable_id(),
        })
    );
    assert!(f.key("End"));
    assert_eq!(f.focused(), Some(d.stable_id()));
    assert!(f.key("ArrowUp"));
    assert_eq!(f.focused(), Some(b.stable_id()));

    // The other axis and modified arrows are not the group's.
    assert!(!f.key("ArrowRight"));
    assert!(!f.shift("ArrowDown"));
    assert_eq!(f.focused(), Some(b.stable_id()));
    assert_eq!(edges.lock().unwrap().len(), 2);

    // Tab still visits every enabled item.
    f.key("Tab");
    assert_eq!(f.focused(), Some(d.stable_id()));
}

#[test]
fn a_wrapping_group_wraps_instead_of_reporting_an_edge() {
    let mut f = Fixture::new();
    let (group, [a, _, _, d]) = capsules(&mut f, RovingFocusGroup::vertical().wrap(true));
    let edges = f.edges(group);
    f.focus(d.stable_id());
    assert!(f.key("ArrowDown"));
    assert_eq!(f.focused(), Some(a.stable_id()));
    assert!(f.key("ArrowUp"));
    assert_eq!(f.focused(), Some(d.stable_id()));
    assert!(edges.lock().unwrap().is_empty());
}

#[test]
fn a_horizontal_group_walks_with_left_and_right() {
    let mut f = Fixture::new();
    let (group, [a, b, _, d]) = capsules(&mut f, RovingFocusGroup::horizontal());
    let edges = f.edges(group);
    f.focus(a.stable_id());
    assert!(!f.key("ArrowDown"));
    assert_eq!(f.focused(), Some(a.stable_id()));
    assert!(f.key("ArrowRight"));
    assert_eq!(f.focused(), Some(b.stable_id()));
    assert!(f.key("ArrowRight"));
    assert_eq!(f.focused(), Some(d.stable_id()));
    assert!(f.key("ArrowLeft"));
    assert_eq!(f.focused(), Some(b.stable_id()));
    assert!(f.key("ArrowLeft"));
    assert!(f.key("ArrowLeft"));
    assert_eq!(f.focused(), Some(a.stable_id()));
    assert_eq!(
        *edges.lock().unwrap(),
        [RovingFocusEdge {
            edge: RovingEdge::Start,
            item: a.stable_id(),
        }]
    );
}

#[test]
fn nested_groups_keep_their_own_items() {
    let mut f = Fixture::new();
    let (_, (outer, inner, a, b1, b2, c)) =
        f.cx.mount_view(f.root, || {
            let outer = entity_ref::<Stack>();
            let inner = entity_ref::<Stack>();
            let a = entity_ref::<ListItem>();
            let b1 = entity_ref::<ListItem>();
            let b2 = entity_ref::<ListItem>();
            let c = entity_ref::<ListItem>();
            let view = column()
                .roving_focus(RovingFocusGroup::vertical())
                .entity_ref(outer)
                .children((
                    list_item("A").entity_ref(a),
                    row()
                        .roving_focus(RovingFocusGroup::horizontal())
                        .entity_ref(inner)
                        .children((
                            list_item("B1").entity_ref(b1),
                            list_item("B2").entity_ref(b2),
                        )),
                    list_item("C").entity_ref(c),
                ));
            with_refs(view, (outer, inner, a, b1, b2, c))
        })
        .unwrap();
    f.layout();
    let outer_edges = f.edges(outer);
    let inner_edges = f.edges(inner);

    f.focus(a.stable_id());
    assert!(f.key("ArrowDown"));
    assert_eq!(f.focused(), Some(c.stable_id()), "B1 and B2 are the row's");

    f.focus(b1.stable_id());
    assert!(!f.key("ArrowDown"), "the row is horizontal");
    assert_eq!(f.focused(), Some(b1.stable_id()));
    assert!(f.key("ArrowRight"));
    assert_eq!(f.focused(), Some(b2.stable_id()));
    assert!(f.key("ArrowRight"));
    assert_eq!(f.focused(), Some(b2.stable_id()));
    assert_eq!(inner_edges.lock().unwrap().len(), 1);
    assert!(outer_edges.lock().unwrap().is_empty());
}

#[test]
fn a_field_in_a_group_keeps_its_own_arrows() {
    let mut f = Fixture::new();
    let (_, (field, after)) =
        f.cx.mount_view(f.root, || {
            let field = entity_ref::<NumberInput>();
            let after = entity_ref::<ListItem>();
            let view = column()
                .roving_focus(RovingFocusGroup::vertical())
                .children((
                    widget(NumberInput::new(1.0)).entity_ref(field),
                    list_item("下一项").entity_ref(after),
                ));
            with_refs(view, (field, after))
        })
        .unwrap();
    f.layout();
    f.focus(field.stable_id());
    assert!(f.key("ArrowDown"));
    assert_eq!(f.focused(), Some(field.stable_id()), "the field stepped");
    // From the next item the group walks back into the field.
    f.focus(after.stable_id());
    assert!(f.key("ArrowUp"));
    assert_eq!(f.focused(), Some(field.stable_id()));
}

#[test]
fn focus_roving_edge_focuses_the_first_or_last_item_the_arrows_reach() {
    let mut f = Fixture::new();
    let (group, [a, b, c, d]) = capsules(&mut f, RovingFocusGroup::vertical());
    assert_eq!(
        f.cx.focus_roving_edge(group.stable_id(), RovingEdge::End)
            .unwrap(),
        Some(d.stable_id())
    );
    assert_eq!(f.focused(), Some(d.stable_id()));
    assert_eq!(
        f.cx.focus_roving_edge(group.stable_id(), RovingEdge::Start)
            .unwrap(),
        Some(a.stable_id())
    );
    assert_eq!(f.focused(), Some(a.stable_id()));
    // Already there: it stays and is reported.
    assert_eq!(
        f.cx.focus_roving_edge(group.stable_id(), RovingEdge::Start)
            .unwrap(),
        Some(a.stable_id())
    );

    // An item, or a container that declares no group, is not a group.
    assert_eq!(
        f.cx.focus_roving_edge(b.stable_id(), RovingEdge::Start)
            .unwrap(),
        None
    );
    assert_eq!(
        f.cx.focus_roving_edge(f.root, RovingEdge::Start).unwrap(),
        None
    );
    assert_eq!(f.focused(), Some(a.stable_id()));
    assert_ne!(f.focused(), Some(c.stable_id()));
}

#[test]
fn focus_roving_edge_sees_items_shown_in_the_same_turn() {
    let mut f = Fixture::new();
    let handles = std::cell::Cell::new(None);
    let (_, group) =
        f.cx.mount_view(f.root, || {
            let open = signal(false);
            let locked = signal(true);
            let group = entity_ref::<Stack>();
            let items = [
                entity_ref::<ListItem>(),
                entity_ref::<ListItem>(),
                entity_ref::<ListItem>(),
                entity_ref::<ListItem>(),
            ];
            handles.set(Some((open, locked, items)));
            let view = column()
                .roving_focus(RovingFocusGroup::vertical())
                .entity_ref(group)
                .children(when(open, move || {
                    (
                        list_item("关闭").disabled(true).entity_ref(items[0]),
                        list_item("截图").entity_ref(items[1]),
                        list_item("录制").entity_ref(items[2]),
                        list_item("直播").disabled(locked).entity_ref(items[3]),
                    )
                }));
            with_refs(view, group)
        })
        .unwrap();
    f.layout();
    let (open, locked, items) = handles.take().unwrap();
    let group = group.stable_id();

    // Closed: the group has no items.
    assert_eq!(
        f.cx.focus_roving_edge(group, RovingEdge::Start).unwrap(),
        None
    );
    assert_eq!(f.focused(), None);

    // Shown by a signal write with no flush in between; the disabled ends
    // are skipped as the arrows skip them.
    open.set(true);
    let [first, second, third, fourth] = items;
    assert_eq!(
        f.cx.focus_roving_edge(group, RovingEdge::Start).unwrap(),
        second.get().map(|item| item.stable_id())
    );
    assert!(f.focused().is_some());
    assert_ne!(f.focused(), first.get().map(|item| item.stable_id()));
    assert_eq!(
        f.cx.focus_roving_edge(group, RovingEdge::End).unwrap(),
        third.get().map(|item| item.stable_id())
    );

    // Enabled the same way, the last one becomes the end.
    locked.set(false);
    let last = f.cx.focus_roving_edge(group, RovingEdge::End).unwrap();
    assert!(last.is_some());
    assert_eq!(last, fourth.get().map(|item| item.stable_id()));
    assert_eq!(f.focused(), last);
}
