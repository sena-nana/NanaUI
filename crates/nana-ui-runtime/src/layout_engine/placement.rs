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
    output: &mut PassOutput,
) -> Result<(), UiWorldError> {
    place_node_scoped(
        id,
        origin,
        size,
        containing,
        viewport,
        parent_font_px,
        None,
        nodes,
        intrinsic,
        output,
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
    /// A child restyled so that what the plan assumed of its children when
    /// it was built no longer holds: the container places in full.
    Stale,
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

/// After a replay that placed only part of `container`, descend into the
/// children it did not visit that still lead to an affected node, at their
/// kept boxes: the replay left them where they were, but not what is inside.
fn descend_unvisited_reaching(
    container: StableNodeId,
    plan: &ContainerPlan,
    patch: Option<&PlanPatch>,
    descend: &mut Vec<Descend>,
    nodes: &LayoutInputMap<'_>,
    output: &PassOutput,
    scope: &ScopeContext<'_>,
) {
    let reaching = children_reaching_affected(container, plan, scope);
    if reaching.is_empty() {
        return;
    }
    // The children the replay itself descends into: their subtrees are not
    // placed yet, so their boxes are not in `output`. Few children reach an
    // edit, so a scan of the list is cheaper than indexing it.
    let visited: Option<HashSet<StableNodeId>> =
        (reaching.len() > 8).then(|| descend.iter().map(|visit| visit.child).collect());
    let mut unvisited = Vec::new();
    for index in reaching {
        // Where the replay left it, which the pass commits with the plan.
        let entry = match patch {
            Some(patch) => patch.entry(plan, index as usize),
            None => &plan.entries[index as usize],
        };
        let (child, origin, size) = (entry.child, entry.origin, entry.size);
        let replayed = match &visited {
            Some(visited) => visited.contains(&child),
            None => descend.iter().any(|visit| visit.child == child),
        };
        if output.contains_key(&child) || replayed {
            continue;
        }
        unvisited.push(Descend {
            child,
            origin,
            size,
            containing: plan.content,
            inherited: inherited_tracks(plan, child, nodes),
        });
    }
    descend.append(&mut unvisited);
}

/// The grid tracks `child` inherits from the grid `plan` placed it in.
fn inherited_tracks(
    plan: &ContainerPlan,
    child: StableNodeId,
    nodes: &LayoutInputMap<'_>,
) -> Option<Box<super::grid::InheritedGridTracks>> {
    plan.grid.as_deref().and_then(|grid| {
        nodes
            .style_ref(child)
            .and_then(|child_style| grid.inherited_tracks_for(child, child_style.as_ref()))
    })
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
        let entry = plan.entries[index].clone();
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
        // The plan read its children's styles once, when it was built: that
        // none reads its main extent let a main size change keep the plan,
        // and that none reads its cross size kept the flow under a cross
        // size change. A child restyled to read either -- a fixed height
        // back to filling -- would keep a size the container no longer has.
        // The same for a child whose subtree came to read the block extent it
        // is offered (`world/block_reads.rs`): the plan kept it under a main
        // size change that it now reads.
        let block_read_arrived = !plan.main_dependent
            && plan.main_direction == FlexDirection::Column
            && nodes.world.block_extent_reaches(child);
        if block_read_arrived
            || style_moved
                && ((!plan.main_dependent && reads_main_extent(&current, plan.main_direction))
                    || (plan.cross_independent
                        && (!matches!(
                            current.resolved_align_self(plan.style.child_align_items()),
                            AlignSpec::Start | AlignSpec::Stretch
                        ) || reads_container_size(&current, plan.main_direction))))
        {
            return Ok(PlanCheck::Stale);
        }
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
            && match entry.at_main.as_deref() {
                Some(&(main, cached)) => {
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
        if on_measure && entry.baseline.is_some() {
            prime_first_baselines(
                [(child, entry.size)],
                plan.writing
                    .inline_size(plan.content.width, plan.content.height),
                viewport,
                plan.child_font_px,
                nodes,
                intrinsic,
                Some(scope),
            )?;
        }
        let baseline_moved = on_measure
            && entry.baseline.get().is_some_and(|aligned| {
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

/// The size `plan` recorded for `child`, while the change closure reaches
/// nothing in it, its style is the recorded one, and that style does not
/// read the main extent, the one extent of the content box allowed to move.
fn kept_size(
    child: StableNodeId,
    plan: &ContainerPlan,
    direction: FlexDirection,
    nodes: &LayoutInputMap<'_>,
    scope: &ScopeContext<'_>,
) -> Option<Size> {
    if scope.affected.contains(&child) || scope.reach.reaches(child) {
        return None;
    }
    let style = nodes.style_ref(child)?;
    let entries = &plan.entries;
    let entry = entries.get(plan.entry_index(child)? as usize)?;
    (entry.child == child
        && (Arc::ptr_eq(style.arc(), &entry.style) || layout_inputs_equal(&style, &entry.style))
        && !reads_main_extent(&style, direction))
    .then_some(entry.intrinsic)
}

/// One child a sequential replay places: its id, and the style and intrinsic
/// size the plan holds for it -- `None` for a child that arrived in a
/// child-list edit, which is measured as a child on the frontier is.
struct RunChild {
    child: StableNodeId,
    planned: Option<(Arc<nana_ui_core::LayoutStyle>, Size)>,
}

impl RunChild {
    fn planned(entry: &PlannedChild) -> Self {
        Self {
            child: entry.child,
            planned: Some((Arc::clone(&entry.style), entry.intrinsic)),
        }
    }
}

/// Replay a sequential container's children from `from` onward, keeping every
/// position before it.
///
/// This and [`replay_child_list_edit`] share the one place that reproduces the
/// container's per-child arithmetic rather than calling into the placement
/// loop, [`replay_sequential_run`]. Anything it meets that it cannot express,
/// it refuses by returning `false`, and the caller falls back to a full
/// container relayout.
#[allow(clippy::too_many_arguments)]
fn replay_sequential_suffix(
    plan: &ContainerPlan,
    from: usize,
    content: Size,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    descend: &mut Vec<Descend>,
    scope: &ScopeContext<'_>,
) -> Result<Option<PlanPatch>, UiWorldError> {
    let cursor = plan
        .entries
        .get(from)
        .map_or(0.0, |entry| entry.cursor_before);
    let run: Vec<RunChild> = plan.entries[from..].iter().map(RunChild::planned).collect();
    let Some(replayed) = replay_sequential_run(
        plan, cursor, run, content, viewport, nodes, intrinsic, descend, scope,
    )?
    else {
        return Ok(None);
    };
    let shifted = shift_reversed_prefix(plan, from, content, descend);
    // The plan stays the previous pass's; what moved is a patch the pass
    // commits, and a bail-out above leaves nothing.
    let mut rows = Vec::with_capacity(shifted.len() + replayed.len());
    rows.extend(shifted.into_iter().map(|(index, origin)| {
        let mut entry = plan.entries[index].clone();
        entry.origin = origin;
        (index as u32, entry)
    }));
    rows.extend(
        replayed
            .into_iter()
            .enumerate()
            .map(|(offset, entry)| ((from + offset) as u32, entry)),
    );
    Ok(Some(PlanPatch {
        rows,
        placed_main: Some(main_extent(content, plan.main_direction)),
    }))
}

/// Give `run`, children of a sequential container in placement order, their
/// boxes from `cursor` on, and return what the plan records for each. The
/// children whose subtrees have to be placed again go to `descend`, entered
/// once this frame is gone; on a refusal what it pushed is taken back.
///
/// This is the one place that reproduces the container's per-child arithmetic
/// rather than calling into the placement loop, so it is deliberately narrow:
/// `ContainerPlan::sequential` already excluded wrapping, space-distributing
/// justification, grid tracks, auto main margins, non-Start/Stretch cross
/// alignment and any grow/shrink redistribution. Like the loop it is
/// flow-relative — the cursor and each child's leading margin are read from
/// the start edge — and turns a reversed axis onto the page the same way.
/// Anything this function meets that it cannot express, it refuses by
/// returning `None`.
#[allow(clippy::too_many_arguments)]
fn replay_sequential_run(
    plan: &ContainerPlan,
    cursor: f32,
    run: Vec<RunChild>,
    content: Size,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    descend: &mut Vec<Descend>,
    scope: &ScopeContext<'_>,
) -> Result<Option<Vec<PlannedChild>>, UiWorldError> {
    let pushed = descend.len();
    let replayed = replay_sequential_children(
        plan, cursor, run, content, viewport, nodes, intrinsic, descend, scope,
    )?;
    if replayed.is_none() {
        descend.truncate(pushed);
    }
    Ok(replayed)
}

/// [`replay_sequential_run`] before a refusal takes back what it pushed.
#[allow(clippy::too_many_arguments)]
fn replay_sequential_children(
    plan: &ContainerPlan,
    mut cursor: f32,
    run: Vec<RunChild>,
    content: Size,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    descend: &mut Vec<Descend>,
    scope: &ScopeContext<'_>,
) -> Result<Option<Vec<PlannedChild>>, UiWorldError> {
    let direction = plan.main_direction;
    let writing = plan.writing;
    let container_align = plan.style.child_align_items();
    let container_cross = cross_extent(plan.content, direction);
    // The replay places against this pass's main extent: on a reversed main
    // axis that is where the far edge, and every origin, is measured from.
    let full_main = main_extent(content, direction);
    let (main_reversed, cross_reversed) = (plan.main_reversed, plan.cross_reversed);
    let mut replayed: Vec<PlannedChild> = Vec::with_capacity(run.len());

    for RunChild { child, planned } in run {
        // Outside the closure nothing can have moved, so reuse the cached
        // measurement; inside it, both were already refreshed by the check.
        // A child that arrived has neither.
        let (style_arc, child_intrinsic) = match planned {
            Some((cached_style, cached_intrinsic)) if !scope.measure.contains(&child) => {
                if scope.affected.contains(&child) {
                    let style = nodes.style(child).unwrap_or(cached_style);
                    (style, cached_intrinsic)
                } else {
                    (cached_style, cached_intrinsic)
                }
            }
            _ => {
                let Some(style) = nodes.style(child) else {
                    return Ok(None);
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
            }
        };
        let child_style = style_arc.as_ref();
        // Leaving the flow (or being omitted) changes sibling coupling the
        // suffix arithmetic does not model. The caller lays the container out
        // again, still inside this formatting context.
        if crate::layout_node::leaves_parent_flow(Some(&plan.style), child_style)
            || child_style.omits_box()
        {
            return Ok(None);
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
            return Ok(None);
        }
        let align = child_style.resolved_align_self(container_align);
        if !matches!(align, AlignSpec::Start | AlignSpec::Stretch) {
            return Ok(None);
        }
        // The replay hands every child its intrinsic main size, which is only
        // the used size while nothing redistributes free space. A child that
        // opts into growing or shrinking does, so refuse it here -- an edit can
        // introduce either one on a child that had neither when the plan was
        // recorded.
        if child_style.flex_grow.unwrap_or(0.0) > 0.0
            || child_style.flex_shrink.unwrap_or(0.0) > 0.0
        {
            return Ok(None);
        }
        let child_fonts = fonts_of(child_style, plan.child_font_px);
        let margin = child_style.resolved_margin_against_fonts(
            Some(writing.inline_size(plan.content.width, plan.content.height)),
            child_fonts,
        );
        let mut child_size = child_intrinsic;
        if align == AlignSpec::Stretch && {
            let natural_ratio = has_natural_ratio(nodes.world, child);
            !cross_axis_is_definite(child_style, direction, natural_ratio)
                && !block_keeps_content_size(&plan.style, writing, child_style, natural_ratio)
        } {
            let cross_available = container_cross - cross_margin(margin, direction);
            set_cross_extent(&mut child_size, direction, cross_available.max(0.0));
        }
        fill_auto_height_from_aspect_ratio(
            child_style,
            &mut child_size,
            Some(writing.inline_size(plan.content.width, plan.content.height)),
            Some(plan.child_available.height),
            viewport,
            child_fonts,
        );
        let (main_lead, main_trail) = main_margins(margin, direction, main_reversed);
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
        let child_offset = match direction {
            FlexDirection::Row => Point {
                x: main_start,
                y: cross_offset,
            },
            FlexDirection::Column => Point {
                x: cross_offset,
                y: main_start,
            },
        };
        let child_origin = Point {
            x: plan.content_origin.x + child_offset.x,
            y: plan.content_origin.y + child_offset.y,
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
            descend.push(Descend {
                child,
                origin: child_origin,
                size: child_size,
                containing: plan.content,
                inherited: None,
            });
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
            baseline: PackedOptF32::NONE,
        });
    }
    Ok(Some(replayed))
}

/// A child's main margins in the order its axis reads them: the leading one
/// first, which on a reversed axis is the physical end one.
fn main_margins(
    margin: nana_ui_core::PaddingSpec,
    direction: FlexDirection,
    reversed: bool,
) -> (f32, f32) {
    if reversed {
        (
            main_end_margin(margin, direction),
            main_start_margin(margin, direction),
        )
    } else {
        (
            main_start_margin(margin, direction),
            main_end_margin(margin, direction),
        )
    }
}

/// A reversed main axis measures every origin back from the far edge. When
/// the container's main size moved, the children before `from` keep their
/// flow positions and move with that edge, all by the same amount: the work
/// is the boxes that really moved, and nothing is measured again. Hands them
/// to `descend` and returns their new origins by entry index.
fn shift_reversed_prefix(
    plan: &ContainerPlan,
    from: usize,
    content: Size,
    descend: &mut Vec<Descend>,
) -> Vec<(usize, Point)> {
    let direction = plan.main_direction;
    let shift = if plan.main_reversed {
        main_extent(content, direction) - plan.placed_main
    } else {
        0.0
    };
    if shift == 0.0 {
        return Vec::new();
    }
    let prefix: Vec<(usize, StableNodeId, Point, Size)> = plan.entries[..from]
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut origin = entry.origin;
            match direction {
                FlexDirection::Row => origin.x += shift,
                FlexDirection::Column => origin.y += shift,
            }
            (index, entry.child, origin, entry.size)
        })
        .collect();
    let mut shifted = Vec::with_capacity(prefix.len());
    for (index, child, origin, size) in prefix {
        descend.push(Descend {
            child,
            origin,
            size,
            containing: plan.content,
            inherited: None,
        });
        shifted.push((index, origin));
    }
    shifted
}

/// The entry of the nearest child in flow among `beside`, the children next
/// to a child-list edit walking away from it: `Some(None)` when the list
/// ends first, `None` when more out-of-flow children than a replay should
/// walk stand in between.
fn nearest_entry(
    plan: &ContainerPlan,
    beside: impl Iterator<Item = StableNodeId>,
) -> Option<Option<usize>> {
    const FARTHEST: usize = 16;
    for (walked, child) in beside.enumerate() {
        if let Some(index) = plan.entry_index(child) {
            return Some(Some(index as usize));
        }
        if walked + 1 == FARTHEST {
            return None;
        }
    }
    Some(None)
}

/// Place a sequential container whose child list changed (see
/// [`ChildListEdit`]) by the edit, and return its plan around it.
///
/// The children before the edit keep their positions; the ones that arrived
/// are measured and placed; the ones after it move by what left and arrived,
/// the replay a child that changed size gets ([`replay_sequential_run`]). At
/// the end of a long list nothing moves and the edit is all the work; at its
/// head every child after it moves once, which is the output, and none is
/// measured again. `changed_from` is the first entry a child the change
/// closure reaches moved, from [`check_plan_children`].
///
/// Returns `None` for an edit the replay does not express -- a child that
/// leaves or arrives positioned or inline, with an auto margin, out of
/// document order, or reading the main extent -- and the caller lays the
/// container out again.
#[allow(clippy::too_many_arguments)]
fn replay_child_list_edit(
    plan: &ContainerPlan,
    children: &Arc<Vec<StableNodeId>>,
    edit: ChildListEdit,
    changed_from: usize,
    content: Size,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    descend: &mut Vec<Descend>,
    scope: &ScopeContext<'_>,
) -> Result<Option<ContainerPlan>, UiWorldError> {
    // A line that solves its overflow shares out what it takes from its
    // items: an edit can change that, which a replay does not solve.
    if plan
        .style
        .adaptation
        .as_deref()
        .is_some_and(super::dynamic::solves)
    {
        return Ok(None);
    }
    let direction = plan.main_direction;
    let block = !plan
        .style
        .display
        .is_some_and(|display| display.is_flex_container() || display.is_grid_container());
    let old = plan.children.as_slice();
    // A child that left in flow had an entry; an omitted one had no slot,
    // and a positioned one is the overlay's, which this does not rebuild.
    let mut left = Vec::new();
    for &child in edit.left(old) {
        match plan.entry_index(child) {
            Some(index) => left.push(index as usize),
            None if plan.overlay.iter().any(|entry| entry.child == child) => return Ok(None),
            None => {}
        }
    }
    left.sort_unstable();
    let (Some(before), Some(after)) = (
        nearest_entry(plan, old[..edit.start].iter().rev().copied()),
        nearest_entry(plan, old[edit.old_end..].iter().copied()),
    ) else {
        return Ok(None);
    };
    // Where the edit goes among the entries: right after the child in flow
    // before it.
    let at = before.map_or(0, |index| index + 1);
    let mut arrivals = Vec::new();
    let mut reads_cross = false;
    for &child in edit.arrivals(children) {
        let Some(style) = nodes.style_ref(child) else {
            continue;
        };
        if style.omits_box() {
            continue;
        }
        if crate::layout_node::leaves_parent_flow(Some(&plan.style), &style)
            || (block && style.is_inline_level())
            || style.order != 0
            || style.margin_auto_left()
            || style.margin_auto_right()
            || style.margin_auto_top()
            || style.margin_auto_bottom()
            || reads_main_extent(&style, direction)
            // Something under it reads the block extent it is offered, which
            // in a column is the main extent the replay keeps.
            || (direction == FlexDirection::Column && nodes.world.block_extent_reaches(child))
        {
            return Ok(None);
        }
        reads_cross |= reads_container_size(&style, direction);
        arrivals.push(child);
    }
    let (from, cursor, run, prefix) = {
        let entries = &plan.entries;
        // Entries run in placement order. A stable sort by `order` keeps the
        // children that set none in document order, next to each other, so
        // around an edit among them the children that left are the entries
        // between the child in flow before it and the one after it.
        let ordered = |index: usize| entries[index].style.order == 0;
        if left
            .iter()
            .enumerate()
            .any(|(offset, &index)| index != at + offset || !ordered(index))
            || before.is_some_and(|index| !ordered(index))
            || after.map_or(at + left.len() != entries.len(), |index| {
                index != at + left.len() || !ordered(index)
            })
        {
            return Ok(None);
        }
        let from = changed_from.min(at);
        let cursor = match entries.get(from) {
            Some(entry) => entry.cursor_before,
            // Arrivals after the last child: the cursor that child left.
            None => entries.last().map_or(0.0, |last| {
                let margin = last.style.resolved_margin_against_fonts(
                    Some(
                        plan.writing
                            .inline_size(plan.content.width, plan.content.height),
                    ),
                    fonts_of(&last.style, plan.child_font_px),
                );
                let (lead, trail) = main_margins(margin, direction, plan.main_reversed);
                let mut cursor = last.cursor_before;
                cursor += main_extent(last.size, direction) + lead + trail + plan.gap;
                cursor
            }),
        };
        let run: Vec<RunChild> = entries[from..at]
            .iter()
            .map(RunChild::planned)
            .chain(arrivals.iter().map(|&child| RunChild {
                child,
                planned: None,
            }))
            .chain(entries[at + left.len()..].iter().map(RunChild::planned))
            .collect();
        (from, cursor, run, entries[..from].to_vec())
    };
    // An omitted arrival gets no slot; a box it kept from before collapses.
    retire_omitted_children(edit.arrivals(children), nodes, output, scope);
    let Some(replayed) = replay_sequential_run(
        plan, cursor, run, content, viewport, nodes, intrinsic, descend, scope,
    )?
    else {
        return Ok(None);
    };
    let shifted = shift_reversed_prefix(plan, from, content, descend);
    let mut entries = prefix;
    for (index, origin) in shifted {
        entries[index].origin = origin;
    }
    entries.extend(replayed);
    // The children before the edit keep their index, the ones after it move
    // by the arrivals less the children that left, and the arrivals take
    // the indices in between. Still sorted by child but for the arrivals,
    // which the stable sort merges in as one short run.
    let by_child = if entries.len() <= super::BY_CHILD_LINEAR_CAP {
        None
    } else if let Some(prev_by_child) = plan.by_child.as_deref() {
        let mut by_child: Vec<(StableNodeId, u32)> = prev_by_child
            .iter()
            .filter_map(|&(child, index)| {
                let index = index as usize;
                if index < at {
                    Some((child, index as u32))
                } else if index < at + left.len() {
                    None
                } else {
                    Some((child, (index - left.len() + arrivals.len()) as u32))
                }
            })
            .collect();
        by_child.extend(
            arrivals
                .iter()
                .enumerate()
                .map(|(offset, &child)| (child, (at + offset) as u32)),
        );
        by_child.sort_by_key(|(child, _)| *child);
        Some(by_child.into_boxed_slice())
    } else {
        super::build_by_child_index(&entries)
    };
    Ok(Some(ContainerPlan {
        origin: plan.origin,
        size: plan.size,
        containing: plan.containing,
        parent_font_px: plan.parent_font_px,
        viewport: plan.viewport,
        style: Arc::clone(&plan.style),
        children: Arc::clone(children),
        content: plan.content,
        child_font_px: plan.child_font_px,
        child_available: plan.child_available,
        main_direction: direction,
        writing: plan.writing,
        main_reversed: plan.main_reversed,
        cross_reversed: plan.cross_reversed,
        content_origin: plan.content_origin,
        gap: plan.gap,
        sequential: true,
        entries,
        placed_main: main_extent(content, direction),
        by_child,
        grid: None,
        cross_independent: plan.cross_independent && !reads_cross,
        main_dependent: plan.main_dependent,
        overlay: plan.overlay.clone(),
    }))
}

/// Shift later wrap lines when one item's cross size changed and the line
/// breaks did not, and return the rows that moved. Anything else returns
/// `None` so the caller lays out this flex container from scratch.
fn replay_wrapped_flex_line(
    plan: &ContainerPlan,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    descend: &mut Vec<Descend>,
    scope: &ScopeContext<'_>,
) -> Result<Option<PlanPatch>, UiWorldError> {
    let style = plan.style.as_ref();
    if plan.sequential || !flex_line_local_style(style) || plan.main_reversed || plan.cross_reversed
    {
        return Ok(None);
    }
    let direction = plan.main_direction;
    let tracks = match direction {
        FlexDirection::Row => style.active_grid_columns(),
        FlexDirection::Column => style.active_grid_rows(),
    };
    if tracks.is_some_and(|tracks| !tracks.is_empty()) {
        return Ok(None);
    }
    let entries = &plan.entries;
    let mut flow = Vec::with_capacity(entries.len());
    let mut old_sizes = Vec::with_capacity(entries.len());
    for entry in entries.iter() {
        if child_blocks_flex_line_local(entry.style.as_ref()) {
            return Ok(None);
        }
        if scope.affected.contains(&entry.child)
            && !nodes.style_ref(entry.child).is_some_and(|current| {
                Arc::ptr_eq(current.arc(), &entry.style)
                    || super::same_but_cross(&current, &entry.style, direction)
            })
        {
            return Ok(None);
        }
        let cross = match direction {
            FlexDirection::Row => entry.style.height,
            FlexDirection::Column => entry.style.width,
        };
        if matches!(cross, Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)) {
            return Ok(None);
        }
        flow.push(entry.child);
        old_sizes.push(entry.intrinsic);
    }
    if flow.is_empty() {
        return Ok(None);
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
            return Ok(None);
        }
        if measured != sizes[index] {
            sizes[index] = measured;
            patched = true;
        }
    }
    if !patched {
        return Ok(None);
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
        return Ok(None);
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
            let mut origin = plan.entries[index].origin;
            match direction {
                FlexDirection::Row => origin.y += carried,
                FlexDirection::Column => origin.x += carried,
            }
            jobs.push((flow[index], origin, sizes[index], index));
        }
        carried += delta;
    }
    descend.extend(jobs.iter().map(|&(child, origin, size, _)| Descend {
        child,
        origin,
        size,
        containing: plan.content,
        inherited: None,
    }));
    let mut rows: Vec<(u32, PlannedChild)> = jobs
        .into_iter()
        .map(|(child, origin, size, index)| {
            let mut entry = plan.entries[index].clone();
            entry.child = child;
            entry.origin = origin;
            entry.size = size;
            entry.intrinsic = size;
            if let Some(child_style) = nodes.style(child) {
                entry.style = child_style;
            }
            (index as u32, entry)
        })
        .collect();
    rows.sort_unstable_by_key(|(index, _)| *index);
    Ok(Some(PlanPatch {
        rows,
        placed_main: None,
    }))
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
    descend: &mut Vec<Descend>,
    scope: &ScopeContext<'_>,
) -> Result<bool, UiWorldError> {
    let Some(old_grid) = plan.grid.clone() else {
        return Ok(false);
    };
    let style = plan.style.as_ref();
    if style.is_subgrid_columns() || style.is_subgrid_rows() {
        return Ok(false);
    }
    let entries = plan.entries.clone();
    let entry_of = |child: StableNodeId| {
        plan.entry_index(child)
            .and_then(|index| entries.get(index as usize))
            .filter(|entry| entry.child == child)
    };
    let mut flow = Vec::new();
    let mut sizes = Vec::new();
    let content_changed = plan.content != content;
    for child in plan.children.iter().copied() {
        let Some(child_style) = nodes.style_ref(child) else {
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
    let mut solved = layout_grid_2d(
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
    solved.fill_layer_cell(style, plan.writing, content);
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
        let current = nodes.style_ref(item.id);
        if !retained_style_matches(current.as_ref(), Some(&entry.style))
            || entry.style.aspect_ratio.is_some()
        {
            continue;
        }
        reuse.insert(item.id, entry.size);
    }
    let mut recorded = Vec::with_capacity(solved.items.len());
    let pushed = descend.len();
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
        Some(scope),
        Some(&reuse),
        Some(&mut recorded),
        descend,
    )?;
    if recorded.len() != solved.items.len() {
        descend.truncate(pushed);
        return Ok(false);
    }
    let mut updated = plan.clone();
    updated.origin = origin;
    updated.size = size;
    updated.content = content;
    updated.content_origin = content_origin;
    updated.grid = Some(Box::new(GridTrackPlan::from_layout(&solved)));
    updated.by_child = super::build_by_child_index(&recorded);
    updated.entries = recorded;
    nodes.container_plans.rebuild(id, updated);
    Ok(true)
}

/// Places `id` and its subtree: one level of the placement recursion.
///
/// A level [`super::recursion::SPAN`] below the outermost writes its own box
/// and hands the rest of its subtree to the pass's queue as a boundary; the
/// outermost call places what it queued before it returns. So the recursion
/// never runs deeper than the span, whatever the tree, and a caller finds the
/// whole subtree placed when this returns (see `recursion`).
#[allow(clippy::too_many_arguments)]
pub(super) fn place_node_scoped(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    inherited: Option<Box<super::grid::InheritedGridTracks>>,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    if intrinsic.placement_levels.depth() == 0 {
        return place_subtree(
            id,
            origin,
            size,
            containing,
            viewport,
            parent_font_px,
            inherited,
            nodes,
            intrinsic,
            output,
            scope,
        );
    }
    place_level(
        id,
        origin,
        size,
        containing,
        viewport,
        parent_font_px,
        inherited,
        nodes,
        intrinsic,
        output,
        scope,
    )
}

/// The outermost placement call: `id`'s levels, then the queue they left.
/// Its own function, so the levels below do not keep its arguments for the
/// queue.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn place_subtree(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    inherited: Option<Box<super::grid::InheritedGridTracks>>,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    place_level(
        id,
        origin,
        size,
        containing,
        viewport,
        parent_font_px,
        inherited,
        nodes,
        intrinsic,
        output,
        scope,
    )?;
    place_boundaries(viewport, nodes, intrinsic, output, scope)
}

/// One level below the outermost: its head, then its body or, at the span,
/// a boundary.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn place_level(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    inherited: Option<Box<super::grid::InheritedGridTracks>>,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    let depth = intrinsic.placement_levels.depth();
    super::recursion::level(
        intrinsic,
        super::recursion::Recursion::Placement,
        |intrinsic| {
            // The head's frame is gone before the body runs: a level holds
            // one of the two, not both.
            let Some(head) = place_node_head(
                id,
                origin,
                size,
                containing,
                parent_font_px,
                nodes,
                intrinsic,
                output,
                scope,
            )?
            else {
                return Ok(());
            };
            if inherited.is_none()
                && !head.facts.style.is_subgrid_columns()
                && !head.facts.style.is_subgrid_rows()
                && !intrinsic.slice.pending.contains_key(&id)
                && keeps_moved_subtree(
                    id,
                    &head.facts,
                    size,
                    containing,
                    parent_font_px,
                    nodes,
                    scope,
                )
            {
                intrinsic.note_moved_subtree_kept();
                #[cfg(any(test, feature = "layout-verify"))]
                super::verify::note_kept_subtree(id);
                return Ok(());
            }
            // A leaf has no subtree to wait: placing it here descends no
            // further.
            let leaf = head.facts.children.is_empty() && head.facts.modal.is_none();
            // A boundary an earlier pass deferred, reached again: placed
            // from this pass's inputs, and through the queue, which may defer
            // it again.
            let resumed =
                intrinsic
                    .slice
                    .reach(id, size, containing, parent_font_px, inherited.as_deref());
            // Under the span, a scroll container or an isolated box is cut
            // too: a unit of the pass's queue whatever its depth.
            let natural = !leaf
                && depth > 0
                && depth < super::recursion::SPAN
                && natural_boundary(id, head.facts.style.as_ref(), nodes);
            if natural {
                intrinsic.execution_stats.placement_natural_boundaries += 1;
            }
            // A sliced pass also cuts a box wholly out of view, so what is out
            // of view can wait however shallow it lies.
            let out_of_view = if !leaf && depth > 0 && intrinsic.slice.active() {
                distance_out_of_view(id, size, nodes, intrinsic, output, scope)
            } else {
                None
            };
            if (depth >= super::recursion::SPAN || natural || resumed || out_of_view.is_some())
                && !leaf
            {
                queue_boundary(
                    super::recursion::Boundary {
                        id,
                        size,
                        containing,
                        parent_font_px,
                        inherited,
                        distance: out_of_view.unwrap_or(0.0),
                    },
                    out_of_view.is_some(),
                    nodes,
                    intrinsic,
                    output,
                    scope,
                );
                return Ok(());
            }
            place_node_body(
                id,
                Some(head.facts),
                size,
                containing,
                viewport,
                parent_font_px,
                inherited.as_deref(),
                nodes,
                intrinsic,
                output,
                scope,
            )
        },
    )
}

/// Whether `id` is where a subtree is cut whatever its depth: a scroll
/// container, whose content scrolls inside a box its parent sized, or a box
/// that isolates its layout (`UiWorld::layout_isolated`: fixed in both
/// extents and in flow). Both are where a later pass can take the subtree as
/// one unit, scheduled apart from what is around it (see `recursion`).
#[inline(never)]
fn natural_boundary(id: StableNodeId, style: &LayoutStyle, nodes: &LayoutInputMap<'_>) -> bool {
    style.overflow_x.scrolls()
        || style.overflow_y.scrolls()
        || (style.layout_isolation && nodes.world.layout_isolated(id))
        || nodes.world.is_scroll_container(id)
}

/// Whether `id`, whose own box the head just wrote, keeps everything under
/// it: a box that only moved.
///
/// Its children are measured from its own origin (see `crate::LayoutFrame`),
/// so its body lays them out exactly as last pass whenever what the body
/// reads is unchanged: its border box's size and used padding, its style and
/// child list (it is no seed and is not measured again, and nothing under it
/// is affected), no line its parent solves for it (Dynamic Layout), no grid
/// tracks it inherits. Then every table the body would write (boxes, padding,
/// contexts, plans) already holds that answer, and the retained cache keeps
/// what a pass does not write. Without this a moved container walks its
/// children again: an inline formatting context packs its line boxes again,
/// a few microseconds a paragraph, for every paragraph below an edit.
///
/// The body also places what hangs from a frame other than this box's: an
/// out-of-flow child whose static position reads where this box stands on
/// the page or in an ancestor, the items of a triggered menu, a modal's
/// slots and the absolute children the remote-overlay sweep reaches. Any of
/// those keeps the body.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn keeps_moved_subtree(
    id: StableNodeId,
    facts: &NodeFacts<'_>,
    size: Size,
    containing: Size,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
    scope: Option<&ScopeContext<'_>>,
) -> bool {
    let Some(scope) = scope else {
        return false;
    };
    if facts.children.is_empty() || facts.modal.is_some() {
        return false;
    }
    // Affected only to be placed again: reached by an edge from its parent
    // or from a sibling in the parent's formatting context, which moves it
    // or resizes it (checked below) but changes nothing it establishes. Not
    // a seed of its own, nor measured, nor in a writing mode or scroll that
    // changed; nothing under it is affected.
    let frontier = scope.frontier;
    if scope.affected.contains(&id)
        && (frontier.is_seed(id)
            || frontier.writing_nodes().contains(&id)
            || frontier.scroll_nodes().contains(&id))
    {
        return false;
    }
    if scope.measure.contains(&id)
        || !scope.reach.children(id).is_empty()
        || !scope.affected_children(id).is_empty()
    {
        return false;
    }
    let Some(previous) = scope.retained.boxes.get(&id) else {
        return false;
    };
    if previous.width.to_bits() != size.width.to_bits()
        || previous.height.to_bits() != size.height.to_bits()
    {
        return false;
    }
    let style = facts.style.as_ref();
    if style.adaptation.is_some() {
        return false;
    }
    let fonts = fonts_of(style, parent_font_px);
    let padding = style.resolved_padding_against_fonts(
        Some(
            facts
                .containing_writing
                .inline_size(containing.width, containing.height),
        ),
        fonts,
    );
    if scope.retained.used_padding_of(id) != Some(padding) {
        return false;
    }
    if scope.retained.remote_overlays.contains(&id)
        || triggered_menu_overlay(nodes.world, id).is_some()
    {
        return false;
    }
    // A child placed in another frame than this box's (an absolute box hung
    // from an ancestor or the page, a fixed one) is placed by this body.
    !facts
        .children
        .iter()
        .any(|child| scope.retained.frames.contains_key(child))
}

/// Queues a subtree for the outermost call: taken once per span, so out of
/// the level's frame. A sliced pass ranks it by how far it lies from view.
#[cold]
#[inline(never)]
fn queue_boundary(
    mut boundary: super::recursion::Boundary,
    ranked_already: bool,
    nodes: &LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &PassOutput,
    scope: Option<&ScopeContext<'_>>,
) {
    let ranked = intrinsic.slice.active();
    if ranked && !ranked_already {
        boundary.distance = distance_from_view(
            boundary.id,
            boundary.size,
            intrinsic.slice.viewport,
            nodes,
            output,
            scope,
        );
    }
    intrinsic.boundaries.push(boundary, ranked);
}

/// In a sliced pass, how far `id`'s box lies out of view, when it does.
#[inline(never)]
fn distance_out_of_view(
    id: StableNodeId,
    size: Size,
    nodes: &LayoutInputMap<'_>,
    intrinsic: &PassIntrinsicCache,
    output: &PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Option<f32> {
    let distance = distance_from_view(id, size, intrinsic.slice.viewport, nodes, output, scope);
    (distance > 0.0).then_some(distance)
}

/// How far `id`'s box, of `size`, lies outside the viewport, in px: on the
/// page, less what the scroll containers along its frame chain scrolled. `0` in view.
fn distance_from_view(
    id: StableNodeId,
    size: Size,
    viewport: Size,
    nodes: &LayoutInputMap<'_>,
    output: &PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> f32 {
    let page = origin_in(None, id, nodes, output, scope);
    let (mut x, mut y) = (page.x, page.y);
    let world = nodes.world;
    let mut cursor = Some(id);
    while let Some(node) = cursor {
        let frame_parent = match pass_frame(node, nodes, scope) {
            crate::LayoutFrame::Parent => world
                .parent_id(node)
                .filter(|parent| !world.is_document_node(*parent)),
            crate::LayoutFrame::Block(block) => Some(block),
            crate::LayoutFrame::Page => None,
        };
        if let Some(container) = frame_parent
            && let Some(offset) = world.scroll_offset(container)
        {
            x -= offset.x;
            y -= offset.y;
        }
        cursor = frame_parent;
    }
    // How far one axis's span lies past the view's, in px; negative while
    // they overlap. A positive-extent span that only touches the view's edge
    // is outside; a zero-extent span on `[0, view]` is in view.
    let past = |start: f32, extent: f32, view: f32| {
        if extent == 0.0 {
            if start < 0.0 {
                -start
            } else if start > view {
                start - view
            } else {
                -1.0
            }
        } else {
            (start - view).max(-(start + extent))
        }
    };
    let past = past(x, size.width, viewport.width).max(past(y, size.height, viewport.height));
    if past < 0.0 { 0.0 } else { past + 1.0 }
}

/// Whether no box under `id` is retained: a subtree a pass may leave whole
/// for a later one, since no later pass can find a stale plan or box in it.
fn holds_no_retained_placement(
    id: StableNodeId,
    nodes: &LayoutInputMap<'_>,
    scope: Option<&ScopeContext<'_>>,
) -> bool {
    let Some(scope) = scope else {
        // A full pass: nothing is retained.
        return true;
    };
    let world = nodes.world;
    let mut stack: Vec<StableNodeId> = world.child_ids(id).to_vec();
    while let Some(node) = stack.pop() {
        if scope.retained.boxes.contains_key(&node) {
            return false;
        }
        stack.extend_from_slice(world.child_ids(node));
    }
    true
}

/// Whether a sliced pass leaves `boundary` for a later one: out of view,
/// with the budget spent, after at least one out-of-view boundary this pass
/// (so a sliced layout always finishes), and with nothing retained under it.
fn defers(
    boundary: &super::recursion::Boundary,
    nodes: &LayoutInputMap<'_>,
    intrinsic: &PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> bool {
    let slice = &intrinsic.slice;
    slice.active()
        && boundary.distance > 0.0
        && slice.placed_offscreen > 0
        && slice.spent(intrinsic.execution_stats.placement_nodes)
        && holds_no_retained_placement(boundary.id, nodes, scope)
}

/// Places every boundary earlier passes deferred that this pass did not
/// reach, from the inputs recorded for it: nothing above it moved it, so they
/// still hold. One under another still waiting is reached when that one is
/// placed, and waits with it otherwise. One whose root left the document or
/// generates no box is dropped.
pub(super) fn resume_deferred(
    document: DocumentId,
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    if intrinsic.slice.pending.is_empty() {
        return Ok(());
    }
    let world = nodes.world;
    let mut roots: Vec<StableNodeId> = intrinsic.slice.pending.keys().copied().collect();
    roots.sort_unstable();
    let pending_roots: crate::NodeSet = roots.iter().copied().collect();
    let deferred: HashSet<StableNodeId> =
        intrinsic.slice.deferred.iter().map(|(id, _)| *id).collect();
    for id in roots {
        let mut gone = !world.contains(id) || world.document_of(id) != Some(document);
        let mut waits = false;
        let mut cursor = (!gone).then_some(id);
        while let Some(node) = cursor {
            if nodes.style_ref(node).is_some_and(|style| style.omits_box()) {
                gone = true;
                break;
            }
            if node != id && (pending_roots.contains(&node) || deferred.contains(&node)) {
                waits = true;
            }
            cursor = world.parent_id(node);
        }
        if gone {
            intrinsic.slice.pending.remove(&id);
            intrinsic.slice.cancelled += 1;
            continue;
        }
        if waits {
            continue;
        }
        let Some(record) = intrinsic.slice.pending.remove(&id) else {
            continue;
        };
        queue_boundary(
            super::recursion::Boundary {
                id,
                size: record.size,
                containing: record.containing,
                parent_font_px: record.parent_font_px,
                inherited: record.inherited,
                distance: 0.0,
            },
            false,
            nodes,
            intrinsic,
            output,
            scope,
        );
    }
    place_boundaries(viewport, nodes, intrinsic, output, scope)
}

/// Places every subtree the pass queued, in the order they were queued (a
/// sliced pass: nearest the viewport first, deferring what its budget leaves
/// out); one placed here can queue more.
fn place_boundaries(
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    while let Some(boundary) = intrinsic.boundaries.pop() {
        if defers(&boundary, nodes, intrinsic, scope) {
            let deferred = super::recursion::DeferredBoundary::of(boundary);
            intrinsic.slice.deferred.push(deferred);
            continue;
        }
        if boundary.distance > 0.0 {
            intrinsic.slice.placed_offscreen += 1;
        }
        intrinsic.execution_stats.placement_boundaries += 1;
        super::recursion::level(
            intrinsic,
            super::recursion::Recursion::Placement,
            |intrinsic| {
                place_node_body(
                    boundary.id,
                    None,
                    boundary.size,
                    boundary.containing,
                    viewport,
                    boundary.parent_font_px,
                    boundary.inherited.as_deref(),
                    nodes,
                    intrinsic,
                    output,
                    scope,
                )
            },
        )?;
    }
    Ok(())
}

/// A level's head once it has written its node's box: what the body needs.
struct PlacedHead<'a> {
    facts: NodeFacts<'a>,
}

/// The first half of a level: write `id`'s own box. `None` when that is all
/// there is to do (no node, or one that generates no box).
#[allow(clippy::too_many_arguments)]
fn place_node_head<'a>(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'a>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Option<PlacedHead<'a>>, UiWorldError> {
    let Some(node) = nodes.get(id)? else {
        nodes.omitted_boxes.insert(id);
        output.insert(
            id,
            LayoutBox {
                x: origin.x,
                y: origin.y,
                width: 0.0,
                height: 0.0,
            },
        );
        return Ok(None);
    };
    let facts = NodeFacts {
        style: node.style.shared(),
        children: node.children,
        writing: node.writing,
        containing_writing: node.containing_writing,
        modal: nodes.world.modal_layout_input(id).map(Box::new),
    };
    let style_arc = facts.style.shared();
    // Explicit boundaries and native atomic entries need a saved placement for independent reflow.
    // Ordinary nodes must not allocate another per-node cache on full layout.
    if style_arc.layout_isolation || nodes.world.inline_parent(id).is_some() {
        nodes
            .placements
            .insert(id, (origin, containing, parent_font_px));
    }
    let style = style_arc.as_ref();
    // A box in flow, and one that generates none, is measured from its
    // parent's origin, whatever frame it had out of flow before; a
    // positioned box's frame is the one `place_positioned_child` gives it.
    if (!style.position.is_out_of_flow() || style.omits_box())
        && scope.is_some_and(|scope| scope.retained.frames.contains_key(&id))
    {
        nodes.frames.insert(id, crate::LayoutFrame::Parent);
    }
    if style.omits_box() {
        nodes.omitted_boxes.insert(id);
        output.insert(
            id,
            LayoutBox {
                x: origin.x,
                y: origin.y,
                width: 0.0,
                height: 0.0,
            },
        );
        return Ok(None);
    }
    let fonts = fonts_of(style, parent_font_px);
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
    Ok(Some(PlacedHead { facts }))
}

/// What a level reads off its node, read once for both of its halves.
struct NodeFacts<'a> {
    style: StyleRef<'a>,
    children: &'a Arc<Vec<StableNodeId>>,
    /// The writing mode and direction this node lays out in, inherited from
    /// its ancestors when it declares none of its own, and its parent's.
    writing: nana_ui_core::WritingContext,
    containing_writing: nana_ui_core::WritingContext,
    modal: Option<Box<crate::ModalLayoutInput>>,
}

/// Everything a level does once its own box is written: lay out and place
/// its children. A boundary resumes here, without `facts`.
#[allow(clippy::too_many_arguments)]
fn place_node_body<'a>(
    id: StableNodeId,
    facts: Option<NodeFacts<'a>>,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    inherited: Option<&super::grid::InheritedGridTracks>,
    nodes: &mut LayoutInputMap<'a>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    let facts = match facts {
        Some(facts) => facts,
        None => {
            let Some(node) = nodes.get(id)? else {
                return Ok(());
            };
            NodeFacts {
                style: node.style.shared(),
                children: node.children,
                writing: node.writing,
                containing_writing: node.containing_writing,
                modal: nodes.world.modal_layout_input(id).map(Box::new),
            }
        }
    };
    let NodeFacts {
        style: style_arc,
        children: child_ids,
        writing,
        containing_writing,
        modal,
    } = facts;
    let style = style_arc.as_ref();
    let fonts = fonts_of(style, parent_font_px);
    let child_font_px = fonts.element_px;
    // The children are placed from this box's own origin: their boxes are
    // measured from it (see `crate::LayoutFrame`), so where it stands moves
    // none of them.
    let origin = Point::ZERO;

    let containing_inline = containing_writing.inline_size(containing.width, containing.height);
    let padding = style.resolved_padding_against_fonts(Some(containing_inline), fonts);
    // A box its line shrank closes its own padding up first (Issue #212).
    let padding = intrinsic.dynamic.resolve_padding(
        id,
        padding,
        containing_inline,
        parent_font_px,
        nodes,
        scope.map(|scope| scope.retained),
    );
    if scope.is_some_and(|scope| scope.retained.omitted_boxes.contains(&id)) {
        nodes.unomitted_boxes.insert(id);
    }
    if padding != nana_ui_core::PaddingSpec::default() {
        nodes.used_padding.insert(id, padding);
    } else if scope.is_some_and(|scope| scope.retained.used_padding.contains_key(&id)) {
        nodes.cleared_padding.insert(id);
    }

    if let Some(modal) = modal.as_ref() {
        let context = establish_context(id, style, child_ids, nodes);
        debug_assert_eq!(context.kind, crate::FormattingContextKind::Overlay);
        nodes.contexts.insert(id, context.kind);
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
    // The children start past the parts laid out before them (#385), inside
    // the padding.
    let (lead, lead_offset) = nodes
        .world
        .leading_parts(id)
        .map_or((Size::default(), Size::default()), |lead| {
            (lead.inset(), lead.offset())
        });
    let content_origin = Point {
        x: origin.x + border.left + padding.left + lead_offset.width,
        y: origin.y + border.top + padding.top + lead_offset.height,
    };
    let content = Size::new(
        size.width - padding.left - padding.right - border.left - border.right - lead.width,
        size.height - padding.top - padding.bottom - border.top - border.bottom - lead.height,
    );
    if nodes.world.inline_record(id).is_some() {
        nodes
            .contexts
            .insert(id, crate::FormattingContextKind::Inline);
        nodes.container_plans.retire(id);
        nodes.measure_plans.insert(id, MeasurePlanSlots::default());
        return place_native_inline(
            id,
            child_ids,
            content_origin,
            content,
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        );
    }
    let had_plan = scope.is_some_and(|scope| scope.retained.container_plans.contains_key(&id));
    let subgrid = style.is_subgrid_columns() || style.is_subgrid_rows();
    if !subgrid && let Some(scope) = scope {
        // A container whose plan answers gives its children their boxes from
        // it; the subtrees under them are entered here, once the replay's
        // frame is gone, like the walk's below.
        let replayed = replay_plan(
            id,
            origin,
            size,
            containing,
            viewport,
            parent_font_px,
            style_arc.arc(),
            child_ids,
            writing,
            content_origin,
            content,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
        if let Some(replay) = replayed {
            let suffix = replay.suffix;
            place_descendants(
                replay.descend,
                viewport,
                child_font_px,
                nodes,
                intrinsic,
                output,
                Some(scope),
            )?;
            return finish_plan_replay(
                id,
                origin,
                size,
                content_origin,
                content,
                viewport,
                child_font_px,
                writing,
                suffix,
                nodes,
                intrinsic,
                output,
                scope,
            );
        }
    }

    let ChildrenLayout {
        descend,
        positioned,
        plan_entries,
        gap,
        plan_sequential,
        cross_independent,
        main_dependent,
        direction,
        main_reversed,
        cross_reversed,
        recorded_grid,
        #[cfg(feature = "benchmark")]
        mut child_phase,
    } = lay_out_children(
        id,
        style_arc.arc(),
        child_ids,
        content_origin,
        content,
        writing,
        fonts,
        child_font_px,
        viewport,
        had_plan,
        inherited,
        nodes,
        intrinsic,
        output,
        scope,
    )?;
    place_descendants(
        descend,
        viewport,
        child_font_px,
        nodes,
        intrinsic,
        output,
        scope,
    )?;
    #[cfg(feature = "benchmark")]
    if let Some(clock) = child_phase.as_mut() {
        clock.lap(7);
    }
    if let Some(entries) = plan_entries {
        if had_plan {
            intrinsic.note_plan_rebuilt();
        }
        nodes.container_plans.rebuild(
            id,
            ContainerPlan {
                content_origin,
                gap,
                sequential: plan_sequential,
                by_child: super::build_by_child_index(&entries),
                origin,
                size,
                containing,
                parent_font_px,
                viewport,
                style: style_arc.to_arc(),
                children: Arc::clone(child_ids),
                content,
                child_font_px,
                child_available: content,
                main_direction: direction,
                writing,
                main_reversed,
                cross_reversed,
                entries,
                placed_main: main_extent(content, direction),
                grid: recorded_grid,
                cross_independent,
                main_dependent,
                overlay: Vec::new(),
            },
        );
    }
    if let Some(overlay) = triggered_menu_overlay(nodes.world, id) {
        // The items hang off the viewport: they are placed from where the
        // menu stands on the page.
        let page = origin_in(None, id, nodes, output, scope);
        place_triggered_menu_items(
            id,
            overlay,
            LayoutBox {
                x: page.x,
                y: page.y,
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
    if let Some(plan) = nodes.container_plans.rebuilt_mut(&id) {
        plan.overlay = recorded_overlay;
    }
    Ok(())
}

/// A container its plan answered for: its children have their boxes, and
/// the subtrees under them wait in `descend` to be entered once the replay's
/// frame is gone (see `recursion`). Then [`finish_plan_replay`] places its
/// positioned children.
struct PlanReplay {
    descend: Vec<Descend>,
    /// A sequential replay that kept a prefix of the children.
    suffix: bool,
}

/// The pass's scratch list for a replay, and the replay's answer: the list
/// back to the scratch when the plan could not answer.
fn plan_replay(
    descend: Vec<Descend>,
    answered: Option<bool>,
    nodes: &mut LayoutInputMap<'_>,
) -> Option<PlanReplay> {
    match answered {
        Some(suffix) => Some(PlanReplay { descend, suffix }),
        None => {
            nodes.scratch.give_descend(descend);
            None
        }
    }
}

/// What a container its plan answered for does once the subtrees under its
/// children are placed: its positioned children, which can read where those
/// stand, as after a walk.
#[allow(clippy::too_many_arguments)]
// Its own frame, taken only on the path that calls it (see `recursion`).
#[inline(never)]
fn finish_plan_replay(
    id: StableNodeId,
    origin: Point,
    size: Size,
    content_origin: Point,
    content: Size,
    viewport: LayoutViewport,
    child_font_px: f32,
    writing: nana_ui_core::WritingContext,
    suffix: bool,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: &ScopeContext<'_>,
) -> Result<(), UiWorldError> {
    // The plan the replay read: an edited child list rebuilt its own into
    // the pass already, which the overlay writes its geometry into.
    if let Some(plan) = scope.retained.container_plans.get(&id) {
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
    }
    intrinsic.note_placement_plan_reused();
    if suffix {
        intrinsic.note_suffix_replayed();
    }
    Ok(())
}

/// A container's children given their boxes from its plan, when the plan
/// answers: by replaying it from a child-list edit, or else by reusing it
/// with only what changed under it placed again.
#[allow(clippy::too_many_arguments)]
// Its own frame, taken only on the path that calls it (see `recursion`).
#[inline(never)]
fn replay_plan(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    style_arc: &Arc<LayoutStyle>,
    child_ids: &Arc<Vec<StableNodeId>>,
    writing: nana_ui_core::WritingContext,
    content_origin: Point,
    content: Size,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: &ScopeContext<'_>,
) -> Result<Option<PlanReplay>, UiWorldError> {
    if let Some(replay) = place_edited_child_list(
        id,
        origin,
        size,
        containing,
        viewport,
        parent_font_px,
        style_arc,
        child_ids,
        writing,
        content_origin,
        content,
        nodes,
        intrinsic,
        output,
        scope,
    )? {
        return Ok(Some(replay));
    }
    // A container whose own inputs and whose children's styles and
    // intrinsic sizes are all unchanged places its children exactly where
    // it did last pass. Reuse that and touch only the children the change
    // closure reaches; otherwise a one-child edit pays a full sibling scan.
    // See `ContainerPlan`.
    place_from_plan(
        id,
        origin,
        size,
        containing,
        viewport,
        parent_font_px,
        style_arc,
        child_ids,
        writing,
        content_origin,
        content,
        nodes,
        intrinsic,
        output,
        scope,
    )
}

/// A sequential container with a new child list, its children given their
/// boxes by replaying its plan from the edit. `None` when the plan cannot
/// answer.
#[allow(clippy::too_many_arguments)]
// Its own frame, taken only on the path that calls it (see `recursion`).
#[inline(never)]
fn place_edited_child_list(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    style_arc: &Arc<LayoutStyle>,
    child_ids: &Arc<Vec<StableNodeId>>,
    writing: nana_ui_core::WritingContext,
    content_origin: Point,
    content: Size,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: &ScopeContext<'_>,
) -> Result<Option<PlanReplay>, UiWorldError> {
    // A new child list on a sequential container: the children before the
    // edit stay, the ones after it move, and nothing it kept is measured
    // again. See `ChildListEdit`.
    let Some(plan) = scope.retained.container_plans.get(&id) else {
        return Ok(None);
    };
    if !(plan.sequential
        && !Arc::ptr_eq(&plan.children, child_ids)
        && plan.can_reuse_flow(
            origin,
            size,
            containing,
            content_origin,
            parent_font_px,
            viewport,
            style_arc,
            writing,
        )
        && nodes.world.children_layout_style_is_local(id)
        && triggered_menu_overlay(nodes.world, id).is_none())
    {
        return Ok(None);
    }
    let edit = ChildListEdit::between(&plan.children, child_ids);
    let changed_from = if plan.flow_membership_holds(id, Some(&edit), scope, nodes) {
        match check_plan_children(plan, viewport, nodes, intrinsic, scope)? {
            PlanCheck::Stale => None,
            PlanCheck::Unchanged => Some(plan.child_count()),
            PlanCheck::ChangedFrom(index) => Some(index),
        }
    } else {
        None
    };
    let Some(changed_from) = changed_from else {
        return Ok(None);
    };
    let mut descend = nodes.scratch.descend();
    let edited = replay_child_list_edit(
        plan,
        child_ids,
        edit,
        changed_from,
        content,
        viewport,
        nodes,
        intrinsic,
        output,
        &mut descend,
        scope,
    )?;
    let answered = edited.map(|edited| {
        descend_unvisited_reaching(id, &edited, None, &mut descend, nodes, output, scope);
        // In before the overlay, which writes its geometry into it. The edit
        // moved no positioned child, so the old plan's overlay is this one's.
        nodes.container_plans.rebuild(id, edited);
        true
    });
    Ok(plan_replay(descend, answered, nodes))
}

/// A container whose plan still holds for its inputs, its children given
/// their boxes from it, with only what changed under them to be placed
/// again. `None` when the plan cannot answer and the container walks its
/// children.
#[allow(clippy::too_many_arguments)]
// Its own frame, taken only on the path that calls it (see `recursion`).
#[inline(never)]
fn place_from_plan(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    style_arc: &Arc<LayoutStyle>,
    child_ids: &Arc<Vec<StableNodeId>>,
    writing: nana_ui_core::WritingContext,
    content_origin: Point,
    content: Size,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: &ScopeContext<'_>,
) -> Result<Option<PlanReplay>, UiWorldError> {
    let Some(plan) = scope.retained.container_plans.get(&id) else {
        return Ok(None);
    };
    if !(Arc::ptr_eq(&plan.children, child_ids)
        && plan.can_reuse_flow(
            origin,
            size,
            containing,
            content_origin,
            parent_font_px,
            viewport,
            style_arc,
            writing,
        )
        && plan.flow_membership_holds(id, None, scope, nodes)
        && nodes.world.children_layout_style_is_local(id)
        && triggered_menu_overlay(nodes.world, id).is_none())
    {
        return Ok(None);
    }
    let grid_geometry_moved = plan.grid.is_some() && (plan.size != size || plan.content != content);
    // A reversed sequential axis whose main size moved: every cached origin
    // moved with the far edge, even with no child changed.
    let far_edge_moved = plan.sequential
        && plan.main_reversed
        && main_extent(content, plan.main_direction).to_bits() != plan.placed_main.to_bits();
    let check = check_plan_children(plan, viewport, nodes, intrinsic, scope)?;
    let check = match check {
        PlanCheck::Unchanged if far_edge_moved => PlanCheck::ChangedFrom(plan.child_count()),
        check => check,
    };
    let mut descend = nodes.scratch.descend();
    let answered = match check {
        PlanCheck::Unchanged if !grid_geometry_moved => {
            // A fixed intermediate box can stay out of the frontier while a
            // descendant inside it is affected. Re-enter the direct child
            // that contains that descendant; its own plan places only the
            // closure.
            for index in children_reaching_affected(id, plan, scope) {
                let (child, origin, size) = {
                    let entries = &plan.entries;
                    let entry = &entries[index as usize];
                    (entry.child, entry.origin, entry.size)
                };
                descend.push(Descend {
                    child,
                    origin,
                    size,
                    containing: plan.content,
                    inherited: inherited_tracks(plan, child, nodes),
                });
            }
            return Ok(plan_replay(descend, Some(false), nodes));
        }
        // Something the closure reaches resized or restyled. Children before
        // it cannot have moved, so a sequential container replays only from
        // there; a tail edit shifts nothing and costs O(1).
        PlanCheck::ChangedFrom(from) if plan.sequential => replay_sequential_suffix(
            plan,
            from,
            content,
            viewport,
            nodes,
            intrinsic,
            &mut descend,
            scope,
        )?
        .map(|patch| (true, Some(patch))),
        PlanCheck::Stale => None,
        PlanCheck::Unchanged | PlanCheck::ChangedFrom(_) => {
            let wrapped = if grid_geometry_moved {
                None
            } else {
                replay_wrapped_flex_line(plan, viewport, nodes, intrinsic, &mut descend, scope)?
            };
            if let Some(patch) = wrapped {
                Some((false, Some(patch)))
            } else {
                replay_grid(
                    id,
                    plan,
                    origin,
                    size,
                    content_origin,
                    content,
                    viewport,
                    nodes,
                    intrinsic,
                    &mut descend,
                    scope,
                )?
                .then_some((false, None))
            }
        }
    };
    let Some((suffix, patch)) = answered else {
        return Ok(plan_replay(descend, None, nodes));
    };
    descend_unvisited_reaching(id, plan, patch.as_ref(), &mut descend, nodes, output, scope);
    if let Some(patch) = patch {
        nodes.container_plans.patch(id, patch);
    }
    Ok(plan_replay(descend, Some(suffix), nodes))
}

/// A child subtree a container's walk gave a box, placed once the walk's frame
/// is gone (see `lay_out_children`).
pub(super) struct Descend {
    pub(super) child: StableNodeId,
    pub(super) origin: Point,
    pub(super) size: Size,
    /// Its containing block: the container's content box, or its grid cell.
    pub(super) containing: Size,
    pub(super) inherited: Option<Box<super::grid::InheritedGridTracks>>,
}

/// Places each of `children` in order, and hands the emptied list back to
/// the pass's scratch.
fn place_descendants(
    mut children: Vec<Descend>,
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    for child in children.drain(..) {
        place_node_scoped(
            child.child,
            child.origin,
            child.size,
            child.containing,
            viewport,
            child_font_px,
            child.inherited,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
    }
    nodes.scratch.give_descend(children);
    Ok(())
}

/// What a container's walk decided for its children, before any in-flow
/// child is placed: where each goes, and the plan it would record.
struct ChildrenLayout {
    /// In-flow children whose subtrees are placed again, in order.
    descend: Vec<Descend>,
    positioned: Vec<StableNodeId>,
    plan_entries: Option<Vec<PlannedChild>>,
    gap: f32,
    plan_sequential: bool,
    cross_independent: bool,
    main_dependent: bool,
    direction: FlexDirection,
    main_reversed: bool,
    cross_reversed: bool,
    recorded_grid: Option<Box<GridTrackPlan>>,
    #[cfg(feature = "benchmark")]
    child_phase: Option<super::plan_stats::PhaseClock>,
}

/// A container's walk of its children: measure them, pack lines, align, and
/// decide every in-flow child's box. It places no in-flow subtree: the caller
/// descends into `descend` once this frame is gone, so a level of the
/// placement recursion does not carry the walk's locals (see `recursion`).
#[allow(clippy::too_many_arguments)]
// Its own frame, taken only on the path that calls it (see `recursion`).
#[inline(never)]
fn lay_out_children(
    id: StableNodeId,
    style_arc: &Arc<LayoutStyle>,
    child_ids: &Arc<Vec<StableNodeId>>,
    content_origin: Point,
    content: Size,
    writing: nana_ui_core::WritingContext,
    fonts: FontSizeContext,
    child_font_px: f32,
    viewport: LayoutViewport,
    had_plan: bool,
    inherited: Option<&super::grid::InheritedGridTracks>,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<ChildrenLayout, UiWorldError> {
    let style = style_arc.as_ref();
    let mut descend = nodes.scratch.descend();
    if had_plan {
        intrinsic.note_plan_miss();
    }
    let (mut flow, descendant_dependent_flow) =
        collect_flow_children_reporting(child_ids, nodes, style)?;
    if let Some(scope) = scope {
        retire_omitted_children(child_ids, nodes, output, scope);
    }
    let mut positioned = collect_positioned_children(child_ids, nodes, style)?;
    sort_by_order(&mut flow, nodes);
    sort_by_order(&mut positioned, nodes);
    // This container is its children's containing block: their percentage
    // margins resolve against its inline size.
    let child_edge_base = writing.inline_size(content.width, content.height);
    let context = establish_context(id, style, &flow, nodes);
    nodes.contexts.insert(id, context.kind);
    let grid_2d = context.grid_2d;
    let ifc = context.inline();
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
    let mut gap = style.main_gap_against_fonts(direction, parent_box, fonts);
    let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
    // A new child list that moved only the main extent of the content box:
    // a child the container kept keeps the size its last plan recorded, as
    // when the container's main size moves. A line aligned by baselines
    // reads each item's from this pass.
    let kept = scope
        .and_then(|scope| Some((scope.retained.container_plans.get(&id)?, scope)))
        .filter(|(plan, _)| {
            !Arc::ptr_eq(&plan.children, child_ids)
                && plan.grid.is_none()
                && !grid_2d
                && (plan.main_direction, plan.writing, plan.viewport)
                    == (direction, writing, viewport)
                && plan.child_font_px == child_font_px
                && cross_extent(plan.child_available, direction) == cross_extent(content, direction)
                && !flow.iter().any(|child| {
                    nodes.style_ref(*child).is_some_and(|child| {
                        child.resolved_align_self(style.child_align_items()) == AlignSpec::Baseline
                    })
                })
        });
    let mut child_sizes = nodes.scratch.sizes(flow.len());
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
        if let Some((plan, scope)) = kept
            && let Some(size) = kept_size(*child, plan, direction, nodes, scope)
        {
            child_sizes.push(size);
            continue;
        }
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
    // make the container uncacheable. Inline formatting, subgrids
    // and triggered menu overlays each add placement inputs the plan does
    // not model, and `children_layout_style_is_local` rules out the cases
    // where an ancestor could change a child's style without marking the
    // child dirty.
    // `descendant_dependent_flow` rules out the containers whose flow list is
    // not decided by the direct children's own styles: an inline-level child
    // is unboxed or not depending on its own subtree, so the plan's
    // `by_child` index cannot answer "is this affected id one of my entries?".
    // Recorded on full passes too, so the first scoped pass after a mount or a
    // viewport change already has a plan to reuse.
    let grid_recordable = grid_2d && !style.is_subgrid_columns() && !style.is_subgrid_rows();
    let cacheable = !descendant_dependent_flow
        && (grid_recordable || !grid_2d)
        && !ifc
        && !style.equal_wrap
        && triggered_menu_overlay(nodes.world, id).is_none()
        && nodes.world.children_layout_style_is_local(id);
    let mut plan_entries: Option<Vec<PlannedChild>> =
        cacheable.then(|| Vec::with_capacity(flow.len()));
    // Narrowed to false by anything the suffix replay cannot express.
    let mut plan_sequential = cacheable;
    let mut cross_independent = cacheable;
    // A line that solves its overflow reads its own main size: what it
    // takes from its items follows it.
    let mut main_dependent = style
        .adaptation
        .as_deref()
        .is_some_and(super::dynamic::solves);
    // Each child's measured size by id, for the plan: a container with few
    // children finds it in `flow` itself; only a long flow is worth a table.
    let plan_intrinsics: Option<HashMap<StableNodeId, Size>> =
        (cacheable && flow.len() > 16).then(|| {
            flow.iter()
                .copied()
                .zip(child_sizes.iter().copied())
                .collect()
        });
    let planned_intrinsic = |child: StableNodeId| -> Option<Size> {
        match plan_intrinsics.as_ref() {
            Some(table) => table.get(&child).copied(),
            None => flow
                .iter()
                .position(|id| *id == child)
                .map(|index| child_sizes[index]),
        }
    };
    if !cacheable {
        intrinsic.note_container_uncacheable();
        // Retire a plan recorded while this container was still cacheable.
        nodes.container_plans.retire(id);
    }

    let mut recorded_grid = None;
    if grid_2d {
        let mut grid = layout_grid_2d(
            style,
            writing,
            &flow,
            &child_sizes,
            content,
            fonts,
            inherited,
            viewport,
            nodes,
            intrinsic,
            scope,
        )?;
        grid.fill_layer_cell(style, writing, content);
        if grid_recordable {
            recorded_grid = Some(Box::new(GridTrackPlan::from_layout(&grid)));
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
            scope,
            None,
            plan_entries.as_mut(),
            &mut descend,
        )?;
    } else {
        let arranged = arrange_flow(
            id,
            style,
            writing,
            &flow,
            &child_sizes,
            content_origin,
            content,
            child_edge_base,
            gap,
            cross_gap,
            ifc,
            direction,
            (main_reversed, cross_reversed),
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            scope,
            true,
        )?;
        gap = arranged.gap;
        plan_sequential &= arranged.sequential_justify;
        let line_count = arranged.lines.len();
        let mut items = arranged.children;
        let lines = arranged.lines;
        for item in items.drain(..) {
            let child = item.child;
            let child_style = item.style.as_ref();
            let child_fonts = fonts_of(child_style, child_font_px);
            let child_size = item.size;
            let child_origin = Point {
                x: content_origin.x + item.offset.x,
                y: content_origin.y + item.offset.y,
            };
            if let Some(entries) = plan_entries.as_mut()
                && let Some(child_intrinsic) = planned_intrinsic(child)
            {
                // Anything that couples this child's position to a sibling
                // other than through the running cursor takes the container
                // off the sequential path. Grow/shrink is detected from the
                // data rather than from the style: if the used main size
                // still equals the intrinsic, no free space was
                // redistributed.
                let align = child_style.resolved_align_self(style.child_align_items());
                plan_sequential &= line_count == 1
                    && lines[item.line].auto_main == 0
                    && main_extent(child_size, direction) == main_extent(child_intrinsic, direction)
                    // `used == intrinsic` proves no space was distributed
                    // THIS pass. A child that can grow may still start
                    // distributing once a later edit frees space up, and
                    // the replay does not model that, so exclude it now.
                    && child_style.flex_grow.unwrap_or(0.0) <= 0.0
                    && child_style.flex_shrink.unwrap_or(0.0) <= 0.0
                    && matches!(align, AlignSpec::Start | AlignSpec::Stretch);
                cross_independent &= matches!(align, AlignSpec::Start | AlignSpec::Stretch)
                    && !reads_container_size(child_style, direction);
                // A child whose height follows its content still reads a
                // column's main size when something under it does.
                main_dependent |= reads_main_extent(child_style, direction)
                    || (direction == FlexDirection::Column
                        && nodes.world.block_extent_reaches(child));
                entries.push(PlannedChild {
                    child,
                    style: item.style.to_arc(),
                    intrinsic: child_intrinsic,
                    at_main: item.at_main.map(Box::new),
                    origin: child_origin,
                    size: child_size,
                    cursor_before: item.cursor_before,
                    baseline: PackedOptF32::from_opt(item.baseline),
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
                descend.push(Descend {
                    child,
                    origin: child_origin,
                    size: child_size,
                    containing: content,
                    inherited: None,
                });
            }
        }
        nodes.scratch.give_arranged(items);
        nodes.scratch.give_lines(lines);
    }
    nodes.scratch.give_ids(flow);
    nodes.scratch.give_sizes(child_sizes);
    Ok(ChildrenLayout {
        descend,
        positioned,
        plan_entries,
        gap,
        plan_sequential,
        cross_independent,
        main_dependent,
        direction,
        main_reversed,
        cross_reversed,
        recorded_grid,
        #[cfg(feature = "benchmark")]
        child_phase,
    })
}

/// An in-flow child of a box's flow where its lines put it.
pub(super) struct ArrangedChild<'a> {
    pub(super) child: StableNodeId,
    pub(super) style: StyleRef<'a>,
    /// Its border box's origin from the content box's, on the page.
    pub(super) offset: Point,
    pub(super) size: Size,
    /// Where its line's main cursor stood before it.
    pub(super) cursor_before: f32,
    /// The first baseline its line aligns it by, under `align-self:
    /// baseline`.
    pub(super) baseline: Option<f32>,
    /// The main size its line gave it and its size measured at it, where
    /// that differs from what it measured.
    pub(super) at_main: Option<(f32, Size)>,
    /// Its line, in [`ArrangedFlow::lines`].
    pub(super) line: usize,
}

/// One line of a box's flow.
pub(super) struct ArrangedLine {
    /// Where it starts across the flow, from the content box's cross start
    /// before a reversed cross axis turns it onto the page.
    pub(super) cross_start: f32,
    /// The baseline its inline-level items sit on, from its cross start,
    /// when it is a line of an ordinary block.
    pub(super) atom_baseline: Option<f32>,
    /// How many of its items have an auto margin on the main axis.
    pub(super) auto_main: usize,
    /// Its items, in [`ArrangedFlow::children`].
    pub(super) children: std::ops::Range<usize>,
}

/// A box's in-flow children where its lines put them: the one arrangement
/// placement places them by and baseline derivation reads (#394).
pub(super) struct ArrangedFlow<'a> {
    pub(super) children: Vec<ArrangedChild<'a>>,
    pub(super) lines: Vec<ArrangedLine>,
    /// The gap a solved single line settled on.
    pub(super) gap: f32,
    /// Whether its justification leaves the children where a running
    /// cursor puts them (`start`, no grid tracks).
    pub(super) sequential_justify: bool,
}

/// Breaks `flow` (measured at `child_sizes`) into the lines of the box `id`
/// lays out at `content`, hands each line's free space out, aligns each
/// item across it and turns the result onto the page. Flex lines (row or
/// column, wrapping or not) and an inline formatting context's line boxes. A
/// 2D grid is placed elsewhere. `commit` lets a solved line keep its
/// assignment, as placement does; a reader passes `false`.
#[allow(clippy::too_many_arguments)]
pub(super) fn arrange_flow<'a>(
    id: StableNodeId,
    style: &LayoutStyle,
    writing: nana_ui_core::WritingContext,
    flow: &[StableNodeId],
    child_sizes: &[Size],
    _content_origin: Point,
    content: Size,
    child_edge_base: f32,
    mut gap: f32,
    cross_gap: f32,
    ifc: bool,
    direction: FlexDirection,
    (main_reversed, cross_reversed): (bool, bool),
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'a>,
    intrinsic: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
    commit: bool,
) -> Result<ArrangedFlow<'a>, UiWorldError> {
    let mut children = nodes.scratch.arranged(flow.len());
    let mut lines = nodes.scratch.lines();
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
    let full_main = main_extent(content, direction);
    let line_slot = |indices| LineBoxSlot {
        indices,
        main_start: 0.0,
        main_available: full_main,
        cross_y: 0.0,
        pin_cross: false,
        baseline_aligned: false,
    };
    let mut line_slots = if wrapping {
        if style.equal_wrap {
            pack_equal_wrap_lines(
                flow,
                child_sizes,
                direction,
                full_main,
                child_edge_base,
                gap,
                child_font_px,
                nodes,
            )
            .into_iter()
            .map(|range| line_slot(range.collect()))
            .collect()
        } else if ifc {
            let inline_avail = writing.inline_size(content.width, content.height);
            prime_atom_baselines(
                flow.iter().copied().zip(child_sizes.iter().copied()),
                inline_avail,
                viewport,
                child_font_px,
                nodes,
                intrinsic,
                scope,
            )?;
            pack_ifc_line_boxes(
                flow,
                child_sizes,
                writing,
                content,
                gap,
                cross_gap,
                viewport,
                child_font_px,
                line_strut(id, style, writing, child_font_px, nodes),
                nodes,
            )
        } else {
            pack_wrap_lines(
                flow,
                child_sizes,
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
            .map(line_slot)
            .collect()
        }
    } else {
        let mut indices = nodes.scratch.indices();
        indices.extend(0..flow.len());
        let mut slots = nodes.scratch.line_slots();
        slots.push(line_slot(indices));
        slots
    };
    if matches!(wrap, FlexWrap::WrapReverse) {
        line_slots.reverse();
    }
    #[allow(clippy::type_complexity)]
    let mut packed: Vec<(
        Vec<StableNodeId>,
        Vec<Size>,
        f32,
        f32,
        f32,
        f32,
        bool,
        Option<(f32, f32)>,
        Option<f32>,
    )> = Vec::with_capacity(line_slots.len());
    // Items measured again at the main size their line gave them.
    let mut hypothetical: HashMap<StableNodeId, (f32, Size)> = HashMap::new();
    for slot in &line_slots {
        let mut line_flow = nodes.scratch.ids();
        line_flow.extend(slot.indices.iter().map(|&index| flow[index]));
        let mut line_sizes = nodes.scratch.sizes(slot.indices.len());
        line_sizes.extend(slot.indices.iter().map(|&index| child_sizes[index]));
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
        let mut line_adjust = None;
        if style.equal_wrap && wrapping {
            let share = equal_wrap_share(
                &line_flow,
                direction,
                slot.main_available,
                child_edge_base,
                gap,
                child_font_px,
                nodes,
            );
            for size in &mut line_sizes {
                set_main_extent(size, direction, share);
            }
        } else if let Some(tracks) = line_tracks.filter(|tracks| !tracks.is_empty()) {
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
            let single_line = line_slots.len() == 1
                && !ifc
                && !matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse);
            let line = super::dynamic::DynamicLine::of(
                id,
                style,
                writing,
                single_line,
                &mut intrinsic.dynamic,
                scope.map(|scope| scope.retained),
                commit,
            );
            let adjust = distribute_flex_main(
                &line_flow,
                &mut line_sizes,
                direction,
                line_content,
                child_edge_base,
                gap,
                viewport,
                child_font_px,
                nodes,
                line,
            );
            // A solved line sets its items closer: every later step of
            // this single line reads the gap it settled on.
            gap = adjust.gap;
            line_adjust = Some(adjust);
        }
        // An item the line gave another main size than it was measured
        // with is as tall (wide, in a column) as its content at that size.
        for (slot_index, child) in line_flow.iter().enumerate() {
            let measured = child_sizes[slot.indices[slot_index]];
            let Some(child_style) = nodes.style_ref(*child) else {
                continue;
            };
            // An item that gave up only its own chrome keeps its content
            // box, and so its cross size.
            if line_adjust
                .as_ref()
                .is_some_and(|adjust| adjust.compressed(slot_index))
            {
                continue;
            }
            if !cross_follows_used_main(&child_style, direction, measured, line_sizes[slot_index]) {
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
        // A line of an ordinary block: its baseline, and the span its
        // items cover once they sit on it.
        let inline_avail = writing.inline_size(content.width, content.height);
        let line_baseline_y = if slot.baseline_aligned {
            prime_atom_baselines(
                line_flow.iter().copied().zip(line_sizes.iter().copied()),
                inline_avail,
                viewport,
                child_font_px,
                nodes,
                intrinsic,
                scope,
            )?;
            Some(ifc_line_metrics(
                line_flow.iter().copied().zip(line_sizes.iter().copied()),
                writing,
                content,
                viewport,
                child_font_px,
                line_strut(id, style, writing, child_font_px, nodes),
                nodes,
            ))
        } else {
            None
        };
        let mut flex_line_ascent = None;
        let mut line_cross = line_baseline_y.map_or(line_cross, |(_, height)| height);
        if !slot.baseline_aligned
            && style.display.is_some_and(DisplaySpec::is_flex_container)
            && direction == writing.inline_flex_direction()
            && line_flow.iter().any(|child| {
                nodes.style_ref(*child).is_some_and(|child_style| {
                    child_style.resolved_align_self(style.child_align_items())
                        == AlignSpec::Baseline
                })
            })
        {
            prime_first_baselines(
                line_flow.iter().copied().zip(line_sizes.iter().copied()),
                inline_avail,
                viewport,
                child_font_px,
                nodes,
                intrinsic,
                scope,
            )?;
            if let Some((line_ascent, grown_cross)) = flex_baseline_line_metrics(
                line_flow.iter().copied().zip(line_sizes.iter().copied()),
                style,
                writing,
                direction,
                cross_reversed,
                child_edge_base,
                child_font_px,
                nodes,
                intrinsic,
            ) {
                line_cross = line_cross.max(grown_cross);
                flex_line_ascent = Some(line_ascent);
            }
        }
        // A line's start, measured from main-start: its physical left offset,
        // or the room its right end leaves when the main axis runs from the
        // right.
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
            line_baseline_y,
            flex_line_ascent,
        ));
    }
    let align_content = style.align_content;
    let line_count = packed.len();
    let container_cross = cross_extent(content, direction);
    let (mut cross_cursor, extra_cross_gap) = if line_count > 1 {
        let total = packed
            .iter()
            .map(|(_, _, cross, _, _, _, _, _, _)| *cross)
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
        line_index,
        (
            line_flow,
            line_sizes,
            line_cross,
            line_origin_main,
            line_main_available,
            line_cross_y,
            pin_cross,
            line_atom_baseline,
            flex_line_ascent,
        ),
    ) in packed.into_iter().enumerate()
    {
        if pin_cross {
            cross_cursor = cross_cursor.max(line_cross_y);
        }
        let line_start_cross = cross_cursor;
        let first_child = children.len();
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
        let needs_baseline = style.child_align_items() == AlignSpec::Baseline
            || line_flow.iter().any(|id| {
                nodes.style_ref(*id).is_some_and(|child| {
                    child.resolved_align_self(style.child_align_items()) == AlignSpec::Baseline
                })
            });
        if needs_baseline {
            prime_first_baselines(
                line_flow.iter().copied().zip(line_sizes.iter().copied()),
                writing.inline_size(content.width, content.height),
                viewport,
                child_font_px,
                nodes,
                intrinsic,
                scope,
            )?;
        }
        let line_baseline = if let Some(ascent) = flex_line_ascent {
            ascent
        } else if needs_baseline {
            line_flow
                .iter()
                .zip(line_sizes.iter())
                .filter_map(|(id, size)| {
                    nodes.style_ref(*id).map(|style| {
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
        for (&child, &child_size) in line_flow.iter().zip(line_sizes.iter()) {
            let mut child_size = child_size;
            let Some(child_style_arc) = nodes.style_shared(child) else {
                continue;
            };
            let child_style = child_style_arc.as_ref();
            let child_fonts = fonts_of(child_style, child_font_px);
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
            let align = child_style.resolved_align_self(style.child_align_items());
            let cross_available = line_box_cross - cross_margin(margin, direction);
            // An atom on a line of an ordinary block sits on the line's
            // baseline whatever `align-self` says, and is not stretched.
            let on_baseline_line = line_atom_baseline.is_some() && child_style.is_inline_level();
            if align == AlignSpec::Stretch && !on_baseline_line && {
                let natural_ratio = has_natural_ratio(nodes.world, child);
                !cross_axis_is_definite(child_style, direction, natural_ratio)
                    && !block_keeps_content_size(style, writing, child_style, natural_ratio)
            } {
                set_cross_extent(&mut child_size, direction, cross_available.max(0.0));
            }
            fill_auto_height_from_aspect_ratio(
                child_style,
                &mut child_size,
                Some(writing.inline_size(content.width, content.height)),
                Some(content.height),
                viewport,
                child_fonts,
            );
            if line_atom_baseline.is_some() {
                prime_atom_baselines(
                    [(child, child_size)],
                    writing.inline_size(content.width, content.height),
                    viewport,
                    child_font_px,
                    nodes,
                    intrinsic,
                    scope,
                )?;
            }
            let atom_placement = line_atom_baseline.and_then(|_| {
                let strut = line_strut(id, style, writing, child_font_px, nodes).into();
                ifc_atom_metrics(
                    child,
                    child_style,
                    child_size,
                    writing,
                    writing.inline_size(content.width, content.height),
                    child_font_px,
                    strut,
                    viewport,
                    nodes,
                )
            });
            let aligned_baseline = (align == AlignSpec::Baseline).then(|| {
                intrinsic
                    .baseline(child, crate::Baseline::First)
                    .unwrap_or_else(|| nodes.baseline(child, child_fonts.element_px, child_size))
            });
            let cross_offset = if let (Some((line_base, line_height)), Some(placement)) =
                (line_atom_baseline, atom_placement)
            {
                match placement {
                    crate::inline_content::AtomLinePlacement::Baseline { above, .. } => {
                        // The margin box's top is `above` above the baseline.
                        cross_cursor + line_base - above + cross_lead(margin)
                    }
                    crate::inline_content::AtomLinePlacement::Top { .. } => {
                        cross_cursor + cross_lead(margin)
                    }
                    crate::inline_content::AtomLinePlacement::Bottom { height } => {
                        cross_cursor + line_height - height + cross_lead(margin)
                    }
                }
            } else {
                match align {
                    AlignSpec::Start | AlignSpec::Stretch => cross_cursor + cross_lead(margin),
                    AlignSpec::Baseline => {
                        let base = aligned_baseline.unwrap_or_default();
                        if flex_line_ascent.is_some() {
                            cross_cursor + line_baseline - base
                        } else {
                            cross_cursor + (line_baseline - base).max(0.0)
                        }
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
            let child_offset = match direction {
                FlexDirection::Row => Point {
                    x: main_start,
                    y: cross_offset,
                },
                FlexDirection::Column => Point {
                    x: cross_offset,
                    y: main_start,
                },
            };
            children.push(ArrangedChild {
                child,
                style: child_style_arc,
                offset: child_offset,
                size: child_size,
                cursor_before: cursor,
                baseline: aligned_baseline,
                at_main: hypothetical.get(&child).copied(),
                line: line_index,
            });
            cursor += main_extent(child_size, direction)
                + main_lead(margin)
                + main_trail(margin)
                + effective_gap;
        }
        nodes.scratch.give_ids(line_flow);
        nodes.scratch.give_sizes(line_sizes);
        lines.push(ArrangedLine {
            cross_start: line_start_cross,
            atom_baseline: line_atom_baseline.map(|(baseline, _)| baseline),
            auto_main,
            children: first_child..children.len(),
        });
        cross_cursor += line_cross + extra_cross_gap;
    }
    for slot in line_slots.drain(..) {
        nodes.scratch.give_indices(slot.indices);
    }
    nodes.scratch.give_line_slots(line_slots);
    Ok(ArrangedFlow {
        children,
        lines,
        gap,
        sequential_justify: justify == JustifySpec::Start && grid_tracks.is_none(),
    })
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
    nodes: &mut LayoutInputMap<'_>,
    output: &mut PassOutput,
    scope: &ScopeContext<'_>,
) {
    for &child in children {
        if !nodes
            .style_ref(child)
            .is_some_and(|style| style.omits_box())
        {
            continue;
        }
        nodes.omitted_boxes.insert(child);
        // No box, and so no frame other than a cold layout's.
        let framed = scope.retained.frames.contains_key(&child);
        if framed {
            nodes.frames.insert(child, crate::LayoutFrame::Parent);
        }
        if framed
            || scope
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
pub(super) fn place_positioned_child(
    child: StableNodeId,
    content_origin: Point,
    content: Size,
    viewport: LayoutViewport,
    child_font_px: f32,
    writing: nana_ui_core::WritingContext,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Option<PlannedOverlay>, UiWorldError> {
    let Some(child_style_arc) = nodes.style(child) else {
        return Ok(None);
    };
    let child_style = child_style_arc.as_ref();
    if free_item_of(child, child_style, nodes).is_some() {
        return place_free_item(
            child,
            child_style_arc,
            content_origin,
            content,
            viewport,
            child_font_px,
            writing,
            nodes,
            intrinsic,
            output,
            scope,
        );
    }
    if !child_style.position.is_out_of_flow() {
        return Ok(None);
    }
    let fixed = child_style.position == PositionSpec::Fixed;
    let block = if fixed {
        None
    } else {
        absolute_containing_block(child, nodes)
    };
    let container = nodes.world.parent_id(child);
    let in_container = block.is_some() && block == container;
    // The child's box is measured from its containing block's origin (the
    // page's for the initial one and for a fixed box), so that only the
    // block moving moves it; everything below is in that frame.
    let (frame, base_origin, base) = match block {
        None => (
            crate::LayoutFrame::Page,
            Point::ZERO,
            Size::new(viewport.width, viewport.height),
        ),
        Some(block) => match padding_box(block, nodes, output, scope) {
            Some((origin, size)) if in_container => (crate::LayoutFrame::Parent, origin, size),
            Some((origin, size)) => (crate::LayoutFrame::Block(block), origin, size),
            None => (crate::LayoutFrame::Parent, content_origin, content),
        },
    };
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
    // A declared ratio fills an auto height from the width the insets and
    // limits left, as in flow; the height then meets its own limits.
    if auto_height {
        fill_auto_height_from_aspect_ratio(
            child_style,
            &mut child_size,
            Some(base.width),
            Some(base.height),
            viewport,
            child_fonts,
        );
    }
    child_size.height = child_size.height.max(child_style.resolved_min_height_fonts(
        Some(base.height),
        vp,
        child_fonts,
    ));
    if let Some(max) = child_style.resolved_max_height_fonts(Some(base.height), vp, child_fonts) {
        child_size.height = child_size.height.min(max);
    }
    let static_position =
        !fixed && ((left.is_none() && right.is_none()) || (top.is_none() && bottom.is_none()));
    // Where the container's content box stands in the child's frame: the
    // box's static position. Only read when an axis keeps it.
    let static_origin = match (frame, container) {
        (_, _) if fixed || !static_position => content_origin,
        (crate::LayoutFrame::Parent, _) | (_, None) => content_origin,
        (crate::LayoutFrame::Block(block), Some(container)) => {
            content_origin_in(Some(block), container, content_origin, nodes, output, scope)
        }
        (crate::LayoutFrame::Page, Some(container)) => {
            content_origin_in(None, container, content_origin, nodes, output, scope)
        }
    };
    let (static_x, static_y) = if fixed {
        (0.0, 0.0)
    } else {
        (
            static_origin.x - base_origin.x,
            static_origin.y - base_origin.y,
        )
    };
    let inset = match child_style.anchor.as_deref().copied().filter(|_| fixed) {
        // Anchor positioning: hung from its anchor, at the size it measured,
        // by the one resolver every anchored surface shares.
        Some(anchor) => {
            let surface = anchored_surface(anchor, child_size, viewport);
            Point {
                x: surface.x - base_origin.x,
                y: surface.y - base_origin.y,
            }
        }
        // An axis with both insets auto keeps the static position, where
        // the box would start in flow: the parent's content origin (CSS 2.1
        // §10.3.7). A fixed box keeps the viewport's origin there.
        None => Point {
            x: left.unwrap_or_else(|| {
                right.map_or(static_x, |value| base.width - value - child_size.width)
            }),
            y: top.unwrap_or_else(|| {
                bottom.map_or(static_y, |value| base.height - value - child_size.height)
            }),
        },
    };
    let child_origin = Point {
        x: base_origin.x + inset.x,
        y: base_origin.y + inset.y,
    };
    // A box placed in another frame than last pass is placed again: its
    // retained box is measured from the other one.
    let frame_kept = pass_frame(child, nodes, scope) == frame;
    // Record the child's frame before entering its subtree so placement
    // queries (e.g. viewport distance) see its actual frame chain. A box that
    // generates none keeps its parent's frame, as a cold layout leaves it.
    if !child_style.omits_box() {
        nodes.frames.insert(child, frame);
    }
    if !frame_kept
        || !subtree_unchanged(
            child,
            child_origin,
            child_size,
            base,
            child_style,
            child_fonts,
            writing,
            scope,
        )
    {
        place_node_scoped(
            child,
            child_origin,
            child_size,
            base,
            viewport,
            child_font_px,
            None,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
    }
    let tracks_containing_block = positioned_tracks_containing_block(child_style);
    Ok(Some(PlannedOverlay {
        child,
        style: child_style_arc,
        fixed,
        block,
        in_container,
        static_origin: static_position.then_some(static_origin),
        tracks_containing_block,
        base,
        base_origin,
        origin: child_origin,
        size: child_size,
    }))
}

/// `child`'s parent when it is a `display: free` container placing `child`
/// by its explicit rect.
fn free_item_of(
    child: StableNodeId,
    child_style: &LayoutStyle,
    nodes: &LayoutInputMap<'_>,
) -> Option<StableNodeId> {
    let parent = nodes.world.parent_id(child)?;
    let parent_style = nodes.style_ref(parent)?;
    crate::layout_node::is_free_item(Some(&parent_style), child_style).then_some(parent)
}

/// Place a child of a `display: free` container from its explicit rect: in
/// the container's padding box, measured from the container's origin. The
/// rect is the child's border box, so nothing measures the child for it; a
/// child without one sits at the padding box's origin at its shrink-to-fit
/// size. Only the parent is read: no sibling, and no flow. The document
/// coordinates follow from the parent's frame, so a parent that moves
/// leaves this box, and the rect, as they are.
#[allow(clippy::too_many_arguments)]
fn place_free_item(
    child: StableNodeId,
    child_style_arc: Arc<LayoutStyle>,
    content_origin: Point,
    content: Size,
    viewport: LayoutViewport,
    child_font_px: f32,
    writing: nana_ui_core::WritingContext,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Option<PlannedOverlay>, UiWorldError> {
    let child_style = child_style_arc.as_ref();
    let Some(parent) = nodes.world.parent_id(child) else {
        return Ok(None);
    };
    let frame = crate::LayoutFrame::Parent;
    let (base_origin, base) =
        padding_box(parent, nodes, output, scope).unwrap_or((content_origin, content));
    let rect = nodes.world.explicit_rect(child);
    let (offset, child_size) = match rect {
        Some(rect) => (
            Point {
                x: rect.x,
                y: rect.y,
            },
            Size::new(rect.width, rect.height),
        ),
        None => (
            Point::ZERO,
            intrinsic_size_out_of_flow(
                child,
                Size::new(base.width, 0.0),
                viewport,
                child_font_px,
                nodes,
                intrinsic,
                scope,
            )?,
        ),
    };
    let child_origin = Point {
        x: base_origin.x + offset.x,
        y: base_origin.y + offset.y,
    };
    let frame_kept = pass_frame(child, nodes, scope) == frame;
    nodes.frames.insert(child, frame);
    let child_fonts = fonts_of(child_style, child_font_px);
    if !frame_kept
        || !subtree_unchanged(
            child,
            child_origin,
            child_size,
            base,
            child_style,
            child_fonts,
            writing,
            scope,
        )
    {
        place_node_scoped(
            child,
            child_origin,
            child_size,
            base,
            viewport,
            child_font_px,
            None,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
    }
    Ok(Some(PlannedOverlay {
        child,
        style: child_style_arc,
        fixed: false,
        block: Some(parent),
        in_container: true,
        static_origin: None,
        // A rect is in pixels: the padding box resizing moves nothing. A
        // shrink-to-fit box reads the width it is offered.
        tracks_containing_block: rect.is_none(),
        base,
        base_origin,
        origin: child_origin,
        size: child_size,
    }))
}

/// The containing block of an absolutely positioned `child` (CSS 2.1
/// §10.1): its nearest positioned ancestor, or `None` for the initial
/// containing block when it has none. A free item is one too: what is
/// positioned inside a layer is positioned in that layer.
fn absolute_containing_block(
    child: StableNodeId,
    nodes: &LayoutInputMap<'_>,
) -> Option<StableNodeId> {
    let world = nodes.world;
    let mut ancestor = world.parent_id(child);
    while let Some(id) = ancestor {
        if world.is_document_node(id) {
            return None;
        }
        if nodes.style_ref(id).is_some_and(|style| {
            style.position.establishes_containing_block()
                || free_item_of(id, &style, nodes).is_some()
        }) {
            return Some(id);
        }
        ancestor = world.parent_id(id);
    }
    None
}

/// The padding box of `block` as this pass placed it, or as the last one
/// left it when this pass did not place it, measured from the block's own
/// origin: the edge an absolute child's insets and percentages resolve
/// against.
fn padding_box(
    block: StableNodeId,
    nodes: &LayoutInputMap<'_>,
    output: &PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Option<(Point, Size)> {
    let border_box = pass_box(block, output, scope)?;
    let border = nodes.style_ref(block)?.resolved_border_edges();
    Some((
        Point {
            x: border.left,
            y: border.top,
        },
        Size::new(
            (border_box.width - border.left - border.right).max(0.0),
            (border_box.height - border.top - border.bottom).max(0.0),
        ),
    ))
}

/// `id`'s box as this pass placed it, or as the last one left it when this
/// pass did not place it, measured from its frame's origin.
pub(super) fn pass_box(
    id: StableNodeId,
    output: &PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Option<LayoutBox> {
    output
        .get(&id)
        .or_else(|| scope.and_then(|scope| scope.retained.boxes.get(&id)))
        .copied()
}

/// What `id`'s box is measured from, as this pass or the last one left it.
fn pass_frame(
    id: StableNodeId,
    nodes: &LayoutInputMap<'_>,
    scope: Option<&ScopeContext<'_>>,
) -> crate::LayoutFrame {
    nodes
        .frames
        .get(&id)
        .or_else(|| scope.and_then(|scope| scope.retained.frames.get(&id)))
        .copied()
        .unwrap_or_default()
}

/// Where `container`'s content box, at `content_origin` from its own
/// origin, stands from `ancestor`'s origin, or the page's for `None`. The
/// boxes on the way are summed down from the first one under `ancestor`,
/// as a document box is. An ancestor the frames do not lead through is
/// reached through the page.
pub(super) fn content_origin_in(
    ancestor: Option<StableNodeId>,
    container: StableNodeId,
    content_origin: Point,
    nodes: &LayoutInputMap<'_>,
    output: &PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Point {
    let origin = origin_in(ancestor, container, nodes, output, scope);
    Point {
        x: origin.x + content_origin.x,
        y: origin.y + content_origin.y,
    }
}

/// Where `id`'s origin stands from `ancestor`'s, or the page's for `None`.
pub(super) fn origin_in(
    ancestor: Option<StableNodeId>,
    id: StableNodeId,
    nodes: &LayoutInputMap<'_>,
    output: &PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Point {
    let world = nodes.world;
    let mut chain: Vec<Point> = Vec::new();
    let mut cursor = Some(id);
    let mut reached = ancestor.is_none();
    while let Some(node) = cursor {
        if Some(node) == ancestor {
            reached = true;
            break;
        }
        let local = pass_box(node, output, scope).unwrap_or_default();
        chain.push(Point {
            x: local.x,
            y: local.y,
        });
        cursor = match pass_frame(node, nodes, scope) {
            crate::LayoutFrame::Parent => world
                .parent_id(node)
                .filter(|parent| !world.is_document_node(*parent)),
            crate::LayoutFrame::Block(block) => Some(block),
            crate::LayoutFrame::Page => None,
        };
    }
    if !reached {
        // The page origin of both, apart.
        let from = origin_in(
            None,
            ancestor.expect("only an ancestor is missed"),
            nodes,
            output,
            scope,
        );
        let to = origin_in(None, id, nodes, output, scope);
        return Point {
            x: to.x - from.x,
            y: to.y - from.y,
        };
    }
    let mut sum = chain.pop().unwrap_or(Point::ZERO);
    while let Some(next) = chain.pop() {
        sum = Point {
            x: sum.x + next.x,
            y: sum.y + next.y,
        };
    }
    sum
}

/// Where a fixed box with `anchor` lands at `size` in `viewport`: inside its
/// room, which is the viewport or the part of it the anchor names.
fn anchored_surface(
    anchor: nana_ui_core::AnchorSpec,
    size: Size,
    viewport: LayoutViewport,
) -> LayoutBox {
    let rect = |[x, y, width, height]: [f32; 4]| LayoutBox {
        x,
        y,
        width,
        height,
    };
    let mut room = rect([0.0, 0.0, viewport.width, viewport.height]);
    if let Some(bounds) = anchor.room.map(rect) {
        let (right, bottom) = (
            (room.x + room.width).min(bounds.x + bounds.width),
            (room.y + room.height).min(bounds.y + bounds.height),
        );
        room.x = room.x.max(bounds.x);
        room.y = room.y.max(bounds.y);
        room.width = (right - room.x).max(0.0);
        room.height = (bottom - room.y).max(0.0);
    }
    crate::popover::resolve_anchored_surface(
        rect(anchor.rect),
        size.width,
        size.height,
        room,
        anchor.placement,
        anchor.alignment,
        anchor.gap,
        crate::popover::AnchorOverflow::Shift,
    )
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
        // An anchored box flips and clamps against the viewport.
        || style.anchor.is_some()
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

#[allow(clippy::too_many_arguments)]
fn overlay_depends_on_block(
    entry: &PlannedOverlay,
    plan: &ContainerPlan,
    content_origin: Point,
    content: Size,
    viewport: LayoutViewport,
    nodes: &LayoutInputMap<'_>,
    output: &PassOutput,
    scope: &ScopeContext<'_>,
) -> bool {
    if entry.fixed {
        return plan.viewport.width.to_bits() != viewport.width.to_bits()
            || plan.viewport.height.to_bits() != viewport.height.to_bits();
    }
    if !entry.in_container {
        return remote_block_moved(entry, plan, viewport, nodes, output, Some(scope));
    }
    if plan.content_origin != content_origin {
        return true;
    }
    (plan.content.width.to_bits() != content.width.to_bits()
        || plan.content.height.to_bits() != content.height.to_bits())
        && entry.tracks_containing_block
}

/// Absolute children hung from an ancestor or from the initial containing
/// block, in containers this pass did not place: no replay of their plan
/// reaches them, so their block moving or resizing (the viewport, for the
/// initial one) places them here. One placed here can be the containing
/// block of another such child; the walk repeats until nothing moves. A
/// child the plan still holds but that is gone or back in flow leaves the
/// plan, and placing a child where the plan already had it moves nothing.
pub(super) fn place_remote_overlays(
    viewport: LayoutViewport,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: &ScopeContext<'_>,
) -> Result<(), UiWorldError> {
    let mut containers: Vec<StableNodeId> = scope
        .retained
        .remote_overlays
        .iter()
        .copied()
        .filter(|id| !output.contains_key(id))
        .collect();
    if containers.is_empty() {
        return Ok(());
    }
    containers.sort_unstable();
    loop {
        let mut placed_any = false;
        for &id in &containers {
            let plan = match nodes.container_plans.get(&id) {
                Some(Some(plan)) => plan.clone(),
                Some(None) => continue,
                None => match scope.retained.container_plans.get(&id) {
                    Some(plan) => plan.clone(),
                    None => continue,
                },
            };
            let stale: Vec<StableNodeId> = plan
                .overlay
                .iter()
                .filter(|entry| {
                    !entry.fixed
                        && !entry.in_container
                        && !output.contains_key(&entry.child)
                        && remote_block_moved(entry, &plan, viewport, nodes, output, Some(scope))
                })
                .map(|entry| entry.child)
                .collect();
            if stale.is_empty() {
                continue;
            }
            let Some(writing) = nodes.get(id)?.map(|node| node.writing) else {
                continue;
            };
            let mut placed = Vec::with_capacity(stale.len());
            let mut gone = Vec::new();
            let mut moved = false;
            for child in stale {
                match place_positioned_child(
                    child,
                    plan.content_origin,
                    plan.content,
                    viewport,
                    plan.child_font_px,
                    writing,
                    nodes,
                    intrinsic,
                    output,
                    Some(scope),
                )? {
                    Some(entry) => {
                        // Only a box written or a placement the plan did not
                        // hold yet moves anything: the same result again
                        // leaves the next round where this one was.
                        moved |= output.contains_key(&child)
                            || plan
                                .overlay
                                .iter()
                                .find(|kept| kept.child == child)
                                .is_none_or(|kept| !kept.same_placement(&entry));
                        placed.push(entry);
                    }
                    // Removed, or back in flow: the plan no longer hangs it.
                    None => gone.push(child),
                }
            }
            publish_overlay_plan(
                id,
                &plan,
                plan.origin,
                plan.size,
                plan.content_origin,
                plan.content,
                &placed,
                false,
                nodes,
            );
            if !gone.is_empty() {
                if let Some(published) = nodes.container_plans.rebuilt_mut(&id) {
                    published
                        .overlay
                        .retain(|entry| !gone.contains(&entry.child));
                }
                moved = true;
            }
            placed_any |= moved;
        }
        if !placed_any {
            return Ok(());
        }
    }
}

/// Whether an absolute child hung from an ancestor or from the initial
/// containing block has to be placed again: its containing block is another
/// box now, or moved, or changed size under a child that reads its size.
fn remote_block_moved(
    entry: &PlannedOverlay,
    plan: &ContainerPlan,
    viewport: LayoutViewport,
    nodes: &LayoutInputMap<'_>,
    output: &PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> bool {
    if absolute_containing_block(entry.child, nodes) != entry.block {
        return true;
    }
    // A box at its static position moves with the container, measured
    // from the containing block: with every box between them.
    if let Some(origin) = entry.static_origin
        && let Some(container) = nodes.world.parent_id(entry.child)
        && content_origin_in(
            entry.block,
            container,
            plan.content_origin,
            nodes,
            output,
            scope,
        ) != origin
    {
        return true;
    }
    let (origin, size) = match entry.block {
        None => (Point::ZERO, Size::new(viewport.width, viewport.height)),
        Some(block) => match padding_box(block, nodes, output, scope) {
            Some(block) => block,
            None => return true,
        },
    };
    origin != entry.base_origin
        || (entry.tracks_containing_block
            && (size.width.to_bits() != entry.base.width.to_bits()
                || size.height.to_bits() != entry.base.height.to_bits()))
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
    output: &mut PassOutput,
    scope: &ScopeContext<'_>,
) -> Result<(), UiWorldError> {
    let stale = plan.overlay.iter().any(|entry| {
        nodes
            .style(entry.child)
            .is_none_or(|style| !crate::layout_node::leaves_parent_flow(Some(&plan.style), &style))
    });
    let mut placed: Vec<PlannedOverlay> = Vec::new();
    if stale {
        intrinsic.note_local_context_fallback();
        let live = collect_positioned_children(&plan.children, nodes, &plan.style)?;
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
                    || overlay_depends_on_block(
                        entry,
                        plan,
                        content_origin,
                        content,
                        viewport,
                        nodes,
                        output,
                        scope,
                    )
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
    if let Some(existing) = nodes.container_plans.rebuilt_mut(&id) {
        write(existing);
        return;
    }
    // The retained plan with what the pass's replay patched in it: the
    // overlay lands on the plan the pass commits.
    let mut updated = nodes.container_plans.patched_copy(id, plan);
    write(&mut updated);
    nodes.container_plans.rebuild(id, updated);
}

fn triggered_menu_overlay(
    world: &crate::UiWorld,
    id: StableNodeId,
) -> Option<crate::TriggeredMenuOverlay> {
    world
        .widget_visual_as::<crate::MenuSurfaceVisual>(id)
        .filter(|v| v.open)
        .and_then(|v| v.overlay)
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
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    let inner_width = (overlay.width - overlay.padding * 2.0).max(0.0);
    let available = Size::new(inner_width, viewport.height);
    let mut item_sizes = Vec::with_capacity(items.len());
    let mut content_height = 0.0;
    // Gaps fall between items that take room: an empty group of items (a
    // keyed list with no rows yet) adds none.
    let item_gap = nodes.world.node_theme(id).spacing().xxs;
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
                content_height += item_gap;
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
        let Some(child_style) = nodes.style_ref(child) else {
            continue;
        };
        let child_style = child_style.as_ref();
        let child_fonts = fonts_of(child_style, parent_font_px);
        let has_box = !child_style.omits_box();
        let child_origin = Point {
            x: item_x,
            y: cursor_y,
        };
        if pass_frame(child, nodes, scope) != crate::LayoutFrame::Page
            || !subtree_unchanged(
                child,
                child_origin,
                child_size,
                available,
                child_style,
                child_fonts,
                nodes.world.containing_writing(child),
                scope,
            )
        {
            place_node_scoped(
                child,
                child_origin,
                child_size,
                available,
                viewport,
                parent_font_px,
                None,
                nodes,
                intrinsic,
                output,
                scope,
            )?;
        }
        if has_box {
            nodes.frames.insert(child, crate::LayoutFrame::Page);
        }
        if child_size.height > 0.0 {
            cursor_y += child_size.height + item_gap;
        }
    }
    Ok(())
}
