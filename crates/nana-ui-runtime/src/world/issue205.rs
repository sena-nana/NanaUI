//! Migrated component parts are published once, then consumed by paint/input.
use super::reflow_oracle::{bundled_face_shaper, product_frame};
use crate::{
    ActionMenuItem, AppContext, Button, Chip, ComponentGeometry, DocumentId, LayoutPartKind,
    LayoutViewport, Stack,
};
use nana_ui_core::{Icon, LengthSpec};
use std::{sync::Arc, time::Duration};

#[test]
fn issue205_empty_label_becoming_text_matches_fresh_content() {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let mut shaper = bundled_face_shaper();
    let root = cx.create_component(document, Stack::column(0.0)).unwrap();
    let button = cx
        .create_detached_component(document, Button::new("").icon(Icon::Add))
        .unwrap();
    cx.append_child(root, button).unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    product_frame(&mut cx, document, viewport, &mut shaper);
    cx.update_component(button, |button, _| button.label = "Save".into())
        .unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let result = cx.world().layout_result(button.stable_id()).unwrap();
    assert!(
        result
            .fragments
            .iter()
            .any(|fragment| fragment.kind == crate::LayoutFragmentKind::TextRun)
    );
    assert!(
        result
            .parts
            .iter()
            .any(|part| part.kind == LayoutPartKind::TextContent && part.bounds.width > 0.0)
    );
    assert_eq!(cx.world().len(), 2);
    let ComponentGeometry::Button { label, .. } =
        cx.world().component_geometry(button.stable_id()).unwrap()
    else {
        panic!("button")
    };
    assert!(label.bounds.width > 0.0);
    assert_eq!(label.content.as_ref(), "Save");
    let fresh = cx
        .create_detached_component(document, Button::new("Save").icon(Icon::Add))
        .unwrap();
    cx.append_child(root, fresh).unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let signature = |id| {
        let result = cx.world().layout_result(id).unwrap();
        (
            result.bounds.width,
            result.bounds.height,
            result
                .parts
                .iter()
                .map(|part| {
                    (
                        part.kind,
                        part.bounds.x - result.bounds.x,
                        part.bounds.y - result.bounds.y,
                        part.bounds.width,
                        part.bounds.height,
                    )
                })
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        signature(button.stable_id()),
        signature(fresh.stable_id()),
        "history must not change canonical geometry"
    );
}

#[test]
fn issue205_narrow_button_parts_stay_in_the_content_box() {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let mut shaper = bundled_face_shaper();
    let root = cx.create_component(document, Stack::column(0.0)).unwrap();
    let mut children = Vec::new();
    for width in [0.0, 4.0, 12.0, 24.0, 80.0] {
        let layout = Arc::new(nana_ui_core::LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(24.0)),
            ..Default::default()
        });
        let button = cx
            .create_detached_component(
                document,
                Button::new("Save document")
                    .icon(Icon::Add)
                    .trailing_icon(Icon::ChevronDown)
                    .layout(layout),
            )
            .unwrap();
        cx.append_child(root, button).unwrap();
        children.push(button);
    }
    product_frame(
        &mut cx,
        document,
        LayoutViewport::new(400.0, 300.0),
        &mut shaper,
    );
    for button in children {
        let result = cx.world().layout_result(button.stable_id()).unwrap();
        for part in result
            .parts
            .iter()
            .filter(|part| part.kind != LayoutPartKind::ComponentContent)
        {
            assert!(
                part.bounds.x >= result.content_box.x
                    && part.bounds.x + part.bounds.width
                        <= result.content_box.x + result.content_box.width + 0.01,
                "{part:?} outside {:?}",
                result.content_box
            );
            assert!(
                part.bounds.y >= result.content_box.y
                    && part.bounds.y + part.bounds.height
                        <= result.content_box.y + result.content_box.height + 0.01
            );
        }
    }
}

#[test]
fn issue205_button_chip_menu_parts_follow_content_changes_without_new_entities() {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let mut shaper = bundled_face_shaper();
    let viewport = LayoutViewport::new(600.0, 400.0);
    let root = cx
        .create_component(document, Stack::column(4.0).width(LengthSpec::Px(240.0)))
        .unwrap();
    let button = cx
        .create_detached_component(
            document,
            Button::new("Save")
                .icon(Icon::Close)
                .trailing_icon(Icon::ChevronDown),
        )
        .unwrap();
    let chip = cx
        .create_detached_component(document, Chip::new("Beta"))
        .unwrap();
    let menu = cx
        .create_detached_component(
            document,
            ActionMenuItem::new("Open")
                .leading(Icon::Close)
                .hint("Ctrl+O"),
        )
        .unwrap();
    cx.append_child(root, button).unwrap();
    cx.append_child(root, chip).unwrap();
    cx.append_child(root, menu).unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    assert_eq!(cx.world().len(), 4);
    let part = |cx: &AppContext, id, kind| {
        cx.world()
            .layout_result(id)
            .unwrap()
            .parts
            .iter()
            .find(|part| part.kind == kind)
            .unwrap()
            .bounds
    };
    let ComponentGeometry::Button {
        label,
        icon: Some((_, icon)),
        trailing_icon: Some((_, trailing)),
        ..
    } = cx.world().component_geometry(button.stable_id()).unwrap()
    else {
        panic!("button")
    };
    assert_eq!(
        label.bounds,
        part(&cx, button.stable_id(), LayoutPartKind::TextContent)
    );
    assert_eq!(
        icon,
        part(&cx, button.stable_id(), LayoutPartKind::LeadingContent)
    );
    assert_eq!(
        trailing,
        part(&cx, button.stable_id(), LayoutPartKind::TrailingContent)
    );
    assert!(
        icon.x + icon.width <= label.bounds.x && label.bounds.x + label.bounds.width <= trailing.x
    );
    let ComponentGeometry::ActionMenuItem {
        label,
        hint: Some(hint),
        ..
    } = cx.world().component_geometry(menu.stable_id()).unwrap()
    else {
        panic!("menu")
    };
    assert_eq!(
        label.bounds,
        part(&cx, menu.stable_id(), LayoutPartKind::TextContent)
    );
    assert_eq!(
        hint.bounds,
        part(&cx, menu.stable_id(), LayoutPartKind::Hint)
    );
    let hint_before = hint.bounds;
    cx.update_component(menu, |menu, _| menu.hint = Some(Arc::from("Ctrl+Shift+O")))
        .unwrap();
    cx.update_component(button, |button, _| button.loading = true)
        .unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    assert_ne!(
        hint_before,
        part(&cx, menu.stable_id(), LayoutPartKind::Hint)
    );
    assert!(
        cx.world()
            .layout_result(button.stable_id())
            .unwrap()
            .parts
            .iter()
            .any(|part| part.kind == LayoutPartKind::Spinner)
    );
    assert_eq!(cx.world().len(), 4);
    cx.update_component(button, |button, _| button.loading = false)
        .unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let snapshot = Arc::clone(&cx.world().layout_results[&button.stable_id()]);
    cx.set_pointer_hover_at(
        document,
        1,
        Some(button.stable_id()),
        Duration::from_millis(16),
    )
    .unwrap();
    let work = product_frame(&mut cx, document, viewport, &mut shaper);
    assert_eq!(
        (work.layout_measure_nodes, work.layout_placement_nodes),
        (0, 0)
    );
    assert!(Arc::ptr_eq(
        &snapshot,
        &cx.world().layout_results[&button.stable_id()]
    ));
    let a11y = cx.world().project_accessibility(document);
    assert!(
        a11y.iter()
            .any(|node| node.id == button.stable_id() && node.label.as_deref() == Some("Save"))
    );
}

#[test]
fn issue205_ten_thousand_plain_buttons_keep_one_entity_and_two_parts_each() {
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    for count in [1_000, 10_000] {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let viewport = LayoutViewport::new(600.0, 400.0);
        let root = cx.create_component(document, Stack::column(0.0)).unwrap();
        let mut children = Vec::with_capacity(count);
        for _ in 0..count {
            children.push(
                cx.create_detached_component(document, Button::new("Save"))
                    .unwrap()
                    .stable_id(),
            );
        }
        let mut q = crate::MutationQueue::new();
        for &id in &children {
            q.insert(root.stable_id(), id, None);
        }
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!(cx.world().len(), count + 1);
        for &id in &children {
            let result = cx.world().layout_result(id).unwrap();
            assert_eq!(
                result.parts.len(),
                2,
                "plain label retains the original two parts"
            );
            assert!(result.child_placements.is_empty());
        }
        let generation = cx.world().layout_generation();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert!(cx.take_system_work().is_empty());
        let idle = cx.world().frame_counters;
        assert_eq!(
            (
                idle.layout_measure_nodes,
                idle.layout_placement_nodes,
                idle.entities_spawned,
                idle.entities_despawned
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(cx.world().layout_generation(), generation);
    }
}

#[test]
fn issue205_row_text_parts_match_cold_and_preserve_state() {
    use super::reflow_oracle::assert_matches_cold;
    use crate::{ListItem, MutationQueue};
    fn build(width: f32, detail: &str) -> (AppContext, crate::Entity<ListItem>) {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx
            .create_component(document, Stack::column(0.0).width(LengthSpec::Px(320.0)))
            .unwrap();
        let mut row = ListItem::new("Document")
            .note("note")
            .detail(detail)
            .selected(true);
        Arc::make_mut(&mut row.style.layout).width = Some(LengthSpec::Px(width));
        let row = cx.create_detached_component(document, row).unwrap();
        cx.append_child(root, row).unwrap();
        let mut q = MutationQueue::new();
        q.request_focus(document, Some(row.stable_id()));
        cx.commit_mutations(q).unwrap();
        (cx, row)
    }
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    let mut shaper = bundled_face_shaper();
    let (mut cx, row) = build(280.0, "Ctrl+O");
    product_frame(&mut cx, document, viewport, &mut shaper);
    for (width, detail) in [(160.0, "long detail"), (48.0, "Ctrl+O"), (280.0, "Ctrl+O")] {
        cx.update_component(row, |row, _| {
            Arc::make_mut(&mut row.style.layout).width = Some(LengthSpec::Px(width));
            row.detail = detail.to_owned();
        })
        .unwrap();
        let work = product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!((work.entities_spawned, work.entities_despawned), (0, 0));
        assert!(cx.read(row, |row| row.selected).unwrap());
        assert_eq!(cx.world().focused(document), Some(row.stable_id()));
        let result = cx.world().layout_result(row.stable_id()).unwrap();
        let part = |kind| {
            result
                .parts
                .iter()
                .find(|part| part.kind == kind)
                .map(|part| part.bounds)
        };
        let ComponentGeometry::ListItem {
            content,
            note,
            detail,
            ..
        } = cx.world().component_geometry(row.stable_id()).unwrap()
        else {
            panic!("row")
        };
        assert_eq!(content, part(LayoutPartKind::TextContent));
        assert_eq!(
            note.as_ref().map(|note| note.bounds),
            part(LayoutPartKind::Note)
        );
        assert_eq!(
            detail.as_ref().map(|detail| detail.bounds),
            part(LayoutPartKind::Detail)
        );
        if cx.read(row, |row| row.detail == "1").unwrap() {
            assert_eq!(
                detail.as_ref().unwrap().bounds.width,
                cx.world()
                    .theme()
                    .spacing()
                    .xxxl
                    .min(result.content_box.width)
            );
        }
        for part in result.parts.iter() {
            assert!(part.bounds.x >= result.content_box.x - 0.01);
            assert!(
                part.bounds.x + part.bounds.width
                    <= result.content_box.x + result.content_box.width + 0.01
            );
        }
        let (mut cold, _) = build(width, &cx.read(row, |row| row.detail.clone()).unwrap());
        product_frame(&mut cold, document, viewport, &mut bundled_face_shaper());
        assert_matches_cold(&mut cx, &mut cold, document);
    }
    for display in [
        nana_ui_core::DisplaySpec::Grid,
        nana_ui_core::DisplaySpec::Inline,
        nana_ui_core::DisplaySpec::Flex,
    ] {
        let root = cx.world().node(row.stable_id()).unwrap().parent.unwrap();
        let change = |cx: &mut AppContext| {
            let mut style = cx.world().node_style(root).unwrap().clone();
            Arc::make_mut(&mut style.layout).display = Some(display);
            let mut q = MutationQueue::new();
            q.set_style(root, style);
            cx.commit_mutations(q).unwrap();
        };
        change(&mut cx);
        let work = product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!((work.entities_spawned, work.entities_despawned), (0, 0));
        assert_eq!(cx.world().focused(document), Some(row.stable_id()));
        assert!(cx.read(row, |row| row.selected).unwrap());
        let (mut cold, _) = build(280.0, "Ctrl+O");
        change(&mut cold);
        product_frame(&mut cold, document, viewport, &mut bundled_face_shaper());
        assert_matches_cold(&mut cx, &mut cold, document);
    }
    let generation = cx.world().layout_generation();
    let mut q = MutationQueue::new();
    let mut style = cx.world().node_style(row.stable_id()).unwrap().clone();
    style.foreground = Some(crate::SemanticColorRole::Danger);
    q.set_style(row.stable_id(), style);
    cx.commit_mutations(q).unwrap();
    let work = product_frame(&mut cx, document, viewport, &mut shaper);
    assert_eq!(
        (
            work.layout_measure_nodes,
            work.layout_placement_nodes,
            work.text_shaped_runs
        ),
        (0, 0, 0)
    );
    assert_eq!(cx.world().layout_generation(), generation);
}

#[test]
fn issue205_ten_thousand_rows_keep_one_entity_and_bounded_parts() {
    use crate::ListItem;
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    for count in [1_000, 10_000] {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx
            .create_component(document, Stack::column(0.0).width(LengthSpec::Px(600.0)))
            .unwrap();
        let mut q = crate::MutationQueue::new();
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let row = cx
                .create_detached_component(
                    document,
                    ListItem::new("Document").note("note").detail("detail"),
                )
                .unwrap();
            ids.push(row.stable_id());
            q.insert(root.stable_id(), row.stable_id(), None);
        }
        cx.commit_mutations(q).unwrap();
        let viewport = LayoutViewport::new(800.0, 600.0);
        let mut shaper = bundled_face_shaper();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!(cx.world().len(), count + 1);
        for id in ids {
            let result = cx.world().layout_result(id).unwrap();
            assert_eq!(result.parts.len(), 4);
            assert!(result.child_placements.is_empty());
        }
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert!(cx.take_system_work().is_empty());
        let idle = cx.world().frame_counters;
        assert_eq!(
            (
                idle.layout_measure_nodes,
                idle.layout_placement_nodes,
                idle.text_shaped_runs,
                idle.entities_spawned,
                idle.entities_despawned
            ),
            (0, 0, 0, 0, 0)
        );
    }
}

#[test]
fn issue205_rtl_row_parts_respect_live_slots_and_hidden_tools() {
    use crate::{ListItem, ListItemSlots, Text};
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let mut row_value = ListItem::new("Document").note("note").detail("detail");
    let style = Arc::make_mut(&mut row_value.style.layout);
    style.width = Some(LengthSpec::Px(320.0));
    style.dir = Some(nana_ui_core::DirSpec::Rtl);
    let row = cx.create_component(document, row_value).unwrap();
    let leading = cx
        .create_detached_component(document, Text::new("L"))
        .unwrap();
    let trailing = cx
        .create_detached_component(document, Text::new("T"))
        .unwrap();
    cx.set_list_item_slots(
        row,
        ListItemSlots {
            leading: Some(leading.stable_id()),
            trailing: Some(trailing.stable_id()),
            content: None,
        },
    )
    .unwrap();
    let viewport = LayoutViewport::new(400.0, 200.0);
    let mut shaper = bundled_face_shaper();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let world = cx.world();
    let result = world.layout_result(row.stable_id()).unwrap();
    let label = result
        .parts
        .iter()
        .find(|part| part.kind == LayoutPartKind::TextContent)
        .unwrap()
        .bounds;
    let detail = result
        .parts
        .iter()
        .find(|part| part.kind == LayoutPartKind::Detail)
        .unwrap()
        .bounds;
    let leading_box = world.layout_box(leading.stable_id()).unwrap();
    let trailing_box = world.layout_box(trailing.stable_id()).unwrap();
    assert!(label.x + label.width <= leading_box.x);
    assert!(detail.x >= trailing_box.x + trailing_box.width);
    assert!(detail.x < label.x);
    let initial_width = label.width;
    cx.update_component(leading, |text, _| {
        Arc::make_mut(&mut text.style.layout).display = Some(nana_ui_core::DisplaySpec::None)
    })
    .unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let world = cx.world();
    let part = world
        .layout_result(row.stable_id())
        .unwrap()
        .parts
        .iter()
        .find(|part| part.kind == LayoutPartKind::TextContent)
        .unwrap();
    assert!(part.bounds.x + part.bounds.width > label.x + initial_width);
    assert_eq!(world.len(), 3);
    let ComponentGeometry::ListItem { leading, .. } =
        world.component_geometry(row.stable_id()).unwrap()
    else {
        panic!("row")
    };
    assert!(leading.is_none());
}

#[test]
fn issue205_form_field_parts_follow_support_state_and_rtl_without_replacing_control() {
    use crate::{FormField, MutationQueue, TextInput};
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let root = cx
        .create_component(document, Stack::column(0.0).width(LengthSpec::Px(320.0)))
        .unwrap();
    let field = cx
        .create_detached_component(document, FormField::new("Email").hint("Work email"))
        .unwrap();
    let control = cx
        .create_detached_component(document, TextInput::new("value"))
        .unwrap();
    cx.append_child(root, field).unwrap();
    cx.set_form_field_control(field, Some(control.stable_id()))
        .unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    let mut shaper = bundled_face_shaper();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let result = cx.world().layout_result(field.stable_id()).unwrap();
    assert!(
        result
            .parts
            .iter()
            .any(|part| part.kind == LayoutPartKind::Label)
    );
    assert!(
        result
            .parts
            .iter()
            .any(|part| part.kind == LayoutPartKind::Hint)
    );
    assert!(
        !result
            .parts
            .iter()
            .any(|part| part.kind == LayoutPartKind::LeadingContent)
    );
    assert!(
        result
            .child_placements
            .iter()
            .any(|placement| placement.node == control.stable_id())
    );
    let control_result = cx.world().layout_result(control.stable_id()).cloned();

    cx.update_component(field, |field, _| {
        field.error = Some("Invalid email".into());
    })
    .unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let result = cx.world().layout_result(field.stable_id()).unwrap();
    let hint = result
        .parts
        .iter()
        .find(|part| part.kind == LayoutPartKind::Hint)
        .unwrap()
        .bounds;
    let indicator_lane = result
        .parts
        .iter()
        .find(|part| part.kind == LayoutPartKind::LeadingContent)
        .unwrap()
        .bounds;
    assert!(hint.x > indicator_lane.x);
    assert_eq!(
        cx.world().layout_result(control.stable_id()),
        control_result.as_ref()
    );
    let ComponentGeometry::FormField {
        support, indicator, ..
    } = cx.world().component_geometry(field.stable_id()).unwrap()
    else {
        panic!("form field")
    };
    assert_eq!(support.as_ref().map(|region| region.bounds), Some(hint));
    assert!(indicator.is_some());

    let mut style = cx.world().node_style(field.stable_id()).unwrap().clone();
    Arc::make_mut(&mut style.layout).dir = Some(nana_ui_core::DirSpec::Rtl);
    let mut q = MutationQueue::new();
    q.set_style(field.stable_id(), style);
    cx.commit_mutations(q).unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let result = cx.world().layout_result(field.stable_id()).unwrap();
    let hint = result
        .parts
        .iter()
        .find(|part| part.kind == LayoutPartKind::Hint)
        .unwrap()
        .bounds;
    let indicator_lane = result
        .parts
        .iter()
        .find(|part| part.kind == LayoutPartKind::LeadingContent)
        .unwrap()
        .bounds;
    assert!(hint.x + hint.width <= indicator_lane.x + 0.01);
}

#[test]
fn issue205_select_trigger_parts_keep_hint_and_handle_indexed_without_layout_on_highlight() {
    use crate::{Select, SelectOption};
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let root = cx
        .create_component(document, Stack::column(0.0).width(LengthSpec::Px(320.0)))
        .unwrap();
    let mut select = Select::new(Some("a"))
        .options([
            SelectOption::new("a", "Alpha").hint("A"),
            SelectOption::new("b", "Beta").hint("B"),
        ])
        .opened(true);
    Arc::make_mut(&mut select.style.layout).width = Some(LengthSpec::Px(240.0));
    let select = cx.create_detached_component(document, select).unwrap();
    cx.append_child(root, select).unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    let mut shaper = bundled_face_shaper();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let result = cx.world().layout_result(select.stable_id()).unwrap();
    let part = |kind| {
        result
            .parts
            .iter()
            .find(|part| part.kind == kind)
            .map(|part| part.bounds)
    };
    let label = part(LayoutPartKind::TextContent).unwrap();
    let hint = part(LayoutPartKind::Hint).unwrap();
    let handle = part(LayoutPartKind::TrailingContent).unwrap();
    let ComponentGeometry::Select {
        label: label_region,
        hint: hint_region,
        handle: handle_region,
        ..
    } = cx.world().component_geometry(select.stable_id()).unwrap()
    else {
        panic!("select")
    };
    assert_eq!(label_region.bounds, label);
    assert_eq!(hint_region.as_ref().map(|region| region.bounds), Some(hint));
    assert_eq!(handle_region, handle);
    assert!(label.x + label.width <= hint.x);
    assert!(hint.x + hint.width <= handle.x);
    let generation = cx.world().layout_generation();
    cx.update_component(select, |select, _| select.highlighted = Some(1))
        .unwrap();
    let work = product_frame(&mut cx, document, viewport, &mut shaper);
    assert_eq!(work.layout_measure_nodes, 0);
    assert_eq!(work.layout_placement_nodes, 0);
    assert_eq!(cx.world().layout_generation(), generation);

    let mut style = cx.world().node_style(select.stable_id()).unwrap().clone();
    Arc::make_mut(&mut style.layout).dir = Some(nana_ui_core::DirSpec::Rtl);
    let mut q = crate::MutationQueue::new();
    q.set_style(select.stable_id(), style);
    cx.commit_mutations(q).unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let result = cx.world().layout_result(select.stable_id()).unwrap();
    let label = result
        .parts
        .iter()
        .find(|part| part.kind == LayoutPartKind::TextContent)
        .unwrap()
        .bounds;
    let handle = result
        .parts
        .iter()
        .find(|part| part.kind == LayoutPartKind::TrailingContent)
        .unwrap()
        .bounds;
    assert!(handle.x + handle.width <= label.x + 0.01);
}

#[test]
fn issue205_ten_thousand_form_fields_and_selects_keep_bounded_parts() {
    use crate::{FormField, Select, SelectOption};
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    for count in [1_000, 10_000] {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx
            .create_component(document, Stack::column(0.0).width(LengthSpec::Px(640.0)))
            .unwrap();
        let mut q = crate::MutationQueue::new();
        let mut fields = Vec::with_capacity(count);
        let mut selects = Vec::with_capacity(count);
        for _ in 0..count {
            let field = cx
                .create_detached_component(
                    document,
                    FormField::new("Field").hint("Hint").error("Error"),
                )
                .unwrap();
            let select = cx
                .create_detached_component(
                    document,
                    Select::new(Some("a")).options([
                        SelectOption::new("a", "Alpha").hint("A"),
                        SelectOption::new("b", "Beta"),
                    ]),
                )
                .unwrap();
            fields.push(field.stable_id());
            selects.push(select.stable_id());
            q.insert(root.stable_id(), field.stable_id(), None);
            q.insert(root.stable_id(), select.stable_id(), None);
        }
        cx.commit_mutations(q).unwrap();
        let viewport = LayoutViewport::new(800.0, 600.0);
        let mut shaper = bundled_face_shaper();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!(cx.world().len(), count * 2 + 1);
        for id in fields {
            let result = cx.world().layout_result(id).unwrap();
            assert_eq!(result.parts.len(), 4);
            assert!(result.child_placements.is_empty());
        }
        for id in selects {
            let result = cx.world().layout_result(id).unwrap();
            assert_eq!(result.parts.len(), 4);
            assert!(result.child_placements.is_empty());
        }
        product_frame(&mut cx, document, viewport, &mut shaper);
        let idle = cx.world().frame_counters;
        assert_eq!(
            (
                idle.layout_measure_nodes,
                idle.layout_placement_nodes,
                idle.text_shaped_runs,
                idle.entities_spawned,
                idle.entities_despawned
            ),
            (0, 0, 0, 0, 0)
        );
    }
}

#[test]
fn issue205_value_parts_keep_action_at_the_end_and_match_cold() {
    use super::reflow_oracle::assert_matches_cold;
    use crate::{LabeledValue, MutationQueue};
    fn build(
        width: f32,
        value: &str,
        rtl: bool,
    ) -> (
        AppContext,
        crate::Entity<LabeledValue>,
        crate::Entity<Button>,
    ) {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx
            .create_component(document, Stack::column(0.0).width(LengthSpec::Px(width)))
            .unwrap();
        let mut model = LabeledValue::new("Revision", value);
        Arc::make_mut(&mut model.style.layout).dir = Some(if rtl {
            nana_ui_core::DirSpec::Rtl
        } else {
            nana_ui_core::DirSpec::Ltr
        });
        let value = cx.create_detached_component(document, model).unwrap();
        cx.append_child(root, value).unwrap();
        let action = cx
            .create_detached_component(document, Button::new("Copy"))
            .unwrap();
        cx.set_labeled_value_action(value, Some(action.stable_id()))
            .unwrap();
        let mut q = MutationQueue::new();
        q.request_focus(document, Some(action.stable_id()));
        cx.commit_mutations(q).unwrap();
        (cx, value, action)
    }
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(500.0, 200.0);
    for rtl in [false, true] {
        let (mut cx, value, action) = build(320.0, "42", rtl);
        let mut shaper = bundled_face_shaper();
        product_frame(&mut cx, document, viewport, &mut shaper);
        for text in ["a much longer value", "1", "42"] {
            cx.update_component(value, |value, _| value.value = Arc::from(text))
                .unwrap();
            let work = product_frame(&mut cx, document, viewport, &mut shaper);
            assert_eq!(
                work.layout_measure_nodes, 0,
                "a text part does not change the outer metric"
            );
            assert_eq!((work.entities_spawned, work.entities_despawned), (0, 0));
            assert_eq!(cx.world().focused(document), Some(action.stable_id()));
            assert_eq!(cx.world().len(), 3);
            let result = cx.world().layout_result(value.stable_id()).unwrap();
            let part = |kind| {
                result
                    .parts
                    .iter()
                    .find(|part| part.kind == kind)
                    .unwrap()
                    .bounds
            };
            let ComponentGeometry::LabeledValue {
                label,
                value: region,
                action: actual,
            } = cx.world().component_geometry(value.stable_id()).unwrap()
            else {
                panic!("value")
            };
            assert_eq!(label.bounds, part(LayoutPartKind::Label));
            assert_eq!(region.bounds, part(LayoutPartKind::Value));
            let actual = actual.unwrap();
            assert!(label.bounds.width > 0.0 && region.bounds.width > 0.0);
            if rtl {
                assert!(actual.x + actual.width <= region.bounds.x);
                assert!(region.bounds.x + region.bounds.width <= label.bounds.x);
            } else {
                assert!(label.bounds.x + label.bounds.width <= region.bounds.x);
                assert!(region.bounds.x + region.bounds.width <= actual.x);
            }
            let (mut cold, _, _) = build(320.0, text, rtl);
            product_frame(&mut cold, document, viewport, &mut bundled_face_shaper());
            assert_matches_cold(&mut cx, &mut cold, document);
        }
        let generation = cx.world().layout_generation();
        let mut visual = cx.world().standard_visual(value.stable_id()).unwrap();
        let crate::StandardVisual::LabeledValue { value_role, .. } = &mut visual else {
            panic!("value")
        };
        *value_role = crate::SemanticColorRole::Danger;
        let mut q = MutationQueue::new();
        q.set_standard_visual(value.stable_id(), Some(visual));
        cx.commit_mutations(q).unwrap();
        let work = product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!(
            (work.layout_measure_nodes, work.layout_placement_nodes),
            (0, 0)
        );
        assert_eq!(cx.world().layout_generation(), generation);
    }
}

#[test]
fn issue205_value_path_parts_honor_padding_and_keep_the_real_value() {
    use crate::LabeledValue;
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let root = cx
        .create_component(document, Stack::column(0.0).width(LengthSpec::Px(160.0)))
        .unwrap();
    let path = "/Users/example/a/very/long/directory/document.txt";
    let mut model = LabeledValue::new("Path", "").path_value(path);
    Arc::make_mut(&mut model.style.layout).padding = Some(LengthSpec::Px(8.0));
    let value = cx.create_detached_component(document, model).unwrap();
    cx.append_child(root, value).unwrap();
    product_frame(
        &mut cx,
        document,
        LayoutViewport::new(400.0, 200.0),
        &mut bundled_face_shaper(),
    );
    let world = cx.world();
    let result = world.layout_result(value.stable_id()).unwrap();
    let ComponentGeometry::LabeledValue {
        label,
        value: display,
        ..
    } = world.component_geometry(value.stable_id()).unwrap()
    else {
        panic!("value")
    };
    assert!(label.bounds.x >= result.content_box.x);
    assert!(
        display.bounds.x + display.bounds.width
            <= result.content_box.x + result.content_box.width + 0.01
    );
    assert_ne!(display.content.as_ref(), path);
    assert_eq!(
        world
            .accessibility(value.stable_id())
            .unwrap()
            .value
            .as_deref(),
        Some(path)
    );
    let crate::StandardVisual::LabeledValue { value: stored, .. } =
        world.standard_visual(value.stable_id()).unwrap()
    else {
        panic!("value")
    };
    assert_eq!(stored.as_ref(), path);
}

#[test]
fn issue205_ten_thousand_values_keep_one_entity_and_three_parts() {
    use crate::LabeledValue;
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    for count in [1_000, 10_000] {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx
            .create_component(document, Stack::column(0.0).width(LengthSpec::Px(600.0)))
            .unwrap();
        let mut q = crate::MutationQueue::new();
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let value = cx
                .create_detached_component(document, LabeledValue::new("Revision", "42"))
                .unwrap();
            ids.push(value.stable_id());
            q.insert(root.stable_id(), value.stable_id(), None);
        }
        cx.commit_mutations(q).unwrap();
        let viewport = LayoutViewport::new(800.0, 600.0);
        let mut shaper = bundled_face_shaper();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!(cx.world().len(), count + 1);
        for id in ids {
            let result = cx.world().layout_result(id).unwrap();
            assert_eq!(result.parts.len(), 3);
            assert!(result.child_placements.is_empty());
        }
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert!(cx.take_system_work().is_empty());
        let idle = cx.world().frame_counters;
        assert_eq!(
            (
                idle.layout_measure_nodes,
                idle.layout_placement_nodes,
                idle.entities_spawned,
                idle.entities_despawned
            ),
            (0, 0, 0, 0)
        );
    }
}

#[test]
fn issue205_card_header_shares_native_text_and_baseline_without_body_inheritance() {
    use crate::{Card, NodeStyle, Text};
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(600.0, 240.0);
    let mut cx = AppContext::new();
    let root = cx
        .create_component(
            document,
            Stack::row(8.0)
                .width(LengthSpec::Px(600.0))
                .with_layout(|style| style.align_items = nana_ui_core::AlignSpec::Baseline),
        )
        .unwrap();
    let mut model = Card::new().title("Activity").loading(true);
    let layout = Arc::make_mut(&mut model.style.layout);
    layout.width = Some(LengthSpec::Px(240.0));
    layout.height = Some(LengthSpec::Px(96.0));
    let card = cx.create_detached_component(document, model).unwrap();
    cx.append_child(root, card).unwrap();
    let body = cx
        .create_detached_component(document, Text::new("Body"))
        .unwrap();
    cx.append_child(card, body).unwrap();
    let reference = cx
        .create_detached_component(
            document,
            Text::new("Activity").style(NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    font_size: Some(crate::view_components::CARD_TITLE_SIZE),
                    font_weight: Some(crate::view_components::CARD_TITLE_WEIGHT),
                    line_height: Some(nana_ui_core::LineHeightSpec::Absolute(
                        crate::view_components::CARD_TITLE_LINE,
                    )),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
        .unwrap();
    cx.append_child(root, reference).unwrap();
    let mut shaper = bundled_face_shaper();
    product_frame(&mut cx, document, viewport, &mut shaper);
    for font in [None, Some(20.0)] {
        if let Some(font) = font {
            cx.update_component(card, |card, _| {
                Arc::make_mut(&mut card.style.layout).font_size = Some(font)
            })
            .unwrap();
            product_frame(&mut cx, document, viewport, &mut shaper);
        }
        let world = cx.world();
        let result = world.layout_result(card.stable_id()).unwrap();
        let header = result
            .parts
            .iter()
            .find(|part| part.kind == LayoutPartKind::Header)
            .unwrap()
            .bounds;
        let spinner = result
            .parts
            .iter()
            .find(|part| part.kind == LayoutPartKind::Spinner)
            .unwrap()
            .bounds;
        let ComponentGeometry::Card {
            title,
            content,
            spinner: actual,
            ..
        } = world.component_geometry(card.stable_id()).unwrap()
        else {
            panic!("card")
        };
        assert_eq!(title.unwrap().bounds, header);
        assert_eq!(actual, Some(spinner));
        assert!(header.y < content.y);
        assert!((spinner.y + spinner.height / 2.0 - header.y - header.height / 2.0).abs() < 0.01);
        let native = world.text_layout(card.stable_id()).unwrap().1;
        assert_eq!(native.constraints.max_width_px, Some(header.width));
        assert_eq!(native.constraints.max_height_px, Some(header.height));
        assert_eq!(native.constraints.max_lines, Some(1));
        assert!(
            (world.text_metrics(card.stable_id()).unwrap().width
                - world.text_metrics(reference.stable_id()).unwrap().width)
                .abs()
                < 0.01
        );
        let reference_baseline = world
            .layout_result(reference.stable_id())
            .unwrap()
            .first_baseline
            .unwrap();
        assert!(
            (result.first_baseline.unwrap() - reference_baseline).abs() < 0.01,
            "intrinsic baseline and painted header baseline must agree"
        );
        assert_eq!(
            world.computed_style(body.stable_id()).unwrap().font_weight,
            world.computed_style(root.stable_id()).unwrap().font_weight
        );
        if let Some(font) = font {
            assert_eq!(
                world.computed_style(body.stable_id()).unwrap().font_size,
                font
            );
        }
    }
    let pointer = cx.world().layout_result(card.stable_id()).unwrap() as *const _ as usize;
    let native_id = cx.world().text_layout(card.stable_id()).unwrap().0;
    cx.update_component(card, |card, _| card.loading_phase = 0.13)
        .unwrap();
    let work = product_frame(&mut cx, document, viewport, &mut shaper);
    assert_eq!(
        (
            work.layout_measure_nodes,
            work.layout_placement_nodes,
            work.text_shaped_runs
        ),
        (0, 0, 0)
    );
    assert_eq!(
        cx.world().layout_result(card.stable_id()).unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        cx.world().text_layout(card.stable_id()).unwrap().0,
        native_id
    );
}

#[test]
fn issue205_card_header_updates_match_cold_at_narrow_widths() {
    use super::reflow_oracle::assert_matches_cold;
    use crate::{Card, Text};
    fn build(
        width: f32,
        title: Option<&str>,
        loading: bool,
        rtl: bool,
    ) -> (AppContext, crate::Entity<Card>) {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx
            .create_component(document, Stack::column(0.0).width(LengthSpec::Px(400.0)))
            .unwrap();
        let mut model = Card::new().loading(loading);
        model.title = title.map(Arc::from);
        let layout = Arc::make_mut(&mut model.style.layout);
        layout.width = Some(LengthSpec::Px(width));
        layout.height = Some(LengthSpec::Px(96.0));
        layout.dir = Some(if rtl {
            nana_ui_core::DirSpec::Rtl
        } else {
            nana_ui_core::DirSpec::Ltr
        });
        let card = cx.create_detached_component(document, model).unwrap();
        cx.append_child(root, card).unwrap();
        let body = cx
            .create_detached_component(document, Text::new("Body"))
            .unwrap();
        cx.append_child(card, body).unwrap();
        (cx, card)
    }
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(500.0, 200.0);
    for rtl in [false, true] {
        let (mut cx, card) = build(240.0, Some("Activity"), true, rtl);
        let mut shaper = bundled_face_shaper();
        product_frame(&mut cx, document, viewport, &mut shaper);
        for (width, title, loading) in [
            (60.0, Some("A very long activity title"), true),
            (16.0, None, true),
            (240.0, Some(""), true),
            (240.0, Some("Activity"), false),
            (240.0, Some("Activity"), true),
        ] {
            cx.update_component(card, |card, _| {
                Arc::make_mut(&mut card.style.layout).width = Some(LengthSpec::Px(width));
                card.title = title.map(Arc::from);
                card.loading = loading;
            })
            .unwrap();
            let work = product_frame(&mut cx, document, viewport, &mut shaper);
            assert_eq!((work.entities_spawned, work.entities_despawned), (0, 0));
            let result = cx.world().layout_result(card.stable_id()).unwrap();
            for part in result.parts.iter().filter(|part| {
                matches!(part.kind, LayoutPartKind::Header | LayoutPartKind::Spinner)
            }) {
                assert!(part.bounds.x >= result.bounds.x && part.bounds.y >= result.bounds.y);
                assert!(
                    part.bounds.x + part.bounds.width
                        <= result.bounds.x + result.bounds.width + 0.01
                );
                assert!(
                    part.bounds.y + part.bounds.height
                        <= result.bounds.y + result.bounds.height + 0.01
                );
            }
            let (mut cold, _) = build(width, title, loading, rtl);
            product_frame(&mut cold, document, viewport, &mut bundled_face_shaper());
            assert_matches_cold(&mut cx, &mut cold, document);
        }
    }
}

#[test]
fn issue205_card_hugging_size_counts_header_height_once_and_reserves_spinner() {
    use crate::Card;
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let root = cx
        .create_component(
            document,
            Stack::row(0.0).with_layout(|style| {
                style.width = Some(LengthSpec::Px(600.0));
                style.height = Some(LengthSpec::Px(200.0));
                style.align_items = nana_ui_core::AlignSpec::Start;
            }),
        )
        .unwrap();
    let mut model = Card::new().title("Activity").loading(true);
    let layout = Arc::make_mut(&mut model.style.layout);
    layout.width = Some(LengthSpec::MaxContent);
    layout.padding = Some(LengthSpec::Px(8.0));
    let card = cx.create_detached_component(document, model).unwrap();
    cx.append_child(root, card).unwrap();
    let mut shaper = bundled_face_shaper();
    let viewport = LayoutViewport::new(600.0, 240.0);
    product_frame(&mut cx, document, viewport, &mut shaper);
    let world = cx.world();
    let result = world.layout_result(card.stable_id()).unwrap();
    assert_eq!(
        result.bounds.height,
        16.0 + crate::view_components::CARD_TITLE_BAND
    );
    let natural = world.text_metrics(card.stable_id()).unwrap().width;
    assert!(
        (result.bounds.width - natural - crate::view_components::CARD_LOADING_RESERVE - 16.0).abs()
            < 0.01
    );
    assert!(
        !world
            .text_layout(card.stable_id())
            .unwrap()
            .1
            .overflow
            .contains(nana_text::OverflowFlags::ELLIPSIZED)
    );
}

#[test]
fn issue205_ten_thousand_cards_keep_one_entity_and_four_parts() {
    use crate::Card;
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    for count in [1_000, 10_000] {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx
            .create_component(document, Stack::column(0.0).width(LengthSpec::Px(600.0)))
            .unwrap();
        let mut q = crate::MutationQueue::new();
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let card = cx
                .create_detached_component(document, Card::new().title("Activity").loading(true))
                .unwrap();
            ids.push(card.stable_id());
            q.insert(root.stable_id(), card.stable_id(), None);
        }
        cx.commit_mutations(q).unwrap();
        let viewport = LayoutViewport::new(800.0, 600.0);
        let mut shaper = bundled_face_shaper();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!(cx.world().len(), count + 1);
        for id in ids {
            let result = cx.world().layout_result(id).unwrap();
            assert_eq!(result.parts.len(), 4);
            assert!(result.child_placements.is_empty());
        }
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert!(cx.take_system_work().is_empty());
        let idle = cx.world().frame_counters;
        assert_eq!(
            (
                idle.layout_measure_nodes,
                idle.layout_placement_nodes,
                idle.entities_spawned,
                idle.entities_despawned
            ),
            (0, 0, 0, 0)
        );
    }
}

/// A checkbox `width` wide (its own size when `None`) labelled `label`, in a
/// row, laid out once.
fn checkbox_page(
    label: &str,
    size: nana_ui_core::ControlSize,
    width: Option<f32>,
) -> (AppContext, crate::Entity<crate::Checkbox>) {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let mut shaper = bundled_face_shaper();
    let root = cx
        .create_component(document, Stack::row(0.0).width(LengthSpec::Px(400.0)))
        .unwrap();
    let checkbox = cx
        .create_detached_component(document, checkbox_view(label, size, width))
        .unwrap();
    cx.append_child(root, checkbox).unwrap();
    product_frame(
        &mut cx,
        document,
        LayoutViewport::new(600.0, 400.0),
        &mut shaper,
    );
    cx.rebuild_hit_test(document);
    (cx, checkbox)
}

fn checkbox_view(
    label: &str,
    size: nana_ui_core::ControlSize,
    width: Option<f32>,
) -> crate::Checkbox {
    crate::Checkbox::new(label, false)
        .size(size)
        .style(crate::NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                width: width.map(LengthSpec::Px),
                ..Default::default()
            }),
            ..Default::default()
        })
}

fn part_of(cx: &AppContext, id: crate::StableNodeId, kind: LayoutPartKind) -> crate::LayoutBox {
    cx.world()
        .layout_result(id)
        .unwrap()
        .parts
        .iter()
        .find(|part| part.kind == kind)
        .unwrap_or_else(|| panic!("{kind:?} part of {id:?}"))
        .bounds
}

/// A checkbox's indicator and label are Flex parts layout publishes (#205):
/// measure, the paragraph budget, placement, paint and the published text
/// fragments all read them, and no copy of the indicator's room is left.
#[test]
fn issue205_checkbox_indicator_and_label_are_published_parts() {
    use nana_ui_core::ControlSize;
    let (cx, checkbox) = checkbox_page("Notifications", ControlSize::Medium, None);
    let id = checkbox.stable_id();
    let world = cx.world();
    let result = world.layout_result(id).unwrap();
    let content = result.content_box;
    let indicator = part_of(&cx, id, LayoutPartKind::LeadingContent);
    let label = part_of(&cx, id, LayoutPartKind::TextContent);
    let extent = ControlSize::Medium.indicator_size();
    let gap = ControlSize::Medium.indicator_gap();
    // The indicator square starts the row, centred on it; the label starts
    // one gap after it and takes the rest.
    assert_eq!(
        indicator,
        crate::LayoutBox {
            x: content.x,
            y: content.y + (content.height - extent) / 2.0,
            width: extent,
            height: extent,
        }
    );
    assert_eq!(label.x, content.x + extent + gap);
    assert_eq!(label.x + label.width, content.x + content.width);
    assert_eq!((label.y, label.height), (content.y, content.height));
    // Measure reads the same parts: the box is exactly as wide as the
    // indicator, the gap and the label's own text.
    let text = world.text_metrics(id).unwrap();
    assert!(text.width > 0.0);
    assert!(
        (label.width - text.width).abs() < 0.01,
        "label part {} != text {}",
        label.width,
        text.width
    );
    // Paint reads them, nothing else.
    let ComponentGeometry::Checkbox {
        indicator: painted,
        label: painted_label,
        ..
    } = world.component_geometry(id).unwrap()
    else {
        panic!("checkbox")
    };
    assert_eq!(painted, indicator);
    assert_eq!(painted_label.bounds, label);
    // The text fragments sit in the label part, not over the indicator.
    let runs: Vec<_> = result
        .fragments
        .iter()
        .filter(|fragment| fragment.kind == crate::LayoutFragmentKind::TextRun)
        .collect();
    assert!(!runs.is_empty());
    for run in runs {
        assert!(
            run.bounds.x >= label.x - 0.01
                && run.bounds.x + run.bounds.width <= label.x + label.width + 0.01,
            "{run:?} outside {label:?}"
        );
    }
    // One entity, the original two parts and the indicator.
    assert_eq!(world.len(), 2);
    assert_eq!(result.parts.len(), 3);

    // A checkbox too narrow for its label shapes the label to the label
    // part's width, not to the content box the indicator sits in.
    let long = "Notify me about every reply in this thread";
    let (cx, checkbox) = checkbox_page(long, ControlSize::Medium, Some(120.0));
    let id = checkbox.stable_id();
    let label = part_of(&cx, id, LayoutPartKind::TextContent);
    let (_, layout) = cx.world().text_layout(id).unwrap();
    let (shaped, _) = layout.physical_size();
    assert!(
        shaped <= label.width + 0.5,
        "label shaped {shaped} wide in a {} part",
        label.width
    );
}

/// Label, size and width changes republish the parts as a cold layout of
/// the same checkbox would; toggling and hovering lay nothing out.
#[test]
fn issue205_checkbox_parts_follow_changes_like_a_cold_layout_and_toggle_paints_only() {
    use nana_ui_core::ControlSize;
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(600.0, 400.0);
    let mut shaper = bundled_face_shaper();
    let (mut cx, checkbox) = checkbox_page("", ControlSize::Medium, None);
    for (label, size, width) in [
        ("Notifications", ControlSize::Medium, None),
        (
            "Notify me about every reply in this thread",
            ControlSize::Small,
            None,
        ),
        (
            "Notify me about every reply in this thread",
            ControlSize::Large,
            Some(90.0),
        ),
        ("", ControlSize::Large, Some(90.0)),
        ("Notifications", ControlSize::Medium, None),
    ] {
        cx.update_component(checkbox, |view, _| {
            *view = checkbox_view(label, size, width);
        })
        .unwrap();
        product_frame(&mut cx, document, viewport, &mut shaper);
        let (mut cold, cold_box) = checkbox_page(label, size, width);
        assert_eq!(cold_box.stable_id(), checkbox.stable_id());
        super::reflow_oracle::assert_matches_cold(&mut cx, &mut cold, document);
        assert_eq!(cx.world().len(), 2);
    }
    let id = checkbox.stable_id();
    let generation = cx.world().layout_generation();
    let snapshot = Arc::clone(&cx.world().layout_results[&id]);
    cx.update_component(checkbox, |view, _| view.checked = true)
        .unwrap();
    let toggled = product_frame(&mut cx, document, viewport, &mut shaper);
    cx.set_pointer_hover_at(document, 1, Some(id), Duration::from_millis(16))
        .unwrap();
    let hovered = product_frame(&mut cx, document, viewport, &mut shaper);
    for work in [toggled, hovered] {
        assert_eq!(
            (work.layout_measure_nodes, work.layout_placement_nodes),
            (0, 0)
        );
    }
    assert_eq!(cx.world().layout_generation(), generation);
    assert!(Arc::ptr_eq(&snapshot, &cx.world().layout_results[&id]));
    let a11y = cx.world().project_accessibility(document);
    let node = a11y.iter().find(|node| node.id == id).unwrap();
    assert_eq!(node.checked, Some(true));
}

/// A checked, focused checkbox moved between Grid, Inline and Flex parents
/// keeps its entity, state and focus, and publishes the parts a cold layout
/// in that context would.
#[test]
fn issue205_checkbox_parts_survive_context_transitions() {
    use super::reflow_oracle::assert_matches_cold;
    use crate::MutationQueue;
    use nana_ui_core::DisplaySpec;
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    let build = |display: DisplaySpec| {
        let mut cx = AppContext::new();
        let root = cx
            .create_component(
                document,
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(320.0));
                    layout.display = Some(display);
                }),
            )
            .unwrap();
        let checkbox = cx
            .create_detached_component(document, crate::Checkbox::new("Notifications", true))
            .unwrap();
        cx.append_child(root, checkbox).unwrap();
        let mut q = MutationQueue::new();
        q.request_focus(document, Some(checkbox.stable_id()));
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document, viewport, &mut bundled_face_shaper());
        (cx, root, checkbox)
    };
    let mut shaper = bundled_face_shaper();
    let (mut cx, root, checkbox) = build(DisplaySpec::Flex);
    let id = checkbox.stable_id();
    for display in [DisplaySpec::Grid, DisplaySpec::Inline, DisplaySpec::Flex] {
        let mut style = cx.world().node_style(root.stable_id()).unwrap().clone();
        Arc::make_mut(&mut style.layout).display = Some(display);
        let mut q = MutationQueue::new();
        q.set_style(root.stable_id(), style);
        cx.commit_mutations(q).unwrap();
        let work = product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!((work.entities_spawned, work.entities_despawned), (0, 0));
        assert!(cx.read(checkbox, |view| view.checked).unwrap());
        assert_eq!(cx.world().focused(document), Some(id));
        let ComponentGeometry::Checkbox {
            indicator, label, ..
        } = cx.world().component_geometry(id).unwrap()
        else {
            panic!("checkbox")
        };
        assert_eq!(indicator, part_of(&cx, id, LayoutPartKind::LeadingContent));
        assert_eq!(label.bounds, part_of(&cx, id, LayoutPartKind::TextContent));
        let (mut cold, ..) = build(display);
        assert_matches_cold(&mut cx, &mut cold, document);
    }
}

#[test]
fn issue205_ten_thousand_checkboxes_keep_one_entity_and_three_parts() {
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    for count in [1_000, 10_000] {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let viewport = LayoutViewport::new(600.0, 400.0);
        let root = cx.create_component(document, Stack::column(0.0)).unwrap();
        let mut children = Vec::with_capacity(count);
        for _ in 0..count {
            children.push(
                cx.create_detached_component(document, crate::Checkbox::new("Notify", false))
                    .unwrap()
                    .stable_id(),
            );
        }
        let mut q = crate::MutationQueue::new();
        for &id in &children {
            q.insert(root.stable_id(), id, None);
        }
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!(cx.world().len(), count + 1);
        for &id in &children {
            let result = cx.world().layout_result(id).unwrap();
            let kinds: Vec<_> = result.parts.iter().map(|part| part.kind).collect();
            assert_eq!(
                kinds,
                [
                    LayoutPartKind::TextContent,
                    LayoutPartKind::ComponentContent,
                    LayoutPartKind::LeadingContent
                ]
            );
            assert!(result.child_placements.is_empty());
        }
        let generation = cx.world().layout_generation();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert!(cx.take_system_work().is_empty());
        let idle = cx.world().frame_counters;
        assert_eq!(
            (
                idle.layout_measure_nodes,
                idle.layout_placement_nodes,
                idle.entities_spawned,
                idle.entities_despawned
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(cx.world().layout_generation(), generation);
    }
}

/// An open menu's room and a component's chrome clip read the clip chain
/// paint and hit testing follow (#201), not a walk of their own: a `fixed`
/// box hangs from nothing, so neither the scroller it sits in nor an empty
/// state's chrome clip above it cuts its menu room or its accessible box.
#[test]
fn issue205_menu_room_and_chrome_clips_follow_the_clip_chain() {
    use super::reflow_oracle::{Builder, fixed};
    use crate::{InteractionState, LayoutBox, MutationQueue, ScrollOffset, StandardVisual};
    use nana_ui_core::{FlexDirection, LayoutStyle, OverflowSpec, PositionSpec};
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(800.0, 600.0);
    let placed = |position, [left, top, width, height]: [f32; 4]| LayoutStyle {
        position,
        offset_left: Some(LengthSpec::Px(left)),
        offset_top: Some(LengthSpec::Px(top)),
        ..fixed(width, height)
    };
    let (mut b, page) = Builder::page(document, 1, 400.0);
    let scroller = b.element(
        page,
        LayoutStyle {
            overflow_x: OverflowSpec::Auto,
            overflow_y: OverflowSpec::Auto,
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            ..fixed(200.0, 100.0)
        },
    );
    let row = b.element(scroller, fixed(100.0, 30.0));
    b.element(scroller, fixed(200.0, 400.0));
    let pinned = b.element(
        scroller,
        placed(PositionSpec::Fixed, [500.0, 400.0, 100.0, 30.0]),
    );
    let empty = b.element(page, fixed(200.0, 100.0));
    let inside = b.element(empty, fixed(50.0, 20.0));
    let floating = b.element(
        empty,
        placed(PositionSpec::Fixed, [300.0, 300.0, 50.0, 20.0]),
    );
    b.queue.set_standard_visual(
        empty,
        Some(StandardVisual::EmptyState {
            title: Arc::from("Empty"),
            message: None,
            icon: None,
            compact: true,
            action: None,
        }),
    );
    for id in [row, pinned, inside, floating] {
        b.queue.set_interaction(
            id,
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
        );
    }
    let mut cx = AppContext::new();
    cx.commit_mutations(b.queue).unwrap();
    let mut shaper = bundled_face_shaper();
    let mut q = MutationQueue::new();
    q.set_scroll_offset(scroller, ScrollOffset { x: 0.0, y: 20.0 });
    cx.commit_mutations(q).unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    cx.rebuild_hit_test(document);
    let world = cx.world();
    let room = |id| {
        let room = world.select_menu_room(id).unwrap();
        (room.left, room.top, room.right, room.bottom)
    };
    // In flow, the scroller cuts the row's room where its scroll puts it.
    assert_eq!(room(row), (0.0, 20.0, 200.0, 120.0));
    // A fixed field's menu has the window, as the field is drawn and hit
    // past the scroller.
    assert_eq!(world.layout_clip_parent(pinned), None);
    assert_eq!(room(pinned), (0.0, 0.0, 800.0, 600.0));
    assert_eq!(world.hit_test(document, 550.0, 410.0), Some(pinned));

    let accessible = |id| {
        world
            .project_accessibility(document)
            .into_iter()
            .find(|node| node.id == id)
            .map(|node| node.bounds)
    };
    let empty_box = world.layout_box(empty).unwrap();
    // A box in the empty state's flow reads inside its chrome clip ...
    let inside_box = world.layout_box(inside).unwrap();
    assert!(empty_box.contains(inside_box.x, inside_box.y));
    let read = accessible(inside).unwrap();
    assert!(
        (read.x - inside_box.x).abs() < 0.01
            && (read.y - inside_box.y).abs() < 0.01
            && (read.width - inside_box.width).abs() < 0.01
            && (read.height - inside_box.height).abs() < 0.01,
        "{read:?} != {inside_box:?}"
    );
    // ... a fixed one outside it is not: it reads where it is drawn and hit.
    let floating_box = world.layout_box(floating).unwrap();
    assert_eq!(
        floating_box,
        LayoutBox {
            x: 300.0,
            y: 300.0,
            width: 50.0,
            height: 20.0
        }
    );
    assert_eq!(world.hit_test(document, 310.0, 305.0), Some(floating));
    assert_eq!(accessible(floating), Some(floating_box));
}

/// A control whose label is cleared gives up the label's metrics: it sizes
/// and publishes as the same control laid out cold with no label. Its text
/// used to keep the old metrics, because text work for a box without text
/// was dropped, so a cleared Button kept the old label's width.
#[test]
fn issue205_a_cleared_label_lays_out_as_a_cold_unlabelled_control() {
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(600.0, 400.0);
    let build = |label: &str| {
        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let root = cx
            .create_component(document, Stack::row(4.0).width(LengthSpec::Px(400.0)))
            .unwrap();
        let button = cx
            .create_detached_component(document, Button::new(label).icon(Icon::Add))
            .unwrap();
        let chip = cx
            .create_detached_component(document, Chip::new(label))
            .unwrap();
        cx.append_child(root, button).unwrap();
        cx.append_child(root, chip).unwrap();
        product_frame(&mut cx, document, viewport, &mut shaper);
        cx.rebuild_hit_test(document);
        (cx, button, chip)
    };
    let mut shaper = bundled_face_shaper();
    let (mut cx, button, chip) = build("Save the document");
    let labelled = cx.world().layout_box(button.stable_id()).unwrap().width;
    cx.update_component(button, |view, _| view.label = "".into())
        .unwrap();
    cx.update_component(chip, |view, _| view.label = "".into())
        .unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let (mut cold, ..) = build("");
    super::reflow_oracle::assert_matches_cold(&mut cx, &mut cold, document);
    assert!(cx.world().layout_box(button.stable_id()).unwrap().width < labelled);
    assert_eq!(
        cx.world().text_metrics(button.stable_id()),
        Some(crate::TextMetrics::default())
    );
    // Nothing is left to shape: the next frame has no work, and lays out
    // nothing.
    let generation = cx.world().layout_generation();
    product_frame(&mut cx, document, viewport, &mut shaper);
    assert!(cx.take_system_work().is_empty());
    let idle = cx.world().frame_counters;
    assert_eq!(
        (idle.layout_measure_nodes, idle.layout_placement_nodes),
        (0, 0)
    );
    assert_eq!(cx.world().layout_generation(), generation);
}

/// A theme that moves every metric a switch's parts read: the track, its
/// gap, the hint gap and the line ratio.
fn dense_switch_theme() -> nana_ui_core::ThemeDefinition {
    let mut definition =
        nana_ui_core::ThemeDefinition::for_appearance(nana_ui_core::ThemeAppearance::Light);
    definition.tokens.metrics.switch.track_width += 6.0;
    definition.tokens.metrics.switch.track_height += 4.0;
    definition.tokens.metrics.switch.label_gap += 2.0;
    definition.tokens.spacing.xxs += 2.0;
    definition.typography.line += 2.0;
    definition.bump()
}

/// `view` in a 320px column, under `theme` when given, laid out once.
fn switch_page(
    view: crate::Switch,
    theme: Option<&nana_ui_core::ThemeDefinition>,
) -> (AppContext, crate::Entity<crate::Switch>) {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    if let Some(theme) = theme {
        cx.set_theme_definition(theme).unwrap();
    }
    let root = cx
        .create_component(document, Stack::column(0.0).width(LengthSpec::Px(320.0)))
        .unwrap();
    let switch = cx.create_detached_component(document, view).unwrap();
    cx.append_child(root, switch).unwrap();
    product_frame(
        &mut cx,
        document,
        LayoutViewport::new(600.0, 400.0),
        &mut bundled_face_shaper(),
    );
    cx.rebuild_hit_test(document);
    (cx, switch)
}

fn switch_view(
    label: &str,
    hint: Option<&str>,
    position: nana_ui_core::SwitchControlPosition,
    size: nana_ui_core::ControlSize,
    width: Option<LengthSpec>,
) -> crate::Switch {
    let mut view = crate::Switch::new(label, false)
        .size(size)
        .control_position(position);
    if let Some(hint) = hint {
        view = view.hint(hint);
    }
    Arc::make_mut(&mut view.style.layout).width = width;
    view
}

/// A switch's track, label and hint are Flex parts layout publishes (#205):
/// the track and a label-over-hint column in one row, the column a nested
/// Flex column. Intrinsic measure gives the box its height from the same
/// declaration, so the style keeps no min-height of its own, and paint reads
/// the published boxes.
#[test]
fn issue205_switch_track_label_and_hint_are_published_parts() {
    use nana_ui_core::{ControlSize, SwitchControlPosition};
    let metrics = nana_ui_core::UI_METRICS.switch;
    let typography = nana_ui_core::TypographyTokens::default();
    let line = |font: f32| (font * typography.line / typography.body).max(font);
    let label_line = line(ControlSize::Medium.text_size());
    let hint_line = line(ControlSize::Medium.caption_size());
    let xxs = nana_ui_core::space::XXS;
    for position in [SwitchControlPosition::Start, SwitchControlPosition::End] {
        let (cx, switch) = switch_page(
            switch_view(
                "Wi-Fi",
                Some("Connect automatically to known networks"),
                position,
                ControlSize::Medium,
                None,
            ),
            None,
        );
        let id = switch.stable_id();
        let world = cx.world();
        let result = world.layout_result(id).unwrap();
        let content = result.content_box;
        let end = position == SwitchControlPosition::End;
        let track = part_of(
            &cx,
            id,
            if end {
                LayoutPartKind::TrailingContent
            } else {
                LayoutPartKind::LeadingContent
            },
        );
        let label = part_of(&cx, id, LayoutPartKind::Label);
        let hint = part_of(&cx, id, LayoutPartKind::Hint);
        // The track sits at its edge, centred on the row; the column takes
        // the rest beyond the gap.
        assert_eq!(
            (track.width, track.height),
            (metrics.track_width, metrics.track_height)
        );
        assert_eq!(track.y, content.y + (content.height - track.height) / 2.0);
        let column_x = if end {
            assert_eq!(track.x + track.width, content.x + content.width);
            content.x
        } else {
            assert_eq!(track.x, content.x);
            content.x + metrics.track_width + metrics.label_gap
        };
        let column_width = content.width - metrics.track_width - metrics.label_gap;
        for part in [label, hint] {
            assert_eq!((part.x, part.width), (column_x, column_width));
        }
        // The label line, the hint gap and the hint line stack down the
        // column, and intrinsic measure made the content box exactly that
        // tall: no style min-height holds it open.
        assert_eq!((label.y, label.height), (content.y, label_line));
        assert!((hint.y - (content.y + label_line + xxs)).abs() < 1e-4);
        assert!((hint.height - hint_line).abs() < 1e-4);
        assert!((content.height - (label_line + xxs + hint_line)).abs() < 1e-4);
        let style = world.node_style(id).unwrap();
        assert_eq!(style.layout.min_height, None);
        assert_eq!(
            style.control_height,
            Some(nana_ui_core::ControlHeight::Min(ControlSize::Medium))
        );
        // Paint reads the parts and nothing else.
        let ComponentGeometry::Switch {
            control,
            label: painted_label,
            hint: painted_hint,
            ..
        } = world.component_geometry(id).unwrap()
        else {
            panic!("switch")
        };
        assert_eq!(control, track);
        assert_eq!(painted_label.bounds, label);
        assert_eq!(painted_hint.unwrap().bounds, hint);
        // One entity; the content box, the track, the label and the hint.
        assert_eq!(world.len(), 2);
        assert_eq!(result.parts.len(), 4);
    }

    // Without a hint the label is the whole column; the control size is
    // the floor the row cannot go under.
    let (cx, switch) = switch_page(
        switch_view(
            "Wi-Fi",
            None,
            SwitchControlPosition::Start,
            ControlSize::Medium,
            None,
        ),
        None,
    );
    let id = switch.stable_id();
    let content = cx.world().layout_result(id).unwrap().content_box;
    let label = part_of(&cx, id, LayoutPartKind::Label);
    assert_eq!((label.y, label.height), (content.y, content.height));
    assert_eq!(
        cx.world().layout_box(id).unwrap().height,
        ControlSize::Medium.height_in(nana_ui_core::UI_METRICS)
    );
    assert_eq!(cx.world().layout_result(id).unwrap().parts.len(), 3);

    // A switch whose width is its content's holds its track and gap: the
    // track used to collapse to nothing in an auto-width box.
    let (cx, switch) = switch_page(
        switch_view(
            "Wi-Fi",
            None,
            SwitchControlPosition::End,
            ControlSize::Medium,
            Some(LengthSpec::MaxContent),
        ),
        None,
    );
    let id = switch.stable_id();
    let track = part_of(&cx, id, LayoutPartKind::TrailingContent);
    assert_eq!(track.width, metrics.track_width);
    let result = cx.world().layout_result(id).unwrap();
    assert_eq!(
        result.content_box.width,
        metrics.track_width + metrics.label_gap
    );
}

/// Label, hint, side, size, width and theme changes republish the parts a
/// cold layout of the same switch would; toggling and hovering lay nothing
/// out.
#[test]
fn issue205_switch_parts_follow_changes_like_a_cold_layout_and_toggle_paints_only() {
    use nana_ui_core::{ControlSize, SwitchControlPosition};
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(600.0, 400.0);
    let mut shaper = bundled_face_shaper();
    let hint = "Connect automatically to known networks";
    let (mut cx, switch) = switch_page(
        switch_view(
            "",
            None,
            SwitchControlPosition::End,
            ControlSize::Medium,
            None,
        ),
        None,
    );
    for (label, hint, position, size, width) in [
        (
            "Wi-Fi",
            None,
            SwitchControlPosition::End,
            ControlSize::Medium,
            None,
        ),
        (
            "Wi-Fi",
            Some(hint),
            SwitchControlPosition::End,
            ControlSize::Medium,
            None,
        ),
        (
            "Wi-Fi",
            Some(hint),
            SwitchControlPosition::Start,
            ControlSize::Small,
            None,
        ),
        (
            "",
            Some(hint),
            SwitchControlPosition::Start,
            ControlSize::Large,
            Some(LengthSpec::Px(90.0)),
        ),
        (
            "Wi-Fi",
            None,
            SwitchControlPosition::End,
            ControlSize::Large,
            Some(LengthSpec::MaxContent),
        ),
        (
            "",
            None,
            SwitchControlPosition::End,
            ControlSize::Medium,
            None,
        ),
        (
            "Wi-Fi",
            Some(hint),
            SwitchControlPosition::End,
            ControlSize::Medium,
            None,
        ),
    ] {
        cx.update_component(switch, |view, _| {
            *view = switch_view(label, hint, position, size, width);
        })
        .unwrap();
        product_frame(&mut cx, document, viewport, &mut shaper);
        let (mut cold, cold_switch) =
            switch_page(switch_view(label, hint, position, size, width), None);
        assert_eq!(cold_switch.stable_id(), switch.stable_id());
        super::reflow_oracle::assert_matches_cold(&mut cx, &mut cold, document);
        assert_eq!(cx.world().len(), 2);
    }
    // A theme that moves the track, its gap, the hint gap and the line
    // ratio moves no style the switch resolves; its parts follow anyway.
    let dense = dense_switch_theme();
    cx.set_theme_definition(&dense).unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let (mut cold, _) = switch_page(
        switch_view(
            "Wi-Fi",
            Some(hint),
            SwitchControlPosition::End,
            ControlSize::Medium,
            None,
        ),
        Some(&dense),
    );
    super::reflow_oracle::assert_matches_cold(&mut cx, &mut cold, document);
    let id = switch.stable_id();
    assert_eq!(
        part_of(&cx, id, LayoutPartKind::TrailingContent).width,
        nana_ui_core::UI_METRICS.switch.track_width + 6.0
    );

    let generation = cx.world().layout_generation();
    let snapshot = Arc::clone(&cx.world().layout_results[&id]);
    cx.update_component(switch, |view, _| view.checked = true)
        .unwrap();
    let toggled = product_frame(&mut cx, document, viewport, &mut shaper);
    // The thumb travels for a while; every frame of it paints only.
    cx.advance_animations(Duration::from_millis(80));
    let travelled = product_frame(&mut cx, document, viewport, &mut shaper);
    cx.set_pointer_hover_at(document, 1, Some(id), Duration::from_millis(800))
        .unwrap();
    let hovered = product_frame(&mut cx, document, viewport, &mut shaper);
    for work in [toggled, travelled, hovered] {
        assert_eq!(
            (work.layout_measure_nodes, work.layout_placement_nodes),
            (0, 0)
        );
    }
    assert_eq!(cx.world().layout_generation(), generation);
    assert!(Arc::ptr_eq(&snapshot, &cx.world().layout_results[&id]));
    let a11y = cx.world().project_accessibility(document);
    let node = a11y.iter().find(|node| node.id == id).unwrap();
    assert_eq!(node.checked, Some(true));
}

/// A checked, focused switch moved between Grid, Inline and Flex parents
/// keeps its entity, state and focus, and publishes the parts a cold layout
/// in that context would.
#[test]
fn issue205_switch_parts_survive_context_transitions() {
    use super::reflow_oracle::assert_matches_cold;
    use crate::MutationQueue;
    use nana_ui_core::DisplaySpec;
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    let build = |display: DisplaySpec| {
        let mut cx = AppContext::new();
        let root = cx
            .create_component(
                document,
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(320.0));
                    layout.display = Some(display);
                }),
            )
            .unwrap();
        let switch = cx
            .create_detached_component(
                document,
                crate::Switch::new("Wi-Fi", true).hint("Connect automatically"),
            )
            .unwrap();
        cx.append_child(root, switch).unwrap();
        let mut q = MutationQueue::new();
        q.request_focus(document, Some(switch.stable_id()));
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document, viewport, &mut bundled_face_shaper());
        (cx, root, switch)
    };
    let mut shaper = bundled_face_shaper();
    let (mut cx, root, switch) = build(DisplaySpec::Flex);
    let id = switch.stable_id();
    for display in [DisplaySpec::Grid, DisplaySpec::Inline, DisplaySpec::Flex] {
        let mut style = cx.world().node_style(root.stable_id()).unwrap().clone();
        Arc::make_mut(&mut style.layout).display = Some(display);
        let mut q = MutationQueue::new();
        q.set_style(root.stable_id(), style);
        cx.commit_mutations(q).unwrap();
        let work = product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!((work.entities_spawned, work.entities_despawned), (0, 0));
        assert!(cx.read(switch, |view| view.checked).unwrap());
        assert_eq!(cx.world().focused(document), Some(id));
        let ComponentGeometry::Switch {
            control,
            label,
            hint,
            ..
        } = cx.world().component_geometry(id).unwrap()
        else {
            panic!("switch")
        };
        assert_eq!(control, part_of(&cx, id, LayoutPartKind::TrailingContent));
        assert_eq!(label.bounds, part_of(&cx, id, LayoutPartKind::Label));
        assert_eq!(hint.unwrap().bounds, part_of(&cx, id, LayoutPartKind::Hint));
        let (mut cold, ..) = build(display);
        assert_matches_cold(&mut cx, &mut cold, document);
    }
}

#[test]
fn issue205_ten_thousand_switches_keep_one_entity_and_bounded_parts() {
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    for count in [1_000, 10_000] {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let viewport = LayoutViewport::new(600.0, 400.0);
        let root = cx.create_component(document, Stack::column(0.0)).unwrap();
        let mut children = Vec::with_capacity(count);
        for index in 0..count {
            let view = if index % 2 == 0 {
                crate::Switch::new("Wi-Fi", false)
            } else {
                crate::Switch::new("Wi-Fi", true).hint("Connect automatically")
            };
            children.push(
                cx.create_detached_component(document, view)
                    .unwrap()
                    .stable_id(),
            );
        }
        let mut q = crate::MutationQueue::new();
        for &id in &children {
            q.insert(root.stable_id(), id, None);
        }
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!(cx.world().len(), count + 1);
        for (index, &id) in children.iter().enumerate() {
            let result = cx.world().layout_result(id).unwrap();
            let kinds: Vec<_> = result.parts.iter().map(|part| part.kind).collect();
            let mut expected = vec![
                LayoutPartKind::ComponentContent,
                LayoutPartKind::TrailingContent,
                LayoutPartKind::Label,
            ];
            if index % 2 == 1 {
                expected.push(LayoutPartKind::Hint);
            }
            assert_eq!(kinds, expected);
            assert!(result.child_placements.is_empty());
        }
        let generation = cx.world().layout_generation();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert!(cx.take_system_work().is_empty());
        let idle = cx.world().frame_counters;
        assert_eq!(
            (
                idle.layout_measure_nodes,
                idle.layout_placement_nodes,
                idle.entities_spawned,
                idle.entities_despawned
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(cx.world().layout_generation(), generation);
    }
}

/// A theme that moves the spacing a checkbox's indicator gap reads: its
/// parts and its paragraph budget follow, as a cold layout under that theme
/// has them.
#[test]
fn issue205_checkbox_parts_follow_a_theme_that_moves_no_style() {
    use nana_ui_core::ControlSize;
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(600.0, 400.0);
    let mut theme =
        nana_ui_core::ThemeDefinition::for_appearance(nana_ui_core::ThemeAppearance::Light);
    theme.tokens.spacing.md += 6.0;
    let theme = theme.bump();
    let long = "Notify me about every reply in this thread";
    let (mut cx, checkbox) = checkbox_page(long, ControlSize::Medium, Some(160.0));
    cx.set_theme_definition(&theme).unwrap();
    product_frame(&mut cx, document, viewport, &mut bundled_face_shaper());
    let mut cold = AppContext::new();
    cold.set_theme_definition(&theme).unwrap();
    let root = cold
        .create_component(document, Stack::row(0.0).width(LengthSpec::Px(400.0)))
        .unwrap();
    let cold_box = cold
        .create_detached_component(
            document,
            checkbox_view(long, ControlSize::Medium, Some(160.0)),
        )
        .unwrap();
    cold.append_child(root, cold_box).unwrap();
    product_frame(&mut cold, document, viewport, &mut bundled_face_shaper());
    assert_eq!(cold_box.stable_id(), checkbox.stable_id());
    super::reflow_oracle::assert_matches_cold(&mut cx, &mut cold, document);
    let id = checkbox.stable_id();
    let indicator = part_of(&cx, id, LayoutPartKind::LeadingContent);
    let label = part_of(&cx, id, LayoutPartKind::TextContent);
    assert_eq!(
        label.x - (indicator.x + indicator.width),
        nana_ui_core::space::MD + 6.0
    );
    let (_, layout) = cx.world().text_layout(id).unwrap();
    assert!(layout.physical_size().0 <= label.width + 0.5);
}

/// A status badge (`kind` 0) or validation message (`kind` 1) saying
/// `label`, `width` wide, in a 240px row beside a button, under `theme`
/// when given, laid out once.
fn status_dot_page(
    kind: u8,
    label: &str,
    compact: bool,
    width: Option<LengthSpec>,
    theme: Option<&nana_ui_core::ThemeDefinition>,
) -> (AppContext, crate::StableNodeId) {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    if let Some(theme) = theme {
        cx.set_theme_definition(theme).unwrap();
    }
    let root = cx
        .create_component(document, Stack::row(4.0).width(LengthSpec::Px(240.0)))
        .unwrap();
    let id = status_dot_view(&mut cx, kind, label, compact, width);
    cx.append_child_id(root.stable_id(), id);
    let sibling = cx
        .create_detached_component(document, Button::new("Retry"))
        .unwrap();
    cx.append_child(root, sibling).unwrap();
    product_frame(
        &mut cx,
        document,
        LayoutViewport::new(600.0, 400.0),
        &mut bundled_face_shaper(),
    );
    cx.rebuild_hit_test(document);
    (cx, id)
}

fn status_badge_view(label: &str, compact: bool, width: Option<LengthSpec>) -> crate::StatusBadge {
    let mut view =
        crate::StatusBadge::new(label, nana_ui_core::StatusTone::Success).compact(compact);
    Arc::make_mut(&mut view.style.layout).width = width;
    view
}

fn validation_view(
    label: &str,
    compact: bool,
    width: Option<LengthSpec>,
) -> crate::ValidationMessage {
    let mut view = crate::ValidationMessage::new(label, nana_ui_core::ValidationIntent::Danger)
        .compact(compact);
    Arc::make_mut(&mut view.style.layout).width = width;
    view
}

fn status_dot_view(
    cx: &mut AppContext,
    kind: u8,
    label: &str,
    compact: bool,
    width: Option<LengthSpec>,
) -> crate::StableNodeId {
    let document = DocumentId::new(1).unwrap();
    if kind == 0 {
        cx.create_detached_component(document, status_badge_view(label, compact, width))
            .unwrap()
            .stable_id()
    } else {
        cx.create_detached_component(document, validation_view(label, compact, width))
            .unwrap()
            .stable_id()
    }
}

/// The slot, gap and left padding a status dot control uses.
fn status_dot_metrics(kind: u8, compact: bool) -> (f32, f32, f32) {
    use nana_ui_core::space;
    match (kind, compact) {
        (0, true) => (space::SM, space::XS, space::SM),
        (0, false) => (space::MD, space::SM, space::MD),
        (_, true) => (space::XL, space::XS, 0.0),
        (_, false) => (space::XXL, space::SM, 0.0),
    }
}

/// A status badge's and a validation message's dot and label are Flex parts
/// layout publishes (#205): the dot is centred in its slot, the label takes
/// the rest, and intrinsic measure, the paragraph budget, paint and the
/// text fragments all read them. The padding is only the control's own
/// inset; it keeps no copy of the dot's room.
#[test]
fn issue205_status_dot_and_label_are_published_parts() {
    for kind in [0, 1] {
        for compact in [true, false] {
            let (cx, id) = status_dot_page(kind, "Saved", compact, None, None);
            let world = cx.world();
            let result = world.layout_result(id).unwrap();
            let content = result.content_box;
            let (slot, gap, inset) = status_dot_metrics(kind, compact);
            let diameter = slot * super::content_parts::STATUS_DOT_FRACTION;
            let dot = part_of(&cx, id, LayoutPartKind::LeadingContent);
            let label = part_of(&cx, id, LayoutPartKind::TextContent);
            assert!((dot.x - (content.x + (slot - diameter) / 2.0)).abs() < 1e-4);
            assert!((dot.y - (content.y + (content.height - diameter) / 2.0)).abs() < 1e-4);
            assert_eq!((dot.width, dot.height), (diameter, diameter));
            assert!((label.x - (content.x + slot + gap)).abs() < 1e-4);
            assert!((label.x + label.width - (content.x + content.width)).abs() < 1e-4);
            assert_eq!((label.y, label.height), (content.y, content.height));
            assert_eq!(content.x - result.bounds.x, inset);
            // Measure reads the same parts: a badge as wide as its content is
            // the dot's slot, the gap and the label's text.
            let text = world.text_metrics(id).unwrap();
            assert!(
                (label.width - text.width).abs() < 0.01,
                "label part {} != text {}",
                label.width,
                text.width
            );
            // Paint reads them, nothing else.
            let (indicator, painted) = match world.component_geometry(id).unwrap() {
                ComponentGeometry::StatusBadge {
                    indicator, label, ..
                }
                | ComponentGeometry::ValidationMessage {
                    indicator, label, ..
                } => (indicator, label),
                other => panic!("{other:?}"),
            };
            assert_eq!(indicator, dot);
            // A badge paints its one centred line across the whole pill.
            let paint_rows = if kind == 0 {
                (result.bounds.y, result.bounds.height)
            } else {
                (label.y, label.height)
            };
            assert_eq!(
                painted.bounds,
                crate::LayoutBox {
                    y: paint_rows.0,
                    height: paint_rows.1,
                    ..label
                }
            );
            let runs: Vec<_> = result
                .fragments
                .iter()
                .filter(|fragment| fragment.kind == crate::LayoutFragmentKind::TextRun)
                .collect();
            assert!(!runs.is_empty());
            for run in runs {
                assert!(
                    run.bounds.x >= label.x - 0.01
                        && run.bounds.x + run.bounds.width <= label.x + label.width + 0.01,
                    "{run:?} outside {label:?}"
                );
            }
            // One entity: the text, the content box and the dot.
            assert_eq!(world.len(), 3);
            assert_eq!(result.parts.len(), 3);
        }
        // A label too long for the row wraps in the label part, and the box
        // does not push past the row by the dot's room.
        let long = "Every reply in this thread was saved to the archive just now";
        let (cx, id) = status_dot_page(kind, long, false, None, None);
        let label = part_of(&cx, id, LayoutPartKind::TextContent);
        let (_, layout) = cx.world().text_layout(id).unwrap();
        assert!(layout.physical_size().0 <= label.width + 0.5);
        assert!(cx.world().layout_box(id).unwrap().width <= 240.0);
    }
}

/// A control's text asks intrinsic measure for the row less the parts
/// beside it, as its paragraph budget has it: a checkbox whose label could
/// fill the row stays in the row instead of overflowing it by its indicator
/// and gap.
#[test]
fn issue205_a_checkbox_text_asks_for_the_row_less_its_indicator() {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let root = cx
        .create_component(document, Stack::row(4.0).width(LengthSpec::Px(240.0)))
        .unwrap();
    let checkbox = cx
        .create_detached_component(
            document,
            crate::Checkbox::new(
                "Every reply in this thread was saved to the archive just now",
                false,
            ),
        )
        .unwrap();
    cx.append_child(root, checkbox).unwrap();
    product_frame(
        &mut cx,
        document,
        LayoutViewport::new(600.0, 400.0),
        &mut bundled_face_shaper(),
    );
    assert!(cx.world().layout_box(checkbox.stable_id()).unwrap().width <= 240.0);
}

/// Label, density and width changes, and a theme that moves the slot and
/// the gap, republish what a cold layout would; a tone or intent change
/// paints only.
#[test]
fn issue205_status_dot_parts_follow_changes_like_a_cold_layout() {
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(600.0, 400.0);
    let long = "Every reply in this thread was saved to the archive just now";
    let mut spacing =
        nana_ui_core::ThemeDefinition::for_appearance(nana_ui_core::ThemeAppearance::Light);
    spacing.tokens.spacing.sm += 2.0;
    spacing.tokens.spacing.md += 3.0;
    spacing.tokens.spacing.xl += 4.0;
    spacing.tokens.spacing.xxl += 4.0;
    spacing.tokens.spacing.xs += 1.0;
    let spacing = spacing.bump();
    for kind in [0u8, 1] {
        let mut shaper = bundled_face_shaper();
        let (mut cx, id) = status_dot_page(kind, "Saved", true, None, None);
        for (label, compact, width) in [
            ("Saved", false, None),
            (long, false, None),
            (long, true, Some(LengthSpec::Px(60.0))),
            ("", true, None),
            ("Saved", false, Some(LengthSpec::Px(200.0))),
        ] {
            cx.update_view_dyn(id, kind, label, compact, width);
            product_frame(&mut cx, document, viewport, &mut shaper);
            let (mut cold, cold_id) = status_dot_page(kind, label, compact, width, None);
            assert_eq!(cold_id, id);
            super::reflow_oracle::assert_matches_cold(&mut cx, &mut cold, document);
        }
        cx.set_theme_definition(&spacing).unwrap();
        product_frame(&mut cx, document, viewport, &mut shaper);
        let (mut cold, _) = status_dot_page(
            kind,
            "Saved",
            false,
            Some(LengthSpec::Px(200.0)),
            Some(&spacing),
        );
        super::reflow_oracle::assert_matches_cold(&mut cx, &mut cold, document);

        let generation = cx.world().layout_generation();
        let snapshot = Arc::clone(&cx.world().layout_results[&id]);
        if kind == 0 {
            let entity = crate::Entity::<crate::StatusBadge>::from_stable_id(id);
            cx.update_component(entity, |view, _| {
                view.tone = nana_ui_core::StatusTone::Danger
            })
            .unwrap();
        } else {
            let entity = crate::Entity::<crate::ValidationMessage>::from_stable_id(id);
            cx.update_component(entity, |view, _| {
                view.intent = nana_ui_core::ValidationIntent::Warning
            })
            .unwrap();
        }
        let work = product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!(
            (work.layout_measure_nodes, work.layout_placement_nodes),
            (0, 0)
        );
        assert_eq!(cx.world().layout_generation(), generation);
        assert!(Arc::ptr_eq(&snapshot, &cx.world().layout_results[&id]));
    }
}

/// Status dots moved between Grid, Inline and Flex parents keep their
/// entities and publish the parts a cold layout in that context would.
#[test]
fn issue205_status_dot_parts_survive_context_transitions() {
    use super::reflow_oracle::assert_matches_cold;
    use crate::MutationQueue;
    use nana_ui_core::DisplaySpec;
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(400.0, 300.0);
    let build = |display: DisplaySpec| {
        let mut cx = AppContext::new();
        let root = cx
            .create_component(
                document,
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(320.0));
                    layout.display = Some(display);
                }),
            )
            .unwrap();
        let badge = status_dot_view(&mut cx, 0, "Saved", false, None);
        let message = status_dot_view(&mut cx, 1, "Name is required", true, None);
        cx.append_child_id(root.stable_id(), badge);
        cx.append_child_id(root.stable_id(), message);
        product_frame(&mut cx, document, viewport, &mut bundled_face_shaper());
        (cx, root, [badge, message])
    };
    let mut shaper = bundled_face_shaper();
    let (mut cx, root, ids) = build(DisplaySpec::Flex);
    for display in [DisplaySpec::Grid, DisplaySpec::Inline, DisplaySpec::Flex] {
        let mut style = cx.world().node_style(root.stable_id()).unwrap().clone();
        Arc::make_mut(&mut style.layout).display = Some(display);
        let mut q = MutationQueue::new();
        q.set_style(root.stable_id(), style);
        cx.commit_mutations(q).unwrap();
        let work = product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!((work.entities_spawned, work.entities_despawned), (0, 0));
        for id in ids {
            let (indicator, label) = match cx.world().component_geometry(id).unwrap() {
                ComponentGeometry::StatusBadge {
                    indicator, label, ..
                }
                | ComponentGeometry::ValidationMessage {
                    indicator, label, ..
                } => (indicator, label),
                other => panic!("{other:?}"),
            };
            assert_eq!(indicator, part_of(&cx, id, LayoutPartKind::LeadingContent));
            let part = part_of(&cx, id, LayoutPartKind::TextContent);
            assert_eq!((label.bounds.x, label.bounds.width), (part.x, part.width));
        }
        let (mut cold, ..) = build(display);
        assert_matches_cold(&mut cx, &mut cold, document);
    }
}

#[test]
fn issue205_ten_thousand_status_dots_keep_one_entity_and_three_parts() {
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    for count in [1_000, 10_000] {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let viewport = LayoutViewport::new(600.0, 400.0);
        let root = cx.create_component(document, Stack::column(0.0)).unwrap();
        let children: Vec<_> = (0..count)
            .map(|index| status_dot_view(&mut cx, (index % 2) as u8, "Saved", true, None))
            .collect();
        let mut q = crate::MutationQueue::new();
        for &id in &children {
            q.insert(root.stable_id(), id, None);
        }
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!(cx.world().len(), count + 1);
        for &id in &children {
            let result = cx.world().layout_result(id).unwrap();
            let kinds: Vec<_> = result.parts.iter().map(|part| part.kind).collect();
            assert_eq!(
                kinds,
                [
                    LayoutPartKind::TextContent,
                    LayoutPartKind::ComponentContent,
                    LayoutPartKind::LeadingContent
                ]
            );
        }
        let generation = cx.world().layout_generation();
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert!(cx.take_system_work().is_empty());
        let idle = cx.world().frame_counters;
        assert_eq!(
            (
                idle.layout_measure_nodes,
                idle.layout_placement_nodes,
                idle.entities_spawned,
                idle.entities_despawned
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(cx.world().layout_generation(), generation);
    }
}

trait StatusDotUpdate {
    fn append_child_id(&mut self, parent: crate::StableNodeId, child: crate::StableNodeId);
    fn update_view_dyn(
        &mut self,
        id: crate::StableNodeId,
        kind: u8,
        label: &str,
        compact: bool,
        width: Option<LengthSpec>,
    );
}

impl StatusDotUpdate for AppContext {
    fn append_child_id(&mut self, parent: crate::StableNodeId, child: crate::StableNodeId) {
        let mut q = crate::MutationQueue::new();
        q.insert(parent, child, None);
        self.commit_mutations(q).unwrap();
    }

    fn update_view_dyn(
        &mut self,
        id: crate::StableNodeId,
        kind: u8,
        label: &str,
        compact: bool,
        width: Option<LengthSpec>,
    ) {
        if kind == 0 {
            let entity = crate::Entity::<crate::StatusBadge>::from_stable_id(id);
            self.update_component(entity, |view, _| {
                *view = status_badge_view(label, compact, width)
            })
            .unwrap();
        } else {
            let entity = crate::Entity::<crate::ValidationMessage>::from_stable_id(id);
            self.update_component(entity, |view, _| {
                *view = validation_view(label, compact, width)
            })
            .unwrap();
        }
    }
}

/// An open context menu's height is the one its projection writes, from
/// the rows it shows against the installed theme. The component used to
/// write its own copy from the default metrics whenever it re-anchored,
/// which the projection always overwrote; that copy is gone.
#[test]
fn issue205_context_menu_height_is_the_projections_alone() {
    use crate::{ContextMenu, ContextMenuItem};
    let document = DocumentId::new(1).unwrap();
    let viewport = LayoutViewport::new(600.0, 400.0);
    let mut shaper = bundled_face_shaper();
    let mut cx = AppContext::new();
    let root = cx.create_component(document, Stack::column(0.0)).unwrap();
    let menu = cx
        .create_detached_component(
            document,
            ContextMenu::new(24.0, 36.0)
                .items([
                    ContextMenuItem::new("rename", "Rename"),
                    ContextMenuItem::new("delete", "Delete"),
                    ContextMenuItem::new("share", "Share"),
                ])
                .searchable(true),
        )
        .unwrap();
    cx.append_child(root, menu).unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let id = menu.stable_id();
    let expected = |cx: &AppContext, items: usize| {
        crate::menus::context_menu_height_in(
            items,
            true,
            cx.world().theme_metrics(),
            cx.world().theme(),
        )
    };
    let height = |cx: &AppContext| cx.world().layout_box(id).unwrap().height;
    assert!(
        cx.read(menu, |view| view.style.layout.height.is_none())
            .unwrap()
    );
    assert_eq!(height(&cx), expected(&cx, 3));
    // Filtering re-anchors the component; the box follows the rows shown.
    cx.update_component(menu, |view, _| *view = view.clone().query("re"))
        .unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    let shown = cx.read(menu, |view| view.visible_items().len()).unwrap();
    assert!(shown < 3);
    assert!(
        cx.read(menu, |view| view.style.layout.height.is_none())
            .unwrap()
    );
    assert_eq!(height(&cx), expected(&cx, shown));
    // Taller rows from the theme reach it, as the projection reads them.
    let mut theme =
        nana_ui_core::ThemeDefinition::for_appearance(nana_ui_core::ThemeAppearance::Light);
    theme.tokens.metrics.compact_control_height += 6.0;
    cx.set_theme_definition(&theme.bump()).unwrap();
    product_frame(&mut cx, document, viewport, &mut shaper);
    assert_eq!(height(&cx), expected(&cx, shown));
    assert!(
        height(&cx)
            > crate::menus::context_menu_height_in(
                shown,
                true,
                nana_ui_core::UI_METRICS,
                &nana_ui_core::CompiledTheme::default(),
            )
    );
}
