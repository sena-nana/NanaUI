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
        #[cfg(feature = "image-viewer")]
        Some(ComponentGeometry::ImageViewer {
            scrim,
            surface,
            stage,
            close,
            name,
            metadata,
            scrim_color,
            surface_color,
            stage_color,
            ..
        }) => {
            let context = VisualPrimitiveContext {
                node: id,
                transform,
                clips,
                opacity,
                z_index: node.z_index,
                document_order: node_order,
            };
            emit(visual_quad(
                &context,
                10,
                scene_rect(*scrim),
                VisualQuadStyle::solid(*scrim_color),
            ));
            emit(visual_quad(
                &context,
                11,
                scene_rect(*surface),
                VisualQuadStyle {
                    background: Some(*surface_color),
                    border_color: None,
                    border_width: 0.0,
                    corner_radius: corner_radii(UI_METRICS.radius_md),
                },
            ));
            emit(visual_quad(
                &context,
                12,
                scene_rect(*stage),
                VisualQuadStyle::solid(*stage_color),
            ));
            emit(visual_quad(
                &context,
                13,
                scene_rect(*close),
                VisualQuadStyle {
                    background: None,
                    border_color: None,
                    border_width: 0.0,
                    corner_radius: corner_radii(UI_METRICS.radius_sm),
                },
            ));
            emit(component_text_primitive(
                id,
                16,
                &ComponentTextRegion {
                    bounds: *close,
                    content: Arc::from("×"),
                    color: node.style.color,
                    font_size: 15.0,
                    font_weight: None,
                },
                TextHorizontalAlignment::Center,
                false,
                node,
                transform,
                clips.clone(),
                opacity,
                node_order,
            ));
            if let Some(name) = name {
                emit(component_text_primitive(
                    id,
                    14,
                    name,
                    TextHorizontalAlignment::Start,
                    true,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                ));
            }
            if let Some(metadata) = metadata {
                emit(component_text_primitive(
                    id,
                    15,
                    metadata,
                    TextHorizontalAlignment::Start,
                    true,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                ));
            }
        }
        _ => {}
    }
}
