//! Shared Runtime layout flex algorithms.

use nana_ui_core::dynamic_layout::{LayoutUnits, lower_box};

use super::dynamic::{DynamicLine, LineChild};
use super::*;

/// What a line's Dynamic Layout solve changed: the gap its items now sit
/// apart at, and what each item gave up (empty when nothing was solved).
pub(super) struct LineAdjust {
    pub(super) gap: f32,
    pub(super) amounts: Vec<LayoutUnits>,
}

impl LineAdjust {
    /// Whether the item at `index` gave up part of its preferred size.
    pub(super) fn compressed(&self, index: usize) -> bool {
        self.amounts
            .get(index)
            .is_some_and(|amount| amount.is_positive())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn distribute_flex_main(
    children: &[StableNodeId],
    sizes: &mut [Size],
    direction: FlexDirection,
    content: Size,
    // Percent base of the children's margins: the container's inline size.
    edge_base: f32,
    gap: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
    dynamic: Option<DynamicLine<'_>>,
) -> LineAdjust {
    let mut adjust = LineAdjust {
        gap,
        amounts: Vec::new(),
    };
    let n = children.len();
    if n == 0 {
        return adjust;
    }
    let content_main = main_extent(content, direction);
    let mut gap_total = gap * n.saturating_sub(1) as f32;
    let vp = Some((viewport.width, viewport.height));
    let mut margin_mains = Vec::with_capacity(n);
    let mut fixed_or_fill: Vec<Option<f32>> = Vec::with_capacity(n);
    let mut mins = Vec::with_capacity(n);
    let mut maxs = Vec::with_capacity(n);
    let mut grows = Vec::with_capacity(n);
    let mut shrinks = Vec::with_capacity(n);
    // Items sized by their content: the only ones a line's solve may shrink.
    let mut content_sized = Vec::with_capacity(n);
    for (index, child) in children.iter().enumerate() {
        let Some(style) = nodes.style(*child) else {
            margin_mains.push(0.0);
            mins.push(0.0);
            maxs.push(None);
            grows.push(0.0);
            shrinks.push(1.0);
            content_sized.push(false);
            fixed_or_fill.push(Some(main_extent(sizes[index], direction)));
            continue;
        };
        let fonts = fonts_of(style.as_ref(), parent_font_px);
        let margin = style.resolved_margin_against_fonts(Some(edge_base), fonts);
        let (margin_main, min_main, max_main) = match direction {
            FlexDirection::Row => (
                margin.left + margin.right,
                style.resolved_min_width_fonts(Some(content.width), vp, fonts),
                style.resolved_max_width_fonts(Some(content.width), vp, fonts),
            ),
            FlexDirection::Column => (
                margin.top + margin.bottom,
                style.resolved_min_height_fonts(Some(content_main), vp, fonts),
                style.resolved_max_height_fonts(Some(content_main), vp, fonts),
            ),
        };
        // `min-width: min-content` / `max-content` / `fit-content`: the item
        // does not give up the size its content measured to, however the
        // line shares out its space.
        let min_spec = match direction {
            FlexDirection::Row => style.min_width,
            FlexDirection::Column => style.min_height,
        };
        let min_main = match min_spec {
            Some(LengthSpec::MinContent | LengthSpec::MaxContent | LengthSpec::FitContent) => {
                min_main.max(main_extent(sizes[index], direction))
            }
            _ => min_main,
        };
        margin_mains.push(margin_main);
        mins.push(min_main);
        maxs.push(max_main);
        let main = style.child_main_length(direction);
        let fill_main = matches!(main, Some(LengthSpec::Fill));
        // `flex-grow: None` on Fill means "take remaining" (product LengthSpec::Fill).
        // Explicit `flex-grow: 0` keeps 100%/Fill as a definite main (css-parity).
        let grow = style
            .flex_grow
            .unwrap_or(if fill_main { 1.0 } else { 0.0 })
            .max(0.0);
        grows.push(grow);
        // Unspecified longhand shrink stays 0 (not CSS initial 1) so overflowing
        // definite rows (lists, toolbars) keep their boxes. `flex` shorthand that
        // omits shrink writes `Some(1.0)` (`flex: initial`, `flex: N`, `flex: N <basis>`).
        // css-parity T-F18/F19 set the longhand explicitly.
        shrinks.push(style.flex_shrink.unwrap_or(0.0).max(0.0));
        let resolved_main = resolve_child_main(main, content_main, viewport, fonts);
        content_sized.push(resolved_main.is_none() && grow <= 0.0 && !fill_main);
        match resolved_main {
            Some(value) => {
                let mut value = value.max(min_main);
                if let Some(max) = max_main {
                    value = value.min(max);
                }
                value = content_box_main_border_size(
                    style.as_ref(),
                    direction,
                    Some(edge_base),
                    value,
                    fonts,
                );
                fixed_or_fill.push(Some(value));
            }
            None => {
                if grow > 0.0 {
                    fixed_or_fill.push(None);
                } else if fill_main {
                    let mut value = content_main.max(min_main);
                    if let Some(max) = max_main {
                        value = value.min(max);
                    }
                    value = content_box_main_border_size(
                        style.as_ref(),
                        direction,
                        Some(edge_base),
                        value,
                        fonts,
                    );
                    fixed_or_fill.push(Some(value));
                } else {
                    // Auto: keep intrinsic (text / children), not a Fill share.
                    let intrinsic_main = main_extent(sizes[index], direction).max(min_main);
                    fixed_or_fill.push(Some(intrinsic_main));
                }
            }
        }
    }
    let mut mains = resolve_flex_fill_sizes(
        content_main,
        gap_total,
        &margin_mains,
        &fixed_or_fill,
        &mins,
        &maxs,
        &grows,
    );
    if let Some(line) = dynamic {
        // A line that overflows gives up its items' declared elasticity,
        // cheapest first, before any shrink factor clips them.
        let inline = direction == FlexDirection::Row;
        line.pass.counters.contexts_considered += 1;
        let occupied = mains.iter().sum::<f32>() + margin_mains.iter().sum::<f32>() + gap_total;
        let overflow = LayoutUnits::from_px(occupied - content_main);
        if overflow.is_positive() {
            let own_gap = lower_box(
                line.profile.axis(inline),
                (0.0, 0.0),
                gap,
                n.saturating_sub(1),
                0.0,
                0,
            );
            let mut parts = Vec::new();
            for (index, child) in children.iter().enumerate() {
                if !content_sized[index] {
                    continue;
                }
                let shape = line.pass.envelope(
                    *child,
                    inline,
                    edge_base,
                    parent_font_px,
                    nodes,
                    line.retained,
                );
                if shape.is_rigid() {
                    continue;
                }
                parts.push(LineChild {
                    index,
                    id: *child,
                    shape,
                    limit: LayoutUnits::from_px(mains[index] - mins[index]),
                });
            }
            if !parts.is_empty() || !own_gap.is_rigid() {
                let (per_child, own) = line.pass.solve_line(
                    line.container,
                    own_gap,
                    &parts,
                    overflow,
                    line.retained,
                    line.commit,
                );
                adjust.amounts = vec![LayoutUnits::ZERO; n];
                for (part, amount) in parts.iter().zip(&per_child) {
                    mains[part.index] -= amount.to_px();
                    adjust.amounts[part.index] = *amount;
                }
                if n > 1 && own.is_positive() {
                    adjust.gap = (gap - own.to_px() / (n - 1) as f32).max(0.0);
                    gap_total = adjust.gap * (n - 1) as f32;
                }
            }
        } else {
            line.pass.counters.fast_fit += 1;
        }
        if line.commit {
            line.pass.commit_line(
                line.container,
                children,
                &adjust.amounts,
                inline,
                line.retained,
            );
        }
    }
    apply_flex_shrink(
        content_main,
        gap_total,
        &margin_mains,
        &mut mains,
        &mins,
        &shrinks,
    );
    for (size, main) in sizes.iter_mut().zip(mains) {
        set_main_extent(size, direction, main);
    }
    adjust
}

pub(super) fn resolve_flex_fill_sizes(
    content_main: f32,
    gap_total: f32,
    margin_mains: &[f32],
    fixed_or_fill: &[Option<f32>],
    mins: &[f32],
    maxs: &[Option<f32>],
    grows: &[f32],
) -> Vec<f32> {
    let n = fixed_or_fill.len();
    let mut sizes = vec![0.0f32; n];
    let mut active: Vec<(usize, f32)> = Vec::new();
    let mut occupied = gap_total;
    for i in 0..n {
        occupied += margin_mains[i];
        if let Some(width) = fixed_or_fill[i] {
            sizes[i] = width.max(0.0);
            occupied += sizes[i];
        } else {
            active.push((i, grows[i].max(0.0)));
        }
    }
    let mut free = (content_main - occupied).max(0.0);
    loop {
        if active.is_empty() {
            break;
        }
        let fr_total: f32 = active.iter().map(|(_, weight)| *weight).sum();
        if fr_total <= 1e-6 {
            let share = free / active.len() as f32;
            let mut freeze: Vec<(usize, f32)> = Vec::new();
            for (fi, &(ci, _)) in active.iter().enumerate() {
                let min = mins[ci].max(0.0);
                if share + 1e-3 < min {
                    freeze.push((fi, min));
                } else if let Some(max) = maxs[ci]
                    && share > max + 1e-3
                {
                    freeze.push((fi, max.max(0.0)));
                }
            }
            if freeze.is_empty() {
                for (ci, _) in active.drain(..) {
                    let mut width = share.max(mins[ci].max(0.0));
                    if let Some(max) = maxs[ci] {
                        width = width.min(max.max(0.0));
                    }
                    sizes[ci] = width;
                }
                break;
            }
            freeze.sort_by_key(|(fi, _)| *fi);
            for (fi, frozen) in freeze.into_iter().rev() {
                let (ci, _) = active.remove(fi);
                sizes[ci] = frozen;
                free = (free - frozen).max(0.0);
            }
            continue;
        }
        let mut freeze: Vec<(usize, f32)> = Vec::new();
        for (fi, &(ci, weight)) in active.iter().enumerate() {
            let share = free * (weight / fr_total);
            let min = mins[ci].max(0.0);
            if share + 1e-3 < min {
                freeze.push((fi, min));
            } else if let Some(max) = maxs[ci]
                && share > max + 1e-3
            {
                freeze.push((fi, max.max(0.0)));
            }
        }
        if freeze.is_empty() {
            for (ci, weight) in active.drain(..) {
                let mut width = (free * (weight / fr_total)).max(mins[ci].max(0.0));
                if let Some(max) = maxs[ci] {
                    width = width.min(max.max(0.0));
                }
                sizes[ci] = width;
            }
            break;
        }
        freeze.sort_by_key(|(fi, _)| *fi);
        for (fi, frozen) in freeze.into_iter().rev() {
            let (ci, _) = active.remove(fi);
            sizes[ci] = frozen;
            free = (free - frozen).max(0.0);
        }
    }
    sizes
}

pub(super) fn apply_flex_shrink(
    content_main: f32,
    gap_total: f32,
    margin_mains: &[f32],
    sizes: &mut [f32],
    mins: &[f32],
    shrinks: &[f32],
) {
    if content_main <= 1e-3 {
        return;
    }
    let margin_total: f32 = margin_mains.iter().copied().sum();
    let used = sizes.iter().sum::<f32>() + margin_total + gap_total;
    let mut overflow = used - content_main;
    if overflow <= 1e-3 {
        return;
    }
    let mut active: Vec<usize> = (0..sizes.len())
        .filter(|&i| shrinks[i] > 1e-6 && sizes[i] > mins[i].max(0.0) + 1e-3)
        .collect();
    loop {
        if active.is_empty() || overflow <= 1e-3 {
            break;
        }
        let fr_total: f32 = active
            .iter()
            .map(|&i| shrinks[i].max(0.0) * sizes[i].max(0.0))
            .sum();
        if fr_total <= 1e-6 {
            break;
        }
        let mut freeze: Vec<(usize, f32)> = Vec::new();
        for (fi, &ci) in active.iter().enumerate() {
            let factor = shrinks[ci].max(0.0) * sizes[ci].max(0.0);
            let reduction = overflow * (factor / fr_total);
            let min = mins[ci].max(0.0);
            if sizes[ci] - reduction + 1e-3 < min {
                freeze.push((fi, min));
            }
        }
        if freeze.is_empty() {
            for &ci in &active {
                let factor = shrinks[ci].max(0.0) * sizes[ci].max(0.0);
                let reduction = overflow * (factor / fr_total);
                sizes[ci] = (sizes[ci] - reduction).max(mins[ci].max(0.0));
            }
            break;
        }
        freeze.sort_by_key(|(fi, _)| *fi);
        for (fi, frozen_min) in freeze.into_iter().rev() {
            let ci = active.remove(fi);
            let reduced = (sizes[ci] - frozen_min).max(0.0);
            sizes[ci] = frozen_min;
            overflow = (overflow - reduced).max(0.0);
        }
    }
}

pub(super) fn main_occupied(
    children: &[StableNodeId],
    sizes: &[Size],
    direction: FlexDirection,
    // Percent base of the children's margins: the container's inline size.
    edge_base: f32,
    gap: f32,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
) -> f32 {
    let mut occupied = 0.0;
    for (id, size) in children.iter().zip(sizes) {
        let margin = match nodes.style(*id) {
            Some(style) => style.resolved_margin_against_fonts(
                Some(edge_base),
                fonts_of(style.as_ref(), parent_font_px),
            ),
            None => Default::default(),
        };
        occupied += main_extent(*size, direction)
            + main_start_margin(margin, direction)
            + main_end_margin(margin, direction);
    }
    occupied + gap * children.len().saturating_sub(1) as f32
}

pub(super) fn justify_offsets(
    justify: JustifySpec,
    available: f32,
    occupied: f32,
    base_gap: f32,
    count: usize,
) -> (f32, f32) {
    let free = (available - occupied).max(0.0);
    match justify {
        JustifySpec::Start | JustifySpec::Stretch => (0.0, base_gap),
        JustifySpec::Center => (free / 2.0, base_gap),
        JustifySpec::End => (free, base_gap),
        JustifySpec::SpaceBetween if count > 1 => (0.0, base_gap + free / (count - 1) as f32),
        JustifySpec::SpaceAround if count > 0 => {
            let extra = free / count as f32;
            (extra / 2.0, base_gap + extra)
        }
        JustifySpec::SpaceEvenly if count > 0 => {
            let extra = free / (count + 1) as f32;
            (extra, base_gap + extra)
        }
        _ => (0.0, base_gap),
    }
}

pub(super) fn clear_offset(clear: ClearSpec, left_bottom: f32, right_bottom: f32) -> f32 {
    match clear {
        ClearSpec::None => 0.0,
        ClearSpec::Left => left_bottom,
        ClearSpec::Right => right_bottom,
        ClearSpec::Both => left_bottom.max(right_bottom),
    }
}

pub(super) fn count_auto_main_margins(
    line: &[StableNodeId],
    direction: FlexDirection,
    nodes: &LayoutInputMap<'_>,
) -> usize {
    line.iter()
        .map(|id| {
            let Some(style) = nodes.style(*id) else {
                return 0;
            };
            match direction {
                FlexDirection::Row => {
                    usize::from(style.margin_auto_left()) + usize::from(style.margin_auto_right())
                }
                FlexDirection::Column => {
                    usize::from(style.margin_auto_top()) + usize::from(style.margin_auto_bottom())
                }
            }
        })
        .sum()
}

pub(super) fn apply_auto_margins(
    style: &LayoutStyle,
    direction: FlexDirection,
    margin: &mut nana_ui_core::PaddingSpec,
    auto_main_share: f32,
    line_cross: f32,
    child_size: Size,
) {
    match direction {
        FlexDirection::Row => {
            if style.margin_auto_left() {
                margin.left += auto_main_share;
            }
            if style.margin_auto_right() {
                margin.right += auto_main_share;
            }
            let used = child_size.height + margin.top + margin.bottom;
            let free = (line_cross - used).max(0.0);
            match (style.margin_auto_top(), style.margin_auto_bottom()) {
                (true, true) => {
                    margin.top += free / 2.0;
                    margin.bottom += free / 2.0;
                }
                (true, false) => margin.top += free,
                (false, true) => margin.bottom += free,
                (false, false) => {}
            }
        }
        FlexDirection::Column => {
            if style.margin_auto_top() {
                margin.top += auto_main_share;
            }
            if style.margin_auto_bottom() {
                margin.bottom += auto_main_share;
            }
            let used = child_size.width + margin.left + margin.right;
            let free = (line_cross - used).max(0.0);
            match (style.margin_auto_left(), style.margin_auto_right()) {
                (true, true) => {
                    margin.left += free / 2.0;
                    margin.right += free / 2.0;
                }
                (true, false) => margin.left += free,
                (false, true) => margin.right += free,
                (false, false) => {}
            }
        }
    }
}
