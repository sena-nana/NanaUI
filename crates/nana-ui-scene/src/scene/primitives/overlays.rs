//! Geometry-to-primitive projection; has no Scene index or Runtime access.
use super::*;
pub(super) fn build(context: &GeometryPaintContext<'_>, emit: &mut impl FnMut(ScenePrimitive)) {
    let node = context.node;
    let transform = context.transform;
    let clips = context.clips;
    let opacity = context.opacity;
    let node_order = context.node_order;
    let parent_clips = context.parent_clips;
    let id = context.node.id;
    match context.node.component_geometry.as_ref() {
        Some(ComponentGeometry::ModalFrame {
            scrim,
            surface,
            body: _,
            title,
            description,
            body_text,
            background,
            elevation,
            ..
        }) => {
            emit(visual_quad(
                &VisualPrimitiveContext {
                    node: id,
                    transform,
                    clips,
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                },
                10,
                scene_rect(*scrim),
                VisualQuadStyle::solid([0.0, 0.0, 0.0, 0.45]),
            ));
            let radius = UI_METRICS.radius_md;
            let docked = match node.standard_visual.as_ref() {
                Some(StandardVisual::ModalFrame {
                    kind: nana_ui_runtime::ModalSurfaceKind::Drawer(side),
                    ..
                }) => Some(*side),
                _ => None,
            };
            let mut surface_bounds = scene_rect(*surface);
            let mut surface_clips = clips.to_vec();
            if let Some(side) = docked {
                surface_clips.push(ClipRegion {
                    bounds: scene_rect(*scrim),
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
                match side {
                    DrawerSide::Right => surface_bounds.width += radius,
                    DrawerSide::Left => {
                        surface_bounds.x -= radius;
                        surface_bounds.width += radius;
                    }
                    DrawerSide::Bottom => surface_bounds.height += radius,
                }
            }
            emit(ScenePrimitive {
                id: PrimitiveId { node: id, slot: 11 },
                node: id,
                bounds: surface_bounds,
                transform,
                clips: surface_clips.into(),
                opacity,
                z_index: node.z_index,
                document_order: node_order,
                kind: ScenePrimitiveKind::Quad {
                    background: Some(*background),
                    border_color: None,
                    border_width: 0.0,
                    corner_radius: corner_radii(radius),
                    shadow: Some(*elevation),
                    surface: QuadSurfacePaint::default(),
                },
            });
            emit(component_text_primitive(
                id,
                12,
                title,
                TextHorizontalAlignment::Start,
                true,
                node,
                transform,
                clips.clone(),
                opacity,
                node_order,
            ));
            if let Some(description) = description {
                emit(component_text_primitive(
                    id,
                    13,
                    description,
                    TextHorizontalAlignment::Start,
                    true,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                ));
            }
            if let Some(body_text) = body_text {
                emit(component_text_primitive(
                    id,
                    14,
                    body_text,
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
        Some(ComponentGeometry::Toast {
            indicator,
            title,
            description,
            dismiss,
        }) => {
            emit(visual_quad(
                &VisualPrimitiveContext {
                    node: id,
                    transform,
                    clips,
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                },
                1,
                scene_rect(*indicator),
                VisualQuadStyle {
                    background: node.standard_visual_foreground,
                    border_color: None,
                    border_width: 0.0,
                    corner_radius: corner_radii(999.0),
                },
            ));
            emit(component_text_primitive(
                id,
                2,
                title,
                TextHorizontalAlignment::Start,
                true,
                node,
                transform,
                clips.clone(),
                opacity,
                node_order,
            ));
            if let Some(description) = description {
                emit(component_text_primitive(
                    id,
                    3,
                    description,
                    TextHorizontalAlignment::Start,
                    true,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                ));
            }
            if let Some(dismiss) = dismiss {
                emit(component_text_primitive(
                    id,
                    4,
                    &ComponentTextRegion {
                        bounds: *dismiss,
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
            }
        }
        Some(ComponentGeometry::ActionMenuItem {
            icon, label, hint, ..
        }) => {
            if let Some((icon, icon_bounds, color)) = icon {
                emit(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 3 },
                    node: id,
                    bounds: scene_rect(*icon_bounds),
                    transform,
                    clips: clips.clone(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Icon {
                        icon: *icon,
                        color: Some(*color),
                    },
                });
            }
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
            if let Some(hint) = hint {
                emit(component_text_primitive(
                    id,
                    4,
                    hint,
                    TextHorizontalAlignment::End,
                    true,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                ));
            }
        }
        Some(ComponentGeometry::MenuSurface {
            trigger,
            trigger_icon,
            trigger_surface,
            surface,
            search,
            search_field,
            options,
            elevation,
            background,
            border,
        }) => {
            if let Some(chrome) = trigger_surface
                && (chrome.background.is_some() || chrome.border.is_some())
            {
                emit(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 1 },
                    node: id,
                    bounds: scene_rect(chrome.bounds),
                    transform,
                    clips: clips.clone(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Quad {
                        background: chrome.background,
                        border_color: chrome.border,
                        border_width: 1.0,
                        corner_radius: corner_radii(UI_METRICS.radius_sm),
                        shadow: None,
                        surface: QuadSurfacePaint::default(),
                    },
                });
            }
            if let Some((icon, icon_bounds)) = trigger_icon {
                emit(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 2 },
                    node: id,
                    bounds: scene_rect(*icon_bounds),
                    transform,
                    clips: clips.clone(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Icon {
                        icon: *icon,
                        color: node.standard_visual_foreground.or(node.style.color),
                    },
                });
            } else if let Some(trigger) = trigger {
                emit(component_text_primitive(
                    id,
                    2,
                    trigger,
                    TextHorizontalAlignment::Start,
                    false,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                ));
            }
            if surface.height > 1.0 && surface.width > 1.0 {
                emit(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 0 },
                    node: id,
                    bounds: scene_rect(*surface),
                    transform,
                    clips: Arc::clone(parent_clips),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Quad {
                        background: Some(*background),
                        border_color: Some(*border),
                        border_width: 1.0,
                        corner_radius: corner_radii(UI_METRICS.radius_md),
                        shadow: Some(*elevation),
                        surface: QuadSurfacePaint::default(),
                    },
                });
            }
            if let Some(field) = search_field {
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
                    scene_rect(*field),
                    VisualQuadStyle {
                        background: Some(*background),
                        border_color: Some(*border),
                        border_width: 1.0,
                        corner_radius: corner_radii(UI_METRICS.radius_sm),
                    },
                ));
            }
            if let Some(search) = search {
                emit(component_text_primitive(
                    id,
                    4,
                    search,
                    TextHorizontalAlignment::Start,
                    true,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                ));
            }
            for (index, option) in options.iter().enumerate() {
                if let Some(background) = option.background {
                    emit(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        10u64.saturating_add(index as u64),
                        scene_rect(option.bounds),
                        VisualQuadStyle {
                            background: Some(background),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(UI_METRICS.radius_sm),
                        },
                    ));
                }
                if let Some((icon, icon_bounds, color)) = option.icon {
                    emit(ScenePrimitive {
                        id: PrimitiveId {
                            node: id,
                            slot: 80u64.saturating_add(index as u64),
                        },
                        node: id,
                        bounds: scene_rect(icon_bounds),
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
                emit(component_text_primitive(
                    id,
                    40u64.saturating_add(index as u64),
                    &option.label,
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
