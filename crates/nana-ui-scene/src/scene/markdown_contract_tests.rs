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
    let text = runtime
        .scene()
        .primitives()
        .filter_map(|p| match &p.kind {
            ScenePrimitiveKind::Text { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        text.is_empty(),
        "typeset blocks do not duplicate their fallback text"
    );
    assert_eq!(
        runtime.context().world().text(markdown.stable_id()),
        Some(expected.as_str())
    );
    assert_eq!(runtime.scene().primitives().filter(|p| matches!(&p.kind, ScenePrimitiveKind::Quad { surface, .. } if surface.content_image.is_some())).count(), 2);
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
