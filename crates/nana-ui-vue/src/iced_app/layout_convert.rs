// LayoutStyle → iced Length / flex / grid / box-model conversion helpers.

/// Definite scrollport extent: prefer parent content box, else viewport.
/// Any positive size is usable — the old `> 1.0` gate fell through to
/// `Length::Fill` and collapsed under Shrink ancestors.
fn definite_scroll_extent(preferred: Option<f32>, viewport_extent: f32) -> Length {
    preferred
        .filter(|v| *v > 0.0)
        .or_else(|| (viewport_extent > 0.0).then_some(viewport_extent))
        .map(Length::Fixed)
        .unwrap_or(Length::Fill)
}

/// Root tree Fixed size from viewport parent box. Same `> 0.0` gate as
/// [`definite_scroll_extent`] — the old `> 1.0` gate collapsed 1px to Fill.
fn root_viewport_axis(extent: Option<f32>) -> Length {
    extent
        .filter(|v| *v > 0.0)
        .map(Length::Fixed)
        .unwrap_or(Length::Fill)
}

/// Split row children into wrap lines using definite main sizes when available.
/// Skips hidden / `display:none` / `position:absolute` like [`crate::measure`].
fn chunk_row_wrap_lines(
    layout: &crate::css_map::LayoutStyle,
    children: &[WidgetId],
    snap: &SemanticSnapshot,
    content_w: Option<f32>,
) -> Vec<Vec<WidgetId>> {
    let visible = flex_flow_child_ids(snap, children, layout.flex_reverse);
    let Some(content_w) = content_w.filter(|w| *w > 0.0) else {
        return vec![visible];
    };
    // Row wrap main-axis gap = column-gap (matches measure / CSS flex).
    let main_gap = layout.main_gap_against(
        FlexDirection::Row,
        ParentBox::new(Some(content_w), None),
    );
    let mut lines: Vec<Vec<WidgetId>> = Vec::new();
    let mut current: Vec<WidgetId> = Vec::new();
    let mut line_main = 0.0f32;
    for id in visible {
        let child_layout = snap
            .get(id)
            .map(|w| &w.props.layout)
            .cloned()
            .unwrap_or_default();
        let m = child_layout.resolved_margin_against(Some(content_w));
        let main = child_layout.child_main_length(FlexDirection::Row);
        let w = match main {
            Some(LengthSpec::Fill) | None => content_w,
            Some(spec) => spec
                .resolve_with(Some(content_w), crate::css_map::active_viewport())
                .unwrap_or(content_w),
        };
        // Match measure::layout_row_wrap / layout_row_line outer main size.
        let outer = w + m.left + m.right;
        let need = if current.is_empty() {
            outer
        } else {
            line_main + main_gap + outer
        };
        if !current.is_empty() && need > content_w + 0.5 {
            lines.push(std::mem::take(&mut current));
            line_main = 0.0;
        }
        if current.is_empty() {
            line_main = outer;
        } else {
            line_main += main_gap + outer;
        }
        current.push(id);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

/// Split column children into wrap columns using definite main (height) sizes.
/// `content_w` is the flex container content width (margin `%` base; matches measure).
fn chunk_column_wrap_lines(
    layout: &crate::css_map::LayoutStyle,
    children: &[WidgetId],
    snap: &SemanticSnapshot,
    content_h: Option<f32>,
    content_w: Option<f32>,
) -> Vec<Vec<WidgetId>> {
    let visible = flex_flow_child_ids(snap, children, layout.flex_reverse);
    let Some(content_h) = content_h.filter(|h| *h > 0.0) else {
        return vec![visible];
    };
    // Column wrap main-axis gap = row-gap (matches measure / CSS flex).
    let main_gap = layout.main_gap_against(
        FlexDirection::Column,
        ParentBox::new(content_w, Some(content_h)),
    );
    let mut lines: Vec<Vec<WidgetId>> = Vec::new();
    let mut current: Vec<WidgetId> = Vec::new();
    let mut line_main = 0.0f32;
    for id in visible {
        let child_layout = snap
            .get(id)
            .map(|w| &w.props.layout)
            .cloned()
            .unwrap_or_default();
        // Vertical margin % resolves against containing-block width (CSS).
        let m = child_layout.resolved_margin_against(content_w);
        let main = child_layout.child_main_length(FlexDirection::Column);
        let h = match main {
            Some(LengthSpec::Fill) | None => content_h,
            Some(spec) => spec
                .resolve_with(Some(content_h), crate::css_map::active_viewport())
                .unwrap_or(content_h),
        };
        let outer = h + m.top + m.bottom;
        let need = if current.is_empty() {
            outer
        } else {
            line_main + main_gap + outer
        };
        if !current.is_empty() && need > content_h + 0.5 {
            lines.push(std::mem::take(&mut current));
            line_main = 0.0;
        }
        if current.is_empty() {
            line_main = outer;
        } else {
            line_main += main_gap + outer;
        }
        current.push(id);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn collect_child_elements<'a, Message>(
    snap: &'a SemanticSnapshot,
    children: &[WidgetId],
    tokens: ThemeTokens,
    flow_box: ParentBox,
    scrollport_box: ParentBox,
    direction: FlexDirection,
    parent_layout: &crate::css_map::LayoutStyle,
    editors: Option<&'a EditorStore>,
    menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Vec<Element<'a, Message>>
where
    Message: Clone + 'a,
{
    let visible: Vec<WidgetId> = children
        .iter()
        .copied()
        .filter(|&child| is_in_flow_layout(snap, child))
        .collect();
    let (_, margins_x) = grid_children_axis_margins(snap, &visible, flow_box.width, true);
    let (_, margins_y) = grid_children_axis_margins(snap, &visible, flow_box.width, false);
    let column_axis = matches!(direction, FlexDirection::Column);
    let track_outers = precompute_grid_track_outers(
        snap,
        &visible,
        parent_layout,
        direction,
        if column_axis {
            flow_box.height.or(scrollport_box.height)
        } else {
            flow_box.width.or(scrollport_box.width)
        },
        if column_axis {
            flow_box.width.or(scrollport_box.width)
        } else {
            flow_box.height.or(scrollport_box.height)
        },
        if column_axis { &margins_y } else { &margins_x },
    );
    let fill_portion_tracks = prefer_fill_portion_grid_tracks(parent_layout, direction);
    crate::css_map::with_grid_item_stretch_cleared(|| {
    visible
        .iter()
        .copied()
        .enumerate()
        .map(|(idx, child)| {
            let child_parent = snap
                .get(child)
                .map(|w| {
                    let mut pb = parent_box_for_flow_child(
                        parent_layout,
                        flow_box,
                        scrollport_box,
                        &w.props.layout,
                    );
                    pb = indefinite_axis_for_auto_track(parent_layout, direction, idx, pb);
                    if pb.width.is_none() {
                        pb = ParentBox::new(
                            flow_box.width.or(scrollport_box.width),
                            pb.height,
                        );
                    }
                    if pb.height.is_none() && matches!(direction, FlexDirection::Row) {
                        pb = ParentBox::new(
                            pb.width,
                            flow_box.height.or(scrollport_box.height),
                        );
                    }
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
                flex_main_overrides(snap, &visible, parent_layout, child_parent, direction);
            let build_child = || {
                view_widget(
                    snap,
                    child,
                    tokens,
                    child_parent,
                    direction,
                    parent_layout.align_items,
                    editors,
                    menus,
                    main_sizes.as_ref().and_then(|s| s[idx]),
                    map_event.clone(),
                )
            };
            let mut el = if track_outers.as_ref().and_then(|o| o.get(idx)).is_some() {
                crate::css_map::with_grid_item_stretch_main(
                    matches!(direction, FlexDirection::Column),
                    build_child,
                )
            } else if track_is_auto(parent_layout, direction, idx) {
                crate::css_map::with_intrinsic_auto_track(
                    matches!(direction, FlexDirection::Column),
                    build_child,
                )
            } else if track_is_sized(parent_layout, direction, idx) {
                crate::css_map::with_grid_item_stretch_main(
                    matches!(direction, FlexDirection::Column),
                    build_child,
                )
            } else {
                build_child()
            };
            if let Some(px) = track_outers.as_ref().and_then(|o| o.get(idx)).copied() {
                if fill_portion_tracks {
                    el = apply_grid_track_width(
                        el,
                        parent_layout,
                        idx,
                        direction,
                        child_parent
                            .width
                            .or(flow_box.width)
                            .or(scrollport_box.width),
                        &margins_x,
                    );
                    el = apply_grid_track_height(
                        el,
                        parent_layout,
                        idx,
                        direction,
                        child_parent
                            .height
                            .or(flow_box.height)
                            .or(scrollport_box.height),
                        &margins_y,
                    );
                } else if column_axis {
                    el = container(el).height(Length::Fixed(px)).into()
                } else {
                    el = container(el).width(Length::Fixed(px)).into()
                }
            } else {
                el = apply_grid_track_width(
                    el,
                    parent_layout,
                    idx,
                    direction,
                    child_parent
                        .width
                        .or(flow_box.width)
                        .or(scrollport_box.width),
                    &margins_x,
                );
                el = apply_grid_track_height(
                    el,
                    parent_layout,
                    idx,
                    direction,
                    child_parent
                        .height
                        .or(flow_box.height)
                        .or(scrollport_box.height),
                    &margins_y,
                );
            }
            el
        })
        .collect()
    })
}

/// CSS `%` / `height:100%` on a scrollport child resolve against the scroll
/// container's padding box — not against indefinite scroll *content*. Keep the
/// scrollport viewport height for those children so Home-style `1fr` grids fit;
/// leave other children on an indefinite CB so list rows do not Fill→0.
fn parent_box_for_flow_child(
    parent_layout: &crate::css_map::LayoutStyle,
    flow_box: ParentBox,
    scrollport_box: ParentBox,
    child_layout: &crate::css_map::LayoutStyle,
) -> ParentBox {
    if parent_layout.scrolls_y() && child_resolves_height_against_scrollport(child_layout) {
        ParentBox::new(
            flow_box.width.or(scrollport_box.width),
            scrollport_box.height,
        )
    } else {
        flow_box
    }
}

fn child_resolves_height_against_scrollport(layout: &crate::css_map::LayoutStyle) -> bool {
    matches!(
        layout.height,
        Some(LengthSpec::Fill)
            | Some(LengthSpec::Percent(_))
            | Some(LengthSpec::CalcPercentOffset { .. })
            | Some(LengthSpec::Viewport { .. })
            | Some(LengthSpec::CalcViewportOffset { .. })
            | Some(LengthSpec::Clamp3(_, _, _))
            | Some(LengthSpec::Min2(_, _))
            | Some(LengthSpec::Max2(_, _))
    )
}

/// Drop the main-axis CB for `auto` tracks so nested Fill/% does not inflate
/// min-content to the grid container's definite size.
fn indefinite_axis_for_auto_track(
    parent_layout: &crate::css_map::LayoutStyle,
    direction: FlexDirection,
    index: usize,
    parent_box: ParentBox,
) -> ParentBox {
    if !track_is_auto(parent_layout, direction, index) {
        return parent_box;
    }
    match direction {
        FlexDirection::Column => ParentBox::new(parent_box.width, None),
        FlexDirection::Row => ParentBox::new(None, parent_box.height),
    }
}

fn track_is_auto(
    parent_layout: &crate::css_map::LayoutStyle,
    direction: FlexDirection,
    index: usize,
) -> bool {
    let track = match direction {
        FlexDirection::Column => parent_layout
            .active_grid_rows()
            .and_then(|t| t.get(index).copied()),
        FlexDirection::Row => parent_layout
            .active_grid_columns()
            .and_then(|t| t.get(index).copied()),
    };
    matches!(track, Some(GridTrack::Auto))
}

fn track_is_sized(
    parent_layout: &crate::css_map::LayoutStyle,
    direction: FlexDirection,
    index: usize,
) -> bool {
    let track = match direction {
        FlexDirection::Column => parent_layout
            .active_grid_rows()
            .and_then(|t| t.get(index).copied()),
        FlexDirection::Row => parent_layout
            .active_grid_columns()
            .and_then(|t| t.get(index).copied()),
    };
    matches!(track, Some(t) if !matches!(t, GridTrack::Auto))
}

fn semantic_to_layout_node(
    snap: &SemanticSnapshot,
    id: WidgetId,
) -> Option<crate::measure::LayoutNode> {
    let widget = snap.get(id)?;
    let children = widget
        .children
        .iter()
        .filter_map(|&child| semantic_to_layout_node(snap, child))
        .collect::<Vec<_>>();
    Some(crate::measure::LayoutNode::with_children(
        id.to_string(),
        widget.props.layout.clone(),
        children,
    ))
}

/// When the grid has a definite main size, resolve all tracks (including `auto`
/// via style-model intrinsic measure) to Fixed outers — iced Shrink+Fill cannot
/// size `auto`/`1fr` correctly on its own.
///
/// Fr-only column tracks under `width:auto` still resolve here so nested
/// `parent_box` gets a per-track CB; callers must **not** wrap those outers in
/// iced `Fixed` (use [`prefer_fill_portion_grid_tracks`] + FillPortion instead).
fn precompute_grid_track_outers(
    snap: &SemanticSnapshot,
    visible: &[WidgetId],
    parent_layout: &crate::css_map::LayoutStyle,
    direction: FlexDirection,
    content_main: Option<f32>,
    content_cross: Option<f32>,
    child_margins: &[f32],
) -> Option<Vec<f32>> {
    let tracks = match direction {
        FlexDirection::Column => parent_layout.active_grid_rows()?,
        FlexDirection::Row => parent_layout.active_grid_columns()?,
    };
    if tracks.is_empty() || visible.is_empty() {
        return None;
    }
    let content = content_main.filter(|c| *c > 0.0)?;
    let cross = content_cross.unwrap_or(0.0).max(0.0);
    let gap = parent_layout.main_gap_against(
        direction,
        match direction {
            FlexDirection::Row => ParentBox::new(Some(content), content_cross),
            FlexDirection::Column => ParentBox::new(content_cross, Some(content)),
        },
    );
    let margin_total: f32 = child_margins.iter().copied().sum();
    let budget = (content - margin_total).max(0.0);
    let track_n = tracks.len().min(visible.len());
    let column_main = matches!(direction, FlexDirection::Column);
    let (cw, ch) = if column_main {
        (cross, content)
    } else {
        (content, cross)
    };
    let mut auto_sizes = vec![0.0f32; track_n];
    if tracks.iter().any(|t| matches!(t, GridTrack::Auto)) {
        for i in 0..track_n {
            if !matches!(tracks[i], GridTrack::Auto) {
                continue;
            }
            let Some(node) = semantic_to_layout_node(snap, visible[i]) else {
                continue;
            };
            auto_sizes[i] =
                crate::measure::measure_grid_auto_contribution(&node, cw, ch, column_main);
        }
    }
    let resolved = resolve_grid_track_sizes(&tracks[..track_n], budget, gap, &auto_sizes);
    let mut outers = Vec::with_capacity(visible.len());
    for i in 0..visible.len() {
        let track = resolved.get(i).copied().unwrap_or(0.0);
        let margin = child_margins.get(i).copied().unwrap_or(0.0);
        outers.push((track + margin).max(0.0));
    }
    Some(outers)
}

/// Fr-only column tracks under `width:auto`/`Fill`: Fixed outers from an ancestor
/// CB can crush the last column inside a narrower iced stretch. Prefer
/// FillPortion weights for the iced wrapper while still injecting track CBs.
fn prefer_fill_portion_grid_tracks(
    parent_layout: &crate::css_map::LayoutStyle,
    direction: FlexDirection,
) -> bool {
    let Some(tracks) = (match direction {
        FlexDirection::Column => parent_layout.active_grid_rows(),
        FlexDirection::Row => parent_layout.active_grid_columns(),
    }) else {
        return false;
    };
    matches!(direction, FlexDirection::Row)
        && !grid_main_size_is_definite(parent_layout, direction)
        && !tracks
            .iter()
            .any(|t| matches!(t, GridTrack::Auto | GridTrack::Px(_)))
}

/// When the flex container has a definite main size, pre-resolve grow+shrink
/// (same as measure) so iced Fixed bases shrink like T-F18/F19.
///
/// Each entry is `Some(px)` only when iced should force a Fixed main size.
/// Intrinsic `width/height:auto` items stay `None` so iced `Fit` measures
/// content — measure's auto→`min` (often 0) must not become `Fixed(0)`, which
/// zero-collapses text into a tall wrap column and starves siblings.
fn flex_main_overrides(
    snap: &SemanticSnapshot,
    visible: &[WidgetId],
    parent_layout: &crate::css_map::LayoutStyle,
    child_box: ParentBox,
    direction: FlexDirection,
) -> Option<Vec<Option<f32>>> {
    if visible.is_empty() {
        return None;
    }
    // Grid tracks own main sizes; do not override with flex shrink.
    match direction {
        FlexDirection::Row if parent_layout.active_grid_columns().is_some() => {
            return None;
        }
        FlexDirection::Column if parent_layout.active_grid_rows().is_some() => {
            return None;
        }
        _ => {}
    }
    let content_main = match direction {
        FlexDirection::Row => child_box.width.filter(|w| *w > 0.0)?,
        FlexDirection::Column => child_box.height.filter(|h| *h > 0.0)?,
    };
    let gap = parent_layout.main_gap_against(direction, child_box);
    let styles: Vec<&crate::css_map::LayoutStyle> = visible
        .iter()
        .filter_map(|&id| snap.get(id).map(|w| &w.props.layout))
        .collect();
    if styles.len() != visible.len() {
        return None;
    }
    let sizes = crate::measure::resolve_flex_children_main_sizes(
        &styles,
        direction,
        content_main,
        child_box.width,
        gap,
    );
    let vp = crate::css_map::active_viewport();
    Some(
        styles
            .iter()
            .zip(sizes)
            .map(|(style, resolved)| {
                flex_main_override_px(style, direction, resolved, child_box.width, content_main, vp)
            })
            .collect(),
    )
}

/// Decide whether a resolved flex main size should become an iced `Fixed` override.
fn flex_main_override_px(
    style: &crate::css_map::LayoutStyle,
    direction: FlexDirection,
    resolved: f32,
    margin_percent_base: Option<f32>,
    content_main: f32,
    viewport: Option<(f32, f32)>,
) -> Option<f32> {
    if style.grows() {
        return Some(resolved.max(0.0));
    }
    let main = style.child_main_length(direction);
    match main {
        None | Some(LengthSpec::Auto) | Some(LengthSpec::Shrink) => {
            // Intrinsic auto: measure maps these to min (often 0). Forcing
            // Fixed(0) makes row text wrap into a ~card-tall column.
            let min = match direction {
                FlexDirection::Row => style.resolved_min_width(margin_percent_base, viewport),
                FlexDirection::Column => {
                    style.resolved_min_height(Some(content_main), viewport)
                }
            };
            if min > 0.0 {
                Some(resolved.max(0.0))
            } else {
                None
            }
        }
        Some(_) => Some(resolved.max(0.0)),
    }
}

/// Sum + per-child main-axis margin (horizontal or vertical) for grid budget.
fn grid_children_axis_margins(
    snap: &SemanticSnapshot,
    children: &[WidgetId],
    percent_base: Option<f32>,
    horizontal: bool,
) -> (f32, Vec<f32>) {
    let mut per = Vec::with_capacity(children.len());
    let mut total = 0.0f32;
    for &id in children {
        let m = snap
            .get(id)
            .map(|w| w.props.layout.resolved_margin_against(percent_base))
            .unwrap_or_default();
        let v = if horizontal {
            m.left + m.right
        } else {
            m.top + m.bottom
        };
        per.push(v);
        total += v;
    }
    (total, per)
}

/// Known-content grid track outer sizes after deducting child margins (matches measure).
/// Each outer = track + that child's axis margin so iced Fixed wrapping margin-padding
/// neither double-shrinks content nor double-expands the row/column.
fn resolve_grid_track_outers(
    tracks: &[GridTrack],
    content: f32,
    gap: f32,
    margin_total: f32,
    child_margins: &[f32],
) -> Vec<f32> {
    let track_n = tracks.len();
    let budget = (content - margin_total).max(0.0);
    let resolved = resolve_grid_column_widths(&tracks[..track_n], budget, gap);
    resolved
        .into_iter()
        .enumerate()
        .map(|(i, track)| (track + child_margins.get(i).copied().unwrap_or(0.0)).max(0.0))
        .collect()
}

/// `grid-template-columns` → Fixed（定宽已知时）或加权 `FillPortion`（minmax 下限经 `.min`）。
fn apply_grid_track_width<'a, Message: 'a>(
    child: Element<'a, Message>,
    parent_layout: &crate::css_map::LayoutStyle,
    index: usize,
    direction: FlexDirection,
    content_w: Option<f32>,
    child_margins: &[f32],
) -> Element<'a, Message> {
    if direction != FlexDirection::Row {
        return child;
    }
    let Some(tracks) = parent_layout.active_grid_columns() else {
        return child;
    };
    if index >= tracks.len() {
        return child;
    }
    let gap = parent_layout.main_gap_against(FlexDirection::Row, ParentBox::new(content_w, None));
    let margin_total: f32 = child_margins.iter().copied().sum();
    let child_margin_main = child_margins.get(index).copied().unwrap_or(0.0);
    // Known-content Fixed outers treat Auto≈0 without intrinsics; skip when any
    // Auto track is present and let iced Shrink + fr FillPortion size instead.
    // Also skip Fixed when the grid's own main size is Fill/auto — ancestor CB
    // can be wider than the eventual iced stretch (e.g. 912 vs 692) and crush
    // the last fr column.
    let has_auto = tracks.iter().any(|t| matches!(t, GridTrack::Auto));
    let definite_main = grid_main_size_is_definite(parent_layout, direction);
    if !has_auto && definite_main {
        if let Some(cw) = content_w.filter(|w| *w > 0.0) {
            let outers = resolve_grid_track_outers(tracks, cw, gap, margin_total, child_margins);
            if let Some(outer) = outers.get(index).copied() {
                return container(child).width(Length::Fixed(outer)).into();
            }
        }
    }
    let Some(track) = tracks.get(index) else {
        return child;
    };
    match *track {
        GridTrack::Px(px) => container(child)
            .width(Length::Fixed((px + child_margin_main).max(0.0)))
            .into(),
        other => container(child)
            .width(grid_track_fallback_length(other))
            .into(),
    }
}

/// `grid-template-rows` → Fixed（定高已知时）或加权 `FillPortion`（minmax 下限经 `.min`）。
fn apply_grid_track_height<'a, Message: 'a>(
    child: Element<'a, Message>,
    parent_layout: &crate::css_map::LayoutStyle,
    index: usize,
    direction: FlexDirection,
    content_h: Option<f32>,
    child_margins: &[f32],
) -> Element<'a, Message> {
    if direction != FlexDirection::Column {
        return child;
    }
    let Some(tracks) = parent_layout.active_grid_rows() else {
        return child;
    };
    if index >= tracks.len() {
        return child;
    }
    let gap = parent_layout.main_gap_against(FlexDirection::Column, ParentBox::new(None, content_h));
    let margin_total: f32 = child_margins.iter().copied().sum();
    let child_margin_main = child_margins.get(index).copied().unwrap_or(0.0);
    // Same Auto rule as columns: Fixed outers without intrinsics collapse Auto
    // to 0 and steal the 1fr remainder — use Shrink/FillPortion instead.
    // Skip Fixed when the grid height is Fill/auto (same ancestor-CB trap).
    let has_auto = tracks.iter().any(|t| matches!(t, GridTrack::Auto));
    let definite_main = grid_main_size_is_definite(parent_layout, direction);
    if !has_auto && definite_main {
        if let Some(ch) = content_h.filter(|h| *h > 0.0) {
            let outers = resolve_grid_track_outers(tracks, ch, gap, margin_total, child_margins);
            if let Some(outer) = outers.get(index).copied() {
                return container(child).height(Length::Fixed(outer)).into();
            }
        }
    }
    let Some(track) = tracks.get(index) else {
        return child;
    };
    match *track {
        GridTrack::Px(px) => container(child)
            .height(Length::Fixed((px + child_margin_main).max(0.0)))
            .into(),
        other => container(child)
            .height(grid_track_fallback_length(other))
            .into(),
    }
}

/// True when the grid container declares a definite main size (px/%/viewport
/// math). `Fill`/`auto` sizes are assigned by iced stretch later — Fixed track
/// outers from an ancestor CB are unreliable then.
fn grid_main_size_is_definite(
    layout: &crate::css_map::LayoutStyle,
    direction: FlexDirection,
) -> bool {
    let spec = match direction {
        FlexDirection::Row => layout.width,
        FlexDirection::Column => layout.height,
    };
    matches!(
        spec,
        Some(LengthSpec::Px(_))
            | Some(LengthSpec::Percent(_))
            | Some(LengthSpec::CalcPercentOffset { .. })
            | Some(LengthSpec::Viewport { .. })
            | Some(LengthSpec::CalcViewportOffset { .. })
            | Some(LengthSpec::Em(_))
            | Some(LengthSpec::Rem(_))
            | Some(LengthSpec::Min2(_, _))
            | Some(LengthSpec::Max2(_, _))
            | Some(LengthSpec::Clamp3(_, _, _))
    )
}

/// FillPortion fallback when content size is unknown; honor minmax min/max clamps.
fn grid_track_fallback_length(track: GridTrack) -> Length {
    match track {
        GridTrack::Px(px) => Length::Fixed(px),
        // Without CB, % cannot resolve; known-content path uses resolve_grid_track_outers.
        // Use the same ×100 scale as `fr` so `25% 1fr` stays ~25:100 (1:4), not 1:100.
        GridTrack::Percent(pct) => Length::FillPortion(fr_portion(pct / 100.0)),
        GridTrack::Fr(fr) => Length::FillPortion(fr_portion(fr)),
        GridTrack::MinMax { min_px, fr, max_px } => {
            let mut length = Length::FillPortion(fr_portion(fr));
            if min_px > 0.0 {
                length = length.min(min_px);
            }
            if let Some(max) = max_px {
                length = length.max(max);
            }
            length
        }
        // CSS `auto` is content-sized — never share free space like `1fr`.
        GridTrack::Auto => Length::Fit,
    }
}

/// Map CSS `fr` / percent-fraction weight to iced `FillPortion` (integer ≥ 1).
///
/// Always use a ×100 scale so `1fr` and `1.3fr` stay commensurate
/// (`100` vs `130`), and so grid `%` tracks share the same scale
/// (`25%` → `fr_portion(0.25)` = `25` vs `1fr` = `100` → 1:4).
/// Mixing whole-number portions (`1`) with ×100 (`130`/`100`) previously
/// made `1.3fr 1fr` ~130:1 and `25% 1fr` ~1:100.
fn fr_portion(fr: f32) -> u16 {
    (fr * 100.0).round().clamp(1.0, 10_000.0) as u16
}

/// Cross-axis stretch length. Definite CB → Fill (honoring auto-track
/// demotion); indefinite → Shrink so content sizes the auto track.
fn cross_axis_stretch_length(
    vertical: bool,
    percent_base: Option<f32>,
    layout: &crate::css_map::LayoutStyle,
) -> Length {
    if percent_base.filter(|v| *v > 0.0).is_none() {
        return Length::Fit;
    }
    length_from_spec(Some(LengthSpec::Fill), percent_base, layout, vertical)
}

fn push_gap_then_fill_col<'a, Message: 'a>(
    mut col: iced::widget::Column<'a, Message>,
    gap: f32,
) -> iced::widget::Column<'a, Message> {
    if gap > 0.0 {
        col = col.push(space().height(Length::Fixed(gap)));
    }
    col.push(space().height(Length::Fill))
}

/// Re-assert column height after `push` / `push_justified`.
///
/// iced `Column::push` upgrades `Fit`/`Shrink` → `Fill` via `Length::enclose`
/// when a child (or SpaceBetween spacer) reports Fill height.
///
/// Mapping (sena-nana iced fork):
/// - Author height / grow → honor `height`
/// - CSS `height:auto` → [`Length::Fit`] (intrinsic, **no** compression).
///   Never use [`Length::Shrink`] for auto: Shrink enables iced compression and
///   crushes Fixed chart children under a tight card.
///
/// Auto-height *rows* (headings) stay Fit via [`pin_flex_row_cross_or_main_height`].
fn pin_flex_container_main_length<'a, Message: 'a>(
    col: iced::widget::Column<'a, Message>,
    height: Option<Length>,
    layout: &crate::css_map::LayoutStyle,
    parent_box: ParentBox,
    vertical: bool,
) -> iced::widget::Column<'a, Message> {
    let _ = (vertical, parent_box);
    match height {
        Some(h) if !(layout.scrolls_y() && matches!(h, Length::Fill)) => col.height(h),
        Some(_) => col, // scrollport owns Fill; leave content intrinsic
        None if layout.grows() => col.height(Length::Fill),
        None => col.height(Length::Fit),
    }
}

/// Pin row height after push. CSS `height:auto` → Fit (intrinsic, no compression).
fn pin_flex_row_cross_or_main_height<'a, Message: 'a>(
    row: iced::widget::Row<'a, Message>,
    height: Option<Length>,
    layout: &crate::css_map::LayoutStyle,
) -> iced::widget::Row<'a, Message> {
    match height {
        Some(h) => row.height(h),
        None if layout.grows() => row.height(Length::Fill),
        None => row.height(Length::Fit),
    }
}

fn push_gap_then_fill_row<'a, Message: 'a>(
    mut r: iced::widget::Row<'a, Message>,
    gap: f32,
) -> iced::widget::Row<'a, Message> {
    if gap > 0.0 {
        r = r.push(space().width(Length::Fixed(gap)));
    }
    r.push(space().width(Length::Fill))
}

fn push_justified<'a, Message: 'a>(
    mut col: iced::widget::Column<'a, Message>,
    children: Vec<Element<'a, Message>>,
    justify: JustifySpec,
    gap: f32,
) -> iced::widget::Column<'a, Message> {
    if children.is_empty() {
        return col;
    }
    match justify {
        JustifySpec::Start => {
            for c in children {
                col = col.push(c);
            }
        }
        JustifySpec::End => {
            col = col.push(space().height(Length::Fill));
            for c in children {
                col = col.push(c);
            }
        }
        JustifySpec::Center => {
            col = col.push(space().height(Length::Fill));
            for c in children {
                col = col.push(c);
            }
            col = col.push(space().height(Length::Fill));
        }
        JustifySpec::SpaceBetween => {
            if children.len() == 1 {
                col = col.push(children.into_iter().next().unwrap());
            } else {
                let mut iter = children.into_iter();
                if let Some(first) = iter.next() {
                    col = col.push(first);
                }
                for c in iter {
                    // Fixed gap + Fill free space ≡ measure's gap + between.
                    col = push_gap_then_fill_col(col, gap);
                    col = col.push(c);
                }
            }
        }
        // SpaceEvenly: n+1 equal Fill spacers (ends + between).
        JustifySpec::SpaceEvenly => {
            col = col.push(space().height(Length::Fill));
            let mut iter = children.into_iter().peekable();
            while let Some(c) = iter.next() {
                col = col.push(c);
                if iter.peek().is_some() {
                    col = push_gap_then_fill_col(col, gap);
                }
            }
            col = col.push(space().height(Length::Fill));
        }
        // SpaceAround: half at ends ≈ 1 Fill at ends, 2 Fill between items.
        JustifySpec::SpaceAround => {
            col = col.push(space().height(Length::Fill));
            let mut iter = children.into_iter().peekable();
            while let Some(c) = iter.next() {
                col = col.push(c);
                if iter.peek().is_some() {
                    col = push_gap_then_fill_col(col, gap);
                    col = col.push(space().height(Length::Fill));
                }
            }
            col = col.push(space().height(Length::Fill));
        }
    }
    col
}

fn push_justified_row<'a, Message: 'a>(
    mut r: iced::widget::Row<'a, Message>,
    children: Vec<Element<'a, Message>>,
    justify: JustifySpec,
    gap: f32,
) -> iced::widget::Row<'a, Message> {
    if children.is_empty() {
        return r;
    }
    match justify {
        JustifySpec::Start => {
            for c in children {
                r = r.push(c);
            }
        }
        JustifySpec::End => {
            r = r.push(space().width(Length::Fill));
            for c in children {
                r = r.push(c);
            }
        }
        JustifySpec::Center => {
            r = r.push(space().width(Length::Fill));
            for c in children {
                r = r.push(c);
            }
            r = r.push(space().width(Length::Fill));
        }
        JustifySpec::SpaceBetween => {
            if children.len() == 1 {
                r = r.push(children.into_iter().next().unwrap());
            } else {
                let mut iter = children.into_iter();
                if let Some(first) = iter.next() {
                    r = r.push(first);
                }
                for c in iter {
                    // Fixed gap + Fill free space ≡ measure's gap + between.
                    r = push_gap_then_fill_row(r, gap);
                    r = r.push(c);
                }
            }
        }
        JustifySpec::SpaceEvenly => {
            r = r.push(space().width(Length::Fill));
            let mut iter = children.into_iter().peekable();
            while let Some(c) = iter.next() {
                r = r.push(c);
                if iter.peek().is_some() {
                    r = push_gap_then_fill_row(r, gap);
                }
            }
            r = r.push(space().width(Length::Fill));
        }
        JustifySpec::SpaceAround => {
            r = r.push(space().width(Length::Fill));
            let mut iter = children.into_iter().peekable();
            while let Some(c) = iter.next() {
                r = r.push(c);
                if iter.peek().is_some() {
                    r = push_gap_then_fill_row(r, gap);
                    r = r.push(space().width(Length::Fill));
                }
            }
            r = r.push(space().width(Length::Fill));
        }
    }
    r
}

fn resolve_container_height(
    layout: &crate::css_map::LayoutStyle,
    parent_box: ParentBox,
) -> Option<Length> {
    let pin = |h: Length| -> Option<Length> {
        // Same contract as apply_flex_child_sizing: Fill under an indefinite
        // parent CB collapses to 0 in iced (scroll content is infinite-max).
        if parent_box.height.is_none()
            && !layout.scrolls_y()
            && matches!(h, Length::Fill | Length::FillPortion(_))
        {
            None
        } else {
            Some(h)
        }
    };
    if let Some(h) = layout.height {
        return pin(length_from_spec(Some(h), parent_box.height, layout, true));
    }
    // CSS grid default stretch: height:auto items fill a definite non-auto
    // row track so nested height:100%/Fill resolve (shell 1fr → workspace).
    if matches!(
        crate::css_map::grid_item_stretch_main_vertical(),
        Some(true)
    ) && parent_box.height.filter(|h| *h > 0.0).is_some()
    {
        return pin(Length::Fill);
    }
    let mh = layout.resolved_min_height(parent_box.height, crate::css_map::active_viewport());
    if mh > 0.0 {
        return Some(Length::Fixed(mh));
    }
    // min-height:0 only allows shrink. Height-chain Fill comes from
    // height:100% / min-height:100% (already mapped to height Fill) or flex-grow.
    if layout.grows() {
        return pin(Length::Fill);
    }
    None
}

/// Containing box for in-flow children of `layout`.
///
/// Scrollports pin their viewport via [`scrollable`]; iced gives the *content*
/// an infinite max height. Passing a definite height CB into that content keeps
/// `flex-grow` / `height:Fill` children on `Length::Fill`, which collapses to 0
/// under infinite max — emptying sidebar body lists while the style-model
/// measure still reports a tall box. Drop height so grow maps to Shrink
/// (intrinsic content); keep width so cross-axis Fill/stretch still works.
fn flow_child_containing_box(
    layout: &crate::css_map::LayoutStyle,
    child_box: ParentBox,
) -> ParentBox {
    if layout.scrolls_y() {
        ParentBox::new(child_box.width, None)
    } else {
        child_box
    }
}

/// iced `Length::Fill` under an infinite max constraint collapses to 0.
/// Scrollports with an indefinite containing block pin to the active viewport.
fn scroll_port_height(parent_height: Option<f32>) -> Length {
    if parent_height.filter(|h| *h > 0.0).is_some() {
        Length::Fill
    } else {
        crate::css_map::active_viewport()
            .map(|(_, vh)| Length::Fixed(vh.max(0.0)))
            .unwrap_or(Length::Fill)
    }
}

fn finalize_layout_container<'a, Message>(
    content: Element<'a, Message>,
    layout: &crate::css_map::LayoutStyle,
    parent_box: ParentBox,
    scroll_id: Option<WidgetId>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: 'a,
{
    // apply_layout_chrome paints chrome only; scrollport is attached once here
    // with a stable iced Id so host scrollIntoView can drive AbsoluteOffset.
    let mut el = apply_layout_chrome(content, layout, parent_box);
    if layout.scrolls_y() {
        let mut s = scrollable(el)
            .width(Length::Fill)
            .height(scroll_port_height(parent_box.height));
        if let Some(id) = scroll_id {
            let map = map_event.clone();
            s = s
                .id(iced::widget::Id::from(crate::scroll::scrollable_widget_id(id)))
                .on_scroll(move |viewport| {
                    let offset = viewport.absolute_offset();
                    let bounds = viewport.bounds();
                    let content = viewport.content_bounds();
                    map(BridgeEvent::Scroll {
                        id,
                        offset: crate::ScrollOffset {
                            x: offset.x,
                            y: offset.y,
                        },
                        metrics: nana_ui_runtime::ScrollMetrics {
                            viewport_width: bounds.width,
                            viewport_height: bounds.height,
                            content_width: content.width,
                            content_height: content.height,
                        },
                    })
                });
        }
        el = s.into();
    }
    el
}

/// Width for the inner `row`/`column` before [`apply_layout_chrome`].
///
/// Chrome wraps with the same definite `width` plus padding. If the inner flex
/// also keeps `Fixed(W)`, iced treats the child's min size as W while the
/// parent content box is only W−padding — the border box expands (sidebar
/// 260+pad inventing a workspace seam). Use Fill so the inner fills the
/// chrome content box instead.
fn inner_flex_axis_length(
    layout: &crate::css_map::LayoutStyle,
    parent_box: ParentBox,
    computed: Length,
    horizontal: bool,
) -> Length {
    let pad = layout.resolved_padding_against(parent_box.width);
    let has_pad = if horizontal {
        pad.left > 0.0 || pad.right > 0.0
    } else {
        pad.top > 0.0 || pad.bottom > 0.0
    };
    let has_definite = if horizontal {
        layout.width.is_some()
    } else {
        layout.height.is_some()
    };
    if has_definite && has_pad {
        Length::Fill
    } else {
        computed
    }
}

fn apply_layout_chrome<'a, Message: 'a>(
    content: Element<'a, Message>,
    layout: &crate::css_map::LayoutStyle,
    parent_box: ParentBox,
) -> Element<'a, Message> {
    let pad = layout.resolved_padding_against(parent_box.width);
    let mut margin = layout.resolved_margin_against(parent_box.width);
    let (dx, dy) = layout.relative_offset_against(parent_box.width, parent_box.height);
    if dx != 0.0 || dy != 0.0 {
        margin.left = (margin.left + dx).max(0.0);
        margin.top = (margin.top + dy).max(0.0);
    }
    let needs_pad = !pad.is_zero();
    let needs_margin = !margin.is_zero();
    let needs_paint = layout.has_surface_paint();
    let needs_clip = layout.clips_overflow();
    let height = resolve_container_height(layout, parent_box).and_then(|h| {
        // Scrollport provides Fill; clamping the scrolled content to Fill hides it.
        if layout.scrolls_y() && matches!(h, Length::Fill) {
            None
        } else {
            Some(h)
        }
    });
    let width = layout
        .width
        .map(|w| length_from_spec(Some(w), parent_box.width, layout, false));
    if !needs_pad && !needs_margin && !needs_paint && !needs_clip && height.is_none() && width.is_none()
    {
        return content;
    }
    if needs_clip && !needs_pad && !needs_margin && !needs_paint && height.is_none() && width.is_none()
    {
        // Clip without inventing Fill — used size still comes from flex/grid parents.
        return container(content).clip(true).into();
    }
    let mut c = container(content);
    // Do not invent Fill for margin-only chrome: flex row items with
    // width:auto must stay intrinsic. Block stretch / explicit width still
    // arrive via `width` or later `apply_flex_child_sizing` cross-Fill.
    if let Some(w) = width {
        c = c.width(w);
    } else if needs_pad || needs_paint || needs_clip || height.is_some() {
        c = c.width(Length::Fill);
    }
    if needs_pad {
        c = c.padding(Padding {
            top: pad.top,
            right: pad.right,
            bottom: pad.bottom,
            left: pad.left,
        });
    }
    if let Some(h) = height {
        c = c.height(h);
    }
    if needs_clip {
        c = c.clip(true);
    }
    if needs_paint {
        let paint = surface_paint_from(layout);
        c = c.style(move |_theme| surface_style(paint));
    }
    let mut el: Element<'a, Message> = c.into();
    if needs_margin {
        el = container(el)
            .padding(Padding {
                top: margin.top,
                right: margin.right,
                bottom: margin.bottom,
                left: margin.left,
            })
            .into();
    }
    el
}

fn rgba_color(c: [f32; 4]) -> Color {
    Color::from_rgba(c[0], c[1], c[2], c[3])
}
