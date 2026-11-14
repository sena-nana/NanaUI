//! CPU-only smooth-scroll sampling probe; excludes layout, scene extraction and GPU.
//! Run with --ignored --nocapture. Compare identical build profiles on one machine.

use crate::{ALLOCATIONS, COUNTING};
use nana_ui_core::ScrollbarVisibility;
use nana_ui_input::{InputModifiers, InputPayload, PointerId, WheelInput, WheelUnit};
use nana_ui_runtime::{
    AppContext, DocumentId, HeadlessInput, LayoutBox, MutationQueue, ScrollAxes, ScrollChanged,
    ScrollView, Text, UserScroll,
};
use std::cell::Cell;
use std::time::{Duration, Instant};

#[test]
#[ignore = "manual CPU/allocation measurement, not a window frame-rate test"]
fn smooth_scroll_sampling_cost() {
    for listeners in [false, true] {
        for visibility in [ScrollbarVisibility::Hidden, ScrollbarVisibility::AutoHide] {
            let mut cx = AppContext::new();
            let doc = DocumentId::new(1).unwrap();
            let scroll = cx
                .create_component(
                    doc,
                    ScrollView::new(ScrollAxes::Both).scrollbars(visibility),
                )
                .unwrap();
            let tail = cx.create_component(doc, Text::new("tail")).unwrap();
            cx.append_child(scroll, tail).unwrap();
            let mut q = MutationQueue::new();
            q.write_layout(
                scroll.stable_id(),
                LayoutBox {
                    x: 0.,
                    y: 0.,
                    width: 100.,
                    height: 100.,
                },
            );
            q.write_layout(
                tail.stable_id(),
                LayoutBox {
                    x: 0.,
                    y: 0.,
                    width: 1000.,
                    height: 1000.,
                },
            );
            cx.commit_mutations(q).unwrap();
            cx.rebuild_hit_test(doc);
            if listeners {
                cx.on(scroll, |_, _: &ScrollChanged, _| {}).unwrap();
                cx.on(scroll, |_, _: &UserScroll, _| {}).unwrap();
            }
            let mut input = HeadlessInput::bind(&mut cx, doc);
            let mut samples = 0_u64;
            let mut micros = Vec::with_capacity(512);
            let mut allocations = 0;
            for gesture in 0..544_u64 {
                let start = Duration::from_millis(gesture * 200);
                input.set_now(start);
                input
                    .route(
                        &mut cx,
                        InputPayload::Wheel(WheelInput {
                            pointer_id: PointerId(1),
                            x: 10.,
                            y: 10.,
                            delta_x: 0.,
                            delta_y: if gesture % 2 == 0 { -1. } else { 1. },
                            unit: WheelUnit::Lines,
                            modifiers: InputModifiers::default(),
                        }),
                    )
                    .unwrap();
                cx.take_system_work();
                ALLOCATIONS.with(|v| v.set(0));
                COUNTING.with(|v| v.set(gesture >= 32));
                let before = Instant::now();
                while let Some(next) = cx.next_animation_deadline() {
                    if next > start + Duration::from_millis(180) {
                        break;
                    }
                    let frame = cx.advance_animations(next);
                    if gesture >= 32 {
                        samples += u64::from(frame.has_updates());
                    }
                    cx.take_system_work();
                }
                let elapsed = before.elapsed().as_nanos();
                COUNTING.with(|v| v.set(false));
                if gesture >= 32 {
                    allocations += ALLOCATIONS.with(Cell::get);
                    micros.push(elapsed as f64 / 1000.);
                }
            }
            assert_eq!(
                samples,
                512 * 23,
                "compare identical effective sample counts"
            );
            assert_eq!(cx.world().scroll_offset(scroll.stable_id()).unwrap().y, 0.0);
            micros.sort_by(f64::total_cmp);
            println!(
                "listeners={listeners} bars={visibility:?} gestures=512 samples={samples} allocations={allocations} median_us={:.2} p95_us={:.2}",
                micros[256], micros[486]
            );
        }
    }
}
