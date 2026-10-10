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
        #[cfg(feature = "controls")]
        Some(ComponentGeometry::ReorderList {
            rows,
            insert,
            inside,
            dragged,
        }) => {
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
                        corner_radius: corner_radii(node.chrome_radii.sm),
                    },
                ));
            }
            // The dragged row is outlined where it was, so the row being
            // moved stays apparent while the insert line shows where it goes.
            // Both paint one level above the list, over live row children:
            // a selected row's fill would hide them otherwise.
            let over_rows = node.z_index.saturating_add(1);
            if let Some((row, color)) = dragged {
                emit(visual_quad(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips,
                        opacity,
                        z_index: over_rows,
                        document_order: node_order,
                    },
                    12,
                    scene_rect(*row),
                    VisualQuadStyle {
                        background: None,
                        border_color: Some(*color),
                        border_width: 1.5,
                        corner_radius: corner_radii(node.chrome_radii.sm),
                    },
                ));
            }
            // The row a drop goes into: a ring and a faint wash of the
            // accent, so its own label stays readable under it.
            if let Some((row, color)) = inside {
                let [r, g, b, a] = *color;
                emit(visual_quad(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips,
                        opacity,
                        z_index: over_rows,
                        document_order: node_order,
                    },
                    13,
                    scene_rect(*row),
                    VisualQuadStyle {
                        background: Some([r, g, b, a * 0.12]),
                        border_color: Some(*color),
                        border_width: 2.0,
                        corner_radius: corner_radii(node.chrome_radii.sm),
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
                        z_index: over_rows,
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
