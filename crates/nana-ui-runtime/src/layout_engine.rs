use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nana_ui_core::box_layout::text_line_box_height_px;
use nana_ui_core::{
    AlignSpec, BoxSizing, ClearSpec, DisplaySpec, FlexDirection, FlexWrap, FloatSpec,
    FontSizeContext, GridAutoFlow, GridLine, GridPlacement, GridRepeatAuto, GridTemplateAreas,
    GridTrack, JustifySpec, LayoutStyle, LengthSpec, PositionSpec, resolve_grid_track_sizes,
};

use crate::{
    DocumentId, LayoutBox, LayoutInput, MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld,
    UiWorldError,
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

/// Backend-neutral layout owner used by canonical Runtime applications.
///
/// Consumes the same `LayoutStyle` and shaped text metrics stored in `UiWorld`
/// (flex wrap / `display:grid` 2D tracks·repeat·areas·placement via
/// `uses_2d_grid` / percent / calc / absolute / fixed / float / IFC subset
/// including shrink-to-avoid-float line boxes beside sibling floats)
/// and returns atomic layout writeback. Vue `measure_layout` and css-parity
/// call [`Self::layout_style_tree`] so mixed trees and fixtures share this
/// algorithm.
#[derive(Debug, Default, Clone, Copy)]
pub struct RuntimeLayoutEngine;

/// Style-only tree accepted by [`RuntimeLayoutEngine::layout_style_tree`].
///
/// Vue `LayoutNode` and css-parity fixtures adapt onto this type; they do not
/// keep a second layout algorithm.
#[derive(Debug, Clone, Default)]
pub struct StyleLayoutNode {
    pub id: String,
    pub style: LayoutStyle,
    pub children: Vec<StyleLayoutNode>,
    pub text: Option<String>,
}

impl RuntimeLayoutEngine {
    pub fn layout_document(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
    ) -> Result<Vec<(StableNodeId, LayoutBox)>, UiWorldError> {
        let order = world.document_order(document);
        let mut nodes = LayoutInputMap::new(world);
        nodes.prefetch(&order)?;
        let roots = world.document_roots(document);
        let mut output = HashMap::with_capacity(nodes.len());
        let mut intrinsic = HashMap::with_capacity(nodes.len());
        let available = Size::new(viewport.width, viewport.height);
        for root in roots {
            let root_size = intrinsic_size(
                root,
                available,
                None,
                viewport,
                ROOT_FONT_PX,
                &mut nodes,
                &mut intrinsic,
            )?;
            place_node(
                root,
                Point::ZERO,
                root_size,
                available,
                viewport,
                ROOT_FONT_PX,
                &mut nodes,
                &mut intrinsic,
                &mut output,
            )?;
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
        let mut nodes = LayoutInputMap::new(world);
        if force_full {
            let order = world.document_order(document);
            nodes.prefetch(&order)?;
        }
        let mut affected = HashSet::new();
        if !force_full {
            for &id in dirty {
                if !world.contains(id) {
                    continue;
                }
                let mut cursor = Some(id);
                while let Some(id) = cursor {
                    if !affected.insert(id) {
                        break;
                    }
                    cursor = world.parent_id(id);
                }
            }
        }
        let scope = ScopeContext {
            affected: &affected,
            retained: &*retained,
        };
        let scope_ref = (!force_full).then_some(&scope);
        let roots = world.document_roots(document);
        let mut output = HashMap::with_capacity(nodes.len());
        let mut intrinsic = HashMap::with_capacity(nodes.len());
        let available = Size::new(viewport.width, viewport.height);
        for root in roots {
            let root_size = intrinsic_size_scoped(
                root,
                available,
                None,
                viewport,
                ROOT_FONT_PX,
                &mut nodes,
                &mut intrinsic,
                scope_ref,
            )?;
            place_node_scoped(
                root,
                Point::ZERO,
                root_size,
                available,
                viewport,
                ROOT_FONT_PX,
                &mut nodes,
                &mut intrinsic,
                &mut output,
                scope_ref,
            )?;
        }
        // Publish recomputed boxes from the placed set; no document_order walk.
        let mut emitted = output.into_iter().collect::<Vec<_>>();
        emitted.sort_unstable_by_key(|(id, _)| *id);
        for (id, box_) in &emitted {
            retained.boxes.insert(*id, *box_);
        }
        retained.intrinsics.extend(intrinsic);
        retained.materialized_inputs = nodes.materialized;
        // Despawned ids linger in the retained maps; keep them bounded.
        // Scoped passes only materialize a subset, so membership is the live
        // world, not the partial input map.
        let universe = if force_full { nodes.len() } else { world.len() };
        if retained.boxes.len() > universe.saturating_mul(2) {
            retained.boxes.retain(|id, _| world.contains(*id));
        }
        if retained.intrinsics.len() > universe.saturating_mul(4) {
            retained
                .intrinsics
                .retain(|(id, _, _), _| world.contains(*id));
        }
        Ok(emitted)
    }

    /// Layout a style tree with the same algorithm as [`Self::layout_document`].
    ///
    /// Hidden / `display:none` nodes are omitted from the result (css-parity /
    /// Vue measure contract). Product `UiWorld` still records a zero box.
    pub fn layout_style_tree(
        self,
        root: &StyleLayoutNode,
        viewport: LayoutViewport,
    ) -> Vec<(String, LayoutBox)> {
        let document = DocumentId::new(1).expect("document 1 is nonzero");
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        let mut names = HashMap::new();
        let mut omitted = HashSet::new();
        let mut next = 1u64;
        fn add(
            node: &StyleLayoutNode,
            parent: Option<StableNodeId>,
            parent_omitted: bool,
            document: DocumentId,
            queue: &mut MutationQueue,
            names: &mut HashMap<StableNodeId, String>,
            omitted: &mut HashSet<StableNodeId>,
            next: &mut u64,
        ) -> StableNodeId {
            let id = StableNodeId::new(*next).expect("style-tree ids start at 1");
            *next += 1;
            queue.create(id, document, NodeKind::Element { tag: "div".into() });
            if let Some(parent) = parent {
                queue.insert(parent, id, None);
            }
            // `display:none` / hidden omit self and descendants. `display:contents`
            // omits only self from the name→box map; descendants still layout.
            let omit_descendants = parent_omitted || node.style.omits_box();
            if omit_descendants || !node.style.generates_box() {
                omitted.insert(id);
            }
            queue.set_style(
                id,
                NodeStyle {
                    layout: Arc::new(node.style.clone()),
                    ..NodeStyle::default()
                },
            );
            names.insert(id, node.id.clone());
            if let Some(text) = node.text.as_deref() {
                queue.set_text(id, crate::TextContent { value: text.into() });
            }
            for child in &node.children {
                add(
                    child,
                    Some(id),
                    omit_descendants,
                    document,
                    queue,
                    names,
                    omitted,
                    next,
                );
            }
            id
        }
        add(
            root,
            None,
            false,
            document,
            &mut queue,
            &mut names,
            &mut omitted,
            &mut next,
        );
        world
            .commit(queue)
            .expect("style-tree mutations are well-formed");
        let order = world.document_order(document);
        world
            .resolve_styles(&order)
            .expect("style-tree style resolve is infallible");
        world
            .shape_text(&order, &mut crate::MeasureTextShaper)
            .expect("style-tree text shaping is infallible");
        let layouts = self
            .layout_document(&world, document, viewport)
            .expect("style-tree layout is infallible");
        layouts
            .into_iter()
            .filter_map(|(id, box_)| {
                if omitted.contains(&id) {
                    return None;
                }
                names.get(&id).cloned().map(|name| (name, box_))
            })
            .collect()
    }
}

/// Cross-frame layout memo for scoped relayout: last published boxes and
/// intrinsic sizes keyed like the per-pass intrinsic cache.
#[derive(Default)]
pub struct RetainedLayoutCache {
    intrinsics: HashMap<(StableNodeId, u32, u32), Size>,
    boxes: HashMap<StableNodeId, LayoutBox>,
    materialized_inputs: usize,
}

impl RetainedLayoutCache {
    fn clear(&mut self) {
        self.intrinsics.clear();
        self.boxes.clear();
        self.materialized_inputs = 0;
    }
}

/// On-demand `LayoutInput` cache. A miss loads exactly that id from `UiWorld`.
struct LayoutInputMap<'a> {
    world: &'a UiWorld,
    nodes: HashMap<StableNodeId, LayoutInput>,
    materialized: usize,
}

impl<'a> LayoutInputMap<'a> {
    fn new(world: &'a UiWorld) -> Self {
        Self {
            world,
            nodes: HashMap::new(),
            materialized: 0,
        }
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }

    fn prefetch(&mut self, ids: &[StableNodeId]) -> Result<(), UiWorldError> {
        let missing = ids
            .iter()
            .copied()
            .filter(|id| !self.nodes.contains_key(id))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(());
        }
        let inputs = self.world.layout_inputs(&missing)?;
        self.materialized = self.materialized.saturating_add(inputs.len());
        self.nodes
            .extend(inputs.into_iter().map(|input| (input.id, input)));
        Ok(())
    }

    fn get(&mut self, id: StableNodeId) -> Result<Option<&LayoutInput>, UiWorldError> {
        if !self.load(id)? {
            return Ok(None);
        }
        Ok(self.nodes.get(&id))
    }

    /// Style for classifying / measuring siblings without assembling `LayoutInput`.
    fn style(&self, id: StableNodeId) -> Option<Arc<nana_ui_core::LayoutStyle>> {
        if let Some(node) = self.nodes.get(&id) {
            return Some(Arc::clone(&node.style));
        }
        self.world.layout_style(id)
    }

    fn load(&mut self, id: StableNodeId) -> Result<bool, UiWorldError> {
        if self.nodes.contains_key(&id) {
            return Ok(true);
        }
        if !self.world.contains(id) {
            return Ok(false);
        }
        let mut batch = self.world.layout_inputs(&[id])?;
        match batch.pop() {
            Some(input) => {
                self.materialized = self.materialized.saturating_add(1);
                self.nodes.insert(id, input);
                Ok(true)
            }
            None => Ok(false),
        }
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
    child_fonts: FontSizeContext,
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
    let (relative_x, relative_y) = child_style.relative_offset_against_fonts(
        Some(containing.width),
        Some(containing.height),
        child_fonts,
    );
    cached.x == origin.x + relative_x
        && cached.y == origin.y + relative_y
        && cached.width == size.width
        && cached.height == size.height
}

fn collect_flow_children(
    children: &[StableNodeId],
    nodes: &mut LayoutInputMap<'_>,
) -> Result<Vec<StableNodeId>, UiWorldError> {
    let mut out = Vec::new();
    collect_flow_children_into(children, nodes, &mut out)?;
    Ok(out)
}

fn collect_flow_children_into(
    children: &[StableNodeId],
    nodes: &mut LayoutInputMap<'_>,
    out: &mut Vec<StableNodeId>,
) -> Result<(), UiWorldError> {
    for child in children.iter().copied() {
        let Some(style) = nodes.style(child) else {
            continue;
        };
        if style.omits_box() {
            continue;
        }
        if style.display.is_some_and(DisplaySpec::is_contents) {
            let nested = match nodes.get(child)? {
                Some(node) => (*node.children).clone(),
                None => continue,
            };
            collect_flow_children_into(&nested, nodes, out)?;
            continue;
        }
        if style.position.is_out_of_flow() {
            continue;
        }
        out.push(child);
    }
    Ok(())
}

fn collect_positioned_children(
    children: &[StableNodeId],
    nodes: &mut LayoutInputMap<'_>,
) -> Result<Vec<StableNodeId>, UiWorldError> {
    let mut out = Vec::new();
    collect_positioned_children_into(children, nodes, &mut out)?;
    Ok(out)
}

fn collect_positioned_children_into(
    children: &[StableNodeId],
    nodes: &mut LayoutInputMap<'_>,
    out: &mut Vec<StableNodeId>,
) -> Result<(), UiWorldError> {
    for child in children.iter().copied() {
        let Some(style) = nodes.style(child) else {
            continue;
        };
        if style.omits_box() {
            continue;
        }
        if style.display.is_some_and(DisplaySpec::is_contents) {
            let nested = match nodes.get(child)? {
                Some(node) => (*node.children).clone(),
                None => continue,
            };
            collect_positioned_children_into(&nested, nodes, out)?;
            continue;
        }
        if style.position.is_out_of_flow() {
            out.push(child);
        }
    }
    Ok(())
}

fn collect_floated_children(
    children: &[StableNodeId],
    nodes: &mut LayoutInputMap<'_>,
) -> Result<Vec<StableNodeId>, UiWorldError> {
    let mut out = Vec::new();
    collect_floated_children_into(children, nodes, &mut out)?;
    Ok(out)
}

fn collect_floated_children_into(
    children: &[StableNodeId],
    nodes: &mut LayoutInputMap<'_>,
    out: &mut Vec<StableNodeId>,
) -> Result<(), UiWorldError> {
    for child in children.iter().copied() {
        let Some(style) = nodes.style(child) else {
            continue;
        };
        if style.omits_box() {
            continue;
        }
        if style.display.is_some_and(DisplaySpec::is_contents) {
            let nested = match nodes.get(child)? {
                Some(node) => (*node.children).clone(),
                None => continue,
            };
            collect_floated_children_into(&nested, nodes, out)?;
            continue;
        }
        if style.is_floated() && !style.position.is_out_of_flow() {
            out.push(child);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct PackedFloat {
    id: StableNodeId,
    origin: Point,
    size: Size,
    side: FloatSpec,
    /// Margin-box left in the same space as [`Self::origin`].
    occupy_x: f32,
    occupy_y: f32,
    occupy_w: f32,
    occupy_h: f32,
}

#[derive(Debug, Default)]
struct PackedFloats {
    items: Vec<PackedFloat>,
    /// Occupied bottom of left floats after pack/wrap, relative to content origin.
    left_bottom: f32,
    /// Occupied bottom of right floats after pack/wrap, relative to content origin.
    right_bottom: f32,
}

impl PackedFloats {
    /// Left/right insets at content-relative `y` (line-box top), from sibling
    /// float margin boxes. Not ancestor intrusion / `shape-outside`.
    fn insets_at_y(&self, content_origin: Point, content_width: f32, y: f32) -> (f32, f32) {
        let abs_y = content_origin.y + y;
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for item in &self.items {
            let top = item.occupy_y;
            let bottom = item.occupy_y + item.occupy_h;
            if abs_y + 0.5 < top || abs_y >= bottom - 0.5 {
                continue;
            }
            match item.side {
                FloatSpec::Right => {
                    let occupy_left = item.occupy_x - content_origin.x;
                    right = right.max((content_width - occupy_left).max(0.0));
                }
                _ => {
                    let occupy_right = item.occupy_x + item.occupy_w - content_origin.x;
                    left = left.max(occupy_right.max(0.0));
                }
            }
        }
        (left, right)
    }

    /// Nearest float margin-box bottom below `y` among floats that occupy `y`.
    fn next_bottom_after(&self, content_origin: Point, y: f32) -> Option<f32> {
        let abs_y = content_origin.y + y;
        let mut best: Option<f32> = None;
        for item in &self.items {
            let top = item.occupy_y;
            let bottom = item.occupy_y + item.occupy_h - content_origin.y;
            if top > abs_y + 0.5 || bottom <= y + 0.5 {
                continue;
            }
            best = Some(best.map_or(bottom, |value| value.min(bottom)));
        }
        best
    }
}

/// One wrap line: child indices plus the shortened IFC line box (full width when
/// not avoiding floats).
#[derive(Debug, Clone)]
struct LineBoxSlot {
    indices: Vec<usize>,
    main_start: f32,
    main_available: f32,
    /// Content-relative cross start after float drop / `clear` (IFC only).
    cross_y: f32,
    pin_cross: bool,
}

/// Geometric same-side pack/wrap. Bottoms are the occupied extent after wrapping,
/// not the pre-pack max of each float's own height (so `clear` clears the second row).
#[allow(clippy::too_many_arguments)]
fn pack_floated_children(
    floated: &[StableNodeId],
    content_origin: Point,
    content: Size,
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<PackedFloats, UiWorldError> {
    let mut items = Vec::with_capacity(floated.len());
    let mut left_cursor_x = content_origin.x;
    let mut left_line_y = content_origin.y;
    let mut left_line_bottom = content_origin.y;
    let mut right_cursor_x = content_origin.x + content.width;
    let mut right_line_y = content_origin.y;
    let mut right_line_bottom = content_origin.y;
    for child in floated {
        let Some(child_style) = nodes.style(*child) else {
            continue;
        };
        let child_style = child_style.as_ref();
        let child_size = intrinsic_size_scoped(
            *child,
            content,
            Some(FlexDirection::Row),
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            scope,
        )?;
        let child_fonts = fonts_of(child_style, child_font_px);
        let margin = child_style.resolved_margin_against_fonts(Some(content.width), child_fonts);
        let outer_w = child_size.width + margin.left + margin.right;
        // A float's own `clear` uses packed bottoms of earlier floats, same
        // contract as in-flow clear (not the pre-pack max of each float).
        let clear_bottom = match child_style.clear {
            ClearSpec::None => None,
            ClearSpec::Left => Some(left_line_bottom),
            ClearSpec::Right => Some(right_line_bottom),
            ClearSpec::Both => Some(left_line_bottom.max(right_line_bottom)),
        };
        let (x, y) = match child_style.float {
            FloatSpec::Right => {
                if let Some(bottom) = clear_bottom
                    && bottom > right_line_y + 0.5
                {
                    right_cursor_x = content_origin.x + content.width;
                    right_line_y = bottom;
                }
                if right_cursor_x < content_origin.x + content.width - 0.5
                    && right_cursor_x - outer_w < content_origin.x - 0.5
                {
                    right_cursor_x = content_origin.x + content.width;
                    right_line_y = right_line_bottom;
                }
                let x = (right_cursor_x - child_size.width - margin.right).max(content_origin.x);
                let y = right_line_y + margin.top;
                right_cursor_x = x - margin.left;
                right_line_bottom = right_line_bottom.max(y + child_size.height + margin.bottom);
                (x, y)
            }
            _ => {
                if let Some(bottom) = clear_bottom
                    && bottom > left_line_y + 0.5
                {
                    left_cursor_x = content_origin.x;
                    left_line_y = bottom;
                }
                if left_cursor_x > content_origin.x + 0.5
                    && left_cursor_x + outer_w > content_origin.x + content.width + 0.5
                {
                    left_cursor_x = content_origin.x;
                    left_line_y = left_line_bottom;
                }
                let x = left_cursor_x + margin.left;
                let y = left_line_y + margin.top;
                left_cursor_x = x + child_size.width + margin.right;
                left_line_bottom = left_line_bottom.max(y + child_size.height + margin.bottom);
                (x, y)
            }
        };
        items.push(PackedFloat {
            id: *child,
            origin: Point { x, y },
            size: child_size,
            side: child_style.float,
            occupy_x: x - margin.left,
            occupy_y: y - margin.top,
            occupy_w: outer_w,
            occupy_h: child_size.height + margin.top + margin.bottom,
        });
    }
    Ok(PackedFloats {
        items,
        left_bottom: (left_line_bottom - content_origin.y).max(0.0),
        right_bottom: (right_line_bottom - content_origin.y).max(0.0),
    })
}

fn sort_by_order(ids: &mut [StableNodeId], nodes: &LayoutInputMap<'_>) {
    ids.sort_by_key(|id| nodes.style(*id).map(|style| style.order).unwrap_or(0));
}

fn uses_2d_grid(style: &LayoutStyle, flow: &[StableNodeId], nodes: &LayoutInputMap<'_>) -> bool {
    if !style.display.is_some_and(DisplaySpec::is_grid_container) {
        return false;
    }
    if style.active_grid_columns().is_some()
        || style.active_grid_rows().is_some()
        || style.grid_columns_repeat.is_some()
        || style.grid_rows_repeat.is_some()
        || style.grid_auto_flow.is_some()
        || style
            .grid_auto_columns
            .as_ref()
            .is_some_and(|tracks| !tracks.is_empty())
        || style
            .grid_auto_rows
            .as_ref()
            .is_some_and(|tracks| !tracks.is_empty())
        || style
            .grid_template_areas
            .as_ref()
            .is_some_and(|areas| !areas.cells.is_empty())
    {
        return true;
    }
    flow.iter().any(|id| {
        nodes
            .style(*id)
            .is_some_and(|child| !child.grid_placement.is_auto())
    })
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
fn intrinsic_size_scoped(
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
    let content_available = Size::new(
        (available.width - chrome.width).max(0.0),
        (available.height - chrome.height).max(0.0),
    );
    let direction = style.direction.unwrap_or(FlexDirection::Column);
    let flow_children = collect_flow_children(&children, nodes)?;
    let mut child_sizes = Vec::with_capacity(flow_children.len());
    let grid_measure = uses_2d_grid(style, &flow_children, nodes);
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
    let wrapping = match direction {
        FlexDirection::Row => matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse),
        FlexDirection::Column => {
            matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse) && content_available.height > 0.5
        }
    };
    let grid_tracks = match direction {
        FlexDirection::Row => style.active_grid_columns(),
        FlexDirection::Column => style.active_grid_rows(),
    };
    let children = if uses_2d_grid(style, &flow_children, nodes) {
        let grid = layout_grid_2d(
            style,
            &flow_children,
            &child_sizes,
            content_available,
            fonts,
            nodes,
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
        match direction {
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
        .map(|size| size.width)
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
    let width_spec = resolve_axis(
        demote_fill_spec_if_indefinite(style.width, available.width),
        available.width,
        viewport,
        fonts,
    );
    let height_spec = resolve_axis(
        demote_fill_spec_if_indefinite(style.height, available.height),
        available.height,
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

#[allow(clippy::too_many_arguments)]
fn place_node(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
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
    )
}

#[allow(clippy::too_many_arguments)]
fn place_node_scoped(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: Option<&ScopeContext<'_>>,
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
    let style = node.style.clone();
    let child_ids = node.children.clone();
    let modal = node.modal.clone();
    let style = style.as_ref();
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
    output.insert(
        id,
        LayoutBox {
            x: origin.x,
            y: origin.y,
            width: size.width,
            height: size.height,
        },
    );

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

    let padding = style.resolved_padding_against_fonts(Some(size.width), fonts);
    let border = style.resolved_border_edges();
    let content_origin = Point {
        x: origin.x + border.left + padding.left,
        y: origin.y + border.top + padding.top,
    };
    let content = Size::new(
        size.width - padding.left - padding.right - border.left - border.right,
        size.height - padding.top - padding.bottom - border.top - border.bottom,
    );
    let mut direction = style.direction.unwrap_or(FlexDirection::Column);
    let mut flow = collect_flow_children(&child_ids, nodes)?;
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
    let packed_floats = if floated.is_empty() {
        PackedFloats::default()
    } else {
        pack_floated_children(
            &floated,
            content_origin,
            content,
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
    if ifc {
        direction = FlexDirection::Row;
    }
    if style.flex_reverse && !grid_2d && !ifc {
        flow.reverse();
        positioned.reverse();
    }
    let parent_box = gap_containing_block(style, content);
    let gap = style.main_gap_against_fonts(direction, parent_box, fonts);
    let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
    let mut child_sizes = Vec::with_capacity(flow.len());
    for child in &flow {
        let child_available = nodes
            .style(*child)
            .filter(|_| grid_2d)
            .map(|child_style| grid_item_measure_available(child_style.as_ref(), content))
            .unwrap_or(content);
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
    if grid_2d {
        let grid = layout_grid_2d(style, &flow, &child_sizes, content, fonts, nodes);
        place_grid_2d_items(
            &grid,
            content_origin,
            content,
            style,
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            output,
            scope,
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
        let mut justify = if ifc {
            style.text_align.to_justify(style.is_rtl())
        } else {
            style.justify_content
        };
        if style.flex_reverse && !ifc {
            justify = flip_justify_for_reverse(justify);
        }
        let full_main = main_extent(content, direction);
        let mut line_slots = if wrapping {
            if ifc {
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
                    full_main,
                    gap,
                    grid_tracks,
                    viewport,
                    child_font_px,
                    nodes,
                    false,
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
                    gap,
                    viewport,
                    child_font_px,
                    nodes,
                );
            }
            let line_cross = line_flow
                .iter()
                .zip(line_sizes.iter())
                .map(|(child, size)| {
                    let margin = nodes
                        .style(*child)
                        .map(|style| {
                            style.resolved_margin_against_fonts(
                                Some(content.width),
                                fonts_of(style.as_ref(), child_font_px),
                            )
                        })
                        .unwrap_or_default();
                    cross_extent(*size, direction) + cross_margin(margin, direction)
                })
                .fold(0.0, f32::max);
            packed.push((
                line_flow,
                line_sizes,
                line_cross,
                slot.main_start,
                slot.main_available,
                slot.cross_y,
                slot.pin_cross,
            ));
        }
        let line_count = packed.len();
        let container_cross = cross_extent(content, direction);
        let (mut cross_cursor, extra_cross_gap) = if line_count > 1 {
            let total = packed
                .iter()
                .map(|(_, _, cross, _, _, _, _)| *cross)
                .sum::<f32>()
                + cross_gap * line_count.saturating_sub(1) as f32;
            if matches!(
                style.align_content,
                JustifySpec::Stretch | JustifySpec::Start
            ) && style.align_content == JustifySpec::Stretch
            {
                let leftover = (container_cross - total).max(0.0);
                let extra = leftover / line_count as f32;
                for packed_line in &mut packed {
                    packed_line.2 += extra;
                }
                (0.0, cross_gap)
            } else {
                justify_offsets(
                    style.align_content,
                    container_cross,
                    total,
                    cross_gap,
                    line_count,
                )
            }
        } else {
            (0.0, cross_gap)
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
                content,
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
            let line_baseline = line_flow
                .iter()
                .filter_map(|id| {
                    nodes
                        .style(*id)
                        .map(|s| s.approximate_baseline(child_font_px))
                })
                .fold(0.0f32, f32::max);
            for (child, mut child_size) in line_flow.into_iter().zip(line_sizes) {
                let Some(child_style) = nodes.style(child) else {
                    continue;
                };
                let child_style = child_style.as_ref();
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
                let mut margin =
                    child_style.resolved_margin_against_fonts(Some(content.width), child_fonts);
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
                    Some(content.width),
                    child_fonts,
                );
                let cross_offset = match align {
                    AlignSpec::Start | AlignSpec::Stretch => {
                        cross_cursor + cross_start_margin(margin, direction)
                    }
                    AlignSpec::Baseline => {
                        let base = child_style.approximate_baseline(child_fonts.element_px);
                        cross_cursor + (line_baseline - base).max(0.0)
                    }
                    AlignSpec::Center => {
                        cross_cursor
                            + ((line_box_cross - cross_extent(child_size, direction)) / 2.0)
                                .max(0.0)
                    }
                    AlignSpec::End => {
                        cross_cursor
                            + (line_box_cross
                                - cross_extent(child_size, direction)
                                - cross_end_margin(margin, direction))
                            .max(0.0)
                    }
                };
                let main_start = line_origin_main + cursor + main_start_margin(margin, direction);
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
                if !subtree_unchanged(
                    child,
                    child_origin,
                    child_size,
                    content,
                    child_style,
                    child_fonts,
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
                    )?;
                }
                cursor += main_extent(child_size, direction)
                    + main_start_margin(margin, direction)
                    + main_end_margin(margin, direction)
                    + effective_gap;
            }
            cross_cursor += line_cross + extra_cross_gap;
        }
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
            )?;
        }
    }
    for child in positioned {
        let Some(child_style) = nodes.style(child) else {
            continue;
        };
        let child_style = child_style.as_ref();
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
        let mut child_size = intrinsic_size_scoped(
            child,
            base,
            None,
            viewport,
            child_font_px,
            nodes,
            intrinsic,
            scope,
        )?;
        let child_fonts = fonts_of(child_style, child_font_px);
        let left =
            LayoutStyle::resolve_inset_fonts(child_style.offset_left, base.width, child_fonts);
        let right =
            LayoutStyle::resolve_inset_fonts(child_style.offset_right, base.width, child_fonts);
        let top =
            LayoutStyle::resolve_inset_fonts(child_style.offset_top, base.height, child_fonts);
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
        if let Some(max) = child_style.resolved_max_height_fonts(Some(base.height), vp, child_fonts)
        {
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
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn place_modal_children(
    _id: StableNodeId,
    origin: Point,
    size: Size,
    modal: &crate::ModalLayoutInput,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
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
fn place_modal_slot(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
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
    )
}

fn gap_containing_block(style: &LayoutStyle, content: Size) -> nana_ui_core::ParentBox {
    // Auto-height wrap: row-gap % falls back to width (T-W05/W06). A Fill/px
    // height is a definite CB and must not use the parent/viewport leftover.
    let height = match style.height {
        None
        | Some(LengthSpec::Auto)
        | Some(LengthSpec::Shrink)
        | Some(LengthSpec::MinContent)
        | Some(LengthSpec::MaxContent)
        | Some(LengthSpec::FitContent) => None,
        Some(LengthSpec::Fill) => Some(content.height).filter(|value| *value > 0.0),
        Some(_) => Some(content.height).filter(|value| *value > 0.0),
    };
    nana_ui_core::ParentBox::new(Some(content.width).filter(|value| *value > 0.0), height)
}

fn flip_justify_for_reverse(justify: JustifySpec) -> JustifySpec {
    match justify {
        JustifySpec::Start => JustifySpec::End,
        JustifySpec::End => JustifySpec::Start,
        other => other,
    }
}

fn demote_fill_spec(spec: Option<LengthSpec>) -> Option<LengthSpec> {
    match spec {
        Some(LengthSpec::Fill) => None,
        Some(LengthSpec::Percent(percent)) if (percent - 100.0).abs() < 0.5 => None,
        Some(LengthSpec::CalcPercentOffset { percent, offset_px })
            if (percent - 100.0).abs() < 0.5 && offset_px <= 0.0 =>
        {
            None
        }
        other => other,
    }
}

/// `100%` / `Fill` against a definite grid CB must not become the auto-track
/// contribution. Measure that axis as indefinite (same as auto tracks).
fn grid_item_measure_available(style: &LayoutStyle, content: Size) -> Size {
    let width = if style.width.is_some() && demote_fill_spec(style.width).is_none() {
        0.0
    } else {
        content.width
    };
    let height = if style.height.is_some() && demote_fill_spec(style.height).is_none() {
        0.0
    } else {
        content.height
    };
    Size::new(width, height)
}

#[allow(clippy::too_many_arguments)]
fn packing_main_size(
    style: &LayoutStyle,
    intrinsic: Size,
    direction: FlexDirection,
    content_main: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    track: Option<GridTrack>,
) -> f32 {
    let spec = style
        .child_main_length(direction)
        .or_else(|| track.map(GridTrack::as_row_main_length));
    let fonts = fonts_of(style, parent_font_px);
    match resolve_child_main(spec, content_main, viewport, fonts) {
        Some(value) => {
            content_box_main_border_size(style, direction, Some(content_main), value, fonts)
        }
        None if style.grows() || matches!(spec, Some(LengthSpec::Fill)) => content_main,
        None => main_extent(intrinsic, direction),
    }
}

/// CSS initial `medium` ≈ 16px. Root `rem` and the em base when no ancestor
/// set `font-size`.
const ROOT_FONT_PX: f32 = 16.0;

fn fonts_of(style: &LayoutStyle, parent_font_px: f32) -> FontSizeContext {
    FontSizeContext::new(ROOT_FONT_PX, style.font_size.unwrap_or(parent_font_px))
}

fn resolve_child_main(
    spec: Option<LengthSpec>,
    percent_base: f32,
    viewport: LayoutViewport,
    fonts: FontSizeContext,
) -> Option<f32> {
    match spec {
        None
        | Some(LengthSpec::Fill)
        | Some(LengthSpec::Shrink)
        | Some(LengthSpec::Auto)
        | Some(LengthSpec::MinContent)
        | Some(LengthSpec::MaxContent)
        | Some(LengthSpec::FitContent) => None,
        Some(other) => other
            .resolve_with_fonts(
                Some(percent_base),
                Some((viewport.width, viewport.height)),
                fonts,
            )
            .map(|value| value.max(0.0)),
    }
}

fn content_box_main_border_size(
    style: &LayoutStyle,
    direction: FlexDirection,
    margin_percent_base: Option<f32>,
    content_main: f32,
    fonts: FontSizeContext,
) -> f32 {
    if !matches!(style.box_sizing, BoxSizing::ContentBox) {
        return content_main;
    }
    let pad = style.resolved_padding_against_fonts(margin_percent_base, fonts);
    let border = style.resolved_border_edges();
    content_main
        + match direction {
            FlexDirection::Row => pad.left + pad.right + border.left + border.right,
            FlexDirection::Column => pad.top + pad.bottom + border.top + border.bottom,
        }
}

struct GridPlacedItem {
    id: StableNodeId,
    col: usize,
    row: usize,
    col_span: usize,
    row_span: usize,
    intrinsic: Size,
}

struct Grid2DLayout {
    col_sizes: Vec<f32>,
    row_sizes: Vec<f32>,
    col_gap: f32,
    row_gap: f32,
    items: Vec<GridPlacedItem>,
}

fn grid_axis_extent(sizes: &[f32], gap: f32) -> f32 {
    sizes.iter().copied().sum::<f32>() + gap * sizes.len().saturating_sub(1) as f32
}

fn explicit_column_tracks(style: &LayoutStyle, content_w: f32, col_gap: f32) -> Vec<GridTrack> {
    explicit_tracks(
        style.grid_columns_repeat.as_ref(),
        style.active_grid_columns(),
        style
            .grid_template_areas
            .as_ref()
            .map(GridTemplateAreas::column_count)
            .unwrap_or(0),
        content_w,
        col_gap,
        GridTrack::Fr(1.0),
    )
}

fn explicit_row_tracks(style: &LayoutStyle, content_h: f32, row_gap: f32) -> Vec<GridTrack> {
    explicit_tracks(
        style.grid_rows_repeat.as_ref(),
        style.active_grid_rows(),
        style
            .grid_template_areas
            .as_ref()
            .map(GridTemplateAreas::row_count)
            .unwrap_or(0),
        content_h,
        row_gap,
        GridTrack::Auto,
    )
}

fn explicit_tracks(
    repeat: Option<&GridRepeatAuto>,
    tracks: Option<&[GridTrack]>,
    area_count: usize,
    container: f32,
    gap: f32,
    area_fallback: GridTrack,
) -> Vec<GridTrack> {
    if let Some(repeat) = repeat {
        repeat.expand(container, gap)
    } else if let Some(tracks) = tracks {
        tracks.to_vec()
    } else if area_count > 0 {
        vec![area_fallback; area_count]
    } else {
        Vec::new()
    }
}

fn expanded_repeat_line_names(
    repeat: Option<&GridRepeatAuto>,
    container: f32,
    gap: f32,
) -> Option<Vec<Vec<String>>> {
    let repeat = repeat.filter(|rep| rep.has_line_names())?;
    let names = repeat.expand_line_names(repeat.fill_count(container, gap));
    names.iter().any(|line| !line.is_empty()).then_some(names)
}

fn implicit_grid_track(auto: Option<&[GridTrack]>, implicit_index: usize) -> GridTrack {
    match auto {
        Some(tracks) if !tracks.is_empty() => tracks[implicit_index % tracks.len()],
        _ => GridTrack::Auto,
    }
}

fn ensure_grid_tracks(
    tracks: &mut Vec<GridTrack>,
    needed: usize,
    auto: Option<&[GridTrack]>,
    explicit: usize,
) {
    while tracks.len() < needed {
        let implicit_index = tracks.len().saturating_sub(explicit);
        tracks.push(implicit_grid_track(auto, implicit_index));
    }
}

/// 1-based CSS line → 0-based track boundary against the explicit grid.
/// Negative indexes count from the end (`-1` is the last line = `explicit` as
/// an exclusive track end).
fn grid_line_boundary(index: i32, explicit: usize) -> i32 {
    if index > 0 {
        index - 1
    } else if index < 0 {
        explicit as i32 + index + 1
    } else {
        0
    }
}

fn resolve_named_line(
    line: &GridLine,
    container: &LayoutStyle,
    columns: bool,
    after: Option<i32>,
    names: Option<&[Vec<String>]>,
) -> GridLine {
    match line {
        GridLine::Name(name) | GridLine::NthName(name, _) => {
            let occurrence = line.name_occurrence().unwrap_or(1) as u32;
            let index = if let Some(prev) = after {
                if columns {
                    container.named_column_line_after_from(name, prev, names)
                } else {
                    container.named_row_line_after_from(name, prev, names)
                }
            } else if columns {
                container.named_column_line_nth_from(name, occurrence, names)
            } else {
                container.named_row_line_nth_from(name, occurrence, names)
            };
            index.map(GridLine::Index).unwrap_or(GridLine::Auto)
        }
        other => other.clone(),
    }
}

fn resolve_item_grid_placement(
    container: &LayoutStyle,
    placement: &nana_ui_core::GridPlacement,
    explicit_cols: usize,
    explicit_rows: usize,
    col_names: Option<&[Vec<String>]>,
    row_names: Option<&[Vec<String>]>,
) -> (Option<i32>, usize, Option<i32>, usize) {
    if let Some(name) = placement.area.as_deref()
        && let Some(areas) = container.grid_template_areas.as_ref()
        && let Some((col, row, col_span, row_span)) = areas.lookup(name)
    {
        return (
            Some(col as i32),
            col_span.max(1),
            Some(row as i32),
            row_span.max(1),
        );
    }
    let col_start = resolve_named_line(&placement.column_start, container, true, None, col_names);
    let col_end_after = match (
        &col_start,
        placement.column_start.as_name(),
        placement.column_end.as_name(),
    ) {
        (GridLine::Index(s), Some(a), Some(b)) if a == b => Some(*s),
        _ => None,
    };
    let col_end = resolve_named_line(
        &placement.column_end,
        container,
        true,
        col_end_after,
        col_names,
    );
    let row_start = resolve_named_line(&placement.row_start, container, false, None, row_names);
    let row_end_after = match (
        &row_start,
        placement.row_start.as_name(),
        placement.row_end.as_name(),
    ) {
        (GridLine::Index(s), Some(a), Some(b)) if a == b => Some(*s),
        _ => None,
    };
    let row_end = resolve_named_line(
        &placement.row_end,
        container,
        false,
        row_end_after,
        row_names,
    );
    let (col_origin, col_span) = resolve_grid_axis(&col_start, &col_end, explicit_cols);
    let (row_origin, row_span) = resolve_grid_axis(&row_start, &row_end, explicit_rows);
    (col_origin, col_span, row_origin, row_span)
}

fn resolve_grid_axis(start: &GridLine, end: &GridLine, explicit: usize) -> (Option<i32>, usize) {
    let span_of = |n: u16| (n as usize).max(1);
    match (start, end) {
        (GridLine::Index(s), GridLine::Index(e)) => {
            let s = grid_line_boundary(*s, explicit);
            let e = grid_line_boundary(*e, explicit);
            let span = if e > s { (e - s) as usize } else { 1 };
            (Some(s), span)
        }
        (GridLine::Index(s), GridLine::Span(n)) => {
            (Some(grid_line_boundary(*s, explicit)), span_of(*n))
        }
        (GridLine::Span(n), GridLine::Index(e)) => {
            let span = span_of(*n);
            (Some(grid_line_boundary(*e, explicit) - span as i32), span)
        }
        (GridLine::Span(n), GridLine::Auto)
        | (GridLine::Auto, GridLine::Span(n))
        | (GridLine::Span(n), GridLine::Span(_)) => (None, span_of(*n)),
        (GridLine::Index(s), GridLine::Auto) => (Some(grid_line_boundary(*s, explicit)), 1),
        (GridLine::Auto, GridLine::Index(e)) => {
            let end = grid_line_boundary(*e, explicit);
            (Some(end - 1), 1)
        }
        (GridLine::Auto, GridLine::Auto)
        | (GridLine::Name(_), _)
        | (_, GridLine::Name(_))
        | (GridLine::NthName(_, _), _)
        | (_, GridLine::NthName(_, _)) => (None, 1),
    }
}

/// Per-row occupied column ranges `[start, end)`, merged and sorted.
#[derive(Default)]
struct GridOccupancy {
    rows: HashMap<usize, Vec<(usize, usize)>>,
}

impl GridOccupancy {
    fn range_free(ranges: &[(usize, usize)], start: usize, end: usize) -> bool {
        !ranges.iter().any(|&(a, b)| a < end && start < b)
    }

    fn free(&self, row: usize, col: usize, row_span: usize, col_span: usize) -> bool {
        let end = col.saturating_add(col_span);
        for r in row..row.saturating_add(row_span) {
            if let Some(ranges) = self.rows.get(&r)
                && !Self::range_free(ranges, col, end)
            {
                return false;
            }
        }
        true
    }

    fn occupy(&mut self, row: usize, col: usize, row_span: usize, col_span: usize) {
        let end = col.saturating_add(col_span);
        for r in row..row.saturating_add(row_span) {
            let ranges = self.rows.entry(r).or_default();
            ranges.push((col, end));
            ranges.sort_unstable();
            let mut merged: Vec<(usize, usize)> = Vec::new();
            for (a, b) in ranges.drain(..) {
                if let Some(last) = merged.last_mut()
                    && a <= last.1
                {
                    last.1 = last.1.max(b);
                    continue;
                }
                merged.push((a, b));
            }
            *ranges = merged;
        }
    }
}

fn search_grid_auto_slot(
    occupied: &GridOccupancy,
    row_origin: Option<usize>,
    col_origin: Option<usize>,
    row_span: usize,
    col_span: usize,
    col_wrap: usize,
    row_wrap: usize,
    start_row: usize,
    start_col: usize,
    column_flow: bool,
) -> (usize, usize) {
    if let (Some(row), Some(col)) = (row_origin, col_origin) {
        return (row, col);
    }
    let search_limit = 4096usize;
    if column_flow {
        let row_wrap = row_wrap.max(row_span);
        if let Some(col) = col_origin {
            for row in start_row..start_row.saturating_add(search_limit) {
                if occupied.free(row, col, row_span, col_span) {
                    return (row, col);
                }
            }
            // Past the scanned region — never silently reuse `start_row`.
            return (start_row.saturating_add(search_limit), col);
        }
        if let Some(row) = row_origin {
            for c in 0..search_limit {
                if occupied.free(row, c, row_span, col_span) {
                    return (row, c);
                }
            }
            return (row, search_limit);
        }
        let mut col = start_col;
        for _ in 0..search_limit {
            let row_begin = if col == start_col { start_row } else { 0 };
            let last = row_wrap.saturating_sub(row_span);
            if row_begin <= last {
                for row in row_begin..=last {
                    if occupied.free(row, col, row_span, col_span) {
                        return (row, col);
                    }
                }
            }
            col += 1;
        }
        (row_wrap, col)
    } else {
        let col_wrap = col_wrap.max(col_span);
        if let Some(row) = row_origin {
            let last = col_wrap.saturating_sub(col_span);
            for col in 0..=last {
                if occupied.free(row, col, row_span, col_span) {
                    return (row, col);
                }
            }
            // Implicit columns beyond the explicit wrap — never (row, 0).
            for col in last.saturating_add(1)..last.saturating_add(1).saturating_add(search_limit) {
                if occupied.free(row, col, row_span, col_span) {
                    return (row, col);
                }
            }
            return (row, last.saturating_add(1).saturating_add(search_limit));
        }
        if let Some(col) = col_origin {
            for row in start_row..start_row.saturating_add(search_limit) {
                if occupied.free(row, col, row_span, col_span) {
                    return (row, col);
                }
            }
            return (start_row.saturating_add(search_limit), col);
        }
        let mut row = start_row;
        for _ in 0..search_limit {
            let col_begin = if row == start_row { start_col } else { 0 };
            let last = col_wrap.saturating_sub(col_span);
            if col_begin <= last {
                for col in col_begin..=last {
                    if occupied.free(row, col, row_span, col_span) {
                        return (row, col);
                    }
                }
            }
            row += 1;
        }
        (row, start_col)
    }
}

fn collapse_unoccupied_tracks(
    tracks: &[GridTrack],
    items: &mut [GridPlacedItem],
    columns: bool,
) -> Vec<GridTrack> {
    let n = tracks.len();
    if n == 0 {
        return Vec::new();
    }
    let mut used = vec![false; n];
    for item in items.iter() {
        let (origin, span) = if columns {
            (item.col, item.col_span)
        } else {
            (item.row, item.row_span)
        };
        let end = origin.saturating_add(span).min(n);
        for occupied in used.iter_mut().take(end).skip(origin) {
            *occupied = true;
        }
    }
    if used.iter().all(|occupied| *occupied) {
        return tracks.to_vec();
    }
    let mut map = vec![0usize; n];
    let mut next = Vec::new();
    for (index, track) in tracks.iter().copied().enumerate() {
        if used[index] {
            map[index] = next.len();
            next.push(track);
        }
    }
    if next.is_empty() {
        return tracks.to_vec();
    }
    for item in items.iter_mut() {
        if columns {
            if item.col < n {
                item.col = map[item.col];
            }
        } else if item.row < n {
            item.row = map[item.row];
        }
    }
    next
}

fn layout_grid_2d(
    style: &LayoutStyle,
    flow: &[StableNodeId],
    child_sizes: &[Size],
    content: Size,
    fonts: FontSizeContext,
    nodes: &LayoutInputMap<'_>,
) -> Grid2DLayout {
    let col_gap = style
        .resolved_column_gap_against_fonts(Some(content.width).filter(|width| *width > 0.0), fonts);
    let row_gap = style.resolved_row_gap_against_fonts(
        Some(content.height)
            .filter(|height| *height > 0.0)
            .or(Some(content.width).filter(|width| *width > 0.0)),
        fonts,
    );
    let mut col_tracks = explicit_column_tracks(style, content.width, col_gap);
    let mut row_tracks = explicit_row_tracks(style, content.height, row_gap);
    let explicit_cols = col_tracks.len();
    let explicit_rows = row_tracks.len();
    let auto_cols = style.grid_auto_columns.as_deref().filter(|t| !t.is_empty());
    let auto_rows = style.grid_auto_rows.as_deref().filter(|t| !t.is_empty());
    let auto_flow = style.grid_auto_flow.unwrap_or(GridAutoFlow::Row);
    let column_flow = auto_flow.is_column();
    let dense = auto_flow.is_dense();

    let default_placement = GridPlacement::default();
    let col_repeat_names =
        expanded_repeat_line_names(style.grid_columns_repeat.as_ref(), content.width, col_gap);
    let row_repeat_names =
        expanded_repeat_line_names(style.grid_rows_repeat.as_ref(), content.height, row_gap);
    let col_names = col_repeat_names
        .as_deref()
        .or(style.grid_column_line_names.as_deref());
    let row_names = row_repeat_names
        .as_deref()
        .or(style.grid_row_line_names.as_deref());
    let mut pending = Vec::with_capacity(flow.len());
    for (id, intrinsic) in flow.iter().copied().zip(child_sizes.iter().copied()) {
        let child_style = nodes.style(id);
        let placement = child_style
            .as_ref()
            .map(|child| &child.grid_placement)
            .unwrap_or(&default_placement);
        let (col_origin, col_span, row_origin, row_span) = resolve_item_grid_placement(
            style,
            placement,
            explicit_cols,
            explicit_rows,
            col_names,
            row_names,
        );
        pending.push((
            id,
            intrinsic,
            col_origin,
            col_span.max(1),
            row_origin,
            row_span.max(1),
        ));
    }

    let mut occupied = GridOccupancy::default();
    let mut items: Vec<GridPlacedItem> = Vec::with_capacity(pending.len());
    let mut placed = vec![false; pending.len()];

    let place_at = |items: &mut Vec<GridPlacedItem>,
                    col_tracks: &mut Vec<GridTrack>,
                    row_tracks: &mut Vec<GridTrack>,
                    occupied: &mut GridOccupancy,
                    id: StableNodeId,
                    intrinsic: Size,
                    row: usize,
                    col: usize,
                    row_span: usize,
                    col_span: usize| {
        ensure_grid_tracks(
            col_tracks,
            col.saturating_add(col_span),
            auto_cols,
            explicit_cols,
        );
        ensure_grid_tracks(
            row_tracks,
            row.saturating_add(row_span),
            auto_rows,
            explicit_rows,
        );
        occupied.occupy(row, col, row_span, col_span);
        items.push(GridPlacedItem {
            id,
            col,
            row,
            col_span,
            row_span,
            intrinsic,
        });
    };

    // Pass 1: both axes definite.
    for (index, &(id, intrinsic, col_origin, col_span, row_origin, row_span)) in
        pending.iter().enumerate()
    {
        let (Some(col), Some(row)) = (col_origin, row_origin) else {
            continue;
        };
        let col = col.max(0) as usize;
        let row = row.max(0) as usize;
        place_at(
            &mut items,
            &mut col_tracks,
            &mut row_tracks,
            &mut occupied,
            id,
            intrinsic,
            row,
            col,
            row_span,
            col_span,
        );
        placed[index] = true;
    }

    let mut cursor_row = 0usize;
    let mut cursor_col = 0usize;
    for (index, &(id, intrinsic, col_origin, col_span, row_origin, row_span)) in
        pending.iter().enumerate()
    {
        if placed[index] {
            continue;
        }
        if let Some(col) = col_origin {
            ensure_grid_tracks(
                &mut col_tracks,
                (col.max(0) as usize).saturating_add(col_span),
                auto_cols,
                explicit_cols,
            );
        }
        if let Some(row) = row_origin {
            ensure_grid_tracks(
                &mut row_tracks,
                (row.max(0) as usize).saturating_add(row_span),
                auto_rows,
                explicit_rows,
            );
        }
        if col_span > col_tracks.len() {
            ensure_grid_tracks(&mut col_tracks, col_span, auto_cols, explicit_cols);
        }
        if row_span > row_tracks.len() {
            ensure_grid_tracks(&mut row_tracks, row_span, auto_rows, explicit_rows);
        }
        let start_row = if dense { 0 } else { cursor_row };
        let start_col = if dense { 0 } else { cursor_col };
        let (row, col) = search_grid_auto_slot(
            &occupied,
            row_origin.map(|v| v.max(0) as usize),
            col_origin.map(|v| v.max(0) as usize),
            row_span,
            col_span,
            col_tracks.len(),
            row_tracks.len(),
            start_row,
            start_col,
            column_flow,
        );
        place_at(
            &mut items,
            &mut col_tracks,
            &mut row_tracks,
            &mut occupied,
            id,
            intrinsic,
            row,
            col,
            row_span,
            col_span,
        );
        if !dense {
            if column_flow {
                cursor_col = col;
                cursor_row = row.saturating_add(row_span);
            } else {
                cursor_row = row;
                cursor_col = col.saturating_add(col_span);
            }
        }
    }

    if style
        .grid_columns_repeat
        .as_ref()
        .is_some_and(|repeat| repeat.kind.is_auto_fit())
    {
        col_tracks = collapse_unoccupied_tracks(&col_tracks, &mut items, true);
    }
    if style
        .grid_rows_repeat
        .as_ref()
        .is_some_and(|repeat| repeat.kind.is_auto_fit())
    {
        row_tracks = collapse_unoccupied_tracks(&row_tracks, &mut items, false);
    }

    let mut col_auto = vec![0.0f32; col_tracks.len()];
    let mut row_auto = vec![0.0f32; row_tracks.len()];
    for item in &items {
        if item.col_span == 1 && item.col < col_auto.len() {
            col_auto[item.col] = col_auto[item.col].max(item.intrinsic.width);
        }
        if item.row_span == 1 && item.row < row_auto.len() {
            row_auto[item.row] = row_auto[item.row].max(item.intrinsic.height);
        }
    }
    let col_sizes = resolve_grid_track_sizes(&col_tracks, content.width, col_gap, &col_auto);
    let mut row_sizes = resolve_grid_track_sizes(&row_tracks, content.height, row_gap, &row_auto);
    // Leftover definite height goes to *empty* auto rows so `height:100%` /
    // empty stretch have a cell, without inflating content-sized auto rows.
    distribute_auto_track_leftover(&row_tracks, &mut row_sizes, content.height, row_gap);
    Grid2DLayout {
        col_sizes,
        row_sizes,
        col_gap,
        row_gap,
        items,
    }
}

fn distribute_auto_track_leftover(
    tracks: &[GridTrack],
    sizes: &mut [f32],
    container: f32,
    gap: f32,
) {
    if container <= 0.5 || sizes.is_empty() || sizes.len() != tracks.len() {
        return;
    }
    let used = sizes.iter().copied().sum::<f32>() + gap * sizes.len().saturating_sub(1) as f32;
    let leftover = container - used;
    if leftover <= 0.5 {
        return;
    }
    // Only empty auto rows (no intrinsic). Content-sized auto rows stay
    // tight so `align-items:start` items keep their packed y (T-G26).
    let autos: Vec<usize> = tracks
        .iter()
        .enumerate()
        .filter(|(index, track)| matches!(track, GridTrack::Auto) && sizes[*index] <= 0.5)
        .map(|(index, _)| index)
        .collect();
    if autos.is_empty() {
        return;
    }
    let share = leftover / autos.len() as f32;
    for index in autos {
        sizes[index] += share;
    }
}

fn grid_track_offsets(sizes: &[f32], gap: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(sizes.len());
    let mut acc = 0.0;
    for (index, size) in sizes.iter().copied().enumerate() {
        out.push(acc);
        acc += size;
        if index + 1 < sizes.len() {
            acc += gap;
        }
    }
    out
}

fn grid_span_extent(sizes: &[f32], start: usize, span: usize, gap: f32) -> f32 {
    if span == 0 || start >= sizes.len() {
        return 0.0;
    }
    let end = start.saturating_add(span).min(sizes.len());
    let sum: f32 = sizes[start..end].iter().copied().sum();
    sum + gap * (end - start).saturating_sub(1) as f32
}

fn size_is_indefinite(spec: Option<LengthSpec>) -> bool {
    !spec.is_some_and(LengthSpec::is_definite_declared)
}

/// After tracks exist, percent / Fill resolve against the final cell.
fn used_in_grid_cell(spec: Option<LengthSpec>, intrinsic: f32, cell: f32) -> f32 {
    match spec {
        Some(LengthSpec::Fill) => cell.max(0.0),
        Some(LengthSpec::Percent(percent)) => (cell * percent / 100.0).max(0.0),
        Some(LengthSpec::CalcPercentOffset { percent, offset_px }) => {
            (cell * percent / 100.0 + offset_px).max(0.0)
        }
        _ => intrinsic,
    }
}

fn align_in_grid_cell(align: AlignSpec, used: f32, cell: f32, stretch: bool) -> (f32, f32) {
    if stretch {
        return (0.0, cell.max(0.0));
    }
    if used + 1e-6 >= cell {
        return (0.0, used);
    }
    let offset = match align {
        AlignSpec::Start | AlignSpec::Stretch | AlignSpec::Baseline => 0.0,
        AlignSpec::Center => ((cell - used) / 2.0).max(0.0),
        AlignSpec::End => (cell - used).max(0.0),
    };
    (offset, used)
}

#[allow(clippy::too_many_arguments)]
fn place_grid_2d_items(
    grid: &Grid2DLayout,
    content_origin: Point,
    content: Size,
    style: &LayoutStyle,
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
    output: &mut HashMap<StableNodeId, LayoutBox>,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    let col_off = grid_track_offsets(&grid.col_sizes, grid.col_gap);
    let row_off = grid_track_offsets(&grid.row_sizes, grid.row_gap);
    for item in &grid.items {
        let Some(child_style) = nodes.style(item.id) else {
            continue;
        };
        let child_style = child_style.as_ref();
        let child_fonts = fonts_of(child_style, child_font_px);
        let cell_x = col_off.get(item.col).copied().unwrap_or(0.0);
        let cell_y = row_off.get(item.row).copied().unwrap_or(0.0);
        let cell_w = grid_span_extent(&grid.col_sizes, item.col, item.col_span, grid.col_gap);
        let cell_h = grid_span_extent(&grid.row_sizes, item.row, item.row_span, grid.row_gap);
        let justify = child_style.resolved_justify_self(style.justify_items);
        let align = child_style.resolved_align_self(style.align_items);
        let stretch_x = justify == AlignSpec::Stretch && size_is_indefinite(child_style.width);
        let ratio_filled_height = aspect_ratio_is_usable(child_style)
            && child_style
                .width
                .is_some_and(LengthSpec::is_definite_declared);
        let stretch_y = align == AlignSpec::Stretch
            && size_is_indefinite(child_style.height)
            && !ratio_filled_height;
        let measured_w = used_in_grid_cell(child_style.width, item.intrinsic.width, cell_w);
        let measured_h = used_in_grid_cell(child_style.height, item.intrinsic.height, cell_h);
        let (off_x, used_w) = align_in_grid_cell(justify, measured_w, cell_w, stretch_x);
        let (off_y, used_h) = align_in_grid_cell(align, measured_h, cell_h, stretch_y);
        let mut child_size = Size::new(used_w, used_h);
        if !stretch_y {
            fill_auto_height_from_aspect_ratio(
                child_style,
                &mut child_size,
                Some(content.width),
                child_fonts,
            );
        }
        let child_origin = Point {
            x: content_origin.x + cell_x + off_x,
            y: content_origin.y + cell_y + off_y,
        };
        if !subtree_unchanged(
            item.id,
            child_origin,
            child_size,
            content,
            child_style,
            child_fonts,
            scope,
        ) {
            place_node_scoped(
                item.id,
                child_origin,
                child_size,
                content,
                viewport,
                child_font_px,
                nodes,
                intrinsic,
                output,
                scope,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn pack_wrap_lines(
    children: &[StableNodeId],
    sizes: &[Size],
    direction: FlexDirection,
    content_main: f32,
    gap: f32,
    grid_tracks: Option<&[GridTrack]>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
    break_on_blocks: bool,
) -> Vec<Vec<usize>> {
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
            Some(content_main),
            fonts_of(style.as_ref(), parent_font_px),
        );
        let main = packing_main_size(
            style.as_ref(),
            sizes[index],
            direction,
            content_main,
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

fn ifc_item_outer(
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
fn pack_ifc_line_boxes(
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
fn wrap_intrinsic_size(
    direction: FlexDirection,
    wrap: FlexWrap,
    children: &[StableNodeId],
    sizes: &[Size],
    available: Size,
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
        content_main,
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
                Some(available.width),
                fonts_of(style.as_ref(), parent_font_px),
            );
            let main = packing_main_size(
                style.as_ref(),
                sizes[index],
                direction,
                content_main,
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

#[allow(clippy::too_many_arguments)]
fn grid_intrinsic_size(
    direction: FlexDirection,
    tracks: &[f32],
    child_sizes: &[Size],
    children: &[StableNodeId],
    content_width: f32,
    gap: f32,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
) -> Size {
    let gaps = gap * tracks.len().saturating_sub(1) as f32;
    let main = tracks.iter().sum::<f32>() + gaps;
    let mut cross = 0.0f32;
    for (index, child) in children.iter().enumerate() {
        let margin = nodes
            .style(*child)
            .map(|style| {
                style.resolved_margin_against_fonts(
                    Some(content_width),
                    fonts_of(style.as_ref(), parent_font_px),
                )
            })
            .unwrap_or_default();
        let size = child_sizes.get(index).copied().unwrap_or_default();
        cross = cross.max(cross_extent(size, direction) + cross_margin(margin, direction));
    }
    match direction {
        FlexDirection::Row => Size::new(main, cross),
        FlexDirection::Column => Size::new(cross, main),
    }
}

#[allow(clippy::too_many_arguments)]
fn auto_track_contributions(
    children: &[StableNodeId],
    tracks: &[GridTrack],
    content: Size,
    column_main: bool,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut IntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Vec<f32>, UiWorldError> {
    let n = tracks.len().min(children.len());
    let mut sizes = vec![0.0f32; n];
    if !tracks.iter().any(|track| matches!(track, GridTrack::Auto)) {
        return Ok(sizes);
    }
    let direction = if column_main {
        FlexDirection::Column
    } else {
        FlexDirection::Row
    };
    for index in 0..n {
        if !matches!(tracks[index], GridTrack::Auto) {
            continue;
        }
        let child = children[index];
        let Some(style) = nodes.style(child) else {
            continue;
        };
        let axis_spec = if column_main {
            style.height
        } else {
            style.width
        };
        let percent_base = if column_main {
            content.height
        } else {
            content.width
        };
        if let Some(px) = resolve_child_main(
            axis_spec,
            percent_base,
            viewport,
            fonts_of(style.as_ref(), parent_font_px),
        ) {
            sizes[index] = px.max(0.0);
            continue;
        }
        let available = if column_main {
            Size::new(content.width, 0.0)
        } else {
            Size::new(0.0, content.height)
        };
        let measured = intrinsic_size_demoted(
            child,
            available,
            Some(direction),
            viewport,
            parent_font_px,
            nodes,
            cache,
            scope,
            column_main,
        )?;
        sizes[index] = if column_main {
            measured.height
        } else {
            measured.width
        };
    }
    Ok(sizes)
}

#[allow(clippy::too_many_arguments)]
fn intrinsic_size_demoted(
    id: StableNodeId,
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut IntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
    column_main: bool,
) -> Result<Size, UiWorldError> {
    // Auto-track throwaway: Fill / 100% on the track axis must not snap to the
    // grid's definite size. Measure against a near-zero available on that axis
    // after treating Fill/100% as content-sized via `available` 0.
    let _ = column_main;
    intrinsic_size_scoped(
        id,
        available,
        parent_direction,
        viewport,
        parent_font_px,
        nodes,
        cache,
        scope,
    )
}

#[allow(clippy::too_many_arguments)]
fn apply_grid_main_sizes(
    children: &[StableNodeId],
    sizes: &mut [Size],
    direction: FlexDirection,
    content: Size,
    gap: f32,
    tracks: &[GridTrack],
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut IntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    let n = children.len();
    if n == 0 {
        return Ok(());
    }
    let mut margins = Vec::with_capacity(n);
    for child in children {
        let margin = nodes
            .style(*child)
            .map(|style| {
                style.resolved_margin_against_fonts(
                    Some(content.width),
                    fonts_of(style.as_ref(), parent_font_px),
                )
            })
            .unwrap_or_default();
        margins.push(margin);
    }
    let track_n = n.min(tracks.len());
    let margin_total: f32 = margins
        .iter()
        .take(track_n)
        .map(|margin| main_start_margin(*margin, direction) + main_end_margin(*margin, direction))
        .sum();
    let budget = (main_extent(content, direction) - margin_total).max(0.0);
    let auto_sizes = auto_track_contributions(
        &children[..track_n],
        &tracks[..track_n],
        content,
        direction == FlexDirection::Column,
        viewport,
        parent_font_px,
        nodes,
        intrinsic,
        scope,
    )?;
    let mut resolved = resolve_grid_track_sizes(&tracks[..track_n], budget, gap, &auto_sizes);
    if resolved.len() < n {
        let used: f32 =
            resolved.iter().sum::<f32>() + gap * resolved.len().saturating_sub(1) as f32;
        let rem = (budget - used).max(0.0);
        let extra = n - resolved.len();
        let each = if extra > 0 { rem / extra as f32 } else { 0.0 };
        resolved.extend(std::iter::repeat_n(each, extra));
    }
    for (size, main) in sizes.iter_mut().zip(resolved) {
        set_main_extent(size, direction, main);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn distribute_flex_main(
    children: &[StableNodeId],
    sizes: &mut [Size],
    direction: FlexDirection,
    content: Size,
    gap: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
) {
    let n = children.len();
    if n == 0 {
        return;
    }
    let content_main = main_extent(content, direction);
    let gap_total = gap * n.saturating_sub(1) as f32;
    let vp = Some((viewport.width, viewport.height));
    let mut margin_mains = Vec::with_capacity(n);
    let mut fixed_or_fill: Vec<Option<f32>> = Vec::with_capacity(n);
    let mut mins = Vec::with_capacity(n);
    let mut maxs = Vec::with_capacity(n);
    let mut grows = Vec::with_capacity(n);
    let mut shrinks = Vec::with_capacity(n);
    for (index, child) in children.iter().enumerate() {
        let Some(style) = nodes.style(*child) else {
            margin_mains.push(0.0);
            mins.push(0.0);
            maxs.push(None);
            grows.push(0.0);
            shrinks.push(1.0);
            fixed_or_fill.push(Some(main_extent(sizes[index], direction)));
            continue;
        };
        let fonts = fonts_of(style.as_ref(), parent_font_px);
        let margin = style.resolved_margin_against_fonts(Some(content.width), fonts);
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
        match resolve_child_main(main, content_main, viewport, fonts) {
            Some(value) => {
                let mut value = value.max(min_main);
                if let Some(max) = max_main {
                    value = value.min(max);
                }
                value = content_box_main_border_size(
                    style.as_ref(),
                    direction,
                    Some(content.width),
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
                        Some(content.width),
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
}

fn resolve_flex_fill_sizes(
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
        occupied += margin_mains[i].max(0.0);
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

fn apply_flex_shrink(
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
    let margin_total: f32 = margin_mains.iter().map(|margin| margin.max(0.0)).sum();
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

fn main_occupied(
    children: &[StableNodeId],
    sizes: &[Size],
    direction: FlexDirection,
    content: Size,
    gap: f32,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
) -> f32 {
    let mut occupied = 0.0;
    for (id, size) in children.iter().zip(sizes) {
        let margin = match nodes.style(*id) {
            Some(style) => style.resolved_margin_against_fonts(
                Some(content.width),
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

fn justify_offsets(
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

fn clear_offset(clear: ClearSpec, left_bottom: f32, right_bottom: f32) -> f32 {
    match clear {
        ClearSpec::None => 0.0,
        ClearSpec::Left => left_bottom,
        ClearSpec::Right => right_bottom,
        ClearSpec::Both => left_bottom.max(right_bottom),
    }
}

fn count_auto_main_margins(
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

fn apply_auto_margins(
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

fn resolve_axis(
    spec: Option<LengthSpec>,
    base: f32,
    viewport: LayoutViewport,
    fonts: FontSizeContext,
) -> Option<f32> {
    spec.and_then(|value| {
        if value == LengthSpec::Fill {
            Some(base)
        } else {
            value
                .resolve_with_fonts(Some(base), Some((viewport.width, viewport.height)), fonts)
                .map(|value| value.max(0.0))
        }
    })
}

fn demote_fill_spec_if_indefinite(spec: Option<LengthSpec>, base: f32) -> Option<LengthSpec> {
    if base > 0.5 {
        spec
    } else {
        demote_fill_spec(spec)
    }
}

fn aspect_ratio_is_usable(style: &nana_ui_core::LayoutStyle) -> bool {
    style.aspect_ratio.is_some_and(|r| r.is_finite() && r > 0.0)
}

/// After stretch (or a flexed used width), fill `height:auto` from the used width.
fn fill_auto_height_from_aspect_ratio(
    style: &nana_ui_core::LayoutStyle,
    size: &mut Size,
    percent_base: Option<f32>,
    fonts: FontSizeContext,
) {
    if !aspect_ratio_is_usable(style) || style.height.is_some() {
        return;
    }
    let padding = style.resolved_padding_against_fonts(percent_base, fonts);
    let border = style.resolved_border_edges();
    let chrome_w = padding.left + padding.right + border.left + border.right;
    let chrome_h = padding.top + padding.bottom + border.top + border.bottom;
    let mut content_w = Some((size.width - chrome_w).max(0.0));
    let mut content_h = None;
    style.apply_aspect_ratio_used(&mut content_w, &mut content_h);
    if let Some(h) = content_h {
        size.height = h + chrome_h;
    }
}

fn cross_axis_is_definite(style: &nana_ui_core::LayoutStyle, direction: FlexDirection) -> bool {
    match direction {
        // Transferred block size from a definite used width + `aspect-ratio`.
        FlexDirection::Row => style.height.is_some() || aspect_ratio_is_usable(style),
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

    use nana_ui_core::{
        AlignSpec, BoxSizing, ClearSpec, DisplaySpec, FlexDirection, FlexWrap, FloatSpec, GridLine,
        GridPlacement, GridRepeatAuto, GridTrack, GridTrackListUnsupported, JustifySpec,
        LayoutStyle, LengthSpec, LineHeightSpec, PositionSpec, WhiteSpaceSpec,
    };

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
            queue.set_text(
                label_id,
                TextContent {
                    value: "行".into()
                },
            );
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

    fn write_changed_boxes(
        world: &mut UiWorld,
        emitted: &[(StableNodeId, LayoutBox)],
    ) -> Vec<StableNodeId> {
        let mut queue = MutationQueue::new();
        let mut written = Vec::new();
        for (id, box_) in emitted {
            if world.layout_box(*id) != Some(*box_) {
                queue.write_layout(*id, *box_);
                written.push(*id);
            }
        }
        if !written.is_empty() {
            world.commit(queue).unwrap();
        }
        written
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
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();

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
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();

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

        let written = write_changed_boxes(&mut world, &emitted);
        let extract = world.take_system_work();
        let changed_row = id(3 + 200 * 2);
        let changed_label = id(4 + 200 * 2);
        let row_above = id(3 + 199 * 2);
        let label_above = id(4 + 199 * 2);
        let shifted_row = id(3 + 201 * 2);
        let shifted_label = id(4 + 201 * 2);
        assert!(written.contains(&changed_row));
        assert!(written.contains(&shifted_row));
        assert!(written.contains(&shifted_label));
        assert!(extract.render_extraction.contains(&changed_row));
        assert!(extract.render_extraction.contains(&shifted_row));
        assert!(extract.render_extraction.contains(&shifted_label));
        assert!(
            !written.contains(&changed_label),
            "bit-identical label of the changed row must not be written"
        );
        assert!(!extract.render_extraction.contains(&changed_label));
        assert!(!extract.render_extraction.contains(&row_above));
        assert!(!extract.render_extraction.contains(&label_above));
    }

    #[test]
    fn scoped_layout_materializes_far_fewer_inputs_than_the_document_for_a_tail_row() {
        let (mut world, document) = column_tree(400);
        let viewport = LayoutViewport::new(300.0, 800.0);
        let mut retained = RetainedLayoutCache::default();
        let _ = world.take_system_work();

        let emitted = RuntimeLayoutEngine
            .layout_document_scoped(&world, document, viewport, &[], &mut retained, true)
            .unwrap();
        assert_eq!(emitted.len(), 802);
        assert_eq!(retained.materialized_inputs, 802);
        write_changed_boxes(&mut world, &emitted);
        let _ = world.take_system_work();

        resize_row(&mut world, 399, 26.0);
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
        assert!(
            emitted.len() < 16,
            "tail row change must stay O(depth), not relayout {} nodes",
            emitted.len()
        );
        // Document + column + dirty row (+ label / path ancestors). Unshifted
        // siblings are classified from layout style, not full LayoutInput.
        assert!(
            retained.materialized_inputs <= 16,
            "tail row must not assemble unshifted siblings, materialized {} of 802",
            retained.materialized_inputs
        );
        for (node, box_) in full_boxes(&world, document, viewport) {
            assert_eq!(
                retained.boxes.get(&node),
                Some(&box_),
                "on-demand scoped layout diverged from full recompute at {node:?}"
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
                    flex_shrink: Some(0.0),
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
    fn unspecified_flex_shrink_keeps_overflowing_definite_row() {
        // Issue #22: omitted flex-shrink is 0, not CSS initial 1.
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
                    width: Some(LengthSpec::Px(200.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    direction: Some(FlexDirection::Row),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        for value in [2, 3] {
            queue.set_style(
                id(value),
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(150.0)),
                        height: Some(LengthSpec::Px(40.0)),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
        }
        world.commit(queue).unwrap();
        let layouts = RuntimeLayoutEngine
            .layout_document(&world, document, LayoutViewport::new(200.0, 40.0))
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(layouts[&id(2)].width, 150.0);
        assert_eq!(layouts[&id(3)].width, 150.0);
        assert_eq!(layouts[&id(3)].x, 150.0);
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

    #[test]
    fn row_wrap_breaks_to_the_next_line() {
        let document = DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document, NodeKind::Document);
        for value in 2..=5 {
            queue.create(id(value), document, NodeKind::Element { tag: "div".into() });
            queue.insert(id(1), id(value), None);
            queue.set_style(
                id(value),
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(80.0)),
                        height: Some(LengthSpec::Px(40.0)),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
        }
        queue.set_style(
            id(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(200.0)),
                    direction: Some(FlexDirection::Row),
                    flex_wrap: FlexWrap::Wrap,
                    gap: Some(LengthSpec::Px(8.0)),
                    align_items: nana_ui_core::AlignSpec::Start,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let layouts = RuntimeLayoutEngine
            .layout_document(&world, document, LayoutViewport::new(200.0, 160.0))
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(layouts[&id(2)].x, 0.0);
        assert_eq!(layouts[&id(3)].x, 88.0);
        assert_eq!(layouts[&id(4)].x, 0.0);
        assert_eq!(layouts[&id(4)].y, 48.0);
        assert_eq!(layouts[&id(5)].x, 88.0);
        assert_eq!(layouts[&id(5)].y, 48.0);
        assert_eq!(layouts[&id(1)].height, 88.0);
    }

    #[test]
    fn grid_template_columns_split_free_space() {
        let document = DocumentId::new(1).unwrap();
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document, NodeKind::Element { tag: "div".into() });
        queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
        queue.create(id(3), document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(1), id(2), None);
        queue.insert(id(1), id(3), None);
        queue.set_style(
            id(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    display: Some(DisplaySpec::Grid),
                    direction: Some(FlexDirection::Row),
                    width: Some(LengthSpec::Px(800.0)),
                    height: Some(LengthSpec::Px(400.0)),
                    grid_columns: Some(vec![GridTrack::Px(220.0), GridTrack::Fr(1.0)]),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        for value in [2, 3] {
            queue.set_style(
                id(value),
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        height: Some(LengthSpec::Px(400.0)),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                },
            );
        }
        world.commit(queue).unwrap();
        let layouts = RuntimeLayoutEngine
            .layout_document(&world, document, LayoutViewport::new(800.0, 400.0))
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(layouts[&id(2)].width, 220.0);
        assert_eq!(layouts[&id(3)].x, 220.0);
        assert_eq!(layouts[&id(3)].width, 580.0);
    }

    #[test]
    fn style_tree_matches_document_layout_for_row_gap() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                direction: Some(FlexDirection::Row),
                width: Some(LengthSpec::Px(400.0)),
                height: Some(LengthSpec::Px(80.0)),
                gap: Some(LengthSpec::Px(12.0)),
                align_items: nana_ui_core::AlignSpec::Start,
                ..LayoutStyle::default()
            },
            children: vec![
                StyleLayoutNode {
                    id: "a".into(),
                    style: LayoutStyle {
                        width: Some(LengthSpec::Px(50.0)),
                        height: Some(LengthSpec::Px(40.0)),
                        ..LayoutStyle::default()
                    },
                    children: Vec::new(),
                    text: None,
                },
                StyleLayoutNode {
                    id: "b".into(),
                    style: LayoutStyle {
                        width: Some(LengthSpec::Px(50.0)),
                        height: Some(LengthSpec::Px(40.0)),
                        ..LayoutStyle::default()
                    },
                    children: Vec::new(),
                    text: None,
                },
            ],
            text: None,
        };
        let boxes = RuntimeLayoutEngine
            .layout_style_tree(&tree, LayoutViewport::new(400.0, 80.0))
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert!((boxes["b"].x - 62.0).abs() < 0.01);
    }

    #[test]
    fn child_em_width_uses_parent_computed_font_size() {
        let tree = StyleLayoutNode {
            id: "parent".into(),
            style: LayoutStyle {
                font_size: Some(32.0),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(80.0)),
                direction: Some(FlexDirection::Row),
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "child".into(),
                style: LayoutStyle {
                    width: Some(LengthSpec::Em(2.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            }],
            text: None,
        };
        let boxes = RuntimeLayoutEngine
            .layout_style_tree(&tree, LayoutViewport::new(200.0, 80.0))
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            boxes["child"].width, 64.0,
            "2em against parent font-size 32px must be 64px, not 32px"
        );
    }

    #[test]
    fn child_em_padding_uses_parent_computed_font_size() {
        let tree = StyleLayoutNode {
            id: "parent".into(),
            style: LayoutStyle {
                font_size: Some(32.0),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(200.0)),
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "child".into(),
                style: LayoutStyle {
                    padding: Some(LengthSpec::Em(1.0)),
                    ..LayoutStyle::default()
                },
                children: vec![StyleLayoutNode {
                    id: "inner".into(),
                    style: LayoutStyle {
                        width: Some(LengthSpec::Px(10.0)),
                        height: Some(LengthSpec::Px(10.0)),
                        ..LayoutStyle::default()
                    },
                    children: Vec::new(),
                    text: None,
                }],
                text: None,
            }],
            text: None,
        };
        let boxes = RuntimeLayoutEngine
            .layout_style_tree(&tree, LayoutViewport::new(200.0, 200.0))
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            boxes["inner"].x, 32.0,
            "1em padding against inherited 32px font-size must inset content 32px, not 16px"
        );
        assert_eq!(boxes["inner"].y, 32.0);
        assert_eq!(
            boxes["child"].width, 74.0,
            "1em padding on both sides must add 64px to the 10px content box"
        );
        assert_eq!(boxes["child"].height, 74.0);
    }

    #[test]
    fn child_em_absolute_inset_uses_parent_computed_font_size() {
        let tree = StyleLayoutNode {
            id: "parent".into(),
            style: LayoutStyle {
                font_size: Some(32.0),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(200.0)),
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "child".into(),
                style: LayoutStyle {
                    position: PositionSpec::Absolute,
                    offset_top: Some(LengthSpec::Em(1.0)),
                    offset_left: Some(LengthSpec::Em(1.0)),
                    width: Some(LengthSpec::Px(40.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            }],
            text: None,
        };
        let boxes = RuntimeLayoutEngine
            .layout_style_tree(&tree, LayoutViewport::new(200.0, 200.0))
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            boxes["child"].x, 32.0,
            "1em left against inherited 32px font-size must place at 32px, not 16px"
        );
        assert_eq!(
            boxes["child"].y, 32.0,
            "1em top against inherited 32px font-size must place at 32px, not 16px"
        );
    }

    #[test]
    fn child_em_min_height_uses_parent_computed_font_size() {
        let tree = StyleLayoutNode {
            id: "parent".into(),
            style: LayoutStyle {
                font_size: Some(32.0),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(200.0)),
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "child".into(),
                style: LayoutStyle {
                    min_height: Some(LengthSpec::Em(2.0)),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            }],
            text: None,
        };
        let boxes = RuntimeLayoutEngine
            .layout_style_tree(&tree, LayoutViewport::new(200.0, 200.0))
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(
            boxes["child"].height, 64.0,
            "2em min-height against parent font-size 32px must be 64px, not 32px"
        );
    }

    fn box_map(root: &StyleLayoutNode, vw: f32, vh: f32) -> HashMap<String, LayoutBox> {
        RuntimeLayoutEngine
            .layout_style_tree(root, LayoutViewport::new(vw, vh))
            .into_iter()
            .collect()
    }

    fn px_box(id: &str, width: f32, height: f32) -> StyleLayoutNode {
        StyleLayoutNode {
            id: id.into(),
            style: LayoutStyle {
                width: Some(LengthSpec::Px(width)),
                height: Some(LengthSpec::Px(height)),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        }
    }

    #[test]
    fn align_content_center_and_space_between_on_wrapped_row() {
        let children = (0..4)
            .map(|i| px_box(&format!("i{i}"), 80.0, 40.0))
            .collect::<Vec<_>>();
        let make = |align_content| StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                direction: Some(FlexDirection::Row),
                flex_wrap: FlexWrap::Wrap,
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(160.0)),
                gap: Some(LengthSpec::Px(8.0)),
                align_items: AlignSpec::Start,
                align_content,
                ..LayoutStyle::default()
            },
            children: children.clone(),
            text: None,
        };
        let center = box_map(&make(JustifySpec::Center), 200.0, 160.0);
        assert!((center["i0"].y - 36.0).abs() < 0.01);
        assert!((center["i1"].y - 36.0).abs() < 0.01);
        assert!((center["i2"].y - 84.0).abs() < 0.01);
        assert!((center["i3"].y - 84.0).abs() < 0.01);
        let between = box_map(&make(JustifySpec::SpaceBetween), 200.0, 160.0);
        assert!((between["i0"].y - 0.0).abs() < 0.01);
        assert!((between["i2"].y - 120.0).abs() < 0.01);
    }

    #[test]
    fn display_contents_hoists_children_into_flex_row_gap() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                direction: Some(FlexDirection::Row),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(40.0)),
                gap: Some(LengthSpec::Px(10.0)),
                align_items: AlignSpec::Start,
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "contents".into(),
                style: LayoutStyle {
                    display: Some(DisplaySpec::Contents),
                    ..LayoutStyle::default()
                },
                children: vec![px_box("a", 50.0, 40.0), px_box("b", 50.0, 40.0)],
                text: None,
            }],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 40.0);
        assert!(
            !boxes.contains_key("contents"),
            "display:contents must be absent from the box map"
        );
        assert!((boxes["a"].x - 0.0).abs() < 0.01);
        assert!((boxes["b"].x - 60.0).abs() < 0.01);
        assert_eq!(boxes["a"].width, 50.0);
        assert_eq!(boxes["b"].width, 50.0);
    }

    #[test]
    fn grid_2d_auto_flow_wraps_fourth_item_to_second_row() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Grid),
                width: Some(LengthSpec::Px(100.0)),
                height: Some(LengthSpec::Px(100.0)),
                grid_columns: Some(vec![GridTrack::Px(50.0), GridTrack::Px(50.0)]),
                ..LayoutStyle::default()
            },
            children: (0..4)
                .map(|i| px_box(&format!("i{i}"), 50.0, 50.0))
                .collect(),
            text: None,
        };
        let boxes = box_map(&tree, 100.0, 100.0);
        assert_eq!(boxes["i0"].x, 0.0);
        assert_eq!(boxes["i0"].y, 0.0);
        assert_eq!(boxes["i1"].x, 50.0);
        assert_eq!(boxes["i1"].y, 0.0);
        assert_eq!(boxes["i2"].x, 0.0);
        assert_eq!(boxes["i2"].y, 50.0);
        assert_eq!(boxes["i3"].x, 50.0);
        assert_eq!(boxes["i3"].y, 50.0);
    }

    #[test]
    fn grid_column_span_two_on_three_columns() {
        let first = StyleLayoutNode {
            id: "a".into(),
            style: LayoutStyle {
                height: Some(LengthSpec::Px(50.0)),
                grid_placement: GridPlacement {
                    column_start: GridLine::Span(2),
                    ..GridPlacement::default()
                },
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        };
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Grid),
                width: Some(LengthSpec::Px(150.0)),
                height: Some(LengthSpec::Px(100.0)),
                grid_columns: Some(vec![
                    GridTrack::Px(50.0),
                    GridTrack::Px(50.0),
                    GridTrack::Px(50.0),
                ]),
                ..LayoutStyle::default()
            },
            children: vec![first, px_box("b", 50.0, 50.0), px_box("c", 50.0, 50.0)],
            text: None,
        };
        let boxes = box_map(&tree, 150.0, 100.0);
        assert!((boxes["a"].x - 0.0).abs() < 0.01);
        assert!((boxes["a"].width - 100.0).abs() < 0.01);
        assert!((boxes["b"].x - 100.0).abs() < 0.01);
        assert!((boxes["b"].y - 0.0).abs() < 0.01);
        assert!((boxes["c"].x - 0.0).abs() < 0.01);
        assert!((boxes["c"].y - 50.0).abs() < 0.01);
    }

    #[test]
    fn grid_justify_self_end_in_definite_column() {
        let mut item = px_box("item", 50.0, 50.0);
        item.style.justify_self = Some(AlignSpec::End);
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Grid),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(50.0)),
                grid_columns: Some(vec![GridTrack::Px(200.0)]),
                ..LayoutStyle::default()
            },
            children: vec![item],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 50.0);
        assert!((boxes["item"].x - 150.0).abs() < 0.01);
        assert_eq!(boxes["item"].width, 50.0);
    }

    #[test]
    fn grid_auto_fit_fills_two_minmax_tracks_in_500px() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Grid),
                width: Some(LengthSpec::Px(500.0)),
                height: Some(LengthSpec::Px(50.0)),
                grid_columns_repeat: Some(GridRepeatAuto {
                    kind: GridTrackListUnsupported::RepeatAutoFit,
                    tracks: vec![GridTrack::MinMax {
                        min_px: 200.0,
                        fr: 1.0,
                        max_px: None,
                    }],
                    ..Default::default()
                }),
                ..LayoutStyle::default()
            },
            children: vec![px_box("a", 50.0, 50.0), px_box("b", 50.0, 50.0)],
            text: None,
        };
        let boxes = box_map(&tree, 500.0, 50.0);
        assert!(
            (boxes["b"].x - 250.0).abs() < 0.5,
            "auto-fit minmax(200px,1fr) in 500px must keep 2 tracks, got b.x={}",
            boxes["b"].x
        );
        assert!((boxes["a"].x - 0.0).abs() < 0.5);
    }

    #[test]
    fn white_space_pre_measures_explicit_newlines() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(200.0)),
                font_size: Some(16.0),
                line_height: Some(LineHeightSpec::Absolute(20.0)),
                white_space: WhiteSpaceSpec::Pre,
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: Some("ab\ncd".into()),
        };
        let boxes = box_map(&tree, 200.0, 80.0);
        assert!(
            (boxes["root"].height - 40.0).abs() < 0.01,
            "pre + 2 lines × 20px line-height must be 40, got {}",
            boxes["root"].height
        );
    }

    #[test]
    fn white_space_pre_wrap_keeps_newlines_and_wraps_long_lines() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(200.0)),
                font_size: Some(16.0),
                line_height: Some(LineHeightSpec::Absolute(20.0)),
                white_space: WhiteSpaceSpec::PreWrap,
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: Some("ab\ncd".into()),
        };
        let boxes = box_map(&tree, 200.0, 120.0);
        assert!(
            (boxes["root"].height - 40.0).abs() < 0.01,
            "pre-wrap must keep explicit newlines (not Normal), got {}",
            boxes["root"].height
        );
    }

    #[test]
    fn measure_text_pre_wrap_wraps_long_line_against_max_width() {
        let mut shaper = crate::MeasureTextShaper;
        let style = ComputedStyle {
            font_size: 16.0,
            line_height: Some(LineHeightSpec::Absolute(20.0)),
            ..ComputedStyle::default()
        };
        let metrics = shaper.shape(
            StableNodeId::new(1).unwrap(),
            &crate::TextContent {
                value: "abcdefghijklmnop\nq".into(),
            },
            &style,
            crate::TextShapeConstraints {
                max_width: Some(200.0),
                wrap: true,
                preserve_lines: true,
                ..crate::TextShapeConstraints::default()
            },
        );
        assert!(
            (metrics.height - 60.0).abs() < 0.01,
            "16em line in 200px + explicit second line → 60, got {}",
            metrics.height
        );
    }

    #[test]
    fn aspect_ratio_square_from_definite_width() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                width: Some(LengthSpec::Px(80.0)),
                aspect_ratio: Some(1.0),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        };
        let boxes = box_map(&tree, 400.0, 200.0);
        assert!(
            (boxes["root"].width - 80.0).abs() < 0.01 && (boxes["root"].height - 80.0).abs() < 0.01,
            "80px width + aspect-ratio 1 must be square, got {:?}",
            boxes["root"]
        );
    }

    #[test]
    fn aspect_ratio_auto_width_uses_containing_block() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                width: Some(LengthSpec::Px(400.0)),
                height: Some(LengthSpec::Px(200.0)),
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "block".into(),
                style: LayoutStyle {
                    height: Some(LengthSpec::Px(80.0)),
                    aspect_ratio: Some(1.0),
                    ..LayoutStyle::default()
                },
                children: vec![StyleLayoutNode {
                    id: "pct".into(),
                    style: LayoutStyle {
                        width: Some(LengthSpec::Percent(50.0)),
                        height: Some(LengthSpec::Px(10.0)),
                        ..LayoutStyle::default()
                    },
                    children: Vec::new(),
                    text: None,
                }],
                text: None,
            }],
            text: None,
        };
        let boxes = box_map(&tree, 400.0, 200.0);
        assert!(
            (boxes["block"].width - 400.0).abs() < 0.01
                && (boxes["block"].height - 80.0).abs() < 0.01,
            "block width:auto + height 80 + aspect-ratio 1 uses CB, not 80×80, got {:?}",
            boxes["block"]
        );
        assert!(
            (boxes["pct"].width - 200.0).abs() < 0.01,
            "% children resolve against the CB, not a shrink-wrapped 80, got {:?}",
            boxes["pct"]
        );
    }

    #[test]
    fn aspect_ratio_row_stretch_does_not_overwrite_transferred_height() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(200.0)),
                align_items: AlignSpec::Stretch,
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "item".into(),
                style: LayoutStyle {
                    width: Some(LengthSpec::Px(80.0)),
                    aspect_ratio: Some(1.0),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            }],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 200.0);
        assert!(
            (boxes["item"].width - 80.0).abs() < 0.01 && (boxes["item"].height - 80.0).abs() < 0.01,
            "row stretch must not overwrite height transferred from width + ratio, got {:?}",
            boxes["item"]
        );
    }

    #[test]
    fn aspect_ratio_column_stretch_fills_auto_height() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Column),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(200.0)),
                align_items: AlignSpec::Stretch,
                ..LayoutStyle::default()
            },
            children: vec![StyleLayoutNode {
                id: "item".into(),
                style: LayoutStyle {
                    aspect_ratio: Some(1.0),
                    ..LayoutStyle::default()
                },
                children: Vec::new(),
                text: None,
            }],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 200.0);
        assert!(
            (boxes["item"].width - 200.0).abs() < 0.01
                && (boxes["item"].height - 200.0).abs() < 0.01,
            "column stretch width then ratio must fill auto height, got {:?}",
            boxes["item"]
        );
    }

    #[test]
    fn grid_percent_and_fill_resolve_against_final_cell() {
        let fill = |id: &str| StyleLayoutNode {
            id: id.into(),
            style: LayoutStyle {
                width: Some(LengthSpec::Percent(100.0)),
                height: Some(LengthSpec::Fill),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        };
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Grid),
                width: Some(LengthSpec::Px(300.0)),
                height: Some(LengthSpec::Px(40.0)),
                grid_columns: Some(vec![GridTrack::Px(100.0), GridTrack::Fr(1.0)]),
                ..LayoutStyle::default()
            },
            children: vec![fill("a"), fill("b")],
            text: None,
        };
        let boxes = box_map(&tree, 300.0, 40.0);
        assert!(
            (boxes["a"].width - 100.0).abs() < 0.5 && (boxes["a"].height - 40.0).abs() < 0.5,
            "100%/Fill must fill the 100px track, not stay 0, got {:?}",
            boxes["a"]
        );
        assert!(
            (boxes["b"].width - 200.0).abs() < 0.5 && (boxes["b"].height - 40.0).abs() < 0.5,
            "100%/Fill must fill the 1fr cell, got {:?}",
            boxes["b"]
        );
    }

    #[test]
    fn empty_grid_item_stretches_into_track() {
        let empty = |id: &str| StyleLayoutNode {
            id: id.into(),
            style: LayoutStyle::default(),
            children: Vec::new(),
            text: None,
        };
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Grid),
                width: Some(LengthSpec::Px(300.0)),
                height: Some(LengthSpec::Px(40.0)),
                grid_columns: Some(vec![GridTrack::Px(100.0), GridTrack::Fr(1.0)]),
                // CSS `display:grid` initial align-items is stretch (css_map sets this).
                align_items: AlignSpec::Stretch,
                ..LayoutStyle::default()
            },
            children: vec![empty("a"), empty("b")],
            text: None,
        };
        let boxes = box_map(&tree, 300.0, 40.0);
        assert!(
            (boxes["a"].width - 100.0).abs() < 0.5 && (boxes["a"].height - 40.0).abs() < 0.5,
            "empty + stretch must fill the track, got {:?}",
            boxes["a"]
        );
        assert!(
            (boxes["b"].width - 200.0).abs() < 0.5 && (boxes["b"].height - 40.0).abs() < 0.5,
            "empty + stretch 1fr, got {:?}",
            boxes["b"]
        );
    }

    #[test]
    fn same_side_floats_do_not_overlap() {
        let floated = |id: &str| StyleLayoutNode {
            id: id.into(),
            style: LayoutStyle {
                width: Some(LengthSpec::Px(60.0)),
                height: Some(LengthSpec::Px(40.0)),
                float: FloatSpec::Left,
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        };
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(80.0)),
                height: Some(LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            },
            children: vec![floated("a"), floated("b")],
            text: None,
        };
        let boxes = box_map(&tree, 80.0, 80.0);
        assert!((boxes["a"].x - 0.0).abs() < 0.5);
        assert!((boxes["a"].y - 0.0).abs() < 0.5);
        assert!(
            (boxes["b"].y - 40.0).abs() < 0.5,
            "second left float must wrap below, got {:?}",
            boxes["b"]
        );
        assert!((boxes["b"].x - 0.0).abs() < 0.5);
    }

    #[test]
    fn float_own_clear_starts_below_packed_same_side() {
        let left = |id: &str, clear: ClearSpec| StyleLayoutNode {
            id: id.into(),
            style: LayoutStyle {
                width: Some(LengthSpec::Px(60.0)),
                height: Some(LengthSpec::Px(40.0)),
                float: FloatSpec::Left,
                clear,
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        };
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            },
            children: vec![left("a", ClearSpec::None), left("b", ClearSpec::Left)],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 80.0);
        assert!((boxes["a"].x - 0.0).abs() < 0.5);
        assert!((boxes["a"].y - 0.0).abs() < 0.5);
        assert!(
            (boxes["b"].y - 40.0).abs() < 0.5 && (boxes["b"].x - 0.0).abs() < 0.5,
            "float with clear:left must start below packed left, not beside it, got {:?}",
            boxes["b"]
        );
    }

    #[test]
    fn ifc_block_sibling_starts_new_line() {
        let inline = |id: &str, x: f32| StyleLayoutNode {
            id: id.into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::InlineBlock),
                width: Some(LengthSpec::Px(x)),
                height: Some(LengthSpec::Px(20.0)),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        };
        let block = StyleLayoutNode {
            id: "mid".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(40.0)),
                height: Some(LengthSpec::Px(20.0)),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        };
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            },
            children: vec![inline("a", 40.0), block, inline("c", 40.0)],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 80.0);
        assert!((boxes["a"].y - 0.0).abs() < 0.5);
        assert!(
            (boxes["mid"].y - 20.0).abs() < 0.5,
            "block sibling must break the IFC line, got {:?}",
            boxes["mid"]
        );
        assert!(
            (boxes["c"].y - 40.0).abs() < 0.5,
            "inline after block starts a new line, got {:?}",
            boxes["c"]
        );
    }

    #[test]
    fn ifc_text_align_start_packs_to_right_in_rtl() {
        let inline = StyleLayoutNode {
            id: "a".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::InlineBlock),
                width: Some(LengthSpec::Px(40.0)),
                height: Some(LengthSpec::Px(20.0)),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        };
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(40.0)),
                dir: Some(nana_ui_core::DirSpec::Rtl),
                text_align: nana_ui_core::TextAlignSpec::Start,
                ..LayoutStyle::default()
            },
            children: vec![inline],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 40.0);
        assert!(
            (boxes["a"].x - 160.0).abs() < 0.5,
            "text-align:start in rtl must pack to inline-start (right), got {:?}",
            boxes["a"]
        );
    }

    fn floated_box(id: &str, side: FloatSpec, width: f32, height: f32) -> StyleLayoutNode {
        StyleLayoutNode {
            id: id.into(),
            style: LayoutStyle {
                width: Some(LengthSpec::Px(width)),
                height: Some(LengthSpec::Px(height)),
                float: side,
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        }
    }

    fn inline_box(id: &str, width: f32, height: f32) -> StyleLayoutNode {
        StyleLayoutNode {
            id: id.into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::InlineBlock),
                width: Some(LengthSpec::Px(width)),
                height: Some(LengthSpec::Px(height)),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        }
    }

    #[test]
    fn ifc_line_box_shrinks_around_float_left() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            },
            children: vec![
                floated_box("fl", FloatSpec::Left, 80.0, 40.0),
                inline_box("a", 50.0, 20.0),
            ],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 80.0);
        assert!((boxes["fl"].x - 0.0).abs() < 0.5);
        assert!(
            (boxes["a"].x - 80.0).abs() < 0.5 && (boxes["a"].y - 0.0).abs() < 0.5,
            "IFC line box must start after the left float, not overlap, got {:?}",
            boxes["a"]
        );
        assert!(
            boxes["a"].x + 0.5 >= boxes["fl"].x + boxes["fl"].width
                || boxes["a"].y + 0.5 >= boxes["fl"].y + boxes["fl"].height,
            "inline vs float must not overlap, a={:?} fl={:?}",
            boxes["a"],
            boxes["fl"]
        );
    }

    #[test]
    fn ifc_inlines_wrap_in_width_beside_float() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            },
            children: vec![
                floated_box("fl", FloatSpec::Left, 80.0, 40.0),
                inline_box("a", 70.0, 20.0),
                inline_box("b", 70.0, 20.0),
            ],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 80.0);
        assert!(
            (boxes["a"].x - 80.0).abs() < 0.5 && (boxes["a"].y - 0.0).abs() < 0.5,
            "first inline sits in the shortened line, got {:?}",
            boxes["a"]
        );
        assert!(
            (boxes["b"].x - 80.0).abs() < 0.5 && (boxes["b"].y - 20.0).abs() < 0.5,
            "70+70 exceeds remaining 120 so b wraps beside the float, got {:?}",
            boxes["b"]
        );
    }

    #[test]
    fn ifc_uses_full_width_below_float() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            },
            children: vec![
                floated_box("fl", FloatSpec::Left, 80.0, 40.0),
                inline_box("a", 70.0, 20.0),
                inline_box("b", 70.0, 20.0),
                inline_box("c", 70.0, 20.0),
            ],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 80.0);
        assert!(
            (boxes["c"].x - 0.0).abs() < 0.5 && (boxes["c"].y - 40.0).abs() < 0.5,
            "below the float the line box is full width, got {:?}",
            boxes["c"]
        );
    }

    #[test]
    fn ifc_oversized_inline_drops_below_float() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            },
            children: vec![
                floated_box("fl", FloatSpec::Left, 80.0, 40.0),
                inline_box("a", 150.0, 20.0),
            ],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 80.0);
        assert!(
            (boxes["a"].x - 0.0).abs() < 0.5 && (boxes["a"].y - 40.0).abs() < 0.5,
            "item wider than remaining width must drop below the float, got {:?}",
            boxes["a"]
        );
    }

    #[test]
    fn ifc_line_box_shrinks_between_left_and_right_floats() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(300.0)),
                height: Some(LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            },
            children: vec![
                floated_box("left", FloatSpec::Left, 80.0, 40.0),
                floated_box("right", FloatSpec::Right, 80.0, 40.0),
                inline_box("a", 40.0, 20.0),
            ],
            text: None,
        };
        let boxes = box_map(&tree, 300.0, 80.0);
        assert!((boxes["left"].x - 0.0).abs() < 0.5);
        assert!((boxes["right"].x - 220.0).abs() < 0.5);
        assert!(
            (boxes["a"].x - 80.0).abs() < 0.5 && (boxes["a"].y - 0.0).abs() < 0.5,
            "line box sits between left and right floats, got {:?}",
            boxes["a"]
        );
        assert!(
            boxes["a"].x + boxes["a"].width <= boxes["right"].x + 0.5,
            "inline must not overlap the right float, a={:?} right={:?}",
            boxes["a"],
            boxes["right"]
        );
    }

    #[test]
    fn in_flow_block_does_not_shrink_beside_float() {
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(80.0)),
                ..LayoutStyle::default()
            },
            children: vec![
                floated_box("fl", FloatSpec::Left, 80.0, 40.0),
                StyleLayoutNode {
                    id: "block".into(),
                    style: LayoutStyle {
                        display: Some(DisplaySpec::Block),
                        width: Some(LengthSpec::Px(100.0)),
                        height: Some(LengthSpec::Px(20.0)),
                        ..LayoutStyle::default()
                    },
                    children: Vec::new(),
                    text: None,
                },
            ],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 80.0);
        assert!(
            (boxes["block"].x - 0.0).abs() < 0.5 && (boxes["block"].y - 0.0).abs() < 0.5,
            "block formatting does not shrink beside floats, got {:?}",
            boxes["block"]
        );
    }

    #[test]
    fn named_line_nth_uses_second_foo() {
        let item = StyleLayoutNode {
            id: "cell".into(),
            style: LayoutStyle {
                height: Some(LengthSpec::Px(40.0)),
                grid_placement: GridPlacement {
                    column_start: GridLine::NthName("foo".into(), 2),
                    column_end: GridLine::Name("foo".into()),
                    ..GridPlacement::default()
                },
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        };
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Grid),
                width: Some(LengthSpec::Px(200.0)),
                height: Some(LengthSpec::Px(40.0)),
                grid_columns: Some(vec![GridTrack::Px(80.0), GridTrack::Px(120.0)]),
                grid_column_line_names: Some(vec![
                    vec!["foo".into()],
                    vec!["foo".into()],
                    vec!["foo".into()],
                ]),
                ..LayoutStyle::default()
            },
            children: vec![item],
            text: None,
        };
        let boxes = box_map(&tree, 200.0, 40.0);
        assert!(
            (boxes["cell"].x - 80.0).abs() < 0.5 && (boxes["cell"].width - 120.0).abs() < 0.5,
            "foo 2 / next foo must be the 120px track, got {:?}",
            boxes["cell"]
        );
    }

    #[test]
    fn auto_fill_nth_named_line_uses_expanded_copies() {
        let item = StyleLayoutNode {
            id: "cell".into(),
            style: LayoutStyle {
                height: Some(LengthSpec::Px(40.0)),
                grid_placement: GridPlacement {
                    column_start: GridLine::NthName("mid".into(), 2),
                    column_end: GridLine::Name("mid".into()),
                    ..GridPlacement::default()
                },
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        };
        let tree = StyleLayoutNode {
            id: "root".into(),
            style: LayoutStyle {
                display: Some(DisplaySpec::Grid),
                width: Some(LengthSpec::Px(240.0)),
                height: Some(LengthSpec::Px(40.0)),
                grid_columns_repeat: Some(GridRepeatAuto {
                    kind: GridTrackListUnsupported::RepeatAutoFill,
                    tracks: vec![GridTrack::Px(80.0)],
                    pattern_line_names: vec![vec!["mid".into()], Vec::new()],
                    ..Default::default()
                }),
                // Pattern stored once — engine must expand, not resolve mid 2
                // against this single copy (which would miss and auto-place at 0).
                grid_column_line_names: Some(vec![vec!["mid".into()], Vec::new()]),
                ..LayoutStyle::default()
            },
            children: vec![item],
            text: None,
        };
        let boxes = box_map(&tree, 240.0, 40.0);
        assert!(
            (boxes["cell"].x - 80.0).abs() < 0.5 && (boxes["cell"].width - 80.0).abs() < 0.5,
            "mid 2 after auto-fit expansion must be the second 80px track, got {:?}",
            boxes["cell"]
        );
    }

    #[test]
    fn grid_auto_slot_overflow_does_not_reuse_origin() {
        let occupied = {
            let mut occ = GridOccupancy::default();
            occ.occupy(0, 0, 1, 2);
            occ
        };
        let (row, col) = search_grid_auto_slot(&occupied, Some(0), None, 1, 1, 2, 1, 0, 0, false);
        assert!(
            !(row == 0 && col == 0),
            "full explicit row must not silently place at (0,0), got ({row},{col})"
        );
        assert_eq!(row, 0);
        assert!(col >= 2, "implicit column past wrap, got {col}");
    }
}
