use nana_ui_core::ControlSize;
use nana_ui_runtime::{
    Button, ComponentGeometry, DocumentId, Entity, LayoutViewport, Select, SelectChanged,
    SelectOption, SettingsRow, Stack,
};
use nana_ui_scene::{PrimitiveId, RuntimeDocument, ScenePrimitive, SceneRect};
use std::sync::{Arc, Mutex};

fn flush(document: &mut RuntimeDocument) {
    let id = document.document();
    document
        .flush_with(|context, work| {
            if !work.layout_frontier_seeds.is_empty() {
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
    let work = doc.context_mut().compat_world_mut().take_system_work();
    assert!(work.input_hit_test.is_empty());
    assert!(!work.render_extraction.is_empty());
}

/// A select node's primitive in `namespace` at `index`, as the scene keys
/// its per-row collections.
fn row_slot(namespace: u64, index: usize) -> u64 {
    (namespace << 32) | index as u64
}

const ROW_LABELS: u64 = 11;
const MENU_SCROLLBAR: u64 = 13;
const MENU_SURFACE: u64 = 8;

fn open_menu(doc: &RuntimeDocument, select: Entity<Select>) -> nana_ui_runtime::SelectMenuGeometry {
    match doc.context().world().component_geometry(select.stable_id()) {
        Some(ComponentGeometry::Select {
            menu: Some(menu), ..
        }) => menu,
        other => panic!("expected an open menu, got {other:?}"),
    }
}

fn primitive(doc: &RuntimeDocument, select: Entity<Select>, slot: u64) -> Option<ScenePrimitive> {
    doc.scene()
        .primitive(PrimitiveId {
            node: select.stable_id(),
            slot,
        })
        .cloned()
}

/// A select's record is in its own coordinates, its menu among it: where
/// `bounds` stands on the page.
fn placed(doc: &RuntimeDocument, select: Entity<Select>, bounds: SceneRect) -> SceneRect {
    doc.scene().place_rect(select.stable_id(), bounds)
}

fn rect(bounds: nana_ui_runtime::LayoutBox) -> SceneRect {
    SceneRect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}

#[test]
fn an_open_menu_keeps_the_whole_arrow_and_paints_its_surface_in_a_slot_of_its_own() {
    let (mut doc, select, _) = fixture();
    doc.context_mut().toggle_select(select).unwrap();
    flush(&mut doc);
    let menu = open_menu(&doc, select);
    let surface = primitive(&doc, select, MENU_SURFACE).expect("the menu's surface");
    assert_eq!(placed(&doc, select, surface.bounds), rect(menu.surface));
    // The arrow's four strokes, 8, 6, 4 and 2 px wide, all still there.
    let strokes = (3..=6)
        .map(|slot| {
            primitive(&doc, select, slot)
                .unwrap_or_else(|| panic!("arrow stroke {slot}"))
                .bounds
                .width
        })
        .collect::<Vec<_>>();
    assert_eq!(strokes, [8.0, 6.0, 4.0, 2.0]);
}

#[test]
fn an_overflowing_menu_paints_only_the_rows_it_shows_inside_its_border() {
    let id = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(id);
    let root = doc
        .context_mut()
        .create_component(id, Stack::column(4.0))
        .unwrap();
    let mut select = Select::new(Some("o0")).size(ControlSize::Small);
    select.options = (0..60)
        .map(|n| SelectOption::new(format!("o{n}"), format!("Option {n}")))
        .collect();
    let select = doc
        .context_mut()
        .create_detached_component(id, select)
        .unwrap();
    doc.context_mut().append_child(root, select).unwrap();
    flush(&mut doc);
    doc.context_mut().toggle_select(select).unwrap();
    flush(&mut doc);
    let check = |doc: &RuntimeDocument| {
        let menu = open_menu(doc, select);
        assert!(menu.max_scroll > 0.0);
        let viewport = rect(menu.viewport);
        let mut shown = 0;
        for (index, option) in menu.options.iter().enumerate() {
            let inside = option.bounds.y < menu.viewport.y + menu.viewport.height
                && option.bounds.y + option.bounds.height > menu.viewport.y;
            let label = primitive(doc, select, row_slot(ROW_LABELS, index));
            assert_eq!(label.is_some(), inside, "row {index}");
            if let Some(label) = label {
                shown += 1;
                let cut = label.clips.last().expect("cut at the menu's border");
                assert_eq!(placed(doc, select, cut.bounds), viewport);
                assert!(cut.corner_radius > 0.0);
            }
        }
        assert!(shown > 0 && shown < menu.options.len());
        let bar = menu.scrollbar.expect("a bar beside the rows");
        let thumb = primitive(doc, select, row_slot(MENU_SCROLLBAR, 1)).expect("the thumb");
        assert_eq!(placed(doc, select, thumb.bounds), rect(bar.thumb));
        menu.scroll
    };
    assert_eq!(check(&doc), 0.0);
    // Scrolling repaints the menu; what the pointer hits is the same surface.
    doc.context_mut()
        .update_component(select, |select, _| {
            select.menu_scroll.offset = Some(150.0);
        })
        .unwrap();
    let work = doc.context_mut().compat_world_mut().take_system_work();
    assert!(work.input_hit_test.is_empty());
    assert!(!work.render_extraction.is_empty());
    // Scrolled as one small edit, so the scene takes it as a delta.
    doc.context_mut()
        .update_component(select, |select, _| {
            select.menu_scroll.offset = Some(300.0);
        })
        .unwrap();
    flush(&mut doc);
    assert_eq!(check(&doc), 300.0);
    assert!(primitive(&doc, select, row_slot(ROW_LABELS, 0)).is_none());
}

/// A menu opened across the next group of the page paints over it: a later
/// sibling with the same z-index no longer covers the rows.
#[test]
fn a_menu_paints_over_a_later_sibling_group() {
    let id = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(id);
    let root = doc
        .context_mut()
        .create_component(id, Stack::column(0.0))
        .unwrap();
    let first = doc
        .context_mut()
        .create_detached_component(
            id,
            Stack::column(0.0).with_layout(|layout| {
                layout.position = nana_ui_core::PositionSpec::Relative;
                layout.z_index = Some(1);
            }),
        )
        .unwrap();
    doc.context_mut().append_child(root, first).unwrap();
    let mut select = Select::new(Some("a")).size(ControlSize::Small);
    select.options = (0..5)
        .map(|n| SelectOption::new(format!("{n}"), format!("Option {n}")))
        .collect();
    select.options[0] = SelectOption::new("a", "First");
    let select = doc
        .context_mut()
        .create_detached_component(id, select)
        .unwrap();
    doc.context_mut().append_child(first, select).unwrap();
    let mut below = Button::new("Below").size(ControlSize::Small);
    {
        let layout = Arc::make_mut(&mut below.style.layout);
        layout.position = nana_ui_core::PositionSpec::Relative;
        layout.z_index = Some(1);
        layout.height = Some(nana_ui_core::LengthSpec::Px(200.0));
    }
    let below = doc
        .context_mut()
        .create_detached_component(id, below)
        .unwrap();
    doc.context_mut().append_child(root, below).unwrap();
    flush(&mut doc);
    doc.context_mut().toggle_select(select).unwrap();
    flush(&mut doc);
    let order = doc
        .scene()
        .primitives()
        .map(|primitive| primitive.id)
        .collect::<Vec<_>>();
    let at = |node: Entity<Select>, slot: u64| {
        order
            .iter()
            .position(|id| id.node == node.stable_id() && id.slot == slot)
            .expect("painted")
    };
    let last_below = order
        .iter()
        .rposition(|id| id.node == below.stable_id())
        .expect("the button paints");
    assert!(at(select, MENU_SURFACE) > last_below);
    assert!(at(select, row_slot(ROW_LABELS, 1)) > last_below);
    // The field itself stays in its group, under the button's.
    assert!(at(select, 0) < last_below);
}
