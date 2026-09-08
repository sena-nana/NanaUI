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
// These caches use internal numeric identities/constraint bits, not external text keys.
use hashbrown::HashMap;
use std::cell::RefCell;
use std::collections::HashSet;
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
        let roots = world.document_roots(document);
        if roots.is_empty() {
            retained.remove_document(document);
            return Ok(Vec::new());
        }
        let retained = retained.documents.entry(document).or_default();
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
                if world.document_of(id) != Some(document) {
                    continue;
                }
                let mut cursor = Some(id);
                while let Some(id) = cursor {
                    if !affected.insert(id) {
                        break;
                    }
                    if world.layout_isolated(id) && retained.boxes.contains_key(&id) {
                        break;
                    }
                    cursor = world.parent_id(id);
                }
            }
        }
        // A content/style change invalidates every previous constraint for
        // the node, including constraints that are not measured this frame.
        for id in &affected {
            retained.intrinsics.remove(id);
        }
        #[cfg(any(test, feature = "benchmark"))]
        plan_stats::note_scope(dirty.len(), affected.len());
        let scope = ScopeContext {
            affected: &affected,
            retained: &*retained,
        };
        let scope_ref = (!force_full).then_some(&scope);
        let mut output = HashMap::with_capacity(nodes.len());
        let mut intrinsic = HashMap::with_capacity(nodes.len());
        let available = Size::new(viewport.width, viewport.height);
        let islands = if force_full {
            Vec::new()
        } else {
            affected
                .iter()
                .copied()
                .filter(|id| {
                    world.layout_isolated(*id)
                        && world
                            .parent_id(*id)
                            .is_some_and(|parent| !affected.contains(&parent))
                        && retained.placements.contains_key(id)
                        && retained.boxes.contains_key(id)
                })
                .collect::<Vec<_>>()
        };
        for &root in &islands {
            let (origin, containing, font) = retained.placements[&root];
            let box_ = retained.boxes[&root];
            place_node_scoped(
                root,
                origin,
                Size::new(box_.width, box_.height),
                containing,
                viewport,
                font,
                &mut nodes,
                &mut intrinsic,
                &mut output,
                scope_ref,
                None,
            )?;
        }
        for root in roots {
            if !force_full && !islands.is_empty() && !affected.contains(&root) {
                continue;
            }
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
        retained.placements.extend(nodes.placements.drain());
        for (id, plan) in nodes.container_plans.drain() {
            match plan {
                Some(plan) => {
                    retained.container_plans.insert(id, plan);
                }
                None => {
                    retained.container_plans.remove(&id);
                }
            }
        }
        for (id, plans) in nodes.measure_plans.drain() {
            if plans.is_empty() {
                retained.measure_plans.remove(&id);
                continue;
            }
            // Merge rather than replace. A container is commonly measured under
            // two constraints per pass but only RE-measured under one of them:
            // the other is answered from its cached plan, which records nothing.
            // Replacing would drop that constraint's plan, so the two slots
            // would alternate instead of holding both.
            let slots = retained.measure_plans.entry(id).or_default();
            for plan in plans.into_plans().collect::<Vec<_>>().into_iter().rev() {
                slots.insert(plan);
            }
        }
        for ((id, width, height), size) in intrinsic {
            retained
                .intrinsics
                .entry(id)
                .or_default()
                .insert(width, height, size);
        }
        retained.materialized_inputs = nodes.materialized;
        // Despawned ids linger in the retained maps; keep them bounded.
        // Scoped passes only materialize a subset, so membership is the live
        // world, not the partial input map.
        let universe = if force_full { nodes.len() } else { world.len() };
        if retained.boxes.len() > universe.saturating_mul(2) {
            #[cfg(any(test, feature = "benchmark"))]
            plan_stats::note_retain_sweep();
            retained.boxes.retain(|id, _| world.contains(*id));
            retained.placements.retain(|id, _| world.contains(*id));
            retained.used_padding.retain(|id, _| world.contains(*id));
            retained.container_plans.retain(|id, _| world.contains(*id));
            retained.measure_plans.retain(|id, _| world.contains(*id));
        }
        if retained.intrinsics.len() > universe.saturating_mul(2) {
            retained.intrinsics.retain(|id, _| world.contains(*id));
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
/// intrinsic sizes keyed like the per-pass intrinsic cache. Each document owns
/// its entries, so a full pass cannot invalidate another window's layout.
#[derive(Default)]
pub struct RetainedLayoutCache {
    documents: HashMap<DocumentId, DocumentLayoutCache>,
}

impl RetainedLayoutCache {
    /// Release all layout state owned by a closed document.
    pub fn remove_document(&mut self, document: DocumentId) {
        self.documents.remove(&document);
    }

    /// Release one deleted node without scanning cached nodes or constraints.
    pub fn remove_node(&mut self, document: DocumentId, id: StableNodeId) {
        if let Some(cache) = self.documents.get_mut(&document) {
            cache.intrinsics.remove(&id);
            cache.boxes.remove(&id);
            cache.placements.remove(&id);
            cache.used_padding.remove(&id);
            cache.container_plans.remove(&id);
            cache.measure_plans.remove(&id);
        }
    }

    pub(crate) fn used_padding(
        &self,
        document: DocumentId,
        id: StableNodeId,
    ) -> Option<nana_ui_core::PaddingSpec> {
        self.documents
            .get(&document)?
            .used_padding
            .get(&id)
            .copied()
    }
}

/// Two constraint variants per node allow measure/place reuse without
/// accumulating a new entry for every pixel of an interactive resize. Eviction
/// only causes remeasurement; the per-pass cache still keeps every constraint.
#[derive(Default)]
struct RetainedIntrinsic {
    measurements: [Option<(u32, u32, Size)>; 2],
}

impl RetainedIntrinsic {
    fn get(&self, width: u32, height: u32) -> Option<Size> {
        self.measurements
            .iter()
            .flatten()
            .find(|(w, h, _)| *w == width && *h == height)
            .map(|(_, _, size)| *size)
    }

    fn insert(&mut self, width: u32, height: u32, size: Size) {
        let next = Some((width, height, size));
        if self.measurements[0].is_some_and(|(w, h, _)| w == width && h == height) {
            self.measurements[0] = next;
            return;
        }
        self.measurements[1] = self.measurements[0];
        self.measurements[0] = next;
    }
}

#[derive(Default)]
struct DocumentLayoutCache {
    intrinsics: HashMap<StableNodeId, RetainedIntrinsic>,
    boxes: HashMap<StableNodeId, LayoutBox>,
    materialized_inputs: usize,
    placements: HashMap<StableNodeId, (Point, Size, f32)>,
    pub(crate) used_padding: HashMap<StableNodeId, nana_ui_core::PaddingSpec>,
    /// Cached in-flow child placement per container. See [`ContainerPlan`].
    container_plans: HashMap<StableNodeId, ContainerPlan>,
    /// Cached intrinsic measurement per content-sized container. See
    /// [`MeasurePlan`].
    measure_plans: HashMap<StableNodeId, MeasurePlanSlots>,
}

impl DocumentLayoutCache {
    fn clear(&mut self) {
        self.intrinsics.clear();
        self.placements.clear();
        self.boxes.clear();
        self.used_padding.clear();
        self.container_plans.clear();
        self.measure_plans.clear();
        self.materialized_inputs = 0;
    }
}

/// Test-only visibility into whether scoped layout is actually incremental.
///
/// The differential harness proves the result is CORRECT; these counters prove
/// it is cheap. Without them a "fix" that quietly relayouts every sibling still
/// passes every equivalence test.
#[cfg(any(test, feature = "benchmark"))]
pub mod plan_stats {
    use std::cell::Cell;

    thread_local! {
        static PLANS_REUSED: Cell<usize> = const { Cell::new(0) };
        static MEASURE_PLANS_REUSED: Cell<usize> = const { Cell::new(0) };
        static CHILDREN_MEASURED: Cell<usize> = const { Cell::new(0) };
        static CONTAINERS_UNCACHEABLE: Cell<usize> = const { Cell::new(0) };
        static DIRTY_SEEDS: Cell<usize> = const { Cell::new(0) };
        static AFFECTED: Cell<usize> = const { Cell::new(0) };
        static RETAIN_SWEEPS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn note_scope(dirty: usize, affected: usize) {
        DIRTY_SEEDS.with(|cell| cell.set(cell.get() + dirty));
        AFFECTED.with(|cell| cell.set(cell.get() + affected));
    }

    pub(crate) fn note_retain_sweep() {
        RETAIN_SWEEPS.with(|cell| cell.set(cell.get() + 1));
    }

    /// Nodes handed to `layout_document_scoped` as the change closure seed.
    #[cfg(feature = "benchmark")]
    pub fn dirty_seeds() -> usize {
        DIRTY_SEEDS.with(Cell::get)
    }

    /// Seeds plus their ancestors: what the pass actually walks.
    #[cfg(feature = "benchmark")]
    pub fn affected() -> usize {
        AFFECTED.with(Cell::get)
    }

    /// Times the retained caches were swept for despawned ids.
    #[cfg(feature = "benchmark")]
    pub fn retain_sweeps() -> usize {
        RETAIN_SWEEPS.with(Cell::get)
    }

    pub fn reset() {
        PLANS_REUSED.with(|cell| cell.set(0));
        MEASURE_PLANS_REUSED.with(|cell| cell.set(0));
        CHILDREN_MEASURED.with(|cell| cell.set(0));
        CONTAINERS_UNCACHEABLE.with(|cell| cell.set(0));
        DIRTY_SEEDS.with(|cell| cell.set(0));
        AFFECTED.with(|cell| cell.set(0));
        RETAIN_SWEEPS.with(|cell| cell.set(0));
    }

    pub(crate) fn note_plan_reused() {
        PLANS_REUSED.with(|cell| cell.set(cell.get() + 1));
    }

    pub(crate) fn note_measure_plan_reused() {
        MEASURE_PLANS_REUSED.with(|cell| cell.set(cell.get() + 1));
    }

    /// Containers that returned a cached intrinsic size instead of re-measuring
    /// their children. See [`super::MeasurePlan`].
    pub fn measure_plans_reused() -> usize {
        MEASURE_PLANS_REUSED.with(Cell::get)
    }

    pub(crate) fn note_child_measured() {
        CHILDREN_MEASURED.with(|cell| cell.set(cell.get() + 1));
    }

    /// A container that took the placement path but could not be cached, so it
    /// will rescan its children on every future frame.
    pub(crate) fn note_container_uncacheable() {
        CONTAINERS_UNCACHEABLE.with(|cell| cell.set(cell.get() + 1));
    }

    #[cfg(feature = "benchmark")]
    pub fn containers_uncacheable() -> usize {
        CONTAINERS_UNCACHEABLE.with(Cell::get)
    }

    pub fn plans_reused() -> usize {
        PLANS_REUSED.with(Cell::get)
    }

    /// Children a container had to intrinsic-measure, counting BOTH sibling
    /// scans: the one in its placement loop and the one in its own intrinsic
    /// measurement. This is the scan that used to make every dirty frame O(N),
    /// and the measure-side half of it is invisible unless both are counted.
    pub fn children_measured() -> usize {
        CHILDREN_MEASURED.with(Cell::get)
    }
}

/// One child's contribution to a cached container placement.
#[derive(Clone)]
struct PlannedChild {
    child: StableNodeId,
    /// The child's own layout style at plan time, compared by pointer. This is
    /// what catches a style edit that moves a child without resizing it --
    /// `margin`, `align_self`, `order`, `flex_grow`.
    style: Arc<nana_ui_core::LayoutStyle>,
    /// Intrinsic size measured BEFORE flex distribution: the pure input the
    /// rest of the container's placement is a function of.
    intrinsic: Size,
    origin: Point,
    size: Size,
    /// Main-axis cursor before this child, i.e. the prefix sum of every
    /// preceding child's outer main extent plus gaps. Lets a suffix replay
    /// start at any index in O(1) instead of re-accumulating from zero.
    cursor_before: f32,
}

/// A container's placement of its in-flow children, cached across passes.
///
/// The whole point of scoped layout is to charge by the change, but a flex
/// container still had to walk every child to discover that none of them
/// moved: `subtree_unchanged` prunes a child's SUBTREE, not the parent's scan
/// of its siblings. So a one-row edit in an N-row list cost O(N).
///
/// The placement of in-flow children is a pure function of the container's own
/// inputs plus, in order, each child's layout style and intrinsic size. When
/// all of those are unchanged the previous result still holds, so the pass can
/// skip straight to the children the change closure actually reaches.
///
/// Only children in that closure need re-checking: a layout-affecting
/// `set_style` marks the node LAYOUT-dirty (`mark_subtree`), which is what puts
/// it in the closure. `UiWorld::children_layout_style_is_local` guards the
/// cases where an ancestor could move a child's style without touching it.
#[derive(Clone)]
struct ContainerPlan {
    origin: Point,
    size: Size,
    containing: Size,
    parent_font_px: f32,
    viewport: LayoutViewport,
    /// The container's effective style, compared by pointer.
    style: Arc<nana_ui_core::LayoutStyle>,
    /// The container's child list, compared by pointer. A structural edit
    /// copy-on-writes this `Arc` (the cache holds a reference, so the world's
    /// `Arc::make_mut` cannot mutate it in place), so a different pointer is a
    /// different list.
    children: Arc<Vec<StableNodeId>>,
    /// Containing block handed to each child.
    content: Size,
    child_font_px: f32,
    /// Available size each child's intrinsic measurement was taken against.
    child_available: Size,
    main_direction: FlexDirection,
    /// Origin of the container's content box.
    content_origin: Point,
    /// Main-axis gap between children.
    gap: f32,
    /// True when this container placed its children as a plain left-to-right
    /// accumulation, so child `i`'s position depends only on the children
    /// before it. Everything that would couple siblings is excluded: wrapping,
    /// a `justify-content` that distributes free space, reversed flow, grid
    /// tracks, auto main margins, baseline or center/end cross alignment, and
    /// any flex grow/shrink redistribution (detected from the data -- every
    /// child's used main size equalled its intrinsic).
    ///
    /// Under that shape a resized child shifts exactly the children after it,
    /// so the pass can keep the prefix and replay only the suffix.
    sequential: bool,
    /// In placement order.
    entries: RefCell<Vec<PlannedChild>>,
    /// `(child, index into entries)`, sorted by child, so the pass can ask
    /// "which of my children are in the change closure?" without walking every
    /// entry. Scanning the entries instead would leave the fast path O(number
    /// of children), which is the cost it exists to remove.
    by_child: Vec<(StableNodeId, u32)>,
}

impl ContainerPlan {
    /// Everything the container's own placement depends on, other than its
    /// children. A mismatch here means the plan is about a different layout.
    #[allow(clippy::too_many_arguments)]
    fn inputs_match(
        &self,
        origin: Point,
        size: Size,
        containing: Size,
        parent_font_px: f32,
        viewport: LayoutViewport,
        style: &Arc<nana_ui_core::LayoutStyle>,
        children: &Arc<Vec<StableNodeId>>,
    ) -> bool {
        self.origin == origin
            && self.size == size
            && self.containing == containing
            && self.parent_font_px == parent_font_px
            && self.viewport == viewport
            && Arc::ptr_eq(&self.style, style)
            && Arc::ptr_eq(&self.children, children)
    }

    fn child_count(&self) -> usize {
        self.by_child.len()
    }

    /// Entry indices for the children the change closure reaches, in placement
    /// order. Driven from the closure (small) rather than the child list.
    fn affected_entries(&self, scope: &ScopeContext<'_>) -> Vec<u32> {
        let mut indices: Vec<u32> = scope
            .affected
            .iter()
            .filter_map(|id| {
                self.by_child
                    .binary_search_by_key(id, |(child, _)| *child)
                    .ok()
                    .map(|slot| self.by_child[slot].1)
            })
            .collect();
        indices.sort_unstable();
        indices
    }
}

/// Whether two layout styles are the same INPUT to layout, as opposed to the
/// same spelling of one.
///
/// `direction` is the one field the engine never reads directly: every use goes
/// through [`used_flow_direction`], which is `unwrap_or(Column)`. So `None` and
/// `Some(Column)` are the same layout, and `Some(Row)` is not.
///
/// That distinction is not academic. Three writers disagree about how to spell
/// a default column: `MessageBridge::register` seeds `direction` from the
/// widget kind, the CSS cascade republishes a style that leaves it `None`, and
/// the Runtime's own `Stack` projection writes `Some(Column)` back. On a
/// 2,000-row Vue list all three run every pointer event, so a plain `==`
/// retires the cached plan on every frame while nothing about the layout has
/// changed. That was the whole of the measure plan's benefit: on that
/// benchmark, `==` left settle at 0.786 ms and the container re-measuring
/// 2,000 children per event; this comparison takes it to 0.630 ms and 5.4.
///
/// The equality is exact, not a tolerance: it accepts exactly the pairs that
/// `used_flow_direction` maps to the same axis, and every other field still has
/// to match outright.
fn layout_inputs_equal(a: &nana_ui_core::LayoutStyle, b: &nana_ui_core::LayoutStyle) -> bool {
    if a == b {
        return true;
    }
    if a.direction.unwrap_or(FlexDirection::Column) != b.direction.unwrap_or(FlexDirection::Column)
    {
        return false;
    }
    // Only reached when the styles differ, so the clone is off the hot path:
    // once per closure child that failed the cheap compare.
    let mut probe = a.clone();
    probe.direction = b.direction;
    probe == *b
}

/// One child's contribution to a cached container measurement.
struct MeasuredChild {
    child: StableNodeId,
    /// The child's effective layout style at plan time, or `None` when the node
    /// was missing. Compared by pointer with a value fallback, for the same
    /// reason as in [`ContainerPlan`]: a host that rebuilds its style objects
    /// every frame hands back a fresh `Arc` holding an identical style.
    style: Option<Arc<nana_ui_core::LayoutStyle>>,
    /// The intrinsic size measured for this child, or `None` for a child the
    /// flow collection dropped (`display:none`, out of flow). A dropped child
    /// contributes nothing to the container's measurement, and it cannot start
    /// contributing without its own style changing -- which the style compare
    /// above catches.
    intrinsic: Option<Size>,
}

/// A container's own intrinsic measurement, cached across passes.
///
/// The measure-side twin of [`ContainerPlan`], and the same shape of hole.
/// [`measure::intrinsic_size_scoped`] short-circuits a node whose width and
/// height both resolve from its own style, which is what stops a dirty frame
/// from re-measuring the whole document. A CONTENT-SIZED container has no such
/// short circuit: its own size is a function of its children, so every affected
/// container re-measured every child -- each one only to hit the retained memo
/// and return the value it already had. Layout invalidation propagates to
/// ancestors, so a single edit puts every content-sized container above it on
/// that path, and the frame is O(number of children) again.
///
/// The measurement is a pure function of the container's own inputs (style,
/// child list, text metrics, available size, viewport, inherited font size,
/// parent flow direction) plus, per child, that child's layout style and its
/// intrinsic size under the recorded available size. When all of those are
/// unchanged the previous result still holds.
///
/// Only children the change closure reaches need re-checking, and the check is
/// driven FROM the closure: a container looks each affected id up in its own
/// sorted entries, rather than walking its children looking for affected ones.
/// Walking the children would leave the fast path O(number of children), which
/// is the cost this exists to remove.
///
/// Entries are sorted by child id rather than kept in flow order: the container
/// is either reusing the whole cached measurement or recomputing it from
/// scratch, and neither needs the order.
///
/// This is a cache of its own, NOT a relaxation of the
/// `retained.intrinsics.remove` in `layout_document_scoped`. That removal is
/// still right and still happens: an affected node's memo holds entries for
/// constraint combinations this frame will not measure, and those really are
/// stale. What a plan caches is the container's result under ONE recorded
/// constraint, re-validated against the closure before it is used, so the two
/// do not overlap.
struct MeasurePlan {
    /// Constraint the container was measured against. This is the part of the
    /// per-pass cache key that the plan, keyed by id alone, has to carry.
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    /// The container's effective style, compared by pointer with a value
    /// fallback.
    style: Arc<nana_ui_core::LayoutStyle>,
    /// The container's child list, compared by pointer. A structural edit
    /// copy-on-writes this `Arc`, so a different pointer is a different list.
    children: Arc<Vec<StableNodeId>>,
    /// The container's own shaped text, which competes with the children for
    /// the content size.
    text_metrics: Option<crate::TextMetrics>,
    /// Available size every in-flow child was measured against. One value for
    /// all of them: the per-child variant only arises on the grid paths, which
    /// are not cached.
    child_available: Size,
    /// Flow direction handed to each child as its `parent_direction`.
    child_direction: FlexDirection,
    /// Sorted by child id.
    entries: Vec<MeasuredChild>,
    /// What the measurement produced.
    size: Size,
}

/// The measure plans retained for one container.
///
/// A container is commonly measured TWICE per pass under two different
/// constraints: once from its parent's own intrinsic measurement, against the
/// parent's available content box, and once from its parent's placement,
/// against the parent's USED content box. Under an auto-height ancestor those
/// two differ, so a single slot is written by one call and missed by the other,
/// every pass, forever. Two slots is the same answer -- and the same reason --
/// as [`RetainedIntrinsic`].
#[derive(Default)]
struct MeasurePlanSlots {
    slots: [Option<MeasurePlan>; 2],
}

impl MeasurePlanSlots {
    fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    /// Picks the slot recorded under this constraint. Selection only, not a
    /// correctness check: `MeasurePlan::inputs_match` compares `available`
    /// again, so handing back the wrong slot costs a recompute, never a stale
    /// answer.
    fn get(&self, available: Size) -> Option<&MeasurePlan> {
        self.slots
            .iter()
            .flatten()
            .find(|plan| plan.available == available)
    }

    fn insert(&mut self, plan: MeasurePlan) {
        if self.slots[0]
            .as_ref()
            .is_some_and(|held| held.available == plan.available)
        {
            self.slots[0] = Some(plan);
            return;
        }
        self.slots.swap(0, 1);
        self.slots[0] = Some(plan);
    }

    fn clear(&mut self) {
        self.slots = [None, None];
    }

    /// The plans this holder carries, most recent first.
    fn into_plans(self) -> impl Iterator<Item = MeasurePlan> {
        self.slots.into_iter().flatten()
    }
}

impl MeasurePlan {
    /// Everything the measurement depends on other than the children.
    #[allow(clippy::too_many_arguments)]
    fn inputs_match(
        &self,
        available: Size,
        parent_direction: Option<FlexDirection>,
        viewport: LayoutViewport,
        parent_font_px: f32,
        style: &Arc<nana_ui_core::LayoutStyle>,
        children: &Arc<Vec<StableNodeId>>,
        text_metrics: Option<crate::TextMetrics>,
    ) -> bool {
        self.available == available
            && self.parent_direction == parent_direction
            && self.viewport == viewport
            && self.parent_font_px == parent_font_px
            && self.text_metrics == text_metrics
            && Arc::ptr_eq(&self.children, children)
            && (Arc::ptr_eq(&self.style, style) || layout_inputs_equal(&self.style, style))
    }

    fn entry(&self, child: StableNodeId) -> Option<&MeasuredChild> {
        self.entries
            .binary_search_by_key(&child, |entry| entry.child)
            .ok()
            .map(|slot| &self.entries[slot])
    }
}

/// On-demand `LayoutInput` cache. A miss loads exactly that id from `UiWorld`.
struct LayoutInputMap<'a> {
    world: &'a UiWorld,
    nodes: HashMap<StableNodeId, LayoutInput>,
    /// Effective styles for nodes this pass never materialized into `nodes`.
    ///
    /// A scoped pass prefetches nothing, so an unchanged sibling is reached
    /// only through [`Self::style`] -- and reached repeatedly: flex main-axis
    /// distribution, the baseline fold, and the cross-axis fold each ask for
    /// the same child's style. `UiWorld::effective_layout_style` is not a
    /// field read; it is several hash lookups plus an `Arc` clone, and a
    /// hidden or overlay-hosted node also pays an `Arc::make_mut` clone of the
    /// whole `LayoutStyle`. Resolving that once per node per pass is exact,
    /// not an approximation: `layout_document_scoped` borrows the world
    /// immutably for the entire pass, so no resolution can change underneath
    /// this map.
    styles: RefCell<HashMap<StableNodeId, Option<Arc<nana_ui_core::LayoutStyle>>>>,
    materialized: usize,
    placements: HashMap<StableNodeId, (Point, Size, f32)>,
    used_padding: HashMap<StableNodeId, nana_ui_core::PaddingSpec>,
    /// Container plans rebuilt this pass. Merged into the retained cache at the
    /// end; containers that took the fast path record nothing, so their
    /// existing plan simply stays. `None` retires a plan recorded when the
    /// container was still on the cacheable path.
    container_plans: HashMap<StableNodeId, Option<ContainerPlan>>,
    /// Measure plans rebuilt this pass, merged the same way. An entry that
    /// ends the pass empty retires the container's retained plans.
    measure_plans: HashMap<StableNodeId, MeasurePlanSlots>,
}

impl<'a> LayoutInputMap<'a> {
    fn new(world: &'a UiWorld) -> Self {
        Self {
            world,
            nodes: HashMap::new(),
            styles: RefCell::new(HashMap::new()),
            materialized: 0,
            placements: HashMap::new(),
            used_padding: HashMap::new(),
            container_plans: HashMap::new(),
            measure_plans: HashMap::new(),
        }
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }

    fn prefetch(&mut self, ids: &[StableNodeId]) -> Result<(), UiWorldError> {
        // Full passes prefetch once, immediately after constructing this map.
        // Fill the final cache directly: materializing a temporary Vec and
        // then moving every input into a HashMap doubles the container work on
        // the cold path that large documents pay most often.
        debug_assert!(self.nodes.is_empty());
        if !ids.is_empty() {
            self.world.record_hot_path_allocation(
                1,
                ids.len().saturating_mul(std::mem::size_of::<LayoutInput>()),
            );
        }
        let mut nodes = HashMap::with_capacity(ids.len());
        for &id in ids {
            let input = self.world.layout_input(id)?;
            nodes.insert(id, input);
        }
        self.materialized = nodes.len();
        self.nodes = nodes;
        Ok(())
    }

    fn get(&mut self, id: StableNodeId) -> Result<Option<&LayoutInput>, UiWorldError> {
        match self.nodes.entry(id) {
            hashbrown::hash_map::Entry::Occupied(entry) => Ok(Some(entry.into_mut())),
            hashbrown::hash_map::Entry::Vacant(entry) => {
                let input = match self.world.layout_input(id) {
                    Ok(input) => input,
                    Err(UiWorldError::MissingNode(_)) => return Ok(None),
                    Err(error) => return Err(error),
                };
                self.materialized = self.materialized.saturating_add(1);
                Ok(Some(entry.insert(input)))
            }
        }
    }

    /// Style for classifying / measuring siblings without assembling `LayoutInput`.
    fn style(&self, id: StableNodeId) -> Option<Arc<nana_ui_core::LayoutStyle>> {
        if let Some(node) = self.nodes.get(&id) {
            return Some(Arc::clone(&node.style));
        }
        if let Some(cached) = self.styles.borrow().get(&id) {
            return cached.clone();
        }
        let resolved = self.world.layout_style(id);
        self.styles.borrow_mut().insert(id, resolved.clone());
        resolved
    }

    fn text_ascent(&self, id: StableNodeId) -> Option<f32> {
        self.nodes
            .get(&id)
            .and_then(|node| node.text_metrics)
            .and_then(|metrics| metrics.ascent)
    }
}

struct ScopeContext<'a> {
    affected: &'a HashSet<StableNodeId>,
    retained: &'a DocumentLayoutCache,
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
    let order_of = |id: StableNodeId| nodes.style(id).map(|style| style.order).unwrap_or(0);
    // Every key costs a map lookup plus an `Arc` clone, so resolve each one at
    // most once. Siblings almost always keep the default order, and a stable
    // sort on all-equal keys is a no-op, so scan for that case and skip.
    if ids.iter().all(|id| order_of(*id) == 0) {
        return;
    }
    // `sort_by_key` re-evaluates the key on every comparison; `sort_by_cached_key`
    // is likewise stable but evaluates it once per element.
    ids.sort_by_cached_key(|id| order_of(*id));
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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Point {
    x: f32,
    y: f32,
}

impl Point {
    const ZERO: Self = Self { x: 0.0, y: 0.0 };
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
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
