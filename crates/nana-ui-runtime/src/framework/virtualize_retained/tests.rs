use super::*;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn project_layout(cx: &mut AppContext) {
    let work = cx.take_system_work();
    cx.resolve_styles(&work.style).unwrap();
    cx.layout_document(document(), crate::LayoutViewport::new(320.0, 100.0))
        .unwrap();
}

#[test]
fn retained_virtual_list_keeps_offscreen_editor_and_sparse_geometry() {
    let mut cx = AppContext::new();
    let list = cx.create_component(document(), List::new()).unwrap();
    let mut items = VirtualListItems::<usize, TextInput>::default();
    let layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 1_000_000));
    let materialize =
        |cx: &mut AppContext, items: &mut VirtualListItems<usize, TextInput>, offset| {
            let mut key_reads = 0;
            cx.materialize_virtual_list_retained_in(
                list,
                items,
                &layout,
                VirtualViewport::vertical(offset, 100.0, 0.0),
                &[],
                |index| {
                    key_reads += 1;
                    index
                },
                |key| Some(*key),
                |index, _| TextInput::new(format!("draft {index}")),
            )
            .unwrap();
            assert!(key_reads <= 8, "must not scan the million logical keys");
        };
    materialize(&mut cx, &mut items, 0.0);
    let editor = items.entity(&2).unwrap();
    cx.focus_node(document(), editor.id).unwrap();
    cx.set_ime_preedit(document(), "拼".into(), None).unwrap();
    materialize(&mut cx, &mut items, 10_000_000.0);
    assert_eq!(
        items.mounted_keys(),
        &[2, 500000, 500001, 500002, 500003, 500004]
    );
    assert_eq!(items.entity(&2), Some(editor));
    assert_eq!(cx.world().focused(document()), Some(editor.id));
    assert_eq!(cx.world().ime(editor.id).unwrap().text, "拼");
    assert_eq!(
        cx.read(editor, |field| field.state.value.clone()).unwrap(),
        "draft 2"
    );
    project_layout(&mut cx);
    assert_eq!(cx.world().layout_box(list.id).unwrap().height, 20_000_000.0);
    assert_eq!(
        cx.world().layout_box(items.containers[&2].id).unwrap().y,
        40.0
    );
    assert_eq!(
        cx.world()
            .layout_box(items.containers[&500000].id)
            .unwrap()
            .y,
        10_000_000.0
    );
    let generation = cx.world().generation();
    materialize(&mut cx, &mut items, 10_000_001.0);
    // This offset adds a partially visible row; return to the exact window,
    // then verify a repeated request performs no Runtime commit.
    materialize(&mut cx, &mut items, 10_000_000.0);
    let stable = cx.world().generation();
    assert!(stable >= generation);
    materialize(&mut cx, &mut items, 10_000_000.0);
    assert_eq!(cx.world().generation(), stable);
    cx.clear_ime(document()).unwrap();
    materialize(&mut cx, &mut items, 10_000_000.0);
    assert!(items.entity(&2).is_some(), "focus survives composition end");
    // Repeatedly traversing distant windows must release inactive rows rather
    // than accumulating one retained entity per visited logical item.
    let mut peak = 0;
    for step in 0..128 {
        materialize(&mut cx, &mut items, step as f32 * 20_000.0);
        peak = peak.max(items.mounted_keys().len());
    }
    assert!(
        peak <= 8,
        "virtual list retained {peak} rows after scrolling"
    );
    let mut mutations = MutationQueue::new();
    mutations.request_focus(document(), None);
    cx.commit_mutations(mutations).unwrap();
    materialize(&mut cx, &mut items, 10_000_000.0);
    assert_eq!(items.mounted_keys().len(), 5);
    assert!(!cx.world().contains(editor.id));
    assert!(cx.read(editor, |_| ()).is_err());
}

#[test]
fn retained_virtual_list_reorder_resize_and_delete_follow_business_keys() {
    let mut cx = AppContext::new();
    let list = cx.create_component(document(), List::new()).unwrap();
    let mut items = VirtualListItems::<usize, TextInput>::default();
    let mut layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 100));
    cx.materialize_virtual_list_retained_in(
        list,
        &mut items,
        &layout,
        VirtualViewport::vertical(0.0, 100.0, 0.0),
        &[],
        |index| index,
        |key| Some(*key),
        |_, _| TextInput::new("draft"),
    )
    .unwrap();
    let editor = items.entity(&2).unwrap();
    let container = items.containers[&2];
    cx.focus_node(document(), editor.id).unwrap();
    layout.update_item_extent(0, 40.0);
    cx.materialize_virtual_list_retained_in(
        list,
        &mut items,
        &layout,
        VirtualViewport::vertical(1000.0, 100.0, 0.0),
        &[],
        |index| {
            if index == 8 {
                2
            } else if index == 2 {
                8
            } else {
                index
            }
        },
        |key| Some(if *key == 2 { 8 } else { *key }),
        |_, _| TextInput::new("other"),
    )
    .unwrap();
    assert_eq!(items.entity(&2), Some(editor));
    assert_eq!(items.containers[&2], container);
    project_layout(&mut cx);
    assert_eq!(cx.world().layout_box(container.id).unwrap().y, 180.0);
    // Stale inverse points at a different key after deletion: never pin it.
    cx.materialize_virtual_list_retained_in(
        list,
        &mut items,
        &layout,
        VirtualViewport::vertical(1000.0, 100.0, 0.0),
        &[],
        |index| index + 100,
        |_| Some(8),
        |_, _| TextInput::new("replacement"),
    )
    .unwrap();
    assert!(!cx.world().contains(editor.id));
    assert!(!cx.world().contains(container.id));
    assert!(cx.world().focused(document()).is_none());
}

#[test]
fn retained_virtual_list_tracks_descendant_focus_and_cleans_descendant_views() {
    let mut cx = AppContext::new();
    let list = cx.create_component(document(), List::new()).unwrap();
    let mut items = VirtualListItems::<usize, Stack>::default();
    let layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 100));
    let fill = |cx: &mut AppContext, items: &mut VirtualListItems<usize, Stack>, offset| {
        cx.materialize_virtual_list_retained_in(
            list,
            items,
            &layout,
            VirtualViewport::vertical(offset, 100.0, 0.0),
            &[],
            |index| index,
            |key| Some(*key),
            |_, _| Stack::column(0.0),
        )
        .unwrap();
    };
    fill(&mut cx, &mut items, 0.0);
    let row = items.entity(&2).unwrap();
    let mut editor = None;
    cx.mount(row, |ui| {
        editor = Some(ui.child("editor", TextInput::new("nested"))?);
        Ok(())
    })
    .unwrap();
    let editor = editor.unwrap();
    cx.focus_node(document(), editor.id).unwrap();
    fill(&mut cx, &mut items, 1000.0);
    assert!(cx.world().contains(editor.id));
    let mut mutations = MutationQueue::new();
    mutations.request_focus(document(), None);
    cx.commit_mutations(mutations).unwrap();
    fill(&mut cx, &mut items, 1000.0);
    assert!(!cx.world().contains(editor.id));
    assert!(!cx.views.contains_key(&editor.id));
    assert_eq!(
        cx.views.len(),
        11,
        "five components and placement containers plus list"
    );
}

#[test]
fn retained_virtual_list_rejects_duplicate_keys_without_partial_publish() {
    let mut cx = AppContext::new();
    let list = cx.create_component(document(), List::new()).unwrap();
    let layout = VirtualListLayout::new([20.0; 20]);
    let mut items = VirtualListItems::<usize, TextInput>::default();
    cx.materialize_virtual_list_retained_in(
        list,
        &mut items,
        &layout,
        VirtualViewport::vertical(0.0, 40.0, 0.0),
        &[],
        |index| index,
        |key| Some(*key),
        |_, _| TextInput::new("draft"),
    )
    .unwrap();
    let before = cx.world().generation();
    let revision = items.materializer.revision();
    let entity = items.entity(&0);
    assert_eq!(
        cx.materialize_virtual_list_retained_in(
            list,
            &mut items,
            &layout,
            VirtualViewport::vertical(80.0, 100.0, 0.0),
            &[],
            |_| 99,
            |_| None,
            |_, _| panic!("invalid plans must not build")
        ),
        Err(FrameworkError::InvalidVirtualization)
    );
    assert_eq!(cx.world().generation(), before);
    assert_eq!(items.materializer.revision(), revision);
    assert_eq!(items.entity(&0), entity);
}

#[test]
fn retained_virtual_tree_collapse_releases_focused_descendants() {
    let mut cx = AppContext::new();
    let list = cx.create_component(document(), List::new()).unwrap();
    let mut items = VirtualTreeItems::<usize, TextInput>::default();
    let mut layout = VirtualTreeLayout::uniform(20.0, [2, 0, 0, 0, 0]);
    cx.materialize_virtual_tree_retained_in(
        list,
        &mut items,
        &layout,
        VirtualViewport::vertical(0.0, 60.0, 0.0),
        &[],
        |index| index,
        |key| Some(*key),
        |_, _| TextInput::new("tree"),
    )
    .unwrap();
    let editor = items.entity(&1).unwrap();
    cx.focus_node(document(), editor.id).unwrap();
    assert!(layout.collapse(0));
    cx.materialize_virtual_tree_retained_in(
        list,
        &mut items,
        &layout,
        VirtualViewport::vertical(0.0, 60.0, 0.0),
        &[],
        |index| [0, 3, 4][index],
        |key| [0, 3, 4].iter().position(|candidate| candidate == key),
        |_, _| TextInput::new("tree"),
    )
    .unwrap();
    assert_eq!(items.mounted_keys(), &[0, 3, 4]);
    assert!(!cx.world().contains(editor.id));
    assert!(cx.world().focused(document()).is_none());
}

#[test]
fn retained_virtual_list_failed_runtime_commit_keeps_the_previous_window() {
    #[derive(Clone, Debug, PartialEq)]
    struct Item {
        invalid: bool,
    }
    impl ComponentView for Item {
        fn node_kind(&self) -> NodeKind {
            TextInput::new("").node_kind()
        }
        fn project(&self, id: StableNodeId, world: &UiWorld, queue: &mut MutationQueue) {
            TextInput::new("draft").project(id, world, queue);
            if self.invalid {
                queue.insert(id, StableNodeId::new(999999).unwrap(), None);
            }
        }
    }
    let mut cx = AppContext::new();
    let list = cx.create_component(document(), List::new()).unwrap();
    let layout = VirtualListLayout::new([20.0; 20]);
    let mut items = VirtualListItems::<usize, Item>::default();
    cx.materialize_virtual_list_retained_in(
        list,
        &mut items,
        &layout,
        VirtualViewport::vertical(0.0, 40.0, 0.0),
        &[],
        |index| index,
        |key| Some(*key),
        |_, _| Item { invalid: false },
    )
    .unwrap();
    let generation = cx.world().generation();
    let revision = items.materializer.revision();
    let views = cx.views.len();
    let editor = items.entity(&0).unwrap();
    assert!(
        cx.materialize_virtual_list_retained_in(
            list,
            &mut items,
            &layout,
            VirtualViewport::vertical(200.0, 40.0, 0.0),
            &[],
            |index| index,
            |key| Some(*key),
            |_, _| Item { invalid: true }
        )
        .is_err()
    );
    assert_eq!(cx.world().generation(), generation);
    assert_eq!(items.materializer.revision(), revision);
    assert_eq!(cx.views.len(), views);
    assert_eq!(items.entity(&0), Some(editor));
    assert!(cx.world().contains(editor.id));
}

#[test]
fn retained_virtual_table_freezes_both_axes_and_preserves_nested_editor() {
    let mut cx = AppContext::new();
    let table = cx.create_component(document(), Table::new()).unwrap();
    let layout = VirtualTableLayout::new(
        std::iter::repeat_n(20.0, 1000000),
        (0..10000).map(|column| nana_ui_core::TableColumn::new(column.to_string(), 40.0)),
    );
    let mut items = VirtualTableItems::<usize, usize>::default();
    let fill = |cx: &mut AppContext, items: &mut VirtualTableItems<usize, usize>, offset| {
        cx.materialize_virtual_table_retained_in(
            table,
            items,
            &layout,
            VirtualViewport {
                offset,
                extent: [200.0, 100.0],
                overscan: [0.0; 2],
            },
            [1, 1],
            &[],
            |index| index,
            |key| Some(*key),
            |index| index,
            |key| Some(*key),
            |_, _| TableRow::new(),
            |row, _, column, _| TableCell::new(format!("{row}/{column}")),
        )
        .unwrap();
    };
    fill(&mut cx, &mut items, [0.0; 2]);
    let cell = items.cell_entity(&2, &2).unwrap();
    let mut editor = None;
    cx.mount(cell, |ui| {
        editor = Some(ui.child("editor", TextInput::new("draft"))?);
        Ok(())
    })
    .unwrap();
    let editor = editor.unwrap();
    cx.focus_node(document(), editor.id).unwrap();
    cx.set_ime_preedit(document(), "拼".into(), None).unwrap();
    fill(&mut cx, &mut items, [320000.0, 10000000.0]);
    assert_eq!(items.rows.len(), 6);
    assert_eq!(items.cells.len(), 36);
    assert_eq!(items.cell_entity(&2, &2), Some(cell));
    assert_eq!(cx.world().ime(editor.id).unwrap().text, "拼");
    project_layout(&mut cx);
    let header = items.row_entity(&0).unwrap();
    assert_eq!(
        cx.world()
            .node_style(header.id)
            .unwrap()
            .layout
            .transform
            .unwrap()
            .f,
        10000000.0
    );
    let frozen_column = items.cell_entity(&500001, &0).unwrap();
    assert_eq!(
        cx.world()
            .node_style(frozen_column.id)
            .unwrap()
            .layout
            .transform
            .unwrap()
            .e,
        320000.0
    );
    let generation = cx.world().generation();
    fill(&mut cx, &mut items, [320000.0, 10000000.0]);
    assert_eq!(
        cx.world().generation(),
        generation,
        "unchanged grid must not reorder children"
    );
    cx.focus_node(document(), items.cell_entity(&500001, &8001).unwrap().id)
        .unwrap();
    fill(&mut cx, &mut items, [320000.0, 10000000.0]);
    assert_eq!(items.cells.len(), 25);
    assert!(!cx.world().contains(editor.id));
    assert!(!cx.views.contains_key(&editor.id));
    assert!(!cx.event_dependencies.contains_key(&editor.id));
}
