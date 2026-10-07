//! Shared Runtime layout inline algorithms.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn pack_wrap_lines(
    children: &[StableNodeId],
    sizes: &[Size],
    direction: FlexDirection,
    content: Size,
    // Percent base of the children's margins: the container's inline size.
    edge_base: f32,
    gap: f32,
    grid_tracks: Option<&[GridTrack]>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
    break_on_blocks: bool,
) -> Vec<Vec<usize>> {
    // Lines fill the main axis; percentage margins resolve against the
    // containing block's inline size, which is the main axis only for a row
    // in `horizontal-tb` (CSS Box Model §5).
    let content_main = main_extent(content, direction);
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut line_main = 0.0f32;
    for (index, child) in children.iter().enumerate() {
        let Some(style) = nodes.style(*child) else {
            continue;
        };
        let block_break = break_on_blocks && !style.is_inline_level();
        if block_break && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            line_main = 0.0;
        }
        let margin = style.resolved_margin_against_fonts(
            Some(edge_base),
            fonts_of(style.as_ref(), parent_font_px),
        );
        let main = packing_main_size(
            style.as_ref(),
            sizes[index],
            direction,
            content_main,
            edge_base,
            viewport,
            parent_font_px,
            grid_tracks.and_then(|tracks| tracks.get(index).copied()),
        );
        let outer =
            main + main_start_margin(margin, direction) + main_end_margin(margin, direction);
        let need = if current.is_empty() {
            outer
        } else {
            line_main + gap + outer
        };
        if !current.is_empty() && need > content_main + 0.5 {
            lines.push(std::mem::take(&mut current));
            line_main = 0.0;
        }
        if current.is_empty() {
            line_main = outer;
        } else {
            line_main += gap + outer;
        }
        current.push(index);
        if block_break && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            line_main = 0.0;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

pub(super) fn ifc_item_outer(
    style: &LayoutStyle,
    size: Size,
    content_width: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
) -> (f32, f32) {
    let direction = FlexDirection::Row;
    let margin =
        style.resolved_margin_against_fonts(Some(content_width), fonts_of(style, parent_font_px));
    let main = packing_main_size(
        style,
        size,
        direction,
        content_width,
        // A horizontal line box: its width is the inline size.
        content_width,
        viewport,
        parent_font_px,
        None,
    );
    let outer_main =
        main + main_start_margin(margin, direction) + main_end_margin(margin, direction);
    let outer_cross = cross_extent(size, direction) + cross_margin(margin, direction);
    (outer_main, outer_cross)
}

/// IFC wrap using the existing content-width line packer, with per-line available
/// width reduced by sibling float occupancy (shrink-to-avoid-float).
#[allow(clippy::too_many_arguments)]
pub(super) fn pack_ifc_line_boxes(
    children: &[StableNodeId],
    sizes: &[Size],
    content_origin: Point,
    content_width: f32,
    gap: f32,
    cross_gap: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
    packed_floats: &PackedFloats,
) -> Vec<LineBoxSlot> {
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut line_main = 0.0f32;
    let mut line_y = 0.0f32;
    let mut left_inset = 0.0f32;
    let mut available = content_width;
    let refresh = |line_y: f32, left_inset: &mut f32, available: &mut f32| {
        let (left, right) = packed_floats.insets_at_y(content_origin, content_width, line_y);
        *left_inset = left;
        *available = (content_width - left - right).max(0.0);
    };
    refresh(line_y, &mut left_inset, &mut available);
    let flush = |lines: &mut Vec<LineBoxSlot>,
                 current: &mut Vec<usize>,
                 line_main: &mut f32,
                 line_y: &mut f32,
                 left_inset: &mut f32,
                 available: &mut f32| {
        if current.is_empty() {
            return;
        }
        let line_cross = current
            .iter()
            .map(|&index| {
                nodes
                    .style(children[index])
                    .map(|style| {
                        ifc_item_outer(
                            style.as_ref(),
                            sizes[index],
                            content_width,
                            viewport,
                            parent_font_px,
                        )
                        .1
                    })
                    .unwrap_or(0.0)
            })
            .fold(0.0f32, f32::max);
        lines.push(LineBoxSlot {
            indices: std::mem::take(current),
            main_start: *left_inset,
            main_available: *available,
            cross_y: *line_y,
            pin_cross: true,
        });
        *line_main = 0.0;
        *line_y += line_cross + cross_gap;
        refresh(*line_y, left_inset, available);
    };
    for (index, child) in children.iter().enumerate() {
        let Some(style) = nodes.style(*child) else {
            continue;
        };
        let style_ref = style.as_ref();
        let clear_y = clear_offset(
            style_ref.clear,
            packed_floats.left_bottom,
            packed_floats.right_bottom,
        );
        if clear_y > line_y + 0.5 {
            flush(
                &mut lines,
                &mut current,
                &mut line_main,
                &mut line_y,
                &mut left_inset,
                &mut available,
            );
            line_y = clear_y;
            refresh(line_y, &mut left_inset, &mut available);
        }
        let block_break = !style_ref.is_inline_level();
        if block_break {
            flush(
                &mut lines,
                &mut current,
                &mut line_main,
                &mut line_y,
                &mut left_inset,
                &mut available,
            );
            let (_, outer_cross) = ifc_item_outer(
                style_ref,
                sizes[index],
                content_width,
                viewport,
                parent_font_px,
            );
            lines.push(LineBoxSlot {
                indices: vec![index],
                main_start: 0.0,
                main_available: content_width,
                cross_y: line_y,
                pin_cross: true,
            });
            line_y += outer_cross + cross_gap;
            refresh(line_y, &mut left_inset, &mut available);
            continue;
        }
        let (outer, _) = ifc_item_outer(
            style_ref,
            sizes[index],
            content_width,
            viewport,
            parent_font_px,
        );
        let need = if current.is_empty() {
            outer
        } else {
            line_main + gap + outer
        };
        if !current.is_empty() && need > available + 0.5 {
            flush(
                &mut lines,
                &mut current,
                &mut line_main,
                &mut line_y,
                &mut left_inset,
                &mut available,
            );
        }
        if current.is_empty() && outer > available + 0.5 {
            while outer > available + 0.5 {
                match packed_floats.next_bottom_after(content_origin, line_y) {
                    Some(next) if next > line_y + 0.5 => {
                        line_y = next;
                        refresh(line_y, &mut left_inset, &mut available);
                    }
                    _ => break,
                }
            }
        }
        if current.is_empty() {
            line_main = outer;
        } else {
            line_main += gap + outer;
        }
        current.push(index);
    }
    flush(
        &mut lines,
        &mut current,
        &mut line_main,
        &mut line_y,
        &mut left_inset,
        &mut available,
    );
    if lines.is_empty() {
        lines.push(LineBoxSlot {
            indices: Vec::new(),
            main_start: 0.0,
            main_available: content_width,
            cross_y: 0.0,
            pin_cross: true,
        });
    }
    lines
}

#[allow(clippy::too_many_arguments)]
pub(super) fn wrap_intrinsic_size(
    direction: FlexDirection,
    wrap: FlexWrap,
    children: &[StableNodeId],
    sizes: &[Size],
    available: Size,
    // Percent base of the children's margins: the container's inline size.
    edge_base: f32,
    gap: f32,
    cross_gap: f32,
    grid_tracks: Option<&[GridTrack]>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
) -> Size {
    let content_main = main_extent(available, direction);
    let mut lines = pack_wrap_lines(
        children,
        sizes,
        direction,
        available,
        edge_base,
        gap,
        grid_tracks,
        viewport,
        parent_font_px,
        nodes,
        false,
    );
    if matches!(wrap, FlexWrap::WrapReverse) {
        lines.reverse();
    }
    let mut cross = 0.0f32;
    let mut max_main = 0.0f32;
    for (line_index, line) in lines.iter().enumerate() {
        let mut line_main = 0.0f32;
        let mut line_cross = 0.0f32;
        for (item_index, &index) in line.iter().enumerate() {
            let Some(style) = nodes.style(children[index]) else {
                continue;
            };
            let margin = style.resolved_margin_against_fonts(
                Some(edge_base),
                fonts_of(style.as_ref(), parent_font_px),
            );
            let main = packing_main_size(
                style.as_ref(),
                sizes[index],
                direction,
                content_main,
                edge_base,
                viewport,
                parent_font_px,
                grid_tracks.and_then(|tracks| tracks.get(index).copied()),
            );
            let outer_main =
                main + main_start_margin(margin, direction) + main_end_margin(margin, direction);
            line_main += outer_main;
            if item_index > 0 {
                line_main += gap;
            }
            line_cross = line_cross
                .max(cross_extent(sizes[index], direction) + cross_margin(margin, direction));
        }
        max_main = max_main.max(line_main);
        cross += line_cross;
        if line_index + 1 < lines.len() {
            cross += cross_gap;
        }
    }
    match direction {
        FlexDirection::Row => Size::new(max_main, cross),
        FlexDirection::Column => Size::new(cross, max_main),
    }
}

/// A horizontal IFC whose line breaks are a function of cached child border
/// boxes. Floats, unboxing, and chrome stay on the full formatting-context path.
pub(super) fn ifc_local_measure(
    style: &LayoutStyle,
    writing: nana_ui_core::WritingContext,
    flow: &[StableNodeId],
    child_ids: &[StableNodeId],
    nodes: &mut LayoutInputMap<'_>,
) -> Result<bool, UiWorldError> {
    if !ifc_plan_style(style, writing) || !flow_is_subsequence(flow, child_ids) {
        return Ok(false);
    }
    for &child in flow {
        let Some(child_style) = nodes.style(child) else {
            return Ok(false);
        };
        if !ifc_local_child(child_style.as_ref()) {
            return Ok(false);
        }
        // An inline-block is atomic. Only a non-atomic inline can hoist a
        // block and change this formatting context's child list.
        if child_style.display == Some(DisplaySpec::Inline) && inline_contains_block(child, nodes)?
        {
            return Ok(false);
        }
    }
    Ok(!flow.is_empty())
}

fn ifc_plan_style(style: &LayoutStyle, writing: nana_ui_core::WritingContext) -> bool {
    if writing.is_vertical()
        || style
            .display
            .is_some_and(|display| display.is_flex_container() || display.is_grid_container())
        || style.aspect_ratio.is_some()
        || style.height.is_some()
        || style.min_height.is_some()
        || style.max_height.is_some()
        || !box_chrome_is_zero(style)
    {
        return false;
    }
    matches!(style.width, Some(LengthSpec::Px(value)) if value.is_finite() && value >= 0.0)
        && style.text_align == nana_ui_core::TextAlignSpec::Start
}

fn box_chrome_is_zero(style: &LayoutStyle) -> bool {
    style.padding.is_none()
        && style.padding_top.is_none()
        && style.padding_right.is_none()
        && style.padding_bottom.is_none()
        && style.padding_left.is_none()
        && style.border_width.is_none()
        && style.border_top_width.is_none()
        && style.border_right_width.is_none()
        && style.border_bottom_width.is_none()
        && style.border_left_width.is_none()
}

fn ifc_local_child(style: &LayoutStyle) -> bool {
    style.is_inline_level()
        && !style.is_floated()
        && style.clear == ClearSpec::None
        && style.order == 0
        && style.flex_grow.unwrap_or(0.0) <= 0.0
        && style.flex_shrink.unwrap_or(0.0) <= 0.0
        && !style.position.is_out_of_flow()
}

fn flow_is_subsequence(flow: &[StableNodeId], children: &[StableNodeId]) -> bool {
    let mut index = 0;
    for child in children {
        if flow.get(index) == Some(child) {
            index += 1;
        }
    }
    index == flow.len()
}

/// Recompute one inline formatting context from cached line contributions.
///
/// Returns `None` when a nested edit, a float, or a child whose size depends
/// on line membership means the break is not local. The caller then measures
/// this formatting context, not the document.
#[allow(clippy::too_many_arguments)]
pub(super) fn ifc_line_measure_delta(
    id: StableNodeId,
    plan: &MeasurePlan,
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: &ScopeContext<'_>,
) -> Result<Option<Size>, UiWorldError> {
    let style = plan.style.as_ref();
    if plan.sequential
        || plan.child_direction != FlexDirection::Row
        || !ifc_plan_style(style, plan.writing)
        || plan.text_metrics.is_some()
        || nodes.world.standard_visual_ref(id).is_some()
    {
        return Ok(None);
    }
    if !affected_edits_are_direct(id, plan, scope, nodes) {
        return Ok(None);
    }
    let mut flow = Vec::new();
    let mut sizes = Vec::new();
    for child in plan.children.iter().copied() {
        let Some(entry) = plan.entry(child) else {
            return Ok(None);
        };
        let Some(intrinsic) = entry.intrinsic else {
            if scope.affected.contains(&child) {
                return Ok(None);
            }
            continue;
        };
        let Some(child_style) = entry.style.as_deref() else {
            return Ok(None);
        };
        if !ifc_local_child(child_style) {
            return Ok(None);
        }
        flow.push(child);
        sizes.push(intrinsic);
    }
    if flow.is_empty() {
        return Ok(None);
    }
    let mut patched = false;
    for index in 0..flow.len() {
        let child = flow[index];
        if !scope.affected.contains(&child) {
            continue;
        }
        let current = nodes.style(child);
        let Some(child_style) = current.as_deref() else {
            return Ok(None);
        };
        if !ifc_local_child(child_style) {
            return Ok(None);
        }
        let cached_style = plan.entry(child).and_then(|entry| entry.style.clone());
        if !scope.measure.contains(&child) {
            if !retained_style_matches(&current, &cached_style) {
                return Ok(None);
            }
            continue;
        }
        let measured = intrinsic_size_scoped(
            child,
            plan.child_available,
            Some(plan.child_direction),
            viewport,
            child_font_px,
            nodes,
            cache,
            Some(scope),
        )?;
        if measured != sizes[index] {
            sizes[index] = measured;
            patched = true;
        }
    }
    if !patched {
        return Ok(None);
    }
    let fonts = fonts_of(style, plan.parent_font_px);
    let parent_box = gap_containing_block(style, plan.child_available);
    let direction = plan.child_direction;
    let gap = style.main_gap_against_fonts(direction, parent_box, fonts);
    let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
    let edge_base = plan
        .writing
        .logical_size(plan.child_available.width, plan.child_available.height)
        .0;
    let content = wrap_intrinsic_size(
        direction,
        FlexWrap::Wrap,
        &flow,
        &sizes,
        plan.child_available,
        edge_base,
        gap,
        cross_gap,
        None,
        viewport,
        child_font_px,
        nodes,
    );
    let mut size = plan.size;
    // The inline size is the definite line budget. Only the block size follows
    // the lines, and this plan has no padding or border to add back.
    size.height = content.height;
    let mut updated = plan.clone();
    updated.size = size;
    for (child, measured) in flow.into_iter().zip(sizes) {
        let Ok(slot) = updated
            .entries
            .binary_search_by_key(&child, |entry| entry.child)
        else {
            continue;
        };
        updated.entries[slot].intrinsic = Some(measured);
        if let Some(child_style) = nodes.style(child) {
            updated.entries[slot].style = Some(child_style);
        }
    }
    nodes.measure_plans.entry(id).or_default().insert(updated);
    Ok(Some(size))
}

fn affected_edits_are_direct(
    container: StableNodeId,
    plan: &MeasurePlan,
    scope: &ScopeContext<'_>,
    nodes: &LayoutInputMap<'_>,
) -> bool {
    for &id in scope.affected {
        if id == container || plan.entry(id).is_some() {
            continue;
        }
        let mut cursor = id;
        let mut direct = None;
        while let Some(parent) = nodes.world.parent_id(cursor) {
            if parent == container {
                direct = Some(cursor);
                break;
            }
            cursor = parent;
        }
        // Content inside a fixed inline-block cannot move the line. The
        // block's own border box, when it changes, is a direct entry above.
        let Some(direct) = direct else {
            continue;
        };
        let Some(style) = nodes.style(direct) else {
            return false;
        };
        if !fixed_inline_block(style.as_ref(), plan.style.as_ref()) {
            return false;
        }
    }
    true
}

/// A fixed inline-block whose parent is outside the frontier. Relayout starts
/// at its retained border box, so the outer line is not packed again.
pub(super) fn fixed_inline_block_island(
    world: &UiWorld,
    id: StableNodeId,
    border: LayoutBox,
    parent_box: LayoutBox,
) -> Option<(Point, Size, f32)> {
    let style = world.layout_style(id)?;
    let parent = world.parent_id(id)?;
    let parent_style = world.layout_style(parent)?;
    if !fixed_inline_block(style.as_ref(), parent_style.as_ref()) {
        return None;
    }
    let pad = parent_style.resolved_padding_against(Some(parent_box.width));
    let edge = parent_style.resolved_border_edges();
    let containing = Size::new(
        (parent_box.width - pad.left - pad.right - edge.left - edge.right).max(0.0),
        (parent_box.height - pad.top - pad.bottom - edge.top - edge.bottom).max(0.0),
    );
    Some((
        Point {
            x: border.x,
            y: border.y,
        },
        containing,
        element_font_px(world, parent),
    ))
}

fn fixed_inline_block(style: &LayoutStyle, parent: &LayoutStyle) -> bool {
    style.display == Some(DisplaySpec::InlineBlock)
        && matches!(style.width, Some(LengthSpec::Px(value)) if value.is_finite() && value >= 0.0)
        && matches!(style.height, Some(LengthSpec::Px(value)) if value.is_finite() && value >= 0.0)
        && style.min_width.is_none()
        && style.max_width.is_none()
        && style.min_height.is_none()
        && style.max_height.is_none()
        && style.align_self.unwrap_or(parent.align_items) != AlignSpec::Baseline
}

fn element_font_px(world: &UiWorld, id: StableNodeId) -> f32 {
    let mut chain = Vec::new();
    let mut cursor = Some(id);
    while let Some(node) = cursor {
        chain.push(node);
        cursor = world.parent_id(node);
    }
    let mut font = ROOT_FONT_PX;
    for node in chain.into_iter().rev() {
        if let Some(style) = world.layout_style(node) {
            font = fonts_of(style.as_ref(), font).element_px;
        }
    }
    font
}
