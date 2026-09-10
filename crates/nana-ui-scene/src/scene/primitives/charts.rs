//! Geometry-to-primitive projection; has no Scene index or Runtime access.
use super::*;
const DONUT_RING_REGIONS: u32 = 40;
pub(super) fn build(context: &GeometryPaintContext<'_>, emit: &mut impl FnMut(ScenePrimitive)) {
    let node = context.node;
    let bounds = context.bounds;
    let transform = context.transform;
    let clips = context.clips;
    let opacity = context.opacity;
    let node_order = context.node_order;
    let id = context.node.id;
    match context.node.component_geometry.as_deref() {
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
        #[cfg(feature = "charts")]
        Some(ComponentGeometry::DonutChart { regions, width }) => {
            let visual = VisualPrimitiveContext {
                node: id,
                transform,
                clips,
                opacity,
                z_index: node.z_index,
                document_order: node_order,
            };
            for (index, (circle, polygon, color)) in regions.iter().enumerate() {
                let mut primitive = visual_quad(
                    &visual,
                    collection_slot(DONUT_RING_REGIONS, index),
                    scene_rect(*circle),
                    VisualQuadStyle {
                        background: None,
                        border_color: Some(*color),
                        border_width: *width,
                        corner_radius: corner_radii(circle.width / 2.0),
                    },
                );
                if let ScenePrimitiveKind::Quad { surface, .. } = &mut primitive.kind {
                    surface.polygon_clip = Some(polygon.clone());
                }
                emit(primitive);
            }
        }
        #[cfg(feature = "charts")]
        Some(ComponentGeometry::StackedTimeSeriesChart {
            bars,
            legend,
            grid,
            line,
            labels,
            marker,
            grid_color,
            line_color,
        }) => {
            let visual = VisualPrimitiveContext {
                node: id,
                transform,
                clips,
                opacity,
                z_index: node.z_index,
                document_order: node_order,
            };
            emit(visual_quad_batch(
                &visual,
                10,
                grid.iter().copied().map(scene_rect),
                VisualQuadStyle::solid(*grid_color),
            ));
            for (index, (bar, color)) in bars.iter().enumerate() {
                let mut primitive = visual_quad(
                    &visual,
                    11,
                    scene_rect(*bar),
                    VisualQuadStyle {
                        background: Some(*color),
                        border_color: None,
                        border_width: 0.0,
                        corner_radius: corner_radii(3.0),
                    },
                );
                primitive.id.slot = 100 + index as u64;
                emit(primitive);
            }
            if line.len() >= 2 {
                emit(visual_stroke(
                    &visual,
                    12,
                    bounds,
                    smooth_chart_line(line),
                    2.0,
                    *line_color,
                ));
            }
            for (index, point) in line.iter().enumerate() {
                let mut primitive = visual_quad(
                    &visual,
                    13,
                    SceneRect {
                        x: point[0] - 2.0,
                        y: point[1] - 2.0,
                        width: 4.0,
                        height: 4.0,
                    },
                    VisualQuadStyle {
                        background: Some(*line_color),
                        border_color: None,
                        border_width: 0.0,
                        corner_radius: corner_radii(2.0),
                    },
                );
                primitive.id.slot = 100 + bars.len() as u64 + index as u64;
                emit(primitive);
            }
            for (index, (disc, color)) in legend.iter().enumerate() {
                let mut primitive = visual_quad(
                    &visual,
                    16,
                    scene_rect(*disc),
                    VisualQuadStyle {
                        background: Some(*color),
                        border_color: None,
                        border_width: 0.0,
                        corner_radius: corner_radii(5.0),
                    },
                );
                primitive.id.slot = 100
                    + bars.len() as u64
                    + line.len() as u64
                    + labels.len() as u64
                    + index as u64;
                emit(primitive);
            }
            for (index, label) in labels.iter().enumerate() {
                let mut primitive = component_text_primitive(
                    id,
                    14,
                    label,
                    TextHorizontalAlignment::Start,
                    false,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                );
                primitive.id.slot = 100 + bars.len() as u64 + line.len() as u64 + index as u64;
                emit(primitive);
            }
            if let Some(marker) = marker {
                emit(visual_quad(
                    &visual,
                    15,
                    scene_rect(*marker),
                    VisualQuadStyle {
                        background: Some(*line_color),
                        border_color: None,
                        border_width: 0.0,
                        corner_radius: corner_radii(3.0),
                    },
                ));
            }
        }
        _ => {}
    }
}

#[cfg(feature = "charts")]
fn smooth_chart_line(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut result = Vec::new();
    for index in 0..points.len().saturating_sub(1) {
        let p0 = points[index.saturating_sub(1)];
        let p1 = points[index];
        let p2 = points[index + 1];
        let p3 = points[(index + 2).min(points.len() - 1)];
        for step in 0..12 {
            let t = step as f32 / 12.0;
            let u = 1.0 - t;
            result.push(std::array::from_fn(|axis| {
                let c1 = p1[axis] + (p2[axis] - p0[axis]) * 0.25 / 3.0;
                let c2 = p2[axis] - (p3[axis] - p1[axis]) * 0.25 / 3.0;
                (u * u * u * p1[axis]
                    + 3.0 * u * u * t * c1
                    + 3.0 * u * t * t * c2
                    + t * t * t * p2[axis])
                    .clamp(p1[axis].min(p2[axis]), p1[axis].max(p2[axis]))
            }));
        }
    }
    if let Some(last) = points.last() {
        result.push(*last);
    }
    result
}
