use nana_ui_core::ControlSize;
use nana_ui_runtime::{
    Button, ComponentGeometry, DocumentId, Entity, LayoutViewport, Select, SelectChanged,
    SelectOption, SettingsRow, Stack,
};
use nana_ui_scene::RuntimeDocument;
use std::sync::{Arc, Mutex};

fn flush(document: &mut RuntimeDocument) {
    let id = document.document();
    document
        .flush_with(|context, work| {
            if !work.layout.is_empty() {
                context.layout_document(id, LayoutViewport::new(600.0, 700.0))?;
            }
            Ok(())
        })
        .unwrap();
}

fn fixture() -> (RuntimeDocument, Entity<Select>, Entity<Button>) {
    let id = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(id);
    let root = doc
        .context_mut()
        .create_component(id, Stack::column(4.0))
        .unwrap();
    let mut select = Select::new(Some("second")).size(ControlSize::Small);
    select.options = vec![
        SelectOption::new("first", "First"),
        SelectOption::new("second", "Second"),
    ];
    let select = doc
        .context_mut()
        .create_detached_component(id, select)
        .unwrap();
    let mut row = SettingsRow::new("Parameter")
        .stacked(true)
        .control_child(select.stable_id());
    Arc::make_mut(&mut row.style.layout).z_index = Some(1);
    let row = doc
        .context_mut()
        .create_detached_component(id, row)
        .unwrap();
    doc.context_mut().append_child(root, row).unwrap();
    doc.context_mut().append_child(row, select).unwrap();
    let button = doc
        .context_mut()
        .create_detached_component(id, Button::new("Stop").size(ControlSize::Small))
        .unwrap();
    doc.context_mut().append_child(root, button).unwrap();
    flush(&mut doc);
    (doc, select, button)
}

#[test]
fn opening_and_closing_select_updates_hit_region_in_the_same_frame() {
    let (mut doc, select, button) = fixture();
    let id = doc.document();
    let changes = Arc::new(Mutex::new(Vec::new()));
    let sink = changes.clone();
    doc.context_mut()
        .on(select, move |_, event: &SelectChanged, _| {
            sink.lock().unwrap().push(event.value.clone())
        })
        .unwrap();
    let bounds = doc
        .context()
        .world()
        .layout_box(button.stable_id())
        .unwrap();
    let point = [
        bounds.x + bounds.width * 0.5,
        bounds.y + bounds.height * 0.5,
    ];
    assert_eq!(
        doc.context().world().hit_test(id, point[0], point[1]),
        Some(button.stable_id())
    );
    doc.context_mut().toggle_select(select).unwrap();
    flush(&mut doc);
    let hit = doc.context().world().hit_test(id, point[0], point[1]);
    assert_eq!(hit, Some(select.stable_id()));
    doc.context_mut()
        .activate_node_at(hit.unwrap(), point[0], point[1])
        .unwrap();
    flush(&mut doc);
    assert_eq!(changes.lock().unwrap().len(), 1);
    assert_eq!(
        doc.context().world().hit_test(id, point[0], point[1]),
        Some(button.stable_id())
    );
}

#[test]
fn option_count_and_size_refresh_menu_hits_but_highlighting_does_not() {
    let (mut doc, select, _) = fixture();
    let id = doc.document();
    doc.context_mut().toggle_select(select).unwrap();
    flush(&mut doc);
    doc.context_mut()
        .update_component(select, |control, _| {
            control.options.push(SelectOption::new("third", "Third"));
            control.size = ControlSize::Large;
        })
        .unwrap();
    flush(&mut doc);
    let ComponentGeometry::Select {
        menu: Some(menu), ..
    } = doc
        .context()
        .world()
        .component_geometry(select.stable_id())
        .unwrap()
    else {
        panic!("opened menu")
    };
    let last = menu.options.last().unwrap().bounds;
    let point = [last.x + last.width * 0.5, last.y + last.height * 0.5];
    assert_eq!(
        doc.context().world().hit_test(id, point[0], point[1]),
        Some(select.stable_id())
    );
    doc.context_mut()
        .update_component(select, |control, _| {
            control.highlighted = Some(2);
        })
        .unwrap();
    let work = doc.context_mut().world_mut().take_system_work();
    assert!(work.input_hit_test.is_empty());
    assert!(!work.render_extraction.is_empty());
}
