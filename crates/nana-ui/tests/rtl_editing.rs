//! An RTL editor draws its value where its carets are.
//!
//! A single-line field's geometry has no line budget, so it lays its line out
//! from line-left and aligns nowhere; the painter lays the same value out in
//! the region it is given, with `start` alignment. If that region is wider
//! than the line, RTL `start` moves the glyphs to its right edge while every
//! caret stays at the left — the text and the caret land in different places.
//! The editor frame gives the painter a region exactly as long as the line and
//! puts it against the inline-start edge itself, in every writing mode.

use nana_ui::{NanaTextShaper, runtime::*};
use std::sync::Arc;
use std::time::Duration;

fn settle(cx: &mut AppContext, doc: DocumentId, ids: &[StableNodeId]) {
    let mut shaper = NanaTextShaper::default();
    cx.resolve_styles(ids).unwrap();
    cx.shape_text(ids, &mut shaper).unwrap();
    cx.layout_document(doc, LayoutViewport::new(600.0, 400.0))
        .unwrap();
    cx.shape_text(ids, &mut shaper).unwrap();
    cx.layout_document(doc, LayoutViewport::new(600.0, 400.0))
        .unwrap();
    cx.rebuild_hit_test(doc);
}

#[test]
fn an_rtl_text_input_sits_at_the_right_and_its_caret_is_on_its_text() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let mut view = TextInput::new("שלום");
    {
        let style = Arc::make_mut(&mut view.style.layout);
        style.dir = Some(DirSpec::Rtl);
        style.width = Some(LengthSpec::Px(200.0));
    }
    let input = cx.create_component(doc, view).unwrap();
    let node = input.stable_id();
    settle(&mut cx, doc, &[node]);
    assert!(cx.focus_node(doc, node).unwrap());
    cx.select_focused_text_range(doc, 0, 0).unwrap();
    settle(&mut cx, doc, &[node]);

    let (content, _) = cx.world().text_input_pointer_context(node).unwrap();
    let Some(ComponentGeometry::TextInput {
        text,
        caret: Some(caret),
        ..
    }) = cx.world().component_geometry(node)
    else {
        panic!("a focused editor")
    };
    let right = content.x + content.width;
    assert!(
        (text.bounds.x + text.bounds.width - right).abs() < 0.5,
        "the line ends flush with the right edge: {:?} in {content:?}",
        text.bounds
    );
    assert!(
        text.bounds.width < content.width - 50.0,
        "exactly as long as the line, no slack for the painter to align in: {:?}",
        text.bounds
    );
    // The caret before the first letter of a Hebrew word is at its right end.
    assert!(
        (caret.x - right).abs() < 1.5,
        "the caret sits on the drawn text: {caret:?} vs {:?}",
        text.bounds
    );

    // A click just inside the left end of the word hits its logical end.
    let mut shaper = NanaTextShaper::default();
    cx.text_editor_pointer_press(
        doc,
        node,
        1,
        text.bounds.x + 1.0,
        caret.y + caret.height / 2.0,
        false,
        false,
        Duration::ZERO,
        &mut shaper,
    )
    .unwrap();
    cx.text_editor_pointer_release(1);
    assert_eq!(
        cx.read(input, |view| view.state.selection.focus).unwrap(),
        "שלום".len()
    );
}

/// A wrapping RTL area's geometry and painter share the content width as
/// their line budget, so both right-align the lines themselves; the region is
/// exactly that wide.
#[test]
fn an_rtl_text_area_aligns_its_lines_to_the_right_in_geometry_and_paint() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let mut view = TextArea::new("שלום");
    {
        let style = Arc::make_mut(&mut view.style.layout);
        style.dir = Some(DirSpec::Rtl);
        style.width = Some(LengthSpec::Px(200.0));
        style.height = Some(LengthSpec::Px(80.0));
    }
    let area = cx.create_component(doc, view).unwrap();
    let node = area.stable_id();
    settle(&mut cx, doc, &[node]);
    assert!(cx.focus_node(doc, node).unwrap());
    cx.select_focused_text_range(doc, 0, 0).unwrap();
    settle(&mut cx, doc, &[node]);
    let (content, _) = cx.world().text_input_pointer_context(node).unwrap();
    let Some(ComponentGeometry::TextInput {
        text,
        caret: Some(caret),
        ..
    }) = cx.world().component_geometry(node)
    else {
        panic!("a focused editor")
    };
    assert!(
        (text.bounds.x - content.x).abs() < 0.5 && (text.bounds.width - content.width).abs() < 0.5,
        "the painter's line budget is the content width: {:?} in {content:?}",
        text.bounds
    );
    assert!(
        (caret.x - (content.x + content.width)).abs() < 1.5,
        "the first letter starts at the right edge: {caret:?} in {content:?}"
    );
}
