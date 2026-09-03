//! Geometry-to-primitive projection; has no Scene index or Runtime access.
use super::*;
pub(super) fn build(context: &GeometryPaintContext<'_>, emit: &mut impl FnMut(ScenePrimitive)) {
    let node = context.node;
    let bounds = context.bounds;
    let transform = context.transform;
    let clips = context.clips;
    let opacity = context.opacity;
    let node_order = context.node_order;
    let parent_clips = context.parent_clips;
    let style = context.node.source_style.layout.as_ref();
    let id = context.node.id;
    match context.node.component_geometry.as_ref() {
        Some(ComponentGeometry::Button { label, .. }) => {
            emit(component_text_primitive(
                id,
                2,
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
        Some(ComponentGeometry::Switch { label, hint, .. }) => {
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
                    3,
                    hint,
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
        Some(ComponentGeometry::Range {
            label, value, unit, ..
        }) => {
            if let Some(label) = label {
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
            }
            emit(component_text_primitive(
                id,
                6,
                value,
                TextHorizontalAlignment::End,
                true,
                node,
                transform,
                clips.clone(),
                opacity,
                node_order,
            ));
            if let Some(unit) = unit {
                emit(component_text_primitive(
                    id,
                    7,
                    unit,
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
        Some(ComponentGeometry::Card {
            title: Some(title), ..
        }) => {
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
        }
        Some(ComponentGeometry::ListItem {
            detail: Some(region),
            ..
        }) => {
            emit(component_text_primitive(
                id,
                3,
                region,
                TextHorizontalAlignment::End,
                true,
                node,
                transform,
                clips.clone(),
                opacity,
                node_order,
            ));
        }
        Some(ComponentGeometry::SelectionOption {
            icon,
            label,
            focus_ring,
            indicator,
        }) => {
            emit(component_text_primitive(
                id,
                2,
                label,
                if indicator.is_some() {
                    TextHorizontalAlignment::Start
                } else {
                    TextHorizontalAlignment::Center
                },
                true,
                node,
                transform,
                clips.clone(),
                opacity,
                node_order,
            ));
            if let Some(indicator) = indicator {
                let indicator_context = VisualPrimitiveContext {
                    node: id,
                    transform,
                    clips,
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                };
                emit(visual_quad(
                    &indicator_context,
                    8,
                    scene_rect(indicator.ring),
                    VisualQuadStyle {
                        background: None,
                        border_color: Some(indicator.ring_color),
                        border_width: if indicator.dot.is_some() { 2.0 } else { 1.0 },
                        corner_radius: corner_radii(indicator.ring.height / 2.0),
                    },
                ));
                if let Some((dot, color)) = indicator.dot {
                    emit(visual_quad(
                        &indicator_context,
                        9,
                        scene_rect(dot),
                        VisualQuadStyle {
                            background: Some(color),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(dot.height / 2.0),
                        },
                    ));
                }
            }
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
            if let Some(color) = focus_ring {
                emit(visual_quad(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips: parent_clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    },
                    7,
                    SceneRect {
                        x: bounds.x - 4.0,
                        y: bounds.y - 4.0,
                        width: bounds.width + 8.0,
                        height: bounds.height + 8.0,
                    },
                    VisualQuadStyle {
                        background: None,
                        border_color: Some(*color),
                        border_width: 2.0,
                        corner_radius: focus_ring_corner_radius(
                            style,
                            SceneRect {
                                x: bounds.x - 4.0,
                                y: bounds.y - 4.0,
                                width: bounds.width + 8.0,
                                height: bounds.height + 8.0,
                            },
                            4.0,
                        ),
                    },
                ));
            }
        }
        Some(ComponentGeometry::Progress {
            track,
            fill,
            label,
            cancel,
            corner_radius,
        }) => {
            if let Some(label) = label {
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
            }
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
                scene_rect(*track),
                VisualQuadStyle {
                    background: node.style.background,
                    border_color: None,
                    border_width: 0.0,
                    corner_radius: corner_radii(*corner_radius),
                },
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
                4,
                scene_rect(*fill),
                VisualQuadStyle {
                    background: node.standard_visual_foreground,
                    border_color: None,
                    border_width: 0.0,
                    corner_radius: corner_radii(*corner_radius),
                },
            ));
            if let Some(cancel) = cancel {
                emit(component_text_primitive(
                    id,
                    5,
                    &ComponentTextRegion {
                        bounds: *cancel,
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
        Some(ComponentGeometry::FormField {
            label,
            support,
            indicator,
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
            if let Some(support) = support {
                emit(component_text_primitive(
                    id,
                    3,
                    support,
                    TextHorizontalAlignment::Start,
                    true,
                    node,
                    transform,
                    clips.clone(),
                    opacity,
                    node_order,
                ));
            }
            if let Some((bounds, color)) = indicator {
                emit(visual_quad(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    },
                    4,
                    scene_rect(*bounds),
                    VisualQuadStyle {
                        background: None,
                        border_color: Some(*color),
                        border_width: 1.0,
                        corner_radius: corner_radii(999.0),
                    },
                ));
            }
        }
        Some(ComponentGeometry::XYPad {
            pad: _,
            thumb,
            h_axis,
            v_axis,
            thumb_color,
            axis_color,
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
                1,
                scene_rect(*h_axis),
                VisualQuadStyle::solid(*axis_color),
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
                2,
                scene_rect(*v_axis),
                VisualQuadStyle::solid(*axis_color),
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
                scene_rect(*thumb),
                VisualQuadStyle {
                    background: Some(*thumb_color),
                    border_color: None,
                    border_width: 0.0,
                    corner_radius: corner_radii(999.0),
                },
            ));
        }
        Some(ComponentGeometry::QrCode { field, dark, .. }) => {
            emit(visual_quad(
                &VisualPrimitiveContext {
                    node: id,
                    transform,
                    clips,
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                },
                0,
                scene_rect(*field),
                VisualQuadStyle {
                    background: Some([1.0, 1.0, 1.0, 1.0]),
                    border_color: None,
                    border_width: 0.0,
                    corner_radius: corner_radii(UI_METRICS.radius_md),
                },
            ));
            if !dark.is_empty() {
                emit(visual_quad_batch(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    },
                    1,
                    dark.iter().copied().map(scene_rect),
                    VisualQuadStyle::solid([0.0, 0.0, 0.0, 1.0]),
                ));
            }
        }
        Some(ComponentGeometry::Select {
            label,
            handle,
            handle_color,
            menu,
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
            paint_select_handle(
                emit,
                id,
                handle,
                *handle_color,
                transform,
                clips,
                opacity,
                node.z_index,
                node_order,
            );
            if let Some(menu) = menu {
                let menu_z = node.z_index.max(1_000);
                emit(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 4 },
                    node: id,
                    bounds: scene_rect(menu.surface),
                    transform,
                    clips: Arc::clone(parent_clips),
                    opacity,
                    z_index: menu_z,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Quad {
                        background: Some(menu.background),
                        border_color: Some(menu.border),
                        border_width: 1.0,
                        corner_radius: corner_radii(UI_METRICS.radius_md),
                        shadow: Some(menu.elevation),
                        surface: QuadSurfacePaint::default(),
                    },
                });
                for (index, option) in menu.options.iter().enumerate() {
                    let index = u8::try_from(index).unwrap_or(u8::MAX);
                    if option.checked {
                        let mut mark = component_text_primitive(
                            id,
                            70u8.saturating_add(index),
                            &nana_ui_runtime::ComponentTextRegion {
                                bounds: nana_ui_runtime::LayoutBox {
                                    x: option.bounds.x,
                                    y: option.bounds.y,
                                    width: 16.0,
                                    height: option.bounds.height,
                                },
                                content: Arc::from("✓"),
                                color: node.standard_visual_foreground.or(option.label.color),
                                font_size: option.label.font_size,
                                font_weight: Some(700),
                            },
                            TextHorizontalAlignment::Center,
                            false,
                            node,
                            transform,
                            Arc::clone(parent_clips),
                            opacity,
                            node_order,
                        );
                        mark.z_index = menu_z;
                        emit(mark);
                    }
                    if let Some(background) = option.background {
                        emit(visual_quad(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: parent_clips,
                                opacity,
                                z_index: menu_z,
                                document_order: node_order,
                            },
                            10u8.saturating_add(index),
                            scene_rect(option.bounds),
                            VisualQuadStyle {
                                background: Some(background),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(UI_METRICS.radius_sm),
                            },
                        ));
                    }
                    let mut label = component_text_primitive(
                        id,
                        40u8.saturating_add(index),
                        &option.label,
                        TextHorizontalAlignment::Start,
                        true,
                        node,
                        transform,
                        Arc::clone(parent_clips),
                        opacity,
                        node_order,
                    );
                    label.z_index = menu_z;
                    emit(label);
                }
            }
        }
        Some(ComponentGeometry::TreeView { rows }) => {
            for (index, row) in rows.iter().enumerate() {
                let index = u8::try_from(index).unwrap_or(u8::MAX);
                if let Some(background) = row.background {
                    emit(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        10u8.saturating_add(index),
                        scene_rect(row.bounds),
                        VisualQuadStyle {
                            background: Some(background),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(UI_METRICS.radius_sm),
                        },
                    ));
                }
                if let Some(disclosure) = row.disclosure {
                    emit(component_text_primitive(
                        id,
                        40u8.saturating_add(index),
                        &nana_ui_runtime::ComponentTextRegion {
                            bounds: disclosure,
                            content: Arc::from(if row.expanded { "▾" } else { "▸" }),
                            color: row.label.color,
                            font_size: row.label.font_size,
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
                if let Some((icon, icon_bounds, color)) = row.icon {
                    emit(ScenePrimitive {
                        id: PrimitiveId {
                            node: id,
                            slot: 80u8.saturating_add(index),
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
                    110u8.saturating_add(index),
                    &row.label,
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
        Some(ComponentGeometry::CommandPalette {
            scrim,
            surface,
            title,
            input,
            empty,
            rows,
            background,
            input_background,
            input_border,
            elevation,
        }) => {
            let overlay_z = node.z_index.max(1_000);
            emit(visual_quad(
                &VisualPrimitiveContext {
                    node: id,
                    transform,
                    clips: parent_clips,
                    opacity,
                    z_index: overlay_z,
                    document_order: node_order,
                },
                10,
                scene_rect(*scrim),
                VisualQuadStyle::solid([0.0, 0.0, 0.0, 0.45]),
            ));
            emit(ScenePrimitive {
                id: PrimitiveId { node: id, slot: 11 },
                node: id,
                bounds: scene_rect(*surface),
                transform,
                clips: Arc::clone(parent_clips),
                opacity,
                z_index: overlay_z,
                document_order: node_order,
                kind: ScenePrimitiveKind::Quad {
                    background: Some(*background),
                    border_color: None,
                    border_width: 0.0,
                    corner_radius: corner_radii(UI_METRICS.radius_md),
                    shadow: Some(*elevation),
                    surface: QuadSurfacePaint::default(),
                },
            });
            let mut title_text = component_text_primitive(
                id,
                20,
                title,
                TextHorizontalAlignment::Start,
                true,
                node,
                transform,
                Arc::clone(parent_clips),
                opacity,
                node_order,
            );
            title_text.z_index = overlay_z;
            emit(title_text);
            emit(visual_quad(
                &VisualPrimitiveContext {
                    node: id,
                    transform,
                    clips: parent_clips,
                    opacity,
                    z_index: overlay_z,
                    document_order: node_order,
                },
                12,
                scene_rect(input.bounds),
                VisualQuadStyle {
                    background: Some(*input_background),
                    border_color: Some(*input_border),
                    border_width: 1.0,
                    corner_radius: corner_radii(UI_METRICS.radius_sm),
                },
            ));
            let mut input_text = component_text_primitive(
                id,
                21,
                input,
                TextHorizontalAlignment::Start,
                true,
                node,
                transform,
                Arc::clone(parent_clips),
                opacity,
                node_order,
            );
            input_text.z_index = overlay_z;
            emit(input_text);
            if let Some(empty) = empty {
                let mut empty_text = component_text_primitive(
                    id,
                    22,
                    empty,
                    TextHorizontalAlignment::Start,
                    true,
                    node,
                    transform,
                    Arc::clone(parent_clips),
                    opacity,
                    node_order,
                );
                empty_text.z_index = overlay_z;
                emit(empty_text);
            }
            for (index, row) in rows.iter().enumerate() {
                let index = u8::try_from(index).unwrap_or(u8::MAX);
                if let Some(background) = row.background {
                    emit(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: parent_clips,
                            opacity,
                            z_index: overlay_z,
                            document_order: node_order,
                        },
                        23u8.saturating_add(index),
                        scene_rect(row.bounds),
                        VisualQuadStyle {
                            background: Some(background),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(UI_METRICS.radius_sm),
                        },
                    ));
                }
                let mut label = component_text_primitive(
                    id,
                    40u8.saturating_add(index),
                    &row.label,
                    TextHorizontalAlignment::Start,
                    true,
                    node,
                    transform,
                    Arc::clone(parent_clips),
                    opacity,
                    node_order,
                );
                label.z_index = overlay_z;
                emit(label);
                if let Some(category) = &row.category {
                    let mut category = component_text_primitive(
                        id,
                        70u8.saturating_add(index),
                        category,
                        TextHorizontalAlignment::Start,
                        true,
                        node,
                        transform,
                        Arc::clone(parent_clips),
                        opacity,
                        node_order,
                    );
                    category.z_index = overlay_z;
                    emit(category);
                }
                if let Some(shortcut) = &row.shortcut {
                    let mut shortcut = component_text_primitive(
                        id,
                        100u8.saturating_add(index),
                        shortcut,
                        TextHorizontalAlignment::End,
                        true,
                        node,
                        transform,
                        Arc::clone(parent_clips),
                        opacity,
                        node_order,
                    );
                    shortcut.z_index = overlay_z;
                    emit(shortcut);
                }
            }
        }
        Some(ComponentGeometry::KeyCaptureLayer { badge, background }) => {
            if let Some(background) = background {
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
                    scene_rect(badge.bounds),
                    VisualQuadStyle {
                        background: Some(*background),
                        border_color: None,
                        border_width: 0.0,
                        corner_radius: corner_radii(UI_METRICS.radius_sm),
                    },
                ));
            }
            emit(component_text_primitive(
                id,
                2,
                badge,
                TextHorizontalAlignment::Center,
                false,
                node,
                transform,
                clips.clone(),
                opacity,
                node_order,
            ));
        }
        Some(ComponentGeometry::KeymapLayer { badge }) => {
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
                scene_rect(badge.bounds),
                VisualQuadStyle {
                    background: node.style.background.or(badge
                        .color
                        .map(|color| [color[0], color[1], color[2], 0.12])),
                    border_color: None,
                    border_width: 0.0,
                    corner_radius: corner_radii(UI_METRICS.radius_sm),
                },
            ));
            emit(component_text_primitive(
                id,
                2,
                badge,
                TextHorizontalAlignment::Center,
                false,
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
