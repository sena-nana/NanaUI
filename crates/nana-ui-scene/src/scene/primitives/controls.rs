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
        #[cfg(feature = "controls")]
        Some(ComponentGeometry::ReorderList { rows, insert }) => {
            let selected = rows
                .iter()
                .filter_map(|(row, _, fill)| fill.map(|color| (scene_rect(*row), color)))
                .collect::<Vec<_>>();
            if !selected.is_empty() {
                let color = selected[0].1;
                emit(visual_quad_batch(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    },
                    10,
                    selected.iter().map(|(rect, _)| *rect),
                    VisualQuadStyle {
                        background: Some(color),
                        border_color: None,
                        border_width: 0.0,
                        corner_radius: corner_radii(UI_METRICS.radius_sm),
                    },
                ));
            }
            if let Some((line, color)) = insert {
                emit(visual_quad(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    },
                    11,
                    scene_rect(*line),
                    VisualQuadStyle::solid(*color),
                ));
            }
            for (index, (_, label, _)) in rows.iter().enumerate() {
                emit(component_text_primitive(
                    id,
                    40u64.saturating_add(index as u64),
                    label,
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
