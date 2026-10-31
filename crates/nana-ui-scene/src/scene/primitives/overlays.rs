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
    match context.node.component_geometry.as_deref() {
        Some(ComponentGeometry::ModalFrame {
            scrim,
            surface,
            body: _,
            title,
            description,
            body_text,
            background,
            elevation,
            corner_radius,
            header_divider,
            footer_divider,
            scrim_color,
            scrim_blur,
            scrim_opacity,
            surface_paint,
            ..
        }) => {
            // The scrim fades by its own opacity, not its node's layer (the
            // card's): see `UiScene::compositor_primitive_opacity`.
            let mut scrim_quad = visual_quad(
                &VisualPrimitiveContext {
                    node: id,
                    transform,
                    clips,
                    opacity: opacity * scrim_opacity.clamp(0.0, 1.0),
                    z_index: node.z_index,
                    document_order: node_order,
                },
                crate::scene::MODAL_SCRIM_SLOT,
                scene_rect(*scrim),
                VisualQuadStyle::solid(*scrim_color),
            );
            // The theme's backdrop blur: the window behind the scrim is
            // blurred first, then the scrim's colour laid over it, as CSS
            // paints an element's `backdrop-filter` under its background.
            let blur = nana_ui_core::BackdropFilter {
                blur_radius: scrim_blur.clamp(0.0, nana_ui_core::BackdropFilter::MAX_BLUR_RADIUS),
                saturate: 1.0,
            };
            if blur.is_active()
                && let ScenePrimitiveKind::Quad { surface, .. } = &mut scrim_quad.kind
            {
                surface.backdrop_filter = Some(blur);
            }
            emit(scrim_quad);
            let radius = *corner_radius;
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
            // A dialog that paints its own card leaves the theme's out,
            // unless its painter asked for it with `draw_default()`.
            let themed_card = surface_paint
                .as_ref()
                .is_none_or(|recording| recording.default_phase().is_some());
            if themed_card {
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
                        background: crate::scene::slot_color(None, Some(*background)),
                        border_color: crate::scene::slot_color(None, None),
                        border_width: 0.0,
                        corner_radius: corner_radii(radius),
                        shadow: Some(*elevation),
                        surface: Box::default(),
                    },
                });
            }
            // The card's section dividers: hairlines across the whole card,
            // inside its edges, so the card's own clip trims them.
            for (slot, divider) in [(15, header_divider), (16, footer_divider)] {
                let Some((bounds, color)) = divider else {
                    continue;
                };
                emit(visual_quad(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    },
                    slot,
                    scene_rect(*bounds),
                    VisualQuadStyle::solid(*color),
                ));
            }
            // A title content slot leaves the title text empty: the slot is
            // drawn in its place, as a child.
            if !title.content.is_empty() {
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
            }
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
            glyph,
            title,
            description,
            dismiss,
            ..
        }) => {
            // The tone mark, in the tone colour: the recipe's glyph, or a dot.
            match (indicator, glyph) {
                (Some(bounds), Some(icon)) => emit(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 1 },
                    node: id,
                    bounds: scene_rect(*bounds),
                    transform,
                    clips: clips.clone(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Icon {
                        icon: *icon,
                        color: crate::scene::slot_color(None, node.standard_visual_foreground),
                    },
                }),
                (Some(bounds), None) => emit(visual_quad(
                    &VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    },
                    1,
                    scene_rect(*bounds),
                    VisualQuadStyle {
                        background: node.standard_visual_foreground,
                        border_color: None,
                        border_width: 0.0,
                        corner_radius: corner_radii(999.0),
                    },
                )),
                (None, _) => {}
            }
            // An assembled toast's copy is child text: it paints none here.
            if let Some(title) = title {
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
                        content: nana_ui_runtime::TextValue::from("×"),
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
                        color: crate::scene::slot_color(
                            matching_paint_color(node.style.paint_colors.color, Some(*color)),
                            Some(*color),
                        ),
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
            trigger_image,
            trigger_surface,
            surface,
            search,
            search_field,
            options,
            elevation,
            background,
            border,
            ..
        }) => {
            if let Some(chrome) = trigger_surface
                && (chrome.background.is_some() || chrome.border.is_some())
            {
                // 显式圆角沿用普通按钮解析；头像与默认入口各自保留主题默认值。
                let style = node.source_style.layout.as_ref();
                let radii = if style.border_radius.is_some() || style.paint.border_radii.is_some() {
                    surface_corner_radii(style, chrome.bounds.width, chrome.bounds.height)
                } else if trigger_image.is_some() {
                    corner_radii(chrome.bounds.width.min(chrome.bounds.height) * 0.5)
                } else {
                    corner_radii(node.chrome_radii.sm)
                };
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
                        background: crate::scene::slot_color(None, chrome.background),
                        border_color: crate::scene::slot_color(None, chrome.border),
                        border_width: 1.0,
                        corner_radius: radii,
                        shadow: None,
                        surface: Box::default(),
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
                        color: crate::scene::slot_color(
                            node.style.paint_colors.color.filter(|paint| {
                                node.standard_visual_foreground == Some(paint.to_srgb())
                            }),
                            node.standard_visual_foreground
                                .or_else(|| {
                                    node.style
                                        .paint_colors
                                        .color
                                        .map(nana_ui_core::PaintColor::to_srgb)
                                })
                                .or(node.style.color),
                        ),
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
                // A triggered surface wraps content Runtime lays out
                // viewport-fixed above the page. It is painted the same way:
                // untransformed, uncut by the trigger's clips, and in the root
                // stacking context at the content's own level, above the
                // trigger and whatever the page stacks after it. A context
                // menu's surface is its own node's box and stays with it.
                let (slot, transform, clips, z_index) = if trigger_surface.is_some() {
                    (
                        TRIGGERED_OVERLAY_SURFACE_SLOT,
                        AffineTransform::IDENTITY,
                        Arc::from([]),
                        nana_ui_runtime::MENU_OVERLAY_Z_INDEX,
                    )
                } else {
                    (0, transform, Arc::clone(parent_clips), node.z_index)
                };
                emit(ScenePrimitive {
                    id: PrimitiveId { node: id, slot },
                    node: id,
                    bounds: scene_rect(*surface),
                    transform,
                    clips,
                    opacity,
                    z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Quad {
                        background: crate::scene::slot_color(None, Some(*background)),
                        border_color: crate::scene::slot_color(None, Some(*border)),
                        border_width: 1.0,
                        corner_radius: corner_radii(node.chrome_radii.md),
                        shadow: Some(*elevation),
                        surface: Box::default(),
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
                        corner_radius: corner_radii(node.chrome_radii.sm),
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
                            corner_radius: corner_radii(node.chrome_radii.sm),
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
                            color: crate::scene::slot_color(
                                matching_paint_color(node.style.paint_colors.color, Some(color)),
                                Some(color),
                            ),
                        },
                    });
                }
                let mut label = component_text_primitive(
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
                );
                if let Some(content) = option.rich_content.as_ref()
                    && let ScenePrimitiveKind::Text { spans, .. } = &mut label.kind
                {
                    *spans = crate::scene::rich_fill_spans(
                        option.label.content.as_ref(),
                        content.rich_text(),
                        std::mem::take(spans),
                    );
                }
                emit(label);
                if let Some(hint) = &option.hint {
                    emit(component_text_primitive(
                        id,
                        collection_slot(CONTEXT_MENU_HINTS, index),
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
        }
        _ => {}
    }
}
