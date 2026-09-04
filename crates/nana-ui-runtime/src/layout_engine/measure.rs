//! Shared Runtime layout measure algorithms.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn intrinsic_size(
    id: StableNodeId,
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut IntrinsicCache,
) -> Result<Size, UiWorldError> {
    intrinsic_size_scoped(
        id,
        available,
        parent_direction,
        viewport,
        parent_font_px,
        nodes,
        cache,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn intrinsic_size_scoped(
    id: StableNodeId,
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut IntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Size, UiWorldError> {
    let cache_key = (id, available.width.to_bits(), available.height.to_bits());
    if let Some(size) = cache.get(&cache_key) {
        return Ok(*size);
    }
    // A subtree outside the affected closure has no change inside it, so its
    // intrinsic size under the same constraints is unchanged.
    if let Some(scope) = scope
        && !scope.affected.contains(&id)
        && let Some(size) = scope.retained.intrinsics.get(&cache_key)
    {
        cache.insert(cache_key, *size);
        return Ok(*size);
    }
    let Some(node) = nodes.get(id)? else {
        return Ok(Size::default());
    };
    let style = node.style.clone();
    let children = node.children.clone();
    let text_metrics = node.text_metrics;
    let style = style.as_ref();
    if style.omits_box() {
        return Ok(Size::default());
    }
    let fonts = fonts_of(style, parent_font_px);
    let child_font_px = fonts.element_px;
    let padding = style.resolved_padding_against_fonts(Some(available.width), fonts);
    let border = style.resolved_border_edges();
    let chrome = Size::new(
        padding.left + padding.right + border.left + border.right,
        padding.top + padding.bottom + border.top + border.bottom,
    );
    // Measure descendants against this node's declared content box, not its
    // parent's full budget. Percent padding still resolves against the parent.
    let margin = style.resolved_margin_against_fonts(Some(available.width), fonts);
    let content_axis = |spec: Option<LengthSpec>, available: f32, margins: f32, chrome: f32| {
        let resolved = resolve_axis(
            demote_fill_spec_if_indefinite(spec, available),
            available,
            available - margins,
            viewport,
            fonts,
        );
        let extent = resolved.unwrap_or(available);
        if resolved.is_some()
            && matches!(style.box_sizing, BoxSizing::ContentBox)
            && spec.is_some_and(LengthSpec::is_definite_declared)
        {
            extent.max(0.0)
        } else {
            (extent - chrome).max(0.0)
        }
    };
    let content_available = Size::new(
        content_axis(
            style.width,
            available.width,
            margin.left + margin.right,
            chrome.width,
        ),
        content_axis(
            style.height,
            available.height,
            margin.top + margin.bottom,
            chrome.height,
        ),
    );
    let flow_children = collect_flow_children(&children, nodes, style.display)?;
    let grid_measure = uses_2d_grid(style, &flow_children, nodes);
    let ifc = !grid_measure
        && !style
            .display
            .is_some_and(|d| d.is_flex_container() || d.is_grid_container())
        && flow_children
            .iter()
            .any(|id| nodes.style(*id).is_some_and(|s| s.is_inline_level()));
    let direction = used_flow_direction(style, ifc);
    let mut child_sizes = Vec::with_capacity(flow_children.len());
    for child in &flow_children {
        let child_available = nodes
            .style(*child)
            .filter(|_| grid_measure)
            .map(|child_style| grid_item_measure_available(child_style.as_ref(), content_available))
            .unwrap_or(content_available);
        child_sizes.push(intrinsic_size_scoped(
            *child,
            child_available,
            Some(direction),
            viewport,
            child_font_px,
            nodes,
            cache,
            scope,
        )?);
    }
    let parent_box = gap_containing_block(style, content_available);
    let gap = style.main_gap_against_fonts(direction, parent_box, fonts);
    let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
    let wrap = style.flex_wrap;
    let wrapping = ifc
        || match direction {
            FlexDirection::Row => matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse),
            FlexDirection::Column => {
                matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse)
                    && content_available.height > 0.5
            }
        };
    let grid_tracks = match direction {
        FlexDirection::Row => style.active_grid_columns(),
        FlexDirection::Column => style.active_grid_rows(),
    };
    let child_margin = |child, nodes: &LayoutInputMap<'_>| {
        nodes
            .style(child)
            .map(|style| {
                style.resolved_margin_against_fonts(
                    Some(content_available.width),
                    fonts_of(&style, child_font_px),
                )
            })
            .unwrap_or_default()
    };
    let children = if uses_2d_grid(style, &flow_children, nodes) {
        let grid = layout_grid_2d(
            style,
            &flow_children,
            &child_sizes,
            content_available,
            fonts,
            nodes,
            None,
        );
        Size::new(
            grid_axis_extent(&grid.col_sizes, grid.col_gap),
            grid_axis_extent(&grid.row_sizes, grid.row_gap),
        )
    } else if let Some(tracks) = grid_tracks.filter(|tracks| !tracks.is_empty()) {
        let auto_sizes = auto_track_contributions(
            &flow_children,
            tracks,
            content_available,
            direction == FlexDirection::Column,
            viewport,
            child_font_px,
            nodes,
            cache,
            scope,
        )?;
        let budget = main_extent(content_available, direction);
        let resolved = resolve_grid_track_sizes(tracks, budget, gap, &auto_sizes);
        grid_intrinsic_size(
            direction,
            &resolved,
            &child_sizes,
            &flow_children,
            content_available.width,
            gap,
            child_font_px,
            nodes,
        )
    } else if wrapping {
        wrap_intrinsic_size(
            direction,
            wrap,
            &flow_children,
            &child_sizes,
            content_available,
            gap,
            cross_gap,
            grid_tracks,
            viewport,
            child_font_px,
            nodes,
        )
    } else {
        let gaps = gap * flow_children.len().saturating_sub(1) as f32;
        let mut main = gaps;
        let mut cross = 0.0f32;
        for (child, size) in flow_children.iter().zip(&child_sizes) {
            let margin = child_margin(*child, nodes);
            main += main_extent(*size, direction)
                + main_start_margin(margin, direction)
                + main_end_margin(margin, direction);
            cross = cross.max(cross_extent(*size, direction) + cross_margin(margin, direction));
        }
        match direction {
            FlexDirection::Row => Size::new(main.max(0.0), cross),
            FlexDirection::Column => Size::new(cross, main.max(0.0)),
        }
    };
    let text = text_metrics.unwrap_or_default();
    let mut content = Size::new(
        children.width.max(text.width),
        children.height.max(text.height),
    );
    if text_metrics.is_none()
        && flow_children.is_empty()
        && let Some(fs) = style.font_size.filter(|value| *value > 0.0)
    {
        content.height = content
            .height
            .max(text_line_box_height_px(fs, style.line_height));
    }
    let max_content_w = content.width + chrome.width;
    let stacked_min_w = child_sizes
        .iter()
        .zip(&flow_children)
        .map(|(size, child)| {
            let margin = child_margin(*child, nodes);
            size.width + margin.left + margin.right
        })
        .fold(0.0f32, f32::max)
        + chrome.width;
    // nowrap row: min-content cannot be narrower than the packed sum.
    // wrap / column / block: min-content is the largest child (plus chrome).
    let min_content_w = if wrapping || direction.is_column() {
        stacked_min_w
    } else {
        max_content_w
    };
    let default_width = match style.width {
        Some(LengthSpec::MinContent) => min_content_w,
        Some(LengthSpec::MaxContent) | Some(LengthSpec::Shrink) => max_content_w,
        Some(LengthSpec::FitContent) => max_content_w.min(available.width).max(stacked_min_w),
        _ if parent_direction.is_none()
            && !style.width.is_some_and(LengthSpec::is_content_sized)
            && !flow_children.is_empty() =>
        {
            available.width
        }
        _ => max_content_w,
    };
    let default_height = content.height + chrome.height;
    // `Fill` sizes the border box to the containing block minus the node's own
    // margins — negative margins widen it, matching the stretch path below;
    // percentages keep resolving against the raw containing block.
    let margin = style.resolved_margin_against_fonts(Some(available.width), fonts);
    let width_spec = resolve_axis(
        demote_fill_spec_if_indefinite(style.width, available.width),
        available.width,
        available.width - margin.left - margin.right,
        viewport,
        fonts,
    );
    let height_spec = resolve_axis(
        demote_fill_spec_if_indefinite(style.height, available.height),
        available.height,
        available.height - margin.top - margin.bottom,
        viewport,
        fonts,
    );
    let width_from_spec = width_spec.is_some();
    let height_from_spec = height_spec.is_some();
    let vp = Some((viewport.width, viewport.height));
    let min_width = style.resolved_min_width_fonts(Some(available.width), vp, fonts);
    let min_height = style.resolved_min_height_fonts(Some(available.height), vp, fonts);
    let mut width = width_spec.unwrap_or(default_width).max(min_width);
    let mut height = height_spec.unwrap_or(default_height).max(min_height);
    if matches!(style.box_sizing, BoxSizing::ContentBox) {
        if style.width.is_some_and(LengthSpec::is_definite_declared) {
            width += chrome.width;
        }
        if style.height.is_some_and(LengthSpec::is_definite_declared) {
            height += chrome.height;
        }
    }
    if style.aspect_ratio.is_some_and(|r| r.is_finite() && r > 0.0) {
        let stretch_fit_width = !width_from_spec
            && style.stretch_fit_inline()
            && !matches!(parent_direction, Some(FlexDirection::Row))
            && available.width > 0.5;
        if stretch_fit_width {
            width = available.width.max(min_width);
        }
        let mut content_w =
            if width_from_spec || stretch_fit_width || (!height_from_spec && width > 0.0) {
                Some((width - chrome.width).max(0.0))
            } else {
                None
            };
        let mut content_h = if height_from_spec {
            Some((height - chrome.height).max(0.0))
        } else {
            None
        };
        style.apply_aspect_ratio_used(&mut content_w, &mut content_h);
        if let Some(content_w) = content_w {
            width = content_w + chrome.width;
        }
        if let Some(content_h) = content_h {
            height = content_h + chrome.height;
        }
        width = width.max(min_width);
        height = height.max(min_height);
    }
    if let Some(max) = style.resolved_max_width_fonts(Some(available.width), vp, fonts) {
        width = width.min(max);
    }
    if let Some(max) = style.resolved_max_height_fonts(Some(available.height), vp, fonts) {
        height = height.min(max);
    }
    let size = Size::new(width, height);
    cache.insert(cache_key, size);
    Ok(size)
}
