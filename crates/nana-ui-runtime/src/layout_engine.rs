use std::collections::{HashMap, HashSet};

use nana_ui_core::{
    AlignSpec, BoxSizing, FlexDirection, JustifySpec, LengthSpec, PositionSpec,
};

use crate::{
    DocumentId, LayoutBox, LayoutInput, StableNodeId, UiWorld, UiWorldError,
};

/// Logical viewport supplied by the platform host to the retained layout system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutViewport {
    pub width: f32,
    pub height: f32,
}

impl LayoutViewport {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: finite_extent(width),
            height: finite_extent(height),
        }
    }
}

/// Backend-neutral flex flow used by canonical Runtime applications.
///
/// This is intentionally a layout owner, not a renderer adapter: it consumes
/// the same `LayoutStyle` and shaped text metrics stored in `UiWorld`, then
/// returns atomic layout writeback. Vue's broader CSS compatibility layout can
/// continue to supply its own writeback without creating another retained tree.
#[derive(Debug, Default, Clone, Copy)]
pub struct RuntimeLayoutEngine;

impl RuntimeLayoutEngine {
    pub fn layout_document(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
    ) -> Result<Vec<(StableNodeId, LayoutBox)>, UiWorldError> {
        let order = world.document_order(document);
        let inputs = world.layout_inputs(&order)?;
        let nodes = inputs
            .into_iter()
            .map(|input| (input.id, input))
            .collect::<HashMap<_, _>>();
        let roots = order
            .iter()
            .copied()
            .filter(|id| nodes[id].parent.is_none())
            .collect::<Vec<_>>();
        let mut output = HashMap::with_capacity(nodes.len());
        let mut intrinsic = HashMap::with_capacity(nodes.len());
        let available = Size::new(viewport.width, viewport.height);
        for root in roots {
            let root_size = intrinsic_size(root, available, None, viewport, &nodes, &mut intrinsic);
            place_node(
                root,
                Point::ZERO,
                root_size,
                available,
                viewport,
                &nodes,
                &mut intrinsic,
                &mut output,
            );
        }
        Ok(order
            .into_iter()
            .map(|id| (id, output.remove(&id).unwrap_or_default()))
            .collect())
    }

    /// Incremental variant of [`Self::layout_document`].
    ///
    /// `dirty` lists layout-dirty nodes; their ancestor closure (`affected`)
    /// is exactly the set of nodes whose subtree contains a change. Subtrees
    /// outside `affected` reuse the retained intrinsic size, and placement
    /// recursion prunes as soon as a recomputed child box is bit-identical to
    /// the retained one (same origin and size ⇒ identical internal layout,
    /// because subtree layout depends only on its own box and content). The
    /// returned vec contains only recomputed nodes; callers diff exactly
    /// those. `force_full` disables pruning (viewport semantics changed) and
    /// rebuilds the retained cache.
    pub fn layout_document_scoped(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
        dirty: &[StableNodeId],
        retained: &mut RetainedLayoutCache,
        force_full: bool,
    ) -> Result<Vec<(StableNodeId, LayoutBox)>, UiWorldError> {
        if force_full {
            retained.clear();
        }
        let order = world.document_order(document);
        let inputs = world.layout_inputs(&order)?;
        let nodes = inputs
            .into_iter()
            .map(|input| (input.id, input))
            .collect::<HashMap<_, _>>();
        let mut affected = HashSet::new();
        if !force_full {
            for &id in dirty {
                if !nodes.contains_key(&id) {
                    continue;
                }
                let mut cursor = Some(id);
                while let Some(id) = cursor {
                    if !affected.insert(id) {
                        break;
                    }
                    cursor = nodes[&id].parent;
                }
            }
        }
        let scope = ScopeContext {
            affected: &affected,
            retained: &*retained,
        };
        let scope_ref = (!force_full).then_some(&scope);
        let roots = order
            .iter()
            .copied()
            .filter(|id| nodes[id].parent.is_none())
            .collect::<Vec<_>>();
        let mut output = HashMap::with_capacity(nodes.len());
        let mut intrinsic = HashMap::with_capacity(nodes.len());
        let available = Size::new(viewport.width, viewport.height);
        for root in roots {
            let root_size = intrinsic_size_scoped(
                root,
                available,
                None,
                viewport,
                &nodes,
                &mut intrinsic,
                scope_ref,
            );
            place_node_scoped(
                root,
                Point::ZERO,
                root_size,
                available,
                viewport,
                &nodes,
                &mut intrinsic,
                &mut output,
                scope_ref,
            );
        }
        // Publish recomputed boxes; emitted set only.
        let mut emitted = Vec::with_capacity(output.len());
        for id in &order {
            if let Some(box_) = output.remove(id) {
                retained.boxes.insert(*id, box_);
                emitted.push((*id, box_));
            }
        }
        retained.intrinsics.extend(intrinsic);
        // Despawned ids linger in the retained maps; keep them bounded.
        if retained.boxes.len() > nodes.len().saturating_mul(2) {
            retained.boxes.retain(|id, _| nodes.contains_key(id));
        }
        if retained.intrinsics.len() > nodes.len().saturating_mul(4) {
            retained.intrinsics.retain(|(id, _, _), _| nodes.contains_key(id));
        }
        Ok(emitted)
    }
}

/// Cross-frame layout memo for scoped relayout: last published boxes and
/// intrinsic sizes keyed like the per-pass intrinsic cache.
#[derive(Default)]
pub struct RetainedLayoutCache {
    intrinsics: HashMap<(StableNodeId, u32, u32), Size>,
    boxes: HashMap<StableNodeId, LayoutBox>,
}

impl RetainedLayoutCache {
    fn clear(&mut self) {
        self.intrinsics.clear();
        self.boxes.clear();
    }
}

struct ScopeContext<'a> {
    affected: &'a HashSet<StableNodeId>,
    retained: &'a RetainedLayoutCache,
}

/// Prune a child recursion when the child is outside the affected closure and
/// its recomputed entry box is bit-identical to the retained one.
fn subtree_unchanged(
    child: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    child_style: &nana_ui_core::LayoutStyle,
    scope: Option<&ScopeContext<'_>>,
) -> bool {
    let Some(scope) = scope else {
        return false;
    };
    if scope.affected.contains(&child) {
        return false;
    }
    let Some(cached) = scope.retained.boxes.get(&child) else {
        return false;
    };
    let (relative_x, relative_y) =
        child_style.relative_offset_against(Some(containing.width), Some(containing.height));
    cached.x == origin.x + relative_x
        && cached.y == origin.y + relative_y
        && cached.width == size.width
        && cached.height == size.height
}

#[derive(Debug, Clone, Copy, Default)]
struct Point {
    x: f32,
    y: f32,
}

impl Point {
    const ZERO: Self = Self { x: 0.0, y: 0.0 };
}

#[derive(Debug, Clone, Copy, Default)]
struct Size {
    width: f32,
    height: f32,
}

type IntrinsicCache = HashMap<(StableNodeId, u32, u32), Size>;

impl Size {
    fn new(width: f32, height: f32) -> Self {
        Self {
            width: finite_extent(width),
            height: finite_extent(height),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn intrinsic_size(
    id: StableNodeId,
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    nodes: &HashMap<StableNodeId, LayoutInput>,
    cache: &mut IntrinsicCache,
) -> Size {
    intrinsic_size_scoped(id, available, parent_direction, viewport, nodes, cache, None)
}

#[allow(clippy::too_many_arguments)]
fn intrinsic_size_scoped(
    id: StableNodeId,
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    nodes: &HashMap<StableNodeId, LayoutInput>,
    cache: &mut IntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Size {
    let cache_key = (id, available.width.to_bits(), available.height.to_bits());
    if let Some(size) = cache.get(&cache_key) {
        return *size;
    }
    // A subtree outside the affected closure has no change inside it, so its
    // intrinsic size under the same constraints is unchanged.
    if let Some(scope) = scope
        && !scope.affected.contains(&id)
        && let Some(size) = scope.retained.intrinsics.get(&cache_key)
    {
        cache.insert(cache_key, *size);
        return *size;
    }
    let node = &nodes[&id];
    let style = node.style.as_ref();
    if style.omits_box() {
        return Size::default();
    }
    let padding = style.resolved_padding_against(Some(available.width));
    let border = style.resolved_border_width();
    let chrome = Size::new(
        padding.left + padding.right + border * 2.0,
        padding.top + padding.bottom + border * 2.0,
    );
    let content_available = Size::new(
        (available.width - chrome.width).max(0.0),
        (available.height - chrome.height).max(0.0),
    );
    let direction = style.direction.unwrap_or(FlexDirection::Column);
    let flow_children = node
        .children
        .iter()
        .copied()
        .filter(|child| {
            nodes.get(child).is_some_and(|child| {
                !child.style.omits_box() && !child.style.position.is_out_of_flow()
            })
        })
        .collect::<Vec<_>>();
    let child_sizes = flow_children
        .iter()
        .map(|child| {
            intrinsic_size_scoped(
                *child,
                content_available,
                Some(direction),
                viewport,
                nodes,
                cache,
                scope,
            )
        })
        .collect::<Vec<_>>();
    let gap = style.main_gap_against(
        direction,
        nana_ui_core::ParentBox::from_viewport(content_available.width, content_available.height),
    );
    let gaps = gap * flow_children.len().saturating_sub(1) as f32;
    let children = match direction {
        FlexDirection::Row => Size::new(
            child_sizes.iter().map(|size| size.width).sum::<f32>() + gaps,
            child_sizes
                .iter()
                .map(|size| size.height)
                .fold(0.0, f32::max),
        ),
        FlexDirection::Column => Size::new(
            child_sizes
                .iter()
                .map(|size| size.width)
                .fold(0.0, f32::max),
            child_sizes.iter().map(|size| size.height).sum::<f32>() + gaps,
        ),
    };
    let text = node.text_metrics.unwrap_or_default();
    let content = Size::new(
        children.width.max(text.width),
        children.height.max(text.height),
    );
    // Auto width is max-content. Only unconstrained roots fill `available.width`.
    let default_width = if parent_direction.is_none()
        && style.width != Some(LengthSpec::Shrink)
        && !flow_children.is_empty()
    {
        available.width
    } else {
        content.width + chrome.width
    };
    let default_height = content.height + chrome.height;
    let mut width = resolve_axis(style.width, available.width, viewport)
        .unwrap_or(default_width)
        .max(style.resolved_min_width(
            Some(available.width),
            Some((viewport.width, viewport.height)),
        ));
    let mut height = resolve_axis(style.height, available.height, viewport)
        .unwrap_or(default_height)
        .max(style.resolved_min_height(
            Some(available.height),
            Some((viewport.width, viewport.height)),
        ));
    if matches!(style.box_sizing, BoxSizing::ContentBox) {
        if style.width.is_some_and(LengthSpec::is_definite_declared) {
            width += chrome.width;
        }
        if style.height.is_some_and(LengthSpec::is_definite_declared) {
            height += chrome.height;
        }
    }
    if let Some(max) = style.resolved_max_width(
        Some(available.width),
        Some((viewport.width, viewport.height)),
    ) {
        width = width.min(max);
    }
    if let Some(max) = style.resolved_max_height(
        Some(available.height),
        Some((viewport.width, viewport.height)),
    ) {
        height = height.min(max);
    }
    let size = Size::new(width, height);
    cache.insert(cache_key, size);
    size
}

#[allow(clippy::too_many_arguments)]
fn place_node(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    nodes: &HashMap<StableNodeId, LayoutInput>,
    intrinsic: &mut IntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
) {
    place_node_scoped(
        id,
        origin,
        size,
        containing,
        viewport,
        nodes,
        intrinsic,
        output,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn place_node_scoped(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    nodes: &HashMap<StableNodeId, LayoutInput>,
    intrinsic: &mut IntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: Option<&ScopeContext<'_>>,
) {
    let node = &nodes[&id];
    let style = node.style.as_ref();
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
        return;
    }
    let (relative_x, relative_y) =
        style.relative_offset_against(Some(containing.width), Some(containing.height));
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

    if let Some(modal) = node.modal.as_ref() {
        place_modal_children(
            id, origin, size, modal, viewport, nodes, intrinsic, output, scope,
        );
        return;
    }

    let padding = style.resolved_padding_against(Some(size.width));
    let border = style.resolved_border_width();
    let content_origin = Point {
        x: origin.x + border + padding.left,
        y: origin.y + border + padding.top,
    };
    let content = Size::new(
        size.width - padding.left - padding.right - border * 2.0,
        size.height - padding.top - padding.bottom - border * 2.0,
    );
    let direction = style.direction.unwrap_or(FlexDirection::Column);
    let mut children = node
        .children
        .iter()
        .copied()
        .filter(|child| {
            nodes
                .get(child)
                .is_some_and(|child| !child.style.omits_box())
        })
        .collect::<Vec<_>>();
    children.sort_by_key(|child| nodes[child].style.order);
    if style.flex_reverse {
        children.reverse();
    }
    let (flow, positioned): (Vec<_>, Vec<_>) = children
        .into_iter()
        .partition(|child| !nodes[child].style.position.is_out_of_flow());
    let gap = style.main_gap_against(
        direction,
        nana_ui_core::ParentBox::from_viewport(content.width, content.height),
    );
    let mut child_sizes = flow
        .iter()
        .map(|child| {
            intrinsic_size_scoped(*child, content, Some(direction), viewport, nodes, intrinsic, scope)
        })
        .collect::<Vec<_>>();
    distribute_fill(&flow, &mut child_sizes, direction, content, gap, nodes);
    let occupied = main_occupied(&flow, &child_sizes, direction, content, gap, nodes);
    let (mut cursor, effective_gap) = justify_offsets(
        style.justify_content,
        main_extent(content, direction),
        occupied,
        gap,
        flow.len(),
    );
    for (child, mut child_size) in flow.into_iter().zip(child_sizes) {
        let child_style = nodes[&child].style.as_ref();
        let margin = child_style.resolved_margin_against(Some(content.width));
        let align = child_style.align_self.unwrap_or(style.align_items);
        let cross_available = cross_extent(content, direction) - cross_margin(margin, direction);
        if align == AlignSpec::Stretch && !cross_axis_is_definite(child_style, direction) {
            set_cross_extent(&mut child_size, direction, cross_available.max(0.0));
        }
        let cross_offset = match align {
            AlignSpec::Start | AlignSpec::Stretch => cross_start_margin(margin, direction),
            AlignSpec::Center => {
                ((cross_extent(content, direction) - cross_extent(child_size, direction)) / 2.0)
                    .max(0.0)
            }
            AlignSpec::End => (cross_extent(content, direction)
                - cross_extent(child_size, direction)
                - cross_end_margin(margin, direction))
            .max(0.0),
        };
        let main_start = cursor + main_start_margin(margin, direction);
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
        if !subtree_unchanged(child, child_origin, child_size, content, child_style, scope) {
            place_node_scoped(
                child,
                child_origin,
                child_size,
                content,
                viewport,
                nodes,
                intrinsic,
                output,
                scope,
            );
        }
        cursor += main_extent(child_size, direction)
            + main_start_margin(margin, direction)
            + main_end_margin(margin, direction)
            + effective_gap;
    }
    for child in positioned {
        let child_style = nodes[&child].style.as_ref();
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
        let child_size =
            intrinsic_size_scoped(child, base, None, viewport, nodes, intrinsic, scope);
        let left = nana_ui_core::LayoutStyle::resolve_inset(child_style.offset_left, base.width);
        let right = nana_ui_core::LayoutStyle::resolve_inset(child_style.offset_right, base.width);
        let top = nana_ui_core::LayoutStyle::resolve_inset(child_style.offset_top, base.height);
        let bottom =
            nana_ui_core::LayoutStyle::resolve_inset(child_style.offset_bottom, base.height);
        let child_origin = Point {
            x: base_origin.x
                + left.unwrap_or_else(|| right.map_or(0.0, |v| base.width - v - child_size.width)),
            y: base_origin.y
                + top
                    .unwrap_or_else(|| bottom.map_or(0.0, |v| base.height - v - child_size.height)),
        };
        if !subtree_unchanged(child, child_origin, child_size, base, child_style, scope) {
            place_node_scoped(
                child,
                child_origin,
                child_size,
                base,
                viewport,
                nodes,
                intrinsic,
                output,
                scope,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn place_modal_children(
    _id: StableNodeId,
    origin: Point,
    size: Size,
    modal: &crate::ModalLayoutInput,
    viewport: LayoutViewport,
    nodes: &HashMap<StableNodeId, LayoutInput>,
    intrinsic: &mut IntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    _scope: Option<&ScopeContext<'_>>,
) {
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
            let body_slot =
                modal
                    .slots
                    .body
                    .filter(|id| nodes.contains_key(id))
                    .map_or(0.0, |id| {
                        intrinsic_size(
                            id,
                            body_available,
                            Some(FlexDirection::Column),
                            viewport,
                            nodes,
                            intrinsic,
                        )
                        .height
                        .min(body_available.height)
                    });
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
    if let Some(id) = modal.slots.body.filter(|id| nodes.contains_key(id)) {
        place_node(
            id,
            Point {
                x: body.x,
                y: slot_y,
            },
            Size::new(body.width, slot_height),
            Size::new(body.width, slot_height),
            viewport,
            nodes,
            intrinsic,
            output,
        );
    }
    if let Some(id) = modal.slots.close_action.filter(|id| nodes.contains_key(id)) {
        let close = chrome.close_box(surface, modal.kind);
        place_node(
            id,
            Point {
                x: close.x,
                y: close.y,
            },
            Size::new(close.width, close.height),
            Size::new(close.width, close.height),
            viewport,
            nodes,
            intrinsic,
            output,
        );
    }
    let footer_y = surface.y + surface.height - chrome.footer_height;
    let action_band = match modal.kind {
        crate::ModalSurfaceKind::Drawer(_) => crate::overlay_surfaces::DRAWER_FOOTER_PAD_Y,
        _ => 0.0,
    };
    let mut action_right = surface.x + surface.width - chrome.pad_x;
    for id in modal
        .slots
        .actions
        .iter()
        .rev()
        .copied()
        .filter(|id| nodes.contains_key(id))
    {
        let measured = intrinsic_size(
            id,
            Size::new(body.width, crate::overlay_surfaces::MODAL_ACTION_HEIGHT),
            Some(FlexDirection::Row),
            viewport,
            nodes,
            intrinsic,
        );
        let action_size = Size::new(
            measured.width.min(body.width),
            measured
                .height
                .min(crate::overlay_surfaces::MODAL_ACTION_HEIGHT),
        );
        action_right -= action_size.width;
        place_node(
            id,
            Point {
                x: action_right,
                y: footer_y + action_band,
            },
            action_size,
            Size::new(body.width, chrome.footer_height),
            viewport,
            nodes,
            intrinsic,
            output,
        );
        action_right -= crate::overlay_surfaces::MODAL_ACTION_GAP;
    }
    if let Some(id) = modal.slots.footer.filter(|id| nodes.contains_key(id)) {
        let width = (action_right - (surface.x + chrome.pad_x)).max(0.0);
        place_node(
            id,
            Point {
                x: surface.x + chrome.pad_x,
                y: footer_y,
            },
            Size::new(width, chrome.footer_height),
            Size::new(width, chrome.footer_height),
            viewport,
            nodes,
            intrinsic,
            output,
        );
    }
}

fn distribute_fill(
    children: &[StableNodeId],
    sizes: &mut [Size],
    direction: FlexDirection,
    content: Size,
    gap: f32,
    nodes: &HashMap<StableNodeId, LayoutInput>,
) {
    let total_gap = gap * children.len().saturating_sub(1) as f32;
    let fixed = children
        .iter()
        .zip(sizes.iter())
        .map(|(id, size)| {
            let margin = nodes[id].style.resolved_margin_against(Some(content.width));
            (if child_fills(nodes[id].style.as_ref(), direction) {
                0.0
            } else {
                main_extent(*size, direction)
            }) + main_start_margin(margin, direction)
                + main_end_margin(margin, direction)
        })
        .sum::<f32>();
    let weights = children
        .iter()
        .filter_map(|id| {
            let style = nodes[id].style.as_ref();
            child_fills(style, direction).then_some(style.flex_grow.unwrap_or(1.0).max(0.0))
        })
        .sum::<f32>();
    if weights <= 0.0 {
        return;
    }
    let remaining = (main_extent(content, direction) - fixed - total_gap).max(0.0);
    for (id, size) in children.iter().zip(sizes.iter_mut()) {
        let style = nodes[id].style.as_ref();
        if child_fills(style, direction) {
            let weight = style.flex_grow.unwrap_or(1.0).max(0.0);
            set_main_extent(size, direction, remaining * weight / weights);
        }
    }
}

fn main_occupied(
    children: &[StableNodeId],
    sizes: &[Size],
    direction: FlexDirection,
    content: Size,
    gap: f32,
    nodes: &HashMap<StableNodeId, LayoutInput>,
) -> f32 {
    children
        .iter()
        .zip(sizes)
        .map(|(id, size)| {
            let margin = nodes[id].style.resolved_margin_against(Some(content.width));
            main_extent(*size, direction)
                + main_start_margin(margin, direction)
                + main_end_margin(margin, direction)
        })
        .sum::<f32>()
        + gap * children.len().saturating_sub(1) as f32
}

fn justify_offsets(
    justify: JustifySpec,
    available: f32,
    occupied: f32,
    base_gap: f32,
    count: usize,
) -> (f32, f32) {
    let free = (available - occupied).max(0.0);
    match justify {
        JustifySpec::Start => (0.0, base_gap),
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

fn resolve_axis(spec: Option<LengthSpec>, base: f32, viewport: LayoutViewport) -> Option<f32> {
    spec.and_then(|value| {
        if value == LengthSpec::Fill {
            Some(base)
        } else {
            value.resolve_non_negative(Some(base), Some((viewport.width, viewport.height)))
        }
    })
}

fn child_fills(style: &nana_ui_core::LayoutStyle, direction: FlexDirection) -> bool {
    matches!(style.child_main_length(direction), Some(LengthSpec::Fill))
}

fn cross_axis_is_definite(style: &nana_ui_core::LayoutStyle, direction: FlexDirection) -> bool {
    match direction {
        FlexDirection::Row => style.height.is_some(),
        FlexDirection::Column => style.width.is_some(),
    }
}

fn main_extent(size: Size, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => size.width,
        FlexDirection::Column => size.height,
    }
}

fn cross_extent(size: Size, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => size.height,
        FlexDirection::Column => size.width,
    }
}

fn set_main_extent(size: &mut Size, direction: FlexDirection, value: f32) {
    match direction {
        FlexDirection::Row => size.width = finite_extent(value),
        FlexDirection::Column => size.height = finite_extent(value),
    }
}

fn set_cross_extent(size: &mut Size, direction: FlexDirection, value: f32) {
    match direction {
        FlexDirection::Row => size.height = finite_extent(value),
        FlexDirection::Column => size.width = finite_extent(value),
    }
}

fn main_start_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.left,
        FlexDirection::Column => margin.top,
    }
}

fn main_end_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.right,
        FlexDirection::Column => margin.bottom,
    }
}

fn cross_start_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.top,
        FlexDirection::Column => margin.left,
    }
}

fn cross_end_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.bottom,
        FlexDirection::Column => margin.right,
    }
}

fn cross_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    cross_start_margin(margin, direction) + cross_end_margin(margin, direction)
}

fn finite_extent(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nana_ui_core::{FlexDirection, JustifySpec, LayoutStyle, LengthSpec};

    use crate::{
        ComputedStyle, MutationQueue, NodeKind, NodeStyle, TextContent, TextMetrics, TextShaper,
        UiWorld,
    };

    use super::*;

    fn id(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    /// Column of `rows` fixed-height rows, each with one fixed-height label,
    /// under document(1) → column(2). Row `r` is id(3 + r*2), label id(4 + r*2).
    fn column_tree(rows: u64) -> (UiWorld, DocumentId) {
        let document = DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document, NodeKind::Document);
        queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(1), id(2), None);
        queue.set_style(
            id(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(300.0)),
                    height: Some(LengthSpec::Fill),
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        for row in 0..rows {
            let row_id = id(3 + row * 2);
            let label_id = id(4 + row * 2);
            queue.create(row_id, document, NodeKind::Element { tag: "div".into() });
            queue.create(label_id, document, NodeKind::Text);
            queue.insert(id(2), row_id, None);
            queue.insert(row_id, label_id, None);
            queue.set_text(label_id, TextContent { value: "行".into() });
            queue.set_style(
                row_id,
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(300.0)),
                        height: Some(LengthSpec::Px(20.0)),
                        direction: Some(FlexDirection::Row),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
            queue.set_style(
                label_id,
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(40.0)),
                        height: Some(LengthSpec::Px(20.0)),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
        }
        world.commit(queue).unwrap();
        (world, document)
    }

    fn resize_row(world: &mut UiWorld, row: u64, height: f32) {
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(3 + row * 2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(300.0)),
                    height: Some(LengthSpec::Px(height)),
                    direction: Some(FlexDirection::Row),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
    }

    fn full_boxes(
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
    ) -> HashMap<StableNodeId, LayoutBox> {
        RuntimeLayoutEngine
            .layout_document(world, document, viewport)
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>()
    }

    #[test]
    fn scoped_layout_touches_only_the_change_closure_and_matches_full_recompute() {
        let (mut world, document) = column_tree(400);
        let viewport = LayoutViewport::new(300.0, 800.0);
        let mut retained = RetainedLayoutCache::default();

        // Production drains dirty work before layout; the create-time marks
        // must not leak into the scoped measurement below.
        let _ = world.take_system_work();

        // Bootstrap: full pass populates the retained cache with every box.
        let emitted = RuntimeLayoutEngine
            .layout_document_scoped(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        assert_eq!(emitted.len(), 802, "full pass emits every node");

        // Change the LAST row: nothing shifts above it, so the scoped pass
        // must recompute only that row's ancestor chain.
        resize_row(&mut world, 399, 26.0);
        let work = world.take_system_work();
        assert!(!work.layout.is_empty());
        let emitted = RuntimeLayoutEngine
            .layout_document_scoped(
                &world,
                document,
                viewport,
                &work.layout,
                &mut retained,
                false,
            )
            .unwrap();
        assert!(
            emitted.len() < 16,
            "tail row change must stay O(depth), not relayout {} nodes",
            emitted.len()
        );
        for (node, box_) in full_boxes(&world, document, viewport) {
            assert_eq!(
                retained.boxes.get(&node),
                Some(&box_),
                "scoped layout diverged from full recompute at {node:?}"
            );
        }

        // Change a MIDDLE row: every row below shifts; the scoped pass must
        // emit exactly the shifted set (rows and their labels) and still
        // match a full recompute. Rows above stay pruned.
        resize_row(&mut world, 200, 32.0);
        let work = world.take_system_work();
        let emitted = RuntimeLayoutEngine
            .layout_document_scoped(
                &world,
                document,
                viewport,
                &work.layout,
                &mut retained,
                false,
            )
            .unwrap();
        assert!(emitted.len() > 16, "shifted rows must be re-emitted");
        // 199 shifted rows + labels + the change closure; the 400 nodes above
        // the change must stay pruned (well under the 802-node document).
        assert!(
            emitted.len() < 420,
            "rows above the change stay pruned, got {} of 802",
            emitted.len()
        );
        for (node, box_) in full_boxes(&world, document, viewport) {
            assert_eq!(
                retained.boxes.get(&node),
                Some(&box_),
                "shifted scoped layout diverged from full recompute at {node:?}"
            );
        }
    }


    #[test]
    fn lays_out_shaped_controls_without_application_geometry() {
        let document = DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document, NodeKind::Document);
        queue.create(
            id(2),
            document,
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        queue.insert(id(1), id(2), None);
        queue.set_style(
            id(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Fill),
                    direction: Some(FlexDirection::Column),
                    padding: Some(LengthSpec::Px(12.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    padding_left: Some(LengthSpec::Px(8.0)),
                    padding_right: Some(LengthSpec::Px(8.0)),
                    min_height: Some(LengthSpec::Px(32.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_text(
            id(2),
            TextContent {
                value: "Build".into(),
            },
        );
        world.commit(queue).unwrap();
        struct FixedShaper;
        impl TextShaper for FixedShaper {
            fn shape(
                &mut self,
                _id: StableNodeId,
                _text: &TextContent,
                _style: &ComputedStyle,
                _constraints: crate::TextShapeConstraints,
            ) -> TextMetrics {
                TextMetrics {
                    width: 40.0,
                    height: 18.0,
                }
            }
        }
        world.shape_text(&[id(2)], &mut FixedShaper).unwrap();

        let layouts = RuntimeLayoutEngine
            .layout_document(&world, document, LayoutViewport::new(320.0, 180.0))
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(layouts[&id(1)].width, 320.0);
        assert_eq!(layouts[&id(2)].x, 12.0);
        assert_eq!(layouts[&id(2)].y, 12.0);
        assert_eq!(layouts[&id(2)].width, 56.0);
        assert_eq!(layouts[&id(2)].height, 32.0);
    }

    #[test]
    fn display_none_child_does_not_take_a_gap_slot() {
        let document = DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document, NodeKind::Document);
        for value in 2..=4 {
            queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
            queue.insert(id(1), id(value), None);
        }
        queue.set_style(
            id(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(200.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    direction: Some(FlexDirection::Row),
                    gap: Some(LengthSpec::Px(10.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        for value in [2, 4] {
            queue.set_style(
                id(value),
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(50.0)),
                        height: Some(LengthSpec::Px(40.0)),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
        }
        queue.set_style(
            id(3),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    display: Some(nana_ui_core::DisplaySpec::None),
                    width: Some(LengthSpec::Px(50.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let layouts = RuntimeLayoutEngine
            .layout_document(&world, document, LayoutViewport::new(200.0, 40.0))
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(layouts[&id(3)].width, 0.0);
        assert_eq!(layouts[&id(3)].height, 0.0);
        assert_eq!(layouts[&id(2)].x, 0.0);
        assert_eq!(layouts[&id(4)].x, 60.0);
    }

    #[test]
    fn row_fill_uses_remaining_content_width() {
        let document = DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document, NodeKind::Document);
        for value in 2..=3 {
            queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
            queue.insert(id(1), id(value), None);
        }
        queue.set_style(
            id(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(300.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    direction: Some(FlexDirection::Row),
                    gap: Some(LengthSpec::Px(10.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(50.0)),
                    height: Some(LengthSpec::Fill),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(3),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Fill),
                    height: Some(LengthSpec::Fill),
                    margin_left: Some(LengthSpec::Px(10.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let layouts = RuntimeLayoutEngine
            .layout_document(&world, document, LayoutViewport::new(300.0, 40.0))
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(layouts[&id(2)].width, 50.0);
        assert_eq!(layouts[&id(3)].x, 70.0);
        assert_eq!(layouts[&id(3)].width, 230.0);
    }

    #[test]
    fn row_space_between_auto_children_keep_the_trailing_control_inside() {
        let document = DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document, NodeKind::Document);
        for value in 2..=5 {
            queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
        }
        queue.create(id(6), document, NodeKind::Text);
        queue.insert(id(1), id(2), None);
        queue.insert(id(2), id(3), None);
        queue.insert(id(2), id(5), None);
        queue.insert(id(3), id(4), None);
        queue.insert(id(4), id(6), None);
        queue.set_style(
            id(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Fill),
                    direction: Some(FlexDirection::Column),
                    padding: Some(LengthSpec::Px(20.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    direction: Some(FlexDirection::Row),
                    justify_content: JustifySpec::SpaceBetween,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(4),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    height: Some(LengthSpec::Px(16.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(5),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    padding_left: Some(LengthSpec::Px(8.0)),
                    padding_right: Some(LengthSpec::Px(8.0)),
                    min_height: Some(LengthSpec::Px(32.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_text(
            id(6),
            TextContent {
                value: "Title".into(),
            },
        );
        queue.set_text(
            id(5),
            TextContent {
                value: "Open".into(),
            },
        );
        world.commit(queue).unwrap();
        struct FixedShaper;
        impl TextShaper for FixedShaper {
            fn shape(
                &mut self,
                id: StableNodeId,
                _text: &TextContent,
                _style: &ComputedStyle,
                _constraints: crate::TextShapeConstraints,
            ) -> TextMetrics {
                if id.get() == 6 {
                    TextMetrics {
                        width: 180.0,
                        height: 16.0,
                    }
                } else {
                    TextMetrics {
                        width: 74.0,
                        height: 16.0,
                    }
                }
            }
        }
        world.shape_text(&[id(6), id(5)], &mut FixedShaper).unwrap();

        let viewport = LayoutViewport::new(400.0, 200.0);
        let layouts = RuntimeLayoutEngine
            .layout_document(&world, document, viewport)
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();
        let trailing = layouts[&id(5)];
        assert!(
            trailing.width > 0.0 && trailing.height > 0.0,
            "trailing control must be hittable, got {trailing:?}"
        );
        assert!(
            trailing.x >= 0.0 && trailing.x + trailing.width <= viewport.width + 0.5,
            "space-between must not push the trailing control outside the viewport, got {trailing:?} viewport={}",
            viewport.width
        );
        assert!(
            layouts[&id(3)].width < layouts[&id(2)].width,
            "auto-width row cluster must shrink instead of eating the header"
        );
        assert!(
            layouts[&id(4)].width < layouts[&id(2)].width,
            "nested auto-width heading must not fill the header, got {:?}",
            layouts[&id(4)]
        );
    }

    #[test]
    fn absolute_panel_children_resolve_fill_against_the_panel_content_box() {
        let document = DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for value in 1..=3 {
            queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
        }
        queue.insert(id(1), id(2), None);
        queue.insert(id(2), id(3), None);
        queue.set_style(
            id(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Fill),
                    height: Some(LengthSpec::Fill),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    position: PositionSpec::Absolute,
                    offset_left: Some(LengthSpec::Px(8.0)),
                    width: Some(LengthSpec::Px(280.0)),
                    height: Some(LengthSpec::Px(200.0)),
                    padding: Some(LengthSpec::Px(8.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(3),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Fill),
                    height: Some(LengthSpec::Px(32.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();

        let layouts = RuntimeLayoutEngine
            .layout_document(&world, document, LayoutViewport::new(1280.0, 900.0))
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();

        assert_eq!(layouts[&id(2)].width, 280.0);
        assert_eq!(layouts[&id(3)].x, 16.0);
        assert_eq!(layouts[&id(3)].width, 264.0);
    }

    #[test]
    fn fixed_content_shrink_accounts_for_flow_chrome_nesting_and_constraints() {
        let document = DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document, NodeKind::Document);
        for value in 2..=15 {
            queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
        }

        for child in [id(2), id(6), id(9), id(13), id(15)] {
            queue.insert(id(1), child, None);
        }
        for child in [id(3), id(4), id(5)] {
            queue.insert(id(2), child, None);
        }
        for child in [id(7), id(8)] {
            queue.insert(id(6), child, None);
        }
        for child in [id(10), id(12)] {
            queue.insert(id(9), child, None);
        }
        queue.insert(id(10), id(11), None);
        queue.insert(id(13), id(14), None);

        queue.set_style(
            id(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Fill),
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Shrink),
                    direction: Some(FlexDirection::Row),
                    gap: Some(LengthSpec::Px(3.0)),
                    padding: Some(LengthSpec::Px(2.0)),
                    border_width: Some(1.0),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        for (node, width) in [(id(3), 20.0), (id(4), 30.0)] {
            queue.set_style(
                node,
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(width)),
                        height: Some(LengthSpec::Px(8.0)),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
        }
        queue.set_style(
            id(5),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    position: PositionSpec::Absolute,
                    width: Some(LengthSpec::Px(200.0)),
                    height: Some(LengthSpec::Px(8.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(6),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Shrink),
                    direction: Some(FlexDirection::Column),
                    padding: Some(LengthSpec::Px(1.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        for (node, width) in [(id(7), 40.0), (id(8), 25.0)] {
            queue.set_style(
                node,
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(width)),
                        height: Some(LengthSpec::Px(8.0)),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
        }
        queue.set_style(
            id(9),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Shrink),
                    direction: Some(FlexDirection::Row),
                    gap: Some(LengthSpec::Px(2.0)),
                    padding: Some(LengthSpec::Px(1.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(10),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Shrink),
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        for (node, width) in [(id(11), 35.0), (id(12), 10.0)] {
            queue.set_style(
                node,
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(width)),
                        height: Some(LengthSpec::Px(8.0)),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
        }
        queue.set_style(
            id(13),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Shrink),
                    min_width: Some(LengthSpec::Px(50.0)),
                    max_width: Some(LengthSpec::Px(55.0)),
                    padding: Some(LengthSpec::Px(2.0)),
                    border_width: Some(1.0),
                    box_sizing: BoxSizing::ContentBox,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(14),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(20.0)),
                    height: Some(LengthSpec::Px(8.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            id(15),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Shrink),
                    max_width: Some(LengthSpec::Px(60.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        queue.set_text(
            id(15),
            TextContent {
                value: "wide".into(),
            },
        );
        world.commit(queue).unwrap();

        struct WideText;
        impl TextShaper for WideText {
            fn shape(
                &mut self,
                _id: StableNodeId,
                _text: &TextContent,
                _style: &ComputedStyle,
                _constraints: crate::TextShapeConstraints,
            ) -> TextMetrics {
                TextMetrics {
                    width: 100.0,
                    height: 8.0,
                }
            }
        }
        world.shape_text(&[id(15)], &mut WideText).unwrap();

        let layout_at = |width| {
            RuntimeLayoutEngine
                .layout_document(&world, document, LayoutViewport::new(width, 240.0))
                .unwrap()
                .into_iter()
                .collect::<HashMap<_, _>>()
        };
        let narrow = layout_at(320.0);
        let wide = layout_at(640.0);

        for layouts in [&narrow, &wide] {
            assert_eq!(layouts[&id(2)].width, 59.0);
            assert_eq!(layouts[&id(6)].width, 42.0);
            assert_eq!(layouts[&id(9)].width, 49.0);
            assert_eq!(layouts[&id(10)].width, 35.0);
            assert_eq!(layouts[&id(13)].width, 50.0);
            assert_eq!(layouts[&id(15)].width, 60.0);
        }
        for node in [id(2), id(6), id(9), id(10), id(13), id(15)] {
            assert_eq!(narrow[&node].width, wide[&node].width);
        }
    }
}
