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
        #[cfg(feature = "calendar")]
        Some(ComponentGeometry::CalendarHeatmap {
            cells,
            labels,
            hover,
        }) => {
            let mut groups: Vec<([f32; 4], Vec<SceneRect>)> = Vec::new();
            for (cell, color) in cells {
                match groups.iter_mut().find(|(existing, _)| existing == color) {
                    Some((_, rects)) => rects.push(scene_rect(*cell)),
                    None => groups.push((*color, vec![scene_rect(*cell)])),
                }
            }
            for (index, (color, rects)) in groups.into_iter().enumerate() {
                if rects.is_empty() {
                    continue;
                }
                emit(visual_quad_batch(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    },
                    10u8.saturating_add(index as u8),
                    rects,
                    VisualQuadStyle {
                        background: Some(color),
                        border_color: None,
                        border_width: 0.0,
                        corner_radius: corner_radii(UI_METRICS.radius_xs),
                    },
                ));
            }
            for (index, label) in labels.iter().enumerate() {
                emit(component_text_primitive(
                    id,
                    40u8.saturating_add(index as u8),
                    label,
                    TextHorizontalAlignment::Start,
                    false,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                ));
            }
            if let Some(hover) = hover {
                let hover_context = VisualPrimitiveContext {
                    node: id,
                    transform,
                    clips,
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                };
                emit(visual_quad(
                    &hover_context,
                    70,
                    scene_rect(hover.ring),
                    VisualQuadStyle {
                        background: None,
                        border_color: Some(hover.ring_color),
                        border_width: 1.5,
                        corner_radius: corner_radii(UI_METRICS.radius_xs + 1.0),
                    },
                ));
                emit(visual_quad(
                    &hover_context,
                    71,
                    scene_rect(hover.tooltip),
                    VisualQuadStyle {
                        background: Some(hover.tooltip_fill),
                        border_color: Some(hover.tooltip_border),
                        border_width: 1.0,
                        corner_radius: corner_radii(nana_ui_core::TooltipConfig::RADIUS),
                    },
                ));
                emit(component_text_primitive(
                    id,
                    72,
                    &hover.title,
                    TextHorizontalAlignment::Start,
                    false,
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
