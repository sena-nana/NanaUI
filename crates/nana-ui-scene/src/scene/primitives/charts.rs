//! Geometry-to-primitive projection; has no Scene index or Runtime access.
use super::*;
pub(super) fn build(context: &GeometryPaintContext<'_>, emit: &mut impl FnMut(ScenePrimitive)) {
    let node = context.node;
    let bounds = context.bounds;
    let transform = context.transform;
    let clips = context.clips;
    let opacity = context.opacity;
    let node_order = context.node_order;
    let id = context.node.id;
    match context.node.component_geometry.as_ref() {
        #[cfg(feature = "charts")]
        Some(ComponentGeometry::TimeSeriesChart {
            grid,
            area,
            line,
            grid_color,
            area_color,
            line_color,
        }) => {
            let context = VisualPrimitiveContext {
                node: id,
                transform,
                clips,
                opacity,
                z_index: node.z_index,
                document_order: node_order,
            };
            if !grid.is_empty() {
                emit(visual_quad_batch(
                    &context,
                    10,
                    grid.iter().copied().map(scene_rect),
                    VisualQuadStyle::solid(*grid_color),
                ));
            }
            if !area.is_empty() {
                emit(visual_quad_batch(
                    &context,
                    11,
                    area.iter().copied().map(scene_rect),
                    VisualQuadStyle::solid(*area_color),
                ));
            }
            if line.len() >= 2 {
                emit(visual_stroke(
                    &context,
                    12,
                    bounds,
                    line.clone(),
                    TimeSeriesChart::LINE_WIDTH,
                    *line_color,
                ));
            }
        }
        _ => {}
    }
}
