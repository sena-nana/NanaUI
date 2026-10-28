// 真实键盘与指针路由验证列表拖动的 Escape 取消及后续松手行为。

#[test]
fn escape_cancels_real_drag_releases_capture_and_keeps_pointer_up_from_reordering() {
    use nana_ui_input::{KeyInput, KeyState, PointerPhase};
    for from_button in [false, true] {
        let mut fixture = ButtonRows::new(false);
        let (x, y) = if from_button {
            fixture.on_button(0)
        } else {
            fixture.on_body(0)
        };
        let (_, end_y) = fixture.on_body(2);
        fixture.pointer(PointerPhase::Down, x, y);
        fixture.pointer(PointerPhase::Move, x, end_y + 20.0);
        assert!(fixture.dragging());
        assert!(fixture.captured());
        let result = fixture
            .input
            .press(
                &mut fixture.context,
                KeyInput::named("Escape", "Escape", KeyState::Pressed, Default::default()),
                None,
                None,
            )
            .unwrap();
        assert!(result.handled && result.prevent_default);
        assert!(!fixture.dragging());
        assert!(!fixture.captured());
        assert_eq!(fixture.painted_drag(), None);
        assert_eq!(fixture.take_events(), [ReorderListEvent::Cancelled]);
        fixture.pointer(PointerPhase::Move, x, end_y + 25.0);
        fixture.pointer(PointerPhase::Up, x, end_y + 25.0);
        assert_eq!(fixture.take_events(), Vec::<ReorderListEvent>::new());
        assert!(fixture.take_clicks().is_empty());
        // 清理后的下一次按下继续进入正常排序，旧手势不会吞掉新拖动。
        fixture.pointer(PointerPhase::Down, x, y);
        fixture.pointer(PointerPhase::Move, x, end_y + 20.0);
        fixture.pointer(PointerPhase::Up, x, end_y + 20.0);
        assert!(matches!(
            fixture.take_events().as_slice(),
            [ReorderListEvent::Reorder { .. }]
        ));
    }
}

#[test]
fn escape_release_and_repeat_leave_the_active_drag_until_the_first_press() {
    use nana_ui_input::{KeyInput, KeyState, PointerPhase};
    let mut fixture = ButtonRows::new(false);
    let (x, y) = fixture.on_body(0);
    let (_, end_y) = fixture.on_body(2);
    fixture.pointer(PointerPhase::Down, x, y);
    fixture.pointer(PointerPhase::Move, x, end_y);
    let release = KeyInput::named("Escape", "Escape", KeyState::Released, Default::default());
    fixture
        .input
        .press(&mut fixture.context, release, None, None)
        .unwrap();
    let mut repeat = KeyInput::named("Escape", "Escape", KeyState::Pressed, Default::default());
    repeat.repeat = true;
    fixture
        .input
        .press(&mut fixture.context, repeat, None, None)
        .unwrap();
    assert!(fixture.dragging() && fixture.captured());
    assert!(fixture.take_events().is_empty());
    fixture
        .input
        .press(
            &mut fixture.context,
            KeyInput::named("Escape", "Escape", KeyState::Pressed, Default::default()),
            None,
            None,
        )
        .unwrap();
    assert_eq!(fixture.take_events(), [ReorderListEvent::Cancelled]);
    fixture.pointer(PointerPhase::Up, x, end_y);
    assert!(fixture.take_events().is_empty());
}
