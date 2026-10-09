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
    intrinsic: &mut PassIntrinsicCache,
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

/// Direct children of `container` that the closure reaches, including a child
/// that only contains an affected descendant. A fixed row can stay out of its
/// parent's frontier while the label inside it still has to be placed.
fn children_reaching_affected(
    container: StableNodeId,
    plan: &ContainerPlan,
    scope: &ScopeContext<'_>,
) -> Vec<u32> {
    let mut indices = plan.affected_entries(scope);
    indices.extend(
        scope
            .reach
            .children(container)
            .iter()
            .filter_map(|&child| plan.entry_index(child)),
    );
    indices.sort_unstable();
    indices.dedup();
    indices
}

/// After a replay that placed only part of `container`, place the children
/// it did not visit that still lead to an affected node, at their kept
/// boxes: the replay left them where they were, but not what is inside.
#[allow(clippy::too_many_arguments)]
fn place_unvisited_reaching(
    container: StableNodeId,
    plan: &ContainerPlan,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: &ScopeContext<'_>,
) -> Result<(), UiWorldError> {
    for index in children_reaching_affected(container, plan, scope) {
        let (child, origin, size) = {
            let entries = plan.entries.borrow();
            let entry = &entries[index as usize];
            (entry.child, entry.origin, entry.size)
        };
        if output.contains_key(&child) {
            continue;
        }
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
    Ok(())
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
    intrinsic: &mut PassIntrinsicCache,
    scope: &ScopeContext<'_>,
) -> Result<PlanCheck, UiWorldError> {
    let mut first_changed: Option<usize> = None;
    for index in plan.affected_entries(scope) {
        let index = index as usize;
        let entry = plan.entries.borrow()[index].clone();
        let child = entry.child;
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
        let style_moved =
            !Arc::ptr_eq(&current, &entry.style) && !layout_inputs_equal(&current, &entry.style);
        let on_measure = scope.measure.contains(&child);
        let measured = if on_measure {
            intrinsic_size_scoped(
                child,
                plan.child_available,
                Some(plan.main_direction),
                viewport,
                plan.child_font_px,
                nodes,
                intrinsic,
                Some(scope),
            )?
        } else {
            entry.intrinsic
        };
        let at_main_moved = on_measure
            && match entry.at_main {
                Some((main, cached)) => {
                    intrinsic_size_at_main(
                        child,
                        main,
                        plan.main_direction,
                        plan.child_available,
                        viewport,
                        plan.child_font_px,
                        nodes,
                        intrinsic,
                        Some(scope),
                    )? != cached
                }
                None => false,
            };
        // Aligned by a baseline that moved: the line aligns again, though no
        // size changed.
        let baseline_moved = on_measure
            && entry.baseline.is_some_and(|aligned| {
                let font_px = fonts_of(&current, plan.child_font_px).element_px;
                intrinsic
                    .baseline(child, crate::Baseline::First)
                    .unwrap_or_else(|| nodes.baseline(child, font_px, entry.size))
                    .to_bits()
                    != aligned.to_bits()
            });
        if style_moved || measured != entry.intrinsic || at_main_moved || baseline_moved {
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
/// justification, grid tracks, auto main margins, non-Start/Stretch cross
/// alignment and any grow/shrink redistribution. Like the loop it is
/// flow-relative — the cursor and each child's leading margin are read from
/// the start edge — and turns a reversed axis onto the page the same way. Anything this
/// function meets that it cannot express, it refuses by returning `false`, and
/// the caller falls back to a full container relayout.
#[allow(clippy::too_many_arguments)]
fn replay_sequential_suffix(
    id: StableNodeId,
    plan: &ContainerPlan,
    from: usize,
    content: Size,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: &ScopeContext<'_>,
) -> Result<bool, UiWorldError> {
    let direction = plan.main_direction;
    let writing = plan.writing;
    let container_align = plan.style.align_items;
    let container_cross = cross_extent(plan.content, direction);
    // The replay places against this pass's main extent: on a reversed main
    // axis that is where the far edge, and every origin, is measured from.
    let full_main = main_extent(content, direction);
    let (main_reversed, cross_reversed) = (plan.main_reversed, plan.cross_reversed);
    let count = plan.child_count();
    let mut cursor = plan
        .entries
        .borrow()
        .get(from)
        .map_or(0.0, |entry| entry.cursor_before);
    let mut replayed: Vec<PlannedChild> = Vec::with_capacity(count - from);

    for index in from..count {
        let (child, cached_style, cached_intrinsic) = {
            let entries = plan.entries.borrow();
            let entry = &entries[index];
            (entry.child, Arc::clone(&entry.style), entry.intrinsic)
        };
        // Outside the closure nothing can have moved, so reuse the cached
        // measurement; inside it, both were already refreshed by the check.
        let (style_arc, child_intrinsic) = if scope.measure.contains(&child) {
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
        } else if scope.affected.contains(&child) {
            let style = nodes.style(child).unwrap_or(cached_style);
            (style, cached_intrinsic)
        } else {
            (cached_style, cached_intrinsic)
        };
        let child_style = style_arc.as_ref();
        // Leaving the flow (or being omitted) changes sibling coupling the
        // suffix arithmetic does not model. The caller lays the container out
        // again, still inside this formatting context.
        if child_style.position.is_out_of_flow() || child_style.omits_box() {
            return Ok(false);
        }
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
        let margin = child_style.resolved_margin_against_fonts(
            Some(writing.inline_size(plan.content.width, plan.content.height)),
            child_fonts,
        );
        let mut child_size = child_intrinsic;
        if align == AlignSpec::Stretch && !cross_axis_is_definite(child_style, direction) {
            let cross_available = container_cross - cross_margin(margin, direction);
            set_cross_extent(&mut child_size, direction, cross_available.max(0.0));
        }
        fill_auto_height_from_aspect_ratio(
            child_style,
            &mut child_size,
            Some(writing.inline_size(plan.content.width, plan.content.height)),
            child_fonts,
        );
        let (main_lead, main_trail) = if main_reversed {
            (
                main_end_margin(margin, direction),
                main_start_margin(margin, direction),
            )
        } else {
            (
                main_start_margin(margin, direction),
                main_end_margin(margin, direction),
            )
        };
        let cross_lead = if cross_reversed {
            cross_end_margin(margin, direction)
        } else {
            cross_start_margin(margin, direction)
        };
        // Flow-relative to the page, exactly as the placement loop does it.
        let cross_offset = if cross_reversed {
            container_cross - cross_lead - cross_extent(child_size, direction)
        } else {
            cross_lead
        };
        let flow_main = cursor + main_lead;
        let main_start = if main_reversed {
            full_main - flow_main - main_extent(child_size, direction)
        } else {
            flow_main
        };
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
        cursor += main_extent(child_size, direction) + main_lead + main_trail + plan.gap;
        if !subtree_unchanged(
            child,
            child_origin,
            child_size,
            plan.content,
            child_style,
            child_fonts,
            writing,
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
            // Nothing redistributes main sizes on the sequential path.
            at_main: None,
            origin: child_origin,
            size: child_size,
            cursor_before,
            // A sequential line aligns nothing by baseline.
            baseline: None,
        });
    }

    let _ = (id, cursor);
    // A reversed main axis measures every origin back from the far edge. When
    // the container's main size moved, the children before `from` keep their
    // flow positions and move with that edge, all by the same amount: the
    // work is the boxes that really moved, and nothing is measured again.
    let shift = if main_reversed {
        full_main - plan.placed_main.get()
    } else {
        0.0
    };
    let mut shifted = Vec::new();
    if shift != 0.0 {
        let entries = plan.entries.borrow();
        shifted.reserve(from);
        for (index, entry) in entries[..from].iter().enumerate() {
            let mut origin = entry.origin;
            match direction {
                FlexDirection::Row => origin.x += shift,
                FlexDirection::Column => origin.y += shift,
            }
            shifted.push((index, entry.child, origin, entry.size));
        }
    }
    for &(_, child, origin, size) in &shifted {
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
    // Commit only after the whole suffix succeeded, so a bail-out above leaves
    // the cached plan exactly as it was.
    let mut entries = plan.entries.borrow_mut();
    for (index, _, origin, _) in shifted {
        entries[index].origin = origin;
    }
    for (slot, entry) in entries[from..].iter_mut().zip(replayed) {
        *slot = entry;
    }
    plan.placed_main.set(full_main);
    Ok(true)
}

/// Shift later wrap lines when one item's cross size changed and the line
/// breaks did not. Anything else returns `false` so the caller lays out this
/// flex container from scratch.
fn replay_wrapped_flex_line(
    plan: &ContainerPlan,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: &ScopeContext<'_>,
) -> Result<bool, UiWorldError> {
    let style = plan.style.as_ref();
    if plan.sequential || !flex_line_local_style(style) || plan.main_reversed || plan.cross_reversed
    {
        return Ok(false);
    }
    let direction = plan.main_direction;
    let tracks = match direction {
        FlexDirection::Row => style.active_grid_columns(),
        FlexDirection::Column => style.active_grid_rows(),
    };
    if tracks.is_some_and(|tracks| !tracks.is_empty()) {
        return Ok(false);
    }
    let entries = plan.entries.borrow();
    let mut flow = Vec::with_capacity(entries.len());
    let mut old_sizes = Vec::with_capacity(entries.len());
    for entry in entries.iter() {
        if child_blocks_flex_line_local(entry.style.as_ref()) {
            return Ok(false);
        }
        if scope.affected.contains(&entry.child)
            && !nodes.style(entry.child).is_some_and(|current| {
                Arc::ptr_eq(&current, &entry.style)
                    || super::same_but_cross(&current, &entry.style, direction)
            })
        {
            return Ok(false);
        }
        let cross = match direction {
            FlexDirection::Row => entry.style.height,
            FlexDirection::Column => entry.style.width,
        };
        if matches!(cross, Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)) {
            return Ok(false);
        }
        flow.push(entry.child);
        old_sizes.push(entry.intrinsic);
    }
    drop(entries);
    if flow.is_empty() {
        return Ok(false);
    }
    let mut sizes = old_sizes.clone();
    let mut patched = false;
    for (index, child) in flow.iter().copied().enumerate() {
        if !scope.measure.contains(&child) {
            continue;
        }
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
        if main_extent(measured, direction).to_bits()
            != main_extent(old_sizes[index], direction).to_bits()
        {
            return Ok(false);
        }
        if measured != sizes[index] {
            sizes[index] = measured;
            patched = true;
        }
    }
    if !patched {
        return Ok(false);
    }
    let edge_base = plan
        .writing
        .inline_size(plan.content.width, plan.content.height);
    let old_lines = pack_wrap_lines(
        &flow,
        &old_sizes,
        direction,
        plan.content,
        edge_base,
        plan.gap,
        None,
        viewport,
        plan.child_font_px,
        nodes,
        false,
    );
    let new_lines = pack_wrap_lines(
        &flow,
        &sizes,
        direction,
        plan.content,
        edge_base,
        plan.gap,
        None,
        viewport,
        plan.child_font_px,
        nodes,
        false,
    );
    if old_lines != new_lines {
        return Ok(false);
    }
    let line_cross = |line: &[usize], sizes: &[Size]| -> f32 {
        let mut cross = 0.0f32;
        for &index in line {
            let margin = nodes
                .style(flow[index])
                .map(|child_style| {
                    child_style.resolved_margin_against_fonts(
                        Some(edge_base),
                        fonts_of(child_style.as_ref(), plan.child_font_px),
                    )
                })
                .unwrap_or_default();
            cross =
                cross.max(cross_extent(sizes[index], direction) + cross_margin(margin, direction));
        }
        cross
    };
    let mut carried = 0.0f32;
    let mut jobs = Vec::new();
    for line in &new_lines {
        let delta = line_cross(line, &sizes) - line_cross(line, &old_sizes);
        for &index in line {
            let moved = sizes[index] != old_sizes[index];
            if carried == 0.0 && !moved {
                continue;
            }
            let entries = plan.entries.borrow();
            let mut origin = entries[index].origin;
            match direction {
                FlexDirection::Row => origin.y += carried,
                FlexDirection::Column => origin.x += carried,
            }
            jobs.push((flow[index], origin, sizes[index], index));
        }
        carried += delta;
    }
    for (child, origin, size, _) in &jobs {
        place_node_scoped(
            *child,
            *origin,
            *size,
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
    let mut entries = plan.entries.borrow_mut();
    for (child, origin, size, index) in jobs {
        let entry = &mut entries[index];
        entry.child = child;
        entry.origin = origin;
        entry.size = size;
        entry.intrinsic = size;
        if let Some(child_style) = nodes.style(child) {
            entry.style = child_style;
        }
    }
    Ok(true)
}

/// Re-solve this grid's tracks from cached contributions.
///
/// Items whose contribution and cell constraint are unchanged keep their used
/// size. Anything the record cannot describe returns `false`, and the caller
/// lays out this grid formatting context from scratch.
#[allow(clippy::too_many_arguments)]
fn replay_grid(
    id: StableNodeId,
    plan: &ContainerPlan,
    origin: Point,
    size: Size,
    content_origin: Point,
    content: Size,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: &ScopeContext<'_>,
) -> Result<bool, UiWorldError> {
    let Some(old_grid) = plan.grid.clone() else {
        return Ok(false);
    };
    let style = plan.style.as_ref();
    if style.is_subgrid_columns() || style.is_subgrid_rows() {
        return Ok(false);
    }
    let entries = plan.entries.borrow().clone();
    let entry_of = |child: StableNodeId| {
        plan.entry_index(child)
            .and_then(|index| entries.get(index as usize))
            .filter(|entry| entry.child == child)
    };
    let mut flow = Vec::new();
    let mut sizes = Vec::new();
    let content_changed = plan.content != content;
    for child in plan.children.iter().copied() {
        let Some(child_style) = nodes.style(child) else {
            continue;
        };
        if !grid_child_in_flow(child_style.as_ref()) {
            continue;
        }
        let Some(entry) = entry_of(child) else {
            return Ok(false);
        };
        let remeasure = scope.measure.contains(&child)
            || (content_changed && !grid_contribution_ignores_content_box(child_style.as_ref()));
        let contribution = if remeasure {
            let available = grid_item_measure_available(child_style.as_ref(), content);
            intrinsic_size_scoped(
                child,
                available,
                Some(plan.main_direction),
                viewport,
                plan.child_font_px,
                nodes,
                intrinsic,
                Some(scope),
            )?
        } else {
            entry.intrinsic
        };
        flow.push(child);
        sizes.push(contribution);
    }
    if flow.is_empty() {
        return Ok(false);
    }
    sort_ids_with_sizes(&mut flow, &mut sizes, nodes);
    let fonts = fonts_of(style, plan.parent_font_px);
    let solved = layout_grid_2d(
        style,
        plan.writing,
        &flow,
        &sizes,
        content,
        fonts,
        None,
        viewport,
        nodes,
        intrinsic,
        Some(scope),
    )?;
    let previous_items: HashMap<StableNodeId, &GridItemPlan> = old_grid
        .items
        .iter()
        .map(|item| (item.child, item))
        .collect();
    let mut reuse = HashMap::new();
    for item in &solved.items {
        let Some(&previous) = previous_items.get(&item.id) else {
            continue;
        };
        if previous.col != item.col as u32
            || previous.row != item.row as u32
            || previous.col_span != item.col_span as u32
            || previous.row_span != item.row_span as u32
            || previous.contribution != item.intrinsic
        {
            continue;
        }
        let (old_inline, old_block) = old_grid.cell(previous);
        let new_inline =
            grid_span_extent(&solved.col_sizes, item.col, item.col_span, solved.col_gap);
        let new_block =
            grid_span_extent(&solved.row_sizes, item.row, item.row_span, solved.row_gap);
        if old_inline.to_bits() != new_inline.to_bits()
            || old_block.to_bits() != new_block.to_bits()
        {
            continue;
        }
        if scope.measure.contains(&item.id) {
            continue;
        }
        let Some(entry) = entry_of(item.id) else {
            continue;
        };
        let current = nodes.style(item.id);
        let cached = Some(Arc::clone(&entry.style));
        if !retained_style_matches(&current, &cached) || entry.style.aspect_ratio.is_some() {
            continue;
        }
        reuse.insert(item.id, entry.size);
    }
    let mut recorded = Vec::with_capacity(solved.items.len());
    place_grid_2d_items(
        &solved,
        plan.writing,
        content_origin,
        content,
        style,
        viewport,
        plan.child_font_px,
        nodes,
        intrinsic,
        output,
        Some(scope),
        Some(&reuse),
        Some(&mut recorded),
    )?;
    if recorded.len() != solved.items.len() {
        return Ok(false);
    }
    let mut updated = plan.clone();
    updated.origin = origin;
    updated.size = size;
    updated.content = content;
    updated.content_origin = content_origin;
    updated.grid = Some(GridTrackPlan::from_layout(&solved));
    updated.by_child = {
        let mut by_child: Vec<(StableNodeId, u32)> = recorded
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.child, index as u32))
            .collect();
        by_child.sort_unstable_by_key(|(child, _)| *child);
        by_child
    };
    updated.entries = RefCell::new(recorded);
    nodes.container_plans.insert(id, Some(updated));
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
    intrinsic: &mut PassIntrinsicCache,
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
    // The writing mode and direction this node lays out in, inherited from
    // its ancestors when it declares none of its own, and its parent's.
    let (writing, containing_writing) = (node.writing, node.containing_writing);
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
    if let Some(scope) = scope
        && !scope.measure.contains(&id)
        && let Some(previous) = scope.retained.boxes.get(&id)
        && previous.width == size.width
        && previous.height == size.height
        && (previous.x != origin.x || previous.y != origin.y)
    {
        intrinsic.note_origin_only();
    }
    intrinsic.note_placement_node();
    output.insert(
        id,
        LayoutBox {
            x: origin.x,
            y: origin.y,
            width: size.width,
            height: size.height,
        },
    );

    let padding = style.resolved_padding_against_fonts(
        Some(containing_writing.inline_size(containing.width, containing.height)),
        fonts,
    );
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
    let had_plan = scope.is_some_and(|scope| scope.retained.container_plans.contains_key(&id));
    if let Some(scope) = scope
        && inherited_grid.is_none()
        && let Some(plan) = scope.retained.container_plans.get(&id)
        && plan.can_reuse_flow(
            origin,
            size,
            containing,
            content_origin,
            parent_font_px,
            viewport,
            &style_arc,
            &child_ids,
            writing,
        )
        && plan.flow_membership_holds(id, scope, nodes)
        && nodes.world.children_layout_style_is_local(id)
        && triggered_menu_overlay(nodes.world, id).is_none()
    {
        let grid_geometry_moved =
            plan.grid.is_some() && (plan.size != size || plan.content != content);
        // A reversed sequential axis whose main size moved: every cached
        // origin moved with the far edge, even with no child changed.
        let far_edge_moved = plan.sequential
            && plan.main_reversed
            && main_extent(content, plan.main_direction).to_bits()
                != plan.placed_main.get().to_bits();
        let check = check_plan_children(plan, viewport, nodes, intrinsic, scope)?;
        let check = match check {
            PlanCheck::Unchanged if far_edge_moved => PlanCheck::ChangedFrom(plan.child_count()),
            check => check,
        };
        match check {
            PlanCheck::Unchanged if !grid_geometry_moved => {
                // A fixed intermediate box can stay out of the frontier while
                // a descendant inside it is affected. Re-enter the direct
                // child that contains that descendant; its own plan places
                // only the closure.
                for index in children_reaching_affected(id, plan, scope) {
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
                finish_positioned_overlay(
                    id,
                    plan,
                    origin,
                    size,
                    content_origin,
                    content,
                    viewport,
                    child_font_px,
                    writing,
                    nodes,
                    intrinsic,
                    output,
                    scope,
                )?;
                intrinsic.note_placement_plan_reused();
                return Ok(());
            }
            // Something the closure reaches resized or restyled. Children
            // before it cannot have moved, so a sequential container replays
            // only from there; a tail edit shifts nothing and costs O(1).
            PlanCheck::ChangedFrom(from) if plan.sequential => {
                if replay_sequential_suffix(
                    id, plan, from, content, viewport, nodes, intrinsic, output, scope,
                )? {
                    place_unvisited_reaching(id, plan, viewport, nodes, intrinsic, output, scope)?;
                    finish_positioned_overlay(
                        id,
                        plan,
                        origin,
                        size,
                        content_origin,
                        content,
                        viewport,
                        child_font_px,
                        writing,
                        nodes,
                        intrinsic,
                        output,
                        scope,
                    )?;
                    intrinsic.note_placement_plan_reused();
                    intrinsic.note_suffix_replayed();
                    return Ok(());
                }
            }
            PlanCheck::Unchanged | PlanCheck::ChangedFrom(_) => {
                if !grid_geometry_moved
                    && replay_wrapped_flex_line(plan, viewport, nodes, intrinsic, output, scope)?
                {
                    place_unvisited_reaching(id, plan, viewport, nodes, intrinsic, output, scope)?;
                    finish_positioned_overlay(
                        id,
                        plan,
                        origin,
                        size,
                        content_origin,
                        content,
                        viewport,
                        child_font_px,
                        writing,
                        nodes,
                        intrinsic,
                        output,
                        scope,
                    )?;
                    intrinsic.note_placement_plan_reused();
                    return Ok(());
                }
                if replay_grid(
                    id,
                    plan,
                    origin,
                    size,
                    content_origin,
                    content,
                    viewport,
                    nodes,
                    intrinsic,
                    output,
                    scope,
                )? {
                    place_unvisited_reaching(id, plan, viewport, nodes, intrinsic, output, scope)?;
                    finish_positioned_overlay(
                        id,
                        plan,
                        origin,
                        size,
                        content_origin,
                        content,
                        viewport,
                        child_font_px,
                        writing,
                        nodes,
                        intrinsic,
                        output,
                        scope,
                    )?;
                    intrinsic.note_placement_plan_reused();
                    return Ok(());
                }
            }
        }
    }

    if had_plan {
        intrinsic.note_plan_miss();
    }
    let (mut flow, descendant_dependent_flow) =
        collect_flow_children_reporting(&child_ids, nodes, style.display)?;
    if let Some(scope) = scope {
        retire_omitted_children(&child_ids, nodes, output, scope);
    }
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
    // This container is its children's containing block: their percentage
    // margins resolve against its inline size, whatever a line or a float
    // leaves them.
    let child_edge_base = writing.inline_size(content.width, content.height);
    let packed_floats = if floated.is_empty() {
        PackedFloats::default()
    } else {
        pack_floated_children(
            &floated,
            content_origin,
            content,
            child_edge_base,
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
    let direction = used_flow_direction(style, writing, ifc);
    // Flow-relative placement. Every position below is measured from the
    // main-start and cross-start edges of the content box, in flow order —
    // lines filled first item first, `justify-content` / `align-items` /
    // `align-content` read as authored — and turned onto the page once, where
    // a child's origin is written. A reversed axis is only that last step:
    // an RTL inline axis (the right, or the bottom of a vertical one),
    // `vertical-rl`'s block axis from the right, or `flex-direction: *-reverse`.
    // Nothing else knows the page is turned: no list is reversed, no
    // alignment keyword flipped, no line packed from the other end.
    let (main_reversed, cross_reversed) =
        flow_axes_reversed(style, writing, direction, ifc, grid_2d);
    nodes.far_start.insert(
        id,
        page_far_start(writing, direction, grid_2d, (main_reversed, cross_reversed)),
    );
    // A triggered menu lays out its positioned items in list order, which was
    // the reversed flow order before placement went flow-relative; keep it.
    if main_reversed && !ifc {
        positioned.reverse();
    }
    let parent_box = gap_containing_block(style, content);
    let gap = style.main_gap_against_fonts(direction, parent_box, fonts);
    let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
    let mut child_sizes = Vec::with_capacity(flow.len());
    #[cfg(feature = "benchmark")]
    let mut child_phase = (flow.len() > 64).then(super::plan_stats::PhaseClock::start);
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
        intrinsic.note_child_measured();
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
    #[cfg(feature = "benchmark")]
    if let Some(clock) = child_phase.as_mut() {
        clock.lap(6);
    }
    // Plain in-flow containers and 2D grids are cacheable. Positioned
    // children are recorded beside the flow plan and do not by themselves
    // make the container uncacheable. Inline formatting, floats, subgrids
    // and triggered menu overlays each add placement inputs the plan does
    // not model, and `children_layout_style_is_local` rules out the cases
    // where an ancestor could change a child's style without marking the
    // child dirty.
    // `descendant_dependent_flow` rules out the containers whose flow list is
    // not decided by the direct children's own styles: `display:contents`
    // splices grandchildren in, and an inline-level child is unboxed or not
    // depending on its own subtree. Either way the plan's `by_child` index
    // cannot answer "is this affected id one of my entries?".
    // Recorded on full passes too, so the first scoped pass after a mount or a
    // viewport change already has a plan to reuse.
    let grid_recordable = grid_2d
        && inherited_grid.is_none()
        && !style.is_subgrid_columns()
        && !style.is_subgrid_rows();
    let cacheable = inherited_grid.is_none()
        && !descendant_dependent_flow
        && (grid_recordable || !grid_2d)
        && !ifc
        && floated.is_empty()
        && triggered_menu_overlay(nodes.world, id).is_none()
        && nodes.world.children_layout_style_is_local(id);
    let mut plan_entries: Option<Vec<PlannedChild>> =
        cacheable.then(|| Vec::with_capacity(flow.len()));
    // Narrowed to false by anything the suffix replay cannot express.
    let mut plan_sequential = cacheable;
    let mut cross_independent = cacheable;
    let mut main_dependent = false;
    let plan_intrinsics: Option<HashMap<StableNodeId, Size>> = cacheable.then(|| {
        flow.iter()
            .copied()
            .zip(child_sizes.iter().copied())
            .collect()
    });
    if !cacheable {
        intrinsic.note_container_uncacheable();
        // Retire a plan recorded while this container was still cacheable.
        nodes.container_plans.insert(id, None);
    }

    let mut recorded_grid = None;
    if grid_2d {
        let grid = layout_grid_2d(
            style,
            writing,
            &flow,
            &child_sizes,
            content,
            fonts,
            inherited_grid,
            viewport,
            nodes,
            intrinsic,
            scope,
        )?;
        if grid_recordable {
            recorded_grid = Some(GridTrackPlan::from_layout(&grid));
        }
        // Grid placement is not a sequential cursor. Keep the recorded entries
        // so a later pass can re-solve tracks without measuring every cell.
        plan_sequential = false;
        place_grid_2d_items(
            &grid,
            writing,
            content_origin,
            content,
            style,
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            output,
            scope,
            None,
            plan_entries.as_mut(),
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
        let justify = if ifc {
            ifc_justify(style.text_align, writing)
        } else {
            style.justify_content
        };
        plan_sequential &= justify == JustifySpec::Start && grid_tracks.is_none();
        let full_main = main_extent(content, direction);
        let mut line_slots = if wrapping {
            if ifc && !writing.is_vertical() {
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
                    content,
                    child_edge_base,
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
        // Items measured again at the main size their line gave them.
        let mut hypothetical: HashMap<StableNodeId, (f32, Size)> = HashMap::new();
        for slot in &line_slots {
            let line_flow: Vec<StableNodeId> =
                slot.indices.iter().map(|&index| flow[index]).collect();
            let mut line_sizes: Vec<Size> = slot
                .indices
                .iter()
                .map(|&index| child_sizes[index])
                .collect();
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
                    child_edge_base,
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
                    child_edge_base,
                    gap,
                    viewport,
                    child_font_px,
                    nodes,
                );
            }
            // An item the line gave another main size than it was measured
            // with is as tall (wide, in a column) as its content at that size.
            for (slot_index, child) in line_flow.iter().enumerate() {
                let measured = child_sizes[slot.indices[slot_index]];
                let Some(child_style) = nodes.style(*child) else {
                    continue;
                };
                if !cross_follows_used_main(
                    &child_style,
                    direction,
                    measured,
                    line_sizes[slot_index],
                ) {
                    continue;
                }
                let main = main_extent(line_sizes[slot_index], direction);
                let at_main = intrinsic_size_at_main(
                    *child,
                    main,
                    direction,
                    content,
                    viewport,
                    child_font_px,
                    nodes,
                    intrinsic,
                    scope,
                )?;
                set_cross_extent(
                    &mut line_sizes[slot_index],
                    direction,
                    cross_extent(at_main, direction),
                );
                hypothetical.insert(*child, (main, at_main));
            }
            let line_cross = line_flow
                .iter()
                .zip(line_sizes.iter())
                .map(|(child, size)| {
                    let margin = nodes
                        .style(*child)
                        .map(|style| {
                            style.resolved_margin_against_fonts(
                                Some(writing.inline_size(content.width, content.height)),
                                fonts_of(style.as_ref(), child_font_px),
                            )
                        })
                        .unwrap_or_default();
                    cross_extent(*size, direction) + cross_margin(margin, direction)
                })
                .fold(0.0, f32::max);
            // A float-narrowed line's start, measured from main-start: its
            // physical left offset, or the room its right end leaves when the
            // main axis runs from the right.
            let line_start = if main_reversed {
                full_main - slot.main_start - slot.main_available
            } else {
                slot.main_start
            };
            packed.push((
                line_flow,
                line_sizes,
                line_cross,
                line_start,
                slot.main_available,
                slot.cross_y,
                slot.pin_cross,
            ));
        }
        let align_content = style.align_content;
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
        } else {
            (0.0, cross_gap)
        };
        // A child's margins on the side its axis starts from, and on the
        // other: on a reversed axis the leading margin is the physical end one.
        let main_lead = |margin| {
            if main_reversed {
                main_end_margin(margin, direction)
            } else {
                main_start_margin(margin, direction)
            }
        };
        let main_trail = |margin| {
            if main_reversed {
                main_start_margin(margin, direction)
            } else {
                main_end_margin(margin, direction)
            }
        };
        let cross_lead = |margin| {
            if cross_reversed {
                cross_end_margin(margin, direction)
            } else {
                cross_start_margin(margin, direction)
            }
        };
        let cross_trail = |margin| {
            if cross_reversed {
                cross_start_margin(margin, direction)
            } else {
                cross_end_margin(margin, direction)
            }
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
                child_edge_base,
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
            let needs_baseline = style.align_items == AlignSpec::Baseline
                || line_flow.iter().any(|id| {
                    nodes.style(*id).is_some_and(|child| {
                        child.resolved_align_self(style.align_items) == AlignSpec::Baseline
                    })
                });
            let line_baseline = if needs_baseline {
                line_flow
                    .iter()
                    .zip(line_sizes.iter())
                    .filter_map(|(id, size)| {
                        nodes.style(*id).map(|style| {
                            let font_px = fonts_of(&style, child_font_px).element_px;
                            intrinsic
                                .baseline(*id, crate::Baseline::First)
                                .unwrap_or_else(|| nodes.baseline(*id, font_px, *size))
                                .max(style.resolved_border_width())
                        })
                    })
                    .fold(0.0f32, f32::max)
            } else {
                0.0
            };
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
                let mut margin = child_style.resolved_margin_against_fonts(
                    Some(writing.inline_size(content.width, content.height)),
                    child_fonts,
                );
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
                    Some(writing.inline_size(content.width, content.height)),
                    child_fonts,
                );
                let aligned_baseline = (align == AlignSpec::Baseline).then(|| {
                    intrinsic
                        .baseline(child, crate::Baseline::First)
                        .unwrap_or_else(|| {
                            nodes.baseline(child, child_fonts.element_px, child_size)
                        })
                });
                let cross_offset = match align {
                    AlignSpec::Start | AlignSpec::Stretch => cross_cursor + cross_lead(margin),
                    AlignSpec::Baseline => {
                        let base = aligned_baseline.unwrap_or_default();
                        cross_cursor + (line_baseline - base).max(0.0)
                    }
                    AlignSpec::Center => {
                        cross_cursor
                            + cross_lead(margin)
                            + ((cross_available - cross_extent(child_size, direction)) / 2.0)
                                .max(0.0)
                    }
                    AlignSpec::End => {
                        cross_cursor
                            + (line_box_cross
                                - cross_extent(child_size, direction)
                                - cross_trail(margin))
                            .max(0.0)
                    }
                };
                // Flow-relative to the page: a reversed axis measures back
                // from its far edge.
                let flow_main = line_origin_main + cursor + main_lead(margin);
                let main_start = if main_reversed {
                    full_main - flow_main - main_extent(child_size, direction)
                } else {
                    flow_main
                };
                let cross_offset = if cross_reversed {
                    container_cross - cross_offset - cross_extent(child_size, direction)
                } else {
                    cross_offset
                };
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
                    let align = child_style.resolved_align_self(style.align_items);
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
                        && matches!(align, AlignSpec::Start | AlignSpec::Stretch);
                    cross_independent &= matches!(align, AlignSpec::Start | AlignSpec::Stretch)
                        && !reads_container_size(child_style, direction);
                    main_dependent |= reads_main_extent(child_style, direction);
                    entries.push(PlannedChild {
                        child,
                        style: Arc::clone(&child_style_arc),
                        intrinsic: child_intrinsic,
                        at_main: hypothetical.get(&child).copied(),
                        origin: child_origin,
                        size: child_size,
                        cursor_before: cursor,
                        baseline: aligned_baseline,
                    });
                }
                if !subtree_unchanged(
                    child,
                    child_origin,
                    child_size,
                    content,
                    child_style,
                    child_fonts,
                    writing,
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
                    + main_lead(margin)
                    + main_trail(margin)
                    + effective_gap;
            }
            cross_cursor += line_cross + extra_cross_gap;
        }
    }
    #[cfg(feature = "benchmark")]
    if let Some(clock) = child_phase.as_mut() {
        clock.lap(7);
    }
    if let Some(entries) = plan_entries {
        if had_plan {
            intrinsic.note_plan_rebuilt();
        }
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
                writing,
                main_reversed,
                cross_reversed,
                entries: RefCell::new(entries),
                placed_main: Cell::new(main_extent(content, direction)),
                grid: recorded_grid,
                cross_independent,
                main_dependent,
                overlay: Vec::new(),
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
            writing,
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
            id,
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
    let mut recorded_overlay = Vec::with_capacity(positioned.len());
    for child in positioned {
        if let Some(entry) = place_positioned_child(
            child,
            content_origin,
            content,
            viewport,
            child_font_px,
            writing,
            nodes,
            intrinsic,
            output,
            scope,
        )? {
            recorded_overlay.push(entry);
        }
    }
    if let Some(Some(plan)) = nodes.container_plans.get_mut(&id) {
        plan.overlay = recorded_overlay;
    }
    Ok(())
}

/// Whether an in-flow child's used box reads its container's content box.
///
/// A cross size other than a fixed length does: `auto` shrinks to the
/// available width, and `fill` or a percentage resolves against it. So does
/// any size, margin, padding, or inset that resolves against the containing
/// block, on either axis. A container whose children all answer `false` keeps
/// their boxes when only its own size changes.
fn reads_container_size(style: &LayoutStyle, direction: FlexDirection) -> bool {
    let cross = match direction {
        FlexDirection::Row => style.height,
        FlexDirection::Column => style.width,
    };
    let fixed_cross = cross.is_some_and(|spec| !depends_on_used_basis(Some(spec)));
    !fixed_cross
        || style.has_logical_box_edges()
        || [
            style.width,
            style.height,
            style.min_width,
            style.max_width,
            style.min_height,
            style.max_height,
            style.flex_basis,
            style.offset_top,
            style.offset_right,
            style.offset_bottom,
            style.offset_left,
        ]
        .into_iter()
        .chain(box_edge_specs(style))
        .any(depends_on_used_basis)
}

/// Whether an in-flow child's size or position reads its container's main
/// extent: a main-axis size, limit or basis against the containing block, an
/// inset on the main axis against it, or -- on a row, whose main axis is the
/// inline one -- a percentage margin or padding, which resolve against the
/// containing block's inline size.
fn reads_main_extent(style: &LayoutStyle, direction: FlexDirection) -> bool {
    if style.has_logical_box_edges() {
        return true;
    }
    match direction {
        FlexDirection::Column => [
            style.height,
            style.min_height,
            style.max_height,
            style.flex_basis,
            style.offset_top,
            style.offset_bottom,
        ]
        .into_iter()
        .any(spec_tracks_containing_block),
        FlexDirection::Row => [
            style.width,
            style.min_width,
            style.max_width,
            style.flex_basis,
            style.offset_left,
            style.offset_right,
        ]
        .into_iter()
        .chain(box_edge_specs(style))
        .any(spec_tracks_containing_block),
    }
}

/// A child that stopped generating a box gets no flow slot, so the container
/// loop never places it. Publish the omitted box a full pass leaves it, or the
/// retained one survives the hide.
fn retire_omitted_children(
    children: &[StableNodeId],
    nodes: &LayoutInputMap<'_>,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: &ScopeContext<'_>,
) {
    for &child in children {
        if nodes.style(child).is_some_and(|style| style.omits_box())
            && scope
                .retained
                .boxes
                .get(&child)
                .is_some_and(|retained| *retained != LayoutBox::default())
        {
            output.insert(child, LayoutBox::default());
        }
    }
}

/// Place one absolute or fixed child against its containing block.
///
/// The full container path and the overlay replay share this so a
/// containing-block change and a content change cannot drift apart.
#[allow(clippy::too_many_arguments)]
fn place_positioned_child(
    child: StableNodeId,
    content_origin: Point,
    content: Size,
    viewport: LayoutViewport,
    child_font_px: f32,
    writing: nana_ui_core::WritingContext,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Option<PlannedOverlay>, UiWorldError> {
    let Some(child_style_arc) = nodes.style(child) else {
        return Ok(None);
    };
    let child_style = child_style_arc.as_ref();
    if !child_style.position.is_out_of_flow() {
        return Ok(None);
    }
    let fixed = child_style.position == PositionSpec::Fixed;
    let base = if fixed {
        Size::new(viewport.width, viewport.height)
    } else {
        content
    };
    let base_origin = if fixed { Point::ZERO } else { content_origin };
    let child_fonts = fonts_of(child_style, child_font_px);
    // An auto height is the content's (CSS shrink-to-fit): what inside it
    // fills or takes a percentage of its height has nothing definite to
    // resolve against, rather than the containing block's height --
    // which a box that sizes the containing block (a measured virtual
    // row) would feed back into itself.
    let auto_height = child_style
        .height
        .is_none_or(|height| height == LengthSpec::Auto || height.is_content_sized())
        && !(child_style.offset_top.is_some() && child_style.offset_bottom.is_some());
    let mut child_size = if auto_height {
        // Its own percentage min/max height read `base`; they apply below.
        intrinsic_size_out_of_flow(
            child,
            Size::new(base.width, 0.0),
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            scope,
        )?
    } else {
        intrinsic_size_scoped(
            child,
            base,
            None,
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            scope,
        )?
    };
    let left = LayoutStyle::resolve_inset_fonts(child_style.offset_left, base.width, child_fonts);
    let right = LayoutStyle::resolve_inset_fonts(child_style.offset_right, base.width, child_fonts);
    let top = LayoutStyle::resolve_inset_fonts(child_style.offset_top, base.height, child_fonts);
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
    if let Some(max) = child_style.resolved_max_height_fonts(Some(base.height), vp, child_fonts) {
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
        writing,
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
    let tracks_containing_block = positioned_tracks_containing_block(child_style);
    Ok(Some(PlannedOverlay {
        child,
        style: child_style_arc,
        fixed,
        tracks_containing_block,
        base,
        base_origin,
        origin: child_origin,
        size: child_size,
    }))
}

fn positioned_tracks_containing_block(style: &LayoutStyle) -> bool {
    fn tracks(spec: Option<LengthSpec>) -> bool {
        spec_tracks_containing_block(spec)
    }
    tracks(style.width)
        || tracks(style.height)
        || tracks(style.min_width)
        || tracks(style.max_width)
        || tracks(style.min_height)
        || tracks(style.max_height)
        || tracks(style.offset_left)
        || tracks(style.offset_right)
        || tracks(style.offset_top)
        || tracks(style.offset_bottom)
        || (style.offset_left.is_some()
            && style.offset_right.is_some()
            && !style.width.is_some_and(LengthSpec::is_definite_declared))
        || (style.offset_top.is_some()
            && style.offset_bottom.is_some()
            && !style.height.is_some_and(LengthSpec::is_definite_declared))
        // Anchored to the far edge: the position is the block's size less
        // the offset and the box.
        || (style.offset_right.is_some() && style.offset_left.is_none())
        || (style.offset_bottom.is_some() && style.offset_top.is_none())
        // An auto width shrinks to fit the space the block leaves.
        || style.width.is_none_or(|width| !width.is_definite_declared())
}

fn overlay_child_affected(
    child: StableNodeId,
    container: StableNodeId,
    scope: &ScopeContext<'_>,
    nodes: &LayoutInputMap<'_>,
) -> bool {
    let _ = (container, nodes);
    scope.affected.contains(&child) || scope.reach.reaches(child)
}

fn overlay_depends_on_block(
    entry: &PlannedOverlay,
    plan: &ContainerPlan,
    content_origin: Point,
    content: Size,
    viewport: LayoutViewport,
) -> bool {
    if entry.fixed {
        return plan.viewport.width.to_bits() != viewport.width.to_bits()
            || plan.viewport.height.to_bits() != viewport.height.to_bits();
    }
    if plan.content_origin != content_origin {
        return true;
    }
    (plan.content.width.to_bits() != content.width.to_bits()
        || plan.content.height.to_bits() != content.height.to_bits())
        && entry.tracks_containing_block
}

/// Place the positioned children that this pass actually depends on.
///
/// A content change of one positioned child does not place in-flow siblings.
/// A containing-block change places only the positioned children that read
/// that block. If the recorded participants no longer describe the context,
/// every positioned child of this container is placed and the walk stops
/// there — still inside that formatting context, not the document.
#[allow(clippy::too_many_arguments)]
fn finish_positioned_overlay(
    id: StableNodeId,
    plan: &ContainerPlan,
    origin: Point,
    size: Size,
    content_origin: Point,
    content: Size,
    viewport: LayoutViewport,
    child_font_px: f32,
    writing: nana_ui_core::WritingContext,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: &ScopeContext<'_>,
) -> Result<(), UiWorldError> {
    let stale = plan.overlay.iter().any(|entry| {
        nodes
            .style(entry.child)
            .is_none_or(|style| !style.position.is_out_of_flow())
    });
    let mut placed: Vec<PlannedOverlay> = Vec::new();
    if stale {
        intrinsic.note_local_context_fallback();
        let live = collect_positioned_children(&plan.children, nodes)?;
        for child in live {
            if let Some(entry) = place_positioned_child(
                child,
                content_origin,
                content,
                viewport,
                child_font_px,
                writing,
                nodes,
                intrinsic,
                output,
                Some(scope),
            )? {
                placed.push(entry);
            }
        }
    } else {
        let dependents: Vec<StableNodeId> = plan
            .overlay
            .iter()
            .filter(|entry| {
                overlay_child_affected(entry.child, id, scope, nodes)
                    || overlay_depends_on_block(entry, plan, content_origin, content, viewport)
            })
            .map(|entry| entry.child)
            .collect();
        for child in dependents {
            if let Some(entry) = place_positioned_child(
                child,
                content_origin,
                content,
                viewport,
                child_font_px,
                writing,
                nodes,
                intrinsic,
                output,
                Some(scope),
            )? {
                placed.push(entry);
            }
        }
    }
    let geometry_moved = plan.origin != origin
        || plan.size != size
        || plan.content != content
        || plan.content_origin != content_origin;
    // The new block is recorded for the positioned children that compare
    // against it. A flow-only plan keeps the geometry its children were
    // placed in: the replays above hand them `plan.content`, so moving it
    // here would make every child plan disagree with its containing block on
    // the next pass and fall back to measuring that whole container.
    if (geometry_moved && !plan.overlay.is_empty()) || !placed.is_empty() || stale {
        publish_overlay_plan(
            id,
            plan,
            origin,
            size,
            content_origin,
            content,
            &placed,
            stale,
            nodes,
        );
    }
    Ok(())
}

fn publish_overlay_plan(
    id: StableNodeId,
    plan: &ContainerPlan,
    origin: Point,
    size: Size,
    content_origin: Point,
    content: Size,
    placed: &[PlannedOverlay],
    replace_overlay: bool,
    nodes: &mut LayoutInputMap<'_>,
) {
    let write = |target: &mut ContainerPlan| {
        target.origin = origin;
        target.size = size;
        target.content = content;
        target.content_origin = content_origin;
        if replace_overlay || target.overlay.is_empty() {
            target.overlay = placed.to_vec();
            return;
        }
        for update in placed {
            if let Some(slot) = target
                .overlay
                .iter_mut()
                .find(|entry| entry.child == update.child)
            {
                *slot = update.clone();
            }
        }
    };
    if let Some(Some(existing)) = nodes.container_plans.get_mut(&id) {
        write(existing);
        return;
    }
    let mut updated = plan.clone();
    write(&mut updated);
    nodes.container_plans.insert(id, Some(updated));
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
    id: StableNodeId,
    overlay: crate::TriggeredMenuOverlay,
    trigger: LayoutBox,
    items: &[StableNodeId],
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    let inner_width = (overlay.width - overlay.padding * 2.0).max(0.0);
    let available = Size::new(inner_width, viewport.height);
    let mut item_sizes = Vec::with_capacity(items.len());
    let mut content_height = 0.0;
    // Gaps fall between items that take room: an empty group of items (a
    // keyed list with no rows yet) adds none.
    let mut placed_any = false;
    for child in items.iter().copied() {
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
        if size.height > 0.0 {
            if placed_any {
                content_height += crate::popover::MENU_ITEM_GAP;
            }
            placed_any = true;
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
    // The items are viewport-fixed, so they hang off where the trigger shows:
    // its box through the scroll offsets and transforms above it, not the
    // unscrolled layout box. Not through its own transform: the trigger's
    // open animation would otherwise leave the surface where its first
    // frame put it.
    let trigger = nodes
        .world
        .project_placement_bounds(id, trigger)
        .unwrap_or(trigger);
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
            nodes.world.containing_writing(child),
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
        if child_size.height > 0.0 {
            cursor_y += child_size.height + crate::popover::MENU_ITEM_GAP;
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
    intrinsic: &mut PassIntrinsicCache,
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
    intrinsic: &mut PassIntrinsicCache,
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
        nodes.world.containing_writing(id),
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
