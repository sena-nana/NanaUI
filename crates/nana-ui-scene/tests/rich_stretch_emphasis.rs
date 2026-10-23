//! 字形伸缩与着重号沿展示、编辑、场景共用的保留排版链回归。

use nana_text::font::{FaceDescriptor, FontSystem, font_blob};
use nana_text::{NativeTextEngine, SharedTextEngine};
use nana_ui_core::{RichSpanStyle, RichText};
use nana_ui_runtime::{
    DocumentId, LayoutViewport, NanaTextEngineShaper, RichTextEditor, RichTextView, StableNodeId,
};
use nana_ui_scene::{RuntimeDocument, ScenePrimitiveKind};
use std::sync::{Arc, Mutex};

fn engine() -> SharedTextEngine {
    let mut fonts = FontSystem::hermetic();
    fonts
        .register_bytes(
            font_blob(include_bytes!("../../nana-text/fonts/nana-test-vf.ttf")),
            &FaceDescriptor::default(),
        )
        .unwrap();
    Arc::new(Mutex::new(NativeTextEngine::new(fonts)))
}

fn document(percent: f32, emphasis: bool) -> (RuntimeDocument, StableNodeId, StableNodeId) {
    let mut runtime = RuntimeDocument::new(DocumentId::new(1).unwrap());
    let value = RichText::new("AAAA").with_span(
        0..4,
        RichSpanStyle::new()
            .family("NanaTestVF")
            .stretch(percent)
            .emphasis(emphasis),
    );
    let cx = runtime.context_mut();
    let view = cx
        .create_component(
            DocumentId::new(1).unwrap(),
            RichTextView::new(value.clone()).font_size(24.0),
        )
        .unwrap();
    let editor = cx
        .create_component(
            DocumentId::new(1).unwrap(),
            RichTextEditor::new(value).font_size(24.0),
        )
        .unwrap();
    let mut shaper = NanaTextEngineShaper::new(engine());
    for _ in 0..3 {
        runtime
            .flush(LayoutViewport::new(600.0, 200.0), &mut shaper)
            .unwrap();
    }
    (runtime, view.stable_id(), editor.stable_id())
}

#[test]
fn stretch_changes_real_advance_and_editor_geometry_matches_display() {
    let (narrow, view, editor) = document(75.0, true);
    let (wide, wide_view, _) = document(150.0, false);
    let layout = |runtime: &RuntimeDocument, id| {
        runtime.context().world().text_layout(id).unwrap().1.clone()
    };
    let a = layout(&narrow, view);
    let b = layout(&wide, wide_view);
    let e = layout(&narrow, editor);
    assert!(
        b.bounds.width > a.bounds.width,
        "wdth changes font advance: {:?} / {:?}",
        a.bounds,
        b.bounds
    );
    assert_eq!(a.bounds.width, e.bounds.width);
    assert_eq!(a.lines[0].metrics.width_px, e.lines[0].metrics.width_px);
    let last = nana_text::CaretPosition::new(4, nana_text::Affinity::Upstream, 0);
    assert_eq!(
        a.caret_geometry(last).unwrap().x_px,
        e.caret_geometry(last).unwrap().x_px
    );
}

#[test]
fn emphasis_reaches_both_scene_text_primitives_and_paint_revision() {
    let (runtime, view, editor) = document(100.0, true);
    let (plain, plain_view, _) = document(100.0, false);
    let marked_height = runtime
        .context()
        .world()
        .text_layout(view)
        .unwrap()
        .1
        .bounds
        .height;
    let plain_height = plain
        .context()
        .world()
        .text_layout(plain_view)
        .unwrap()
        .1
        .bounds
        .height;
    assert!(marked_height > plain_height, "dots reserve bottom space");
    for id in [view, editor] {
        let rich = runtime
            .scene()
            .primitives()
            .find_map(|primitive| match &primitive.kind {
                ScenePrimitiveKind::Text { rich, .. } if primitive.node == id => rich.clone(),
                _ => None,
            })
            .expect("emphasis creates paint effects for display and editor");
        assert!(rich.effects_at(0).emphasis);
        assert!(rich.has_decoration());
        assert!(!rich.base.emphasis);
        let mut style = RichSpanStyle::new().emphasis(true);
        style = style.overlay(&RichSpanStyle::new().emphasis(false));
        assert_eq!(style.paint.emphasis, Some(false));
    }
}
