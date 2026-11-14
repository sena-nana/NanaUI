//! A shown scrollbar lane owns the pointer over it: hover, the hot lane and
//! the press that grabs the bar.

use std::sync::Arc;
use std::time::Duration;

use nana_ui_core::{LengthSpec, ScrollbarAxis, ScrollbarVisibility};

use super::*;
use crate::{ComponentGeometry, Entity, NodeStyle, ScrollAxes, ScrollView, ScrollbarBar, Stack};

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn pointer_at(phase: PointerPhase, x: f32, y: f32) -> Gesture {
    pointer_fixture! {
        phase,
        pointer_id: 1,
        pointer_type: PointerType::Mouse,
        x,
        y,
        screen_x: x,
        screen_y: y,
        button: 0,
        buttons: u16::from(phase == PointerPhase::Down),
        pressure: 0.0,
        tangential_pressure: 0.0,
        tilt_x: 0,
        tilt_y: 0,
        twist: 0,
        is_primary: true,
        activation_click: false,
        modifiers: InputModifiers::default(),
    }
}

fn hittable_row(height: f32) -> Stack {
    Stack::row(0.0).hittable().with_layout(|layout| {
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(height));
    })
}

/// A 200×120 scroll view of five hittable 40px rows, laid out, with its
/// rows; `extra` is placed in it after the rows, where an open menu's or
/// popover's content stands.
fn scroll_with_rows(
    context: &mut AppContext,
    visibility: ScrollbarVisibility,
    extra: Option<Stack>,
) -> (Entity<ScrollView>, Vec<StableNodeId>, Option<StableNodeId>) {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(200.0));
        layout.height = Some(LengthSpec::Px(120.0));
    }
    let scroll = context
        .create_component(
            document(),
            ScrollView::new(ScrollAxes::Vertical)
                .scrollbars(visibility)
                .style(style),
        )
        .unwrap();
    let rows = (0..5)
        .map(|_| {
            let row = context
                .create_detached_component(document(), hittable_row(40.0))
                .unwrap();
            context.append_child(scroll, row).unwrap();
            row.stable_id()
        })
        .collect();
    let extra = extra.map(|extra| {
        let extra = context
            .create_detached_component(document(), extra)
            .unwrap();
        context.append_child(scroll, extra).unwrap();
        extra.stable_id()
    });
    context
        .layout_document(document(), crate::LayoutViewport::new(400.0, 300.0))
        .unwrap();
    context.rebuild_hit_test(document());
    (scroll, rows, extra)
}

fn bar(context: &AppContext, scroll: Entity<ScrollView>) -> ScrollbarBar {
    match context.world().component_geometry(scroll.stable_id()) {
        Some(ComponentGeometry::Scrollbar {
            vertical: Some(bar),
            ..
        }) => bar,
        other => panic!("no vertical bar: {other:?}"),
    }
}

fn hot(context: &AppContext, scroll: Entity<ScrollView>) -> Option<ScrollbarAxis> {
    context.read(scroll, |view| view.hot).unwrap()
}

/// Over a shown lane the pointer is the bar's: the row under it loses its
/// hover, the lane heats up and its thumb grows; back over the content the
/// row has it again.
#[test]
fn a_shown_lane_takes_hover_from_the_row_under_it() {
    let mut context = AppContext::new();
    let (scroll, rows, _) = scroll_with_rows(&mut context, ScrollbarVisibility::Always, None);
    let lane = bar(&context, scroll).lane;
    let on_lane = (lane.x + lane.width / 2.0, 20.0);
    let mut adapter = TestInput::default();

    adapter
        .dispatch_at(
            &mut context,
            document(),
            &pointer_at(PointerPhase::Move, 60.0, 20.0),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(context.world().pointer_hover(document(), 1), Some(rows[0]));
    assert_eq!(hot(&context, scroll), None);

    adapter
        .dispatch_at(
            &mut context,
            document(),
            &pointer_at(PointerPhase::Move, on_lane.0, on_lane.1),
            Duration::from_millis(10),
        )
        .unwrap();
    assert_eq!(
        context.world().pointer_hover(document(), 1),
        Some(scroll.stable_id()),
        "the row under the lane is not hovered"
    );
    assert_eq!(hot(&context, scroll), Some(ScrollbarAxis::Vertical));
    context.advance_animations(Duration::from_millis(400));
    assert_eq!(
        bar(&context, scroll).thumb.width,
        nana_ui_core::SCROLLBAR_METRICS.thumb_active_thickness,
        "the hot thumb grows"
    );

    adapter
        .dispatch_at(
            &mut context,
            document(),
            &pointer_at(PointerPhase::Move, 60.0, 20.0),
            Duration::from_millis(420),
        )
        .unwrap();
    assert_eq!(context.world().pointer_hover(document(), 1), Some(rows[0]));
    assert_eq!(hot(&context, scroll), None);
    context.advance_animations(Duration::from_millis(900));
    assert_eq!(
        bar(&context, scroll).thumb.width,
        nana_ui_core::SCROLLBAR_METRICS.thumb_thickness
    );
}

/// A press on the lane grabs the bar, not the row under it; letting go
/// holds the bars for the hide delay.
#[test]
fn a_press_on_the_lane_grabs_the_bar() {
    let mut context = AppContext::new();
    let (scroll, _, _) = scroll_with_rows(&mut context, ScrollbarVisibility::AutoHide, None);
    let mut adapter = TestInput::default();
    adapter
        .dispatch_at(
            &mut context,
            document(),
            &pointer_at(PointerPhase::Move, 60.0, 20.0),
            Duration::ZERO,
        )
        .unwrap();
    context.advance_animations(Duration::from_millis(300));
    let thumb = bar(&context, scroll).thumb;
    let grab = (thumb.x + thumb.width / 2.0, thumb.y + thumb.height / 2.0);
    for (phase, at) in [(PointerPhase::Move, 310), (PointerPhase::Down, 320)] {
        adapter
            .dispatch_at(
                &mut context,
                document(),
                &pointer_at(phase, grab.0, grab.1),
                Duration::from_millis(at),
            )
            .unwrap();
    }
    assert_eq!(
        context.world().pointer_capture(document(), 1),
        Some(scroll.stable_id())
    );
    assert!(
        context
            .read(scroll, |view| view.dragging.is_some())
            .unwrap()
    );
    adapter
        .dispatch_at(
            &mut context,
            document(),
            &pointer_at(PointerPhase::Up, grab.0, grab.1),
            Duration::from_millis(330),
        )
        .unwrap();
    assert_eq!(context.world().pointer_capture(document(), 1), None);
    assert!(context.read(scroll, |view| view.held).unwrap());
}

/// A viewport-fixed surface (a menu, a popover) over the lane keeps the
/// pointer: it paints over the bar.
#[test]
fn a_fixed_surface_over_the_lane_keeps_the_pointer() {
    let mut context = AppContext::new();
    let surface = Stack::row(0.0).hittable().with_layout(|layout| {
        layout.position = nana_ui_core::PositionSpec::Fixed;
        layout.offset_left = Some(LengthSpec::Px(150.0));
        layout.offset_top = Some(LengthSpec::Px(0.0));
        layout.width = Some(LengthSpec::Px(60.0));
        layout.height = Some(LengthSpec::Px(40.0));
    });
    let (scroll, _, surface) =
        scroll_with_rows(&mut context, ScrollbarVisibility::Always, Some(surface));
    let surface = surface.unwrap();
    let lane = bar(&context, scroll).lane;
    let mut adapter = TestInput::default();
    adapter
        .dispatch_at(
            &mut context,
            document(),
            &pointer_at(PointerPhase::Move, lane.x + lane.width / 2.0, 20.0),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(context.world().pointer_hover(document(), 1), Some(surface));
    assert_eq!(hot(&context, scroll), None);
}

#[test]
fn pointer_events_none_scroll_view_passes_empty_area_through_and_grabs_shown_lane() {
    let mut context = AppContext::new();
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(200.0));
        layout.height = Some(LengthSpec::Px(120.0));
        layout.pointer_events = Some(nana_ui_core::PointerEventsSpec::None);
    }
    let scroll = context
        .create_component(
            document(),
            ScrollView::new(ScrollAxes::Vertical)
                .scrollbars(ScrollbarVisibility::Always)
                .style(style),
        )
        .unwrap();
    let child_style = {
        let mut s = NodeStyle::default();
        let layout = Arc::make_mut(&mut s.layout);
        layout.width = Some(LengthSpec::Px(180.0));
        layout.height = Some(LengthSpec::Px(320.0));
        layout.pointer_events = Some(nana_ui_core::PointerEventsSpec::None);
        s
    };
    let child = context
        .create_detached_component(document(), Stack::column(0.0).style(child_style))
        .unwrap();
    context.append_child(scroll, child).unwrap();
    context
        .layout_document(document(), crate::LayoutViewport::new(400.0, 300.0))
        .unwrap();
    context.rebuild_hit_test(document());

    assert_eq!(context.pointer_target(document(), 60.0, 20.0), None);
    let current = bar(&context, scroll);
    let lane_center_x = current.lane.x + current.lane.width / 2.0;
    assert_eq!(
        context.pointer_target(document(), lane_center_x, 20.0),
        Some(scroll.stable_id())
    );

    let mut adapter = TestInput::default();
    adapter
        .dispatch_at(
            &mut context,
            document(),
            &pointer_at(PointerPhase::Move, lane_center_x, 20.0),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(
        context.world().pointer_hover(document(), 1),
        Some(scroll.stable_id())
    );
    assert_eq!(hot(&context, scroll), Some(ScrollbarAxis::Vertical));

    let thumb = bar(&context, scroll).thumb;
    let grab = (thumb.x + thumb.width / 2.0, thumb.y + thumb.height / 2.0);
    adapter
        .dispatch_at(
            &mut context,
            document(),
            &pointer_at(PointerPhase::Down, grab.0, grab.1),
            Duration::from_millis(10),
        )
        .unwrap();
    assert_eq!(
        context.world().pointer_capture(document(), 1),
        Some(scroll.stable_id())
    );
    adapter
        .dispatch_at(
            &mut context,
            document(),
            &pointer_at(PointerPhase::Move, grab.0, grab.1 + 30.0),
            Duration::from_millis(20),
        )
        .unwrap();
    let offset_y = context.world().scroll_offset(scroll.stable_id()).unwrap().y;
    assert!(offset_y > 0.0, "expected scroll offset > 0, got {offset_y}");
}
