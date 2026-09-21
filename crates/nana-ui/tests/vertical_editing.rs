//! #59: editing vertical text. A `vertical-rl` TextArea lays its value out in
//! columns, draws its caret across the column, walks the column with Up/Down
//! and crosses columns with Left/Right, and a click lands on the glyph under
//! it — end to end, through the real engine.

use nana_ui::{NanaTextShaper, runtime::*};
use std::sync::Arc;

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

    // Down walks the column.
    cx.move_focused_text_caret(doc, TextCaretIntent::Down, false, Some(&mut shaper))
        .unwrap();
    let state = cx.read(area, |view| view.state.clone()).unwrap();
    assert_eq!(
        state.selection.focus,
        "一".len(),
        "one glyph down the column"
    );

    // Left crosses to the next column, at the same depth.
    cx.move_focused_text_caret(doc, TextCaretIntent::Left, false, Some(&mut shaper))
        .unwrap();
    let state = cx.read(area, |view| view.state.clone()).unwrap();
    assert_eq!(
        state.selection.focus,
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
    let state = cx.read(area, |view| view.state.clone()).unwrap();
    assert_eq!(state.selection.focus, "一二三四五六".len());
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
