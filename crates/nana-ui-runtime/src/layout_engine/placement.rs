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
/// What re-checking the change closure found in a container's cached plan.
enum PlanCheck {
    /// Nothing the closure reaches changed: every cached position still holds.
    Unchanged,
    /// The lowest entry index whose style or intrinsic size moved. Children
    /// before it keep their positions; from here on the container must replay.
    ChangedFrom(usize),
}

/// Re-check only the children the change closure reaches.
///
/// Everything outside it is unchanged by construction: a layout-affecting
/// `set_style` marks the node LAYOUT-dirty, which is what puts it in the
/// closure, and the caller has already confirmed via
/// `UiWorld::children_layout_style_is_local` that no ancestor-derived
/// adjustment can move a child's style without touching the child.
///
/// The style is compared as well as the intrinsic size, because an edit can
/// move a child without resizing it -- `margin`, `align_self`, `order`,
/// `flex_grow` all change the placement while the intrinsic stays put.
fn check_plan_children(
    plan: &ContainerPlan,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
    scope: &ScopeContext<'_>,
) -> Result<PlanCheck, UiWorldError> {
    let mut first_changed: Option<usize> = None;
    for index in plan.affected_entries(scope) {
        let index = index as usize;
        let (child, cached_style, cached_intrinsic) = {
            let entries = plan.entries.borrow();
            let entry = &entries[index];
            (entry.child, Arc::clone(&entry.style), entry.intrinsic)
        };
        let Some(current) = nodes.style(child) else {
            return Ok(PlanCheck::ChangedFrom(
                index.min(first_changed.unwrap_or(index)),
            ));
        };
        // Pointer first, value second. A host that rebuilds its style objects
        // every frame (a CSS cascade, say) hands back a fresh `Arc` holding an
        // identical `LayoutStyle`; rejecting on pointer alone would retire the
        // plan on every event and give back all of the saving. The value
        // compare only ever runs for children in the change closure, so it
        // stays off the per-sibling path.
        let style_moved = !Arc::ptr_eq(&current, &cached_style) && current != cached_style;
        let measured = intrinsic_size_scoped(
            child,
            plan.child_available,
            Some(plan.main_direction),
            viewport,
            plan.child_font_px,
            nodes,
            intrinsic,
            Some(scope),
        )?;
        if style_moved || measured != cached_intrinsic {
            first_changed = Some(first_changed.map_or(index, |current| current.min(index)));
        }
    }
    Ok(match first_changed {
        None => PlanCheck::Unchanged,
        Some(index) => PlanCheck::ChangedFrom(index),
    })
}

/// Replay a sequential container's children from `from` onward, keeping every
/// position before it.
///
/// This is the one place that reproduces the container's per-child arithmetic
/// rather than calling into the placement loop, so it is deliberately narrow:
/// `ContainerPlan::sequential` already excluded wrapping, space-distributing
/// justification, reversed flow, grid tracks, auto main margins, non-Start/
/// Stretch cross alignment and any grow/shrink redistribution. Anything this
/// function meets that it cannot express, it refuses by returning `false`, and
/// the caller falls back to a full container relayout.
#[allow(clippy::too_many_arguments)]
fn replay_sequential_suffix(
    id: StableNodeId,
    plan: &ContainerPlan,
    from: usize,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: &ScopeContext<'_>,
) -> Result<bool, UiWorldError> {
    let direction = plan.main_direction;
    let container_align = plan.style.align_items;
    let container_cross = cross_extent(plan.content, direction);
    let count = plan.child_count();
    let mut cursor = plan.entries.borrow()[from].cursor_before;
    let mut replayed: Vec<PlannedChild> = Vec::with_capacity(count - from);

    for index in from..count {
        let (child, cached_style, cached_intrinsic) = {
            let entries = plan.entries.borrow();
            let entry = &entries[index];
            (entry.child, Arc::clone(&entry.style), entry.intrinsic)
        };
        // Outside the closure nothing can have moved, so reuse the cached
        // measurement; inside it, both were already refreshed by the check.
        let (style_arc, child_intrinsic) = if scope.affected.contains(&child) {
            let Some(style) = nodes.style(child) else {
                return Ok(false);
            };
            let measured = intrinsic_size_scoped(
                child,
                plan.child_available,
                Some(direction),
                viewport,
                plan.child_font_px,
                nodes,
                intrinsic,
                Some(scope),
            )?;
            (style, measured)
        } else {
            (cached_style, cached_intrinsic)
        };
        let child_style = style_arc.as_ref();
        // A newly arrived auto margin or exotic alignment makes this container
        // no longer sequential; the caller must relayout it properly.
        let auto_main = match direction {
            FlexDirection::Row => child_style.margin_auto_left() || child_style.margin_auto_right(),
            FlexDirection::Column => {
                child_style.margin_auto_top() || child_style.margin_auto_bottom()
            }
        };
        if auto_main {
            return Ok(false);
        }
        let align = child_style.resolved_align_self(container_align);
        if !matches!(align, AlignSpec::Start | AlignSpec::Stretch) {
            return Ok(false);
        }
        // The replay hands every child its intrinsic main size, which is only
        // the used size while nothing redistributes free space. A child that
        // opts into growing or shrinking does, so refuse it here -- an edit can
        // introduce either one on a child that had neither when the plan was
        // recorded.
        if child_style.flex_grow.unwrap_or(0.0) > 0.0
            || child_style.flex_shrink.unwrap_or(0.0) > 0.0
        {
            return Ok(false);
        }
        let child_fonts = fonts_of(child_style, plan.child_font_px);
        let margin =
            child_style.resolved_margin_against_fonts(Some(plan.content.width), child_fonts);
        let mut child_size = child_intrinsic;
        if align == AlignSpec::Stretch && !cross_axis_is_definite(child_style, direction) {
            let cross_available = container_cross - cross_margin(margin, direction);
            set_cross_extent(&mut child_size, direction, cross_available.max(0.0));
        }
        fill_auto_height_from_aspect_ratio(
            child_style,
            &mut child_size,
            Some(plan.content.width),
            child_fonts,
        );
        let cross_offset = cross_start_margin(margin, direction);
        let main_start = cursor + main_start_margin(margin, direction);
        let child_origin = match direction {
            FlexDirection::Row => Point {
                x: plan.content_origin.x + main_start,
                y: plan.content_origin.y + cross_offset,
            },
            FlexDirection::Column => Point {
                x: plan.content_origin.x + cross_offset,
                y: plan.content_origin.y + main_start,
            },
        };
        let cursor_before = cursor;
        cursor += main_extent(child_size, direction)
            + main_start_margin(margin, direction)
            + main_end_margin(margin, direction)
            + plan.gap;
        if !subtree_unchanged(
            child,
            child_origin,
            child_size,
            plan.content,
            child_style,
            child_fonts,
            Some(scope),
        ) {
            place_node_scoped(
                child,
                child_origin,
                child_size,
                plan.content,
                viewport,
                plan.child_font_px,
                nodes,
                intrinsic,
                output,
                Some(scope),
                None,
            )?;
        }
        replayed.push(PlannedChild {
            child,
            style: style_arc,
            intrinsic: child_intrinsic,
            origin: child_origin,
            size: child_size,
            cursor_before,
        });
    }

    let _ = (id, cursor);
    // Commit only after the whole suffix succeeded, so a bail-out above leaves
    // the cached plan exactly as it was.
    let mut entries = plan.entries.borrow_mut();
    for (slot, entry) in entries[from..].iter_mut().zip(replayed) {
        *slot = entry;
    }
    Ok(true)
}

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
    let style_arc = node.style.clone();
    let child_ids = node.children.clone();
    let modal = node.modal.clone();
    // Only explicit boundaries need a saved placement for independent reflow.
    // Ordinary nodes must not allocate another per-node cache on full layout.
    if style_arc.layout_isolation {
        nodes
            .placements
            .insert(id, (origin, containing, parent_font_px));
    }
    let style = style_arc.as_ref();
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

    let padding = style.resolved_padding_against_fonts(Some(containing.width), fonts);
    nodes.used_padding.insert(id, padding);

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

    // Leaf geometry and used padding are complete. Modal slots above can have
    // their own placement contract; ordinary leaves have no child work.
    if child_ids.is_empty() {
        return Ok(());
    }

    let border = style.resolved_border_edges();
    let content_origin = Point {
        x: origin.x + border.left + padding.left,
        y: origin.y + border.top + padding.top,
    };
    let content = Size::new(
        size.width - padding.left - padding.right - border.left - border.right,
        size.height - padding.top - padding.bottom - border.top - border.bottom,
    );
    // A container whose own inputs and whose children's styles and intrinsic
    // sizes are all unchanged places its children exactly where it did last
    // pass. Reuse that and touch only the children the change closure reaches;
    // otherwise a one-child edit pays a full sibling scan. See `ContainerPlan`.
    if let Some(scope) = scope
        && inherited_grid.is_none()
        && let Some(plan) = scope.retained.container_plans.get(&id)
        && plan.inputs_match(
            origin,
            size,
            containing,
            parent_font_px,
            viewport,
            &style_arc,
            &child_ids,
        )
        && nodes.world.children_layout_style_is_local(id)
        && triggered_menu_overlay(nodes.world, id).is_none()
    {
        match check_plan_children(plan, viewport, nodes, intrinsic, scope)? {
            PlanCheck::Unchanged => {
                #[cfg(test)]
                super::plan_stats::note_plan_reused();
                for index in plan.affected_entries(scope) {
                    let (child, origin, size) = {
                        let entries = plan.entries.borrow();
                        let entry = &entries[index as usize];
                        (entry.child, entry.origin, entry.size)
                    };
                    place_node_scoped(
                        child,
                        origin,
                        size,
                        plan.content,
                        viewport,
                        plan.child_font_px,
                        nodes,
                        intrinsic,
                        output,
                        Some(scope),
                        None,
                    )?;
                }
                return Ok(());
            }
            // Something the closure reaches resized or restyled. Children
            // before it cannot have moved, so a sequential container replays
            // only from there; a tail edit shifts nothing and costs O(1).
            PlanCheck::ChangedFrom(from) if plan.sequential => {
                if replay_sequential_suffix(
                    id, plan, from, viewport, nodes, intrinsic, output, scope,
                )? {
                    #[cfg(test)]
                    super::plan_stats::note_plan_reused();
                    return Ok(());
                }
            }
            PlanCheck::ChangedFrom(_) => {}
        }
    }

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
        // Resolving the child style is a map lookup plus an `Arc` clone, so keep
        // it behind the grid check rather than filtering it away afterwards.
        let child_available = if grid_2d {
            nodes
                .style(*child)
                .map(|child_style| grid_item_measure_available(child_style.as_ref(), content))
                .unwrap_or(content)
        } else {
            content
        };
        #[cfg(test)]
        super::plan_stats::note_child_measured();
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
    // Only the plain in-flow path is cacheable. Grids, inline formatting,
    // floats, out-of-flow children and triggered menu overlays each add
    // placement inputs the plan does not model, and
    // `children_layout_style_is_local` rules out the cases where an ancestor
    // could change a child's style without marking the child dirty.
    // Recorded on full passes too, so the first scoped pass after a mount or a
    // viewport change already has a plan to reuse.
    let cacheable = inherited_grid.is_none()
        && !grid_2d
        && !ifc
        && floated.is_empty()
        && positioned.is_empty()
        && triggered_menu_overlay(nodes.world, id).is_none()
        && nodes.world.children_layout_style_is_local(id);
    let mut plan_entries: Option<Vec<PlannedChild>> =
        cacheable.then(|| Vec::with_capacity(flow.len()));
    // Narrowed to false by anything the suffix replay cannot express.
    let mut plan_sequential = cacheable && !reverse_main;
    let plan_intrinsics: Option<HashMap<StableNodeId, Size>> = cacheable.then(|| {
        flow.iter()
            .copied()
            .zip(child_sizes.iter().copied())
            .collect()
    });
    if !cacheable {
        // Retire a plan recorded while this container was still cacheable.
        nodes.container_plans.insert(id, None);
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
        plan_sequential &= justify == JustifySpec::Start && grid_tracks.is_none();
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
                let Some(child_style_arc) = nodes.style(child) else {
                    continue;
                };
                let child_style = child_style_arc.as_ref();
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
                            + cross_start_margin(margin, direction)
                            + ((cross_available - cross_extent(child_size, direction)) / 2.0)
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
                if let Some(entries) = plan_entries.as_mut()
                    && let Some(intrinsics) = plan_intrinsics.as_ref()
                    && let Some(child_intrinsic) = intrinsics.get(&child).copied()
                {
                    // Anything that couples this child's position to a sibling
                    // other than through the running cursor takes the container
                    // off the sequential path. Grow/shrink is detected from the
                    // data rather than from the style: if the used main size
                    // still equals the intrinsic, no free space was
                    // redistributed.
                    plan_sequential &= line_count == 1
                        && auto_main == 0
                        && main_extent(child_size, direction)
                            == main_extent(child_intrinsic, direction)
                        // `used == intrinsic` proves no space was distributed
                        // THIS pass. A child that can grow may still start
                        // distributing once a later edit frees space up, and
                        // the replay does not model that, so exclude it now.
                        && child_style.flex_grow.unwrap_or(0.0) <= 0.0
                        && child_style.flex_shrink.unwrap_or(0.0) <= 0.0
                        && matches!(
                            child_style.resolved_align_self(style.align_items),
                            AlignSpec::Start | AlignSpec::Stretch
                        );
                    entries.push(PlannedChild {
                        child,
                        style: Arc::clone(&child_style_arc),
                        intrinsic: child_intrinsic,
                        origin: child_origin,
                        size: child_size,
                        cursor_before: cursor,
                    });
                }
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
    if let Some(entries) = plan_entries {
        nodes.container_plans.insert(
            id,
            Some(ContainerPlan {
                content_origin,
                gap,
                sequential: plan_sequential,
                by_child: {
                    let mut by_child: Vec<(StableNodeId, u32)> = entries
                        .iter()
                        .enumerate()
                        .map(|(index, entry)| (entry.child, index as u32))
                        .collect();
                    by_child.sort_unstable_by_key(|(child, _)| *child);
                    by_child
                },
                origin,
                size,
                containing,
                parent_font_px,
                viewport,
                style: Arc::clone(&style_arc),
                children: Arc::clone(&child_ids),
                content,
                child_font_px,
                child_available: content,
                main_direction: direction,
                entries: RefCell::new(entries),
            }),
        );
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
    if let Some(overlay) = triggered_menu_overlay(nodes.world, id) {
        place_triggered_menu_items(
            overlay,
            LayoutBox {
                x: origin.x,
                y: origin.y,
                width: size.width,
                height: size.height,
            },
            &positioned,
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
        return Ok(());
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

fn triggered_menu_overlay(
    world: &crate::UiWorld,
    id: StableNodeId,
) -> Option<crate::TriggeredMenuOverlay> {
    match world.standard_visual(id) {
        Some(crate::StandardVisual::MenuSurface {
            open: true,
            overlay: Some(overlay),
            ..
        }) => Some(overlay),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn place_triggered_menu_items(
    overlay: crate::TriggeredMenuOverlay,
    trigger: LayoutBox,
    items: &[StableNodeId],
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    let inner_width = (overlay.width - overlay.padding * 2.0).max(0.0);
    let available = Size::new(inner_width, viewport.height);
    let mut item_sizes = Vec::with_capacity(items.len());
    let mut content_height = 0.0;
    for (index, child) in items.iter().copied().enumerate() {
        let size = intrinsic_size_scoped(
            child,
            available,
            None,
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            scope,
        )?;
        if index > 0 {
            content_height += crate::popover::MENU_ITEM_GAP;
        }
        content_height += size.height;
        item_sizes.push(size);
    }
    let surface_height = overlay.padding * 2.0 + content_height;
    let viewport_box = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: viewport.width,
        height: viewport.height,
    };
    let (origin_x, origin_y) = crate::popover::resolve_popover_origin(
        trigger,
        overlay.width,
        surface_height,
        viewport_box,
        overlay.placement,
        overlay.alignment,
        overlay.gap,
    );
    let mut cursor_y = origin_y + overlay.padding;
    let item_x = origin_x + overlay.padding;
    for (child, child_size) in items.iter().copied().zip(item_sizes) {
        let Some(child_style) = nodes.style(child) else {
            continue;
        };
        let child_style = child_style.as_ref();
        let child_fonts = fonts_of(child_style, parent_font_px);
        let child_origin = Point {
            x: item_x,
            y: cursor_y,
        };
        if !subtree_unchanged(
            child,
            child_origin,
            child_size,
            available,
            child_style,
            child_fonts,
            scope,
        ) {
            place_node_scoped(
                child,
                child_origin,
                child_size,
                available,
                viewport,
                parent_font_px,
                nodes,
                intrinsic,
                output,
                scope,
                None,
            )?;
        }
        cursor_y += child_size.height + crate::popover::MENU_ITEM_GAP;
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
