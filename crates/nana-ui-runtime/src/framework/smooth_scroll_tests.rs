//! Explicit clocks and canonical wheel events exercise the hosted input path.
use super::*;
use crate::{HeadlessInput, LayoutBox, NodeStyle, Text};
use nana_ui_input::{InputModifiers, InputPayload, PointerId, WheelInput, WheelUnit};

fn doc() -> DocumentId {
    DocumentId::new(1).unwrap()
}
fn offset(cx: &AppContext, scroll: Entity<ScrollView>) -> ScrollOffset {
    cx.world.scroll_offset(scroll.id).unwrap()
}
fn wheel(cx: &mut AppContext, input: &mut HeadlessInput, unit: WheelUnit, x: f32, y: f32) -> bool {
    input
        .route(
            cx,
            InputPayload::Wheel(WheelInput {
                pointer_id: PointerId(1),
                x: 10.0,
                y: 10.0,
                delta_x: x,
                delta_y: y,
                unit,
                modifiers: InputModifiers::default(),
            }),
        )
        .unwrap()
        .handled
}
fn fixture() -> (AppContext, Entity<ScrollView>, HeadlessInput) {
    let mut cx = AppContext::new();
    let scroll = cx
        .create_component(
            doc(),
            ScrollView::new(ScrollAxes::Both).scrollbars(nana_ui_core::ScrollbarVisibility::Hidden),
        )
        .unwrap();
    let tail = cx.create_component(doc(), Text::new("tail")).unwrap();
    cx.append_child(scroll, tail).unwrap();
    let mut q = MutationQueue::new();
    q.write_layout(
        scroll.id,
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        },
    );
    q.write_layout(
        tail.id,
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 1000.0,
        },
    );
    cx.commit_mutations(q).unwrap();
    cx.take_system_work();
    cx.rebuild_hit_test(doc());
    let input = HeadlessInput::bind(&mut cx, doc());
    (cx, scroll, input)
}
fn advance(cx: &mut AppContext, millis: u64) -> AnimationFrame {
    cx.advance_animations(Duration::from_millis(millis))
}

#[test]
fn line_wheel_has_intermediate_positions_and_finishes_without_a_deadline() {
    let (mut cx, scroll, mut input) = fixture();
    assert!(wheel(&mut cx, &mut input, WheelUnit::Lines, -1.0, -2.0));
    assert_eq!(offset(&cx, scroll), ScrollOffset::default());
    assert!(cx.next_animation_deadline().is_some());
    let frame = advance(&mut cx, 90);
    let middle = offset(&cx, scroll);
    assert!(frame.component_updates.contains(&scroll.id));
    assert!(middle.x > 0.0 && middle.x < 60.0);
    assert!(middle.y > 0.0 && middle.y < 120.0);
    advance(&mut cx, 180);
    assert_eq!(offset(&cx, scroll), ScrollOffset { x: 60.0, y: 120.0 });
    assert_eq!(cx.next_animation_deadline(), None);
    assert!(!advance(&mut cx, 500).has_updates());
}

#[test]
fn consecutive_notches_accumulate_and_reverse_from_the_visible_position() {
    let (mut cx, scroll, mut input) = fixture();
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    input.set_now(Duration::from_millis(30));
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    advance(&mut cx, 210);
    assert_eq!(offset(&cx, scroll).y, 120.0);
    input.set_now(Duration::from_millis(210));
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -3.0);
    advance(&mut cx, 270);
    let before = offset(&cx, scroll).y;
    input.set_now(Duration::from_millis(270));
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, 1.0);
    advance(&mut cx, 278);
    assert!(offset(&cx, scroll).y < before);
    advance(&mut cx, 450);
    assert!((offset(&cx, scroll).y - (before - 60.0)).abs() < 0.001);
}

#[test]
fn pixels_cancel_the_animation_and_remain_immediate() {
    let (mut cx, scroll, mut input) = fixture();
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    advance(&mut cx, 60);
    let before = offset(&cx, scroll).y;
    wheel(&mut cx, &mut input, WheelUnit::Pixels, 0.0, -3.25);
    assert_eq!(offset(&cx, scroll).y, before + 3.25);
    assert_eq!(cx.next_animation_deadline(), None);
    advance(&mut cx, 500);
    assert_eq!(offset(&cx, scroll).y, before + 3.25);
}

#[test]
fn programmatic_and_assistive_positions_cancel_even_when_unchanged() {
    for assistive in [false, true] {
        let (mut cx, scroll, mut input) = fixture();
        wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
        advance(&mut cx, 60);
        let before = offset(&cx, scroll);
        if assistive {
            cx.apply_accessibility_action(
                doc(),
                AccessibilityActionRequest {
                    target: scroll.id,
                    action: AccessibilityAction::SetScrollOffset(before),
                },
            )
            .unwrap();
        } else {
            cx.scroll_to(scroll, before).unwrap();
        }
        assert!(cx.component_lifecycle.wheel_scrolls.is_empty());
        advance(&mut cx, 500);
        assert_eq!(offset(&cx, scroll), before);
    }
}

#[test]
fn explicit_mutation_cancels_even_a_noop_offset_write() {
    let (mut cx, scroll, mut input) = fixture();
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    let mut q = MutationQueue::new();
    q.set_scroll_offset(scroll.id, ScrollOffset::default());
    cx.commit_mutations(q).unwrap();
    assert_eq!(cx.next_animation_deadline(), None);
    advance(&mut cx, 500);
    assert_eq!(offset(&cx, scroll), ScrollOffset::default());
}

#[test]
fn reduced_motion_finishes_pending_input_and_future_notches_are_immediate() {
    let (mut cx, scroll, mut input) = fixture();
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    advance(&mut cx, 30);
    cx.set_system_reduced_motion(true);
    assert_eq!(offset(&cx, scroll).y, 60.0);
    assert_eq!(cx.next_animation_deadline(), None);
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    assert_eq!(offset(&cx, scroll).y, 120.0);
    assert_eq!(cx.next_animation_deadline(), None);
}

#[test]
fn removed_and_parked_containers_leave_no_deadline() {
    for remove in [false, true] {
        let (mut cx, scroll, mut input) = fixture();
        wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
        let mut q = MutationQueue::new();
        if remove {
            q.despawn_subtree(scroll.id);
        } else {
            q.park_subtree(scroll.id);
        }
        cx.commit_mutations(q).unwrap();
        assert!(cx.component_lifecycle.wheel_scrolls.is_empty());
        assert_eq!(cx.next_animation_deadline(), None);
        assert!(!advance(&mut cx, 500).has_updates());
    }
}

#[test]
fn shrinking_content_clamps_the_target_and_does_not_overshoot() {
    let (mut cx, scroll, mut input) = fixture();
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -5.0);
    advance(&mut cx, 30);
    cx.set_scroll_metrics(
        scroll,
        ScrollMetrics {
            viewport_width: 100.0,
            viewport_height: 100.0,
            content_width: 100.0,
            content_height: 140.0,
            origin_x: 0.0,
            origin_y: 0.0,
        },
    )
    .unwrap();
    advance(&mut cx, 60);
    assert!(offset(&cx, scroll).y <= 40.0);
    advance(&mut cx, 300);
    assert_eq!(offset(&cx, scroll).y, 40.0);
    assert_eq!(cx.next_animation_deadline(), None);
}

#[test]
fn destination_at_edge_bubbles_before_the_inner_animation_finishes() {
    let (mut cx, outer, mut input) = fixture();
    let inner = cx
        .create_component(
            doc(),
            ScrollView::new(ScrollAxes::Vertical)
                .scrollbars(nana_ui_core::ScrollbarVisibility::Hidden),
        )
        .unwrap();
    let content = cx.create_component(doc(), Text::new("inside")).unwrap();
    cx.append_child(outer, inner).unwrap();
    cx.append_child(inner, content).unwrap();
    let mut q = MutationQueue::new();
    q.write_layout(
        inner.id,
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 80.0,
        },
    );
    q.write_layout(
        content.id,
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 140.0,
        },
    );
    cx.commit_mutations(q).unwrap();
    cx.rebuild_hit_test(doc());
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    assert_eq!(offset(&cx, inner).y, 0.0);
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    advance(&mut cx, 180);
    assert_eq!(offset(&cx, inner).y, 60.0);
    assert_eq!(offset(&cx, outer).y, 60.0);
}

#[test]
fn overflow_boxes_and_negative_origins_share_the_same_motion() {
    let mut cx = AppContext::new();
    let id = StableNodeId::new(100).unwrap();
    let child = StableNodeId::new(101).unwrap();
    let mut style = NodeStyle::default();
    Arc::make_mut(&mut style.layout).overflow_x = nana_ui_core::OverflowSpec::Auto;
    Arc::make_mut(&mut style.layout).dir = Some(nana_ui_core::DirSpec::Rtl);
    let mut q = MutationQueue::new();
    q.create(id, doc(), NodeKind::Element { tag: "div".into() });
    q.create(child, doc(), NodeKind::Element { tag: "div".into() });
    q.insert(id, child, None);
    q.set_style(id, style);
    q.write_layout(
        id,
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        },
    );
    q.write_layout(
        child,
        LayoutBox {
            x: -200.0,
            y: 0.0,
            width: 300.0,
            height: 100.0,
        },
    );
    cx.commit_mutations(q).unwrap();
    cx.take_system_work();
    cx.rebuild_hit_test(doc());
    let mut input = HeadlessInput::bind(&mut cx, doc());
    assert!(wheel(&mut cx, &mut input, WheelUnit::Lines, 1.0, 0.0));
    advance(&mut cx, 90);
    let middle = cx.world.scroll_offset(id).unwrap().x;
    assert!(middle < 0.0 && middle > -60.0);
    advance(&mut cx, 180);
    assert_eq!(cx.world.scroll_offset(id).unwrap().x, -60.0);
    assert_eq!(cx.world.layout_box(child).unwrap().x, -200.0);
    assert!(!cx.is_scroll_view(id));
    assert_eq!(cx.next_animation_deadline(), None);
}

#[test]
fn samples_report_effective_offsets_without_style_text_or_layout_work() {
    use std::sync::Mutex;
    let (mut cx, scroll, mut input) = fixture();
    let changes = Arc::new(Mutex::new(Vec::new()));
    let users = Arc::new(Mutex::new(Vec::new()));
    let changed = changes.clone();
    cx.on(scroll, move |_, event: &ScrollChanged, _| {
        changed.lock().unwrap().push(event.offset);
    })
    .unwrap();
    let user = users.clone();
    cx.on(scroll, move |_, event: &crate::UserScroll, _| {
        user.lock().unwrap().push(event.offset);
    })
    .unwrap();
    cx.take_system_work();
    let before = cx.world.layout_box(scroll.id);
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    assert!(changes.lock().unwrap().is_empty());
    for time in [30, 90, 180] {
        advance(&mut cx, time);
        let actual = offset(&cx, scroll);
        assert_eq!(changes.lock().unwrap().last(), Some(&actual));
        assert_eq!(users.lock().unwrap().last(), Some(&actual));
        assert_eq!(cx.world.layout_box(scroll.id), before);
        let work = cx.take_system_work();
        assert!(work.style.is_empty());
        assert!(work.text.is_empty());
        assert!(work.layout_frontier_seeds.is_empty());
        let accessible = cx.world.project_accessibility_delta(&work);
        let container = accessible
            .updated
            .iter()
            .find(|node| node.id == scroll.id)
            .unwrap();
        assert_eq!(container.scroll_y.unwrap().value, actual.y as f64);
    }
    assert_eq!(cx.next_animation_deadline(), None);
}

#[test]
fn an_event_observer_can_cancel_a_sample_with_an_immediate_position() {
    let (mut cx, scroll, mut input) = fixture();
    cx.on(scroll, move |_, _: &ScrollChanged, view| {
        view.mutations()
            .set_scroll_offset(scroll.id, ScrollOffset { x: 0.0, y: 15.0 });
    })
    .unwrap();
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    advance(&mut cx, 30);
    assert_eq!(offset(&cx, scroll).y, 15.0);
    assert_eq!(cx.next_animation_deadline(), None);
    advance(&mut cx, 300);
    assert_eq!(offset(&cx, scroll).y, 15.0);
}

#[test]
fn dragging_the_thumb_cancels_motion_and_stays_under_the_pointer() {
    let (mut cx, scroll, mut input) = fixture();
    cx.update_component(scroll, |view, _| {
        view.scrollbars = Some(nana_ui_core::ScrollbarVisibility::Always)
    })
    .unwrap();
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    advance(&mut cx, 60);
    let Some(crate::ComponentGeometry::Scrollbar {
        vertical: Some(bar),
        ..
    }) = cx.world.component_geometry(scroll.id)
    else {
        panic!("bar")
    };
    let x = bar.thumb.x + bar.thumb.width / 2.0;
    let y = bar.thumb.y + bar.thumb.height / 2.0;
    assert!(
        cx.begin_scrollbar_drag(1, scroll.id, nana_ui_core::ScrollbarAxis::Vertical, x, y)
            .unwrap()
    );
    assert!(cx.component_lifecycle.wheel_scrolls.is_empty());
    assert!(cx.update_scrollbar_drag(doc(), 1, x, y + 20.0).unwrap());
    let dragged = offset(&cx, scroll);
    advance(&mut cx, 500);
    assert_eq!(offset(&cx, scroll), dragged);
}

#[test]
fn textarea_line_wheel_survives_projection_and_caret_navigation_cancels_it() {
    let mut cx = AppContext::new();
    let area = cx
        .create_component(doc(), TextArea::new("line\n".repeat(120)))
        .unwrap();
    let mut q = MutationQueue::new();
    q.write_layout(
        area.id,
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        },
    );
    cx.commit_mutations(q).unwrap();
    let work = cx.take_system_work();
    cx.resolve_styles(&work.style).unwrap();
    cx.shape_text(&work.text, &mut crate::MeasureTextShaper)
        .unwrap();
    cx.rebuild_hit_test(doc());
    let mut input = HeadlessInput::bind(&mut cx, doc());
    assert!(wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -2.0));
    advance(&mut cx, 90);
    let y = cx.world.scroll_offset(area.id).unwrap().y;
    assert!(y > 0.0 && y < 120.0);
    assert_eq!(cx.read(area, |area| area.scroll_offset.y).unwrap(), y);
    cx.update_component(area, |area, _| area.invalid = true)
        .unwrap();
    advance(&mut cx, 180);
    assert_eq!(cx.world.scroll_offset(area.id).unwrap().y, 120.0);
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    cx.set_editable_selection(area, crate::TextSelection::caret(10))
        .unwrap();
    assert!(cx.component_lifecycle.wheel_scrolls.is_empty());
}

#[test]
fn virtual_list_windows_follow_each_sample_instead_of_the_pending_destination() {
    let (mut cx, scroll, mut input) = fixture();
    let list = cx.create_component(doc(), List::new()).unwrap();
    cx.append_child(scroll, list).unwrap();
    let layout = crate::VirtualListLayout::new(std::iter::repeat_n(20.0, 100));
    let mut items = crate::VirtualListItems::<usize, Text>::default();
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -2.0);
    for time in [30, 90, 180] {
        advance(&mut cx, time);
        let actual = offset(&cx, scroll).y;
        let expected = layout.window(actual, 100.0, 0.0);
        let window = cx
            .sync_virtual_list_retained_in(
                scroll,
                list,
                &mut items,
                &layout,
                0.0,
                0,
                &[],
                |index| index,
                |key| Some(*key),
                |index, _| Text::new(format!("row {index}")),
            )
            .unwrap();
        assert_eq!(window.range, expected.range);
        assert_eq!(items.mounted_keys().first(), Some(&window.range.start));
    }
    assert_eq!(cx.next_animation_deadline(), None);
}

#[test]
fn rich_editor_line_wheel_retains_its_sample_until_selection_changes() {
    let mut cx = AppContext::new();
    let editor = cx
        .create_component(doc(), crate::RichTextEditor::new("line\n".repeat(100)))
        .unwrap();
    let mut q = MutationQueue::new();
    q.write_layout(
        editor.id,
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        },
    );
    cx.commit_mutations(q).unwrap();
    let mut fonts = nana_text::font::FontSystem::hermetic();
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../nana-ui/assets/fonts/NotoSansSC-Variable.ttf"
            ),
            &nana_text::font::FaceDescriptor::default(),
        )
        .unwrap();
    let engine = Arc::new(std::sync::Mutex::new(nana_text::NativeTextEngine::new(
        fonts,
    )));
    let work = cx.take_system_work();
    cx.resolve_styles(&work.style).unwrap();
    cx.shape_text(&work.text, &mut crate::NanaTextEngineShaper::new(engine))
        .unwrap();
    cx.rebuild_hit_test(doc());
    let mut input = HeadlessInput::bind(&mut cx, doc());
    assert!(wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -2.0));
    advance(&mut cx, 90);
    let y = cx.world.scroll_offset(editor.id).unwrap().y;
    assert!(y > 0.0 && y < 120.0);
    assert_eq!(cx.read(editor, |editor| editor.scroll_offset.y).unwrap(), y);
    cx.reproject_component(editor).unwrap();
    advance(&mut cx, 180);
    assert_eq!(cx.world.scroll_offset(editor.id).unwrap().y, 120.0);
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    cx.rich_edit(editor, crate::RichEditCommand::Select(10..10))
        .unwrap();
    assert!(cx.component_lifecycle.wheel_scrolls.is_empty());
}

#[test]
fn focus_reveal_is_immediate_and_long_frame_gaps_land_at_the_destination() {
    let (mut cx, scroll, mut input) = fixture();
    let button = cx.create_component(doc(), Button::new("focus")).unwrap();
    cx.append_child(scroll, button).unwrap();
    let mut q = MutationQueue::new();
    q.write_layout(
        button.id,
        LayoutBox {
            x: 0.0,
            y: 150.0,
            width: 80.0,
            height: 30.0,
        },
    );
    cx.commit_mutations(q).unwrap();
    cx.rebuild_hit_test(doc());
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    cx.focus_node(doc(), button.id).unwrap();
    assert_eq!(offset(&cx, scroll).y, 80.0);
    assert!(cx.component_lifecycle.wheel_scrolls.is_empty());
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    advance(&mut cx, 10_000);
    assert_eq!(offset(&cx, scroll).y, 140.0);
    assert!(cx.component_lifecycle.wheel_scrolls.is_empty());
}

#[test]
fn a_blocking_overlay_keeps_wheel_motion_inside_its_own_subtree() {
    let (mut cx, background, _input) = fixture();
    let host = cx.create_component(doc(), OverlayHost::new()).unwrap();
    cx.append_child(background, host).unwrap();
    let dialog = cx
        .create_component(doc(), crate::Dialog::new("dialog"))
        .unwrap();
    cx.append_child(host, dialog).unwrap();
    let scroll = cx
        .create_detached_component(
            doc(),
            ScrollView::new(ScrollAxes::Vertical)
                .scrollbars(nana_ui_core::ScrollbarVisibility::Hidden),
        )
        .unwrap();
    let content = cx.create_component(doc(), Text::new("content")).unwrap();
    cx.set_modal_slots(
        dialog,
        crate::ModalSlots {
            body: Some(scroll.id),
            ..Default::default()
        },
    )
    .unwrap();
    cx.append_child(scroll, content).unwrap();
    let mut q = MutationQueue::new();
    for (id, width, height) in [
        (dialog.id, 100.0, 100.0),
        (scroll.id, 80.0, 80.0),
        (content.id, 80.0, 140.0),
    ] {
        q.write_layout(
            id,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
        );
    }
    cx.commit_mutations(q).unwrap();
    cx.activate_overlay(host, dialog).unwrap();
    cx.rebuild_hit_test(doc());
    let delta = ScrollOffset { x: 0.0, y: 60.0 };
    assert_eq!(
        cx.wheel_scroll_from(doc(), Some(content.id), delta, true, true)
            .unwrap(),
        Some(scroll.id)
    );
    assert_eq!(
        cx.wheel_scroll_from(doc(), Some(content.id), delta, true, true)
            .unwrap(),
        None
    );
    advance(&mut cx, 180);
    assert_eq!(offset(&cx, scroll).y, 60.0);
    assert_eq!(offset(&cx, background).y, 0.0);
    assert!(cx.component_lifecycle.wheel_scrolls.is_empty());
}

#[test]
fn non_finite_wheel_offsets_never_start_a_gesture() {
    let (mut cx, scroll, _input) = fixture();
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(matches!(
            cx.wheel_scroll_from(
                doc(),
                Some(scroll.id),
                ScrollOffset { x: 0.0, y: value },
                true,
                false
            ),
            Err(FrameworkError::InvalidInput)
        ));
        assert_eq!(offset(&cx, scroll), ScrollOffset::default());
        assert_eq!(cx.next_animation_deadline(), None);
    }
}

#[test]
fn user_scroll_observes_the_position_committed_by_scroll_changed() {
    use std::sync::Mutex;
    let (mut cx, scroll, mut input) = fixture();
    cx.on(scroll, move |_, _: &ScrollChanged, cx| {
        cx.mutations()
            .set_scroll_offset(scroll.id, ScrollOffset { x: 0.0, y: 15.0 });
    })
    .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let users = seen.clone();
    cx.on(scroll, move |_, event: &crate::UserScroll, _| {
        users.lock().unwrap().push(event.offset);
    })
    .unwrap();
    wheel(&mut cx, &mut input, WheelUnit::Lines, -1.0, -1.0);
    advance(&mut cx, 30);
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &[ScrollOffset { x: 0.0, y: 15.0 }]
    );
    assert_eq!(cx.next_animation_deadline(), None);
}

#[test]
fn listeners_registered_after_unobserved_samples_receive_future_offsets() {
    use std::sync::Mutex;
    let (mut cx, scroll, mut input) = fixture();
    wheel(&mut cx, &mut input, WheelUnit::Lines, 0.0, -1.0);
    advance(&mut cx, 30);
    let work = cx.take_system_work();
    assert!(work.style.is_empty() && work.text.is_empty() && work.layout_frontier_seeds.is_empty());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let changes = seen.clone();
    cx.on(scroll, move |_, event: &ScrollChanged, _| {
        changes.lock().unwrap().push(event.offset);
    })
    .unwrap();
    advance(&mut cx, 90);
    advance(&mut cx, 180);
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(seen.lock().unwrap().last(), Some(&offset(&cx, scroll)));
    assert_eq!(offset(&cx, scroll).y, 60.0);
    assert_eq!(cx.next_animation_deadline(), None);
}
