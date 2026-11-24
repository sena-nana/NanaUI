//! The value thumb, the bounds, the reset button and the fine drag.

use std::sync::{Arc, Mutex};

use nana_ui_input::{
    CommittedText, InputModifiers, InputPayload, KeyInput, KeyState, LogicalKey, PhysicalKey,
    PointerInput, PointerPhase,
};

use crate::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityRole, AppContext, DocumentId,
    Entity, HeadlessInput, IconButton, LayoutBox, LayoutPartKind, LayoutViewport, RangeChanged,
    RangeField, RangeInput, RangeValueThumb,
};

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Heard {
    Input(f64),
    Changed(f64),
}

fn listen(cx: &mut AppContext, range: Entity<RangeField>) -> Arc<Mutex<Vec<Heard>>> {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let input = Arc::clone(&heard);
    cx.on(range, move |_, event: &RangeInput, _| {
        input.lock().unwrap().push(Heard::Input(event.value));
    })
    .unwrap();
    let changed = Arc::clone(&heard);
    cx.on(range, move |_, event: &RangeChanged, _| {
        changed.lock().unwrap().push(Heard::Changed(event.value));
    })
    .unwrap();
    heard
}

fn take(heard: &Mutex<Vec<Heard>>) -> Vec<Heard> {
    std::mem::take(&mut *heard.lock().unwrap())
}

/// `view` created, assembled and laid out across a 400px viewport.
fn laid_out(view: RangeField) -> (AppContext, Entity<RangeField>) {
    let mut cx = AppContext::new();
    let range = cx.create_component(document(), view).unwrap();
    cx.assemble_range_field(range).unwrap();
    relayout(&mut cx);
    (cx, range)
}

fn relayout(cx: &mut AppContext) {
    cx.layout_document(document(), LayoutViewport::new(400.0, 100.0))
        .unwrap();
    cx.rebuild_hit_test(document());
}

/// The track (the span a thumb's centre travels) and the value thumb.
fn geometry(cx: &AppContext, range: Entity<RangeField>) -> (LayoutBox, Option<RangeValueThumb>) {
    let geometry = cx
        .world()
        .range_geometry(range.stable_id())
        .expect("range geometry");
    (geometry.track, geometry.value_thumb)
}

fn value(cx: &AppContext, range: Entity<RangeField>) -> f64 {
    cx.read(range, |range| range.value).unwrap()
}

fn part(cx: &AppContext, range: Entity<RangeField>, kind: LayoutPartKind) -> Option<LayoutBox> {
    let result = cx.world().layout_result(range.stable_id())?;
    let origin = cx.world().layout_box(range.stable_id())?;
    result
        .parts
        .iter()
        .find(|part| part.kind == kind)
        .map(|part| LayoutBox {
            x: origin.x + part.bounds.x,
            y: origin.y + part.bounds.y,
            ..part.bounds
        })
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn alt_slows_a_drag_to_a_tenth_and_switching_it_never_jumps() {
    let (mut cx, range) = laid_out(RangeField::new(0.0, 0.0, 100.0, 0.01));
    let (track, _) = geometry(&cx, range);
    let per_pixel = 100.0 / f64::from(track.width);
    let start = track.x + track.width * 0.2;
    cx.begin_range_drag(document(), 1, range.stable_id(), start)
        .unwrap();
    let at_press = value(&cx, range);
    assert!(close(at_press, 20.0), "{at_press}");

    // Alt pressed while the pointer rests: nothing moves.
    cx.update_range_drag_fine(document(), 1, start, true)
        .unwrap();
    assert!(close(value(&cx, range), at_press));
    // Forty pixels with Alt move a tenth as far.
    cx.update_range_drag_fine(document(), 1, start + 40.0, true)
        .unwrap();
    let fine = value(&cx, range);
    let expected = at_press + 40.0 * per_pixel * 0.1;
    assert!(
        (fine - expected).abs() <= 0.005 + 1e-9,
        "{fine} vs {expected}"
    );
    // Alt released where the pointer is: still no jump, although the
    // pointer no longer stands over the value.
    cx.update_range_drag_fine(document(), 1, start + 40.0, false)
        .unwrap();
    assert!(close(value(&cx, range), fine));
    // From there, the ordinary pace again.
    cx.update_range_drag_fine(document(), 1, start + 50.0, false)
        .unwrap();
    let after = value(&cx, range);
    let expected = fine + 10.0 * per_pixel;
    assert!(
        (after - expected).abs() <= 0.01 + 1e-9,
        "{after} vs {expected}"
    );
    cx.end_range_drag(document(), 1, false).unwrap();
}

#[test]
fn a_drag_without_alt_still_follows_the_pointer() {
    let (mut cx, range) = laid_out(RangeField::new(0.0, 0.0, 100.0, 1.0));
    let (track, _) = geometry(&cx, range);
    cx.begin_range_drag(
        document(),
        1,
        range.stable_id(),
        track.x + track.width * 0.25,
    )
    .unwrap();
    cx.update_range_drag(document(), 1, track.x + track.width * 0.75)
        .unwrap();
    assert_eq!(value(&cx, range), 75.0);
    cx.end_range_drag(document(), 1, false).unwrap();
}

#[test]
fn a_press_on_the_value_thumb_drags_from_the_value_without_a_jump() {
    let (mut cx, range) = laid_out(RangeField::new(30.0, 0.0, 100.0, 1.0).value_thumb(true));
    let heard = listen(&mut cx, range);
    let (track, thumb) = geometry(&cx, range);
    let thumb = thumb.expect("a value thumb");
    // Off the capsule's centre, still on it.
    let press = thumb.capsule.x + thumb.capsule.width * 0.8;
    cx.begin_range_drag(document(), 1, range.stable_id(), press)
        .unwrap();
    assert_eq!(value(&cx, range), 30.0);
    assert!(take(&heard).is_empty(), "the press moves nothing");
    let per_pixel = 100.0 / f64::from(track.width);
    cx.update_range_drag(document(), 1, press + 40.0).unwrap();
    assert_eq!(value(&cx, range), (30.0 + 40.0 * per_pixel).round());
    cx.end_range_drag(document(), 1, false).unwrap();

    // A press on the track beside the capsule jumps there first.
    let beside = track.x + track.width * 0.9;
    cx.begin_range_drag(document(), 2, range.stable_id(), beside)
        .unwrap();
    assert_eq!(value(&cx, range), 90.0);
    cx.update_range_drag(document(), 2, beside - 20.0).unwrap();
    assert_eq!(value(&cx, range), (90.0 - 20.0 * per_pixel).round());
    cx.end_range_drag(document(), 2, false).unwrap();
}

#[test]
fn the_value_thumb_stays_on_the_track_between_the_bounds() {
    for show_label in [true, false] {
        let view = RangeField::new(0.0, 0.0, 200.0, 1.0)
            .label("缩放")
            .show_label(show_label)
            .unit("%")
            .value_thumb(true)
            .show_bounds(true)
            .default_value(Some(100.0));
        let (mut cx, range) = laid_out(view);
        let minimum = part(&cx, range, LayoutPartKind::Minimum).expect("minimum part");
        let maximum = part(&cx, range, LayoutPartKind::Maximum).expect("maximum part");
        let reset = part(&cx, range, LayoutPartKind::TrailingContent).expect("reset part");
        assert!(
            part(&cx, range, LayoutPartKind::Value).is_none(),
            "no readout"
        );
        assert_eq!(
            part(&cx, range, LayoutPartKind::Label).is_some(),
            show_label
        );
        assert!(minimum.x + minimum.width < maximum.x);
        assert!(maximum.x + maximum.width <= reset.x);
        let Some(crate::RangeGeometry {
            bounds: Some([low, high]),
            ..
        }) = cx.world().range_geometry(range.stable_id())
        else {
            panic!("bounds are drawn");
        };
        assert_eq!(&*low.content, "0");
        assert_eq!(&*high.content, "200%");
        for value in [0.0, 200.0] {
            cx.set_range_value(range, value).unwrap();
            relayout(&mut cx);
            let (track, thumb) = geometry(&cx, range);
            let capsule = thumb.unwrap().capsule;
            assert!(
                capsule.x >= minimum.x + minimum.width,
                "{capsule:?} {minimum:?}"
            );
            assert!(
                capsule.x + capsule.width <= maximum.x,
                "{capsule:?} {maximum:?}"
            );
            // The track is where the capsule's centre goes.
            let centre = capsule.x + capsule.width / 2.0;
            let end = if value == 0.0 {
                track.x
            } else {
                track.x + track.width
            };
            assert!((centre - end).abs() < 0.01, "{centre} vs {end}");
        }
    }
}

#[test]
fn bounds_alone_lay_the_track_between_them_beside_the_readout() {
    let view = RangeField::new(1.0, 0.0, 2.0, 0.1)
        .label("时长")
        .unit("秒")
        .show_bounds(true);
    let (cx, range) = laid_out(view);
    let label = part(&cx, range, LayoutPartKind::Label).unwrap();
    let minimum = part(&cx, range, LayoutPartKind::Minimum).unwrap();
    let maximum = part(&cx, range, LayoutPartKind::Maximum).unwrap();
    let readout = part(&cx, range, LayoutPartKind::Value).unwrap();
    let (track, thumb) = geometry(&cx, range);
    assert!(thumb.is_none());
    assert!(label.x + label.width <= minimum.x);
    assert!(minimum.x + minimum.width < track.x);
    assert!(track.x + track.width < maximum.x);
    assert!(maximum.x + maximum.width <= readout.x);
    let Some(crate::RangeGeometry {
        bounds: Some([low, high]),
        ..
    }) = cx.world().range_geometry(range.stable_id())
    else {
        panic!("bounds are drawn");
    };
    assert_eq!(&*low.content, "0.0");
    assert_eq!(&*high.content, "2.0 秒");
}

#[test]
fn a_click_on_the_value_thumb_types_enter_commits_and_escape_reverts() {
    let (mut cx, range) = laid_out(
        RangeField::new(30.0, 0.0, 100.0, 1.0)
            .label("音量")
            .value_thumb(true),
    );
    let heard = listen(&mut cx, range);
    let (_, thumb) = geometry(&cx, range);
    let capsule = thumb.unwrap().capsule;
    let (x, y) = (
        capsule.x + capsule.width / 2.0,
        capsule.y + capsule.height / 2.0,
    );
    let mut input = HeadlessInput::bind(&mut cx, document());
    let pointer = |phase, x: f32| InputPayload::Pointer(PointerInput::mouse(phase, x, y));
    let key = |name: &'static str| {
        InputPayload::Key(KeyInput {
            physical: PhysicalKey(name.into()),
            logical: LogicalKey(name.into()),
            state: KeyState::Pressed,
            repeat: false,
            modifiers: InputModifiers::empty(),
        })
    };
    let typed = |text: &str| {
        InputPayload::Text(CommittedText {
            text: text.to_owned(),
            key: None,
        })
    };

    // A press that moves under two pixels is a click: it types.
    input
        .route(&mut cx, pointer(PointerPhase::Down, x))
        .unwrap();
    input
        .route(&mut cx, pointer(PointerPhase::Move, x + 1.0))
        .unwrap();
    input
        .route(&mut cx, pointer(PointerPhase::Up, x + 1.0))
        .unwrap();
    let parts = cx
        .assembled_slot::<crate::view_components::RangeParts>(range.stable_id())
        .copied()
        .unwrap();
    assert!(cx.read(range, |range| range.editing).unwrap());
    assert_eq!(cx.world().focused(document()), Some(parts.input));
    assert!(take(&heard).is_empty());
    relayout(&mut cx);
    // The field lies over the capsule.
    let field = cx.world().layout_box(parts.input).unwrap();
    let centre = field.x + field.width / 2.0;
    assert!((centre - x).abs() < 1.0, "{field:?} over {capsule:?}");
    let Some(crate::RangeGeometry {
        value_thumb: Some(thumb),
        ..
    }) = cx.world().range_geometry(range.stable_id())
    else {
        panic!("value thumb");
    };
    assert!(thumb.editing, "the capsule makes way");

    input.route(&mut cx, typed("142")).unwrap();
    input.route(&mut cx, key("Enter")).unwrap();
    assert_eq!(
        take(&heard),
        [Heard::Input(100.0), Heard::Changed(100.0)],
        "held inside the range"
    );
    assert!(!cx.read(range, |range| range.editing).unwrap());
    assert_eq!(cx.world().focused(document()), Some(range.stable_id()));

    // Enter on the range types too; Escape gives the value back unchanged.
    input.route(&mut cx, key("Enter")).unwrap();
    assert!(cx.read(range, |range| range.editing).unwrap());
    assert_eq!(cx.world().focused(document()), Some(parts.input));
    input.route(&mut cx, typed("7")).unwrap();
    input.route(&mut cx, key("Escape")).unwrap();
    assert!(take(&heard).is_empty());
    assert!(!cx.read(range, |range| range.editing).unwrap());
    assert_eq!(value(&cx, range), 100.0);
    assert_eq!(cx.world().focused(document()), Some(range.stable_id()));

    // Escape with nothing typed ends typing as well.
    input.route(&mut cx, key("Enter")).unwrap();
    input.route(&mut cx, key("Escape")).unwrap();
    assert!(!cx.read(range, |range| range.editing).unwrap());

    // A drag on the capsule does not type.
    relayout(&mut cx);
    let (_, thumb) = geometry(&cx, range);
    let capsule = thumb.unwrap().capsule;
    let x = capsule.x + capsule.width / 2.0;
    input
        .route(&mut cx, pointer(PointerPhase::Down, x))
        .unwrap();
    input
        .route(&mut cx, pointer(PointerPhase::Move, x - 30.0))
        .unwrap();
    input
        .route(&mut cx, pointer(PointerPhase::Up, x - 30.0))
        .unwrap();
    assert!(!cx.read(range, |range| range.editing).unwrap());
    assert!(value(&cx, range) < 100.0);
}

#[test]
fn the_reset_button_is_disabled_at_the_default_and_puts_it_back() {
    let (mut cx, range) =
        laid_out(RangeField::new(50.0, 0.0, 100.0, 1.0).default_value(Some(50.0)));
    let heard = listen(&mut cx, range);
    let parts = cx
        .assembled_slot::<crate::view_components::RangeParts>(range.stable_id())
        .copied()
        .unwrap();
    let reset = Entity::<IconButton>::from_stable_id(parts.reset);
    let node = |cx: &AppContext| cx.world().accessibility(parts.reset).unwrap().into_owned();
    assert_eq!(node(&cx).role, AccessibilityRole::Button);
    assert_eq!(node(&cx).label.as_deref(), Some("恢复默认"));
    assert!(node(&cx).disabled, "at the default");
    assert!(!cx.world().interaction(parts.reset).unwrap().focusable);

    cx.set_range_value(range, 20.0).unwrap();
    take(&heard);
    assert!(!cx.read(reset, |button| button.disabled).unwrap());
    relayout(&mut cx);
    // It sits in the square the row keeps for it.
    let square = part(&cx, range, LayoutPartKind::TrailingContent).unwrap();
    let button = cx.world().layout_box(parts.reset).unwrap();
    assert!(
        (button.x - square.x).abs() < 0.5,
        "{button:?} in {square:?}"
    );
    assert!(
        (button.y - square.y).abs() < 0.5,
        "{button:?} in {square:?}"
    );

    assert!(
        cx.apply_accessibility_action(
            document(),
            AccessibilityActionRequest {
                target: parts.reset,
                action: AccessibilityAction::Click,
            },
        )
        .unwrap()
    );
    assert_eq!(take(&heard), [Heard::Input(50.0), Heard::Changed(50.0)]);
    assert_eq!(value(&cx, range), 50.0);
    assert!(cx.read(reset, |button| button.disabled).unwrap());

    // Without a default the button goes.
    cx.update_component(range, |range, _| range.default_value = None)
        .unwrap();
    relayout(&mut cx);
    assert!(part(&cx, range, LayoutPartKind::TrailingContent).is_none());
    assert!(cx.world().node_style(parts.reset).unwrap().layout.hidden);
}

/// Alt reaches the drag from the pointer's own modifiers.
#[test]
fn alt_on_the_pointer_slows_the_drag() {
    let (mut cx, range) = laid_out(RangeField::new(0.0, 0.0, 100.0, 0.01));
    let (track, _) = geometry(&cx, range);
    let per_pixel = 100.0 / f64::from(track.width);
    let y = track.y + track.height / 2.0;
    let start = track.x + track.width * 0.5;
    let mut input = HeadlessInput::bind(&mut cx, document());
    let at = |phase, x: f32, alt: bool| {
        InputPayload::Pointer(PointerInput {
            modifiers: InputModifiers {
                alt,
                ..InputModifiers::empty()
            },
            ..PointerInput::mouse(phase, x, y)
        })
    };
    input
        .route(&mut cx, at(PointerPhase::Down, start, false))
        .unwrap();
    let pressed = value(&cx, range);
    input
        .route(&mut cx, at(PointerPhase::Move, start + 50.0, true))
        .unwrap();
    let moved = value(&cx, range) - pressed;
    let expected = 50.0 * per_pixel * 0.1;
    assert!((moved - expected).abs() <= 0.01, "{moved} vs {expected}");
    input
        .route(&mut cx, at(PointerPhase::Up, start + 50.0, true))
        .unwrap();
}

/// A view binds the default value and turns the value thumb and the bounds
/// on; the range assembles its button as the view builds it.
#[test]
fn a_view_binds_the_default_value_and_assembles_the_button() {
    use crate::view::{signal, slider};
    let mut cx = AppContext::new();
    let root = cx
        .create_component(document(), crate::Stack::column(0.0))
        .unwrap();
    let default = signal(Some(40.0));
    cx.mount_view(root.stable_id(), move || {
        slider(0.0, 100.0, 1.0)
            .value_thumb(true)
            .show_bounds(true)
            .default_value(default)
    })
    .unwrap();
    let range = Entity::<RangeField>::from_stable_id(
        cx.world().node(root.stable_id()).unwrap().children[0],
    );
    let (thumb, bounds, wanted) = cx
        .read(range, |range| {
            (range.value_thumb, range.show_bounds, range.default_value)
        })
        .unwrap();
    let parts = cx
        .assembled_slot::<crate::view_components::RangeParts>(range.stable_id())
        .copied();
    assert!(thumb && bounds);
    assert_eq!(wanted, Some(40.0));
    let parts = parts.expect("assembled by the view");
    assert!(
        !cx.read(Entity::<IconButton>::from_stable_id(parts.reset), |b| b
            .disabled)
            .unwrap()
    );
    default.set(Some(0.0));
    cx.flush_reactive().unwrap();
    assert_eq!(
        cx.read(range, |range| range.default_value).unwrap(),
        Some(0.0)
    );
    assert!(
        cx.read(Entity::<IconButton>::from_stable_id(parts.reset), |b| b
            .disabled)
            .unwrap()
    );
}

#[test]
fn clicking_or_activating_a_value_thumb_in_a_scrolled_list_preserves_scroll_offset() {
    use crate::{LengthSpec, NodeStyle, ScrollAxes, ScrollOffset, ScrollView};

    let mut cx = AppContext::new();
    let mut viewport = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut viewport.layout);
        layout.width = Some(LengthSpec::Px(320.0));
        layout.height = Some(LengthSpec::Px(120.0));
    }
    let scroll = cx
        .create_component(
            document(),
            ScrollView::new(ScrollAxes::Vertical).style(viewport),
        )
        .unwrap();
    let mut ranges = Vec::new();
    for _ in 0..10 {
        let range = cx
            .create_component(
                document(),
                RangeField::new(30.0, 0.0, 100.0, 1.0)
                    .label("音量")
                    .value_thumb(true),
            )
            .unwrap();
        cx.append_child(scroll, range).unwrap();
        cx.assemble_range_field(range).unwrap();
        ranges.push(range);
    }
    let relayout_list = |cx: &mut AppContext| {
        let work = cx.take_system_work_for(document());
        cx.resolve_styles(&work.style).unwrap();
        cx.layout_document(document(), LayoutViewport::new(320.0, 120.0))
            .unwrap();
        cx.rebuild_hit_test(document());
    };
    relayout_list(&mut cx);

    assert!(
        cx.scroll_to(scroll, ScrollOffset { x: 0.0, y: 160.0 })
            .unwrap()
    );
    cx.rebuild_hit_test(document());
    assert_eq!(
        cx.world().scroll_offset(scroll.stable_id()).unwrap().y,
        160.0
    );

    let visible = *ranges
        .iter()
        .find(|&&range| {
            let bounds = cx.world().layout_box(range.stable_id()).unwrap();
            bounds.y >= 160.0 && bounds.y + bounds.height <= 280.0
        })
        .expect("a slider fully inside the scrolled viewport");
    let (_, thumb) = geometry(&cx, visible);
    let capsule = thumb.unwrap().capsule;
    let x = capsule.x + capsule.width / 2.0;
    let screen_y = capsule.y + capsule.height / 2.0 - 160.0;

    let mut input = HeadlessInput::bind(&mut cx, document());
    let pointer =
        |phase, px: f32, py: f32| InputPayload::Pointer(PointerInput::mouse(phase, px, py));
    let key = |name: &'static str| {
        InputPayload::Key(KeyInput {
            physical: PhysicalKey(name.into()),
            logical: LogicalKey(name.into()),
            state: KeyState::Pressed,
            repeat: false,
            modifiers: InputModifiers::empty(),
        })
    };

    // First click on the value thumb: starts editing without scrolling back to top.
    input
        .route(&mut cx, pointer(PointerPhase::Down, x, screen_y))
        .unwrap();
    input
        .route(&mut cx, pointer(PointerPhase::Up, x, screen_y))
        .unwrap();
    let parts = cx
        .assembled_slot::<crate::view_components::RangeParts>(visible.stable_id())
        .copied()
        .unwrap();
    assert!(cx.read(visible, |range| range.editing).unwrap());
    assert_eq!(cx.world().focused(document()), Some(parts.input));
    assert_eq!(
        cx.world().scroll_offset(scroll.stable_id()).unwrap().y,
        160.0
    );

    // Lay out while editing, press Escape, lay out again (collapses `parts.input`
    // back to zero), then click the thumb a second time.
    relayout_list(&mut cx);
    input.route(&mut cx, key("Escape")).unwrap();
    assert!(!cx.read(visible, |range| range.editing).unwrap());
    relayout_list(&mut cx);
    input
        .route(&mut cx, pointer(PointerPhase::Down, x, screen_y))
        .unwrap();
    input
        .route(&mut cx, pointer(PointerPhase::Up, x, screen_y))
        .unwrap();
    assert!(cx.read(visible, |range| range.editing).unwrap());
    assert_eq!(cx.world().focused(document()), Some(parts.input));
    assert_eq!(
        cx.world().scroll_offset(scroll.stable_id()).unwrap().y,
        160.0
    );

    // Keyboard activation (Enter) while the focused slider is already in view keeps 160.0.
    input.route(&mut cx, key("Escape")).unwrap();
    relayout_list(&mut cx);
    assert_eq!(cx.world().focused(document()), Some(visible.stable_id()));
    input.route(&mut cx, key("Enter")).unwrap();
    assert!(cx.read(visible, |range| range.editing).unwrap());
    assert_eq!(
        cx.world().scroll_offset(scroll.stable_id()).unwrap().y,
        160.0
    );

    // Keyboard activation (Enter) while the focused slider is offscreen below reveals
    // the full height of the slider rather than leaving it offscreen or jumping to 0.
    input.route(&mut cx, key("Escape")).unwrap();
    relayout_list(&mut cx);
    cx.scroll_to(scroll, ScrollOffset { x: 0.0, y: 0.0 })
        .unwrap();
    cx.rebuild_hit_test(document());
    assert_eq!(cx.world().scroll_offset(scroll.stable_id()).unwrap().y, 0.0);
    input.route(&mut cx, key("Enter")).unwrap();
    assert!(cx.read(visible, |range| range.editing).unwrap());
    let offset = cx.world().scroll_offset(scroll.stable_id()).unwrap().y;
    let visible_box = cx.world().layout_box(visible.stable_id()).unwrap();
    assert!(
        offset > 0.0,
        "expected scroll to reveal offscreen slider, got {offset}"
    );
    assert!(
        visible_box.y + 0.01 >= offset
            && visible_box.y + visible_box.height <= offset + 120.0 + 0.01,
        "expected full slider {visible_box:?} inside viewport [0, 120] at offset {offset}"
    );

    // Clicking the visible thumb capsule of a slider partially clipped at the bottom edge
    // focuses in place without scrolling.
    input.route(&mut cx, key("Escape")).unwrap();
    relayout_list(&mut cx);
    let partial = ranges[8];
    let partial_box = cx.world().layout_box(partial.stable_id()).unwrap();
    let (_, partial_thumb) = geometry(&cx, partial);
    let partial_capsule = partial_thumb.unwrap().capsule;
    let partial_scroll_y =
        (partial_capsule.y + partial_capsule.height / 2.0 + 2.0 - 120.0).max(0.0);
    assert!(
        partial_box.y + partial_box.height > partial_scroll_y + 120.0,
        "expected bottom of {partial_box:?} to be clipped below {}",
        partial_scroll_y + 120.0
    );
    cx.scroll_to(
        scroll,
        ScrollOffset {
            x: 0.0,
            y: partial_scroll_y,
        },
    )
    .unwrap();
    cx.rebuild_hit_test(document());
    let px = partial_capsule.x + partial_capsule.width / 2.0;
    let py = partial_capsule.y + partial_capsule.height / 2.0 - partial_scroll_y;
    input
        .route(&mut cx, pointer(PointerPhase::Down, px, py))
        .unwrap();
    input
        .route(&mut cx, pointer(PointerPhase::Up, px, py))
        .unwrap();
    assert!(cx.read(partial, |range| range.editing).unwrap());
    assert_eq!(
        cx.world().scroll_offset(scroll.stable_id()).unwrap().y,
        partial_scroll_y
    );
}
