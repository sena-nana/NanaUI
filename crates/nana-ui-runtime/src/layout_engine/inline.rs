//! Shared Runtime layout inline algorithms.

use super::*;

/// Line construction, BiDi, clusters and ellipsis belong to nana-text.
/// This adapter only places the same retained atomic nodes its objects name.
pub(super) fn place_native_inline(
    id: StableNodeId,
    children: &[StableNodeId],
    origin: Point,
    content: Size,
    viewport: LayoutViewport,
    font: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    cache.execution_stats.inline_reflows += 1;
    let layout = nodes
        .world
        .text_layout(id)
        .map(|(_, layout)| Arc::clone(layout))
        .ok_or(UiWorldError::InlineTextEngineRequired(id))?;
    let bounds = LayoutBox {
        x: origin.x,
        y: origin.y,
        width: content.width,
        height: content.height,
    };
    // The content box and the border box from the box's own origin, as
    // everything under it is placed.
    let own = output.get(&id).map_or(bounds, |placed| LayoutBox {
        x: 0.0,
        y: 0.0,
        ..*placed
    });
    let parts = nodes.world.solve_content_parts_at(id, bounds, own);
    let text_box =
        super::parts::part_bounds(&parts, crate::LayoutPartKind::TextContent).unwrap_or(bounds);
    let anchor = nodes.world.native_text_origin(id, text_box, &layout);
    let positions: HashMap<_, _> = layout
        .objects
        .iter()
        .map(|object| (object.offset, layout.page_rect(object.rect, text_box.width)))
        .collect();
    let atoms = Arc::clone(
        &nodes
            .world
            .inline_record(id)
            .expect("mixed paragraph")
            .atoms,
    );
    let by_node: HashMap<_, _> = atoms.iter().map(|atom| (atom.node, atom.offset)).collect();
    let mut measured = HashMap::with_capacity(atoms.len());
    for &child in children {
        if nodes
            .style(child)
            .is_some_and(|style| style.position.is_out_of_flow())
        {
            super::placement::place_positioned_child(
                child,
                origin,
                content,
                viewport,
                font,
                nodes.world.layout_writing(id),
                nodes,
                cache,
                output,
                scope,
            )?;
            continue;
        }
        cache.execution_stats.inline_atomic_measure_requests += 1;
        cache.note_child_measured();
        let size =
            intrinsic_size_scoped(child, content, None, viewport, font, nodes, cache, scope)?;
        let writing = nodes.world.layout_writing(id);
        let margin = nodes
            .world
            .inline_object_margin(child, writing.inline_size(content.width, content.height));
        prime_atom_baselines(
            [(child, size)],
            writing.inline_size(content.width, content.height),
            viewport,
            font,
            nodes,
            cache,
            scope,
        )?;
        let parent_strut = nodes.world.inline_parent_strut(id, font);
        let (object_metrics, _) = nodes.atom_line_metrics(
            child,
            writing,
            size,
            margin,
            font,
            writing.inline_size(content.width, content.height),
            parent_strut,
            viewport,
        );
        measured.insert(child, object_metrics);
        let rect = by_node
            .get(&child)
            .and_then(|offset| positions.get(offset))
            .copied();
        // Omitted objects (line-clamp/ellipsis) keep identity and state but
        // have no physical box in this formatting result.
        let size = if rect.is_some() {
            size
        } else {
            Size::default()
        };
        let offset = rect.map_or(Point::ZERO, |rect| Point {
            x: rect.x + margin.left,
            y: rect.y + margin.top,
        });
        place_node_scoped(
            child,
            Point {
                x: anchor[0] + offset.x,
                y: anchor[1] + offset.y,
            },
            size,
            content,
            viewport,
            font,
            None,
            nodes,
            cache,
            output,
            scope,
        )?;
    }
    nodes.inline_objects.insert(
        id,
        atoms
            .iter()
            .map(|atom| nana_text::InlineObject {
                offset: atom.offset,
                id: atom.node.get(),
                metrics: measured.get(&atom.node).copied().unwrap_or(
                    nana_text::InlineObjectMetrics {
                        width_px: 0.0,
                        ascent_px: 0.0,
                        descent_px: 0.0,
                        align: nana_text::InlineObjectAlign::Baseline,
                        baseline_shift_px: 0.0,
                        envelope: None,
                    },
                ),
            })
            .collect::<Vec<_>>()
            .into(),
    );
    Ok(())
}

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
        let Some(style) = nodes.style_ref(*child) else {
            continue;
        };
        let block_break = break_on_blocks
            && nodes.participation(*child, &style, crate::FormattingContextKind::Inline)
                == crate::ParticipationKind::FlexItem;
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
    writing: nana_ui_core::WritingContext,
    content: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
) -> (f32, f32) {
    let direction = writing.inline_flex_direction();
    let inline_size = writing.inline_size(content.width, content.height);
    let margin =
        style.resolved_margin_against_fonts(Some(inline_size), fonts_of(style, parent_font_px));
    let main = packing_main_size(
        style,
        size,
        direction,
        inline_size,
        inline_size,
        viewport,
        parent_font_px,
        None,
    );
    let outer_main =
        main + main_start_margin(margin, direction) + main_end_margin(margin, direction);
    let outer_cross = cross_extent(size, direction) + cross_margin(margin, direction);
    (outer_main, outer_cross)
}

/// An inline-level item's ascent and descent about the baseline of a
/// line of an ordinary block, margins included
/// ([`LayoutInputMap::atom_line_metrics`]). `None` for a block-level item,
/// which holds a line alone and sits at its start.
#[allow(clippy::too_many_arguments)]
pub(super) fn ifc_atom_metrics(
    child: StableNodeId,
    style: &LayoutStyle,
    size: Size,
    writing: nana_ui_core::WritingContext,
    inline_base: f32,
    parent_font_px: f32,
    parent_strut: crate::inline_content::ParentLineStrut,
    viewport: LayoutViewport,
    nodes: &LayoutInputMap<'_>,
) -> Option<crate::inline_content::AtomLinePlacement> {
    if !style.is_inline_level() {
        return None;
    }
    let margin =
        style.resolved_margin_against_fonts(Some(inline_base), fonts_of(style, parent_font_px));
    let (_, placement) = nodes.atom_line_metrics(
        child,
        writing,
        size,
        margin,
        parent_font_px,
        inline_base,
        parent_strut,
        viewport,
    );
    Some(placement)
}

/// Derives, for each atom among `items` that sits on its line by its last
/// line box while every line box it holds is a descendant's, that baseline
/// at the size the atom takes on the line ([`descendant_baselines`]).
/// [`LayoutInputMap::atom_line_metrics`] reads it from there, so every line
/// that sits the atom, measured or placed, reads one value. `inline_base`
/// is the line's containing block's inline size.
#[allow(clippy::too_many_arguments)]
pub(super) fn prime_atom_baselines(
    items: impl IntoIterator<Item = (StableNodeId, Size)>,
    inline_base: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    for (child, size) in items {
        if nodes.atom_sits_on_line_box(child) && nodes.line_boxes_are_descendants(child) {
            descendant_baselines(
                child,
                size,
                inline_base,
                viewport,
                parent_font_px,
                nodes,
                cache,
                scope,
            )?;
        }
    }
    Ok(())
}

/// Derive, for each item of a flex line that aligns it by its first
/// baseline while every line box it holds is a descendant's, its baselines
/// at the size it takes on the line ([`descendant_baselines`]), so the
/// line, measured or placed, and the intrinsic facts it publishes read one
/// value ([`LayoutInputMap::baseline_metrics`]). `inline_base` is the flex
/// container's inline size.
#[allow(clippy::too_many_arguments)]
pub(super) fn prime_first_baselines(
    items: impl IntoIterator<Item = (StableNodeId, Size)>,
    inline_base: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    for (item, size) in items {
        if nodes.world.aligns_by_first_baseline(item) && nodes.line_boxes_are_descendants(item) {
            descendant_baselines(
                item,
                size,
                inline_base,
                viewport,
                parent_font_px,
                nodes,
                cache,
                scope,
            )?;
        }
    }
    Ok(())
}

/// The first and last baselines of `id` at border-box size `used`, from
/// its top, when every line box it holds is a descendant's (CSS 2.1
/// §10.8.1, CSS Inline 3 §4.2.1): the baselines of its first and its last
/// in-flow line box, where those lines sit inside it.
///
/// It reads where placement puts the box's children ([`arrange_flow`]),
/// from the sizes they measure at its content box, rather than laying them
/// out again. A block's first line and its last are each either a line its
/// inline-level items sit on, or a block holding a line, whose own first or
/// last baseline counts (derived again if its lines are its descendants'),
/// looked for in the flow's order. A flex container's are the baseline of
/// the line and item Chrome takes ([`flex_first_baseline`], #394), first
/// and last alike, and a grid container's those of its first row
/// ([`grid_first_baseline`]). Recorded per pass in
/// `LayoutInputMap::derived_baselines`; both `None` for a subgrid or a
/// vertical box, which keep the approximation of `baseline_metrics`.
#[allow(clippy::too_many_arguments)]
pub(super) fn descendant_baselines(
    id: StableNodeId,
    used: Size,
    inline_base: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<BaselineMetrics, UiWorldError> {
    let key = derived_baseline_key(id, used);
    if let Some(known) = nodes.derived_baselines.get(&key) {
        return Ok(*known);
    }
    let derived = derive_baselines(
        id,
        used,
        inline_base,
        viewport,
        parent_font_px,
        nodes,
        cache,
        scope,
    )?;
    nodes.derived_baselines.insert(key, derived);
    Ok(derived)
}

#[allow(clippy::too_many_arguments)]
fn derive_baselines(
    id: StableNodeId,
    used: Size,
    inline_base: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<BaselineMetrics, UiWorldError> {
    let Some(style) = nodes.style(id) else {
        return Ok(BaselineMetrics::default());
    };
    let Some(node) = nodes.get(id)? else {
        return Ok(BaselineMetrics::default());
    };
    let (child_ids, writing) = (node.children, node.writing);
    // A subgrid's tracks are its parent's.
    if style.omits_box() || style.is_subgrid_columns() || style.is_subgrid_rows() {
        return Ok(BaselineMetrics::default());
    }
    let flex = style.display.is_some_and(DisplaySpec::is_flex_container);
    let fonts = fonts_of(&style, parent_font_px);
    let child_font_px = fonts.element_px;
    let padding = style.resolved_padding_against_fonts(Some(inline_base), fonts);
    let border = style.resolved_border_edges();
    let top = grid::margin_at(border, writing.block_start())
        + grid::margin_at(padding, writing.block_start());
    // The content box placement lays the children out in.
    let content = Size::new(
        used.width - padding.left - padding.right - border.left - border.right,
        used.height - padding.top - padding.bottom - border.top - border.bottom,
    );
    let (mut flow, _) = collect_flow_children_reporting(child_ids, nodes, &style)?;
    sort_by_order(&mut flow, nodes);
    let context = establish_context(id, &style, &flow, nodes);
    if context.grid_2d {
        let direction = used_flow_direction(&style, writing, false);
        let mut sizes = Vec::with_capacity(flow.len());
        for &child in &flow {
            let available = nodes.style_ref(child).map_or(content, |child_style| {
                grid_item_measure_available(&child_style, content)
            });
            sizes.push(intrinsic_size_scoped(
                child,
                available,
                Some(direction),
                viewport,
                child_font_px,
                nodes,
                cache,
                scope,
            )?);
        }
        let mut solved = layout_grid_2d(
            &style, writing, &flow, &sizes, content, fonts, None, viewport, nodes, cache, scope,
        )?;
        solved.fill_layer_cell(&style, writing, content);
        let arranged = arrange_grid_2d(
            &solved,
            writing,
            content,
            &style,
            viewport,
            child_font_px,
            nodes,
            cache,
            scope,
            None,
        )?;
        let first = grid_first_baseline(
            &arranged,
            writing,
            content,
            viewport,
            child_font_px,
            nodes,
            cache,
            scope,
        )?
        .map(|baseline| top + baseline);
        // Like a flex container's, its last baseline is its first.
        return Ok(BaselineMetrics { first, last: first });
    }
    let ifc = context.inline();
    let direction = used_flow_direction(&style, writing, ifc);
    let child_edge_base = writing.inline_size(content.width, content.height);
    let parent_box = gap_containing_block(&style, content);
    let gap = style.main_gap_against_fonts(direction, parent_box, fonts);
    let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
    let mut sizes = Vec::with_capacity(flow.len());
    for &child in &flow {
        sizes.push(intrinsic_size_scoped(
            child,
            content,
            Some(direction),
            viewport,
            child_font_px,
            nodes,
            cache,
            scope,
        )?);
    }
    // Where placement puts every child: one arrangement, read here rather
    // than laid out again.
    let arranged = arrange_flow(
        id,
        &style,
        writing,
        &flow,
        &sizes,
        Point { x: 0.0, y: 0.0 },
        content,
        child_edge_base,
        gap,
        cross_gap,
        ifc,
        direction,
        flow_axes_reversed(&style, writing, direction, ifc, false),
        viewport,
        child_font_px,
        nodes,
        cache,
        scope,
        false,
    )?;
    if flex || context.kind == crate::FormattingContextKind::Grid {
        let first = flex_first_baseline(
            &arranged,
            direction,
            writing,
            content,
            child_edge_base,
            viewport,
            child_font_px,
            nodes,
            cache,
            scope,
        )?
        .map(|baseline| top + baseline);
        // A flex container's last baseline, where a block's last line box
        // is looked for, is its first: what Chrome aligns an inline-block
        // ending in one by.
        return Ok(BaselineMetrics { first, last: first });
    }
    // A block's first line box forward, its last back, in the flow's order:
    // a line its inline-level items sit on, or a block on it with a
    // baseline of its own.
    let mut found = BaselineMetrics::default();
    for which in [crate::Baseline::First, crate::Baseline::Last] {
        let mut order: Vec<usize> = (0..arranged.lines.len()).collect();
        if which == crate::Baseline::Last {
            order.reverse();
        }
        'lines: for index in order {
            let line = &arranged.lines[index];
            if let Some(baseline) = line.atom_baseline {
                if !line.children.is_empty() {
                    found.set(which, top + line.cross_start + baseline);
                    break;
                }
                continue;
            }
            let mut items: Vec<usize> = line.children.clone().collect();
            if which == crate::Baseline::Last {
                items.reverse();
            }
            for item in items {
                let item = &arranged.children[item];
                if let Some(baseline) = block_baseline(
                    item.child,
                    item.size,
                    which,
                    child_edge_base,
                    viewport,
                    child_font_px,
                    nodes,
                    cache,
                    scope,
                )? {
                    let item_block_start = writing.block_origin_offset(
                        (item.offset.x, item.offset.y),
                        (item.size.width, item.size.height),
                        (content.width, content.height),
                    );
                    found.set(which, top + item_block_start + baseline);
                    break 'lines;
                }
            }
        }
    }
    Ok(found)
}

#[allow(clippy::too_many_arguments)]
fn grid_first_baseline(
    arranged: &[ArrangedGridItem],
    writing: nana_ui_core::WritingContext,
    content: Size,
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Option<f32>, UiWorldError> {
    let Some(row) = arranged.iter().map(|item| item.row).min() else {
        return Ok(None);
    };
    let first_row = || arranged.iter().filter(move |item| item.row == row);
    if let Some(shared) = first_row().find_map(|item| {
        item.baseline.map(|baseline| {
            writing.block_origin_offset(
                (item.offset.x, item.offset.y),
                (item.size.width, item.size.height),
                (content.width, content.height),
            ) + baseline
        })
    }) {
        return Ok(Some(shared));
    }
    let Some(item) = first_row().min_by_key(|item| item.col) else {
        return Ok(None);
    };
    let item_inline = writing.inline_size(item.cell.width, item.cell.height);
    let item_block = writing.logical_size(item.size.width, item.size.height).1;
    let kind = nodes.world.node_baseline_kind(item.id);
    let baseline = block_baseline(
        item.id,
        item.size,
        crate::Baseline::First,
        item_inline,
        viewport,
        child_font_px,
        nodes,
        cache,
        scope,
    )?
    .unwrap_or_else(|| {
        crate::inline_content::synthesized_block_baseline(item_block, writing, kind)
    });
    let item_block_start = writing.block_origin_offset(
        (item.offset.x, item.offset.y),
        (item.size.width, item.size.height),
        (content.width, content.height),
    );
    Ok(Some(item_block_start + baseline))
}

#[allow(clippy::too_many_arguments)]
fn flex_first_baseline(
    arranged: &ArrangedFlow,
    direction: FlexDirection,
    writing: nana_ui_core::WritingContext,
    content: Size,
    inline_base: f32,
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Option<f32>, UiWorldError> {
    let main_is_inline = direction == writing.inline_flex_direction();
    // The first line is the one nearest the cross start of the page, which
    // under `wrap-reverse` is the flow's last.
    let page_cross_start = |line: &&super::placement::ArrangedLine| {
        arranged.children[line.children.clone()]
            .iter()
            .map(|item| {
                let (offset, size) = (
                    (item.offset.x, item.offset.y),
                    (item.size.width, item.size.height),
                );
                if main_is_inline {
                    writing.block_origin_offset(offset, size, (content.width, content.height))
                } else {
                    writing.inline_start_coord(offset, size)
                }
            })
            .fold(f32::INFINITY, f32::min)
    };
    let Some(line) = arranged
        .lines
        .iter()
        .filter(|line| !line.children.is_empty())
        .min_by(|a, b| page_cross_start(a).total_cmp(&page_cross_start(b)))
    else {
        return Ok(None);
    };
    let items = &arranged.children[line.children.clone()];
    if main_is_inline
        && let Some(shared) = items.iter().find_map(|item| {
            item.baseline.map(|baseline| {
                writing.block_origin_offset(
                    (item.offset.x, item.offset.y),
                    (item.size.width, item.size.height),
                    (content.width, content.height),
                ) + baseline
            })
        })
    {
        return Ok(Some(shared));
    }
    let Some(item) =
        items.iter().min_by(|a, b| {
            if main_is_inline {
                writing
                    .inline_start_coord((a.offset.x, a.offset.y), (a.size.width, a.size.height))
                    .total_cmp(&writing.inline_start_coord(
                        (b.offset.x, b.offset.y),
                        (b.size.width, b.size.height),
                    ))
            } else {
                writing
                    .block_origin_offset(
                        (a.offset.x, a.offset.y),
                        (a.size.width, a.size.height),
                        (content.width, content.height),
                    )
                    .total_cmp(&writing.block_origin_offset(
                        (b.offset.x, b.offset.y),
                        (b.size.width, b.size.height),
                        (content.width, content.height),
                    ))
            }
        })
    else {
        return Ok(None);
    };
    let item_block = writing.logical_size(item.size.width, item.size.height).1;
    let kind = nodes.world.node_baseline_kind(item.child);
    let baseline = block_baseline(
        item.child,
        item.size,
        crate::Baseline::First,
        inline_base,
        viewport,
        child_font_px,
        nodes,
        cache,
        scope,
    )?
    .unwrap_or_else(|| {
        crate::inline_content::synthesized_block_baseline(item_block, writing, kind)
    });
    let item_block_start = writing.block_origin_offset(
        (item.offset.x, item.offset.y),
        (item.size.width, item.size.height),
        (content.width, content.height),
    );
    Ok(Some(item_block_start + baseline))
}

/// The first or last baseline of block-level `child` of a box's flow, from
/// its top, or `None` when it holds no line box: replaced content, or a box
/// with no line box in it.
#[allow(clippy::too_many_arguments)]
fn block_baseline(
    child: StableNodeId,
    size: Size,
    which: crate::Baseline,
    inline_base: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Option<f32>, UiWorldError> {
    if nodes.line_boxes_are_descendants(child) {
        let derived = descendant_baselines(
            child,
            size,
            inline_base,
            viewport,
            parent_font_px,
            nodes,
            cache,
            scope,
        )?
        .get(which);
        if derived.is_some() {
            return Ok(derived);
        }
    } else if !nodes.world.lays_out_own_line(child)
        || nodes
            .style(child)
            .is_none_or(|style| nodes.world.aligns_by_bottom_edge(child, &style))
    {
        return Ok(None);
    }
    let metrics = nodes.baseline_metrics(child, parent_font_px, Some(inline_base), Some(size));
    Ok(match which {
        crate::Baseline::First => metrics.first,
        crate::Baseline::Last => metrics.last.or(metrics.first),
    })
}

/// The strut a line box of block `id` starts from (CSS 2.1 §10.8.1): the
/// ascent and descent of a zero-width inline box in the block's own font at
/// its own line height, so an atom that sits by its bottom edge leaves the
/// font's descent below it. The face's ascent and descent are the ones the
/// text pass read for the block (`UiWorld::block_strut`), which a native
/// paragraph in the same face starts its lines from too; under a shaper that
/// reports none they are the shared approximation
/// ([`nana_ui_core::TEXT_APPROX_ASCENT_EM`] of the em, the rest below). The
/// half-leading splits what the line height leaves around them.
pub(super) fn line_strut(
    id: StableNodeId,
    style: &LayoutStyle,
    writing: nana_ui_core::WritingContext,
    font_px: f32,
    nodes: &LayoutInputMap<'_>,
) -> LineStrut {
    let font = font_px.max(0.0);
    let (font_ascent, font_descent) = nodes.world.block_strut(id).map_or_else(
        || {
            let ascent = font * nana_ui_core::TEXT_APPROX_ASCENT_EM;
            (ascent, font - ascent)
        },
        |strut| (font * strut.ascent, font * strut.descent),
    );
    let x_height = nodes
        .world
        .block_x_height(id)
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
        .map_or(font * 0.5, |ratio| font * ratio);
    let line_h = text_line_box_height_px(font, style.line_height);
    let half_leading = (line_h - (font_ascent + font_descent)) * 0.5;
    let dominant_baseline = nodes.world.node_baseline_kind(id);
    let ascent = if !writing.is_vertical() {
        match dominant_baseline {
            nana_ui_core::UsedBaselineKind::Alphabetic => font_ascent + half_leading,
            nana_ui_core::UsedBaselineKind::Central => line_h * 0.5,
            nana_ui_core::UsedBaselineKind::Ideographic => {
                (font_ascent + half_leading + font_descent).min(line_h)
            }
        }
    } else {
        match dominant_baseline {
            nana_ui_core::UsedBaselineKind::Central => line_h * 0.5,
            nana_ui_core::UsedBaselineKind::Alphabetic => {
                if writing.block_reversed() {
                    font_ascent + half_leading
                } else {
                    font_descent + half_leading
                }
            }
            nana_ui_core::UsedBaselineKind::Ideographic => {
                if writing.block_reversed() {
                    (line_h * 0.5 + 0.5 * (font_ascent + font_descent)).min(line_h)
                } else {
                    (line_h * 0.5 - 0.5 * (font_ascent + font_descent)).max(0.0)
                }
            }
        }
    };
    LineStrut {
        ascent,
        descent: line_h - ascent,
        font_ascent,
        font_descent,
        x_height,
        font_px: font,
        dominant_baseline,
    }
}

/// Shared baseline-aligned flex line cross-size computation (CSS Flexbox §9.4):
/// returns `Some((line_ascent, line_cross))` when `direction` aligns along the
/// inline axis and at least one item aligns by baseline, growing the line cross
/// size to `max_non_baseline.max(max_ascent + max_descent)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn flex_baseline_line_metrics(
    items: impl IntoIterator<Item = (StableNodeId, Size)>,
    container_style: &LayoutStyle,
    writing: nana_ui_core::WritingContext,
    direction: FlexDirection,
    cross_reversed: bool,
    far_baseline: bool,
    edge_base: f32,
    child_font_px: f32,
    nodes: &LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
) -> Option<(f32, f32)> {
    if direction != writing.inline_flex_direction() {
        return None;
    }
    let mut has_baseline = false;
    let mut max_ascent = 0.0f32;
    let mut max_descent = 0.0f32;
    let mut max_non_baseline = 0.0f32;
    for (child, size) in items {
        let Some(child_style) = nodes.style_ref(child) else {
            continue;
        };
        let fonts = fonts_of(child_style.as_ref(), child_font_px);
        let margin = child_style.resolved_margin_against_fonts(Some(edge_base), fonts);
        let lead = if cross_reversed {
            cross_end_margin(margin, direction)
        } else {
            cross_start_margin(margin, direction)
        };
        let trail = if cross_reversed {
            cross_start_margin(margin, direction)
        } else {
            cross_end_margin(margin, direction)
        };
        let item_cross = cross_extent(size, direction);
        if child_style.resolved_align_self(container_style.child_align_items())
            == AlignSpec::Baseline
        {
            has_baseline = true;
            let baseline = intrinsic
                .baseline(child, crate::Baseline::First)
                .unwrap_or_else(|| nodes.baseline(child, fonts.element_px, size))
                .max(child_style.resolved_border_width())
                .min(item_cross.max(child_style.resolved_border_width()));
            // Under `wrap-reverse` the line's cross-start is the item's far
            // side, so its baseline is measured from there.
            let baseline = if far_baseline {
                item_cross - baseline
            } else {
                baseline
            };
            max_ascent = max_ascent.max(lead + baseline);
            max_descent = max_descent.max((item_cross - baseline) + trail);
        } else {
            max_non_baseline = max_non_baseline.max(item_cross + lead + trail);
        }
    }
    has_baseline.then(|| (max_ascent, max_non_baseline.max(max_ascent + max_descent)))
}

/// A line box of an ordinary block: its baseline from `block-start`, and its
/// block extent. Its inline-level items sit on that baseline with the block's
/// strut, and the line spans the highest `block-start` to the furthest
/// `block-end` among them (CSS 2.1 §10.8); a block-level item holds a line
/// alone, counted from the line's `block-start`, with no strut.
pub(super) fn ifc_line_metrics(
    items: impl IntoIterator<Item = (StableNodeId, Size)>,
    writing: nana_ui_core::WritingContext,
    content: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    strut: LineStrut,
    nodes: &LayoutInputMap<'_>,
) -> (f32, f32) {
    let inline_size = writing.inline_size(content.width, content.height);
    let parent_strut = strut.into();
    let (mut baseline_above, mut baseline_below, mut max_top, mut max_bottom, mut from_top) =
        (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for (child, size) in items {
        let Some(style) = nodes.style_ref(child) else {
            continue;
        };
        match ifc_atom_metrics(
            child,
            &style,
            size,
            writing,
            inline_size,
            parent_font_px,
            parent_strut,
            viewport,
            nodes,
        ) {
            Some(crate::inline_content::AtomLinePlacement::Baseline { above, below }) => {
                baseline_above = baseline_above.max(above).max(strut.ascent);
                baseline_below = baseline_below.max(below).max(strut.descent);
            }
            Some(crate::inline_content::AtomLinePlacement::Top { height }) => {
                baseline_above = baseline_above.max(strut.ascent);
                baseline_below = baseline_below.max(strut.descent);
                max_top = max_top.max(height);
            }
            Some(crate::inline_content::AtomLinePlacement::Bottom { height }) => {
                baseline_above = baseline_above.max(strut.ascent);
                baseline_below = baseline_below.max(strut.descent);
                max_bottom = max_bottom.max(height);
            }
            None => {
                let (_, outer) =
                    ifc_item_outer(&style, size, writing, content, viewport, parent_font_px);
                from_top = from_top.max(outer);
            }
        }
    }
    let mut ascent = baseline_above;
    let mut height = baseline_above + baseline_below;
    if max_top > height {
        height = max_top;
    }
    if max_bottom > height {
        ascent += max_bottom - height;
        height = max_bottom;
    }
    (ascent, height.max(from_top))
}

/// IFC wrap using the existing content-width line packer, with per-line available
/// width reduced by sibling float occupancy (shrink-to-avoid-float).
#[allow(clippy::too_many_arguments)]
pub(super) fn pack_ifc_line_boxes(
    children: &[StableNodeId],
    sizes: &[Size],
    writing: nana_ui_core::WritingContext,
    content: Size,
    gap: f32,
    cross_gap: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    strut: LineStrut,
    nodes: &LayoutInputMap<'_>,
) -> Vec<LineBoxSlot> {
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut line_main = 0.0f32;
    let mut line_y = 0.0f32;
    let available = writing.inline_size(content.width, content.height);
    let flush = |lines: &mut Vec<LineBoxSlot>,
                 current: &mut Vec<usize>,
                 line_main: &mut f32,
                 line_y: &mut f32| {
        if current.is_empty() {
            return;
        }
        let (_, line_cross) = ifc_line_metrics(
            current.iter().map(|&index| (children[index], sizes[index])),
            writing,
            content,
            viewport,
            parent_font_px,
            strut,
            nodes,
        );
        lines.push(LineBoxSlot {
            indices: std::mem::take(current),
            main_start: 0.0,
            main_available: available,
            cross_y: *line_y,
            pin_cross: true,
            baseline_aligned: true,
        });
        *line_main = 0.0;
        *line_y += line_cross + cross_gap;
    };
    for (index, child) in children.iter().enumerate() {
        let Some(style) = nodes.style_ref(*child) else {
            continue;
        };
        let style_ref = style.as_ref();
        let block_break = !style_ref.is_inline_level();
        if block_break {
            flush(&mut lines, &mut current, &mut line_main, &mut line_y);
            let (_, outer_cross) = ifc_item_outer(
                style_ref,
                sizes[index],
                writing,
                content,
                viewport,
                parent_font_px,
            );
            lines.push(LineBoxSlot {
                indices: vec![index],
                main_start: 0.0,
                main_available: available,
                cross_y: line_y,
                pin_cross: true,
                baseline_aligned: false,
            });
            line_y += outer_cross + cross_gap;
            continue;
        }
        let (outer, _) = ifc_item_outer(
            style_ref,
            sizes[index],
            writing,
            content,
            viewport,
            parent_font_px,
        );
        let need = if current.is_empty() {
            outer
        } else {
            line_main + gap + outer
        };
        if !current.is_empty() && need > available + 0.5 {
            flush(&mut lines, &mut current, &mut line_main, &mut line_y);
        }
        if current.is_empty() {
            line_main = outer;
        } else {
            line_main += gap + outer;
        }
        current.push(index);
    }
    flush(&mut lines, &mut current, &mut line_main, &mut line_y);
    if lines.is_empty() {
        lines.push(LineBoxSlot {
            indices: Vec::new(),
            main_start: 0.0,
            main_available: available,
            cross_y: 0.0,
            pin_cross: true,
            baseline_aligned: false,
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
    nodes: &mut LayoutInputMap<'_>,
    writing: nana_ui_core::WritingContext,
    // The lines of an ordinary block, with its strut: inline-level items
    // share a baseline and a line is as tall as they span
    // ([`ifc_line_metrics`]).
    baseline_lines: Option<LineStrut>,
    // When measuring a wrapping flex container with baseline-aligned items,
    // pass container style + intrinsic cache so line cross size includes baseline growth.
    flex_baseline: Option<(
        &LayoutStyle,
        &mut PassIntrinsicCache,
        Option<&ScopeContext<'_>>,
    )>,
) -> Result<Size, UiWorldError> {
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
    let mut flex_baseline = flex_baseline;
    for (line_index, line) in lines.iter().enumerate() {
        let mut line_main = 0.0f32;
        let mut line_cross = 0.0f32;
        for (item_index, &index) in line.iter().enumerate() {
            let Some(style) = nodes.style_ref(children[index]) else {
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
        if let Some(strut) = baseline_lines {
            line_cross = ifc_line_metrics(
                line.iter().map(|&index| (children[index], sizes[index])),
                writing,
                available,
                viewport,
                parent_font_px,
                strut,
                nodes,
            )
            .1;
        } else if let Some((container_style, cache, scope)) = &mut flex_baseline
            && direction == writing.inline_flex_direction()
            && line.iter().any(|&index| {
                nodes.style_ref(children[index]).is_some_and(|child| {
                    child.resolved_align_self(container_style.child_align_items())
                        == AlignSpec::Baseline
                })
            })
        {
            let cache_ref: &mut PassIntrinsicCache = cache;
            prime_first_baselines(
                line.iter().map(|&index| (children[index], sizes[index])),
                edge_base,
                viewport,
                parent_font_px,
                nodes,
                cache_ref,
                *scope,
            )?;
            let cross_axis = match direction {
                FlexDirection::Row => FlexDirection::Column,
                FlexDirection::Column => FlexDirection::Row,
            };
            let cross_reversed =
                writing.physical_axis_reversed(cross_axis) ^ matches!(wrap, FlexWrap::WrapReverse);
            if let Some((_, grown)) = flex_baseline_line_metrics(
                line.iter().map(|&index| (children[index], sizes[index])),
                container_style,
                writing,
                direction,
                cross_reversed,
                matches!(wrap, FlexWrap::WrapReverse),
                edge_base,
                parent_font_px,
                nodes,
                cache_ref,
            ) {
                line_cross = line_cross.max(grown);
            }
        }
        max_main = max_main.max(line_main);
        cross += line_cross;
        if line_index + 1 < lines.len() {
            cross += cross_gap;
        }
    }
    Ok(match direction {
        FlexDirection::Row => Size::new(max_main, cross),
        FlexDirection::Column => Size::new(cross, max_main),
    })
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
        let Some(child_style) = nodes.style_ref(child) else {
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
    let sides = &*style.border_sides;
    style.padding.is_none()
        && style.padding_top.is_none()
        && style.padding_right.is_none()
        && style.padding_bottom.is_none()
        && style.padding_left.is_none()
        && style.border_width.is_none()
        && sides.top_width.is_none()
        && sides.right_width.is_none()
        && sides.bottom_width.is_none()
        && sides.left_width.is_none()
}

fn ifc_local_child(style: &LayoutStyle) -> bool {
    style.is_inline_level()
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

pub(super) struct IfcLineDeltaPrepared {
    flow: Vec<StableNodeId>,
    sizes: Vec<Size>,
    pub(super) queries: Vec<(usize, StableNodeId)>,
}

/// Recompute one inline formatting context from cached line contributions.
///
/// Returns `None` when a nested edit, a float, or a child whose size depends
/// on line membership means the break is not local. The caller then measures
/// this formatting context, not the document.
pub(super) fn ifc_line_measure_delta_begin(
    id: StableNodeId,
    plan: &MeasurePlan,
    nodes: &LayoutInputMap<'_>,
    scope: &ScopeContext<'_>,
) -> Option<IfcLineDeltaPrepared> {
    let style = plan.style.as_ref();
    if plan.sequential
        || plan.child_direction != FlexDirection::Row
        || !ifc_plan_style(style, plan.writing)
        || plan.text_metrics().is_some()
        || nodes.world.widget_visual_ref(id).is_some()
    {
        return None;
    }
    if !affected_edits_are_direct(id, plan, scope, nodes) {
        return None;
    }
    let mut flow = Vec::new();
    let mut sizes = Vec::new();
    for child in plan.children.iter().copied() {
        let entry = plan.entry(child)?;
        let Some(intrinsic) = entry.intrinsic.get() else {
            if scope.affected.contains(&child) {
                return None;
            }
            continue;
        };
        let child_style = entry.style.as_deref()?;
        if !ifc_local_child(child_style) {
            return None;
        }
        flow.push(child);
        sizes.push(intrinsic);
    }
    if flow.is_empty() {
        return None;
    }
    let mut queries = Vec::new();
    for (index, &child) in flow.iter().enumerate() {
        if !scope.affected.contains(&child) {
            continue;
        }
        let current = nodes.style_ref(child);
        let child_style = current.as_deref()?;
        if !ifc_local_child(child_style) {
            return None;
        }
        let cached_style = plan.entry(child).and_then(|entry| entry.style.as_ref());
        if !scope.measure.contains(&child) {
            if !retained_style_matches(current.as_ref(), cached_style) {
                return None;
            }
            continue;
        }
        queries.push((index, child));
    }
    if queries.is_empty() {
        return None;
    }
    Some(IfcLineDeltaPrepared {
        flow,
        sizes,
        queries,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn ifc_line_measure_delta_finish(
    id: StableNodeId,
    plan: &MeasurePlan,
    state: IfcLineDeltaPrepared,
    measured: &[Size],
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: &ScopeContext<'_>,
) -> Result<Option<Size>, UiWorldError> {
    let style = plan.style.as_ref();
    let IfcLineDeltaPrepared {
        flow,
        mut sizes,
        queries,
    } = state;
    let mut patched = false;
    for ((index, _), &size) in queries.into_iter().zip(measured) {
        if size != sizes[index] {
            sizes[index] = size;
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
    prime_atom_baselines(
        flow.iter().copied().zip(sizes.iter().copied()),
        edge_base,
        viewport,
        child_font_px,
        nodes,
        cache,
        Some(scope),
    )?;
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
        plan.writing,
        Some(line_strut(id, style, plan.writing, child_font_px, nodes)),
        None,
    )?;
    let mut size = plan.size;
    // The inline size is the definite line budget. Only the block size follows
    // the lines, and this plan has no padding or border to add back.
    if plan.writing.is_vertical() {
        size.width = content.width;
    } else {
        size.height = content.height;
    }
    let mut updated = plan.clone();
    updated.size = size;
    let parent_strut: crate::inline_content::ParentLineStrut =
        line_strut(id, style, plan.writing, child_font_px, nodes).into();
    for (child, measured) in flow.into_iter().zip(sizes) {
        let Ok(slot) = updated
            .entries
            .binary_search_by_key(&child, |entry| entry.child)
        else {
            continue;
        };
        updated.entries[slot].intrinsic = PackedOptSize::from_size(measured);
        if let Some(child_style) = nodes.style(child) {
            updated.entries[slot].baseline = PackedOptF32::from_opt(
                ifc_atom_metrics(
                    child,
                    &child_style,
                    measured,
                    plan.writing,
                    edge_base,
                    child_font_px,
                    parent_strut,
                    viewport,
                    nodes,
                )
                .and_then(|placement| match placement {
                    crate::inline_content::AtomLinePlacement::Baseline { above, .. } => Some(above),
                    crate::inline_content::AtomLinePlacement::Top { .. }
                    | crate::inline_content::AtomLinePlacement::Bottom { .. } => None,
                }),
            );
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
        let Some(style) = nodes.style_ref(direct) else {
            return false;
        };
        if !fixed_inline_block(nodes.world, direct, style.as_ref(), plan.style.as_ref()) {
            return false;
        }
    }
    true
}

/// A fixed inline-block whose parent is outside the frontier. Relayout starts
/// at its retained border box, so the outer line is not packed again.
/// `parent_padding`: the padding the parent's last placement used, whose
/// percentages resolved against the parent's own containing block.
pub(super) fn fixed_inline_block_island(
    world: &UiWorld,
    id: StableNodeId,
    border: LayoutBox,
    parent_box: LayoutBox,
    parent_padding: Option<nana_ui_core::PaddingSpec>,
) -> Option<(Point, Size, f32)> {
    let style = world.layout_style(id)?;
    let parent = world.parent_id(id)?;
    let parent_style = world.layout_style(parent)?;
    if !fixed_inline_block(world, id, style.as_ref(), parent_style.as_ref()) {
        return None;
    }
    let pad = parent_padding
        .unwrap_or_else(|| parent_style.resolved_padding_against(Some(parent_box.width)));
    let edge = parent_style.resolved_border_edges();
    // The parent's leading parts take their band out of its content box (#385).
    let lead = world
        .leading_parts(parent)
        .map_or(Size::default(), |lead| lead.inset());
    let containing = Size::new(
        (parent_box.width - pad.left - pad.right - edge.left - edge.right - lead.width).max(0.0),
        (parent_box.height - pad.top - pad.bottom - edge.top - edge.bottom - lead.height).max(0.0),
    );
    let font = element_font_px(world, parent);
    // The kept border box already carries the relative offset, which
    // placement adds to the origin it is given: hand it the box before it.
    let (relative_x, relative_y) = style.relative_offset_against_fonts(
        Some(containing.width),
        Some(containing.height),
        nana_ui_core::FontSizeContext::new(super::ROOT_FONT_PX, style.font_size.unwrap_or(font)),
    );
    Some((
        Point {
            x: border.x - relative_x,
            y: border.y - relative_y,
        },
        containing,
        font,
    ))
}

/// A fixed inline-block whose content cannot move its line: its border box
/// is the style's, and it sits on the line by an edge rather than by a
/// baseline its content puts there ([`UiWorld::atom_baseline_reads_content`]).
fn fixed_inline_block(
    world: &UiWorld,
    id: StableNodeId,
    style: &LayoutStyle,
    parent: &LayoutStyle,
) -> bool {
    matches!(
        style.display,
        Some(DisplaySpec::InlineFlex | DisplaySpec::InlineGrid)
    ) && !world.atom_baseline_reads_content(id)
        && matches!(style.width, Some(LengthSpec::Px(value)) if value.is_finite() && value >= 0.0)
        && matches!(style.height, Some(LengthSpec::Px(value)) if value.is_finite() && value >= 0.0)
        && style.min_width.is_none()
        && style.max_width.is_none()
        && style.min_height.is_none()
        && style.max_height.is_none()
        && style.resolved_align_self(parent.child_align_items()) != AlignSpec::Baseline
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
