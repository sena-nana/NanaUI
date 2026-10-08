//! An action menu whose commands come from a keyed list: as many as the data
//! has, with activation, disabled state, spoken names and keyboard focus that
//! follow rows inserted and removed while the menu is open.

use std::sync::{Arc, Mutex};

use super::*;
use crate::view::{
    Signal, action_menu, action_menu_item, each, entity_ref, signal, widget, with_refs,
};
use crate::{ActionMenu, ActionMenuItem, Activate, Entity, LayoutViewport, Stack};

#[derive(Clone, PartialEq)]
struct Entry {
    id: u32,
    label: String,
    disabled: bool,
}

fn entry(id: u32) -> Entry {
    Entry {
        id,
        label: format!("预设 {id}"),
        disabled: false,
    }
}

struct Fixture {
    cx: AppContext,
    input: TestInput,
    doc: DocumentId,
    menu: Entity<ActionMenu>,
    manage: StableNodeId,
    entries: Signal<Vec<Entry>>,
    chosen: Arc<Mutex<Vec<u32>>>,
}

impl Fixture {
    fn new(count: u32) -> Self {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let root = cx.create_component(doc, Stack::column(0.0)).unwrap();
        let chosen = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&chosen);
        let list = std::cell::Cell::new(None);
        let (_, (menu, manage)) = cx
            .mount_view(root.stable_id(), || {
                let entries = signal((1..=count).map(entry).collect::<Vec<_>>());
                list.set(Some(entries));
                let menu = entity_ref::<ActionMenu>();
                let manage = entity_ref::<ActionMenuItem>();
                let view = action_menu("预设").entity_ref(menu).children((
                    each(
                        entries,
                        |entry: &Entry| entry.id,
                        move |entry: Entry| {
                            let heard = Arc::clone(&heard);
                            let id = entry.id;
                            // A row follows its data through bindings,
                            // not by being rebuilt.
                            action_menu_item(entry.label.clone())
                                .accessible_name(format!("切换到{}", entry.label))
                                .disabled(move || {
                                    entries.with(|all| {
                                        all.iter()
                                            .find(|entry| entry.id == id)
                                            .is_none_or(|entry| entry.disabled)
                                    })
                                })
                                .on(move |_: &Activate| heard.lock().unwrap().push(id))
                        },
                    ),
                    widget(ActionMenuItem::new("管理预设…")).entity_ref(manage),
                ));
                with_refs(view, (menu, manage))
            })
            .unwrap();
        let mut fixture = Self {
            cx,
            input: TestInput::default(),
            doc,
            menu,
            manage: manage.stable_id(),
            entries: list.get().unwrap(),
            chosen,
        };
        fixture.layout();
        fixture
    }

    fn layout(&mut self) {
        self.cx
            .layout_document(self.doc, LayoutViewport::new(800.0, 4000.0))
            .unwrap();
        self.cx.rebuild_hit_test(self.doc);
    }

    fn open(&mut self) {
        self.cx
            .update_component(self.menu, |menu, _| menu.popover.open = true)
            .unwrap();
        self.cx.focus_node(self.doc, self.menu.stable_id()).unwrap();
        self.layout();
    }

    fn is_open(&self) -> bool {
        self.cx.read(self.menu, |menu| menu.popover.open).unwrap()
    }

    fn items(&self) -> Vec<StableNodeId> {
        self.cx.action_menu_items(self.menu)
    }

    fn focused(&self) -> Option<StableNodeId> {
        self.cx.world().focused(self.doc)
    }

    fn label(&self, id: StableNodeId) -> String {
        self.cx
            .read(Entity::<ActionMenuItem>::from_stable_id(id), |item| {
                item.label.to_string()
            })
            .unwrap()
    }

    fn key(&mut self, key: &str) {
        self.input
            .dispatch(
                &mut self.cx,
                self.doc,
                &key_fixture! {
                    pressed: true,
                    key: key.into(),
                    code: key.into(),
                    text: None,
                    repeat: false,
                    modifiers: InputModifiers::default(),
                },
            )
            .unwrap();
        self.layout();
    }

    fn update(&mut self, change: impl FnOnce(&mut Vec<Entry>)) {
        self.entries.update(change);
        self.cx.flush_reactive().unwrap();
        self.layout();
    }
}

#[test]
fn every_row_of_a_long_list_is_a_command_spaced_like_the_fixed_ones() {
    let mut f = Fixture::new(40);
    f.open();
    let items = f.items();
    assert_eq!(items.len(), 41, "no cap on the rows a menu can hold");
    assert_eq!(
        *items.last().unwrap(),
        f.manage,
        "the fixed command keeps its place"
    );
    let boxes: Vec<_> = items
        .iter()
        .map(|item| f.cx.world().layout_box(*item).unwrap())
        .collect();
    for pair in boxes.windows(2) {
        let gap = pair[1].y - (pair[0].y + pair[0].height);
        assert!(
            (gap - crate::popover::MENU_ITEM_GAP).abs() < 0.01,
            "rows and the fixed command are spaced as menu items: {gap}"
        );
        assert!(pair[0].width > 0.0 && (pair[0].width - pair[1].width).abs() < 0.01);
    }
    // Spoken names come from each row's own binding.
    assert_eq!(
        f.cx.world()
            .accessibility(items[6])
            .and_then(|state| state.label.clone())
            .as_deref(),
        Some("切换到预设 7")
    );

    // A command from deep in the list runs and closes the menu.
    assert!(f.cx.activate_node(items[35]).unwrap());
    assert_eq!(*f.chosen.lock().unwrap(), vec![36]);
    assert!(!f.is_open());
}

#[test]
fn rows_inserted_and_removed_while_open_keep_focus_and_keys_on_real_commands() {
    let mut f = Fixture::new(3);
    f.open();
    f.key("ArrowDown");
    assert_eq!(f.focused(), Some(f.items()[0]));
    f.key("ArrowDown");
    let second = f.items()[1];
    assert_eq!(f.focused(), Some(second));

    // A row arrives above the focused one while the menu is open: focus stays
    // on the same command, and the keys walk the new order.
    f.update(|entries| entries.insert(0, entry(9)));
    assert!(f.is_open());
    assert_eq!(f.focused(), Some(second));
    assert_eq!(f.items().len(), 5);
    f.key("ArrowUp");
    assert_eq!(f.label(f.focused().unwrap()), "预设 1");
    f.key("Home");
    assert_eq!(f.label(f.focused().unwrap()), "预设 9");
    f.key("ArrowUp");
    assert_eq!(f.focused(), Some(f.manage), "wraps to the end");

    // A disabled row is skipped by the keys and cannot run.
    f.update(|entries| entries[1].disabled = true);
    f.key("Home");
    f.key("ArrowDown");
    assert_eq!(
        f.label(f.focused().unwrap()),
        "预设 2",
        "skips the disabled 预设 1"
    );
    let disabled = f.items()[1];
    assert!(!f.cx.activate_node(disabled).unwrap());
    assert!(f.chosen.lock().unwrap().is_empty());

    // The focused row goes: the command now in its place takes focus.
    f.update(|entries| entries.retain(|entry| entry.id != 2));
    assert!(f.is_open());
    let focused = f.focused().expect("focus stays in the open menu");
    assert_eq!(f.label(focused), "预设 3");
    // The last row goes while focused: the one above it (the fixed command
    // stays reachable) takes over.
    f.key("ArrowDown");
    assert_eq!(f.focused(), Some(f.manage));
    f.update(|entries| entries.clear());
    assert_eq!(f.items(), vec![f.manage]);
    assert_eq!(f.focused(), Some(f.manage));
    let manage = f.cx.world().layout_box(f.manage).unwrap();
    assert!(
        manage.height > 0.0,
        "an empty list leaves the fixed command shown"
    );

    // Enter runs the focused command and closes the menu.
    f.update(|entries| entries.push(entry(5)));
    f.key("Home");
    f.key("Enter");
    assert_eq!(*f.chosen.lock().unwrap(), vec![5]);
    assert!(!f.is_open());
}
