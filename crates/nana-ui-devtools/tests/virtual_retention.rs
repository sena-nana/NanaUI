#![cfg(feature = "runtime-agent")]

use nana_ui::runtime::{
    DocumentId, List, MutationQueue, NodeStyle, RuntimeDocument, ScrollAxes, ScrollOffset,
    ScrollView, TextInput, VirtualListItems, VirtualListLayout,
};
use nana_ui_core::{LayoutStyle, LengthSpec, VirtualViewport};
use nana_ui_devtools::agent::RuntimeAgentSession;
use nana_ui_devtools::offscreen;
use std::sync::Arc;

#[test]
fn rust_virtual_list_retains_editor_and_matches_scroll_hit_geometry() {
    if !offscreen::pixels_available() {
        return;
    }
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    let cx = document.context_mut();
    let scroll = cx
        .create_component(
            id,
            ScrollView::new(ScrollAxes::Vertical).style(NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(320.0)),
                    height: Some(LengthSpec::Px(160.0)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
        .unwrap();
    let list = cx.create_component(id, List::new()).unwrap();
    let mut mutations = MutationQueue::new();
    mutations.insert(scroll.stable_id(), list.stable_id(), None);
    cx.commit_mutations(mutations).unwrap();
    let layout = VirtualListLayout::new(std::iter::repeat_n(32.0, 1_000_000));
    let mut items = VirtualListItems::<usize, TextInput>::default();
    cx.materialize_virtual_list_retained_in(
        list,
        &mut items,
        &layout,
        VirtualViewport::vertical(0.0, 160.0, 0.0),
        &[],
        |index| index,
        |key| Some(*key),
        |index, _| TextInput::new(format!("Draft {index}")),
    )
    .unwrap();
    let editor = items.entity(&2).unwrap();
    let mut session = RuntimeAgentSession::new(document, 360, 200).unwrap();
    assert_eq!(items.mounted_keys().len(), 5);
    let cx = session.document_mut().context_mut();
    cx.focus_node(id, editor.stable_id()).unwrap();
    cx.set_ime_preedit(id, "拼".into(), None).unwrap();
    cx.materialize_virtual_list_retained_in(
        list,
        &mut items,
        &layout,
        VirtualViewport::vertical(16_000_000.0, 160.0, 0.0),
        &[],
        |index| index,
        |key| Some(*key),
        |index, _| TextInput::new(format!("Draft {index}")),
    )
    .unwrap();
    session.flush().unwrap();
    session
        .document_mut()
        .context_mut()
        .scroll_to(
            scroll,
            ScrollOffset {
                x: 0.0,
                y: 16_000_000.0,
            },
        )
        .unwrap();
    session.flush().unwrap();
    assert_eq!(items.mounted_keys().len(), 6);
    assert_eq!(items.entity(&2), Some(editor));
    assert_eq!(
        session
            .document()
            .context()
            .world()
            .ime(editor.stable_id())
            .unwrap()
            .text,
        "拼"
    );
    let visible = items.entity(&500000).unwrap();
    let bounds = session
        .document()
        .scene()
        .draw_node_bounds(visible.stable_id())
        .unwrap();
    assert_eq!(bounds.y, 0.0);
    assert!(
        session
            .document()
            .scene()
            .draw_node_bounds(editor.stable_id())
            .unwrap()
            .y
            < 0.0
    );
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/virtual-rust");
    std::fs::create_dir_all(&output).unwrap();
    let retained_nodes = session.document().context().world().len();
    let scene_primitives = session.document().scene().primitive_count();
    let mut viewport = bounds;
    viewport.x = 0.0;
    viewport.y = 0.0;
    viewport.width = 360.0;
    viewport.height = 200.0;
    let visible_operations = session
        .document()
        .scene()
        .visible_operations(viewport)
        .unwrap()
        .len();
    session.screenshot_png(output.join("jumped.png")).unwrap();
    session.click_xy(bounds.x + 12.0, bounds.y + 12.0).unwrap();
    assert_eq!(
        session.document().context().world().focused(id),
        Some(visible.stable_id())
    );
    let cx = session.document_mut().context_mut();
    cx.materialize_virtual_list_retained_in(
        list,
        &mut items,
        &layout,
        VirtualViewport::vertical(16_000_000.0, 160.0, 0.0),
        &[],
        |index| index,
        |key| Some(*key),
        |index, _| TextInput::new(format!("Draft {index}")),
    )
    .unwrap();
    session.flush().unwrap();
    assert_eq!(items.mounted_keys().len(), 5);
    assert!(
        !session
            .document()
            .context()
            .world()
            .contains(editor.stable_id())
    );
    let report = serde_json::json!({
        "logical_items": 1000000,
        "mounted_before": 5, "mounted_active": 6, "mounted_after_release": 5,
        "retained_nodes_with_editor": retained_nodes,
        "scene_primitives_with_editor": scene_primitives,
        "conservative_visible_operations_with_editor": visible_operations,
        "gpu_screenshot_and_click": true, "refresh_rate_measured": false,
    });
    std::fs::write(
        output.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[test]
fn rust_virtual_table_frozen_regions_match_real_clicks() {
    if !offscreen::pixels_available() {
        return;
    }
    use nana_ui::runtime::{Table, TableCell, TableRow, VirtualTableItems, VirtualTableLayout};
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    let cx = document.context_mut();
    let scroll = cx
        .create_component(
            id,
            ScrollView::new(ScrollAxes::Both).style(NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(320.0)),
                    height: Some(LengthSpec::Px(160.0)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
        .unwrap();
    let table = cx.create_component(id, Table::new()).unwrap();
    let mut mutations = MutationQueue::new();
    mutations.insert(scroll.stable_id(), table.stable_id(), None);
    cx.commit_mutations(mutations).unwrap();
    let layout = VirtualTableLayout::new(
        std::iter::repeat_n(32.0, 1000000),
        (0..10000).map(|column| nana_ui_core::TableColumn::new(column.to_string(), 80.0)),
    );
    let mut items = VirtualTableItems::<usize, usize>::default();
    let fill = |cx: &mut nana_ui::runtime::AppContext,
                items: &mut VirtualTableItems<usize, usize>,
                offset| {
        cx.materialize_virtual_table_retained_in(
            table,
            items,
            &layout,
            VirtualViewport {
                offset,
                extent: [320.0, 160.0],
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
    fill(cx, &mut items, [0.0; 2]);
    let cell = items.cell_entity(&2, &2).unwrap();
    let mut editor = None;
    cx.mount(cell, |ui| {
        editor = Some(ui.child("editor", TextInput::new("draft"))?);
        Ok(())
    })
    .unwrap();
    let editor = editor.unwrap();
    let mut session = RuntimeAgentSession::new(document, 360, 200).unwrap();
    let cx = session.document_mut().context_mut();
    cx.focus_node(id, editor.stable_id()).unwrap();
    cx.set_ime_preedit(id, "拼".into(), None).unwrap();
    fill(cx, &mut items, [640000.0, 16000000.0]);
    session.flush().unwrap();
    session
        .document_mut()
        .context_mut()
        .scroll_to(
            scroll,
            ScrollOffset {
                x: 640000.0,
                y: 16000000.0,
            },
        )
        .unwrap();
    session.flush().unwrap();
    assert_eq!(
        items.mounted_rows().len() * items.mounted_columns().len(),
        30
    );
    let nodes = session.document().context().world().len();
    let primitives = session.document().scene().primitive_count();
    let output =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/virtual-rust-table");
    std::fs::create_dir_all(&output).unwrap();
    session.screenshot_png(output.join("jumped.png")).unwrap();
    for (row, column, x, y) in [
        (0, 0, 12.0, 12.0),
        (0, 8001, 92.0, 12.0),
        (500001, 0, 12.0, 44.0),
        (500001, 8001, 92.0, 44.0),
    ] {
        let cell = items.cell_entity(&row, &column).unwrap();
        let bounds = session
            .document()
            .scene()
            .draw_node_bounds(cell.stable_id())
            .unwrap();
        assert!(
            x >= bounds.x
                && x < bounds.x + bounds.width
                && y >= bounds.y
                && y < bounds.y + bounds.height,
            "{row}/{column}: {bounds:?}"
        );
        session.click_xy(x, y).unwrap();
        assert_eq!(
            session.document().context().world().focused(id),
            Some(cell.stable_id()),
            "{row}/{column}"
        );
    }
    fill(
        session.document_mut().context_mut(),
        &mut items,
        [640000.0, 16000000.0],
    );
    session.flush().unwrap();
    assert_eq!(
        items.mounted_rows().len() * items.mounted_columns().len(),
        20
    );
    assert!(
        !session
            .document()
            .context()
            .world()
            .contains(editor.stable_id())
    );
    let report = serde_json::json!({"logical_rows":1000000,"logical_columns":10000,
        "mounted_before":20,"mounted_active":30,"mounted_after_release":20,
        "retained_nodes_with_editor":nodes,"scene_primitives_with_editor":primitives,
        "frozen_corner_header_column_body_clicks":true,"refresh_rate_measured":false});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
}
