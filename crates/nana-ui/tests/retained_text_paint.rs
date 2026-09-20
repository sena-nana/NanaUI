//! The painter draws the paragraph Runtime *measured*, not one it lays out
//! again (#99).
//!
//! The handle only carries that promise when the box a node was measured into
//! is the box it paints into, and those two numbers are computed by different
//! code in different crates: `UiWorld::text_shape_constraints_for` subtracts
//! padding, border and a leading visual from the layout box, while the scene's
//! text primitive subtracts the same plus a trailing visual. Nothing makes
//! them agree by construction, so this asserts they do on the plain path —
//! otherwise `TextPipeline::prepare` would silently fall back for every node
//! and the retained layout would be dead weight.

use nana_ui::{NanaTextShaper, runtime::*};
use nana_ui_scene::{ScenePrimitiveKind, UiScene};

fn text_primitive(
    scene: &UiScene,
    node: StableNodeId,
) -> (nana_ui_scene::SceneRect, Option<RetainedTextLayout>) {
    let primitive = scene
        .primitives()
        .find(|primitive| {
            primitive.node == node && matches!(primitive.kind, ScenePrimitiveKind::Text { .. })
        })
        .expect("the node paints text");
    let ScenePrimitiveKind::Text { ref layout, .. } = primitive.kind else {
        unreachable!("filtered above")
    };
    (primitive.bounds, layout.clone())
}

fn settle(cx: &mut AppContext, doc: DocumentId, ids: &[StableNodeId]) {
    cx.resolve_styles(ids).unwrap();
    cx.shape_text(ids, &mut NanaTextShaper::default()).unwrap();
    cx.layout_document(doc, LayoutViewport::new(600.0, 400.0))
        .unwrap();
    // The constraint-aware pass: a node measured before layout knew its box is
    // re-resolved against the box it got, and that is the layout the scene
    // carries.
    if cx
        .shape_text_for_layout(doc, &mut NanaTextShaper::default())
        .unwrap()
    {
        cx.layout_document(doc, LayoutViewport::new(600.0, 400.0))
            .unwrap();
    }
}

#[test]
fn a_plain_text_node_is_measured_into_the_box_its_primitive_paints() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let root = cx.create_component(doc, Stack::column(8.0)).unwrap();
    let label = cx
        .create_component(doc, Text::new("A label wide enough to wrap somewhere"))
        .unwrap();
    cx.append_child(root, label).unwrap();
    let (root, label) = (root.stable_id(), label.stable_id());
    settle(&mut cx, doc, &[root, label]);

    let mut scene = UiScene::new();
    scene.apply_delta(cx.world().extract_document(doc), []);
    let (bounds, retained) = text_primitive(&scene, label);

    let retained = retained.expect("a plain text node retains the layout it was measured with");
    assert_eq!(
        retained.layout.constraints.max_width_px,
        Some(bounds.width),
        "the alignment box and the paint box are the same number, or the \
         painter falls back to laying the paragraph out again"
    );
}

/// A box too short for its text is an overflow, not a shorter paragraph.
///
/// `nana-text` *drops* the lines that do not fit `max_height_px`, so handing
/// it a declared height whenever one exists would delete text the box merely
/// clips — and since the painter now draws this very layout, deleted lines
/// would not be drawn at all. The height only goes to the engine when
/// truncation was asked for.
#[test]
fn a_short_box_clips_its_wrapped_text_instead_of_losing_lines() {
    fn lines(height: Option<f32>, ellipsis: bool) -> usize {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let root = cx.create_component(doc, Stack::column(0.0)).unwrap();
        let mut label = Text::new("A deliberately long label that has to wrap onto several lines");
        {
            let style = std::sync::Arc::make_mut(&mut label.style.layout);
            style.width = Some(nana_ui_core::LengthSpec::Px(180.0));
            style.height = height.map(nana_ui_core::LengthSpec::Px);
            style.text_overflow_ellipsis = ellipsis;
        }
        let label = cx.create_component(doc, label).unwrap();
        cx.append_child(root, label).unwrap();
        let (root, label) = (root.stable_id(), label.stable_id());
        settle(&mut cx, doc, &[root, label]);

        let mut scene = UiScene::new();
        scene.apply_delta(cx.world().extract_document(doc), []);
        let (_, retained) = text_primitive(&scene, label);
        retained
            .expect("a plain text node retains its layout")
            .layout
            .lines
            .len()
    }

    let natural = lines(None, false);
    assert!(natural >= 2, "the fixture has to wrap: {natural} lines");
    assert_eq!(
        lines(Some(24.0), false),
        natural,
        "a box one line tall does not delete the lines it cannot show"
    );
    assert!(
        lines(Some(24.0), true) < natural,
        "asking for an ellipsis is what makes the height a truncation budget"
    );
}

/// #59: a node that asks for a vertical writing mode is laid out horizontally,
/// and the frame says so.
///
/// The fallback is the right answer — the engine has no glyph orientation, and
/// reporting horizontal metrics as vertical ones would be worse. What was
/// missing is that it was invisible: the flag sat on the layout and nothing
/// read it, so a document could ask for vertical text and get horizontal with
/// nobody the wiser. Now it reaches the pass counters.
#[test]
fn a_vertical_writing_mode_falls_back_horizontally_and_the_frame_reports_it() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let root = cx.create_component(doc, Stack::column(0.0)).unwrap();
    let mut label = Text::new("縦書きの段落");
    {
        let style = std::sync::Arc::make_mut(&mut label.style.layout);
        style.width = Some(nana_ui_core::LengthSpec::Px(200.0));
        style.writing_mode = Some(nana_ui_core::WritingModeSpec::VerticalRl);
    }
    let label = cx.create_component(doc, label).unwrap();
    cx.append_child(root, label).unwrap();
    let (root, label) = (root.stable_id(), label.stable_id());
    settle(&mut cx, doc, &[root, label]);

    let counters = cx.world().last_text_work_counters();
    assert!(
        counters.vertical_writing_fallbacks > 0,
        "the frame has to be able to say the vertical request was not honoured: {counters:?}"
    );

    let mut scene = UiScene::new();
    scene.apply_delta(cx.world().extract_document(doc), []);
    let (_, retained) = text_primitive(&scene, label);
    assert!(
        retained
            .expect("a plain text node retains its layout")
            .layout
            .unsupported_writing_mode,
        "and the layout itself carries the same answer"
    );
}
