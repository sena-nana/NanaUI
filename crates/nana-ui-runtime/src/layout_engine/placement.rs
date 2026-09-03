//! Shared Runtime layout placement algorithms.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn place_node(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
) -> Result<(), UiWorldError> {
    place_node_scoped(
        id,
        origin,
        size,
        containing,
        viewport,
        parent_font_px,
        nodes,
        intrinsic,
        output,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn place_node_scoped(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: Option<&ScopeContext<'_>>,
    inherited_grid: Option<&InheritedGridTracks>,
) -> Result<(), UiWorldError> {
    let Some(node) = nodes.get(id)? else {
        output.insert(
            id,
            LayoutBox {
                x: origin.x,
                y: origin.y,
                width: 0.0,
                height: 0.0,
            },
        );
        return Ok(());
    };
    let style = node.style.clone();
    let child_ids = node.children.clone();
    let modal = node.modal.clone();
    let style = style.as_ref();
    if style.omits_box() {
        output.insert(
            id,
            LayoutBox {
                x: origin.x,
                y: origin.y,
                width: 0.0,
                height: 0.0,
            },
        );
        return Ok(());
    }
    let fonts = fonts_of(style, parent_font_px);
    let child_font_px = fonts.element_px;
    let (relative_x, relative_y) =
        style.relative_offset_against_fonts(Some(containing.width), Some(containing.height), fonts);
    let origin = Point {
        x: origin.x + relative_x,
        y: origin.y + relative_y,
    };
    output.insert(
        id,
        LayoutBox {
            x: origin.x,
            y: origin.y,
            width: size.width,
            height: size.height,
        },
    );

    if let Some(modal) = modal.as_ref() {
        place_modal_children(
            id,
            origin,
            size,
            modal,
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
        return Ok(());
    }

    let padding = style.resolved_padding_against_fonts(Some(size.width), fonts);
    let border = style.resolved_border_edges();
    let content_origin = Point {
        x: origin.x + border.left + padding.left,
        y: origin.y + border.top + padding.top,
    };
    let content = Size::new(
        size.width - padding.left - padding.right - border.left - border.right,
        size.height - padding.top - padding.bottom - border.top - border.bottom,
    );
    let mut flow = collect_flow_children(&child_ids, nodes, style.display)?;
    let mut positioned = collect_positioned_children(&child_ids, nodes)?;
    let floated = if style
        .display
        .is_some_and(|d| d.is_flex_container() || d.is_grid_container())
    {
        Vec::new()
    } else {
        collect_floated_children(&child_ids, nodes)?
    };
    if !floated.is_empty() {
        flow.retain(|id| !floated.contains(id));
    }
    sort_by_order(&mut flow, nodes);
    sort_by_order(&mut positioned, nodes);
    let packed_floats = if floated.is_empty() {
        PackedFloats::default()
    } else {
        pack_floated_children(
            &floated,
            content_origin,
            content,
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            scope,
        )?
    };
    let float_left_bottom = packed_floats.left_bottom;
    let float_right_bottom = packed_floats.right_bottom;
    let grid_2d = uses_2d_grid(style, &flow, nodes);
    let ifc = !grid_2d
        && !style
            .display
            .is_some_and(|d| d.is_flex_container() || d.is_grid_container())
        && flow
            .iter()
            .any(|id| nodes.style(*id).is_some_and(|s| s.is_inline_level()));
    let direction = used_flow_direction(style, ifc);
    let rtl_inline = style.is_rtl() && !style.resolved_writing_mode().is_vertical();
    let reverse_main = !grid_2d
        && !ifc
        && if direction.is_row() {
            let block_rev = style.resolved_writing_mode().block_start_is_right();
            style.flex_reverse != (rtl_inline || block_rev)
        } else {
            style.flex_reverse
        };
    if reverse_main {
        flow.reverse();
        positioned.reverse();
    }
    let parent_box = gap_containing_block(style, content);
    let gap = style.main_gap_against_fonts(direction, parent_box, fonts);
    let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
    let mut child_sizes = Vec::with_capacity(flow.len());
    for child in &flow {
        let child_available = nodes
            .style(*child)
            .filter(|_| grid_2d)
            .map(|child_style| grid_item_measure_available(child_style.as_ref(), content))
            .unwrap_or(content);
        child_sizes.push(intrinsic_size_scoped(
            *child,
            child_available,
            Some(direction),
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            scope,
        )?);
    }
    if grid_2d {
        let grid = layout_grid_2d(
            style,
            &flow,
            &child_sizes,
            content,
            fonts,
            nodes,
            inherited_grid,
        );
        place_grid_2d_items(
            &grid,
            content_origin,
            content,
            style,
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
    } else {
        let wrap = if ifc { FlexWrap::Wrap } else { style.flex_wrap };
        let wrapping = ifc
            || match direction {
                FlexDirection::Row => matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse),
                FlexDirection::Column => {
                    matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse) && content.height > 0.5
                }
            };
        let grid_tracks = match direction {
            FlexDirection::Row => style.active_grid_columns(),
            FlexDirection::Column => style.active_grid_rows(),
        };
        let mut justify = if ifc {
            ifc_justify(
                style.text_align,
                style.is_rtl(),
                style.resolved_writing_mode(),
            )
        } else {
            style.justify_content
        };
        if reverse_main {
            justify = flip_justify_for_reverse(justify);
        }
        let full_main = main_extent(content, direction);
        let mut line_slots = if wrapping {
            if ifc && style.resolved_writing_mode().is_horizontal() {
                pack_ifc_line_boxes(
                    &flow,
                    &child_sizes,
                    content_origin,
                    content.width,
                    gap,
                    cross_gap,
                    viewport,
                    child_font_px,
                    nodes,
                    &packed_floats,
                )
            } else {
                pack_wrap_lines(
                    &flow,
                    &child_sizes,
                    direction,
                    full_main,
                    gap,
                    grid_tracks,
                    viewport,
                    child_font_px,
                    nodes,
                    ifc,
                )
                .into_iter()
                .map(|indices| LineBoxSlot {
                    indices,
                    main_start: 0.0,
                    main_available: full_main,
                    cross_y: 0.0,
                    pin_cross: false,
                })
                .collect()
            }
        } else {
            vec![LineBoxSlot {
                indices: (0..flow.len()).collect(),
                main_start: 0.0,
                main_available: full_main,
                cross_y: 0.0,
                pin_cross: false,
            }]
        };
        if matches!(wrap, FlexWrap::WrapReverse) {
            line_slots.reverse();
        }
        let mut packed: Vec<(Vec<StableNodeId>, Vec<Size>, f32, f32, f32, f32, bool)> =
            Vec::with_capacity(line_slots.len());
        for slot in &line_slots {
            let mut line_flow: Vec<StableNodeId> =
                slot.indices.iter().map(|&index| flow[index]).collect();
            let mut line_sizes: Vec<Size> = slot
                .indices
                .iter()
                .map(|&index| child_sizes[index])
                .collect();
            if ifc && rtl_inline {
                line_flow.reverse();
                line_sizes.reverse();
            }
            let mut line_content = content;
            set_main_extent(&mut line_content, direction, slot.main_available);
            let line_tracks = grid_tracks.map(|tracks| {
                let start = slot.indices.first().copied().unwrap_or(0);
                let end = slot
                    .indices
                    .last()
                    .map(|index| index + 1)
                    .unwrap_or(0)
                    .min(tracks.len());
                let start = start.min(end);
                &tracks[start..end]
            });
            if let Some(tracks) = line_tracks.filter(|tracks| !tracks.is_empty()) {
                apply_grid_main_sizes(
                    &line_flow,
                    &mut line_sizes,
                    direction,
                    line_content,
                    gap,
                    tracks,
                    viewport,
                    child_font_px,
                    nodes,
                    intrinsic,
                    scope,
                )?;
            } else {
                distribute_flex_main(
                    &line_flow,
                    &mut line_sizes,
                    direction,
                    line_content,
                    gap,
                    viewport,
                    child_font_px,
                    nodes,
                );
            }
            let line_cross = line_flow
                .iter()
                .zip(line_sizes.iter())
                .map(|(child, size)| {
                    let margin = nodes
                        .style(*child)
                        .map(|style| {
                            style.resolved_margin_against_fonts(
                                Some(content.width),
                                fonts_of(style.as_ref(), child_font_px),
                            )
                        })
                        .unwrap_or_default();
                    cross_extent(*size, direction) + cross_margin(margin, direction)
                })
                .fold(0.0, f32::max);
            packed.push((
                line_flow,
                line_sizes,
                line_cross,
                slot.main_start,
                slot.main_available,
                slot.cross_y,
                slot.pin_cross,
            ));
        }
        let from_block_end = pack_block_from_end(style, direction);
        if from_block_end {
            packed.reverse();
        }
        let align_content = if from_block_end {
            flip_justify_for_reverse(style.align_content)
        } else {
            style.align_content
        };
        let line_count = packed.len();
        let container_cross = cross_extent(content, direction);
        let (mut cross_cursor, extra_cross_gap) = if line_count > 1 {
            let total = packed
                .iter()
                .map(|(_, _, cross, _, _, _, _)| *cross)
                .sum::<f32>()
                + cross_gap * line_count.saturating_sub(1) as f32;
            if matches!(align_content, JustifySpec::Stretch | JustifySpec::Start)
                && align_content == JustifySpec::Stretch
            {
                let leftover = (container_cross - total).max(0.0);
                let extra = leftover / line_count as f32;
                for packed_line in &mut packed {
                    packed_line.2 += extra;
                }
                (0.0, cross_gap)
            } else {
                justify_offsets(align_content, container_cross, total, cross_gap, line_count)
            }
        } else if from_block_end {
            let line_cross = packed
                .first()
                .map(|(_, _, cross, _, _, _, _)| *cross)
                .unwrap_or(0.0);
            ((container_cross - line_cross).max(0.0), cross_gap)
        } else {
            (0.0, cross_gap)
        };
        for (
            line_flow,
            line_sizes,
            line_cross,
            line_origin_main,
            line_main_available,
            line_cross_y,
            pin_cross,
        ) in packed
        {
            if pin_cross {
                cross_cursor = cross_cursor.max(line_cross_y);
            }
            let occupied = main_occupied(
                &line_flow,
                &line_sizes,
                direction,
                content,
                gap,
                child_font_px,
                nodes,
            );
            let auto_main = count_auto_main_margins(&line_flow, direction, nodes);
            let (mut cursor, effective_gap, auto_main_share) = if auto_main > 0 {
                let free = (line_main_available - occupied).max(0.0);
                (0.0, gap, free / auto_main as f32)
            } else {
                let (start, extra_gap) =
                    justify_offsets(justify, line_main_available, occupied, gap, line_flow.len());
                (start, extra_gap, 0.0)
            };
            let line_baseline = line_flow
                .iter()
                .filter_map(|id| {
                    nodes
                        .style(*id)
                        .map(|s| s.baseline_from_ascent(child_font_px, nodes.text_ascent(*id)))
                })
                .fold(0.0f32, f32::max);
            for (child, mut child_size) in line_flow.into_iter().zip(line_sizes) {
                let Some(child_style) = nodes.style(child) else {
                    continue;
                };
                let child_style = child_style.as_ref();
                let child_fonts = fonts_of(child_style, child_font_px);
                let clear_y =
                    clear_offset(child_style.clear, float_left_bottom, float_right_bottom);
                if clear_y > 0.0 {
                    if direction.is_column() {
                        cursor = cursor.max(clear_y);
                    } else {
                        cross_cursor = cross_cursor.max(clear_y);
                    }
                }
                let mut margin =
                    child_style.resolved_margin_against_fonts(Some(content.width), child_fonts);
                let line_box_cross = if line_count > 1 {
                    line_cross
                } else {
                    container_cross
                };
                apply_auto_margins(
                    child_style,
                    direction,
                    &mut margin,
                    auto_main_share,
                    line_box_cross,
                    child_size,
                );
                let align = child_style.resolved_align_self(style.align_items);
                let cross_available = line_box_cross - cross_margin(margin, direction);
                if align == AlignSpec::Stretch && !cross_axis_is_definite(child_style, direction) {
                    set_cross_extent(&mut child_size, direction, cross_available.max(0.0));
                }
                fill_auto_height_from_aspect_ratio(
                    child_style,
                    &mut child_size,
                    Some(content.width),
                    child_fonts,
                );
                let cross_offset = match align {
                    AlignSpec::Start | AlignSpec::Stretch => {
                        cross_cursor + cross_start_margin(margin, direction)
                    }
                    AlignSpec::Baseline => {
                        let base = child_style
                            .baseline_from_ascent(child_fonts.element_px, nodes.text_ascent(child));
                        cross_cursor + (line_baseline - base).max(0.0)
                    }
                    AlignSpec::Center => {
                        cross_cursor
                            + ((line_box_cross - cross_extent(child_size, direction)) / 2.0)
                                .max(0.0)
                    }
                    AlignSpec::End => {
                        cross_cursor
                            + (line_box_cross
                                - cross_extent(child_size, direction)
                                - cross_end_margin(margin, direction))
                            .max(0.0)
                    }
                };
                let main_start = line_origin_main + cursor + main_start_margin(margin, direction);
                let child_origin = match direction {
                    FlexDirection::Row => Point {
                        x: content_origin.x + main_start,
                        y: content_origin.y + cross_offset,
                    },
                    FlexDirection::Column => Point {
                        x: content_origin.x + cross_offset,
                        y: content_origin.y + main_start,
                    },
                };
                if !subtree_unchanged(
                    child,
                    child_origin,
                    child_size,
                    content,
                    child_style,
                    child_fonts,
                    scope,
                ) {
                    place_node_scoped(
                        child,
                        child_origin,
                        child_size,
                        content,
                        viewport,
                        child_font_px,
                        nodes,
                        intrinsic,
                        output,
                        scope,
                        None,
                    )?;
                }
                cursor += main_extent(child_size, direction)
                    + main_start_margin(margin, direction)
                    + main_end_margin(margin, direction)
                    + effective_gap;
            }
            cross_cursor += line_cross + extra_cross_gap;
        }
    }
    for packed in &packed_floats.items {
        let Some(child_style) = nodes.style(packed.id) else {
            continue;
        };
        let child_style = child_style.as_ref();
        let child_fonts = fonts_of(child_style, child_font_px);
        if !subtree_unchanged(
            packed.id,
            packed.origin,
            packed.size,
            content,
            child_style,
            child_fonts,
            scope,
        ) {
            place_node_scoped(
                packed.id,
                packed.origin,
                packed.size,
                content,
                viewport,
                child_font_px,
                nodes,
                intrinsic,
                output,
                scope,
                None,
            )?;
        }
    }
    for child in positioned {
        let Some(child_style) = nodes.style(child) else {
            continue;
        };
        let child_style = child_style.as_ref();
        let base = if child_style.position == PositionSpec::Fixed {
            Size::new(viewport.width, viewport.height)
        } else {
            content
        };
        let base_origin = if child_style.position == PositionSpec::Fixed {
            Point::ZERO
        } else {
            content_origin
        };
        let mut child_size = intrinsic_size_scoped(
            child,
            base,
            None,
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            scope,
        )?;
        let child_fonts = fonts_of(child_style, child_font_px);
        let left =
            LayoutStyle::resolve_inset_fonts(child_style.offset_left, base.width, child_fonts);
        let right =
            LayoutStyle::resolve_inset_fonts(child_style.offset_right, base.width, child_fonts);
        let top =
            LayoutStyle::resolve_inset_fonts(child_style.offset_top, base.height, child_fonts);
        let bottom =
            LayoutStyle::resolve_inset_fonts(child_style.offset_bottom, base.height, child_fonts);
        if let (Some(left), Some(right)) = (left, right)
            && !child_style
                .width
                .is_some_and(LengthSpec::is_definite_declared)
        {
            child_size.width = (base.width - left - right).max(0.0);
        }
        if let (Some(top), Some(bottom)) = (top, bottom)
            && !child_style
                .height
                .is_some_and(LengthSpec::is_definite_declared)
        {
            child_size.height = (base.height - top - bottom).max(0.0);
        }
        let vp = Some((viewport.width, viewport.height));
        child_size.width = child_size.width.max(child_style.resolved_min_width_fonts(
            Some(base.width),
            vp,
            child_fonts,
        ));
        if let Some(max) = child_style.resolved_max_width_fonts(Some(base.width), vp, child_fonts) {
            child_size.width = child_size.width.min(max);
        }
        child_size.height = child_size.height.max(child_style.resolved_min_height_fonts(
            Some(base.height),
            vp,
            child_fonts,
        ));
        if let Some(max) = child_style.resolved_max_height_fonts(Some(base.height), vp, child_fonts)
        {
            child_size.height = child_size.height.min(max);
        }
        let child_origin = Point {
            x: base_origin.x
                + left.unwrap_or_else(|| {
                    right.map_or(0.0, |value| base.width - value - child_size.width)
                }),
            y: base_origin.y
                + top.unwrap_or_else(|| {
                    bottom.map_or(0.0, |value| base.height - value - child_size.height)
                }),
        };
        if !subtree_unchanged(
            child,
            child_origin,
            child_size,
            base,
            child_style,
            child_fonts,
            scope,
        ) {
            place_node_scoped(
                child,
                child_origin,
                child_size,
                base,
                viewport,
                child_font_px,
                nodes,
                intrinsic,
                output,
                scope,
                None,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn place_modal_children(
    _id: StableNodeId,
    origin: Point,
    size: Size,
    modal: &crate::ModalLayoutInput,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    let has_close = modal.slots.close_action.is_some();
    let has_footer = modal.slots.footer.is_some() || !modal.slots.actions.is_empty();
    let chrome = crate::overlay_surfaces::ModalChrome::measure(
        modal.kind,
        modal.title,
        modal.description,
        has_close,
        has_footer,
    );
    let body_copy = modal.body_text.map_or(0.0, |metrics| metrics.height);
    let body_gap = if body_copy > 0.0 && modal.slots.body.is_some() {
        8.0
    } else {
        0.0
    };
    let root = LayoutBox {
        x: origin.x,
        y: origin.y,
        width: size.width,
        height: size.height,
    };
    let surface = match modal.kind {
        crate::ModalSurfaceKind::Dialog(_) | crate::ModalSurfaceKind::Confirm(_) => {
            let provisional = crate::overlay_surfaces::modal_surface_bounds(root, modal.kind, None);
            let body_available = Size::new(
                (provisional.width - chrome.pad_x * 2.0).max(0.0),
                (provisional.height
                    - chrome.header_height
                    - chrome.body_pad_top
                    - chrome.body_pad_bottom
                    - chrome.footer_height
                    - body_copy
                    - body_gap)
                    .max(0.0),
            );
            let body_slot = if let Some(id) = modal.slots.body {
                if nodes.get(id)?.is_some() {
                    intrinsic_size_scoped(
                        id,
                        body_available,
                        Some(FlexDirection::Column),
                        viewport,
                        parent_font_px,
                        nodes,
                        intrinsic,
                        scope,
                    )?
                    .height
                    .min(body_available.height)
                } else {
                    0.0
                }
            } else {
                0.0
            };
            crate::overlay_surfaces::modal_surface_bounds(
                root,
                modal.kind,
                Some(chrome.chrome_height(body_copy + body_gap + body_slot)),
            )
        }
        _ => crate::overlay_surfaces::modal_surface_bounds(root, modal.kind, None),
    };
    let body = chrome.body_box(surface);
    let slot_y = body.y
        + if body_copy > 0.0 {
            body_copy + body_gap
        } else {
            0.0
        };
    let slot_height = (body.y + body.height - slot_y).max(0.0);
    if let Some(id) = modal.slots.body
        && nodes.get(id)?.is_some()
    {
        place_modal_slot(
            id,
            Point {
                x: body.x,
                y: slot_y,
            },
            Size::new(body.width, slot_height),
            Size::new(body.width, slot_height),
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
    }
    if let Some(id) = modal.slots.close_action
        && nodes.get(id)?.is_some()
    {
        let close = chrome.close_box(surface, modal.kind);
        place_modal_slot(
            id,
            Point {
                x: close.x,
                y: close.y,
            },
            Size::new(close.width, close.height),
            Size::new(close.width, close.height),
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
    }
    let footer_y = surface.y + surface.height - chrome.footer_height;
    let action_band = match modal.kind {
        crate::ModalSurfaceKind::Drawer(_) => crate::overlay_surfaces::DRAWER_FOOTER_PAD_Y,
        _ => 0.0,
    };
    let mut action_right = surface.x + surface.width - chrome.pad_x;
    let mut actions = Vec::new();
    for id in modal.slots.actions.iter().rev().copied() {
        if nodes.get(id)?.is_some() {
            actions.push(id);
        }
    }
    for id in actions {
        let measured = intrinsic_size_scoped(
            id,
            Size::new(body.width, crate::overlay_surfaces::MODAL_ACTION_HEIGHT),
            Some(FlexDirection::Row),
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            scope,
        )?;
        let action_size = Size::new(
            measured.width.min(body.width),
            measured
                .height
                .min(crate::overlay_surfaces::MODAL_ACTION_HEIGHT),
        );
        action_right -= action_size.width;
        place_modal_slot(
            id,
            Point {
                x: action_right,
                y: footer_y + action_band,
            },
            action_size,
            Size::new(body.width, chrome.footer_height),
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
        action_right -= crate::overlay_surfaces::MODAL_ACTION_GAP;
    }
    if let Some(id) = modal.slots.footer
        && nodes.get(id)?.is_some()
    {
        let width = (action_right - (surface.x + chrome.pad_x)).max(0.0);
        place_modal_slot(
            id,
            Point {
                x: surface.x + chrome.pad_x,
                y: footer_y,
            },
            Size::new(width, chrome.footer_height),
            Size::new(width, chrome.footer_height),
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn place_modal_slot(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    let Some(child_style) = nodes.get(id)?.map(|node| node.style.clone()) else {
        return Ok(());
    };
    if subtree_unchanged(
        id,
        origin,
        size,
        containing,
        child_style.as_ref(),
        fonts_of(child_style.as_ref(), parent_font_px),
        scope,
    ) {
        return Ok(());
    }
    place_node_scoped(
        id,
        origin,
        size,
        containing,
        viewport,
        parent_font_px,
        nodes,
        intrinsic,
        output,
        scope,
        None,
    )
}
