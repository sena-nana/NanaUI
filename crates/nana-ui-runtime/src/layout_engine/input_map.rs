//! Per-pass layout input map, style facts, scratch pools, and intrinsic memo.

use std::cell::{Cell, RefCell};

use super::*;

pub(super) type PassInput<'a> = crate::world::LayoutInputView<'a>;

/// What a measurement asks of a node's layout style before it measures
/// anything, decided once per style: a page holds thousands of boxes and a
/// handful of distinct (interned) styles, and each of these reads a dozen
/// scattered fields of a 2.6 KB style. A pass keeps them beside each node's
/// input ([`LayoutInputMap::facts`]); every answer is the function it names,
/// for a horizontal page where the function takes a writing context.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StyleFacts(u16);

impl StyleFacts {
    /// `viewport_axes().width` / `.height`.
    pub(super) const VIEWPORT_WIDTH: u16 = 1 << 0;
    pub(super) const VIEWPORT_HEIGHT: u16 = 1 << 1;
    /// [`measure::sizes_own_axes`] on a horizontal page.
    pub(super) const OWN_WIDTH: u16 = 1 << 2;
    pub(super) const OWN_HEIGHT: u16 = 1 << 3;
    /// [`measure::block_containing_size_affects_leaf`] on a horizontal page.
    pub(super) const LEAF_READS_BLOCK: u16 = 1 << 4;
    /// [`measure::plain_childless_content`].
    pub(super) const PLAIN_CHILDLESS: u16 = 1 << 5;
    /// The two style questions a measurement's constraint class asks: a
    /// usable aspect ratio, a full percent fill on either axis.
    pub(super) const ASPECT_RATIO: u16 = 1 << 6;
    pub(super) const FULL_PERCENT_FILL: u16 = 1 << 7;
    /// Set on every answer [`Self::of`] gives.
    pub(super) const KNOWN: u16 = 1 << 8;

    /// Not decided yet: [`LayoutInputMap::facts`] decides it.
    pub(crate) const UNKNOWN: Self = Self(0);

    pub(crate) fn of(style: &LayoutStyle) -> Self {
        let horizontal = nana_ui_core::WritingContext::default();
        let viewport = style.viewport_axes();
        let (own_width, own_height) = measure::sizes_own_axes(style, horizontal);
        let bit = |on: bool, flag: u16| if on { flag } else { 0 };
        Self(
            Self::KNOWN
                | bit(viewport.width, Self::VIEWPORT_WIDTH)
                | bit(viewport.height, Self::VIEWPORT_HEIGHT)
                | bit(own_width, Self::OWN_WIDTH)
                | bit(own_height, Self::OWN_HEIGHT)
                | bit(
                    measure::block_containing_size_affects_leaf(style, horizontal),
                    Self::LEAF_READS_BLOCK,
                )
                | bit(
                    measure::plain_childless_content(style),
                    Self::PLAIN_CHILDLESS,
                )
                | bit(
                    style
                        .aspect_ratio
                        .is_some_and(|ratio| ratio.is_finite() && ratio > 0.0),
                    Self::ASPECT_RATIO,
                )
                | bit(
                    style.width.is_some_and(LengthSpec::is_full_percent_fill)
                        || style.height.is_some_and(LengthSpec::is_full_percent_fill),
                    Self::FULL_PERCENT_FILL,
                ),
        )
    }

    pub(super) fn has(self, flag: u16) -> bool {
        self.0 & flag != 0
    }

    /// [`viewport_basis`].
    pub(super) fn viewport_basis(self, viewport: LayoutViewport) -> LayoutViewport {
        LayoutViewport::new(
            if self.has(Self::VIEWPORT_WIDTH) {
                viewport.width
            } else {
                0.0
            },
            if self.has(Self::VIEWPORT_HEIGHT) {
                viewport.height
            } else {
                0.0
            },
        )
    }

    /// [`measure::sizes_own_axes`].
    pub(super) fn sizes_own_axes(self, writing: nana_ui_core::WritingContext) -> (bool, bool) {
        if writing.is_vertical() {
            return (false, false);
        }
        (self.has(Self::OWN_WIDTH), self.has(Self::OWN_HEIGHT))
    }

    /// [`measure::block_containing_size_affects_leaf`].
    pub(super) fn block_containing_size_affects_leaf(
        self,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        writing.is_vertical() || self.has(Self::LEAF_READS_BLOCK)
    }

    /// [`measure::plain_childless_content`].
    pub(super) fn plain_childless_content(self) -> bool {
        self.has(Self::PLAIN_CHILDLESS)
    }

    /// The constraint class a measurement under `available` is keyed by.
    pub(super) fn constraint_class(self, available: Size) -> crate::ConstraintClass {
        let extents = (available.width.max(0.0), available.height.max(0.0));
        if self.has(Self::ASPECT_RATIO) {
            crate::ConstraintClass::aspect_ratio(extents.0, extents.1)
        } else if self.has(Self::FULL_PERCENT_FILL) {
            crate::ConstraintClass::fill(extents.0, extents.1)
        } else {
            crate::ConstraintClass::percentage_cb(extents.0, extents.1)
        }
    }
}

/// The boxes a pass placed, by node. A full pass places every node of the
/// document: a dense table ([`crate::store::DenseNodeMap`]) sized once for
/// the document, which hands the boxes back in id order and becomes the
/// retained table whole. A scoped pass places a few nodes, possibly spread
/// over the whole id range (the rows below an edit near the top), and keeps
/// them in a hash map: a dense table would grow to the highest id and be
/// allocated, zeroed and walked every frame.
pub(super) enum PassOutput {
    Dense(crate::store::DenseNodeMap<LayoutBox>),
    Sparse(crate::NodeMap<LayoutBox>),
}

impl Default for PassOutput {
    fn default() -> Self {
        Self::Sparse(crate::NodeMap::default())
    }
}

impl PassOutput {
    /// A dense table with room for every id of `ids`.
    pub(super) fn with_highest_of(ids: &[StableNodeId]) -> Self {
        Self::Dense(crate::store::DenseNodeMap::with_highest_of(ids))
    }

    pub(super) fn insert(&mut self, id: StableNodeId, layout: LayoutBox) -> Option<LayoutBox> {
        match self {
            Self::Dense(map) => map.insert(id, layout),
            Self::Sparse(map) => map.insert(id, layout),
        }
    }

    pub(super) fn get(&self, id: &StableNodeId) -> Option<&LayoutBox> {
        match self {
            Self::Dense(map) => map.get(id),
            Self::Sparse(map) => map.get(id),
        }
    }

    pub(super) fn contains_key(&self, id: &StableNodeId) -> bool {
        self.get(id).is_some()
    }

    pub(super) fn remove(&mut self, id: &StableNodeId) -> Option<LayoutBox> {
        match self {
            Self::Dense(map) => map.remove(id),
            Self::Sparse(map) => map.remove(id),
        }
    }

    /// `id`'s box, an empty one placed first if it has none.
    pub(super) fn or_default(&mut self, id: StableNodeId) -> &mut LayoutBox {
        match self {
            Self::Dense(map) => map.entry(id).or_default(),
            Self::Sparse(map) => map.entry(id).or_default(),
        }
    }

    /// Every box, in id order.
    pub(super) fn sorted(&self) -> Vec<(StableNodeId, LayoutBox)> {
        let mut emitted: Vec<_> = match self {
            Self::Dense(map) => map.iter().map(|(id, &layout)| (id, layout)).collect(),
            Self::Sparse(map) => map.iter().map(|(&id, &layout)| (id, layout)).collect(),
        };
        emitted.sort_unstable_by_key(|(id, _)| *id);
        emitted
    }

    /// Move every box into `boxes`: a dense table whole when `boxes` is
    /// empty (a first layout).
    pub(super) fn move_into(self, boxes: &mut crate::store::DenseNodeMap<LayoutBox>) {
        match self {
            Self::Dense(mut map) => boxes.absorb(&mut map),
            Self::Sparse(map) => boxes.extend(map),
        }
    }
}

/// Working lists a pass's walks reuse instead of allocating one per
/// container: each container's walk takes the lists it needs (its flow,
/// its children's sizes, the children it descends into) and hands them back
/// once it is done, so a pass over thousands of containers allocates about
/// as many lists as its tree is deep. A list a walk keeps (a plan's
/// entries) or drops on an early return simply leaves the pool.
#[derive(Default)]
pub(super) struct PassScratch<'a> {
    pub(super) ids: Vec<Vec<StableNodeId>>,
    pub(super) sizes: Vec<Vec<Size>>,
    pub(super) descend: Vec<Vec<placement::Descend>>,
    pub(super) arranged: Vec<Vec<placement::ArrangedChild<'a>>>,
    pub(super) indices: Vec<Vec<usize>>,
    pub(super) line_slots: Vec<Vec<flow::LineBoxSlot>>,
    pub(super) lines: Vec<Vec<placement::ArrangedLine>>,
}

/// Lists a pool keeps: deeper than any walk nests on one stack.
pub(super) const SCRATCH_KEPT: usize = 128;

pub(super) fn take_scratch<T>(pool: &mut Vec<Vec<T>>) -> Vec<T> {
    pool.pop().unwrap_or_default()
}

pub(super) fn give_scratch<T>(pool: &mut Vec<Vec<T>>, mut list: Vec<T>) {
    if list.capacity() > 0 && pool.len() < SCRATCH_KEPT {
        list.clear();
        pool.push(list);
    }
}

impl<'a> PassScratch<'a> {
    pub(super) fn ids(&mut self) -> Vec<StableNodeId> {
        take_scratch(&mut self.ids)
    }

    pub(super) fn give_ids(&mut self, list: Vec<StableNodeId>) {
        give_scratch(&mut self.ids, list);
    }

    pub(super) fn sizes(&mut self, capacity: usize) -> Vec<Size> {
        let mut list = take_scratch(&mut self.sizes);
        list.reserve(capacity);
        list
    }

    pub(super) fn give_sizes(&mut self, list: Vec<Size>) {
        give_scratch(&mut self.sizes, list);
    }

    pub(super) fn descend(&mut self) -> Vec<placement::Descend> {
        take_scratch(&mut self.descend)
    }

    pub(super) fn give_descend(&mut self, list: Vec<placement::Descend>) {
        give_scratch(&mut self.descend, list);
    }

    pub(super) fn arranged(&mut self, capacity: usize) -> Vec<placement::ArrangedChild<'a>> {
        let mut list = take_scratch(&mut self.arranged);
        list.reserve(capacity);
        list
    }

    pub(super) fn give_arranged(&mut self, list: Vec<placement::ArrangedChild<'a>>) {
        give_scratch(&mut self.arranged, list);
    }

    pub(super) fn indices(&mut self) -> Vec<usize> {
        take_scratch(&mut self.indices)
    }

    pub(super) fn give_indices(&mut self, list: Vec<usize>) {
        give_scratch(&mut self.indices, list);
    }

    pub(super) fn lines(&mut self) -> Vec<placement::ArrangedLine> {
        take_scratch(&mut self.lines)
    }

    pub(super) fn give_lines(&mut self, list: Vec<placement::ArrangedLine>) {
        give_scratch(&mut self.lines, list);
    }

    pub(super) fn line_slots(&mut self) -> Vec<flow::LineBoxSlot> {
        take_scratch(&mut self.line_slots)
    }

    pub(super) fn give_line_slots(&mut self, list: Vec<flow::LineBoxSlot>) {
        give_scratch(&mut self.line_slots, list);
    }
}

/// On-demand `LayoutInput` cache. A miss loads exactly that id from `UiWorld`.
pub(super) struct LayoutInputMap<'a> {
    pub(super) inline_objects: HashMap<StableNodeId, Arc<[nana_text::InlineObject]>>,
    /// The frame of each positioned box this pass placed, and of each box
    /// back in flow that had another one: what its box is measured from.
    pub(super) frames: crate::NodeMap<crate::LayoutFrame>,
    pub(super) world: &'a UiWorld,
    pub(super) nodes: crate::store::DenseNodeMap<PassInput<'a>>,
    /// Effective styles for nodes this pass never materialized into `nodes`.
    ///
    /// A scoped pass prefetches nothing, so an unchanged sibling is reached
    /// only through [`Self::style`] -- and reached repeatedly: flex main-axis
    /// distribution, the baseline fold, and the cross-axis fold each ask for
    /// the same child's style. `UiWorld::effective_layout_style` is not a
    /// field read; it is several hash lookups plus an `Arc` clone, and a
    /// hidden or overlay-hosted node also pays an `Arc::make_mut` clone of the
    /// whole `LayoutStyle`. Resolving that once per node per pass is exact,
    /// not an approximation: `layout_document_with_frontier` borrows the world
    /// immutably for the entire pass, so no resolution can change underneath
    /// this map.
    pub(super) styles: RefCell<crate::store::DenseNodeMap<Option<Arc<nana_ui_core::LayoutStyle>>>>,
    pub(super) materialized: usize,
    /// [`StyleFacts`] of the styles this pass met, most recent first: a
    /// handful of interned styles serve every box. Each entry holds its
    /// style, so an address is never reused while it is here.
    pub(super) style_facts: RefCell<Vec<(Arc<nana_ui_core::LayoutStyle>, StyleFacts)>>,
    pub(super) scratch: PassScratch<'a>,
    pub(super) placements: crate::store::DenseNodeMap<(Point, Size, f32)>,
    pub(super) used_padding: crate::store::DenseNodeMap<nana_ui_core::PaddingSpec>,
    pub(super) omitted_boxes: crate::NodeSet,
    pub(super) unomitted_boxes: crate::NodeSet,
    pub(super) cleared_padding: crate::NodeSet,
    pub(super) far_start: crate::store::DenseNodeMap<[bool; 2]>,
    /// Formatting context each container measured or placed this pass ran.
    pub(super) contexts: crate::store::DenseNodeMap<crate::FormattingContextKind>,
    /// Container plans rebuilt this pass. Merged into the retained cache at the
    /// end; containers that took the fast path record nothing, so their
    /// existing plan simply stays. A retired one drops a plan recorded when
    /// the container was still on the cacheable path.
    pub(super) container_plans: PassContainerPlans,
    /// Measure plans rebuilt this pass, merged the same way. An entry that
    /// ends the pass empty retires the container's retained plans.
    pub(super) measure_plans: crate::store::DenseNodeMap<MeasurePlanSlots>,
    /// First and last baselines of boxes whose line boxes are all their
    /// descendants', by node and used border-box size, derived this pass from
    /// where those descendants sit ([`inline::descendant_baselines`]). `None`:
    /// a context the derivation does not lay out, which keeps the
    /// approximation of [`Self::baseline_metrics`].
    pub(super) derived_baselines: HashMap<(StableNodeId, u32, u32), BaselineMetrics>,
    /// Which boxes hold an in-flow line box, settled once per box per pass
    /// ([`UiWorld::holds_line_box_in`]); exact for the same reason as
    /// [`Self::styles`].
    pub(super) line_boxes: RefCell<crate::NodeMap<bool>>,
    /// Boxes this pass's line-box questions settled by walking.
    pub(super) line_boxes_settled: Cell<usize>,
}

/// Whether a seed may change which boxes hold an in-flow line box
/// ([`UiWorld::holds_line_box_in`]), at its node or at an ancestor.
///
/// The answer reads a box's own text, text layout, inline record and visual,
/// its child list, and of each child whether it omits its box, leaves the
/// flow (positioned or floated), is inline-level or is replaced. A style
/// write classifies every changed field ([`nana_ui_core::LayoutStyleChange`]):
/// `display` and isolation as flow, `position` and `float` as position,
/// `hidden` as visibility; text, child lists, inline atoms and replaced
/// content come with reasons and fields of their own. Only a seed that
/// sized, spaced, aligned, scrolled or turned a box, or changed what it
/// takes pointer input for, for a reason that names none of those, keeps the
/// answers: everything else drops them.
pub(crate) fn may_move_line_boxes(invalidation: nana_ui_core::LayoutInvalidation) -> bool {
    use nana_ui_core::{InvalidationKind, InvalidationReason, LayoutStyleChange};
    const KEEPS_FIELDS: LayoutStyleChange = LayoutStyleChange::SIZING_BOTH
        .union(LayoutStyleChange::SPACING)
        .union(LayoutStyleChange::ALIGNMENT)
        .union(LayoutStyleChange::SCROLL)
        .union(LayoutStyleChange::TRANSFORM)
        .union(LayoutStyleChange::INTERACTION)
        .union(LayoutStyleChange::CURSOR)
        .union(LayoutStyleChange::INERT)
        .union(LayoutStyleChange::PAINT)
        .union(LayoutStyleChange::STACKING)
        .union(LayoutStyleChange::CONTAINER);
    const KEEPS_REASONS: InvalidationReason = InvalidationReason::STYLE
        .union(InvalidationReason::PARENT_CONSTRAINT)
        .union(InvalidationReason::CONTAINING_BLOCK)
        .union(InvalidationReason::SIBLING)
        .union(InvalidationReason::CONTEXT)
        .union(InvalidationReason::SCROLL)
        .union(InvalidationReason::VIEWPORT);
    // A seed that names no field or no reason says nothing of what it kept.
    invalidation.changed_inputs == LayoutStyleChange::NONE
        || invalidation.reason.is_empty()
        || !KEEPS_FIELDS.contains(invalidation.changed_inputs)
        || !KEEPS_REASONS.contains(invalidation.reason)
        || invalidation.kind.intersects(InvalidationKind::TOPOLOGY)
}

/// Whether a seed may change, besides its own node's answer, whether boxes
/// below it omit theirs: a child list, a box shown or hidden, a cause it
/// does not name. The boxes below derive hidden from their ancestors
/// (`UiWorld::effective_layout_style`), so their own answers move without a
/// seed of their own.
pub(crate) fn moves_line_boxes_below(invalidation: nana_ui_core::LayoutInvalidation) -> bool {
    use nana_ui_core::{InvalidationKind, InvalidationReason, LayoutStyleChange};
    invalidation.kind.intersects(InvalidationKind::TOPOLOGY)
        || invalidation
            .changed_inputs
            .intersects(LayoutStyleChange::VISIBILITY)
        || invalidation.reason.intersects(InvalidationReason::UNKNOWN)
}

pub(super) fn derived_baseline_key(id: StableNodeId, used: Size) -> (StableNodeId, u32, u32) {
    (id, used.width.to_bits(), used.height.to_bits())
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct BaselineMetrics {
    pub(super) first: Option<f32>,
    pub(super) last: Option<f32>,
}

impl BaselineMetrics {
    pub(super) fn get(self, which: crate::Baseline) -> Option<f32> {
        match which {
            crate::Baseline::First => self.first,
            crate::Baseline::Last => self.last,
        }
    }

    pub(super) fn set(&mut self, which: crate::Baseline, baseline: f32) {
        match which {
            crate::Baseline::First => self.first = Some(baseline),
            crate::Baseline::Last => self.last = Some(baseline),
        }
    }
}

impl<'a> LayoutInputMap<'a> {
    /// Execution and inspection consume the same participation mapping. This
    /// reads only node content metadata; it does not materialize LayoutInput.
    pub(super) fn participation(
        &self,
        id: StableNodeId,
        style: &LayoutStyle,
        parent: crate::FormattingContextKind,
    ) -> crate::ParticipationKind {
        crate::ParticipationKind::classify(
            parent,
            style,
            self.world
                .layout_content_kind(id)
                .unwrap_or(crate::LayoutContentKind::Empty),
        )
    }
    pub(super) fn new(world: &'a UiWorld) -> Self {
        Self {
            inline_objects: HashMap::new(),
            frames: crate::NodeMap::default(),
            world,
            nodes: crate::store::DenseNodeMap::default(),
            styles: RefCell::default(),
            materialized: 0,
            style_facts: RefCell::default(),
            scratch: PassScratch::default(),
            placements: Default::default(),
            used_padding: Default::default(),
            omitted_boxes: Default::default(),
            unomitted_boxes: Default::default(),
            cleared_padding: Default::default(),
            far_start: Default::default(),
            contexts: Default::default(),
            container_plans: PassContainerPlans::default(),
            measure_plans: Default::default(),
            derived_baselines: HashMap::new(),
            line_boxes: RefCell::default(),
            line_boxes_settled: Cell::new(0),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn prefetch(&mut self, ids: &[StableNodeId]) -> Result<(), UiWorldError> {
        // Full passes prefetch once, immediately after constructing this map.
        // Fill the final cache directly: materializing a temporary Vec and
        // then moving every input into a HashMap doubles the container work on
        // the cold path that large documents pay most often.
        debug_assert!(self.nodes.is_empty());
        if !ids.is_empty() {
            self.world.record_hot_path_allocation(
                ids.len(),
                ids.len()
                    .saturating_mul(std::mem::size_of::<PassInput<'_>>()),
            );
        }
        let mut nodes = crate::store::DenseNodeMap::with_highest_of(ids);
        let mut containers = 0;
        for &id in ids {
            let input = self
                .world
                .layout_input_view(id, |style| Self::facts_in(&self.style_facts, style))?;
            containers += usize::from(!input.children.is_empty());
            nodes.insert(id, input);
        }
        self.materialized = nodes.len();
        self.nodes = nodes;
        self.container_plans.reserve(containers);
        Ok(())
    }

    pub(super) fn get(&mut self, id: StableNodeId) -> Result<Option<&PassInput<'a>>, UiWorldError> {
        let world = self.world;
        let materialized = &mut self.materialized;
        let facts = &self.style_facts;
        self.nodes.get_or_try_insert(id, || {
            match world.layout_input_view(id, |style| Self::facts_in(facts, style)) {
                Ok(input) => {
                    *materialized = materialized.saturating_add(1);
                    Ok(Some(input))
                }
                Err(UiWorldError::MissingNode(_)) => Ok(None),
                Err(error) => Err(error),
            }
        })
    }

    /// [`Self::get`] with the [`StyleFacts`] of the node's style: those its
    /// input carries, or, for a style the input owns (decided only when a
    /// measurement asks, since most of those nodes are only placed), from
    /// the memo.
    pub(super) fn get_with_facts(
        &mut self,
        id: StableNodeId,
    ) -> Result<Option<(&PassInput<'a>, StyleFacts)>, UiWorldError> {
        let world = self.world;
        let materialized = &mut self.materialized;
        let memo = &self.style_facts;
        let Some(node) = self.nodes.get_or_try_insert(id, || {
            match world.layout_input_view(id, |style| Self::facts_in(memo, style)) {
                Ok(input) => {
                    *materialized = materialized.saturating_add(1);
                    Ok(Some(input))
                }
                Err(UiWorldError::MissingNode(_)) => Ok(None),
                Err(error) => Err(error),
            }
        })?
        else {
            return Ok(None);
        };
        let facts = if node.facts.has(StyleFacts::KNOWN) {
            node.facts
        } else {
            Self::facts_in(memo, node.style.arc())
        };
        Ok(Some((node, facts)))
    }

    /// [`StyleFacts`] of `style`, from the pass's memo of the styles it met.
    pub(super) fn facts_in(
        memo: &RefCell<Vec<(Arc<nana_ui_core::LayoutStyle>, StyleFacts)>>,
        style: &Arc<nana_ui_core::LayoutStyle>,
    ) -> StyleFacts {
        /// Distinct styles kept: a page's boxes share a few.
        const KEPT: usize = 8;
        let mut memo = memo.borrow_mut();
        if let Some(index) = memo.iter().position(|(kept, _)| Arc::ptr_eq(kept, style)) {
            let facts = memo[index].1;
            #[cfg(any(test, feature = "layout-verify"))]
            assert_eq!(
                facts,
                StyleFacts::of(style),
                "the facts of a style went stale"
            );
            if index > 0 {
                memo.swap(index, index - 1);
            }
            return facts;
        }
        let facts = StyleFacts::of(style);
        if memo.len() == KEPT {
            memo.pop();
        }
        memo.insert(0, (Arc::clone(style), facts));
        facts
    }

    /// Style for classifying / measuring siblings without assembling `LayoutInput`.
    ///
    /// Borrowed from the node's input when the pass holds one (every node of
    /// a full pass): a pass asks about twenty times a box, and an `Arc` clone
    /// and drop each time cost more than the question.
    pub(super) fn style_ref(&self, id: StableNodeId) -> Option<StyleRef<'_>> {
        if let Some(node) = self.nodes.get(&id) {
            return Some(node.style.reborrow());
        }
        if let Some(cached) = self.styles.borrow().get(&id) {
            return cached.clone().map(StyleRef::Owned);
        }
        let resolved = self.world.layout_style(id);
        self.styles.borrow_mut().insert(id, resolved.clone());
        resolved.map(StyleRef::Owned)
    }

    /// [`Self::style_ref`] as an owned handle, for a caller that keeps it
    /// while it uses the map again.
    pub(super) fn style(&self, id: StableNodeId) -> Option<Arc<nana_ui_core::LayoutStyle>> {
        self.style_ref(id).map(|style| style.to_arc())
    }

    /// [`Self::style`] without taking a reference where the pass's input
    /// borrows the world's: an ordinary box of a full pass hands its style
    /// out as that borrow, anything else as an owned handle.
    pub(super) fn style_shared(&self, id: StableNodeId) -> Option<StyleRef<'a>> {
        if let Some(node) = self.nodes.get(&id) {
            return Some(node.style.shared());
        }
        self.style_ref(id)
            .map(|style| StyleRef::Owned(style.to_arc()))
    }

    /// Whether `id` holds an in-flow line box: [`UiWorld::holds_line_box_in`]
    /// on the retained memo.
    pub(super) fn holds_line_box(&self, id: StableNodeId) -> bool {
        self.checked_line_boxes(id, UiWorld::holds_line_box_in)
    }

    /// [`UiWorld::line_boxes_are_descendants_in`] on the retained memo.
    pub(super) fn line_boxes_are_descendants(&self, id: StableNodeId) -> bool {
        self.checked_line_boxes(id, UiWorld::line_boxes_are_descendants_in)
    }

    /// [`UiWorld::atom_sits_on_line_box`] on the retained memo.
    pub(super) fn atom_sits_on_line_box(&self, id: StableNodeId) -> bool {
        self.checked_line_boxes(id, UiWorld::atom_sits_on_line_box_in)
    }

    /// `ask` on the line-box memo. The memo outlives the pass, so a test
    /// build checks its answer against a fresh walk wherever that walk
    /// stays cheap.
    pub(super) fn checked_line_boxes(
        &self,
        id: StableNodeId,
        ask: fn(&UiWorld, StableNodeId, &mut crate::NodeMap<bool>) -> bool,
    ) -> bool {
        let answer = {
            let mut memo = self.line_boxes.borrow_mut();
            let known = memo.len();
            let answer = ask(self.world, id, &mut memo);
            self.line_boxes_settled
                .set(self.line_boxes_settled.get() + (memo.len() - known));
            answer
        };
        #[cfg(any(test, feature = "layout-verify"))]
        if self.world.len() <= 4096 {
            assert_eq!(
                answer,
                ask(self.world, id, &mut crate::NodeMap::default()),
                "{id:?}: the retained line-box answer is stale"
            );
        }
        answer
    }

    /// Whether `id` aligns by its border-box bottom rather than a text
    /// baseline: [`UiWorld::aligns_by_bottom_edge`].
    pub(super) fn aligns_by_bottom_edge(
        &self,
        id: StableNodeId,
        style: &nana_ui_core::LayoutStyle,
    ) -> bool {
        self.world.aligns_by_bottom_edge(id, style)
    }

    /// Where atomic inline `id` sits on a line box: its ascent and descent
    /// about the line's baseline, margins included. Native paragraphs and
    /// the lines of an ordinary block both read this, so the two cannot
    /// disagree (`vertical-align: baseline`).
    ///
    /// An `inline-flex` / `inline-grid` atom sits by the first baseline read
    /// from [`Self::baseline_metrics`]; non-container inline atoms fall back to
    /// their last line box. Replaced content and atoms with no in-flow line box
    /// (or empty flex/grid bottom synthesis) follow [`UiWorld::atom_sits_on_line_box`].
    pub(super) fn atom_line_metrics(
        &self,
        id: StableNodeId,
        writing: nana_ui_core::WritingContext,
        used: Size,
        margin: nana_ui_core::PaddingSpec,
        parent_font_px: f32,
        inline_base: f32,
        parent_strut: crate::inline_content::ParentLineStrut,
        viewport: crate::LayoutViewport,
    ) -> (
        nana_text::InlineObjectMetrics,
        crate::inline_content::AtomLinePlacement,
    ) {
        let block = writing.logical_size(used.width, used.height).1.max(0.0);
        let on_line_box = !writing.is_vertical() && self.atom_sits_on_line_box(id);
        // Every line reads one derived value: a caller that did not prime
        // this atom ([`inline::prime_atom_baselines`]) would place it by the
        // approximation instead.
        #[cfg(any(test, feature = "layout-verify"))]
        assert!(
            !on_line_box
                || !self.line_boxes_are_descendants(id)
                || self
                    .derived_baselines
                    .contains_key(&derived_baseline_key(id, used)),
            "{id:?} at {used:?}: its descendants' baseline was not derived"
        );
        let first_only = self.style(id).is_some_and(|s| {
            s.display
                .is_some_and(|d| d.is_flex_container() || d.is_grid_container())
        });
        let baseline = on_line_box
            .then(|| {
                let metrics =
                    self.baseline_metrics(id, parent_font_px, Some(inline_base), Some(used));
                if first_only {
                    metrics.first.or(metrics.last)
                } else {
                    metrics.last.or(metrics.first)
                }
            })
            .flatten()
            .map(|baseline| baseline.clamp(0.0, block));
        let default_style = nana_ui_core::LayoutStyle::default();
        let child_style = self.style_ref(id);
        crate::inline_content::resolve_atom_vertical_align(
            child_style.as_deref().unwrap_or(&default_style),
            writing,
            used,
            margin,
            baseline,
            parent_strut,
            parent_font_px,
            viewport,
        )
    }

    /// Shared baseline authority for all formatting contexts. Retained
    /// `nana-text` layouts provide first/last line baselines; host-shaped text
    /// falls back to its first-line ascent. Replaced/custom content has an
    /// explicit bottom-edge fallback so it never accidentally inherits text's
    /// approximate ascent.
    pub(super) fn baseline_metrics(
        &self,
        id: StableNodeId,
        fallback_font_px: f32,
        inline_base: Option<f32>,
        used: Option<Size>,
    ) -> BaselineMetrics {
        let Some(style) = self.style(id) else {
            return BaselineMetrics::default();
        };
        let font = fonts_of(&style, fallback_font_px).element_px;
        let chrome_top =
            style.resolved_padding_against(inline_base).top + style.resolved_border_width();
        let uses_card_title = self
            .world
            .widget_visual_ref(id)
            .is_some_and(crate::WidgetVisualHandle::uses_card_title_typography);
        if uses_card_title && self.world.text(id).is_none_or(str::is_empty) {
            return BaselineMetrics::default();
        }
        if self.aligns_by_bottom_edge(id, &style) {
            // Replaced content's baseline is its border-box bottom. The
            // measure caller supplies the used extent when it is available;
            // an absent extent remains an explicit fallback for a later query.
            let block = used.and_then(|used| {
                let (_, block) = self
                    .nodes
                    .get(&id)
                    .map(|node| node.writing.logical_size(used.width, used.height))?;
                Some(block.max(0.0))
            });
            return BaselineMetrics {
                first: block,
                last: block,
            };
        }
        if uses_card_title && let Some(used) = used {
            let padding = style
                .resolved_padding_against_fonts(inline_base, fonts_of(&style, fallback_font_px));
            let border = style.resolved_border_edges();
            let bounds = LayoutBox {
                x: 0.0,
                y: 0.0,
                width: used.width,
                height: used.height,
            };
            let content = crate::layout_result::inset(
                crate::layout_result::inset(
                    bounds,
                    border.top,
                    border.right,
                    border.bottom,
                    border.left,
                ),
                padding.top,
                padding.right,
                padding.bottom,
                padding.left,
            );
            let parts = self.world.solve_content_parts_at(id, content, bounds);
            if let Some(header) = parts::part_bounds(&parts, crate::LayoutPartKind::Header) {
                if let Some((_, layout)) = self.world.text_layout(id) {
                    if layout.is_vertical() {
                        return BaselineMetrics::default();
                    }
                    let top = self.world.native_text_origin(id, header, layout)[1];
                    return BaselineMetrics {
                        first: layout
                            .lines
                            .first()
                            .map(|line| top + line.metrics.baseline_y_px),
                        last: layout
                            .lines
                            .last()
                            .map(|line| top + line.metrics.baseline_y_px),
                    };
                }
                let baseline = self.world.single_text_baseline(id, header);
                return BaselineMetrics {
                    first: baseline,
                    last: baseline,
                };
            }
        }
        if let Some((_, layout)) = self.world.text_layout(id)
            && !layout.is_vertical()
            && !layout.lines.is_empty()
        {
            let content_height = used.map_or(layout.physical_size().1, |used| {
                let padding = style.resolved_padding_against_fonts(
                    inline_base,
                    fonts_of(&style, fallback_font_px),
                );
                let border = style.resolved_border_edges();
                (used.height - padding.top - padding.bottom - border.top - border.bottom).max(0.0)
            });
            let alignment = self
                .world
                .node_style(id)
                .map_or(crate::TextVerticalAlignment::Top, |style| {
                    style.text_vertical_alignment
                });
            let top = crate::text_layout_origin(
                LayoutBox {
                    x: 0.0,
                    y: chrome_top,
                    width: 0.0,
                    height: content_height,
                },
                alignment,
                layout.physical_size().1,
            )[1];
            let first = layout
                .lines
                .first()
                .map(|line| top + line.metrics.baseline_y_px);
            let last = layout
                .lines
                .last()
                .map(|line| top + line.metrics.baseline_y_px);
            return BaselineMetrics { first, last };
        }
        let button_label = self
            .world
            .widget_visual_ref(id)
            .and_then(crate::WidgetVisualHandle::button_label_state);
        if button_label == Some(false) {
            let block = used.map(|used| {
                let writing = self
                    .nodes
                    .get(&id)
                    .map_or_else(Default::default, |node| node.writing);
                writing.logical_size(used.width, used.height).1.max(0.0)
            });
            return BaselineMetrics {
                first: block,
                last: block,
            };
        }
        // A box with no line box in it, of its own or a descendant's, has
        // no baseline: one is synthesized from its border-box bottom (CSS
        // Align 3 §9.1), as a flex container takes from such an item (#394).
        if !self.holds_line_box(id)
            && !self.world.has_in_flow_flex_or_grid_item(id, &style)
            && let Some(used) = used
        {
            let writing = self
                .nodes
                .get(&id)
                .map_or_else(Default::default, |node| node.writing);
            let block = Some(writing.logical_size(used.width, used.height).1.max(0.0));
            return BaselineMetrics {
                first: block,
                last: block,
            };
        }
        let button_with_label = button_label == Some(true);
        let ascent = self
            .nodes
            .get(&id)
            .and_then(|node| node.text_metrics)
            .and_then(|metrics| metrics.ascent)
            .filter(|value| value.is_finite() && *value >= 0.0);
        let ascent = ascent.unwrap_or(font * nana_ui_core::TEXT_APPROX_ASCENT_EM);
        let approximate = Some(chrome_top + ascent);
        if button_with_label {
            // A labelled button's baseline belongs to its internal label
            // content. The shaped text metrics above are authoritative; the
            // approximation is only the same explicit host fallback used
            // when shaping has not produced a line yet.
            return BaselineMetrics {
                first: approximate,
                last: approximate,
            };
        }
        // A box whose line boxes are its descendants' starts on the first of
        // them and ends on the last, where they sit inside the box. Where
        // nothing derived them, its own font's first line is the
        // approximation of both.
        let derived = used
            .and_then(|used| {
                self.derived_baselines
                    .get(&derived_baseline_key(id, used))
                    .copied()
            })
            .unwrap_or_default();
        let first = derived.first.or(approximate);
        BaselineMetrics {
            first,
            last: derived.last.or(first),
        }
    }

    pub(super) fn baseline(&self, id: StableNodeId, fallback_font_px: f32, used: Size) -> f32 {
        // A flex line reads one derived value: a caller that did not prime
        // this item ([`inline::prime_first_baselines`]) would align it by
        // the approximation instead.
        #[cfg(any(test, feature = "layout-verify"))]
        assert!(
            !self.world.aligns_by_first_baseline(id)
                || !self.line_boxes_are_descendants(id)
                || self
                    .derived_baselines
                    .contains_key(&derived_baseline_key(id, used)),
            "{id:?} at {used:?}: its descendants' first baseline was not derived"
        );
        let writing = self
            .nodes
            .get(&id)
            .map_or_else(Default::default, |node| node.writing);
        let (_, block_extent) = writing.logical_size(used.width, used.height);
        if self
            .style(id)
            .is_some_and(|style| self.aligns_by_bottom_edge(id, &style))
        {
            // Replaced/custom nodes align to their bottom edge by default.
            return block_extent.max(0.0);
        }
        let metrics = self.baseline_metrics(
            id,
            fallback_font_px,
            Some(writing.inline_size(used.width, used.height)),
            Some(used),
        );
        // Read both ends here so callers share one retained result even when
        // the current parent only asks for first baseline alignment.
        let _last = metrics.last;
        metrics.first.unwrap_or(block_extent.max(0.0))
    }
}

/// Collapse every kept box at or under an affected node that omits its box
/// and was not placed this pass to the empty box a cold layout leaves it, and
/// forget the contexts those nodes established, which a cold layout never
/// records, with the plans placed under them: shown again, they are placed
/// afresh and record their contexts again (#392).
pub(super) fn collapse_omitted_boxes(
    affected: &crate::NodeSet,
    nodes: &mut LayoutInputMap<'_>,
    retained: &mut DocumentLayoutCache,
    output: &mut PassOutput,
) {
    for &id in affected {
        // A container may write the omitted box itself, as a zero box, and
        // stop there: its descendants still keep the boxes they had.
        if !nodes.style_ref(id).is_some_and(|style| style.omits_box()) {
            continue;
        }
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            retained.contexts.remove(&node);
            retained.used_padding.remove(&node);
            retained.omitted_boxes.insert(node);
            retained.container_plans.remove(&node);
            retained.remote_overlays.remove(&node);
            // No box, and so no frame other than a cold layout's.
            if retained.frames.contains_key(&node) {
                nodes.frames.insert(node, crate::LayoutFrame::Parent);
                output.or_default(node);
            }
            if retained
                .boxes
                .get(&node)
                .is_some_and(|kept| *kept != LayoutBox::default())
            {
                output.or_default(node);
            }
            if let Some(record) = nodes.world.node(node) {
                stack.extend(record.children.iter().copied());
            }
        }
    }
}

pub(super) struct ScopeContext<'a> {
    /// What reached each affected node: a seed of its own, or which kind of
    /// dependency edge.
    pub(super) frontier: &'a LayoutFrontier,
    pub(super) affected: &'a crate::NodeSet,
    /// `affected` by parent: what a container's plan re-checks or patches.
    pub(super) affected_children: &'a AffectedChildren,
    pub(super) measure: &'a crate::NodeSet,
    pub(super) metrics_changed: &'a crate::NodeSet,
    pub(super) retained: &'a DocumentLayoutCache,
    /// Which boxes lead to an affected node, built once per pass.
    pub(super) reach: &'a AffectedIndex,
}

/// The affected nodes by parent, each list in id order. A container's plan
/// re-checks and patches its own children only: asking every affected node
/// of the pass whether it is one of them made each container that a pass
/// reuses cost the whole closure, so a pass that reuses the plans down a
/// deep chain cost the chain times the closure. In id order, the children a
/// re-check measures before it stops, and the order a patch sums them in,
/// do not follow the hash order of `affected`.
#[derive(Default)]
pub(super) struct AffectedChildren(crate::NodeMap<Vec<StableNodeId>>);

impl AffectedChildren {
    pub(super) fn new(world: &UiWorld, affected: &crate::NodeSet) -> Self {
        let mut by_parent = crate::NodeMap::<Vec<StableNodeId>>::default();
        for &id in affected {
            if let Some(parent) = world.parent_id(id) {
                by_parent.entry(parent).or_default().push(id);
            }
        }
        for children in by_parent.values_mut() {
            children.sort_unstable();
        }
        Self(by_parent)
    }
}

impl ScopeContext<'_> {
    /// The affected direct children of `container`, in id order.
    pub(super) fn affected_children(&self, container: StableNodeId) -> &[StableNodeId] {
        let children = self
            .affected_children
            .0
            .get(&container)
            .map_or(&[][..], Vec::as_slice);
        #[cfg(test)]
        for _ in children {
            crate::world::page_origins::steps::step(
                crate::world::page_origins::steps::Walk::PlanChildren,
            );
        }
        children
    }
}

/// Every node an affected node is in, and per container the direct children
/// that are affected or contain an affected node. A fixed box may stay out of
/// the frontier while a node inside it changes; every place that prunes a
/// subtree or replays part of a container asks this, so the path down to that
/// node is never cut.
#[derive(Default)]
pub(crate) struct AffectedIndex {
    pub(super) reach: crate::NodeSet,
    pub(super) reaching: HashMap<StableNodeId, Vec<StableNodeId>>,
}

impl AffectedIndex {
    /// From `affected`, leaving out the nodes inside `covered` (island roots,
    /// laid out on their own).
    pub(crate) fn new<'a>(
        world: &UiWorld,
        affected: impl IntoIterator<Item = &'a StableNodeId>,
        covered: &HashSet<StableNodeId>,
    ) -> Self {
        let mut index = Self::default();
        // Nodes found inside an island. A node in `reach` is outside every
        // island, and so is all above it: a walk up stops at either, so the
        // nodes of a deep chain are not each walked to the root.
        let mut inside = crate::NodeSet::default();
        let mut passed = Vec::new();
        for &id in affected {
            let inside_island = !covered.is_empty() && {
                passed.clear();
                let mut cursor = Some(id);
                let found = loop {
                    let Some(node) = cursor else {
                        break false;
                    };
                    #[cfg(test)]
                    crate::world::page_origins::steps::step(
                        crate::world::page_origins::steps::Walk::Islands,
                    );
                    if covered.contains(&node) || inside.contains(&node) {
                        break true;
                    }
                    if index.reach.contains(&node) {
                        break false;
                    }
                    passed.push(node);
                    cursor = world.parent_id(node);
                };
                if found {
                    inside.extend(passed.iter().copied());
                }
                #[cfg(any(test, feature = "layout-verify"))]
                assert_eq!(
                    found,
                    std::iter::successors(Some(id), |id| world.parent_id(*id))
                        .any(|node| covered.contains(&node)),
                    "{id:?}: whether it is inside an island"
                );
                found
            };
            if inside_island || !index.reach.insert(id) {
                continue;
            }
            let mut child = id;
            while let Some(parent) = world.parent_id(child) {
                index.reaching.entry(parent).or_default().push(child);
                if !index.reach.insert(parent) {
                    break;
                }
                child = parent;
            }
        }
        index
    }

    /// Whether `id` is affected or contains an affected node.
    pub(crate) fn reaches(&self, id: StableNodeId) -> bool {
        self.reach.contains(&id)
    }

    /// The direct children of `container` that lead to an affected node.
    pub(crate) fn children(&self, container: StableNodeId) -> &[StableNodeId] {
        self.reaching.get(&container).map_or(&[], Vec::as_slice)
    }
}

/// Prune a child recursion when the child is outside the affected closure and
/// its recomputed entry box is bit-identical to the retained one.
pub(super) fn subtree_unchanged(
    child: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    child_style: &nana_ui_core::LayoutStyle,
    child_fonts: FontSizeContext,
    // The parent's: it is the child's containing block.
    writing: nana_ui_core::WritingContext,
    scope: Option<&ScopeContext<'_>>,
) -> bool {
    let Some(scope) = scope else {
        return false;
    };
    if scope.affected.contains(&child) || scope.reach.reaches(child) {
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
    scope.retained.used_padding_of(child)
        == Some(child_style.resolved_padding_against_fonts(
            Some(writing.inline_size(containing.width, containing.height)),
            child_fonts,
        ))
        && cached.x == origin.x + relative_x
        && cached.y == origin.y + relative_y
        && cached.width == size.width
        && cached.height == size.height
}

pub(super) fn sort_by_order(ids: &mut [StableNodeId], nodes: &LayoutInputMap<'_>) {
    let order_of = |id: StableNodeId| nodes.style_ref(id).map(|style| style.order).unwrap_or(0);
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

pub(super) fn uses_2d_grid(
    style: &LayoutStyle,
    flow: &[StableNodeId],
    nodes: &LayoutInputMap<'_>,
) -> bool {
    if !style.display.is_some_and(DisplaySpec::is_grid_container) {
        return false;
    }
    if style.display.is_some_and(DisplaySpec::is_layers) {
        return true;
    }
    if !style.grid.is_shared_default() {
        let grid = &*style.grid;
        if grid.columns.is_some()
            || grid.rows.is_some()
            || grid.columns_subgrid
            || grid.rows_subgrid
            || grid.columns_unsupported.is_some()
            || grid.rows_unsupported.is_some()
            || grid.columns_repeat.is_some()
            || grid.rows_repeat.is_some()
            || grid.auto_flow.is_some()
            || grid
                .auto_columns
                .as_ref()
                .is_some_and(|tracks| !tracks.is_empty())
            || grid
                .auto_rows
                .as_ref()
                .is_some_and(|tracks| !tracks.is_empty())
            || grid
                .template_areas
                .as_ref()
                .is_some_and(|areas| !areas.cells.is_empty())
        {
            return true;
        }
    }
    flow.iter().any(|id| {
        nodes
            .style(*id)
            .is_some_and(|child| !child.grid.is_shared_default() && !child.grid.placement.is_auto())
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Point {
    pub(super) x: f32,
    pub(super) y: f32,
}

impl Point {
    pub(super) const ZERO: Self = Self { x: 0.0, y: 0.0 };
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Size {
    pub(crate) width: f32,
    pub(crate) height: f32,
}

/// All non-content inputs that can change a used measurement.  The containing
/// block size remains in the key, while font, viewport, writing mode, and
/// parent flow are carried alongside it so a compatible constraint cannot
/// accidentally reuse a result from another layout environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct StoredMeasurementKey {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) viewport_width: u32,
    pub(super) viewport_height: u32,
    pub(super) parent_font: u32,
    pub(super) constraint_a: u32,
    pub(super) constraint_b: u32,
    /// Bits `0..2`: `parent_direction`; bit `2`: `direction_sensitive`;
    /// bits `3..6`: `writing`; bits `6..9`: `containing_writing`;
    /// bits `9..13`: `constraint_tag`; bit `15`: `1` (NonZeroU16 niche for `Option<RetainedIntrinsic>`).
    pub(super) flags: std::num::NonZeroU16,
}

const _: () = assert!(std::mem::size_of::<Option<RetainedIntrinsic>>() <= 48);

#[inline]
pub(super) const fn writing_context_bits(writing: nana_ui_core::WritingContext) -> u16 {
    let mode = match writing.mode {
        nana_ui_core::WritingModeSpec::HorizontalTb => 0,
        nana_ui_core::WritingModeSpec::VerticalRl => 1,
        nana_ui_core::WritingModeSpec::VerticalLr => 2,
    };
    let rtl = if writing.direction.is_rtl() { 1 } else { 0 };
    mode | (rtl << 2)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct MeasurementKey {
    pub(super) id: StableNodeId,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) parent_direction: u8,
    pub(super) viewport_width: u32,
    pub(super) viewport_height: u32,
    pub(super) parent_font: u32,
    pub(super) writing: nana_ui_core::WritingContext,
    pub(super) containing_writing: nana_ui_core::WritingContext,
    pub(super) constraint: crate::ConstraintClass,
    pub(super) direction_sensitive: bool,
}

impl MeasurementKey {
    pub(super) fn stored(self) -> StoredMeasurementKey {
        let (constraint_tag, (constraint_a, constraint_b)) = self.constraint.canonical();
        let bits = (u16::from(self.parent_direction) & 0b11)
            | (u16::from(self.direction_sensitive) << 2)
            | (writing_context_bits(self.writing) << 3)
            | (writing_context_bits(self.containing_writing) << 6)
            | (u16::from(constraint_tag & 0b1111) << 9)
            | (1u16 << 15);
        StoredMeasurementKey {
            width: self.width,
            height: self.height,
            viewport_width: self.viewport_width,
            viewport_height: self.viewport_height,
            parent_font: self.parent_font,
            constraint_a,
            constraint_b,
            flags: std::num::NonZeroU16::new(bits).expect("sentinel bit keeps flags nonzero"),
        }
    }

    pub(super) fn new(
        id: StableNodeId,
        available: Size,
        parent_direction: Option<FlexDirection>,
        viewport: LayoutViewport,
        parent_font_px: f32,
        writing: nana_ui_core::WritingContext,
        containing_writing: nana_ui_core::WritingContext,
        constraint: crate::ConstraintClass,
        direction_sensitive: bool,
    ) -> Self {
        Self {
            id,
            width: available.width.to_bits(),
            height: available.height.to_bits(),
            parent_direction: match parent_direction {
                None => 0,
                Some(FlexDirection::Column) => 1,
                Some(FlexDirection::Row) => 2,
            },
            viewport_width: viewport.width.to_bits(),
            viewport_height: viewport.height.to_bits(),
            parent_font: parent_font_px.to_bits(),
            writing,
            containing_writing,
            constraint: constraint.normalized(),
            direction_sensitive,
        }
    }
}

/// Work a layout pass ran. Frontier membership is recorded separately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LayoutExecutionStats {
    pub measure_nodes: usize,
    pub measure_cache_hits: usize,
    pub measure_cache_misses: usize,
    pub placement_nodes: usize,
    pub origin_only_updates: usize,
    /// Containers that only moved and kept their whole subtree: their own
    /// box was written, their body did not run (`keeps_moved_subtree`).
    pub moved_subtrees_kept: usize,
    /// Containers placed from their retained [`ContainerPlan`].
    pub placement_plans_reused: usize,
    /// Containers measured from a retained [`MeasurePlan`], without a walk of
    /// their children.
    pub measure_plans_reused: usize,
    /// Sequential containers that kept their prefix and replayed only the
    /// children from the first changed one on.
    pub suffixes_replayed: usize,
    /// Children a container walk measured, in its measure and in its
    /// placement: the scan that makes a dirty frame O(N) when a plan misses.
    pub children_measured: usize,
    pub inline_reflows: usize,
    pub inline_atomic_measure_requests: usize,
    /// Containers that placed their children but could not record a plan,
    /// so they walk them again on every later pass.
    pub containers_uncacheable: usize,
    /// Containers that had a retained plan for this pass's inputs and still
    /// walked their children: the plan could not answer. A query is a hit
    /// (one of the two `*_plans_reused`) or one of these.
    pub plan_misses: usize,
    /// Plans a walk recorded over an earlier plan for the same container
    /// and constraint. A container's first plan is not a rebuild.
    pub plan_rebuilds: usize,
    /// Positioned contexts laid out whole because their plan was stale.
    pub local_context_fallbacks: usize,
    /// Retained-cache sweeps that dropped despawned ids.
    pub retain_sweeps: usize,
    /// What Dynamic Layout did this pass.
    pub dynamic: nana_ui_core::DynamicLayoutCounters,
    /// Most stack one level of the measure recursion took, in bytes; 0 where
    /// the platform cannot tell. See `recursion`.
    pub measure_stack_per_level: usize,
    /// The same for the placement recursion.
    pub placement_stack_per_level: usize,
    /// Subtrees placed from the boundary queue. See `recursion`.
    pub placement_boundaries: usize,
    /// Of those, the ones cut above the span because their root is a scroll
    /// container or isolates its layout (`placement::natural_boundary`).
    pub placement_natural_boundaries: usize,
    /// Boundaries out of view that a sliced pass left for later passes,
    /// including those earlier passes left that it did not reach: what is
    /// pending once it commits.
    pub placement_boundaries_deferred: usize,
    /// 1 when this pass stopped at its budget and deferred a boundary.
    pub placement_slices: usize,
    /// Deferred boundaries dropped before they were finished: the root left
    /// the document or stopped generating a box, or a pass placed its box
    /// with other inputs and the work was redone from them.
    pub placement_deferrals_cancelled: usize,
    /// Boxes a line-box question settled by walking rather than from the
    /// retained memo ([`UiWorld::holds_line_box_in`]): what a seed that drops
    /// the memo up its ancestors costs the next pass.
    pub line_boxes_settled: usize,
}

/// Per-pass used-size memo.  The public [`crate::IntrinsicCache`] owns the
/// generation-aware intrinsic contract; this tiny adapter keeps the existing
/// layout algorithm's physical-size representation while exposing the same
/// hit/miss accounting.  Its key deliberately has no formatting-context id,
/// so a child measured by flex/grid/inline can share the result in a pass.
#[derive(Default)]
pub(super) struct PassIntrinsicCache {
    /// Intrinsic facts are kept separately from used sizes.  A percentage,
    /// fill, or stretch result is a resolution against one containing block;
    /// it must never become the preferred value in the shared authority.
    pub(super) cache: crate::intrinsic_cache::PassIntrinsicFacts,
    /// Every used size this pass measured, in the order it measured them:
    /// the pass's used-size memo, and the writeback merged into the retained
    /// cache at the end. A node keeps two retained variants; writing them back
    /// in measuring order keeps the two it measured last. A key's key contains
    /// the concrete available size because that is exactly what the
    /// placement algorithm resolves.
    pub(super) used_order: Vec<(MeasurementKey, Size)>,
    /// The memo's index: each node's newest entry of `used_order`, and from
    /// each entry the node's next older one (`u32::MAX` for none). A lookup
    /// compares a node's few keys instead of hashing the whole key.
    pub(super) used_newest: crate::store::DenseNodeMap<u32>,
    pub(super) used_older: Vec<u32>,
    /// Containers a measure plan answered by patching what it recorded: the
    /// intrinsic facts retained for them were measured from content the
    /// patch changed, and no longer describe it under any constraint. They
    /// leave the retained cache with this pass.
    pub(super) retired_facts: Vec<StableNodeId>,
    /// Containers whose retained measure plan answered unchanged this pass.
    pub(super) unchanged_plan_nodes: crate::NodeSet,
    /// Retained measure plans this pass answered from, by container and the
    /// constraint each holds: they saw what changed under them this pass.
    pub(super) consulted_plans: Vec<(StableNodeId, Size)>,
    pub(super) execution_stats: LayoutExecutionStats,
    /// Whether containers record the measure plans later scoped passes
    /// answer from: every pass but a full one a caller asked for.
    pub(super) records_measure_plans: bool,
    /// Dynamic Layout state for the pass.
    pub(super) dynamic: dynamic::DynamicPass,
    /// The open levels of the placement recursion.
    pub(super) placement_levels: recursion::LevelStack,
    /// Subtrees placement queued at its span, placed before the outermost
    /// placement call returns. See `recursion`.
    pub(super) boundaries: recursion::BoundaryQueue,
    /// The pass's budget and deferrals. See `recursion::LayoutSliceBudget`.
    pub(super) slice: recursion::PassSlice,
}

impl PassIntrinsicCache {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            // A node may be measured at its parent width and again at a
            // flex/grid used width; the pass keeps every variant for
            // baseline and cross-context consumers.
            cache: crate::intrinsic_cache::PassIntrinsicFacts::with_capacity(capacity),
            used_order: Vec::with_capacity(capacity),
            used_newest: crate::store::DenseNodeMap::default(),
            used_older: Vec::with_capacity(capacity),
            retired_facts: Vec::new(),
            unchanged_plan_nodes: crate::NodeSet::default(),
            consulted_plans: Vec::new(),
            execution_stats: LayoutExecutionStats::default(),
            records_measure_plans: false,
            dynamic: dynamic::DynamicPass::default(),
            placement_levels: recursion::LevelStack::default(),
            boundaries: recursion::BoundaryQueue::default(),
            slice: recursion::PassSlice::default(),
        }
    }

    /// Room for every id up to `highest` in the per-node tables a full pass
    /// fills.
    pub(super) fn reserve_through(&mut self, highest: StableNodeId, entries: usize) {
        self.used_newest.reserve_through(highest, entries);
        self.cache.reserve_through(highest, entries);
    }

    pub(super) fn levels(&mut self, recursion: recursion::Recursion) -> &mut recursion::LevelStack {
        match recursion {
            recursion::Recursion::Placement => &mut self.placement_levels,
        }
    }

    pub(super) fn note_measure_cache_hit(&mut self) {
        self.execution_stats.measure_cache_hits =
            self.execution_stats.measure_cache_hits.saturating_add(1);
    }

    pub(super) fn note_measure_cache_miss(&mut self) {
        self.execution_stats.measure_cache_misses =
            self.execution_stats.measure_cache_misses.saturating_add(1);
    }

    pub(super) fn note_measure_node(&mut self) {
        self.execution_stats.measure_nodes = self.execution_stats.measure_nodes.saturating_add(1);
    }

    pub(super) fn note_placement_plan_reused(&mut self) {
        self.execution_stats.placement_plans_reused = self
            .execution_stats
            .placement_plans_reused
            .saturating_add(1);
    }

    pub(super) fn note_measure_plan_reused(&mut self) {
        self.execution_stats.measure_plans_reused =
            self.execution_stats.measure_plans_reused.saturating_add(1);
    }

    /// `id`'s measure plan answered by a patch: see `retired_facts`.
    pub(super) fn retire_intrinsic_facts(&mut self, id: StableNodeId) {
        self.retired_facts.push(id);
    }

    pub(super) fn note_unchanged_plan_node(&mut self, id: StableNodeId) {
        self.unchanged_plan_nodes.insert(id);
    }

    /// `id`'s retained measure plan under `available` answered this pass:
    /// see `consulted_plans`.
    pub(super) fn note_plan_consulted(&mut self, id: StableNodeId, available: Size) {
        self.consulted_plans.push((id, available));
    }

    pub(super) fn note_suffix_replayed(&mut self) {
        self.execution_stats.suffixes_replayed =
            self.execution_stats.suffixes_replayed.saturating_add(1);
    }

    pub(super) fn note_child_measured(&mut self) {
        self.execution_stats.children_measured =
            self.execution_stats.children_measured.saturating_add(1);
    }

    pub(super) fn note_container_uncacheable(&mut self) {
        self.execution_stats.containers_uncacheable = self
            .execution_stats
            .containers_uncacheable
            .saturating_add(1);
    }

    pub(super) fn note_plan_miss(&mut self) {
        self.execution_stats.plan_misses = self.execution_stats.plan_misses.saturating_add(1);
    }

    pub(super) fn note_plan_rebuilt(&mut self) {
        self.execution_stats.plan_rebuilds = self.execution_stats.plan_rebuilds.saturating_add(1);
    }

    pub(super) fn note_local_context_fallback(&mut self) {
        self.execution_stats.local_context_fallbacks = self
            .execution_stats
            .local_context_fallbacks
            .saturating_add(1);
    }

    pub(super) fn note_placement_node(&mut self) {
        self.execution_stats.placement_nodes =
            self.execution_stats.placement_nodes.saturating_add(1);
    }

    pub(super) fn note_moved_subtree_kept(&mut self) {
        self.execution_stats.moved_subtrees_kept =
            self.execution_stats.moved_subtrees_kept.saturating_add(1);
    }

    pub(super) fn note_origin_only(&mut self) {
        self.execution_stats.origin_only_updates =
            self.execution_stats.origin_only_updates.saturating_add(1);
    }

    pub(super) fn get(&mut self, key: &MeasurementKey) -> Option<Size> {
        // A hit answers the query. Intrinsic accounting stays on the miss
        // path; this memo must not increment those counters.
        let mut index = self.used_newest.get(&key.id).copied().unwrap_or(u32::MAX);
        while index != u32::MAX {
            let (held, size) = &self.used_order[index as usize];
            if held == key {
                return Some(*size);
            }
            index = self.used_older[index as usize];
        }
        None
    }

    pub(super) fn insert(&mut self, key: MeasurementKey, size: Size) {
        // The newest entry for a key answers it, as a map's last write would.
        let index = u32::try_from(self.used_order.len()).expect("a pass measures under 4G sizes");
        self.used_order.push((key, size));
        self.used_older
            .push(self.used_newest.insert(key.id, index).unwrap_or(u32::MAX));
    }

    /// Publish content-derived facts. `preferred` is the natural border-box
    /// result before resolving the current containing block. Callers pass the
    /// final used value separately to [`Self::insert`].
    pub(super) fn insert_intrinsic_bounds(
        &mut self,
        key: MeasurementKey,
        min_inline: f32,
        max_inline: f32,
        min_block: f32,
        max_block: Option<f32>,
        preferred: Size,
        first_baseline: Option<f32>,
        last_baseline: Option<f32>,
        aspect_ratio: Option<f32>,
    ) {
        let cache_key = Self::intrinsic_key(key);
        let metrics = crate::IntrinsicMetrics::new(
            min_inline,
            max_inline,
            min_block,
            max_block,
            crate::UsedSize::new(preferred.width, preferred.height),
        )
        .with_baselines(first_baseline, last_baseline)
        .with_aspect_ratio(aspect_ratio);
        let context = match key.parent_direction {
            1 => Some(crate::FormattingContextId::new(1)),
            2 => Some(crate::FormattingContextId::new(2)),
            _ => None,
        };
        self.cache
            .insert_measured(key.id, cache_key, metrics, context);
    }

    /// Record facts for the retained cache without making them visible to
    /// this pass's lookups.
    ///
    /// Placement reads a baseline from those lookups only for `align-items:
    /// baseline`; every other alignment recomputes from the node. A plain
    /// leaf still hands the retained cache the same metrics the general
    /// insert would have recorded.
    pub(super) fn remember_intrinsic_bounds(
        &mut self,
        key: MeasurementKey,
        min_inline: f32,
        max_inline: f32,
        min_block: f32,
        max_block: Option<f32>,
        preferred: Size,
        first_baseline: Option<f32>,
        last_baseline: Option<f32>,
        aspect_ratio: Option<f32>,
    ) {
        let cache_key = Self::intrinsic_key(key);
        let metrics = crate::IntrinsicMetrics::new(
            min_inline,
            max_inline,
            min_block,
            max_block,
            crate::UsedSize::new(preferred.width, preferred.height),
        )
        .with_baselines(first_baseline, last_baseline)
        .with_aspect_ratio(aspect_ratio);
        self.cache.record(cache_key, metrics);
    }

    pub(super) fn seed_intrinsic(
        &mut self,
        key: crate::IntrinsicCacheKey,
        metrics: crate::IntrinsicMetrics,
        context: Option<crate::FormattingContextId>,
    ) {
        if !self.cache.contains(&key) {
            self.cache.insert(key, metrics, context);
        }
        if let Some(id) = StableNodeId::new(key.content) {
            self.cache.note_latest(id, &key);
        }
    }

    pub(super) fn get_intrinsic(
        &mut self,
        key: MeasurementKey,
        context: Option<crate::FormattingContextId>,
    ) -> Option<crate::IntrinsicMetrics> {
        let key = Self::intrinsic_key(key);
        self.cache.get(&key, context)
    }

    pub(super) fn record_full_subtree(&mut self) {
        self.cache.record_full_subtree();
    }

    pub(super) fn baseline(&mut self, id: StableNodeId, which: crate::Baseline) -> Option<f32> {
        let Some(key) = self.cache.latest_key(id) else {
            self.cache.record_baseline_miss();
            return None;
        };
        self.cache.baseline(&key, None, which)
    }

    pub(super) fn counters(&self) -> crate::IntrinsicCacheCounters {
        self.cache.counters()
    }

    pub(super) fn intrinsic_key(key: MeasurementKey) -> crate::IntrinsicCacheKey {
        // Natural contributions for wrapping and percentage descendants can
        // depend on the containing block. Keep that relevant basis in the
        // constraint class and carry the remaining environment in the style
        // identity; the formatting-context name itself is still absent.
        use std::hash::{BuildHasher, Hash, Hasher};
        // A fixed seed: the identity is compared across passes.
        let mut hasher = foldhash::fast::FixedState::with_seed(0x6e61_6e61).build_hasher();
        // Parent flow direction selects the *used-size* resolution path, but
        // it is not part of ordinary intrinsic facts. Aspect-ratio stretch
        // transfer is the deliberate exception: its used width depends on
        // whether the parent is a row, so keep that relevant dependency in
        // the identity while all other content still crosses contexts.
        if key.direction_sensitive
            || matches!(key.constraint, crate::ConstraintClass::AspectRatio { .. })
        {
            key.parent_direction.hash(&mut hasher);
        }
        key.viewport_width.hash(&mut hasher);
        key.viewport_height.hash(&mut hasher);
        key.parent_font.hash(&mut hasher);
        key.writing.hash(&mut hasher);
        key.containing_writing.hash(&mut hasher);
        crate::IntrinsicCacheKey::new(key.id.get(), hasher.finish(), key.constraint)
    }
}
