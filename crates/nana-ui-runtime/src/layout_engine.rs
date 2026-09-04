mod grid;
use grid::*;
mod flow;
use flow::*;
mod measure;
use measure::*;
mod placement;
use placement::*;
mod inline;
use inline::*;
mod flex;
use flex::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nana_ui_core::box_layout::text_line_box_height_px;
use nana_ui_core::{
    AlignSpec, BoxSizing, ClearSpec, DisplaySpec, FlexDirection, FlexWrap, FloatSpec,
    FontSizeContext, GridAutoFlow, GridLine, GridPlacement, GridRepeatAuto, GridTemplateAreas,
    GridTrack, JustifySpec, LayoutStyle, LengthSpec, PositionSpec, TextAlignSpec, WritingModeSpec,
    resolve_grid_track_sizes,
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
                None,
            )?;
        }
        // Publish recomputed boxes from the placed set; no document_order walk.
        let mut emitted = output.into_iter().collect::<Vec<_>>();
        emitted.sort_unstable_by_key(|(id, _)| *id);
        for (id, box_) in &emitted {
            retained.boxes.insert(*id, *box_);
        }
        retained.used_padding.extend(nodes.used_padding.drain());
        retained.intrinsics.extend(intrinsic);
        retained.materialized_inputs = nodes.materialized;
        // Despawned ids linger in the retained maps; keep them bounded.
        // Scoped passes only materialize a subset, so membership is the live
        // world, not the partial input map.
        let universe = if force_full { nodes.len() } else { world.len() };
        if retained.boxes.len() > universe.saturating_mul(2) {
            retained.boxes.retain(|id, _| world.contains(*id));
            retained.used_padding.retain(|id, _| world.contains(*id));
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
    pub(crate) used_padding: HashMap<StableNodeId, nana_ui_core::PaddingSpec>,
}

impl RetainedLayoutCache {
    fn clear(&mut self) {
        self.intrinsics.clear();
        self.boxes.clear();
        self.used_padding.clear();
        self.materialized_inputs = 0;
    }
}

/// On-demand `LayoutInput` cache. A miss loads exactly that id from `UiWorld`.
struct LayoutInputMap<'a> {
    world: &'a UiWorld,
    nodes: HashMap<StableNodeId, LayoutInput>,
    materialized: usize,
    used_padding: HashMap<StableNodeId, nana_ui_core::PaddingSpec>,
}

impl<'a> LayoutInputMap<'a> {
    fn new(world: &'a UiWorld) -> Self {
        Self {
            world,
            nodes: HashMap::new(),
            materialized: 0,
            used_padding: HashMap::new(),
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

    fn text_ascent(&self, id: StableNodeId) -> Option<f32> {
        self.nodes
            .get(&id)
            .and_then(|node| node.text_metrics)
            .and_then(|metrics| metrics.ascent)
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
    scope.retained.used_padding.get(&child).copied()
        == Some(child_style.resolved_padding_against_fonts(Some(containing.width), child_fonts))
        && cached.x == origin.x + relative_x
        && cached.y == origin.y + relative_y
        && cached.width == size.width
        && cached.height == size.height
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
        || style.is_subgrid_columns()
        || style.is_subgrid_rows()
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

/// Physical main axis for this formatting context.
///
/// IFC always follows the writing-mode inline axis. Flex `row`/`column` are
/// remapped through writing-mode; block containers without an explicit
/// `flex-direction` stack along the block axis.
fn used_flow_direction(style: &LayoutStyle, ifc: bool) -> FlexDirection {
    let mode = style.resolved_writing_mode();
    if ifc {
        return mode.inline_flex_direction();
    }
    let css = style.direction.unwrap_or(FlexDirection::Column);
    mode.physical_flex_direction(css)
}

/// `vertical-rl` packs lines from the physical right (block-start) when the
/// cross axis is horizontal.
fn pack_block_from_end(style: &LayoutStyle, direction: FlexDirection) -> bool {
    style.resolved_writing_mode().block_start_is_right() && direction.is_column()
}

fn ifc_justify(align: TextAlignSpec, rtl: bool, writing_mode: WritingModeSpec) -> JustifySpec {
    // Vertical writing-mode skips RTL so inline-start stays physical top.
    align.to_justify(rtl && !writing_mode.is_vertical())
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
        Some(s) if s.is_full_percent_fill() => None,
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

fn resolve_axis(
    spec: Option<LengthSpec>,
    percent_base: f32,
    fill_base: f32,
    viewport: LayoutViewport,
    fonts: FontSizeContext,
) -> Option<f32> {
    spec.and_then(|value| {
        if value == LengthSpec::Fill {
            Some(fill_base)
        } else {
            value
                .resolve_with_fonts(
                    Some(percent_base),
                    Some((viewport.width, viewport.height)),
                    fonts,
                )
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
mod tests;
