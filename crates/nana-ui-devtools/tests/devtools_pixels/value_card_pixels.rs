use nana_ui::runtime::{
    Button, Card, ComponentGeometry, DocumentId, LabeledValue, LengthSpec, RuntimeDocument, Stack,
    Text,
};
use nana_ui_devtools::{agent::RuntimeAgentSession, offscreen};
use std::sync::Arc;

#[test]
fn issue205_value_card_parts_preserve_content_pixels() {
    if !offscreen::pixels_available() {
        assert!(std::env::var_os("NANA_COMPONENT_REQUIRE_GPU").is_none());
        return;
    }
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let cx = doc.context_mut();
    cx.set_reduced_motion(true);
    let root = cx
        .create_component(
            document,
            Stack::column(8.0)
                .padding(12.0)
                .width(LengthSpec::Px(320.0)),
        )
        .unwrap();
    for width in [280.0, 160.0, 48.0] {
        let group = cx
            .create_detached_component(document, Stack::column(0.0).width(LengthSpec::Px(width)))
            .unwrap();
        cx.append_child(root, group).unwrap();
        let value = cx
            .create_detached_component(document, LabeledValue::new("Revision", "42"))
            .unwrap();
        cx.append_child(group, value).unwrap();
    }
    let group = cx
        .create_detached_component(document, Stack::column(0.0).width(LengthSpec::Px(280.0)))
        .unwrap();
    cx.append_child(root, group).unwrap();
    let action_value = cx
        .create_detached_component(document, LabeledValue::new("Revision", "42"))
        .unwrap();
    cx.append_child(group, action_value).unwrap();
    let action = cx
        .create_detached_component(document, Button::new("Edit"))
        .unwrap();
    cx.set_labeled_value_action(action_value, Some(action.stable_id()))
        .unwrap();
    let mut cards = Vec::new();
    for width in [280.0, 160.0] {
        let mut value = Card::new().title("Activity").loading(true);
        let layout = Arc::make_mut(&mut value.style.layout);
        layout.width = Some(LengthSpec::Px(width));
        layout.height = Some(LengthSpec::Px(80.0));
        let card = cx.create_detached_component(document, value).unwrap();
        cx.append_child(root, card).unwrap();
        let body = cx
            .create_detached_component(document, Text::new("Body"))
            .unwrap();
        cx.append_child(card, body).unwrap();
        cards.push(card.stable_id());
    }
    let mut session = RuntimeAgentSession::new(doc, 340, 380).unwrap();
    let (size, initial) = session.screenshot_rgba().unwrap();
    assert_eq!(initial, session.screenshot_rgba().unwrap().1);
    let world = session.document().context().world();
    let rect =
        |bounds: nana_ui::runtime::LayoutBox| [bounds.x, bounds.y, bounds.width, bounds.height];
    let mut masks = vec![rect(world.layout_box(action_value.stable_id()).unwrap())];
    for card in cards {
        let ComponentGeometry::Card { spinner, .. } = world.component_geometry(card).unwrap()
        else {
            panic!("card")
        };
        if let Some(spinner) = spinner {
            masks.push(rect(spinner));
        }
    }
    if let Some(path) = std::env::var_os("NANA_SURFACE_RECORD_BASELINE") {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path.with_extension("rgba"), &initial).unwrap();
        std::fs::write(
            path.with_extension("json"),
            serde_json::to_vec(&masks).unwrap(),
        )
        .unwrap();
        offscreen::write_png(&path.with_extension("png"), size, &initial).unwrap();
    }
    if let Some(path) = std::env::var_os("NANA_SURFACE_COMPARE_BASELINE") {
        let path = std::path::PathBuf::from(path);
        let expected = std::fs::read(path.with_extension("rgba")).unwrap();
        let before_masks: Vec<[f32; 4]> =
            serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap()).unwrap();
        masks.extend(before_masks);
        assert_eq!(initial.len(), expected.len());
        let changed = initial
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .enumerate()
            .filter(|(index, (a, b))| {
                if a == b {
                    return false;
                }
                let x = (*index % size.width as usize) as f32 + 0.5;
                let y = (*index / size.width as usize) as f32 + 0.5;
                !masks.iter().any(|rect| {
                    x >= rect[0] - 2.0
                        && x <= rect[0] + rect[2] + 2.0
                        && y >= rect[1] - 2.0
                        && y <= rect[1] + rect[3] + 2.0
                })
            })
            .count();
        assert_eq!(
            changed, 0,
            "unexpected changes outside the action/spinner fixes"
        );
    }
    let action_bounds = session
        .document()
        .context()
        .world()
        .layout_box(action.stable_id())
        .unwrap();
    session
        .document_mut()
        .context_mut()
        .update_component(action_value, |value, _| value.value = Arc::from("43"))
        .unwrap();
    let (_, updated) = session.screenshot_rgba().unwrap();
    assert!(
        initial != updated,
        "changing the value must change live glyph pixels"
    );
    let world = session.document().context().world();
    assert!(world.layout_result(action.stable_id()).is_some());
    assert_eq!(world.layout_box(action.stable_id()), Some(action_bounds));
    assert_eq!(
        session
            .document()
            .context()
            .last_work_counters()
            .layout_measure_nodes,
        0
    );
    assert!(updated == session.screenshot_rgba().unwrap().1);
}

#[test]
fn issue205_narrow_card_content_stays_inside_its_parts() {
    if !offscreen::pixels_available() {
        assert!(std::env::var_os("NANA_COMPONENT_REQUIRE_GPU").is_none());
        return;
    }
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let cx = doc.context_mut();
    cx.set_reduced_motion(true);
    let root = cx
        .create_component(
            document,
            Stack::column(4.0).padding(4.0).width(LengthSpec::Px(128.0)),
        )
        .unwrap();
    let mut cards = Vec::new();
    for width in [0.0, 4.0, 16.0, 32.0, 60.0, 120.0] {
        let mut model = Card::new().title("A long activity title").loading(true);
        let style = Arc::make_mut(&mut model.style.layout);
        style.width = Some(LengthSpec::Px(width));
        style.height = Some(LengthSpec::Px(64.0));
        let card = cx.create_detached_component(document, model).unwrap();
        cx.append_child(root, card).unwrap();
        cards.push(card.stable_id());
    }
    let mut session = RuntimeAgentSession::new(doc, 140, 420).unwrap();
    let (size, pixels) = session.screenshot_rgba().unwrap();
    let stride = size.width as usize;
    let pixel = |x: usize, y: usize| &pixels[(y * stride + x) * 4..(y * stride + x + 1) * 4];
    let world = session.document().context().world();
    let bounds = cards
        .iter()
        .map(|id| world.layout_box(*id).unwrap())
        .collect::<Vec<_>>();
    let background = pixel(139, 0);
    for y in 0..size.height as usize {
        for x in 0..stride {
            let inside = bounds.iter().any(|b| {
                x as f32 >= b.x - 1.0
                    && (x as f32) < b.x + b.width + 1.0
                    && y as f32 >= b.y - 1.0
                    && (y as f32) < b.y + b.height + 1.0
            });
            if !inside {
                assert_eq!(
                    pixel(x, y),
                    background,
                    "paint outside narrow card at {x},{y}"
                );
            }
        }
    }
    for id in cards {
        let result = world.layout_result(id).unwrap();
        let header = result
            .parts
            .iter()
            .find(|part| part.kind == nana_ui::runtime::LayoutPartKind::Header)
            .unwrap()
            .bounds;
        let spinner = result
            .parts
            .iter()
            .find(|part| part.kind == nana_ui::runtime::LayoutPartKind::Spinner)
            .unwrap()
            .bounds;
        if header.width <= 0.0 || spinner.width <= 0.0 {
            continue;
        }
        let surface = pixel(
            (result.bounds.x + 2.0) as usize,
            (result.bounds.y + result.bounds.height - 10.0) as usize,
        );
        for x in (header.x + header.width + 2.0).ceil() as usize..(spinner.x - 2.0).floor() as usize
        {
            for y in header.y.ceil() as usize..(header.y + header.height).floor() as usize {
                assert_eq!(
                    pixel(x, y),
                    surface,
                    "ellipsis must leave the spinner gap clear at {x},{y}"
                );
            }
        }
    }
}
