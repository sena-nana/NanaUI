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
