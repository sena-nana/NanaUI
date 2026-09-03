//! Geometry-to-primitive projection; has no Scene index or Runtime access.
use super::*;
pub(super) fn build(context: &GeometryPaintContext<'_>, emit: &mut impl FnMut(ScenePrimitive)) {
    let node = context.node;
    let transform = context.transform;
    let clips = context.clips;
    let text_input_clips = context.text_input_clips;
    let empty_state_content_clips = context.empty_state_content_clips;
    let opacity = context.opacity;
    let node_order = context.node_order;
    let id = context.node.id;
    match context.node.component_geometry.as_ref() {
        Some(ComponentGeometry::TextInput {
            text,
            selection,
            selection_color,
            steppers,
            ..
        }) => {
            if let Some(steppers) = steppers {
                for (slot, icon, bounds, color) in [
                    (
                        8,
                        nana_ui_core::Icon::ChevronUp,
                        steppers.increment,
                        steppers.increment_color,
                    ),
                    (
                        9,
                        nana_ui_core::Icon::ChevronDown,
                        steppers.decrement,
                        steppers.decrement_color,
                    ),
                ] {
                    let extent = steppers
                        .glyph_size
                        .min(bounds.width)
                        .min(bounds.height)
                        .max(0.0);
                    emit(ScenePrimitive {
                        id: PrimitiveId { node: id, slot },
                        node: id,
                        bounds: SceneRect {
                            x: bounds.x + (bounds.width - extent) / 2.0,
                            y: bounds.y + (bounds.height - extent) / 2.0,
                            width: extent,
                            height: extent,
                        },
                        transform,
                        clips: clips.clone(),
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Icon {
                            icon,
                            color: Some(color),
                        },
                    });
                }
            }
            if !selection.is_empty() {
                emit(visual_quad_batch(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips: text_input_clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    },
                    1,
                    selection.iter().map(|selection| scene_rect(*selection)),
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
                text_input_clips.clone(),
                opacity,
                node_order,
            ));
        }
        Some(ComponentGeometry::StatusBadge {
            indicator,
            label,
            foreground,
            ..
        }) => {
            emit(component_text_primitive(
                id,
                2,
                label,
                TextHorizontalAlignment::Start,
                true,
                node,
                transform,
                clips.clone(),
                opacity,
                node_order,
            ));
            emit(visual_quad(
                &VisualPrimitiveContext {
                    node: id,
                    transform,
                    clips,
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                },
                3,
                scene_rect(*indicator),
                VisualQuadStyle {
                    background: Some(*foreground),
                    border_color: None,
                    border_width: 0.0,
                    corner_radius: corner_radii(999.0),
                },
            ));
        }
        Some(ComponentGeometry::ValidationMessage {
            indicator,
            label,
            foreground,
        }) => {
            emit(component_text_primitive(
                id,
                2,
                label,
                TextHorizontalAlignment::Start,
                true,
                node,
                transform,
                clips.clone(),
                opacity,
                node_order,
            ));
            emit(visual_quad(
                &VisualPrimitiveContext {
                    node: id,
                    transform,
                    clips,
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                },
                3,
                scene_rect(*indicator),
                VisualQuadStyle {
                    background: None,
                    border_color: Some(*foreground),
                    border_width: 1.0,
                    corner_radius: corner_radii(999.0),
                },
            ));
        }
        Some(ComponentGeometry::EmptyState {
            icon,
            title,
            message,
            ..
        }) => {
            emit(component_text_primitive(
                id,
                2,
                title,
                TextHorizontalAlignment::Start,
                false,
                node,
                transform,
                empty_state_content_clips.clone(),
                opacity,
                node_order,
            ));
            if let Some((icon, bounds, color)) = icon {
                emit(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 3 },
                    node: id,
                    bounds: scene_rect(*bounds),
                    transform,
                    clips: empty_state_content_clips.clone(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Icon {
                        icon: *icon,
                        color: Some(*color),
                    },
                });
            }
            if let Some(message) = message {
                emit(component_text_primitive(
                    id,
                    4,
                    message,
                    TextHorizontalAlignment::Start,
                    false,
                    node,
                    transform,
                    empty_state_content_clips.clone(),
                    opacity,
                    node_order,
                ));
            }
        }
        Some(ComponentGeometry::LabeledValue { label, value, .. }) => {
            emit(component_text_primitive(
                id,
                2,
                label,
                TextHorizontalAlignment::Start,
                true,
                node,
                transform,
                clips.clone(),
                opacity,
                node_order,
            ));
            emit(component_text_primitive(
                id,
                3,
                value,
                TextHorizontalAlignment::End,
                true,
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
