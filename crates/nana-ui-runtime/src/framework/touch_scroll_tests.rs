//! Touch samples through the hosted input path, against an explicit clock:
//! slop, press cancellation, axis lock, nested handoff, and each platform's
//! fling and edge checked against its formula.
use super::*;
use crate::{Activate, Button, HeadlessInput, LayoutBox, RangeChanged, RangeField, Text};
use nana_ui_input::{InputPayload, PointerId, PointerInput, PointerPhase, PointerType};
use std::sync::{Arc, Mutex};

fn doc() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn offset(cx: &AppContext, id: StableNodeId) -> ScrollOffset {
    cx.world.scroll_offset(id).unwrap()
}

fn layout(cx: &mut AppContext, boxes: &[(StableNodeId, [f32; 4])]) {
    let mut q = MutationQueue::new();
    for (id, [x, y, width, height]) in boxes {
        q.write_layout(
            *id,
            LayoutBox {
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            },
        );
    }
    cx.commit_mutations(q).unwrap();
    cx.take_system_work();
    cx.rebuild_hit_test(doc());
}

struct Touch {
    input: HeadlessInput,
}

impl Touch {
    fn new(cx: &mut AppContext) -> Self {
        Self {
            input: HeadlessInput::bind(cx, doc()),
        }
    }

    fn send(&mut self, cx: &mut AppContext, ms: u64, phase: PointerPhase, x: f32, y: f32) {
        self.input.set_now(Duration::from_millis(ms));
        self.input
            .route(
                cx,
                InputPayload::Pointer(PointerInput {
                    pointer_id: PointerId(7),
                    pointer_type: PointerType::Touch,
                    ..PointerInput::mouse(phase, x, y)
                }),
            )
            .unwrap();
    }

    /// A finger that lands at `from`, then moves by `step` every 10 ms for
    /// `steps` samples and lifts where it stopped, 10 ms after the last.
    fn swipe(
        &mut self,
        cx: &mut AppContext,
        start: u64,
        from: (f32, f32),
        step: (f32, f32),
        steps: u32,
    ) -> u64 {
        self.send(cx, start, PointerPhase::Down, from.0, from.1);
        let mut at = (from.0, from.1);
        let mut ms = start;
        for _ in 0..steps {
            ms += 10;
            at = (at.0 + step.0, at.1 + step.1);
            self.send(cx, ms, PointerPhase::Move, at.0, at.1);
        }
        ms += 10;
        at = (at.0 + step.0, at.1 + step.1);
        self.send(cx, ms, PointerPhase::Move, at.0, at.1);
        self.send(cx, ms, PointerPhase::Up, at.0, at.1);
        ms
    }
}

/// A 100×100 vertical scroll view over 1000 px of content, a button on its
/// first 40 px.
struct List {
    cx: AppContext,
    scroll: StableNodeId,
    button: StableNodeId,
    activations: Arc<Mutex<u32>>,
}

fn list(axes: ScrollAxes) -> List {
    let mut cx = AppContext::new();
    let scroll = cx
        .create_component(
            doc(),
            ScrollView::new(axes).scrollbars(nana_ui_core::ScrollbarVisibility::Hidden),
        )
        .unwrap();
    let filler = cx.create_component(doc(), Text::new("tail")).unwrap();
    let button = cx.create_component(doc(), Button::new("Row")).unwrap();
    cx.append_child(scroll, filler).unwrap();
    cx.append_child(scroll, button).unwrap();
    let activations = Arc::new(Mutex::new(0));
    let count = activations.clone();
    cx.on(button, move |_button, _event: &Activate, _cx| {
        *count.lock().unwrap() += 1;
    })
    .unwrap();
    layout(
        &mut cx,
        &[
            (scroll.id, [0.0, 0.0, 100.0, 100.0]),
            (filler.id, [0.0, 0.0, 1000.0, 1000.0]),
            (button.id, [0.0, 0.0, 100.0, 40.0]),
        ],
    );
    List {
        cx,
        scroll: scroll.id,
        button: button.id,
        activations,
    }
}

fn settle(cx: &mut AppContext, scroll: StableNodeId, from: u64) -> Vec<f32> {
    let mut seen = Vec::new();
    let mut now = Duration::from_millis(from);
    for _ in 0..2_000 {
        let Some(deadline) = cx.touch_scroll_deadline() else {
            return seen;
        };
        assert!(deadline >= now, "deadline went back");
        assert!(deadline.saturating_sub(now) <= Duration::from_millis(8));
        now = deadline;
        cx.advance_animations(now);
        seen.push(offset(cx, scroll).y);
    }
    panic!("the fling never settled");
}

#[test]
fn a_tap_inside_the_slop_activates_the_button() {
    let List {
        mut cx,
        scroll,
        activations,
        ..
    } = list(ScrollAxes::Vertical);
    let mut touch = Touch::new(&mut cx);
    touch.send(&mut cx, 0, PointerPhase::Down, 50.0, 20.0);
    touch.send(&mut cx, 30, PointerPhase::Move, 52.0, 26.0);
    touch.send(&mut cx, 60, PointerPhase::Up, 52.0, 26.0);
    assert_eq!(*activations.lock().unwrap(), 1);
    assert_eq!(offset(&cx, scroll), ScrollOffset::default());
    assert_eq!(cx.touch_scroll_deadline(), None);
}

#[test]
fn past_the_slop_the_press_is_cancelled_and_the_view_follows_the_finger() {
    let List {
        mut cx,
        scroll,
        activations,
        ..
    } = list(ScrollAxes::Vertical);
    let mut touch = Touch::new(&mut cx);
    touch.send(&mut cx, 0, PointerPhase::Down, 50.0, 20.0);
    assert!(cx.world.pointer_press(doc(), 1).is_some());
    // The scroll takes over where the finger left the slop: no jump.
    touch.send(&mut cx, 20, PointerPhase::Move, 50.0, 10.0);
    assert_eq!(offset(&cx, scroll).y, 0.0);
    assert_eq!(cx.world.pointer_press(doc(), 1), None);
    assert_eq!(cx.world.pointer_capture(doc(), 1), Some(scroll));
    touch.send(&mut cx, 40, PointerPhase::Move, 50.0, -15.0);
    assert_eq!(offset(&cx, scroll).y, 25.0);
    // Back over the button and lifted after resting: no click, no fling.
    touch.send(&mut cx, 60, PointerPhase::Move, 50.0, 0.0);
    touch.send(&mut cx, 300, PointerPhase::Up, 50.0, 0.0);
    assert_eq!(*activations.lock().unwrap(), 0);
    assert_eq!(offset(&cx, scroll).y, 10.0);
    assert_eq!(cx.world.pointer_capture(doc(), 1), None);
    assert_eq!(cx.world.pointer_hover(doc(), 1), None);
    assert_eq!(cx.touch_scroll_deadline(), None);
}

#[test]
fn ios_fling_follows_the_deceleration_formula() {
    let List { mut cx, scroll, .. } = list(ScrollAxes::Vertical);
    assert_eq!(cx.scroll_physics(), ScrollPhysics::Ios);
    let mut touch = Touch::new(&mut cx);
    // The finger moves up at 1000 px/s; the content flings down at it.
    let lifted = touch.swipe(&mut cx, 0, (50.0, 95.0), (0.0, -10.0), 9);
    let from = offset(&cx, scroll).y;
    assert_eq!(from, 90.0);
    assert_eq!(
        cx.touch_scroll_deadline(),
        Some(Duration::from_millis(lifted + 8))
    );
    let expected = |ms: f32| from + 1.0 * (0.998f32.powf(ms) - 1.0) / 0.998f32.ln();
    for ms in [8u64, 50, 200, 700, 1500] {
        cx.advance_animations(Duration::from_millis(lifted + ms));
        let at = offset(&cx, scroll).y;
        assert!(
            (at - expected(ms as f32)).abs() < 0.5,
            "{ms} ms: {at} vs {}",
            expected(ms as f32)
        );
    }
    let rest = from - 1.0 / 0.998f32.ln();
    settle(&mut cx, scroll, lifted + 1500);
    assert!((offset(&cx, scroll).y - rest).abs() < 0.5);
    assert_eq!(cx.touch_scroll_deadline(), None);
}

/// `SplineOverScroller`'s spline, solved directly at `t` rather than read
/// from the 100-sample table.
fn aosp_spline(t: f64) -> f64 {
    let (p1, p2) = (0.5 * 0.35, 1.0 - (1.0 - 0.35));
    let (mut low, mut high) = (0.0f64, 1.0f64);
    for _ in 0..60 {
        let x = (low + high) / 2.0;
        let tx = 3.0 * x * (1.0 - x) * ((1.0 - x) * p1 + x * p2) + x * x * x;
        if tx > t {
            high = x;
        } else {
            low = x;
        }
    }
    let x = (low + high) / 2.0;
    3.0 * x * (1.0 - x) * ((1.0 - x) * 0.5 + x) + x * x * x
}

#[test]
fn android_fling_follows_the_spline_and_its_distance() {
    let List { mut cx, scroll, .. } = list(ScrollAxes::Vertical);
    cx.set_scroll_physics(ScrollPhysics::Android);
    let mut touch = Touch::new(&mut cx);
    let lifted = touch.swipe(&mut cx, 0, (50.0, 95.0), (0.0, -10.0), 9);
    let from = offset(&cx, scroll).y as f64;
    // getSplineFlingDistance / getSplineFlingDuration at 1000 dp/s.
    let decel = 0.78f64.ln() / 0.9f64.ln();
    let coeff = 9.806_65f64 * 39.37 * 160.0 * 0.84;
    let l = (0.35 * 1000.0 / (0.015 * coeff)).ln();
    let duration = (l / (decel - 1.0)).exp();
    let distance = 0.015 * coeff * (decel / (decel - 1.0) * l).exp();
    assert!((duration - 0.555).abs() < 0.005, "{duration}");
    assert!((distance - 194.3).abs() < 0.5, "{distance}");
    let mut last = lifted;
    for share in [0.05, 0.2, 0.35, 0.6, 0.9] {
        let ms = (duration * share * 1_000.0).round();
        last = lifted + ms as u64;
        cx.advance_animations(Duration::from_millis(last));
        let expected = from + distance * aosp_spline(ms / 1_000.0 / duration);
        let at = offset(&cx, scroll).y as f64;
        assert!((at - expected).abs() < 0.5, "{share}: {at} vs {expected}");
    }
    settle(&mut cx, scroll, last);
    assert!((offset(&cx, scroll).y as f64 - (from + distance)).abs() < 0.5);
}

#[test]
fn a_slow_android_lift_does_not_fling() {
    let List { mut cx, scroll, .. } = list(ScrollAxes::Vertical);
    cx.set_scroll_physics(ScrollPhysics::Android);
    let mut touch = Touch::new(&mut cx);
    // 30 px/s is below ViewConfiguration's minimum fling velocity.
    touch.send(&mut cx, 0, PointerPhase::Down, 50.0, 50.0);
    touch.send(&mut cx, 10, PointerPhase::Move, 50.0, 40.0);
    for step in 1..=10 {
        touch.send(
            &mut cx,
            10 + step * 10,
            PointerPhase::Move,
            50.0,
            40.0 - step as f32 * 0.3,
        );
    }
    touch.send(&mut cx, 110, PointerPhase::Up, 50.0, 37.0);
    assert_eq!(cx.touch_scroll_deadline(), None);
    assert!((offset(&cx, scroll).y - 3.0).abs() < 1e-4);
}

#[test]
fn ios_rubber_bands_past_the_edge_and_springs_back() {
    let List {
        mut cx,
        scroll,
        button,
        ..
    } = list(ScrollAxes::Vertical);
    let mut touch = Touch::new(&mut cx);
    touch.send(&mut cx, 0, PointerPhase::Down, 50.0, 20.0);
    touch.send(&mut cx, 10, PointerPhase::Move, 50.0, 30.0);
    touch.send(&mut cx, 20, PointerPhase::Move, 50.0, 80.0);
    // 50 px of finger past the top of a 100 px viewport.
    let band = -(1.0 - 1.0 / (50.0 * 0.55 / 100.0 + 1.0)) * 100.0;
    assert!((offset(&cx, scroll).y - band).abs() < 1e-3);
    // Paint and hit testing follow the stretch; the layout box stays.
    assert_eq!(
        cx.world.component_layout_box(button).unwrap(),
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 40.0,
        }
    );
    cx.rebuild_hit_test(doc());
    assert_eq!(cx.world.hit_test(doc(), 50.0, 55.0), Some(button));
    // Further pulls give less and less, never a whole viewport.
    touch.send(&mut cx, 30, PointerPhase::Move, 50.0, 1030.0);
    let far = offset(&cx, scroll).y;
    assert!(far < band && far > -100.0);
    touch.send(&mut cx, 40, PointerPhase::Move, 50.0, 80.0);
    touch.send(&mut cx, 400, PointerPhase::Up, 50.0, 80.0);
    assert!(cx.touch_scroll_deadline().is_some());
    let path = settle(&mut cx, scroll, 400);
    // Critically damped from rest: straight back, never past the edge.
    assert!(path.windows(2).all(|pair| pair[1] >= pair[0]));
    assert!(path.iter().all(|y| *y <= 0.0));
    assert_eq!(offset(&cx, scroll).y, 0.0);
    // The spring settles about half a second after the lift.
    assert!(path.len() < 100, "{}", path.len());
}

#[test]
fn ios_fling_into_the_edge_overshoots_and_settles_on_it() {
    let List { mut cx, scroll, .. } = list(ScrollAxes::Vertical);
    cx.scroll_to(
        Entity::from_stable_id(scroll),
        ScrollOffset { x: 0.0, y: 700.0 },
    )
    .unwrap();
    let mut touch = Touch::new(&mut cx);
    let lifted = touch.swipe(&mut cx, 0, (50.0, 95.0), (0.0, -10.0), 9);
    assert_eq!(offset(&cx, scroll).y, 790.0);
    let path = settle(&mut cx, scroll, lifted);
    let peak = path.iter().copied().fold(f32::MIN, f32::max);
    assert!(peak > 900.5, "{peak}");
    assert_eq!(offset(&cx, scroll).y, 900.0);
}

#[test]
fn android_clamps_drag_and_fling_at_the_edge() {
    let List { mut cx, scroll, .. } = list(ScrollAxes::Vertical);
    cx.set_scroll_physics(ScrollPhysics::Android);
    let mut touch = Touch::new(&mut cx);
    touch.send(&mut cx, 0, PointerPhase::Down, 50.0, 20.0);
    touch.send(&mut cx, 10, PointerPhase::Move, 50.0, 30.0);
    touch.send(&mut cx, 20, PointerPhase::Move, 50.0, 80.0);
    assert_eq!(offset(&cx, scroll).y, 0.0);
    touch.send(&mut cx, 300, PointerPhase::Up, 50.0, 80.0);
    assert_eq!(cx.touch_scroll_deadline(), None);

    cx.scroll_to(
        Entity::from_stable_id(scroll),
        ScrollOffset { x: 0.0, y: 800.0 },
    )
    .unwrap();
    let lifted = touch.swipe(&mut cx, 400, (50.0, 95.0), (0.0, -10.0), 9);
    let path = settle(&mut cx, scroll, lifted);
    assert!(path.iter().all(|y| *y <= 900.0));
    assert_eq!(offset(&cx, scroll).y, 900.0);
}

#[test]
fn a_tap_on_a_moving_list_stops_it_and_clicks_nothing() {
    let List {
        mut cx,
        scroll,
        activations,
        ..
    } = list(ScrollAxes::Vertical);
    let mut touch = Touch::new(&mut cx);
    let lifted = touch.swipe(&mut cx, 0, (50.0, 95.0), (0.0, -10.0), 9);
    cx.advance_animations(Duration::from_millis(lifted + 50));
    let stopped = offset(&cx, scroll).y;
    // Scrolled under the finger, the button is at the top again only in
    // content space; tap where the list is.
    touch.send(&mut cx, lifted + 60, PointerPhase::Down, 50.0, 50.0);
    assert_eq!(cx.touch_scroll_deadline(), None);
    touch.send(&mut cx, lifted + 90, PointerPhase::Up, 50.0, 50.0);
    cx.advance_animations(Duration::from_millis(lifted + 400));
    assert_eq!(offset(&cx, scroll).y, stopped);
    assert_eq!(*activations.lock().unwrap(), 0);
}

#[test]
fn programmatic_positions_end_a_fling() {
    let List { mut cx, scroll, .. } = list(ScrollAxes::Vertical);
    let mut touch = Touch::new(&mut cx);
    let lifted = touch.swipe(&mut cx, 0, (50.0, 95.0), (0.0, -10.0), 9);
    cx.advance_animations(Duration::from_millis(lifted + 50));
    cx.scroll_to(
        Entity::from_stable_id(scroll),
        ScrollOffset { x: 0.0, y: 5.0 },
    )
    .unwrap();
    assert_eq!(cx.touch_scroll_deadline(), None);
    cx.advance_animations(Duration::from_millis(lifted + 400));
    assert_eq!(offset(&cx, scroll).y, 5.0);
}

#[test]
fn the_dominant_axis_locks_a_two_axis_view() {
    let List { mut cx, scroll, .. } = list(ScrollAxes::Both);
    let mut touch = Touch::new(&mut cx);
    touch.send(&mut cx, 0, PointerPhase::Down, 60.0, 60.0);
    touch.send(&mut cx, 10, PointerPhase::Move, 56.0, 50.0);
    touch.send(&mut cx, 20, PointerPhase::Move, 30.0, 20.0);
    assert_eq!(offset(&cx, scroll), ScrollOffset { x: 0.0, y: 30.0 });
    touch.send(&mut cx, 300, PointerPhase::Up, 30.0, 20.0);
}

fn range_list() -> (
    AppContext,
    StableNodeId,
    Entity<RangeField>,
    Arc<Mutex<u32>>,
) {
    let mut cx = AppContext::new();
    let scroll = cx
        .create_component(
            doc(),
            ScrollView::new(ScrollAxes::Vertical)
                .scrollbars(nana_ui_core::ScrollbarVisibility::Hidden),
        )
        .unwrap();
    let filler = cx.create_component(doc(), Text::new("tail")).unwrap();
    let range = cx
        .create_component(doc(), RangeField::new(0.5, 0.0, 1.0, 0.0))
        .unwrap();
    cx.append_child(scroll, filler).unwrap();
    cx.append_child(scroll, range).unwrap();
    let changes = Arc::new(Mutex::new(0));
    let count = changes.clone();
    cx.on(range, move |_range, _event: &RangeChanged, _cx| {
        *count.lock().unwrap() += 1;
    })
    .unwrap();
    layout(
        &mut cx,
        &[
            (scroll.id, [0.0, 0.0, 300.0, 100.0]),
            (filler.id, [0.0, 0.0, 300.0, 1000.0]),
            (range.id, [0.0, 10.0, 300.0, 32.0]),
        ],
    );
    (cx, scroll.id, range, changes)
}

#[test]
fn a_sideways_drag_moves_the_range_and_not_the_list() {
    let (mut cx, scroll, range, changes) = range_list();
    let track = match cx.world().component_geometry(range.stable_id()) {
        Some(crate::ComponentGeometry::Range { track, .. }) => track,
        _ => panic!("range geometry expected"),
    };
    let start = track.x + track.width * 0.5;
    let mut touch = Touch::new(&mut cx);
    touch.send(&mut cx, 0, PointerPhase::Down, start, 26.0);
    touch.send(&mut cx, 10, PointerPhase::Move, start + 20.0, 28.0);
    touch.send(&mut cx, 20, PointerPhase::Move, start + 60.0, 22.0);
    assert_eq!(cx.world.pointer_capture(doc(), 1), Some(range.stable_id()));
    touch.send(&mut cx, 30, PointerPhase::Up, start + 60.0, 22.0);
    assert_eq!(offset(&cx, scroll), ScrollOffset::default());
    assert!(cx.read(range, |range| range.value).unwrap() > 0.6);
    assert_eq!(*changes.lock().unwrap(), 1);
}

#[test]
fn an_upward_drag_on_the_range_scrolls_the_list_and_restores_the_range() {
    let (mut cx, scroll, range, changes) = range_list();
    let track = match cx.world().component_geometry(range.stable_id()) {
        Some(crate::ComponentGeometry::Range { track, .. }) => track,
        _ => panic!("range geometry expected"),
    };
    // Pressing off-centre jumps the value, as a press on a track does.
    let start = track.x + track.width * 0.8;
    let mut touch = Touch::new(&mut cx);
    touch.send(&mut cx, 0, PointerPhase::Down, start, 26.0);
    assert!(cx.read(range, |range| range.value).unwrap() > 0.7);
    touch.send(&mut cx, 10, PointerPhase::Move, start + 2.0, 14.0);
    touch.send(&mut cx, 20, PointerPhase::Move, start + 5.0, -6.0);
    assert_eq!(cx.world.pointer_capture(doc(), 1), Some(scroll));
    assert_eq!(cx.read(range, |range| range.value).unwrap(), 0.5);
    assert_eq!(offset(&cx, scroll).y, 20.0);
    touch.send(&mut cx, 300, PointerPhase::Up, start + 5.0, -6.0);
    assert_eq!(*changes.lock().unwrap(), 0);
    assert_eq!(cx.read(range, |range| range.value).unwrap(), 0.5);
}

/// A 100×100 outer list over 400 px holding, at its top, a 100×50 inner
/// list over 200 px.
fn nested() -> (AppContext, StableNodeId, StableNodeId) {
    let mut cx = AppContext::new();
    let hidden = nana_ui_core::ScrollbarVisibility::Hidden;
    let outer = cx
        .create_component(
            doc(),
            ScrollView::new(ScrollAxes::Vertical).scrollbars(hidden),
        )
        .unwrap();
    let outer_tail = cx.create_component(doc(), Text::new("outer")).unwrap();
    let inner = cx
        .create_component(
            doc(),
            ScrollView::new(ScrollAxes::Vertical).scrollbars(hidden),
        )
        .unwrap();
    let inner_tail = cx.create_component(doc(), Text::new("inner")).unwrap();
    cx.append_child(outer, outer_tail).unwrap();
    cx.append_child(outer, inner).unwrap();
    cx.append_child(inner, inner_tail).unwrap();
    layout(
        &mut cx,
        &[
            (outer.id, [0.0, 0.0, 100.0, 100.0]),
            (outer_tail.id, [0.0, 0.0, 100.0, 400.0]),
            (inner.id, [0.0, 0.0, 100.0, 50.0]),
            (inner_tail.id, [0.0, 0.0, 100.0, 200.0]),
        ],
    );
    (cx, outer.id, inner.id)
}

#[test]
fn the_inner_list_scrolls_until_its_edge_then_the_outer_one_does() {
    let (mut cx, outer, inner) = nested();
    let mut touch = Touch::new(&mut cx);
    touch.send(&mut cx, 0, PointerPhase::Down, 50.0, 40.0);
    touch.send(&mut cx, 10, PointerPhase::Move, 50.0, 30.0);
    touch.send(&mut cx, 20, PointerPhase::Move, 50.0, 0.0);
    assert_eq!(offset(&cx, inner).y, 30.0);
    assert_eq!(offset(&cx, outer).y, 0.0);
    touch.send(&mut cx, 300, PointerPhase::Up, 50.0, 0.0);

    cx.scroll_to(
        Entity::from_stable_id(inner),
        ScrollOffset { x: 0.0, y: 150.0 },
    )
    .unwrap();
    touch.send(&mut cx, 400, PointerPhase::Down, 50.0, 40.0);
    touch.send(&mut cx, 410, PointerPhase::Move, 50.0, 30.0);
    touch.send(&mut cx, 420, PointerPhase::Move, 50.0, 0.0);
    assert_eq!(offset(&cx, inner).y, 150.0);
    assert_eq!(offset(&cx, outer).y, 30.0);
    touch.send(&mut cx, 700, PointerPhase::Up, 50.0, 0.0);

    // Pulled back down, the inner list has room again and takes it.
    touch.send(&mut cx, 800, PointerPhase::Down, 50.0, 10.0);
    touch.send(&mut cx, 810, PointerPhase::Move, 50.0, 20.0);
    touch.send(&mut cx, 820, PointerPhase::Move, 50.0, 40.0);
    assert_eq!(offset(&cx, inner).y, 130.0);
    assert_eq!(offset(&cx, outer).y, 30.0);
    touch.send(&mut cx, 1100, PointerPhase::Up, 50.0, 40.0);
}

#[test]
fn reduced_motion_lands_a_lift_at_once() {
    let List { mut cx, scroll, .. } = list(ScrollAxes::Vertical);
    cx.set_reduced_motion(true);
    let mut touch = Touch::new(&mut cx);
    touch.send(&mut cx, 0, PointerPhase::Down, 50.0, 20.0);
    touch.send(&mut cx, 10, PointerPhase::Move, 50.0, 30.0);
    touch.send(&mut cx, 20, PointerPhase::Move, 50.0, 80.0);
    assert!(offset(&cx, scroll).y < 0.0);
    touch.send(&mut cx, 30, PointerPhase::Up, 50.0, 90.0);
    assert_eq!(offset(&cx, scroll).y, 0.0);
    assert_eq!(cx.touch_scroll_deadline(), None);
}

#[test]
fn a_removed_list_leaves_no_deadline() {
    let List { mut cx, scroll, .. } = list(ScrollAxes::Vertical);
    let mut touch = Touch::new(&mut cx);
    let lifted = touch.swipe(&mut cx, 0, (50.0, 95.0), (0.0, -10.0), 9);
    assert!(cx.touch_scroll_deadline().is_some());
    let mut q = MutationQueue::new();
    q.despawn_subtree(scroll);
    cx.commit_mutations(q).unwrap();
    assert_eq!(cx.touch_scroll_deadline(), None);
    assert!(
        !cx.advance_animations(Duration::from_millis(lifted + 100))
            .component_updates
            .contains(&scroll)
    );
}
