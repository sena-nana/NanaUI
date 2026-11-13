//! Geometry-to-primitive projection; has no Scene index or Runtime access.
use super::*;
pub(super) fn build(context: &GeometryPaintContext<'_>, emit: &mut impl FnMut(ScenePrimitive)) {
    let node = context.node;
    let transform = context.transform;
    let clips = context.clips;
    let opacity = context.opacity;
    let node_order = context.node_order;
    let id = context.node.id;
    match context.node.component_geometry.as_deref() {
        #[cfg(feature = "rich-text")]
        Some(ComponentGeometry::NativeMarkdown {
            drawing,
            text,
            selection,
            selection_color,
        }) => {
            if !selection.is_empty() {
                let mut selection = visual_quad_batch(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    },
                    1,
                    selection.iter().copied().map(scene_rect),
                    VisualQuadStyle::solid(*selection_color),
                );
                if let ScenePrimitiveKind::QuadBatch { background, .. } = &mut selection.kind {
                    *background = (node.style.paint_colors.selection_background).or(*background);
                }
                emit(selection);
            }
            super::markdown_drawing::build(
                context,
                drawing,
                text.color.unwrap_or([0.8, 0.8, 0.8, 1.0]),
                emit,
            );
        }
        #[cfg(feature = "rich-text")]
        Some(ComponentGeometry::SelectableRichText {
            text,
            selection,
            selection_color,
        }) => {
            if !selection.is_empty() {
                let mut selection = visual_quad_batch(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    },
                    1,
                    selection.iter().copied().map(scene_rect),
                    VisualQuadStyle::solid(*selection_color),
                );
                if let ScenePrimitiveKind::QuadBatch { background, .. } = &mut selection.kind {
                    *background = (node.style.paint_colors.selection_background).or(*background);
                }
                emit(selection);
            }
            emit(component_text_primitive(
                id,
                2,
                text,
                TextHorizontalAlignment::Start,
                false,
                node,
                transform,
                clips.clone(),
                opacity,
                node_order,
            ));
        }
        _ => {}
    }
}

#[cfg(all(test, feature = "rich-text"))]
mod tests {
    use nana_ui_runtime::{
        ComputedStyle, DocumentId, LayoutViewport, RichSpan, SelectableRichText, SemanticColorRole,
        StableNodeId, TextContent, TextMetrics, TextShapeConstraints, TextShaper,
    };

    use crate::{PrimitiveId, RuntimeDocument, ScenePrimitiveKind, SceneTextSpan};

    struct LineShaper;

    impl TextShaper for LineShaper {
        fn shape(
            &mut self,
            _id: StableNodeId,
            text: &TextContent,
            _style: &ComputedStyle,
            constraints: TextShapeConstraints,
        ) -> TextMetrics {
            let intrinsic = text.value.len() as f32 * 8.0;
            TextMetrics {
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
        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
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
            .flush(LayoutViewport::new(320.0, 120.0), &mut LineShaper)
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
        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let wide_selection = nana_ui_core::PaintColor::Oklch {
            l: 0.72,
            c: 0.3,
            h: Some(150.0),
            alpha: 0.5,
        };
        let mut node_style = nana_ui_runtime::NodeStyle::default();
        let layout = std::sync::Arc::make_mut(&mut node_style.layout);
        layout.selection_background = Some(wide_selection.to_srgb());
        layout.paint_colors.selection_background = Some(wide_selection);
        // Distinct `selection_color` (foreground) must NOT bleed into selection background quads.
        layout.selection_color = Some([1.0, 0.0, 0.0, 1.0]);

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
            .flush(LayoutViewport::new(320.0, 120.0), &mut LineShaper)
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
}
