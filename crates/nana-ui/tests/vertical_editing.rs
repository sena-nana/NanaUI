//! #59: editing vertical text. A `vertical-rl` TextArea lays its value out in
//! columns, draws its caret across the column, walks the column with Up/Down
//! and crosses columns with Left/Right, and a click lands on the glyph under
//! it — end to end, through the real engine.

use nana_ui::{NanaTextShaper, RuntimeInputAdapter, runtime::*};
use nana_ui_platform::{InputEvent, InputModifiers};
use std::sync::Arc;
use std::time::Duration;

/// A physical key press, as a platform reports it.
fn press(
    cx: &mut AppContext,
    doc: DocumentId,
    key: &str,
    modifiers: InputModifiers,
    shaper: &mut NanaTextShaper,
) {
    let event = InputEvent::Keyboard {
        pressed: true,
        key: key.into(),
        code: key.into(),
        text: None,
        repeat: false,
        modifiers,
    };
    RuntimeInputAdapter::default()
        .dispatch_with_shaper(cx, doc, &event, Duration::ZERO, Some(shaper))
        .unwrap();
}

fn focus_of(cx: &AppContext, area: Entity<TextArea>) -> usize {
    cx.read(area, |view| view.state.selection.focus).unwrap()
}

fn settle(cx: &mut AppContext, doc: DocumentId, ids: &[StableNodeId]) {
    let mut shaper = NanaTextShaper::default();
    cx.resolve_styles(ids).unwrap();
    cx.shape_text(ids, &mut shaper).unwrap();
    cx.layout_document(doc, LayoutViewport::new(600.0, 400.0))
        .unwrap();
    if cx.shape_text_for_layout(doc, &mut shaper).unwrap() {
        cx.layout_document(doc, LayoutViewport::new(600.0, 400.0))
            .unwrap();
    }
    cx.shape_text(ids, &mut shaper).unwrap();
    cx.layout_document(doc, LayoutViewport::new(600.0, 400.0))
        .unwrap();
    cx.rebuild_hit_test(doc);
}

fn caret(cx: &AppContext, node: StableNodeId) -> LayoutBox {
    let Some(ComponentGeometry::TextInput {
        caret: Some(caret), ..
    }) = cx.world().component_geometry(node)
    else {
        panic!("a focused editor draws a caret")
    };
    caret
}

#[test]
fn a_vertical_text_area_edits_in_columns() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let mut view = TextArea::new("一二三四五六七八\n九十");
    {
        let style = Arc::make_mut(&mut view.style.layout);
        style.writing_mode = Some(nana_ui_core::WritingModeSpec::VerticalRl);
        style.font_size = Some(16.0);
        style.width = Some(nana_ui_core::LengthSpec::Px(200.0));
        style.height = Some(nana_ui_core::LengthSpec::Px(64.0));
        style.padding_top = Some(nana_ui_core::LengthSpec::Px(0.0));
        style.padding_bottom = Some(nana_ui_core::LengthSpec::Px(0.0));
    }
    let area = cx.create_component(doc, view).unwrap();
    let node = area.stable_id();
    settle(&mut cx, doc, &[node]);
    assert!(cx.focus_node(doc, node).unwrap());
    let mut shaper = NanaTextShaper::default();
    cx.select_focused_text_range(doc, 0, 0).unwrap();
    settle(&mut cx, doc, &[node]);

    let (content, _) = cx.world().text_input_pointer_context(node).unwrap();
    let at_start = caret(&cx, node);
    // A caret spans its column: the editor's line box, whatever line height
    // the TextArea asks for.
    let column = at_start.width;
    assert!(
        at_start.width > at_start.height,
        "a caret across a column is a horizontal bar: {at_start:?}"
    );
    assert!(
        (at_start.x + at_start.width - (content.x + content.width)).abs() < 0.5
            && (at_start.y - content.y).abs() < 0.5,
        "the first column is the rightmost, its caret at the top: {at_start:?} in {content:?}"
    );

    // The physical ↓ key walks the column.
    let plain = InputModifiers::default();
    press(&mut cx, doc, "ArrowDown", plain, &mut shaper);
    assert_eq!(focus_of(&cx, area), "一".len(), "one glyph down the column");

    // ← crosses to the next column, at the same depth.
    press(&mut cx, doc, "ArrowLeft", plain, &mut shaper);
    assert_eq!(
        focus_of(&cx, area),
        "一二三四五".len(),
        "五 heads the second column, so one down it is after 五"
    );
    settle(&mut cx, doc, &[node]);
    let in_second = caret(&cx, node);
    assert!(
        (in_second.x - (at_start.x - column)).abs() < 0.5
            && (in_second.y - (content.y + 16.0)).abs() < 0.5,
        "drawn one column left, one glyph down: {in_second:?}"
    );

    // A click just below the top of 七, in the second column, lands before it.
    let x = content.x + content.width - column * 1.5;
    let y = content.y + 33.0;
    cx.text_editor_pointer_press(
        doc,
        node,
        1,
        x,
        y,
        false,
        false,
        std::time::Duration::ZERO,
        &mut shaper,
    )
    .unwrap();
    cx.text_editor_pointer_release(1);
    assert_eq!(focus_of(&cx, area), "一二三四五六".len());

    // Modifiers follow the key into line space. Cmd+↓ runs to the end of
    // the line along the column, as Cmd+→ does in a horizontal editor — its
    // logical line, like there — not to the end of the document…
    let meta = InputModifiers {
        meta: true,
        ..InputModifiers::default()
    };
    press(&mut cx, doc, "ArrowDown", meta, &mut shaper);
    assert_eq!(focus_of(&cx, area), "一二三四五六七八".len());
    press(&mut cx, doc, "ArrowUp", meta, &mut shaper);
    assert_eq!(focus_of(&cx, area), 0, "and Cmd+↑ to its start");
    // …while Cmd+← in `vertical-rl` heads for the last column: the end of
    // the text, as Cmd+↓ is in a horizontal editor. Cmd+→ goes back to the
    // start.
    press(&mut cx, doc, "ArrowLeft", meta, &mut shaper);
    assert_eq!(focus_of(&cx, area), "一二三四五六七八\n九十".len());
    press(&mut cx, doc, "ArrowRight", meta, &mut shaper);
    assert_eq!(focus_of(&cx, area), 0);
}

/// #59: a wheel scrolls a vertical editor the way the page is turned. Its
/// scroll offset is in line space — `y` down the columns, `x` across them from
/// the first — and a `vertical-rl` editor's columns run leftwards, so the
/// wheel's horizontal delta runs the other way.
#[test]
fn a_wheel_scrolls_a_vertical_rl_editor_towards_its_later_columns() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    // Far more columns than the box is wide.
    let mut view = TextArea::new("一二三四五六七八九十".repeat(8));
    {
        let style = Arc::make_mut(&mut view.style.layout);
        style.writing_mode = Some(nana_ui_core::WritingModeSpec::VerticalRl);
        style.font_size = Some(16.0);
        style.width = Some(nana_ui_core::LengthSpec::Px(120.0));
        style.height = Some(nana_ui_core::LengthSpec::Px(64.0));
    }
    let area = cx.create_component(doc, view).unwrap();
    let node = area.stable_id();
    settle(&mut cx, doc, &[node]);
    let bounds = cx.world().layout_box(node).unwrap();
    let wheel = |delta_x: f32| InputEvent::Wheel {
        x: bounds.x + bounds.width / 2.0,
        y: bounds.y + bounds.height / 2.0,
        delta_x,
        delta_y: 0.0,
        line_delta: false,
        modifiers: InputModifiers::default(),
    };
    let mut adapter = RuntimeInputAdapter::default();
    // A platform's positive horizontal delta scrolls the page leftwards,
    // which in `vertical-rl` is going on through the text.
    adapter.dispatch(&mut cx, doc, &wheel(40.0)).unwrap();
    let scrolled = cx.world().scroll_offset(node).unwrap_or_default();
    assert!(
        scrolled.x > 0.0,
        "towards the later columns on the left: {scrolled:?}"
    );
    // And rightwards comes back to the first column.
    adapter.dispatch(&mut cx, doc, &wheel(-400.0)).unwrap();
    assert_eq!(cx.world().scroll_offset(node).unwrap_or_default().x, 0.0);
}

/// A single-line vertical field centres its one column across the box, the
/// way a horizontal field centres its line down it, and its caret follows.
#[test]
fn a_vertical_text_input_centres_its_column() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let mut view = TextInput::new("縦書き");
    {
        let style = Arc::make_mut(&mut view.style.layout);
        style.writing_mode = Some(nana_ui_core::WritingModeSpec::VerticalRl);
        style.width = Some(nana_ui_core::LengthSpec::Px(80.0));
        style.height = Some(nana_ui_core::LengthSpec::Px(120.0));
    }
    let input = cx.create_component(doc, view).unwrap();
    let node = input.stable_id();
    settle(&mut cx, doc, &[node]);
    assert!(cx.focus_node(doc, node).unwrap());
    cx.select_focused_text_range(doc, 0, 0).unwrap();
    settle(&mut cx, doc, &[node]);

    let (content, _) = cx.world().text_input_pointer_context(node).unwrap();
    let at = caret(&cx, node);
    let centre = at.x + at.width / 2.0;
    assert!(
        (centre - (content.x + content.width / 2.0)).abs() < 0.5,
        "the column sits in the middle of the field: {at:?} in {content:?}"
    );
    let Some(ComponentGeometry::TextInput { text, .. }) = cx.world().component_geometry(node)
    else {
        panic!("an editor");
    };
    assert!(
        (text.bounds.x + text.bounds.width - (at.x + at.width)).abs() < 0.5,
        "and the painter anchors the column where the caret is: {:?} vs {at:?}",
        text.bounds
    );
}
