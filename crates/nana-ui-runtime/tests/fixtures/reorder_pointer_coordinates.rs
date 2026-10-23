//! 滚动列表的窗口坐标输入应命中真实行，并保留右键事件的窗口锚点。
use super::*;
use crate::{AppContext, Button, DocumentId, LayoutViewport, ScrollAxes, ScrollOffset, ScrollView};
use std::sync::Mutex;

/// 自绘行、真实行表面和行内按钮共用窗口到布局坐标转换。
#[test]
fn scrolled_reorder_inputs_keep_the_source_drop_and_secondary_anchor() {
    for live in [false, true] {
        for control in [false, true] {
            if control && !live {
                continue;
            }
            let mut cx = AppContext::new();
            let document = DocumentId::new(1).unwrap();
            let scroll = cx
                .create_component(
                    document,
                    ScrollView::new(ScrollAxes::Vertical).with_layout(|l| {
                        l.width = Some(LengthSpec::Px(200.0));
                        l.height = Some(LengthSpec::Px(120.0));
                    }),
                )
                .unwrap();
            let list = cx
                .create_detached_component(
                    document,
                    ReorderList::new(
                        (0..8).map(|i| ReorderItem::new(format!("row-{i}"), format!("第{i}行"))),
                    )
                    .live_rows(live),
                )
                .unwrap();
            cx.append_child(scroll, list).unwrap();
            if live {
                for i in 0..8 {
                    let row = cx
                        .create_detached_component(
                            document,
                            crate::Stack::column(0.0).with_layout(|l| {
                                l.height = Some(LengthSpec::Px(40.0));
                                l.flex_shrink = Some(0.0);
                            }),
                        )
                        .unwrap();
                    let button = cx
                        .create_detached_component(
                            document,
                            Button::new(format!("操作{i}")).layout(Arc::new(
                                nana_ui_core::LayoutStyle {
                                    width: Some(LengthSpec::Px(60.0)),
                                    ..Default::default()
                                },
                            )),
                        )
                        .unwrap();
                    cx.append_child(row, button).unwrap();
                    cx.append_child(list, row).unwrap();
                }
            }
            cx.layout_document(document, LayoutViewport::new(200.0, 120.0))
                .unwrap();
            let list_bounds = cx.world().component_layout_box(list.stable_id()).unwrap();
            let rows = cx.reorder_row_boxes(list.stable_id(), list_bounds);
            cx.scroll_to(
                scroll,
                ScrollOffset {
                    x: 0.0,
                    y: rows[3].y,
                },
            )
            .unwrap();
            let offset = cx.world().scroll_offset(scroll.stable_id()).unwrap().y;
            cx.rebuild_hit_test(document);
            let x = rows[4].x + if control { 10.0 } else { 150.0 };
            let source_y = rows[4].y + rows[4].height * 0.5 - offset;
            let target_y = rows[3].y + 1.0 - offset;
            let events = Arc::new(Mutex::new(Vec::new()));
            let received = Arc::clone(&events);
            cx.on(list, move |_, event: &ReorderListEvent, _| {
                received.lock().unwrap().push(event.clone());
            })
            .unwrap();
            assert!(
                cx.emit_reorder_row_secondary(list.stable_id(), x, source_y)
                    .unwrap()
            );
            assert_eq!(
                events.lock().unwrap().as_slice(),
                &[ReorderListEvent::Secondary {
                    source: "row-4".into(),
                    x,
                    y: source_y
                }]
            );
            events.lock().unwrap().clear();
            let mut input = crate::HeadlessInput::bind(&mut cx, document);
            // 行内按钮的按下会先武装拖动，移动后接管；行表面立即捕获。
            input
                .pointer(&mut cx, nana_ui_input::PointerPhase::Down, x, source_y)
                .unwrap();
            input
                .pointer(&mut cx, nana_ui_input::PointerPhase::Move, x, target_y)
                .unwrap();
            assert_eq!(
                cx.read(list, |v| v.drag_visual().map(|v| v.source))
                    .unwrap(),
                Some(4)
            );
            input
                .pointer(&mut cx, nana_ui_input::PointerPhase::Up, x, target_y)
                .unwrap();
            assert_eq!(
                events.lock().unwrap().as_slice(),
                &[ReorderListEvent::Reorder {
                    source: "row-4".into(),
                    before: Some("row-3".into())
                }]
            );
            events.lock().unwrap().clear();
            // 真实键盘取消同时释放捕获；随后的 Up 保持取消，下一拖动仍可提交。
            input
                .pointer(&mut cx, nana_ui_input::PointerPhase::Down, x, source_y)
                .unwrap();
            input
                .pointer(&mut cx, nana_ui_input::PointerPhase::Move, x, target_y)
                .unwrap();
            input
                .press(
                    &mut cx,
                    nana_ui_input::KeyInput::named(
                        "Escape",
                        "Escape",
                        nana_ui_input::KeyState::Pressed,
                        Default::default(),
                    ),
                    None,
                    None,
                )
                .unwrap();
            input
                .pointer(&mut cx, nana_ui_input::PointerPhase::Up, x, target_y)
                .unwrap();
            assert_eq!(
                events.lock().unwrap().as_slice(),
                &[ReorderListEvent::Cancelled]
            );
            assert!(cx.world().pointer_captures(document).is_empty());
            events.lock().unwrap().clear();
            for (phase, y) in [
                (nana_ui_input::PointerPhase::Down, source_y),
                (nana_ui_input::PointerPhase::Move, target_y),
                (nana_ui_input::PointerPhase::Up, target_y),
            ] {
                input.pointer(&mut cx, phase, x, y).unwrap();
            }
            assert_eq!(
                events.lock().unwrap().as_slice(),
                &[ReorderListEvent::Reorder {
                    source: "row-4".into(),
                    before: Some("row-3".into()),
                }]
            );
            events.lock().unwrap().clear();
            input
                .pointer(&mut cx, nana_ui_input::PointerPhase::Down, x, source_y)
                .unwrap();
            input
                .pointer(&mut cx, nana_ui_input::PointerPhase::Move, x, target_y)
                .unwrap();
            // 无法反投影的结束点只取消本次操作，随后仍释放列表捕获。
            cx.end_reorder_list_pointer(document, 1, f32::NAN, target_y, false)
                .unwrap();
            assert_eq!(
                events.lock().unwrap().as_slice(),
                &[ReorderListEvent::Cancelled],
                "live={live} control={control}"
            );
            assert_eq!(cx.world().pointer_capture(document, 1), None);
            assert!(!cx.read(list, ReorderList::is_dragging).unwrap());
        }
    }
}
