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
        #[cfg(feature = "charts")]
        Some(ComponentGeometry::TimestampSeriesChart {
            grid,
            area,
            segments,
            labels,
            grid_color,
            area_color,
            line_color,
        }) => {
            let visual_context = VisualPrimitiveContext {
                node: id,
                transform,
                clips,
                opacity,
                z_index: node.z_index,
                document_order: node_order,
            };
            if !grid.is_empty() {
                emit(visual_quad_batch(
                    &visual_context,
                    10,
                    grid.iter().copied().map(scene_rect),
                    VisualQuadStyle::solid(*grid_color),
                ));
            }
            if !area.is_empty() {
                emit(visual_quad_batch(
                    &visual_context,
                    11,
                    area.iter().copied().map(scene_rect),
                    VisualQuadStyle::solid(*area_color),
                ));
            }
            // A single stroke primitive avoids the 256-slot ceiling for sparse series.
            // Segment colors explicitly suppress bridges between independent runs.
            let mut points = Vec::new();
            let mut colors = Vec::new();
            let mut dots = Vec::new();
            for run in segments {
                if run.len() == 1 {
                    dots.push(SceneRect {
                        x: run[0][0] - 1.5,
                        y: run[0][1] - 1.5,
                        width: 3.0,
                        height: 3.0,
                    });
                }
                for (index, point) in run.iter().enumerate() {
                    points.push(*point);
                    colors.push(if index + 1 == run.len() {
                        [0.0; 4]
                    } else {
                        *line_color
                    });
                }
            }
            if points.len() >= 2 {
                let mut stroke = visual_stroke(
                    &visual_context,
                    12,
                    bounds,
                    points,
                    TimeSeriesChart::LINE_WIDTH,
                    *line_color,
                );
                if let ScenePrimitiveKind::Stroke { pattern, .. } = &mut stroke.kind {
                    *pattern = Some(Box::new(StrokePattern {
                        colors,
                        ..Default::default()
                    }));
                }
                emit(stroke);
            }
            if !dots.is_empty() {
                emit(visual_quad_batch(
                    &visual_context,
                    13,
                    dots,
                    VisualQuadStyle::solid(*line_color),
                ));
            }
            for (index, label) in labels.iter().enumerate() {
                emit(component_text_primitive(
                    id,
                    20 + index as u64,
                    label,
                    if index == 2 {
                        TextHorizontalAlignment::End
                    } else {
                        TextHorizontalAlignment::Start
                    },
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
