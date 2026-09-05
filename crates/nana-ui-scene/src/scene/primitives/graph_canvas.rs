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
        #[cfg(feature = "graph-canvas")]
        Some(ComponentGeometry::GraphCanvas {
            nodes: graph_nodes,
            separators,
            ports,
            port_labels,
            edges,
            edge_labels,
            grid,
            background,
            grid_color,
            separator_color,
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
                bounds,
                VisualQuadStyle::solid(*background),
            ));
            if !grid.is_empty() {
                emit(visual_quad_batch(
                    &context,
                    11,
                    grid.iter().copied().map(scene_rect),
                    VisualQuadStyle::solid(*grid_color),
                ));
            }
            for (index, (points, color)) in edges.iter().enumerate() {
                if points.len() < 2 {
                    continue;
                }
                emit(visual_stroke(
                    &context,
                    12u64.saturating_add(index as u64),
                    bounds,
                    points.clone(),
                    1.6,
                    *color,
                ));
            }
            for (index, (node_bounds, label, fill, border)) in graph_nodes.iter().enumerate() {
                let index = u64::try_from(index).unwrap_or(u64::MAX);
                emit(visual_quad(
                    &context,
                    20u64.saturating_add(index),
                    scene_rect(*node_bounds),
                    VisualQuadStyle {
                        background: Some(*fill),
                        border_color: *border,
                        border_width: 1.0,
                        corner_radius: corner_radii(UI_METRICS.radius_sm),
                    },
                ));
                emit(component_text_primitive(
                    id,
                    50u64.saturating_add(index),
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
            if !separators.is_empty() {
                emit(visual_quad_batch(
                    &context,
                    40,
                    separators.iter().copied().map(scene_rect),
                    VisualQuadStyle::solid(*separator_color),
                ));
            }
            for (index, (port, fill, border, border_width)) in ports.iter().enumerate() {
                let index = u64::try_from(index).unwrap_or(u64::MAX);
                emit(visual_quad(
                    &context,
                    80u64.saturating_add(index),
                    scene_rect(*port),
                    VisualQuadStyle {
                        background: Some(*fill),
                        border_color: Some(*border),
                        border_width: *border_width,
                        corner_radius: corner_radii(999.0),
                    },
                ));
            }
            for (index, (label, alignment)) in port_labels.iter().enumerate() {
                let index = u64::try_from(index).unwrap_or(u64::MAX);
                emit(component_text_primitive(
                    id,
                    110u64.saturating_add(index),
                    label,
                    *alignment,
                    true,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                ));
            }
            for (index, label) in edge_labels.iter().enumerate() {
                let index = u64::try_from(index).unwrap_or(u64::MAX);
                emit(component_text_primitive(
                    id,
                    140u64.saturating_add(index),
                    label,
                    TextHorizontalAlignment::Center,
                    true,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                ));
            }
        }
        #[cfg(feature = "graph-canvas")]
        Some(ComponentGeometry::GraphMinimap {
            nodes,
            node_fill,
            indicator,
            indicator_fill,
            indicator_border,
        }) => {
            let context = VisualPrimitiveContext {
                node: id,
                transform,
                clips,
                opacity,
                z_index: node.z_index,
                document_order: node_order,
            };
            if !nodes.is_empty() {
                emit(visual_quad_batch(
                    &context,
                    10,
                    nodes.iter().copied().map(scene_rect),
                    VisualQuadStyle::solid(*node_fill),
                ));
            }
            if let Some(indicator) = indicator {
                emit(visual_quad(
                    &context,
                    11,
                    scene_rect(*indicator),
                    VisualQuadStyle {
                        background: Some(*indicator_fill),
                        border_color: Some(*indicator_border),
                        border_width: 1.5,
                        corner_radius: corner_radii(0.0),
                    },
                ));
            }
        }
        _ => {}
    }
}
