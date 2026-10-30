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
    output: &mut HashMap<StableNodeId, LayoutBox>,
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
    let parts =
        nodes
            .world
            .solve_content_parts_at(id, bounds, output.get(&id).copied().unwrap_or(bounds));
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
            content.width,
            viewport,
            font,
            nodes,
            cache,
            scope,
        )?;
        measured.insert(
            child,
            nodes.atom_line_metrics(child, writing, size, margin, font, content.width),
        );
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
            nodes,
            cache,
            output,
            scope,
            None,
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
        let Some(style) = nodes.style(*child) else {
            continue;
        };
        let block_break = break_on_blocks
            && nodes.participation(*child, &style, crate::FormattingContextKind::Inline)
                == crate::ParticipationKind::FlowItem;
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

/// An inline-level item's ascent and descent about the baseline of a
/// horizontal line of an ordinary block, margins included
/// ([`LayoutInputMap::atom_line_metrics`]). `None` for a block-level item,
/// which holds a line alone and sits at its top.
pub(super) fn ifc_atom_metrics(
    child: StableNodeId,
    style: &LayoutStyle,
    size: Size,
    content_width: f32,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
) -> Option<(f32, f32)> {
    if !style.is_inline_level() {
        return None;
    }
    let margin =
        style.resolved_margin_against_fonts(Some(content_width), fonts_of(style, parent_font_px));
    let metrics = nodes.atom_line_metrics(
        child,
        nana_ui_core::WritingContext::HORIZONTAL_LTR,
        size,
        margin,
        parent_font_px,
        content_width,
    );
    Some((metrics.ascent_px, metrics.descent_px))
}

/// Derives, for each atom among `items` that sits on its line by its last
/// line box while every line box it holds is a descendant's, that baseline
/// at the size the atom takes on the line ([`descendant_last_baseline`]).
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
        if nodes.world.atom_sits_on_line_box(child) && nodes.world.line_boxes_are_descendants(child)
        {
            descendant_last_baseline(
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

/// The last baseline of `id` at border-box size `used`, from its top, when
/// every line box it holds is a descendant's (CSS 2.1 §10.8.1): the
/// baseline of its last in-flow line box, where that line sits inside it.
///
/// It lays the box's flow out the way placement does, from the sizes its
/// children measure at its content box: a block flow stacks them, an inline
/// formatting context packs its lines with [`pack_ifc_line_boxes`] and sits
/// each on its baseline ([`ifc_line_metrics`]). The last line is either such
/// a line, or a block holding a line alone, whose own last baseline counts
/// (derived again if its lines are its descendants'). Recorded per pass in
/// `LayoutInputMap::derived_baselines`; `None` where the box lays its flow
/// out otherwise (a flex or grid container, a reversed or wrapping block
/// flow, floats beside its lines), which keeps the approximation of
/// `baseline_metrics`.
#[allow(clippy::too_many_arguments)]
pub(super) fn descendant_last_baseline(
    id: StableNodeId,
    used: Size,
    inline_base: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Option<f32>, UiWorldError> {
    let key = derived_baseline_key(id, used);
    if let Some(known) = nodes.derived_baselines.get(&key) {
        return Ok(*known);
    }
    let derived = derive_last_baseline(
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
fn derive_last_baseline(
    id: StableNodeId,
    used: Size,
    inline_base: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Option<f32>, UiWorldError> {
    let Some(style) = nodes.style(id) else {
        return Ok(None);
    };
    let Some(node) = nodes.get(id)? else {
        return Ok(None);
    };
    let (child_ids, writing) = (Arc::clone(&node.children), node.writing);
    if writing.is_vertical()
        || style.omits_box()
        || style
            .display
            .is_some_and(|display| display.is_flex_container() || display.is_grid_container())
    {
        return Ok(None);
    }
    let fonts = fonts_of(&style, parent_font_px);
    let child_font_px = fonts.element_px;
    let padding = style.resolved_padding_against_fonts(Some(inline_base), fonts);
    let border = style.resolved_border_edges();
    let top = border.top + padding.top;
    // The content box placement lays the children out in.
    let content = Size::new(
        used.width - padding.left - padding.right - border.left - border.right,
        used.height - padding.top - padding.bottom - border.top - border.bottom,
    );
    let (mut flow, _) = collect_flow_children_reporting(&child_ids, nodes, &style)?;
    if flow
        .iter()
        .any(|&child| nodes.style(child).is_some_and(|child| child.is_floated()))
    {
        return Ok(None);
    }
    sort_by_order(&mut flow, nodes);
    let context = establish_context(id, &style, &flow, nodes);
    let ifc = context.inline();
    if context.grid_2d || !(ifc || matches!(context.kind, crate::FormattingContextKind::Flow)) {
        return Ok(None);
    }
    let direction = used_flow_direction(&style, writing, ifc);
    if !ifc
        && (direction != FlexDirection::Column
            || matches!(style.flex_wrap, FlexWrap::Wrap | FlexWrap::WrapReverse)
            || style.justify_content != JustifySpec::Start
            || style
                .active_grid_rows()
                .is_some_and(|tracks| !tracks.is_empty()))
    {
        return Ok(None);
    }
    let parent_box = gap_containing_block(&style, content);
    let gap = style.main_gap_against_fonts(direction, parent_box, fonts);
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
    let margin_of = |child: StableNodeId, nodes: &LayoutInputMap<'_>| {
        nodes
            .style(child)
            .map(|child_style| {
                child_style.resolved_margin_against_fonts(
                    Some(content.width),
                    fonts_of(&child_style, child_font_px),
                )
            })
            .unwrap_or_default()
    };
    if ifc {
        prime_atom_baselines(
            flow.iter().copied().zip(sizes.iter().copied()),
            content.width,
            viewport,
            child_font_px,
            nodes,
            cache,
            scope,
        )?;
        let strut = line_strut(id, &style, child_font_px, nodes);
        let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
        let lines = pack_ifc_line_boxes(
            &flow,
            &sizes,
            Point { x: 0.0, y: 0.0 },
            content.width,
            gap,
            cross_gap,
            viewport,
            child_font_px,
            strut,
            nodes,
            &PackedFloats::default(),
        );
        for line in lines.iter().rev() {
            let Some(&first) = line.indices.first() else {
                continue;
            };
            if line.baseline_aligned {
                let (ascent, _) = ifc_line_metrics(
                    line.indices
                        .iter()
                        .map(|&index| (flow[index], sizes[index])),
                    content.width,
                    viewport,
                    child_font_px,
                    strut,
                    nodes,
                );
                return Ok(Some(top + line.cross_y + ascent));
            }
            if let Some(baseline) = block_last_baseline(
                flow[first],
                sizes[first],
                content.width,
                viewport,
                child_font_px,
                nodes,
                cache,
                scope,
            )? {
                let margin = margin_of(flow[first], nodes);
                return Ok(Some(top + line.cross_y + margin.top + baseline));
            }
        }
        return Ok(None);
    }
    // A block flow stacks its children from the top of its content box.
    let mut tops = Vec::with_capacity(flow.len());
    let mut cursor = 0.0f32;
    for (index, &child) in flow.iter().enumerate() {
        if index > 0 {
            cursor += gap;
        }
        let margin = margin_of(child, nodes);
        tops.push(cursor + margin.top);
        cursor += margin.top + sizes[index].height + margin.bottom;
    }
    for index in (0..flow.len()).rev() {
        let child = flow[index];
        let Some(child_style) = nodes.style(child) else {
            continue;
        };
        // Stretched across the block as placement stretches it, so what it
        // holds lays out at the width it gets.
        let mut size = sizes[index];
        if child_style.resolved_align_self(style.align_items) == AlignSpec::Stretch
            && !cross_axis_is_definite(
                &child_style,
                direction,
                has_natural_ratio(nodes.world, child),
            )
        {
            let margin = margin_of(child, nodes);
            set_cross_extent(
                &mut size,
                direction,
                (content.width - margin.left - margin.right).max(0.0),
            );
        }
        if let Some(baseline) = block_last_baseline(
            child,
            size,
            content.width,
            viewport,
            child_font_px,
            nodes,
            cache,
            scope,
        )? {
            return Ok(Some(top + tops[index] + baseline));
        }
    }
    Ok(None)
}

/// The last baseline of block-level `child` of a box's flow, from its top,
/// or `None` when it holds no line box: replaced content, or a box with no
/// line box in it.
#[allow(clippy::too_many_arguments)]
fn block_last_baseline(
    child: StableNodeId,
    size: Size,
    inline_base: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Option<f32>, UiWorldError> {
    if nodes.world.line_boxes_are_descendants(child) {
        let derived = descendant_last_baseline(
            child,
            size,
            inline_base,
            viewport,
            parent_font_px,
            nodes,
            cache,
            scope,
        )?;
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
    Ok(metrics.last.or(metrics.first))
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
    font_px: f32,
    nodes: &LayoutInputMap<'_>,
) -> LineStrut {
    let font = font_px.max(0.0);
    let (ascent, descent) = nodes.world.block_strut(id).map_or_else(
        || {
            let ascent = font * nana_ui_core::TEXT_APPROX_ASCENT_EM;
            (ascent, font - ascent)
        },
        |strut| (font * strut.ascent, font * strut.descent),
    );
    let half_leading =
        (text_line_box_height_px(font, style.line_height) - (ascent + descent)) * 0.5;
    LineStrut {
        ascent: ascent + half_leading,
        descent: descent + half_leading,
    }
}

/// A horizontal line box of an ordinary block: its baseline below the
/// line's top, and its height. Its inline-level items sit on that baseline
/// with the block's strut, and the line spans the highest top to the lowest
/// bottom among them (CSS 2.1 §10.8); a block-level item holds a line alone,
/// counted from the line's top, with no strut.
pub(super) fn ifc_line_metrics(
    items: impl IntoIterator<Item = (StableNodeId, Size)>,
    content_width: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    strut: LineStrut,
    nodes: &LayoutInputMap<'_>,
) -> (f32, f32) {
    let (mut ascent, mut descent, mut from_top) = (0.0f32, 0.0f32, 0.0f32);
    for (child, size) in items {
        let Some(style) = nodes.style(child) else {
            continue;
        };
        match ifc_atom_metrics(child, &style, size, content_width, parent_font_px, nodes) {
            Some((above, below)) => {
                ascent = ascent.max(above).max(strut.ascent);
                descent = descent.max(below).max(strut.descent);
            }
            None => {
                let (_, outer) =
                    ifc_item_outer(&style, size, content_width, viewport, parent_font_px);
                from_top = from_top.max(outer);
            }
        }
    }
    (ascent, (ascent + descent).max(from_top))
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
    strut: LineStrut,
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
        let (_, line_cross) = ifc_line_metrics(
            current.iter().map(|&index| (children[index], sizes[index])),
            content_width,
            viewport,
            parent_font_px,
            strut,
            nodes,
        );
        lines.push(LineBoxSlot {
            indices: std::mem::take(current),
            main_start: *left_inset,
            main_available: *available,
            cross_y: *line_y,
            pin_cross: true,
            baseline_aligned: true,
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
                baseline_aligned: false,
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
    nodes: &LayoutInputMap<'_>,
    // The lines of an ordinary block, with its strut: inline-level items
    // share a baseline and a line is as tall as they span
    // ([`ifc_line_metrics`]).
    baseline_lines: Option<LineStrut>,
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
        if let Some(strut) = baseline_lines {
            line_cross = ifc_line_metrics(
                line.iter().map(|&index| (children[index], sizes[index])),
                edge_base,
                viewport,
                parent_font_px,
                strut,
                nodes,
            )
            .1;
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
        Some(line_strut(id, style, child_font_px, nodes)),
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
            updated.entries[slot].baseline = ifc_atom_metrics(
                child,
                &child_style,
                measured,
                edge_base,
                child_font_px,
                nodes,
            )
            .map(|(ascent, _)| ascent);
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
    style.display == Some(DisplaySpec::InlineBlock)
        && !world.atom_baseline_reads_content(id)
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
