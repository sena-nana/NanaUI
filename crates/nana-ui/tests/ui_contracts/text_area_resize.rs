use nana_ui::{HeadlessInput, NanaTextShaper, runtime::*};
use nana_ui_platform::{
    InputModifiers, InputPayload, PointerId, PointerInput, PointerPhase, PointerType,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
fn pointer(phase: PointerPhase, x: f32, y: f32) -> InputPayload {
    InputPayload::Pointer(PointerInput {
        phase,
        pointer_id: PointerId(1),
        pointer_type: PointerType::Mouse,
        x,
        y,
        screen_x: x,
        screen_y: y,
        button: 0,
        buttons: u16::from(phase != PointerPhase::Up),
        pressure: 1.0,
        tangential_pressure: 0.0,
        tilt_x: 0,
        tilt_y: 0,
        twist: 0,
        is_primary: true,
        activation_click: false,
        modifiers: InputModifiers::default(),
    })
}

fn layout(cx: &mut AppContext, doc: DocumentId, area: Entity<TextArea>) {
    cx.resolve_styles(&[area.stable_id()]).unwrap();
    cx.shape_text(&[area.stable_id()], &mut NanaTextShaper::default())
        .unwrap();
    cx.layout_document(doc, LayoutViewport::new(500.0, 500.0))
        .unwrap();
    cx.rebuild_hit_test(doc);
}
fn grip(cx: &AppContext, area: Entity<TextArea>) -> (f32, f32) {
    let TextInputGeometry {
        resize_grip: Some(rect),
        ..
    } = cx.world().text_input_geometry(area.stable_id()).unwrap()
    else {
        panic!("resize geometry")
    };
    cx.world()
        .layout_pointer_position(area.stable_id(), rect.x + 7.0, rect.y + 7.0)
        .unwrap()
}

#[test]
fn textarea_resize_normal_pointer_clamps_preserves_text_and_cancel_restores_height() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let mut view = TextArea::new("unchanged")
        .height(160.0)
        .resize_vertical(true);
    let style = Arc::make_mut(&mut view.style.layout);
    style.width = Some(nana_ui_core::LengthSpec::Px(280.0));
    style.min_height = Some(nana_ui_core::LengthSpec::Px(148.0));
    style.max_height = Some(nana_ui_core::LengthSpec::Px(240.0));
    let area = cx.create_component(doc, view).unwrap();
    let events = Arc::new(AtomicUsize::new(0));
    let event_count = Arc::clone(&events);
    cx.on(area, move |_, _: &TextChanged, _| {
        event_count.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    layout(&mut cx, doc, area);
    let initial = cx.read(area, |view| view.state.clone()).unwrap();
    let mut adapter = HeadlessInput::bind(&mut cx, doc);
    let (x, y) = grip(&cx, area);
    assert!(
        adapter
            .route(&mut cx, pointer(PointerPhase::Down, x, y))
            .unwrap()
            .prevent_default
    );
    assert_eq!(cx.world().pointer_capture(doc, 1), Some(area.stable_id()));
    adapter
        .route(&mut cx, pointer(PointerPhase::Move, x + 100.0, y + 500.0))
        .unwrap();
    layout(&mut cx, doc, area);
    let bounds = cx.world().layout_box(area.stable_id()).unwrap();
    assert_eq!(bounds.width, 280.0);
    assert_eq!(bounds.height, 240.0);
    adapter
        .route(&mut cx, pointer(PointerPhase::Cancel, x, y))
        .unwrap();
    layout(&mut cx, doc, area);
    assert_eq!(
        cx.world().layout_box(area.stable_id()).unwrap().height,
        160.0
    );
    assert_eq!(cx.world().pointer_capture(doc, 1), None);
    let (x, y) = grip(&cx, area);
    adapter
        .route(&mut cx, pointer(PointerPhase::Down, x, y))
        .unwrap();
    adapter
        .route(&mut cx, pointer(PointerPhase::Move, x, y - 100.0))
        .unwrap();
    adapter
        .route(&mut cx, pointer(PointerPhase::Up, x, y - 100.0))
        .unwrap();
    layout(&mut cx, doc, area);
    assert_eq!(
        cx.world().layout_box(area.stable_id()).unwrap().height,
        148.0
    );
    assert_eq!(cx.read(area, |view| view.state.clone()).unwrap(), initial);
    assert_eq!(events.load(Ordering::SeqCst), 0);
    cx.update_component(area, |view, _| {
        view.state.replace_value("external draft");
    })
    .unwrap();
    layout(&mut cx, doc, area);
    assert_eq!(
        cx.world().layout_box(area.stable_id()).unwrap().height,
        148.0
    );
    assert_eq!(cx.world().text(area.stable_id()), Some("external draft"));
}

#[test]
fn textarea_resize_readonly_allowed_disabled_rejected_and_removal_releases_capture() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let area = cx
        .create_component(
            doc,
            TextArea::new("read only")
                .height(160.0)
                .read_only(true)
                .resize_vertical(true),
        )
        .unwrap();
    let mut adapter = HeadlessInput::bind(&mut cx, doc);
    layout(&mut cx, doc, area);
    let (x, y) = grip(&cx, area);
    adapter
        .route(&mut cx, pointer(PointerPhase::Down, x, y))
        .unwrap();
    adapter
        .route(&mut cx, pointer(PointerPhase::Move, x, y + 30.0))
        .unwrap();
    adapter
        .route(&mut cx, pointer(PointerPhase::Up, x, y + 30.0))
        .unwrap();
    layout(&mut cx, doc, area);
    assert_eq!(
        cx.world().layout_box(area.stable_id()).unwrap().height,
        190.0
    );
    cx.update_component(area, |view, _| view.disabled = true)
        .unwrap();
    layout(&mut cx, doc, area);
    let (x, y) = grip(&cx, area);
    adapter
        .route(&mut cx, pointer(PointerPhase::Down, x, y))
        .unwrap();
    assert_eq!(cx.world().pointer_capture(doc, 1), None);
    cx.update_component(area, |view, _| view.disabled = false)
        .unwrap();
    layout(&mut cx, doc, area);
    let (x, y) = grip(&cx, area);
    adapter
        .route(&mut cx, pointer(PointerPhase::Down, x, y))
        .unwrap();
    assert_eq!(cx.world().pointer_capture(doc, 1), Some(area.stable_id()));
    cx.remove_view(area).unwrap();
    assert_eq!(cx.world().pointer_capture(doc, 1), None);
    adapter
        .route(&mut cx, pointer(PointerPhase::Move, x, y + 30.0))
        .unwrap();
}

#[test]
fn textarea_resize_uses_transformed_logical_coordinates_and_content_box_chrome() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let mut view = TextArea::new("transform")
        .height(100.0)
        .resize_vertical(true);
    let style = Arc::make_mut(&mut view.style.layout);
    style.width = Some(nana_ui_core::LengthSpec::Px(150.0));
    style.box_sizing = nana_ui_core::BoxSizing::ContentBox;
    style.min_height = None;
    style.transforms.transform = Some(nana_ui_core::PaintTransform {
        a: 2.0,
        d: 2.0,
        ..Default::default()
    });
    let area = cx.create_component(doc, view).unwrap();
    layout(&mut cx, doc, area);
    let height = cx.world().layout_box(area.stable_id()).unwrap().height;
    let (x, y) = grip(&cx, area);
    let mut adapter = HeadlessInput::bind(&mut cx, doc);
    adapter
        .route(&mut cx, pointer(PointerPhase::Down, x, y))
        .unwrap();
    adapter
        .route(&mut cx, pointer(PointerPhase::Move, x, y + 40.0))
        .unwrap();
    adapter
        .route(&mut cx, pointer(PointerPhase::Up, x, y + 40.0))
        .unwrap();
    layout(&mut cx, doc, area);
    assert!((cx.world().layout_box(area.stable_id()).unwrap().height - height - 20.0).abs() < 0.1);
}

/// The limits hold the drag, not the grip: a field with `min_height` /
/// `max_height` keeps its grip in either box model, and the edge follows the
/// pointer again as soon as it comes back inside the range.
#[test]
fn textarea_resize_keeps_grip_under_limits_and_clamps_the_drag() {
    for box_sizing in [
        nana_ui_core::BoxSizing::BorderBox,
        nana_ui_core::BoxSizing::ContentBox,
    ] {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let mut view = TextArea::new("limits").height(160.0).resize_vertical(true);
        let style = Arc::make_mut(&mut view.style.layout);
        style.width = Some(nana_ui_core::LengthSpec::Px(280.0));
        style.box_sizing = box_sizing;
        style.min_height = Some(nana_ui_core::LengthSpec::Px(120.0));
        style.max_height = Some(nana_ui_core::LengthSpec::Px(200.0));
        let area = cx.create_component(doc, view).unwrap();
        layout(&mut cx, doc, area);
        let initial = cx.world().layout_box(area.stable_id()).unwrap().height;
        let (x, y) = grip(&cx, area);
        let mut adapter = HeadlessInput::bind(&mut cx, doc);
        adapter
            .route(&mut cx, pointer(PointerPhase::Down, x, y))
            .unwrap();
        assert_eq!(
            cx.world().pointer_capture(doc, 1),
            Some(area.stable_id()),
            "{box_sizing:?}: the grip stays under limits"
        );
        adapter
            .route(&mut cx, pointer(PointerPhase::Move, x, y + 400.0))
            .unwrap();
        layout(&mut cx, doc, area);
        let top = cx.world().layout_box(area.stable_id()).unwrap().height;
        assert_eq!(top, 200.0, "{box_sizing:?}: max_height holds the drag");
        // Back inside the range: 10px short of where `max_height` holds it.
        adapter
            .route(
                &mut cx,
                pointer(PointerPhase::Move, x, y + top - initial - 10.0),
            )
            .unwrap();
        layout(&mut cx, doc, area);
        let back = cx.world().layout_box(area.stable_id()).unwrap().height;
        assert!(
            (top - back - 10.0).abs() < 0.1,
            "{box_sizing:?}: the edge follows the pointer back ({top} -> {back})"
        );
        adapter
            .route(&mut cx, pointer(PointerPhase::Move, x, y - 400.0))
            .unwrap();
        adapter
            .route(&mut cx, pointer(PointerPhase::Up, x, y - 400.0))
            .unwrap();
        layout(&mut cx, doc, area);
        let bottom = cx.world().layout_box(area.stable_id()).unwrap().height;
        // Padding and border sit on top of a content-box height.
        let chrome = initial - 160.0;
        assert_eq!(
            bottom,
            120.0 + chrome,
            "{box_sizing:?}: min_height holds the drag"
        );
    }
}

/// A multiline editor's value keeps its line breaks whatever `white-space`
/// says. The scene has to say so: the renderer lays the same text out again,
/// and a paragraph measured as six lines and painted as one overflows nothing
/// — it silently loses the breaks the user typed.
#[test]
fn a_multiline_editors_text_primitive_says_its_newlines_are_line_breaks() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let view = TextArea::new("first\nsecond\nthird");
    let area = cx.create_component(doc, view).unwrap();
    layout(&mut cx, doc, area);
    let extracted = cx.world().extract_document(doc);
    let node = extracted
        .iter()
        .find(|node| node.id == area.stable_id())
        .expect("the editor must be extracted");
    assert!(
        node.text_preserve_lines,
        "a multiline editor preserves the newlines in its value"
    );
}

#[test]
fn textarea_resize_reconcile_preserves_dragged_height_and_clamps_percent_and_em_limits() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let root = cx
        .create_component(
            doc,
            Stack::column(0.0).with_layout(|layout| {
                layout.width = Some(nana_ui_core::LengthSpec::Px(300.0));
                layout.height = Some(nana_ui_core::LengthSpec::Px(400.0));
            }),
        )
        .unwrap();
    let mut view = TextArea::new("limits").height(150.0).resize_vertical(true);
    let style = Arc::make_mut(&mut view.style.layout);
    style.width = Some(nana_ui_core::LengthSpec::Px(260.0));
    style.font_size = Some(16.0);
    style.min_height = Some(nana_ui_core::LengthSpec::Em(6.0)); // 96px
    style.max_height = Some(nana_ui_core::LengthSpec::Percent(0.5)); // 200px of 400px parent
    let area = cx.create_detached_component(doc, view.clone()).unwrap();
    cx.append_child(root, area).unwrap();
    layout(&mut cx, doc, area);

    let (x, y) = grip(&cx, area);
    let mut adapter = HeadlessInput::bind(&mut cx, doc);
    adapter
        .route(&mut cx, pointer(PointerPhase::Down, x, y))
        .unwrap();
    adapter
        .route(&mut cx, pointer(PointerPhase::Move, x, y + 300.0))
        .unwrap();
    adapter
        .route(&mut cx, pointer(PointerPhase::Up, x, y + 300.0))
        .unwrap();
    layout(&mut cx, doc, area);
    assert_eq!(
        cx.world().layout_box(area.stable_id()).unwrap().height,
        200.0
    );

    // Declarative reconcile with resize_vertical still enabled keeps the dragged height.
    let next = view.clone();
    cx.update_component(area, |current, _| current.reconcile(next))
        .unwrap();
    layout(&mut cx, doc, area);
    assert_eq!(
        cx.world().layout_box(area.stable_id()).unwrap().height,
        200.0
    );

    // Drag down to em-based min_height.
    let (x, y) = grip(&cx, area);
    adapter
        .route(&mut cx, pointer(PointerPhase::Down, x, y))
        .unwrap();
    adapter
        .route(&mut cx, pointer(PointerPhase::Move, x, y - 300.0))
        .unwrap();
    adapter
        .route(&mut cx, pointer(PointerPhase::Up, x, y - 300.0))
        .unwrap();
    layout(&mut cx, doc, area);
    assert_eq!(
        cx.world().layout_box(area.stable_id()).unwrap().height,
        96.0
    );

    // Disabling resize_vertical during reconcile drops the kept dragged height.
    let disabled_resize = TextArea::new("limits").height(150.0).resize_vertical(false);
    cx.update_component(area, |current, _| current.reconcile(disabled_resize))
        .unwrap();
    layout(&mut cx, doc, area);
    assert_eq!(
        cx.world().layout_box(area.stable_id()).unwrap().height,
        150.0
    );
}
