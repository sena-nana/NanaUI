//! 下拉菜单的命中区域、绘制遮挡及展开生命周期回归验证。
use nana_ui_core::ControlSize;
use nana_ui_runtime::{
    Button, ComponentGeometry, DocumentId, Dropdown, DropdownOption, Entity, LayoutViewport,
    Select, SelectChanged, SelectOption, SettingsRow, StableNodeId, Stack,
};
use nana_ui_scene::{PrimitiveId, RuntimeDocument, ScenePrimitive, ScenePrimitiveKind, SceneRect};
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
    assert_eq!(surface.bounds, rect(menu.surface));
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
                assert_eq!(cut.bounds, viewport);
                assert!(cut.corner_radius > 0.0);
            }
        }
        assert!(shown > 0 && shown < menu.options.len());
        let bar = menu.scrollbar.expect("a bar beside the rows");
        let thumb = primitive(doc, select, row_slot(MENU_SCROLLBAR, 1)).expect("the thumb");
        assert_eq!(thumb.bounds, rect(bar.thumb));
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

/// 菜单覆盖按钮的全部绘制内容，包括表面、选中背景、文字及多选标记。
fn assert_menu_covers_button(
    document: &RuntimeDocument,
    field: StableNodeId,
    button: StableNodeId,
    multiple: bool,
) {
    let ComponentGeometry::Select {
        menu: Some(menu), ..
    } = document
        .context()
        .world()
        .component_geometry(field)
        .unwrap()
    else {
        panic!("展开控件应提供菜单几何")
    };
    let button_bounds = document.context().world().layout_box(button).unwrap();
    assert!(
        menu.surface.x < button_bounds.x + button_bounds.width
            && menu.surface.x + menu.surface.width > button_bounds.x
            && menu.surface.y < button_bounds.y + button_bounds.height
            && menu.surface.y + menu.surface.height > button_bounds.y,
        "回归场景中的菜单应实际覆盖按钮区域"
    );
    let primitives: Vec<_> = document.scene().primitives().collect();
    let button_last = primitives
        .iter()
        .rposition(|primitive| primitive.node == button)
        .expect("按钮应包含可绘制内容");
    let menu_primitives: Vec<_> = primitives
        .iter()
        .enumerate()
        .filter(|(_, primitive)| primitive.node == field && primitive.bounds.y >= menu.surface.y)
        .collect();
    assert!(
        menu_primitives
            .iter()
            .all(|(index, _)| *index > button_last)
    );
    assert!(
        menu_primitives.iter().any(|(_, primitive)| {
            matches!(primitive.kind, ScenePrimitiveKind::Quad { .. })
                && primitive.bounds.y == menu.surface.y
                && primitive.bounds.height == menu.surface.height
        }),
        "菜单表面应覆盖按钮"
    );
    assert!(
        menu_primitives.iter().any(|(_, primitive)| {
            matches!(primitive.kind, ScenePrimitiveKind::Quad { .. })
                && primitive.bounds.y == menu.options[0].bounds.y
                && primitive.bounds.height == menu.options[0].bounds.height
        }),
        "选中选项应包含背景"
    );
    for label in ["First", "Second"] {
        assert!(menu_primitives.iter().any(|(_, primitive)| {
            matches!(&primitive.kind, ScenePrimitiveKind::Text { content, .. } if content == label)
        }), "菜单应绘制选项 {label}");
    }
    if multiple {
        assert!(menu_primitives.iter().any(|(_, primitive)| {
            matches!(&primitive.kind, ScenePrimitiveKind::Text { content, .. } if content == "✓")
        }), "多选菜单应绘制选中标记");
    }
}

/// 首帧展开、增量展开及兄弟层级变化均保持菜单遮挡，关闭立即恢复按钮命中。
#[test]
fn select_and_dropdown_menu_paint_above_buttons_through_open_sort_and_close() {
    for multiple in [false, true] {
        for initially_open in [false, true] {
            let id = DocumentId::new(1).unwrap();
            let mut document = RuntimeDocument::new(id);
            let cx = document.context_mut();
            let root = cx.create_component(id, Stack::column(4.0)).unwrap();
            let field = if multiple {
                let field = cx
                    .create_component(
                        id,
                        Dropdown::multiple(["first"])
                            .options([
                                DropdownOption::new("first", "First"),
                                DropdownOption::new("second", "Second"),
                            ])
                            .size(ControlSize::Small)
                            .opened(initially_open),
                    )
                    .unwrap();
                cx.append_child(root, field).unwrap();
                field.stable_id()
            } else {
                let field = cx
                    .create_component(
                        id,
                        Select::new(Some("first"))
                            .options([
                                SelectOption::new("first", "First"),
                                SelectOption::new("second", "Second"),
                            ])
                            .size(ControlSize::Small)
                            .opened(initially_open),
                    )
                    .unwrap();
                cx.append_child(root, field).unwrap();
                field.stable_id()
            };
            let button = cx
                .create_component(id, Button::new("Stop").size(ControlSize::Small))
                .unwrap();
            cx.append_child(root, button).unwrap();
            flush(&mut document);
            if !initially_open {
                toggle_field(&mut document, field, multiple);
                flush(&mut document);
            }
            assert_menu_covers_button(&document, field, button.stable_id(), multiple);

            document
                .context_mut()
                .update_component(button, |button, _| {
                    Arc::make_mut(&mut button.style.layout).z_index = Some(1);
                })
                .unwrap();
            flush(&mut document);
            assert_menu_covers_button(&document, field, button.stable_id(), multiple);

            toggle_field(&mut document, field, multiple);
            flush(&mut document);
            let world = document.context().world();
            let button_bounds = world.layout_box(button.stable_id()).unwrap();
            assert_eq!(
                world.hit_test(
                    id,
                    button_bounds.x + button_bounds.width * 0.5,
                    button_bounds.y + button_bounds.height * 0.5,
                ),
                Some(button.stable_id())
            );
            let field_bounds = world.layout_box(field).unwrap();
            assert!(
                document.scene().primitives().all(|primitive| {
                    primitive.node != field
                        || primitive.bounds.y < field_bounds.y + field_bounds.height
                }),
                "关闭菜单应移除全部浮层绘制内容"
            );
        }
    }
}

/// 通过各控件的公开交互入口切换菜单状态。
fn toggle_field(document: &mut RuntimeDocument, field: StableNodeId, multiple: bool) {
    let cx = document.context_mut();
    if multiple {
        let entity = cx.view_entity::<Dropdown>(field).unwrap();
        cx.update_component(entity, |dropdown, _| {
            dropdown.toggle_open();
        })
        .unwrap();
    } else {
        let entity = cx.view_entity::<Select>(field).unwrap();
        cx.toggle_select(entity).unwrap();
    }
}
