//! Keyed lists whose keys collide on their 64-bit hash.

use std::hash::{Hash, Hasher};

use super::super::{checkbox, row, text};
use super::*;
use crate::view::reactive::{ReactiveStats, reactive_stats};
use crate::{AppContext, Checkbox, DocumentId, Entity, StableNodeId, Stack, Text};

/// A key whose every value hashes alike: `Eq` alone tells two apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Clash(u64);

impl Hash for Clash {
    fn hash<H: Hasher>(&self, state: &mut H) {
        0u8.hash(state);
    }
}

#[derive(Clone)]
struct Task {
    id: u64,
    title: String,
    done: bool,
}

#[derive(Clone)]
struct Board {
    tasks: Vec<Task>,
}

trait TaskFields: StorePath<Value = Task> {
    fn title(self) -> Subfield<Self, String> {
        Subfield::__new(self, 1, |task| &task.title, |task| &mut task.title)
    }
    fn done(self) -> Subfield<Self, bool> {
        Subfield::__new(self, 2, |task| &task.done, |task| &mut task.done)
    }
}
impl<P: StorePath<Value = Task>> TaskFields for P {}

trait BoardFields: StorePath<Value = Board> {
    fn tasks(self) -> Subfield<Self, Vec<Task>> {
        Subfield::__new(self, 0, |board| &board.tasks, |board| &mut board.tasks)
    }
}
impl<P: StorePath<Value = Board>> BoardFields for P {}

fn task(id: u64) -> Task {
    Task {
        id,
        title: format!("任务 {id}"),
        done: false,
    }
}

fn clash(task: &Task) -> Clash {
    Clash(task.id)
}

fn effects_run_since(before: ReactiveStats) -> u64 {
    reactive_stats().effects_run - before.effects_run
}

/// A list of `ids` keyed by [`Clash`], a row per task: its title and a
/// checkbox bound both ways to `done`.
fn mount(ids: &[u64]) -> (AppContext, Store<Board>, StableNodeId) {
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let parent = cx
        .create_component(document, Stack::column(0.0))
        .unwrap()
        .stable_id();
    let board = std::cell::Cell::new(None);
    cx.mount_view(parent, || {
        let state = store(Board {
            tasks: ids.iter().copied().map(task).collect(),
        });
        board.set(Some(state));
        state.tasks().keyed(clash).each(|task| {
            row().children((
                text(task.title()).key("title"),
                checkbox("").model(task.done()).key("done"),
            ))
        })
    })
    .unwrap();
    (cx, board.get().unwrap(), parent)
}

fn rows(cx: &AppContext, list: StableNodeId) -> Vec<StableNodeId> {
    cx.world().node(list).unwrap().children.to_vec()
}

/// Each row's title and whether its checkbox is checked, in order.
fn shown(cx: &AppContext, list: StableNodeId) -> Vec<(String, bool)> {
    rows(cx, list)
        .into_iter()
        .map(|row| {
            let title: Entity<Text> =
                Entity::from_stable_id(cx.resolve_assembly_path(row, "title").unwrap());
            let done: Entity<Checkbox> =
                Entity::from_stable_id(cx.resolve_assembly_path(row, "done").unwrap());
            (
                cx.read(title, |text| text.value.clone()).unwrap(),
                cx.read(done, |checkbox| checkbox.checked).unwrap(),
            )
        })
        .collect()
}

fn row_of(cx: &AppContext, list: StableNodeId, title: &str) -> StableNodeId {
    let at = shown(cx, list)
        .iter()
        .position(|(shown, _)| shown == title)
        .unwrap_or_else(|| panic!("no row {title}"));
    rows(cx, list)[at]
}

#[test]
fn colliding_keys_keep_rows_of_their_own_through_insert_reorder_and_remove() {
    let (mut cx, board, list) = mount(&[1, 2, 3]);
    let tasks = board.tasks().keyed(clash);
    assert_eq!(
        shown(&cx, list),
        [
            ("任务 1".into(), false),
            ("任务 2".into(), false),
            ("任务 3".into(), false),
        ],
        "three keys with one hash are three rows"
    );
    let ids: Vec<u64> = tasks.items().iter().map(Item::id).collect();
    assert_eq!(ids[0], key_hash(&Clash(1)), "the first key keeps its hash");
    assert_eq!(
        ids.iter().collect::<hashbrown::HashSet<_>>().len(),
        3,
        "and the others get ids of their own"
    );

    // State the row holds: a checkbox ticked through the row's own binding.
    let row_two = rows(&cx, list)[1];
    let done: Entity<Checkbox> =
        Entity::from_stable_id(cx.resolve_assembly_path(row_two, "done").unwrap());
    cx.toggle_checkbox(done).unwrap();
    cx.flush_reactive().unwrap();
    assert!(tasks.at(&Clash(2)).done().get_untracked());
    assert!(!tasks.at(&Clash(1)).done().get_untracked());

    // A colliding key inserted ahead of every row: no row changes hands.
    let before = rows(&cx, list);
    board.tasks().insert(0, task(4));
    cx.flush_reactive().unwrap();
    assert_eq!(&rows(&cx, list)[1..], &before[..]);
    assert_eq!(
        shown(&cx, list),
        [
            ("任务 4".into(), false),
            ("任务 1".into(), false),
            ("任务 2".into(), true),
            ("任务 3".into(), false),
        ]
    );

    // A write through one key reaches that key's row only.
    let stats = reactive_stats();
    tasks.at(&Clash(3)).title().set("第三".into());
    cx.flush_reactive().unwrap();
    assert_eq!(
        effects_run_since(stats),
        1,
        "only row 3's title binding ran"
    );
    assert_eq!(shown(&cx, list)[3], ("第三".into(), false));

    // Reordering moves the rows with their items.
    let by_title: Vec<_> = ["任务 4", "任务 1", "任务 2", "第三"]
        .iter()
        .map(|title| row_of(&cx, list, title))
        .collect();
    board.tasks().sort_by_key(|task| std::cmp::Reverse(task.id));
    cx.flush_reactive().unwrap();
    assert_eq!(
        rows(&cx, list),
        [by_title[0], by_title[3], by_title[2], by_title[1]]
    );
    assert_eq!(
        shown(&cx, list),
        [
            ("任务 4".into(), false),
            ("第三".into(), false),
            ("任务 2".into(), true),
            ("任务 1".into(), false),
        ]
    );

    // Removing the key that had the plain hash leaves the others where
    // they are, and a key added later gets a row of its own.
    board.tasks().retain(|task| task.id != 1);
    board.tasks().push(task(5));
    cx.flush_reactive().unwrap();
    assert!(!cx.world().contains(by_title[1]), "task 1's row is gone");
    assert_eq!(
        &rows(&cx, list)[..3],
        [by_title[0], by_title[3], by_title[2]]
    );
    assert_eq!(
        shown(&cx, list),
        [
            ("任务 4".into(), false),
            ("第三".into(), false),
            ("任务 2".into(), true),
            ("任务 5".into(), false),
        ]
    );
    assert!(tasks.at(&Clash(1)).try_with(|_| ()).is_none());
    assert_eq!(tasks.at(&Clash(5)).title().get_untracked(), "任务 5");

    // Replacing the whole list keeps the rows of the keys it still has.
    let kept = row_of(&cx, list, "任务 2");
    board.tasks().set(vec![task(6), task(2)]);
    cx.flush_reactive().unwrap();
    assert_eq!(rows(&cx, list)[1], kept);
    assert_eq!(
        shown(&cx, list),
        [("任务 6".into(), false), ("任务 2".into(), false)],
        "task 2 came back from the new value"
    );
}

#[test]
fn a_repeated_key_still_builds_its_first_item_only() {
    let (mut cx, board, list) = mount(&[1, 2]);
    board.tasks().push(Task {
        id: 1,
        title: "又一个 1".into(),
        done: true,
    });
    cx.flush_reactive().unwrap();
    assert_eq!(
        shown(&cx, list),
        [("任务 1".into(), false), ("任务 2".into(), false)]
    );
}

#[test]
fn a_handle_for_a_colliding_key_not_in_the_list_reaches_no_other_row() {
    let (mut cx, board, _) = mount(&[1, 2]);
    let tasks = board.tasks().keyed(clash);
    let waiting = tasks.at(&Clash(7));
    assert_eq!(waiting.try_with(|task| task.id), None);
    board.tasks().push(task(7));
    cx.flush_reactive().unwrap();
    assert_eq!(
        waiting.try_with(|task| task.id),
        Some(7),
        "the key, once added, is the one it reaches"
    );
    assert_eq!(tasks.at(&Clash(7)).id(), waiting.id());
    assert_eq!(tasks.at(&Clash(1)).title().get_untracked(), "任务 1");
    assert_eq!(tasks.at(&Clash(2)).title().get_untracked(), "任务 2");
}
