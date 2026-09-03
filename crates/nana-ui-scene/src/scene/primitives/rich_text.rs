//! Geometry-to-primitive projection; has no Scene index or Runtime access.
use super::*;
pub(super) fn build(context: &GeometryPaintContext<'_>, emit: &mut impl FnMut(ScenePrimitive)) {
    let node = context.node;
    let transform = context.transform;
    let clips = context.clips;
    let opacity = context.opacity;
    let node_order = context.node_order;
    let id = context.node.id;
    match context.node.component_geometry.as_ref() {
        #[cfg(feature = "rich-text")]
        Some(ComponentGeometry::NativeMarkdown {
            text,
            selection,
            selection_color,
        })
        | Some(ComponentGeometry::SelectableRichText {
            text,
            selection,
            selection_color,
        }) => {
            if !selection.is_empty() {
                emit(visual_quad_batch(
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
                ));
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
