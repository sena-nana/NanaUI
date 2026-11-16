use super::*;
use nana_ui_runtime::{
    DocumentId, HighlightRequest, LayoutViewport, MeasureTextShaper, NativeMarkdown,
};

fn flush(runtime: &mut crate::RuntimeDocument) {
    runtime
        .flush(LayoutViewport::new(430.0, 900.0), &mut MeasureTextShaper)
        .unwrap();
}

#[test]
fn markdown_preserves_large_plain_text_projection_and_removes_it() {
    // Long documents retain unique identities for every drawing command.
    let source = (0..300)
        .map(|i| format!("Paragraph {i}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let view = NativeMarkdown::from_source(&source);
    assert_eq!(view.blocks().len(), 300);
    let expected = view.plain_text();
    let document = DocumentId::new(1).unwrap();
    let mut runtime = crate::RuntimeDocument::new(document);
    let markdown = runtime
        .context_mut()
        .create_component(document, view)
        .unwrap();
    flush(&mut runtime);
    let content = runtime
        .scene()
        .primitives()
        .filter_map(|p| match &p.kind {
            ScenePrimitiveKind::Text { content, .. } if p.node == markdown.stable_id() => {
                Some(content.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(content.len(), 300);
    assert_eq!(content.join("\n\n"), expected);
    let ids = runtime
        .scene()
        .primitives()
        .filter(|p| p.node == markdown.stable_id())
        .map(|p| p.id)
        .collect::<Vec<_>>();
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        ids.len()
    );
    runtime.context_mut().remove_view(markdown).unwrap();
    flush(&mut runtime);
    assert!(
        runtime
            .scene()
            .primitives()
            .all(|p| p.node != markdown.stable_id())
    );
}

#[test]
fn markdown_fences_keep_host_presenter_identity_without_duplicate_scene_text() {
    let source = "$$\\frac{1}{\\sqrt{x^2+1}}$$\n\n```mermaid\nflowchart TD\nA-->B\n```";
    let view = NativeMarkdown::from_source(source);
    let expected = view.plain_text();
    let document = DocumentId::new(1).unwrap();
    let mut runtime = crate::RuntimeDocument::new(document);
    let markdown = runtime
        .context_mut()
        .create_component(document, view)
        .unwrap();
    runtime.context_mut().assemble_markdown(markdown).unwrap();
    let children = runtime
        .context()
        .world()
        .node(markdown.stable_id())
        .unwrap()
        .children;
    assert_eq!(children.len(), 2);
    assert_eq!(
        runtime.context().world().highlight_request(children[0]),
        Some(&HighlightRequest::new(
            NativeMarkdown::MATH_PRESENTER,
            "\\frac{1}{\\sqrt{x^2+1}}"
        ))
    );
    assert_eq!(
        runtime.context().world().highlight_request(children[1]),
        Some(&HighlightRequest::new(
            NativeMarkdown::MERMAID_PRESENTER,
            "flowchart TD\nA-->B"
        ))
    );
    runtime.context_mut().assemble_markdown(markdown).unwrap();
    assert_eq!(
        runtime
            .context()
            .world()
            .node(markdown.stable_id())
            .unwrap()
            .children,
        children
    );
    flush(&mut runtime);
    for child in children {
        assert!(runtime.scene().primitives().all(|p| p.node != child));
    }
    let diagram_slot = |p: &ScenePrimitive| {
        p.id.slot >> 32 == u64::from(crate::scene::custom_paint::WIDGET_VECTOR_PAINT)
    };
    let text = runtime
        .scene()
        .primitives()
        .filter(|p| !diagram_slot(p))
        .filter_map(|p| match &p.kind {
            ScenePrimitiveKind::Text { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        text.is_empty(),
        "typeset blocks do not duplicate their fallback text"
    );
    // The diagram draws its own labels, nothing else.
    let mut labels = runtime
        .scene()
        .primitives()
        .filter(|p| diagram_slot(p))
        .filter_map(|p| match &p.kind {
            ScenePrimitiveKind::Text { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    labels.sort_unstable();
    assert_eq!(labels, ["A", "B"]);
    assert_eq!(
        runtime.context().world().text(markdown.stable_id()),
        Some(expected.as_str())
    );
    assert_eq!(runtime.scene().primitives().filter(|p| matches!(&p.kind, ScenePrimitiveKind::Quad { surface, .. } if surface.content_image.is_some())).count(), 1);
}

#[test]
fn markdown_diagrams_draw_as_vector_geometry_reused_until_dropped() {
    let document = DocumentId::new(14).unwrap();
    let mut runtime = crate::RuntimeDocument::new(document);
    let markdown = runtime
        .context_mut()
        .create_component(
            document,
            NativeMarkdown::from_source(
                "Intro\n\n```mermaid\nflowchart LR\nA[Start] -->|go| B{Check} --> C((End))\n```\n\n```mermaid\ngraph TD\nX-->Y\n```",
            ),
        )
        .unwrap();
    flush(&mut runtime);
    let id = markdown.stable_id();
    let in_namespace = |p: &&ScenePrimitive| {
        p.node == id
            && p.id.slot >> 32 == u64::from(crate::scene::custom_paint::WIDGET_VECTOR_PAINT)
    };
    let meshes = |runtime: &crate::RuntimeDocument| -> Vec<Arc<PathMesh>> {
        runtime
            .scene()
            .primitives()
            .filter(in_namespace)
            .filter_map(|p| match &p.kind {
                ScenePrimitiveKind::Path { mesh, .. } => Some(Arc::clone(mesh)),
                _ => None,
            })
            .collect()
    };
    let first = meshes(&runtime);
    assert!(!first.is_empty(), "edges and non-rect shapes are meshes");
    assert!(
        runtime
            .scene()
            .primitives()
            .filter(in_namespace)
            .any(|p| matches!(p.kind, ScenePrimitiveKind::Quad { .. }))
    );
    // Two diagrams, two slot blocks.
    let blocks: std::collections::BTreeSet<u64> = runtime
        .scene()
        .primitives()
        .filter(in_namespace)
        .map(|p| (p.id.slot & 0xFFFF_FFFF) / crate::scene::custom_paint::WIDGET_VECTOR_BLOCK as u64)
        .collect();
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        runtime
            .scene()
            .widget_vector_paint
            .get(&id)
            .map(|b| b.len()),
        Some(2)
    );
    // Selecting text rebuilds the node without re-triangulating anything.
    runtime
        .context_mut()
        .update_component(markdown, |view, _| {
            *view = NativeMarkdown::from_source(
                "Intro\n\n```mermaid\nflowchart LR\nA[Start] -->|go| B{Check} --> C((End))\n```\n\n```mermaid\ngraph TD\nX-->Y\n```",
            )
        })
        .unwrap();
    flush(&mut runtime);
    let again = meshes(&runtime);
    assert_eq!(first.len(), again.len());
    assert!(first.iter().zip(&again).all(|(a, b)| Arc::ptr_eq(a, b)));
    runtime
        .context_mut()
        .update_component(markdown, |view, _| {
            *view = NativeMarkdown::from_source("plain")
        })
        .unwrap();
    flush(&mut runtime);
    assert!(meshes(&runtime).is_empty());
    assert!(runtime.scene().widget_vector_paint.is_empty());
}

#[test]
fn markdown_inline_decorations_share_scene_strokes_and_clear_on_update() {
    let document = DocumentId::new(13).unwrap();
    let mut runtime = crate::RuntimeDocument::new(document);
    let markdown = runtime
        .context_mut()
        .create_component(
            document,
            NativeMarkdown::from_source("[link](https://example.test) ~~strike~~ `code`"),
        )
        .unwrap();
    flush(&mut runtime);
    let primitives = runtime
        .scene()
        .primitives()
        .filter(|p| p.node == markdown.stable_id())
        .collect::<Vec<_>>();
    let strokes = primitives
        .iter()
        .filter(|p| matches!(p.kind, ScenePrimitiveKind::Stroke { .. }))
        .collect::<Vec<_>>();
    assert_eq!(strokes.len(), 2);
    assert!(
        strokes
            .iter()
            .all(|p| p.bounds.width > 0.0 && p.bounds.height > 0.0)
    );
    let ids = primitives
        .iter()
        .map(|p| p.id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        ids.len(),
        primitives.len(),
        "text, code background and decoration slots must remain distinct"
    );
    runtime
        .context_mut()
        .update_component(markdown, |view, _| {
            *view = NativeMarkdown::from_source("plain")
        })
        .unwrap();
    flush(&mut runtime);
    assert!(
        !runtime
            .scene()
            .primitives()
            .any(|p| p.node == markdown.stable_id()
                && matches!(p.kind, ScenePrimitiveKind::Stroke { .. }))
    );
}

struct SelectableRichTextLineShaper;

impl nana_ui_runtime::TextShaper for SelectableRichTextLineShaper {
    fn shape(
        &mut self,
        _id: nana_ui_runtime::StableNodeId,
        text: &nana_ui_runtime::TextContent,
        _style: &nana_ui_runtime::ComputedStyle,
        constraints: nana_ui_runtime::TextShapeConstraints,
    ) -> nana_ui_runtime::TextMetrics {
        let intrinsic = text.value.len() as f32 * 8.0;
        nana_ui_runtime::TextMetrics {
            width: constraints.max_width.unwrap_or(intrinsic).min(intrinsic),
            height: 18.0,
            ascent: None,
        }
    }
}

/// The span roles of a selectable rich text reach its one text primitive
/// as byte ranges in the installed palette's colors.
#[test]
fn selectable_rich_text_paints_span_roles_on_its_text_primitive() {
    use nana_ui_runtime::{RichSpan, SelectableRichText, SemanticColorRole};
    let document = DocumentId::new(1).unwrap();
    let mut runtime = crate::RuntimeDocument::new(document);
    let text = runtime
        .context_mut()
        .create_component(
            document,
            SelectableRichText::new([
                RichSpan::plain("pub").color(SemanticColorRole::Keyword),
                RichSpan::plain(" fn"),
                RichSpan::plain(" // 注释").color(SemanticColorRole::Muted),
            ]),
        )
        .unwrap();
    runtime
        .flush(
            LayoutViewport::new(320.0, 120.0),
            &mut SelectableRichTextLineShaper,
        )
        .unwrap();
    let palette = runtime.context().world().style_model();
    let color = |role| nana_ui_core::PaintColor::Srgb {
        rgba: palette.color(role).as_rgba_array(),
    };
    let primitive = runtime
        .scene()
        .primitive(PrimitiveId {
            node: text.stable_id(),
            slot: 2,
        })
        .expect("the rich text paints one text primitive");
    let ScenePrimitiveKind::Text { content, spans, .. } = &primitive.kind else {
        panic!("expected a text primitive, got {:?}", primitive.kind);
    };
    assert_eq!(content.as_ref(), "pub fn // 注释");
    assert_eq!(
        spans,
        &vec![
            SceneTextSpan {
                start: 0,
                end: 3,
                color: color(SemanticColorRole::Keyword),
            },
            SceneTextSpan {
                start: "pub fn".len(),
                end: content.len(),
                color: color(SemanticColorRole::Muted),
            },
        ]
    );
}

#[test]
fn selectable_rich_text_selection_uses_selection_background_paint_color() {
    use nana_ui_runtime::{RichSpan, SelectableRichText};
    let document = DocumentId::new(1).unwrap();
    let mut runtime = crate::RuntimeDocument::new(document);
    let wide_selection = nana_ui_core::PaintColor::Oklch {
        l: 0.72,
        c: 0.3,
        h: Some(150.0),
        alpha: 0.5,
    };
    let mut node_style = nana_ui_runtime::NodeStyle::default();
    let layout = std::sync::Arc::make_mut(&mut node_style.layout);
    layout.text_ext.selection_background = Some(wide_selection.to_srgb());
    layout.paint_colors.selection_background = Some(wide_selection);
    // Distinct `selection_color` (foreground) must NOT bleed into selection background quads.
    layout.text_ext.selection_color = Some([1.0, 0.0, 0.0, 1.0]);

    let view = SelectableRichText::new([RichSpan::plain("hello world")]).style(node_style);
    let bounds = nana_ui_runtime::LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 20.0,
    };
    view.pointer_down(0.0, 2.0, bounds);
    view.pointer_move(100.0, 2.0, bounds);
    let _ = view.pointer_up(100.0, 2.0, bounds);

    let text = runtime
        .context_mut()
        .create_component(document, view)
        .unwrap();
    runtime
        .flush(
            LayoutViewport::new(320.0, 120.0),
            &mut SelectableRichTextLineShaper,
        )
        .unwrap();

    let selection = runtime
        .scene()
        .primitive(PrimitiveId {
            node: text.stable_id(),
            slot: 1,
        })
        .expect("selected rich text emits a selection QuadBatch");
    let ScenePrimitiveKind::QuadBatch { background, .. } = &selection.kind else {
        panic!("expected selection QuadBatch, got {:?}", selection.kind);
    };
    assert_eq!(*background, Some(wide_selection));
    assert!(runtime.scene().has_extended_color());
}
