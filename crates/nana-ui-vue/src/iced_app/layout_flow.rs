// Layout flow: column/row/wrap, fixed/overlay layers, text, chrome heuristics.

/// Containing box for children, including CSS grid stretch: `height/width:auto`
/// items that iced maps to Fill must still expose a definite CB to descendants
/// (`height:100%` / nested Fill). [`LayoutStyle::resolve_content_box`] only
/// sees the stylesheet height, which stays `None` under stretch.
fn resolve_flow_content_box(
    layout: &crate::css_map::LayoutStyle,
    parent_box: ParentBox,
) -> ParentBox {
    let resolved = layout.resolve_content_box(parent_box);
    let stretch = crate::css_map::grid_item_stretch_main_vertical();
    let pad = layout.resolved_padding_against(parent_box.width);
    let bw = layout.resolved_border_width();
    let height = resolved.height.or_else(|| {
        if matches!(stretch, Some(true)) {
            parent_box
                .height
                .map(|h| (h - pad.top - pad.bottom - 2.0 * bw).max(0.0))
        } else {
            None
        }
    });
    let width = resolved.width.or_else(|| {
        if matches!(stretch, Some(false)) {
            parent_box
                .width
                .map(|w| (w - pad.left - pad.right - 2.0 * bw).max(0.0))
        } else if !matches!(layout.width, Some(LengthSpec::Shrink))
            && (layout.active_grid_columns().is_some()
                || layout.active_grid_rows().is_some()
                || !matches!(layout.direction, Some(FlexDirection::Row)))
        {
            // Block / column / grid `width:auto` fills the containing block so
            // nested grids, Fill, and `%` see a definite CB. Row flex items keep
            // intrinsic width unless explicitly Stretch/Fill via flex sizing.
            parent_box
                .width
                .map(|w| (w - pad.left - pad.right - 2.0 * bw).max(0.0))
        } else {
            None
        }
    });
    ParentBox::new(width, height)
}

/// CSS height:100%/Fill (or grid stretch) on a Card must reach iced — `Card`
/// defaults to Shrink and would otherwise leave the grid area empty below.
fn card_with_css_height<'a, Message: 'a>(
    mut card: Card<'a, Message>,
    layout: &crate::css_map::LayoutStyle,
    parent_box: ParentBox,
) -> Card<'a, Message> {
    let stretch_fill = layout.height.is_none()
        && matches!(
            crate::css_map::grid_item_stretch_main_vertical(),
            Some(true)
        )
        && parent_box.height.filter(|h| *h > 0.0).is_some();
    if matches!(
        layout.height,
        Some(LengthSpec::Fill)
            | Some(LengthSpec::Percent(_))
            | Some(LengthSpec::CalcPercentOffset { .. })
    ) || stretch_fill
    {
        let h = length_from_spec(
            layout.height.or(Some(LengthSpec::Fill)),
            parent_box.height,
            layout,
            true,
        );
        card = card.height(h);
    }
    card
}

/// Prefer authored CSS padding on `.card` over NanaUI default panel metrics.
fn card_with_css_padding<'a, Message: 'a>(
    mut card: Card<'a, Message>,
    layout: &crate::css_map::LayoutStyle,
    parent_box: ParentBox,
) -> Card<'a, Message> {
    let pad = layout.resolved_padding_against(parent_box.width);
    if !pad.is_zero() {
        card = card.padding(Padding {
            top: pad.top,
            right: pad.right,
            bottom: pad.bottom,
            left: pad.left,
        });
    }
    card
}

/// Content props for a Card body: strip chrome/height so `Card::view` owns them.
fn card_body_props(props: &crate::bridge::WidgetProps) -> crate::bridge::WidgetProps {
    let mut p = props.clone();
    p.label.clear();
    p.layout.height = None;
    p.layout.min_height = None;
    p.layout.max_height = None;
    p.layout.width = Some(LengthSpec::Fill);
    p.layout.padding = None;
    p.layout.padding_top = None;
    p.layout.padding_right = None;
    p.layout.padding_bottom = None;
    p.layout.padding_left = None;
    p.layout.background = None;
    p.layout.border_width = None;
    p.layout.border_color = None;
    p.layout.border_radius = None;
    p.layout.overflow_x = OverflowSpec::Visible;
    p.layout.overflow_y = OverflowSpec::Visible;
    if p.layout.align_items == AlignSpec::Start {
        p.layout.align_items = AlignSpec::Stretch;
    }
    p
}

fn layout_column<'a, Message>(
    snap: &'a SemanticSnapshot,
    widget: &'a SemanticWidget,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&'a EditorStore>,
    menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let layout = &widget.props.layout;
    let child_box = resolve_flow_content_box(layout, parent_box);
    let flow_box = flow_child_containing_box(layout, child_box);
    let main_gap = layout.main_gap_against(FlexDirection::Column, child_box);
    let cross_gap = layout.cross_gap_against(FlexDirection::Column, child_box);
    let justify = effective_justify(layout);
    let item_spacing = flex_item_spacing(main_gap, justify);
    let width = inner_flex_axis_length(
        layout,
        parent_box,
        length_from_spec(layout.width, parent_box.width, layout, false),
        true,
    );
    let align = align_from_spec(layout.align_items);
    let height = resolve_container_height(layout, parent_box);

    // Borrowed path must match wrap_layout_owned / measure (T-W07).
    let wrap_height = wrap_content_height(layout, child_box, parent_box);
    if matches!(layout.flex_wrap, FlexWrap::Wrap | FlexWrap::WrapReverse) && wrap_height.is_some()
    {
        let wrap_width = wrap_content_width(layout, child_box, parent_box);
        let lines =
            chunk_column_wrap_lines(layout, &widget.children, snap, wrap_height, wrap_width);
        // Cross-axis gap between wrap columns = column-gap.
        let mut r = row![].spacing(cross_gap).width(width).align_y(align);
        if let Some(h) = height {
            if !(layout.scrolls_y() && matches!(h, Length::Fill)) {
                r = r.height(h);
            }
        }
        let reverse = matches!(layout.flex_wrap, FlexWrap::WrapReverse);
        let line_iter: Box<dyn Iterator<Item = Vec<WidgetId>> + 'a> = if reverse {
            Box::new(lines.into_iter().rev())
        } else {
            Box::new(lines.into_iter())
        };
        for line in line_iter {
            let line_els = collect_child_elements(
                snap,
                &line,
                tokens,
                flow_box,
                child_box,
                FlexDirection::Column,
                layout,
                editors,
                menus,
                map_event.clone(),
            );
            let mut c = column![]
                .spacing(item_spacing)
                .height(Length::Fill)
                .align_x(align);
            c = push_justified(c, line_els, justify, main_gap);
            r = r.push(c);
        }
        return finalize_layout_container(
            r.into(),
            layout,
            parent_box,
            Some(widget.id),
            map_event,
        );
    }

    let mut col = column![].spacing(item_spacing).width(width).align_x(align);
    // Scrollport owns Fill height; pinning the same Fill on the inner column
    // clips shrink-wrapped children (Lilia home overview cards). Height is
    // applied AFTER push via pin_flex_container_main_length (enclose trap).
    if !widget.props.label.is_empty() && widget.children.is_empty() {
        // Same contract as WidgetKind::Text: CSS font-size wins, else ControlSize.
        col = col.push(label_text(
            widget.props.label.clone(),
            widget.props.size,
            layout,
            parent_box.width,
        ));
    }
    let flow_children = flex_flow_child_ids(snap, &widget.children, layout.flex_reverse);
    let children = collect_child_elements(
        snap,
        &flow_children,
        tokens,
        flow_box,
        child_box,
        FlexDirection::Column,
        layout,
        editors,
        menus,
        map_event.clone(),
    );
    col = push_justified(col, children, justify, main_gap);
    col = pin_flex_container_main_length(col, height, layout, parent_box, true);
    finalize_layout_container(
        col.into(),
        layout,
        parent_box,
        Some(widget.id),
        map_event,
    )
}

fn layout_row<'a, Message>(
    snap: &'a SemanticSnapshot,
    widget: &'a SemanticWidget,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    editors: Option<&'a EditorStore>,
    menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let layout = &widget.props.layout;
    let child_box = resolve_flow_content_box(layout, parent_box);
    let flow_box = flow_child_containing_box(layout, child_box);
    let main_gap = layout.main_gap_against(FlexDirection::Row, child_box);
    let cross_gap = layout.cross_gap_against(FlexDirection::Row, child_box);
    let justify = effective_justify(layout);
    let item_spacing = flex_item_spacing(main_gap, justify);
    let width = inner_flex_axis_length(
        layout,
        parent_box,
        length_from_spec(layout.width, parent_box.width, layout, false),
        true,
    );
    let align = align_from_spec(layout.align_items);

    // Borrowed path must match wrap_layout_owned / measure (T-W01 / T-W02 / T-W03).
    let wrap_width = wrap_content_width(layout, child_box, parent_box);
    if matches!(layout.flex_wrap, FlexWrap::Wrap | FlexWrap::WrapReverse) && wrap_width.is_some() {
        let lines = chunk_row_wrap_lines(layout, &widget.children, snap, wrap_width);
        // Cross-axis gap between wrap lines = row-gap (not distributed justify).
        let mut col = column![].spacing(cross_gap).width(width).align_x(align);
        if let Some(h) = resolve_container_height(layout, parent_box) {
            col = col.height(h);
        }
        let reverse = matches!(layout.flex_wrap, FlexWrap::WrapReverse);
        let line_iter: Box<dyn Iterator<Item = Vec<WidgetId>> + 'a> = if reverse {
            Box::new(lines.into_iter().rev())
        } else {
            Box::new(lines.into_iter())
        };
        for line in line_iter {
            let line_els = collect_child_elements(
                snap,
                &line,
                tokens,
                flow_box,
                child_box,
                FlexDirection::Row,
                layout,
                editors,
                menus,
                map_event.clone(),
            );
            let mut r = row![]
                .spacing(item_spacing)
                .width(Length::Fill)
                .align_y(align);
            r = push_justified_row(r, line_els, justify, main_gap);
            col = col.push(r);
        }
        return finalize_layout_container(
            col.into(),
            layout,
            parent_box,
            Some(widget.id),
            map_event,
        );
    }

    let mut r = row![].spacing(item_spacing).width(width).align_y(align);
    if !widget.props.label.is_empty() && widget.children.is_empty() {
        // Same contract as layout_column / WidgetKind::Text.
        r = r.push(label_text(
            widget.props.label.clone(),
            widget.props.size,
            layout,
            parent_box.width,
        ));
    }
    let flow_children = flex_flow_child_ids(snap, &widget.children, layout.flex_reverse);
    let children = collect_child_elements(
        snap,
        &flow_children,
        tokens,
        flow_box,
        child_box,
        FlexDirection::Row,
        layout,
        editors,
        menus,
        map_event.clone(),
    );
    r = push_justified_row(r, children, justify, main_gap);
    let row_height = resolve_container_height(layout, parent_box);
    r = pin_flex_row_cross_or_main_height(r, row_height, layout);
    finalize_layout_container(
        r.into(),
        layout,
        parent_box,
        Some(widget.id),
        map_event,
    )
}

fn wrap_layout_owned<Message>(
    column_axis: bool,
    props: &crate::bridge::WidgetProps,
    children: Vec<WidgetId>,
    snap: &SemanticSnapshot,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    parent_direction: FlexDirection,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    scroll_id: Option<WidgetId>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let layout = &props.layout;
    let direction = if column_axis {
        FlexDirection::Column
    } else {
        FlexDirection::Row
    };
    let child_box = resolve_flow_content_box(layout, parent_box);
    let flow_box = flow_child_containing_box(layout, child_box);
    let main_gap = layout.main_gap_against(direction, child_box);
    let cross_gap = layout.cross_gap_against(direction, child_box);
    let justify = effective_justify(layout);
    let item_spacing = flex_item_spacing(main_gap, justify);
    // Flex row items with width:auto must stay intrinsic. `length_from_spec(None)`
    // maps to Fill, which collapses under SpaceBetween Fill spacers.
    let computed_width = if matches!(parent_direction, FlexDirection::Row)
        && layout.width.is_none()
        && !layout.grows()
        && !matches!(
            layout.flex_basis,
            Some(LengthSpec::Px(_)) | Some(LengthSpec::Percent(_))
        ) {
        Length::Fit
    } else {
        length_from_spec(layout.width, parent_box.width, layout, false)
    };
    let width = inner_flex_axis_length(layout, parent_box, computed_width, true);
    let align = align_from_spec(layout.align_items);
    let visible = flex_flow_child_ids(snap, &children, layout.flex_reverse);
    // Resolve this container under grid-item stretch, but do not leak the TLS
    // into descendants (nested height:auto must stay intrinsic).
    let child_els: Vec<Element<'static, Message>> =
        crate::css_map::with_grid_item_stretch_cleared(|| {
        let (_, margins_x) = grid_children_axis_margins(snap, &visible, flow_box.width, true);
        let (_, margins_y) = grid_children_axis_margins(snap, &visible, flow_box.width, false);
        let track_outers = precompute_grid_track_outers(
            snap,
            &visible,
            layout,
            direction,
            if column_axis {
                flow_box.height.or(child_box.height).or(parent_box.height)
            } else {
                flow_box.width.or(child_box.width).or(parent_box.width)
            },
            if column_axis {
                flow_box.width.or(child_box.width).or(parent_box.width)
            } else {
                flow_box.height.or(child_box.height).or(parent_box.height)
            },
            if column_axis { &margins_y } else { &margins_x },
        );
        let fill_portion_tracks = prefer_fill_portion_grid_tracks(layout, direction);
        visible
            .iter()
            .copied()
            .enumerate()
            .map(|(idx, child)| {
                let child_parent = snap
                    .get(child)
                    .map(|w| {
                        let mut pb =
                            parent_box_for_flow_child(layout, flow_box, child_box, &w.props.layout);
                        pb = indefinite_axis_for_auto_track(layout, direction, idx, pb);
                        // Column grid/flex: cross-axis width comes from the
                        // parent CB even when this container's width is auto.
                        if pb.width.is_none() {
                            pb = ParentBox::new(
                                flow_box
                                    .width
                                    .or(child_box.width)
                                    .or(parent_box.width),
                                pb.height,
                            );
                        }
                        if pb.height.is_none() && matches!(direction, FlexDirection::Row) {
                            pb = ParentBox::new(
                                pb.width,
                                flow_box
                                    .height
                                    .or(child_box.height)
                                    .or(parent_box.height),
                            );
                        }
                        // Precomputed track size is a definite grid area CB.
                        if let Some(px) = track_outers.as_ref().and_then(|o| o.get(idx)).copied() {
                            if column_axis {
                                pb = ParentBox::new(pb.width, Some(px));
                            } else {
                                pb = ParentBox::new(Some(px), pb.height);
                            }
                        }
                        pb
                    })
                    .unwrap_or(flow_box);
                let main_sizes =
                    flex_main_overrides(snap, &visible, layout, child_parent, direction);
                let build_child = || {
                    view_widget_owned(
                        snap,
                        child,
                        tokens,
                        child_parent,
                        direction,
                        layout.align_items,
                        editors,
                        menus,
                        viewport,
                        main_sizes.as_ref().and_then(|s| s[idx]),
                        map_event.clone(),
                    )
                };
                let mut el = if track_outers.as_ref().and_then(|o| o.get(idx)).is_some() {
                    // Track size already resolved (incl. auto via measure): stretch
                    // into the area. Intrinsic demotion would shrink nested
                    // height:100% cards below the Fixed track.
                    crate::css_map::with_grid_item_stretch_main(
                        matches!(direction, FlexDirection::Column),
                        build_child,
                    )
                } else if track_is_auto(layout, direction, idx) {
                    crate::css_map::with_intrinsic_auto_track(
                        matches!(direction, FlexDirection::Column),
                        build_child,
                    )
                } else if track_is_sized(layout, direction, idx) {
                    // Non-auto tracks: stretch height/width:auto items into the
                    // grid area so nested Fill/% see a definite CB (shell 1fr,
                    // home 1fr). Outer FillPortion alone leaves Shrink columns.
                    crate::css_map::with_grid_item_stretch_main(
                        matches!(direction, FlexDirection::Column),
                        build_child,
                    )
                } else {
                    build_child()
                };
                if let Some(px) = track_outers.as_ref().and_then(|o| o.get(idx)).copied() {
                    if fill_portion_tracks {
                        // CB injected above; iced sizes via FillPortion weights.
                        el = apply_grid_track_width(
                            el,
                            layout,
                            idx,
                            direction,
                            child_parent
                                .width
                                .or(flow_box.width)
                                .or(child_box.width)
                                .or(parent_box.width),
                            &margins_x,
                        );
                        el = apply_grid_track_height(
                            el,
                            layout,
                            idx,
                            direction,
                            child_parent
                                .height
                                .or(flow_box.height)
                                .or(child_box.height)
                                .or(parent_box.height),
                            &margins_y,
                        );
                    } else if column_axis {
                        el = container(el).height(Length::Fixed(px)).into();
                    } else {
                        el = container(el).width(Length::Fixed(px)).into();
                    }
                } else {
                    el = apply_grid_track_width(
                        el,
                        layout,
                        idx,
                        direction,
                        // Grid items stretch to the container's CB even when the
                        // row/column widget itself has width:auto (CSS grid default).
                        child_parent
                            .width
                            .or(flow_box.width)
                            .or(child_box.width)
                            .or(parent_box.width),
                        &margins_x,
                    );
                    el = apply_grid_track_height(
                        el,
                        layout,
                        idx,
                        direction,
                        child_parent
                            .height
                            .or(flow_box.height)
                            .or(child_box.height)
                            .or(parent_box.height),
                        &margins_y,
                    );
                }
                el
            })
            .collect()
        });

    if column_axis {
        let height = resolve_container_height(layout, parent_box);
        // Empty iced `column![]` ignores Fixed height; surface slots (GPU preview,
        // heatmap cells) need a sized `space` to materialize background paint.
        if child_els.is_empty()
            && props.label.is_empty()
            && layout.has_surface_paint()
            && matches!(height, Some(Length::Fixed(_)))
        {
            let h = height.unwrap();
            // Paint on the sized spacer itself — nested empty `column` + chrome
            // was still collapsing in iced's layout pass.
            let paint = surface_paint_from(layout);
            let spacer = container(space().width(Length::Fill).height(h))
                .width(width)
                .height(h)
                .style(move |_theme| surface_style(paint));
            return spacer.into();
        }

        // flex-wrap on column → row of columns (T-W07).
        let wrap_height = wrap_content_height(layout, child_box, parent_box);
        if matches!(layout.flex_wrap, FlexWrap::Wrap | FlexWrap::WrapReverse)
            && wrap_height.is_some()
        {
            let wrap_width = wrap_content_width(layout, child_box, parent_box);
            let lines =
                chunk_column_wrap_lines(layout, &children, snap, wrap_height, wrap_width);
            let mut r = row![].spacing(cross_gap).width(width).align_y(align);
            if let Some(h) = height {
                if !layout.scrolls_y() {
                    r = r.height(h);
                }
            }
            let line_iter: Box<dyn Iterator<Item = Vec<WidgetId>>> =
                if matches!(layout.flex_wrap, FlexWrap::WrapReverse) {
                    Box::new(lines.into_iter().rev())
                } else {
                    Box::new(lines.into_iter())
                };
            for line in line_iter {
                let line_sizes =
                    flex_main_overrides(snap, &line, layout, flow_box, FlexDirection::Column);
                let line_els: Vec<Element<'static, Message>> = line
                    .into_iter()
                    .enumerate()
                    .map(|(idx, child)| {
                        view_widget_owned(
                            snap,
                            child,
                            tokens,
                            flow_box,
                            FlexDirection::Column,
                        layout.align_items,
                        editors,
                        menus,
                        viewport,
                        line_sizes.as_ref().and_then(|s| s[idx]),
                        map_event.clone(),
                    )
                    })
                    .collect();
                let mut c = column![]
                    .spacing(item_spacing)
                    .height(Length::Fill)
                    .align_x(align);
                c = push_justified(c, line_els, justify, main_gap);
                r = r.push(c);
            }
            return finalize_layout_container(
                r.into(),
                layout,
                parent_box,
                scroll_id,
                map_event,
            );
        }

        let mut col = column![].spacing(item_spacing).width(width).align_x(align);
        if !props.label.is_empty() && child_els.is_empty() {
            // Same contract as WidgetKind::Text: CSS font-size wins, else ControlSize.
            col = col.push(label_text(
                props.label.clone(),
                props.size,
                layout,
                parent_box.width,
            ));
        }
        col = push_justified(col, child_els, justify, main_gap);
        // Pin height AFTER push (enclose trap). Under a definite parent CB,
        // height:auto → Fill (no iced Shrink compression). Children with
        // height:auto are pinned Shrink so they do not steal the main axis.
        col = pin_flex_container_main_length(col, height, layout, parent_box, true);
        return finalize_layout_container(
            col.into(),
            layout,
            parent_box,
            scroll_id,
            map_event,
        );
    }

    // flex-wrap: Wrap → column of rows. Align with measure.rs: when the row
    // itself is width:auto/None but the parent has a definite width, still wrap
    // against that available content width (T-W01 / parent-constrained wrap).
    let wrap_width = wrap_content_width(layout, child_box, parent_box);
    if matches!(layout.flex_wrap, FlexWrap::Wrap | FlexWrap::WrapReverse) && wrap_width.is_some() {
        let lines = chunk_row_wrap_lines(layout, &children, snap, wrap_width);
        // Cross-axis gap between wrap lines = row-gap (not distributed justify).
        let mut col = column![].spacing(cross_gap).width(width).align_x(align);
        if let Some(h) = resolve_container_height(layout, parent_box) {
            col = col.height(h);
        }
        let line_iter: Box<dyn Iterator<Item = Vec<WidgetId>>> =
            if matches!(layout.flex_wrap, FlexWrap::WrapReverse) {
                Box::new(lines.into_iter().rev())
            } else {
                Box::new(lines.into_iter())
            };
        for line in line_iter {
            let line_sizes =
                flex_main_overrides(snap, &line, layout, flow_box, FlexDirection::Row);
            let line_els: Vec<Element<'static, Message>> = line
                .into_iter()
                .enumerate()
                .map(|(idx, child)| {
                    view_widget_owned(
                        snap,
                        child,
                        tokens,
                        flow_box,
                        FlexDirection::Row,
                        layout.align_items,
                        editors,
                        menus,
                        viewport,
                        line_sizes.as_ref().and_then(|s| s[idx]),
                        map_event.clone(),
                    )
                })
                .collect();
            let mut r = row![]
                .spacing(item_spacing)
                .width(Length::Fill)
                .align_y(align);
            r = push_justified_row(r, line_els, justify, main_gap);
            col = col.push(r);
        }
        return finalize_layout_container(
            col.into(),
            layout,
            parent_box,
            scroll_id,
            map_event,
        );
    }

    let mut r = row![].spacing(item_spacing).width(width).align_y(align);
    if !props.label.is_empty() && child_els.is_empty() {
        // Same contract as column-axis / WidgetKind::Text.
        r = r.push(label_text(
            props.label.clone(),
            props.size,
            layout,
            parent_box.width,
        ));
    }
    r = push_justified_row(r, child_els, justify, main_gap);
    // Rows: height:auto stays Shrink (cross axis). Do not use Fill here —
    // that would stretch auto-height headings inside Fill columns.
    let row_height = resolve_container_height(layout, parent_box);
    r = pin_flex_row_cross_or_main_height(r, row_height, layout);
    finalize_layout_container(r.into(), layout, parent_box, scroll_id, map_event)
}

/// Available main-axis width for row flex-wrap, matching measure's use of parent
/// content width when the node itself is `width: auto` / unspecified.
fn wrap_content_width(
    layout: &crate::css_map::LayoutStyle,
    child_box: ParentBox,
    parent_box: ParentBox,
) -> Option<f32> {
    if let Some(w) = child_box.width.filter(|w| *w > 0.0) {
        return Some(w);
    }
    let parent_w = parent_box.width.filter(|w| *w > 0.0)?;
    // Auto / unspecified still wrap against the parent's definite content box.
    if matches!(
        layout.width,
        None | Some(LengthSpec::Auto) | Some(LengthSpec::Fill)
    ) || layout.grows()
    {
        let pad = layout.resolved_padding_against(Some(parent_w));
        Some((parent_w - pad.left - pad.right).max(0.0))
    } else {
        None
    }
}

/// Available main-axis height for column flex-wrap (definite height required).
fn wrap_content_height(
    layout: &crate::css_map::LayoutStyle,
    child_box: ParentBox,
    parent_box: ParentBox,
) -> Option<f32> {
    if let Some(h) = child_box.height.filter(|h| *h > 0.0) {
        return Some(h);
    }
    let parent_h = parent_box.height.filter(|h| *h > 0.0)?;
    if matches!(
        layout.height,
        None | Some(LengthSpec::Auto) | Some(LengthSpec::Fill)
    ) || layout.grows()
    {
        let pad = layout.resolved_padding_against(parent_box.width);
        Some((parent_h - pad.top - pad.bottom).max(0.0))
    } else {
        None
    }
}

/// CSS / measure default: missing gap is 0 (never silent 8px).
/// Prefer [`LayoutStyle::main_gap`] / [`LayoutStyle::cross_gap`] for axis-aware
/// two-value / `row-gap`·`column-gap` layouts.
#[cfg(test)]
fn layout_gap(layout: &crate::css_map::LayoutStyle) -> f32 {
    layout.gap_or(0.0)
}

/// Distributed justify inserts Fixed(gap) + Fill spacers; outer `.spacing(gap)`
/// would also apply between spacers and double-count against measure.
fn flex_item_spacing(gap: f32, justify: JustifySpec) -> f32 {
    match justify {
        JustifySpec::SpaceBetween | JustifySpec::SpaceAround | JustifySpec::SpaceEvenly => 0.0,
        JustifySpec::Start | JustifySpec::End | JustifySpec::Center => gap,
    }
}

/// Match measure: CSS `order` ascending (stable → source order), then `*-reverse`.
fn sort_flex_items_by_order(snap: &SemanticSnapshot, children: &mut [WidgetId]) {
    children.sort_by_key(|&id| {
        snap.get(id)
            .map(|w| w.props.layout.order)
            .unwrap_or(0)
    });
}

/// In-flow children ordered for flex layout (order → source → optional reverse).
fn flex_flow_child_ids(
    snap: &SemanticSnapshot,
    children: &[WidgetId],
    flex_reverse: bool,
) -> Vec<WidgetId> {
    let mut visible: Vec<WidgetId> = children
        .iter()
        .copied()
        .filter(|&id| is_in_flow_layout(snap, id))
        .collect();
    sort_flex_items_by_order(snap, &mut visible);
    if flex_reverse {
        visible.reverse();
    }
    visible
}

/// Match measure: `*-reverse` packs from the opposite main-start (Start↔End).
fn effective_justify(layout: &crate::css_map::LayoutStyle) -> JustifySpec {
    if !layout.flex_reverse {
        return layout.justify_content;
    }
    match layout.justify_content {
        JustifySpec::Start => JustifySpec::End,
        JustifySpec::End => JustifySpec::Start,
        other => other,
    }
}

/// Axis for `WidgetKind::Text` hosts that paint nested children (`h2` + `#text`).
///
/// CSS `display:flex` defaults to row; forcing a column would make
/// `align-items:center` center on the horizontal axis (card title blank).
fn text_host_column_axis(layout: &crate::css_map::LayoutStyle) -> bool {
    match layout.direction {
        Some(FlexDirection::Row) => false,
        Some(FlexDirection::Column) => true,
        None => !layout.display.is_some_and(DisplaySpec::is_flex_container),
    }
}

/// Resolve `position:fixed` border box against the content viewport (logical px).
fn resolve_fixed_box(layout: &crate::css_map::LayoutStyle, vw: f32, vh: f32) -> (f32, f32, f32, f32) {
    let left = crate::css_map::LayoutStyle::resolve_inset(layout.offset_left, vw);
    let right = crate::css_map::LayoutStyle::resolve_inset(layout.offset_right, vw);
    let top = crate::css_map::LayoutStyle::resolve_inset(layout.offset_top, vh);
    let bottom = crate::css_map::LayoutStyle::resolve_inset(layout.offset_bottom, vh);

    let mut width = layout
        .width
        .and_then(|w| w.resolve_px(Some(vw)))
        .unwrap_or(0.0)
        .max(0.0);
    if let (Some(l), Some(r)) = (left, right) {
        if layout.width.is_none()
            || matches!(
                layout.width,
                Some(LengthSpec::Auto) | Some(LengthSpec::Fill) | Some(LengthSpec::Shrink)
            )
        {
            width = (vw - l - r).max(0.0);
        }
    }
    let mw = layout.resolved_min_width(Some(vw), Some((vw, vh)));
    width = width.max(mw);
    if let Some(mw) = layout.resolved_max_width(Some(vw), Some((vw, vh))) {
        width = width.min(mw);
    }

    let mut height = layout
        .height
        .and_then(|h| h.resolve_px(Some(vh)))
        .unwrap_or(0.0)
        .max(0.0);
    if let (Some(t), Some(b)) = (top, bottom) {
        if layout.height.is_none()
            || matches!(
                layout.height,
                Some(LengthSpec::Auto) | Some(LengthSpec::Fill) | Some(LengthSpec::Shrink)
            )
        {
            height = (vh - t - b).max(0.0);
        }
    }
    let mh = layout.resolved_min_height(Some(vh), Some((vw, vh)));
    height = height.max(mh);
    if let Some(mh) = layout.resolved_max_height(Some(vh), Some((vw, vh))) {
        height = height.min(mh);
    }

    let x = if let Some(l) = left {
        l
    } else if let Some(r) = right {
        (vw - r - width).max(0.0)
    } else {
        0.0
    };
    let y = if let Some(t) = top {
        t
    } else if let Some(b) = bottom {
        (vh - b - height).max(0.0)
    } else {
        0.0
    };
    (x, y, width, height)
}

/// Collect open Nana Overlay widgets (preorder) for the root viewport stack.
/// Companion CSS `fixed`/`sticky` is already stripped on these kinds.
fn collect_open_overlay_ids(snap: &SemanticSnapshot) -> Vec<WidgetId> {
    let mut out = Vec::new();
    fn walk(snap: &SemanticSnapshot, id: WidgetId, out: &mut Vec<WidgetId>) {
        let Some(w) = snap.get(id) else {
            return;
        };
        if w.kind.is_overlay() && overlay_is_open(&w.props) {
            out.push(id);
        }
        for &c in &w.children {
            walk(snap, c, out);
        }
    }
    for &r in &snap.roots {
        walk(snap, r, &mut out);
    }
    out
}

/// Viewport-sized Nana Overlay layer (Dialog / Drawer / Popover / ContextMenu).
fn view_overlay_layer_owned<Message>(
    snap: &SemanticSnapshot,
    id: WidgetId,
    tokens: ThemeTokens,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let Some(widget) = snap.get(id) else {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    };
    let parent = ParentBox::new(Some(viewport.width), Some(viewport.height));
    match qualified_runtime_scene_view(snap, widget) {
        QualifiedSceneRoute::Scene(view) => return view,
        QualifiedSceneRoute::Pending => return pending_qualified_placeholder(widget),
        QualifiedSceneRoute::Compatibility => {}
    }
    match widget.kind {
        WidgetKind::Dialog => overlay_dialog_owned(
            snap,
            &widget.props,
            id,
            &widget.children,
            tokens,
            parent,
            editors,
            menus,
            viewport,
            map_event,
        ),
        WidgetKind::Drawer => overlay_drawer_owned(
            snap,
            &widget.props,
            id,
            &widget.children,
            tokens,
            parent,
            editors,
            menus,
            viewport,
            map_event,
        ),
        WidgetKind::Popover => overlay_popover_owned(
            snap,
            &widget.props,
            id,
            &widget.children,
            tokens,
            parent,
            editors,
            menus,
            viewport,
            map_event,
        ),
        WidgetKind::ContextMenu => {
            overlay_context_menu_owned(&widget.props, id, tokens, viewport, menus, map_event)
        }
        _ => space().width(Length::Shrink).height(Length::Shrink).into(),
    }
}

/// Collect non-overlay `position:fixed` widgets (preorder), then sort by z-index.
fn collect_css_fixed_ids(snap: &SemanticSnapshot) -> Vec<WidgetId> {
    let mut ids = Vec::new();
    fn walk(snap: &SemanticSnapshot, id: WidgetId, ids: &mut Vec<WidgetId>) {
        let Some(w) = snap.get(id) else {
            return;
        };
        if w.props.layout.hidden {
            return;
        }
        if !w.kind.is_overlay() && w.props.layout.is_fixed() {
            ids.push(id);
        }
        for &child in &w.children {
            walk(snap, child, ids);
        }
    }
    for &root in &snap.roots {
        walk(snap, root, &mut ids);
    }
    ids.sort_by_key(|&id| {
        snap.get(id)
            .map(|w| w.props.layout.z_index.unwrap_or(0))
            .unwrap_or(0)
    });
    ids
}

/// Viewport-pinned layer for one CSS fixed widget (above document flow).
fn view_fixed_layer_owned<Message>(
    snap: &SemanticSnapshot,
    id: WidgetId,
    tokens: ThemeTokens,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let Some(widget) = snap.get(id) else {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    };
    let (x, y, w, h) = resolve_fixed_box(&widget.props.layout, viewport.width, viewport.height);
    let parent = ParentBox::from_viewport(viewport.width, viewport.height);
    let content = view_widget_owned_forced(
        snap,
        id,
        tokens,
        parent,
        FlexDirection::Column,
        AlignSpec::Stretch,
        editors,
        menus,
        viewport,
        None,
        true,
        map_event,
    );
    let sized = container(content)
        .width(Length::Fixed(w.max(1.0)))
        .height(Length::Fixed(h.max(1.0)));
    container(
        column![
            space().height(Length::Fixed(y.max(0.0))),
            row![space().width(Length::Fixed(x.max(0.0))), sized],
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn is_layout_visible(snap: &SemanticSnapshot, id: WidgetId) -> bool {
    snap.get(id)
        .map(|w| !w.props.layout.hidden)
        .unwrap_or(false)
}

/// Text with optional CSS `text-overflow: ellipsis` / `white-space: nowrap`.
fn label_text<'a, Message: 'a>(
    content: String,
    size: ControlSize,
    layout: &crate::css_map::LayoutStyle,
    parent_width: Option<f32>,
) -> Element<'a, Message> {
    let px = label_text_size_px(size, layout);
    let weight = css_font_weight_to_iced(layout.font_weight);
    let font = resolve_iced_font(layout.font_family.as_deref(), weight);
    let color = layout
        .color
        .map(|c| Color::from_rgba(c[0], c[1], c[2], c[3]));

    let tracking = layout
        .letter_spacing
        .filter(|v| v.abs() > 0.01)
        .unwrap_or(0.0);

    // Non-zero letter-spacing: approximate via per-glyph row spacing (same
    // technique as nana-ui tracked titles). Ellipsis path stays on Text.
    if tracking.abs() > 0.01 && !layout.uses_text_ellipsis() {
        let mut content_row = row![].spacing(tracking).align_y(Alignment::Center);
        for ch in content.chars() {
            let mut glyph = text(ch.to_string()).size(px).font(font);
            if let Some(c) = color {
                glyph = glyph.color(c);
            }
            if let Some(lh) = layout.line_height {
                glyph = glyph.line_height(line_height_from_spec(lh));
            }
            content_row = content_row.push(glyph);
        }
        // Glyph rows otherwise measure ~0 tall; pin line-box + intrinsic width.
        let line_px = crate::css_map::text_line_box_height_px(px, layout.line_height);
        return container(content_row)
            .width(Length::Fit)
            .height(Length::Fixed(line_px.max(1.0)))
            .into();
    }

    let mut t = text(content).size(px).font(font);
    if let Some(c) = color {
        t = t.color(c);
    }
    if let Some(lh) = layout.line_height {
        t = t.line_height(line_height_from_spec(lh));
    }
    if layout.white_space_nowrap || layout.uses_text_ellipsis() {
        t = t.wrapping(text_widget::Wrapping::None);
    }
    if layout.uses_text_ellipsis() {
        // Fill+ellipsis needs a definite cross/main budget; under an indefinite
        // parent width iced resolves Fill to 0 and the label vanishes.
        if parent_width.filter(|w| *w > 0.0).is_some()
            || matches!(layout.width, Some(LengthSpec::Fill) | Some(LengthSpec::Percent(_)))
            || layout.grows()
        {
            t = t.width(Length::Fill).ellipsis(Ellipsis::End);
        } else {
            t = t.ellipsis(Ellipsis::End);
        }
    }
    t.into()
}

pub(crate) fn line_height_from_spec(spec: crate::css_map::LineHeightSpec) -> LineHeight {
    match spec {
        crate::css_map::LineHeightSpec::Relative(f) => LineHeight::Relative(f.max(0.0)),
        crate::css_map::LineHeightSpec::Absolute(px) => LineHeight::Absolute(px.max(0.0).into()),
    }
}

pub(crate) fn css_font_weight_to_iced(weight: Option<u16>) -> font::Weight {
    match weight.unwrap_or(400) {
        0..=199 => font::Weight::Thin,
        200..=299 => font::Weight::ExtraLight,
        300..=349 => font::Weight::Light,
        350..=449 => font::Weight::Normal,
        450..=549 => font::Weight::Medium,
        550..=649 => font::Weight::Semibold,
        650..=749 => font::Weight::Bold,
        750..=849 => font::Weight::ExtraBold,
        _ => font::Weight::Black,
    }
}

/// Map CSS `font-family` preference onto the bundled Nana UI face when possible.
/// Unknown names fall back to [`ui_font`] (Noto Sans SC) so CJK never silently
/// drops to an unloaded system default while `bundled-fonts` is active.
pub(crate) fn resolve_iced_font(family: Option<&str>, weight: font::Weight) -> iced::Font {
    match family.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("monospace") | Some("ui-monospace") => iced::Font {
            weight,
            ..iced::Font::MONOSPACE
        },
        // Named stacks still prefer the host-registered Nana face.
        Some("noto sans sc") | Some(_) | None => ui_font(weight),
    }
}

/// In-flow iced children: visible and not CSS absolute / fixed / deferred sticky.
///
/// Absolute is measure-only. Fixed leaves flow and paints via the root viewport
/// layer. Sticky stays deferred. **Open** Overlay kinds leave flow and paint on
/// the root Nana Overlay stack (viewport); closed overlays keep a Shrink slot.
fn is_in_flow_layout(snap: &SemanticSnapshot, id: WidgetId) -> bool {
    snap.get(id)
        .map(|w| {
            if w.props.layout.hidden {
                return false;
            }
            if w.kind.is_overlay() {
                return !overlay_is_open(&w.props);
            }
            !w.props.layout.is_out_of_flow()
                && !w.props.layout.position.is_unsupported_positioning()
        })
        .unwrap_or(false)
}

fn class_token(props: &WidgetProps, name: &str) -> bool {
    props.class_names.iter().any(|c| c == name)
}

/// Documented host GPU preview slot (`nana-gpu` / `nana-gpu-preview` / agent).
fn is_gpu_preview_slot(props: &WidgetProps) -> bool {
    props.class_names.iter().any(|c| {
        matches!(
            c.as_str(),
            "nana-gpu-preview" | "nana-gpu" | "nana-gpu-slot"
        )
    }) || props.attrs.contains_key("data-nana-gpu")
        || props.agent_id.to_ascii_lowercase().contains("gpu")
        || props.role.eq_ignore_ascii_case("nana-gpu")
}

fn is_raster_resource_slot(props: &WidgetProps) -> bool {
    props.attrs.contains_key("data-nana-canvas") || props.attrs.contains_key("data-nana-image")
}

fn raster_resource_view<Message: 'static>(props: &WidgetProps) -> Element<'static, Message> {
    let id = props
        .attrs
        .get("data-nana-canvas")
        .or_else(|| props.attrs.get("data-nana-image"))
        .and_then(|id| id.parse().ok());
    #[cfg(feature = "hosted")]
    {
        if let Some(id) = id
            && let Some(binding) = active_host_texture(&crate::canvas_gpu::slot(
                nana_ui_web_api::CanvasId(id),
            ))
        {
            let aspect_ratio = binding.aspect_ratio();
            return nana_ui::GpuTextureView::from_binding(binding)
                .with_corner_radius(props.layout.border_radius.unwrap_or(0.0))
                .contain(aspect_ratio);
        }
    }
    let Some(bitmap) = id.and_then(active_canvas_bitmap) else {
        return Space::new().width(Length::Fill).height(Length::Fill).into();
    };
    let handle = cached_canvas_handle(&bitmap);
    iced::widget::image::Image::new(handle)
        .width(Length::Fill)
        .height(Length::Fill)
        .content_fit(iced::ContentFit::Contain)
        .border_radius(props.layout.border_radius.unwrap_or(0.0))
        .into()
}

thread_local! {
    static CANVAS_HANDLE_CACHE: RefCell<std::collections::HashMap<u64, (u64, iced::widget::image::Handle)>> =
        RefCell::new(std::collections::HashMap::new());
}

fn cached_canvas_handle(bitmap: &CanvasBitmap) -> iced::widget::image::Handle {
    CANVAS_HANDLE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((version, handle)) = cache.get(&bitmap.id.0)
            && *version == bitmap.version
        {
            return handle.clone();
        }
        let handle =
            iced::widget::image::Handle::from_rgba(bitmap.width, bitmap.height, bitmap.rgba.clone());
        cache.insert(bitmap.id.0, (bitmap.version, handle.clone()));
        handle
    })
}

/// Row with `justify-content: space-between` and a definite cross-axis height.
fn is_space_between_fixed_row(props: &WidgetProps) -> bool {
    props.layout.justify_content == JustifySpec::SpaceBetween
        && matches!(props.layout.direction, Some(FlexDirection::Row) | None)
        && matches!(props.layout.height, Some(LengthSpec::Px(h)) if h > 1.0)
}

/// Chrome tray: row-ish node with border + padding + background (Style Model).
fn is_chrome_tray(props: &WidgetProps) -> bool {
    let l = &props.layout;
    let is_row = matches!(l.direction, Some(FlexDirection::Row));
    if !is_row {
        return false;
    }
    let has_border = l.border_width.unwrap_or(0.0) > 0.0;
    let has_pad = l.padding.is_some()
        || l.padding_top.is_some()
        || l.padding_right.is_some()
        || l.padding_bottom.is_some()
        || l.padding_left.is_some();
    let has_bg = l.background.is_some();
    has_border && has_pad && has_bg
}

/// Compact single-line row (Fixed short height + nowrap).
fn is_compact_nowrap_row(props: &WidgetProps) -> bool {
    props.layout.white_space_nowrap
        && matches!(props.layout.direction, Some(FlexDirection::Row) | None)
        && matches!(props.layout.height, Some(LengthSpec::Px(h)) if (1.0..33.0).contains(&h))
}

/// Fill + grow column that needs a definite parent box for nested shells.
/// Scrollports must not take this path — they own viewport sizing via
/// `wrap_layout_owned` + `scrollable`, and seeding a definite height CB here
/// re-introduces Fill→0 collapse for list rows inside the scroll content.
fn needs_definite_fill_column(props: &WidgetProps) -> bool {
    props.layout.grows()
        && matches!(props.layout.height, Some(LengthSpec::Fill))
        && !props.layout.scrolls_y()
        && !matches!(props.layout.direction, Some(FlexDirection::Row))
        && props.layout.active_grid_rows().is_none()
}

/// Unhosted `<nana-gpu>` surface: light theme fill + radius/border from tokens.
/// No invented tech label — only render text when the app supplies label/hint.
fn gpu_preview_placeholder<Message: 'static>(
    id: WidgetId,
    props: &WidgetProps,
    children: &[WidgetId],
    snap: &SemanticSnapshot,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    let scene_binding = active_scene_host_texture(id);
    let fallback_binding = props
        .attrs
        .get("data-nana-gpu")
        .and_then(|slot| active_host_texture(slot));
    if let Some(binding) = scene_binding.or(fallback_binding) {
        let aspect_ratio = binding.aspect_ratio();
        return nana_ui::GpuTextureView::from_binding(binding)
            // Compatibility painting still applies ancestor opacity/transform
            // through the surrounding Iced widgets. Use only this node's local
            // value here so the composed UiScene opacity is not multiplied twice.
            .with_opacity(props.layout.opacity.unwrap_or(1.0))
            .with_corner_radius(props.layout.border_radius.unwrap_or(0.0))
            .contain(aspect_ratio);
    }
    let slot_h = children
        .iter()
        .find_map(|cid| {
            snap.get(*cid).and_then(|c| match c.props.layout.height {
                Some(LengthSpec::Px(h)) if h > 1.0 => Some(h),
                _ => None,
            })
        })
        .or_else(|| match props.layout.height {
            Some(LengthSpec::Px(h)) if h > 1.0 => Some(h),
            _ => None,
        })
        .unwrap_or(100.0)
        .clamp(72.0, 100.0);
    let radius = props.layout.border_radius.unwrap_or(10.0);
    let surface = tokens.colors.surface;
    let border = tokens.colors.border;
    let muted = tokens.colors.muted;
    let label = if !props.label.is_empty() {
        Some(props.label.clone())
    } else if !props.hint.is_empty() {
        Some(props.hint.clone())
    } else {
        None
    };
    let body: Element<'static, Message> = match label {
        Some(text_label) => text(text_label)
            .size(13)
            .color(muted)
            .width(Length::Fill)
            .align_x(Alignment::Center)
            .into(),
        None => Space::new().width(Length::Fill).height(Length::Fill).into(),
    };
    container(body)
    .width(Length::Fill)
    .height(Length::Fixed(slot_h))
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .style(move |_t| iced::widget::container::Style {
        background: Some(Background::Color(surface)),
        border: Border {
            color: border,
            width: 1.0,
            radius: radius.into(),
        },
        ..Default::default()
    })
    .into()
}

fn find_child_with_class(
    snap: &SemanticSnapshot,
    children: &[WidgetId],
    class: &str,
) -> Option<WidgetId> {
    children.iter().copied().find(|&id| {
        snap.get(id)
            .is_some_and(|w| w.props.class_names.iter().any(|c| c == class))
    })
}

fn collect_plain_text(snap: &SemanticSnapshot, id: WidgetId) -> String {
    let Some(w) = snap.get(id) else {
        return String::new();
    };
    // SVG path boxes / Lucide glyphs are not human-readable captions.
    if matches!(w.kind, WidgetKind::Icon | WidgetKind::Box)
        && (looks_like_svg_path(w.props.display_label())
            || w.props
                .class_names
                .iter()
                .any(|c| c == "lucide" || c.starts_with("lucide-") || c.contains("heatmap__")))
    {
        return String::new();
    }
    let own = w.props.display_label().trim();
    if looks_like_svg_path(own) {
        let mut parts = Vec::new();
        for &child in &w.children {
            let t = collect_plain_text(snap, child);
            if !t.is_empty() {
                parts.push(t);
            }
        }
        return parts.join(" ");
    }
    if !own.is_empty() && w.children.is_empty() {
        return own.to_string();
    }
    let mut parts = Vec::new();
    if !own.is_empty()
        && !w
            .props
            .class_names
            .iter()
            .any(|c| c.contains("hint") || c.contains("__hint"))
    {
        parts.push(own.to_string());
    }
    for &child in &w.children {
        let t = collect_plain_text(snap, child);
        if !t.is_empty() {
            parts.push(t);
        }
    }
    parts
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
