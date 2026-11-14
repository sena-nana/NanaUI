//! Real control measurements and Scene chrome follow one installed theme.
use nana_ui::{HeadlessInput, NanaTextShaper, PointerPhase, runtime::*};
use nana_ui_core::{ControlSize, LengthSpec, SwitchControlPosition, ThemeDefinition};
use nana_ui_platform::{InputModifiers, InputPayload, KeyInput, KeyState};
use nana_ui_scene::{ScenePrimitive, ScenePrimitiveKind, UiScene};

fn settle(cx: &mut AppContext, doc: DocumentId, shaper: &mut NanaTextShaper) {
    for _ in 0..8 {
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        cx.shape_text(&work.text, shaper).unwrap();
        cx.layout_document(doc, LayoutViewport::new(800.0, 600.0))
            .unwrap();
        if cx.shape_text_for_layout(doc, shaper).unwrap() {
            cx.layout_document(doc, LayoutViewport::new(800.0, 600.0))
                .unwrap();
        }
    }
    cx.rebuild_hit_test(doc);
}

fn primitive(scene: &UiScene, node: StableNodeId, slot: u64) -> &ScenePrimitive {
    scene
        .primitives()
        .find(|p| p.node == node && p.id.slot == slot)
        .unwrap()
}

#[test]
fn checkbox_indicator_label_and_check_follow_the_theme_without_losing_focus_or_toggle() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(931).unwrap();
    let root = cx
        .create_component(doc, Stack::column(0.0).align(AlignSpec::Start))
        .unwrap();
    let checkbox = cx
        .create_detached_component(
            doc,
            Checkbox::new("隐藏滚动弹幕", true).size(ControlSize::Small),
        )
        .unwrap();
    cx.append_child(root, checkbox).unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    cx.focus_node_in_place(doc, checkbox.stable_id()).unwrap();
    let original = cx.world().layout_box(checkbox.stable_id()).unwrap();
    let mut theme = ThemeDefinition::NANA_DARK;
    theme.tokens.spacing.xxl = 30.0;
    theme.tokens.spacing.sm = 12.0;
    theme.typography.meta = 24.0;
    theme.typography.line = 32.0;
    theme.typography.bold = 800;
    theme.tokens.border.hairline = 3.0;
    theme.tokens.metrics.radius_sm = 10.0;
    cx.set_theme_definition(&theme).unwrap();
    settle(&mut cx, doc, &mut shaper);
    let Some(ComponentGeometry::Checkbox {
        indicator, label, ..
    }) = cx.world().component_geometry(checkbox.stable_id())
    else {
        panic!("resolved checkbox");
    };
    let changed = cx.world().layout_box(checkbox.stable_id()).unwrap();
    assert!(changed.width > original.width && changed.height > original.height);
    assert_eq!(cx.world().focused(doc), Some(checkbox.stable_id()));
    assert_eq!(indicator.width, 30.0);
    assert!((label.bounds.x - changed.x - 42.0).abs() < 0.5);
    let natural = shaper.shape(
        checkbox.stable_id(),
        &TextContent {
            value: "隐藏滚动弹幕".into(),
        },
        cx.world().computed_style(checkbox.stable_id()).unwrap(),
        TextShapeConstraints::default(),
    );
    assert!(label.bounds.width + 0.5 >= natural.width);
    let retained = cx
        .world()
        .extract_document(doc)
        .into_iter()
        .find(|node| node.id == checkbox.stable_id())
        .unwrap()
        .text_layout
        .unwrap();
    assert!((retained.layout.constraints.max_width_px.unwrap() - label.bounds.width).abs() < 0.5);
    let mut scene = UiScene::new();
    scene.apply_delta(cx.world().extract_document(doc), []);
    let painted = primitive(&scene, checkbox.stable_id(), 3);
    assert_eq!(painted.bounds.width, indicator.width);
    let ScenePrimitiveKind::Quad {
        border_width,
        corner_radius,
        ..
    } = &painted.kind
    else {
        panic!("indicator quad");
    };
    assert_eq!(*border_width, 3.0);
    assert_eq!(*corner_radius, [10.0; 4]);
    let ScenePrimitiveKind::Text { weight, .. } = &primitive(&scene, checkbox.stable_id(), 4).kind
    else {
        panic!("check glyph");
    };
    assert_eq!(*weight, Some(800));
    let label_paint = primitive(&scene, checkbox.stable_id(), 2);
    assert!((label_paint.bounds.x - label.bounds.x).abs() < 0.5);
    let mut input = HeadlessInput::bind(&mut cx, doc);
    let (x, y) = (
        label.bounds.x + 2.0,
        label.bounds.y + label.bounds.height / 2.0,
    );
    input.pointer(&mut cx, PointerPhase::Down, x, y).unwrap();
    input.pointer(&mut cx, PointerPhase::Up, x, y).unwrap();
    assert!(!cx.read(checkbox, |checkbox| checkbox.checked).unwrap());
    assert_eq!(cx.world().focused(doc), Some(checkbox.stable_id()));
}

#[test]
fn switch_thumb_and_range_rail_paint_their_resolved_theme_geometry() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(932).unwrap();
    let root = cx
        .create_component(doc, Stack::column(8.0).width(LengthSpec::Px(500.0)))
        .unwrap();
    let switches = [SwitchControlPosition::Start, SwitchControlPosition::End].map(|position| {
        cx.create_detached_component(
            doc,
            Switch::new("弹幕", true)
                .hint("隐藏滚动弹幕")
                .size(ControlSize::Small)
                .control_position(position),
        )
        .unwrap()
    });
    for switch in switches {
        cx.append_child(root, switch).unwrap();
    }
    let range = cx
        .create_detached_component(
            doc,
            RangeField::new(50.0, 0.0, 100.0, 1.0).size(ControlSize::Small),
        )
        .unwrap();
    cx.append_child(root, range).unwrap();
    let rail = cx
        .create_detached_component(
            doc,
            RangeField::new(50.0, 0.0, 100.0, 1.0)
                .size(ControlSize::Small)
                .rail(2.0)
                .show_value(false),
        )
        .unwrap();
    cx.append_child(root, rail).unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    cx.focus_node_in_place(doc, rail.stable_id()).unwrap();
    let mut input = HeadlessInput::bind(&mut cx, doc);
    input
        .route(
            &mut cx,
            InputPayload::Key(KeyInput::named(
                "ArrowRight",
                "ArrowRight",
                KeyState::Pressed,
                InputModifiers::default(),
            )),
        )
        .unwrap();
    assert_eq!(cx.read(rail, |rail| rail.value).unwrap(), 51.0);
    settle(&mut cx, doc, &mut shaper);
    let mut theme = ThemeDefinition::NANA_DARK;
    theme.tokens.metrics.switch.track_width = 60.0;
    theme.tokens.metrics.switch.track_height = 32.0;
    theme.tokens.metrics.switch.label_gap = 16.0;
    theme.typography.meta = 24.0;
    theme.typography.line = 32.0;
    theme.tokens.spacing.xs = 8.0;
    theme.tokens.border.hairline = 3.0;
    cx.set_theme_definition(&theme).unwrap();
    settle(&mut cx, doc, &mut shaper);
    let mut scene = UiScene::new();
    scene.apply_delta(cx.world().extract_document(doc), []);
    for switch in switches {
        let Some(ComponentGeometry::Switch {
            control,
            label,
            hint,
            ..
        }) = cx.world().component_geometry(switch.stable_id())
        else {
            panic!("switch geometry");
        };
        assert_eq!((control.width, control.height), (60.0, 32.0));
        let track = primitive(&scene, switch.stable_id(), 4);
        let thumb = primitive(&scene, switch.stable_id(), 5);
        // Where it stands on the page, as the geometry it is compared with.
        let thumb_box = scene.primitive_layout_bounds(thumb);
        assert_eq!((thumb_box.width, thumb_box.height), (20.0, 20.0));
        assert!((thumb_box.x + thumb_box.width - control.x - control.width + 6.0).abs() < 0.5);
        assert!((thumb_box.y - control.y - 6.0).abs() < 0.5);
        let ScenePrimitiveKind::Quad {
            border_width,
            corner_radius,
            ..
        } = &track.kind
        else {
            panic!("switch track");
        };
        assert_eq!(*border_width, 3.0);
        assert_eq!(*corner_radius, [16.0; 4]);
        let hint = hint.unwrap();
        let ScenePrimitiveKind::Text {
            line_height, size, ..
        } = &primitive(&scene, switch.stable_id(), 3).kind
        else {
            panic!("switch hint text");
        };
        assert_eq!(
            *line_height,
            Some(nana_ui_core::LineHeightSpec::Absolute(hint.bounds.height))
        );
        assert_eq!(*size, hint.font_size);
        assert!(
            label.bounds.x + label.bounds.width <= control.x - 16.0 + 0.5
                || label.bounds.x >= control.x + control.width + 16.0 - 0.5
        );
    }
    let Some(ComponentGeometry::Range {
        track, track_girth, ..
    }) = cx.world().component_geometry(range.stable_id())
    else {
        panic!("range geometry");
    };
    assert_eq!(track_girth, 8.0);
    assert_eq!(
        primitive(&scene, range.stable_id(), 3).bounds.height,
        track_girth
    );
    let thumb = primitive(&scene, range.stable_id(), 5);
    assert_eq!(thumb.bounds.height, track.height);
    let ScenePrimitiveKind::Quad { border_width, .. } = &thumb.kind else {
        panic!("range thumb");
    };
    assert_eq!(*border_width, 3.0);
    let Some(ComponentGeometry::Range {
        track, track_girth, ..
    }) = cx.world().component_geometry(rail.stable_id())
    else {
        panic!("rail-only geometry");
    };
    assert_eq!(track_girth, 2.0);
    assert_eq!(primitive(&scene, rail.stable_id(), 3).bounds.height, 2.0);
    assert_eq!(
        primitive(&scene, rail.stable_id(), 5).bounds.height,
        track.height
    );
    assert_eq!(track.height, 24.0);
    assert_eq!(cx.read(rail, |rail| rail.value).unwrap(), 51.0);
}

#[test]
fn a_bare_switch_answers_hover_on_its_track_not_with_a_box_wash() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(933).unwrap();
    let root = cx
        .create_component(doc, Stack::column(8.0).width(LengthSpec::Px(500.0)))
        .unwrap();
    let bare = cx
        .create_detached_component(doc, Switch::new("", false))
        .unwrap();
    cx.append_child(root, bare).unwrap();
    let labelled = cx
        .create_detached_component(doc, Switch::new("弹幕", false))
        .unwrap();
    cx.append_child(root, labelled).unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    let track = |cx: &AppContext, switch: StableNodeId| match cx.world().component_geometry(switch)
    {
        Some(ComponentGeometry::Switch {
            control,
            track_border,
            ..
        }) => (control, track_border),
        _ => panic!("switch geometry"),
    };
    let mut input = HeadlessInput::bind(&mut cx, doc);
    let mut hover = |cx: &mut AppContext, switch: StableNodeId, step: u64| {
        let (control, _) = track(cx, switch);
        input
            .pointer(
                cx,
                PointerPhase::Move,
                control.x + control.width / 2.0,
                control.y + control.height / 2.0,
            )
            .unwrap();
        cx.advance_animations(std::time::Duration::from_secs(step));
        settle(cx, doc, &mut shaper);
        cx.world().computed_style(switch).unwrap().background
    };

    let (_, resting) = track(&cx, bare.stable_id());
    assert_eq!(
        hover(&mut cx, bare.stable_id(), 1),
        None,
        "no box wash spreads past the pill"
    );
    let (_, hovered) = track(&cx, bare.stable_id());
    assert_ne!(hovered, resting, "the track itself shows the hover");

    assert!(
        hover(&mut cx, labelled.stable_id(), 2).is_some(),
        "a labelled switch keeps its row wash"
    );
}
