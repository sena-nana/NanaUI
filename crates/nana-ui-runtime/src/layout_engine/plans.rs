//! Retained container and measure plans, patch application, and input equality.

use nana_ui_core::LayoutStyleChange;

use super::*;

#[allow(dead_code)]
pub mod plan_stats {
    use std::cell::Cell;

    /// Coarse clocks for `--profile-layout`. Slots:
    /// prefetch, root measure, root place, engine tail,
    /// large-container child measure, large-container measure fold,
    /// large-container place remeasure, large-container place pack,
    /// result build, result store,
    /// writeback compare, view scan, commit, publish,
    /// plain-leaf content, plain-leaf baseline, plain-leaf metric record.
    pub(super) const PHASES: usize = 17;

    thread_local! {
        static PHASE_NS: Cell<[u64; 17]> = const { Cell::new([0; 17]) };
    }

    pub(crate) fn add_phase(slot: usize, elapsed: std::time::Duration) {
        PHASE_NS.with(|cell| {
            let mut slots = cell.get();
            slots[slot] = slots[slot].saturating_add(elapsed.as_nanos() as u64);
            cell.set(slots);
        });
    }

    pub fn take_phases_ns() -> [u64; PHASES] {
        PHASE_NS.with(|cell| cell.replace([0; PHASES]))
    }

    pub(crate) struct PhaseClock {
        start: std::time::Instant,
    }

    impl PhaseClock {
        pub(crate) fn start() -> Self {
            Self {
                start: std::time::Instant::now(),
            }
        }

        pub(crate) fn lap(&mut self, slot: usize) {
            let now = std::time::Instant::now();
            add_phase(slot, now.saturating_duration_since(self.start));
            self.start = now;
        }
    }
}

/// One positioned child of a cached container. The list is the direct
/// participants of that positioned formatting context, replaced when the
/// container is recorded again.
#[derive(Clone)]
pub(super) struct PlannedOverlay {
    pub(super) child: StableNodeId,
    /// Retained style and used box for this participant. A later pass replaces
    /// the record; the fields are the cached placement, not a second authority.
    #[allow(dead_code)]
    pub(super) style: Arc<nana_ui_core::LayoutStyle>,
    pub(super) fixed: bool,
    /// The absolute child's containing block (CSS 2.1 §10.1): its nearest
    /// positioned ancestor, whose padding box `base_origin` and `base` are,
    /// or `None` for the initial containing block -- the viewport at the
    /// document origin. `None` as well for a fixed child.
    pub(super) block: Option<StableNodeId>,
    /// `block` is the container whose plan holds this entry: the child moves
    /// with it. Otherwise only its containing block moves it.
    pub(super) in_container: bool,
    /// The container's content origin at placement, in the child's frame,
    /// when an axis has both insets auto: the box keeps its static position
    /// there, so a box moving between the containing block and the
    /// container moves it.
    pub(super) static_origin: Option<Point>,
    /// The used box reads the containing block's size (percentage, fill,
    /// or both insets). A fixed child reads the viewport instead.
    pub(super) tracks_containing_block: bool,
    pub(super) base: Size,
    /// The block's padding-box origin, and below the child's own, from the
    /// origin of the frame the child's box is measured from (see
    /// [`crate::LayoutFrame`]).
    pub(super) base_origin: Point,
    pub(super) origin: Point,
    pub(super) size: Size,
}

impl PlannedOverlay {
    /// Whether `other` places the child the same way, against the same
    /// containing block.
    pub(super) fn same_placement(&self, other: &Self) -> bool {
        let same_size = |a: Size, b: Size| {
            a.width.to_bits() == b.width.to_bits() && a.height.to_bits() == b.height.to_bits()
        };
        self.block == other.block
            && self.base_origin == other.base_origin
            && same_size(self.base, other.base)
            && self.origin == other.origin
            && same_size(self.size, other.size)
    }
}

/// One child's contribution to a cached container placement.
#[derive(Clone)]
pub(super) struct PlannedChild {
    pub(super) child: StableNodeId,
    /// The child's own layout style at plan time, compared by pointer. This is
    /// what catches a style edit that moves a child without resizing it --
    /// `margin`, `align_self`, `order`, `flex_grow`.
    pub(super) style: Arc<nana_ui_core::LayoutStyle>,
    /// Intrinsic size measured BEFORE flex distribution: the pure input the
    /// rest of the container's placement is a function of.
    pub(super) intrinsic: Size,
    /// The main size the line gave this child when it differs from the
    /// intrinsic one, and the child measured at it: its cross size is then
    /// the line's input, and an edit can change it while the intrinsic stays.
    pub(super) at_main: Option<Box<(f32, Size)>>,
    pub(super) origin: Point,
    pub(super) size: Size,
    /// Main-axis cursor before this child, i.e. the prefix sum of every
    /// preceding child's outer main extent plus gaps. Lets a suffix replay
    /// start at any index in O(1) instead of re-accumulating from zero.
    pub(super) cursor_before: f32,
    /// The first baseline the child's line aligned it by, when it aligns by
    /// baseline. A child whose baseline moves moves the rest of its line,
    /// even when no size changed: its text grew inside a fixed box.
    pub(super) baseline: PackedOptF32,
}

const _: () = assert!(std::mem::size_of::<PlannedChild>() <= 56);

/// One grid item's occupied tracks and the intrinsic contribution those
/// tracks were solved from. Spans are the placement result, not a second
/// copy of the child's style.
#[derive(Clone)]
pub(super) struct GridItemPlan {
    pub(super) child: StableNodeId,
    pub(super) col: u32,
    pub(super) row: u32,
    pub(super) col_span: u32,
    pub(super) row_span: u32,
    pub(super) contribution: Size,
}

/// Resolved grid tracks plus the contribution that produced them.
///
/// A later pass re-solves every track from these contributions. Items whose
/// contribution and cell constraint are unchanged are not measured again.
/// Subgrid, a child entering or leaving flow, and a container whose own
/// content-sized keywords this record cannot finish fall back to measuring
/// this grid, not the document.
#[derive(Clone)]
pub(super) struct GridTrackPlan {
    pub(super) items: Vec<GridItemPlan>,
    pub(super) col_sizes: Vec<f32>,
    pub(super) row_sizes: Vec<f32>,
    pub(super) col_gap: f32,
    pub(super) row_gap: f32,
}

impl GridTrackPlan {
    pub(super) fn from_layout(grid: &Grid2DLayout) -> Self {
        Self {
            items: grid
                .items
                .iter()
                .map(|item| GridItemPlan {
                    child: item.id,
                    col: item.col as u32,
                    row: item.row as u32,
                    col_span: item.col_span as u32,
                    row_span: item.row_span as u32,
                    contribution: item.intrinsic,
                })
                .collect(),
            col_sizes: grid.col_sizes.clone(),
            row_sizes: grid.row_sizes.clone(),
            col_gap: grid.col_gap,
            row_gap: grid.row_gap,
        }
    }

    pub(super) fn cell(&self, item: &GridItemPlan) -> (f32, f32) {
        (
            grid_span_extent(
                &self.col_sizes,
                item.col as usize,
                item.col_span as usize,
                self.col_gap,
            ),
            grid_span_extent(
                &self.row_sizes,
                item.row as usize,
                item.row_span as usize,
                self.row_gap,
            ),
        )
    }

    pub(super) fn inherited_tracks_for(
        &self,
        child: StableNodeId,
        child_style: &LayoutStyle,
    ) -> Option<Box<grid::InheritedGridTracks>> {
        if !child_style.is_subgrid_columns() && !child_style.is_subgrid_rows() {
            return None;
        }
        let item = self.items.iter().find(|item| item.child == child)?;
        Some(Box::new(grid::InheritedGridTracks {
            area_inline: grid::grid_span_extent(
                &self.col_sizes,
                item.col as usize,
                item.col_span as usize,
                self.col_gap,
            ),
            columns: child_style.is_subgrid_columns().then(|| {
                grid::spanned_track_sizes(
                    &self.col_sizes,
                    item.col as usize,
                    item.col_span as usize,
                )
            }),
            column_gap: self.col_gap,
            rows: child_style.is_subgrid_rows().then(|| {
                grid::spanned_track_sizes(
                    &self.row_sizes,
                    item.row as usize,
                    item.row_span as usize,
                )
            }),
            row_gap: self.row_gap,
        }))
    }
}

pub(super) fn grid_child_in_flow(style: &LayoutStyle) -> bool {
    !style.omits_box() && !style.position.is_out_of_flow()
}

/// Both axes are a length that does not read the grid content box, so a
/// sibling-driven change of that box does not change this item's contribution.
pub(super) fn grid_contribution_ignores_content_box(style: &LayoutStyle) -> bool {
    fn fixed(spec: Option<LengthSpec>) -> bool {
        spec.is_some_and(|spec| {
            matches!(
                spec,
                LengthSpec::Px(_) | LengthSpec::Em(_) | LengthSpec::Rem(_)
            ) || spec.as_em_offset().is_some()
                || spec.as_rem_offset().is_some()
        })
    }
    style.aspect_ratio.is_none()
        && fixed(style.width)
        && fixed(style.height)
        && style.min_width.is_none_or(|spec| fixed(Some(spec)))
        && style.min_height.is_none_or(|spec| fixed(Some(spec)))
        && style.max_width.is_none_or(|spec| fixed(Some(spec)))
        && style.max_height.is_none_or(|spec| fixed(Some(spec)))
}

pub(super) fn sort_ids_with_sizes(
    ids: &mut [StableNodeId],
    sizes: &mut [Size],
    nodes: &LayoutInputMap<'_>,
) {
    if ids.len() != sizes.len() {
        return;
    }
    let order_of = |id: StableNodeId| nodes.style_ref(id).map(|style| style.order).unwrap_or(0);
    if ids.iter().copied().all(|id| order_of(id) == 0) {
        return;
    }
    let mut order: Vec<usize> = (0..ids.len()).collect();
    order.sort_by_key(|&index| order_of(ids[index]));
    let old_ids = ids.to_vec();
    let old_sizes = sizes.to_vec();
    for (slot, index) in order.into_iter().enumerate() {
        ids[slot] = old_ids[index];
        sizes[slot] = old_sizes[index];
    }
}

/// Where a container's child list differs from the one a plan recorded: the
/// children between the prefix and the suffix the two lists share left it
/// (`old[start..old_end]`) and arrived (`new[start..new_end]`). An insertion,
/// a removal, or a move within the list is one such range; a child that moved
/// within it both left and arrived.
///
/// A sequential plan takes the edit instead of walking every child again: a
/// child that left takes its share out, one that arrived is measured and puts
/// its share in, and the children after the edit move by the difference.
/// Finding the range compares ids; it measures nothing.
#[derive(Debug, Clone, Copy)]
pub(super) struct ChildListEdit {
    pub(super) start: usize,
    pub(super) old_end: usize,
    pub(super) new_end: usize,
}

impl ChildListEdit {
    pub(super) fn between(old: &[StableNodeId], new: &[StableNodeId]) -> Self {
        let start = old
            .iter()
            .zip(new)
            .take_while(|(old, new)| old == new)
            .count();
        let shared = old.len().min(new.len()) - start;
        let suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(shared)
            .take_while(|(old, new)| old == new)
            .count();
        Self {
            start,
            old_end: old.len() - suffix,
            new_end: new.len() - suffix,
        }
    }

    pub(super) fn left<'a>(&self, old: &'a [StableNodeId]) -> &'a [StableNodeId] {
        &old[self.start..self.old_end]
    }

    pub(super) fn arrivals<'a>(&self, new: &'a [StableNodeId]) -> &'a [StableNodeId] {
        &new[self.start..self.new_end]
    }

    /// Whether `child`, one of `container`'s children now, arrived in this
    /// edit, by its index in the new list.
    pub(super) fn arrived(
        &self,
        world: &UiWorld,
        container: StableNodeId,
        child: StableNodeId,
    ) -> bool {
        world
            .child_index(container, child)
            .is_some_and(|index| (self.start..self.new_end).contains(&index))
    }
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
pub(super) struct ContainerPlan {
    pub(super) origin: Point,
    pub(super) size: Size,
    pub(super) containing: Size,
    pub(super) parent_font_px: f32,
    pub(super) viewport: LayoutViewport,
    /// The container's effective style, compared by pointer.
    pub(super) style: Arc<nana_ui_core::LayoutStyle>,
    /// The container's child list, compared by pointer. A structural edit
    /// copy-on-writes this `Arc` (the cache holds a reference, so the world's
    /// `Arc::make_mut` cannot mutate it in place), so a different pointer is a
    /// different list.
    pub(super) children: Arc<Vec<StableNodeId>>,
    /// Containing block handed to each child.
    pub(super) content: Size,
    pub(super) child_font_px: f32,
    /// Available size each child's intrinsic measurement was taken against.
    pub(super) child_available: Size,
    pub(super) main_direction: FlexDirection,
    /// The writing mode and direction the container laid out in, inherited.
    /// An ancestor can change it without touching this container's own style,
    /// so the plan compares it with the other inputs.
    pub(super) writing: nana_ui_core::WritingContext,
    /// The main and cross axes run from their far page edge — the right or
    /// the bottom. Placement is flow-relative (every cursor and margin is read
    /// from the start edge) and only turned onto the page where an origin is
    /// written; the replay turns it the same way.
    pub(super) main_reversed: bool,
    pub(super) cross_reversed: bool,
    /// Origin of the container's content box.
    pub(super) content_origin: Point,
    /// Main-axis gap between children.
    pub(super) gap: f32,
    /// True when this container placed its children as a plain accumulation
    /// from the main-start edge, so child `i`'s position depends only on the
    /// children before it — on a reversed axis too, where it is measured back
    /// from the far edge. Everything that would couple siblings is excluded:
    /// wrapping, a `justify-content` that distributes free space, grid tracks,
    /// auto main margins, baseline or center/end cross alignment, and any flex
    /// grow/shrink redistribution (detected from the data -- every child's
    /// used main size equalled its intrinsic).
    ///
    /// Under that shape a resized child shifts exactly the children after it,
    /// so the pass can keep the prefix and replay only the suffix.
    pub(super) sequential: bool,
    /// In placement order.
    pub(super) entries: Vec<PlannedChild>,
    /// Main extent of the content box the entries' origins were placed in.
    /// On a reversed main axis an origin is measured back from the far edge,
    /// so when the container's main size moves, every child before the first
    /// changed one moves with that edge. The replay shifts them by the
    /// difference and records the new extent here.
    pub(super) placed_main: f32,
    /// `(child, index into entries)`, sorted by child for large containers so
    /// the pass can probe the change closure without scanning every entry.
    /// Small containers (`<= BY_CHILD_LINEAR_CAP` children) omit the side
    /// index and search `entries` directly.
    pub(super) by_child: Option<Box<[(StableNodeId, u32)]>>,
    /// Occupied tracks, contributions, and the resolved track sizes. `None`
    /// on every flex and block plan.
    pub(super) grid: Option<Box<GridTrackPlan>>,
    /// No in-flow child reads this container's content box on either axis,
    /// so a change of this container's own size refreshes positioned children
    /// only.
    pub(super) cross_independent: bool,
    /// Some in-flow child reads this container's main extent: a main size,
    /// limit or basis against the containing block, say. Its entries hold
    /// the sizes the old extent gave, so a new main size retires the plan.
    pub(super) main_dependent: bool,
    /// Positioned participants of this container. Empty on a flow-only plan.
    pub(super) overlay: Vec<PlannedOverlay>,
}

impl ContainerPlan {
    /// Everything the container's own placement depends on, other than its
    /// child list, its children and its used border-box size. A mismatch
    /// here means the plan is about a different layout.
    pub(super) fn flow_identity_holds(
        &self,
        origin: Point,
        containing: Size,
        parent_font_px: f32,
        viewport: LayoutViewport,
        style: &Arc<nana_ui_core::LayoutStyle>,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        self.writing == writing
            && self.origin == origin
            && self.containing_compatible(containing)
            && self.parent_font_px == parent_font_px
            && viewport_basis(&self.style, self.viewport) == viewport_basis(style, viewport)
            && Arc::ptr_eq(&self.style, style)
    }

    /// Everything the container's own placement depends on, other than its
    /// child list and its children. A mismatch here means the plan is about
    /// a different layout.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn inputs_match(
        &self,
        origin: Point,
        size: Size,
        containing: Size,
        parent_font_px: f32,
        viewport: LayoutViewport,
        style: &Arc<nana_ui_core::LayoutStyle>,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        self.flow_identity_holds(origin, containing, parent_font_px, viewport, style, writing)
            && self.size_compatible(size)
    }

    /// In-flow start edges stay put when this container's cross size changes
    /// and no child stretches to it. Positioned children that read the
    /// containing block are refreshed separately.
    pub(super) fn flow_stable_under_own_size(&self, content_origin: Point) -> bool {
        self.cross_independent
            && self.sequential
            && self.grid.is_none()
            && !self.main_reversed
            && !self.cross_reversed
            && self.content_origin == content_origin
    }

    /// Flow reuse, including a containing-block size change that does not
    /// move in-flow children. The child list is the caller's to compare: the
    /// same list replays the children the change closure reaches, a new one
    /// the edit (see [`ChildListEdit`]).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn can_reuse_flow(
        &self,
        origin: Point,
        size: Size,
        containing: Size,
        content_origin: Point,
        parent_font_px: f32,
        viewport: LayoutViewport,
        style: &Arc<nana_ui_core::LayoutStyle>,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        // The content box's origin is not only the box's origin and its
        // style's padding: a line that shrank the box may have closed its
        // padding up (Issue #212), which moves every child.
        (self.content_origin == content_origin
            && self.inputs_match(
                origin,
                size,
                containing,
                parent_font_px,
                viewport,
                style,
                writing,
            ))
            || (self.flow_identity_holds(
                origin,
                containing,
                parent_font_px,
                viewport,
                style,
                writing,
            ) && self.flow_stable_under_own_size(content_origin))
    }

    /// A sequential plan places from the start edge. Its main size can grow
    /// with a child without moving the cross axis or, on a forward main axis,
    /// the prefix. On a reversed one the replay moves the prefix with the far
    /// edge; see [`Self::placed_main`].
    pub(super) fn size_compatible(&self, size: Size) -> bool {
        if self.size == size {
            return true;
        }
        // Track sizes are solved again from the recorded contributions, so
        // the used border box may grow with a row or a column. A pass whose
        // children did not change still re-solves when this size moved.
        if self.grid.is_some() {
            return true;
        }
        if self.sequential {
            let held = |extent: fn(Size, FlexDirection) -> f32| {
                extent(self.size, self.main_direction).to_bits()
                    == extent(size, self.main_direction).to_bits()
            };
            return held(cross_extent) && (!self.main_dependent || held(main_extent));
        }
        // A wrap container's cross size is the sum of its line cross sizes.
        // The main size is the line budget; if that moved, line membership
        // has to be solved again by the formatting context.
        if !flex_line_local_style(self.style.as_ref()) || self.main_reversed || self.cross_reversed
        {
            return false;
        }
        match self.main_direction {
            FlexDirection::Row => {
                self.style.height.is_none() && self.size.width.to_bits() == size.width.to_bits()
            }
            FlexDirection::Column => {
                self.style.width.is_none() && self.size.height.to_bits() == size.height.to_bits()
            }
        }
    }

    /// The line budget is this container's main size. The parent's cross size
    /// can grow with this container without changing that budget.
    pub(super) fn containing_compatible(&self, containing: Size) -> bool {
        if self.containing == containing {
            return true;
        }
        if self.grid.is_some() {
            return true;
        }
        // A container reads its containing block for percentage padding and
        // margins, which resolve against its inline size, and for
        // percentage relative offsets. A content-sized ancestor growing
        // along the block axis changes neither: the plan still holds.
        let inline = |size: Size| self.writing.inline_size(size.width, size.height).to_bits();
        if inline(self.containing) == inline(containing)
            && !reads_containing_block_size(&self.style)
        {
            return true;
        }
        if !flex_line_local_style(self.style.as_ref()) || self.main_reversed || self.cross_reversed
        {
            return false;
        }
        match self.main_direction {
            FlexDirection::Row => self.containing.width.to_bits() == containing.width.to_bits(),
            FlexDirection::Column => {
                self.containing.height.to_bits() == containing.height.to_bits()
            }
        }
    }

    pub(super) fn child_count(&self) -> usize {
        self.entries.len()
    }

    /// The index in `entries` of `child`'s recorded placement.
    pub(super) fn entry_index(&self, child: StableNodeId) -> Option<u32> {
        if let Some(by_child) = self.by_child.as_deref() {
            return by_child
                .binary_search_by_key(&child, |(child, _)| *child)
                .ok()
                .map(|slot| by_child[slot].1);
        }
        self.entries
            .iter()
            .position(|entry| entry.child == child)
            .map(|index| index as u32)
    }

    /// Every affected direct child still has the role this plan recorded:
    /// in flow, positioned, or omitted. A child that enters or leaves flow, or
    /// becomes positioned, changes the participant lists themselves, which no
    /// replay can patch. Driven from the closure, like [`Self::affected_entries`].
    /// A child that arrived in a child-list `edit` has no role here yet; the
    /// edit places it.
    pub(super) fn flow_membership_holds(
        &self,
        container: StableNodeId,
        edit: Option<&ChildListEdit>,
        scope: &ScopeContext<'_>,
        nodes: &LayoutInputMap<'_>,
    ) -> bool {
        scope.affected_children(container).iter().all(|&child| {
            if nodes.world.parent_id(child) != Some(container)
                || edit.is_some_and(|edit| edit.arrived(nodes.world, container, child))
            {
                return true;
            }
            let Some(style) = nodes.style_ref(child) else {
                return false;
            };
            // Only a container that lays no lines out records a plan: an
            // inline-level child in its flow now makes it one that does.
            let leaves_flow = crate::layout_node::leaves_parent_flow(Some(&self.style), &style);
            if style.is_inline_level()
                && !leaves_flow
                && !self.style.display.is_some_and(|display| {
                    display.is_flex_container() || display.is_grid_container()
                })
            {
                return false;
            }
            let was_in_flow = self.entry_index(child).is_some();
            let was_positioned = self.overlay.iter().any(|entry| entry.child == child);
            if style.omits_box() {
                !was_in_flow && !was_positioned
            } else if leaves_flow {
                was_positioned
            } else {
                was_in_flow
            }
        })
    }

    /// Entry indices for the children the change closure reaches, in placement
    /// order. Driven from the closure (small) rather than the child list.
    pub(super) fn affected_entries(&self, scope: &ScopeContext<'_>) -> Vec<u32> {
        let mut indices: Vec<u32> = scope
            .affected
            .iter()
            .filter_map(|&id| self.entry_index(id))
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
/// That distinction is not academic. Writers can disagree about how to spell
/// a default column: a cascade that republishes a style leaves it `None`, and
/// the Runtime's own `Stack` projection writes `Some(Column)` back. On a
/// 2,000-row list where both run every pointer event, a plain `==`
/// retires the cached plan on every frame while nothing about the layout has
/// changed. That was the whole of the measure plan's benefit: on that
/// benchmark, `==` left settle at 0.786 ms and the container re-measuring
/// 2,000 children per event; this comparison takes it to 0.630 ms and 5.4.
///
/// The equality is exact, not a tolerance: it accepts exactly the pairs that
/// `used_flow_direction` maps to the same axis, and every other field still has
/// to match outright.
/// Whether a flex item's `current` style is its `cached` one but for its
/// cross size: the one change a line-local replay recomputes. Anything else
/// (a margin, `align-self`, `flex-grow`) moves the line or its neighbours.
pub(super) fn same_but_cross(
    current: &nana_ui_core::LayoutStyle,
    cached: &nana_ui_core::LayoutStyle,
    direction: FlexDirection,
) -> bool {
    let diff = current.changed_fields(cached);
    let (allowed, same_bounds) = match direction {
        FlexDirection::Row => (
            LayoutStyleChange::HEIGHT,
            current.min_height == cached.min_height && current.max_height == cached.max_height,
        ),
        FlexDirection::Column => (
            LayoutStyleChange::WIDTH,
            current.min_width == cached.min_width && current.max_width == cached.max_width,
        ),
    };
    diff.difference(allowed).is_empty() && same_bounds && current.theme == cached.theme
}

/// Whether `style` resolves anything against its containing block's block
/// size: a percentage offset (relative positioning) or a percentage size.
pub(super) fn reads_containing_block_size(style: &nana_ui_core::LayoutStyle) -> bool {
    let percent = |spec: Option<LengthSpec>| matches!(spec, Some(LengthSpec::Percent(_)));
    percent(style.offset_top)
        || percent(style.offset_bottom)
        || percent(style.offset_left)
        || percent(style.offset_right)
        || percent(style.width)
        || percent(style.height)
        || percent(style.min_width)
        || percent(style.min_height)
        || percent(style.max_width)
        || percent(style.max_height)
}

/// [`layout_inputs_equal`] for a measurement, which no inset reaches: insets
/// place a positioned box in its containing block. What they do to its size
/// -- two opposite insets stretch it -- arrives as the constraint it is
/// measured against, which a plan compares on its own. A virtual row moved
/// down by the row above it keeps its measurement.
pub(super) fn measure_inputs_equal(
    a: &nana_ui_core::LayoutStyle,
    b: &nana_ui_core::LayoutStyle,
) -> bool {
    let diff = a.changed_fields(b);
    if diff.is_empty() {
        return a.theme == b.theme;
    }
    diff.difference(LayoutStyleChange::POSITION).is_empty()
        && a.position == b.position
        && a.anchor == b.anchor
        && a.logical_edges.inset == b.logical_edges.inset
        && a.theme == b.theme
}

pub(super) fn layout_inputs_equal(
    a: &nana_ui_core::LayoutStyle,
    b: &nana_ui_core::LayoutStyle,
) -> bool {
    std::ptr::eq(a, b) || (a.changed_fields(b).is_empty() && a.theme == b.theme)
}

/// Wrap flex whose line breaks depend only on each item's main size.
pub(super) fn flex_line_local_style(style: &LayoutStyle) -> bool {
    style
        .display
        .is_some_and(|display| display.is_flex_container())
        && matches!(style.flex_wrap, FlexWrap::Wrap)
        && style.justify_content == JustifySpec::Start
        && style.align_items == AlignSpec::Start
        && style.align_content == JustifySpec::Start
        && style.aspect_ratio.is_none()
        && !style.flex_reverse
}

pub(super) fn child_blocks_flex_line_local(style: &LayoutStyle) -> bool {
    style.flex_grow.unwrap_or(0.0) > 0.0
        || style.flex_shrink.unwrap_or(0.0) > 0.0
        || style.order != 0
        || style.aspect_ratio.is_some()
        || style
            .align_self
            .is_some_and(|align| align != AlignSpec::Start)
        || matches!(
            style.margin_left,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
        || matches!(
            style.margin_right,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
        || matches!(
            style.margin_top,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
        || matches!(
            style.margin_bottom,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
}

/// One child's contribution to a cached container measurement.
#[derive(Clone)]
pub(super) struct MeasuredChild {
    pub(super) child: StableNodeId,
    /// The child's effective layout style at plan time, or `None` when the node
    /// was missing. Compared by pointer with a value fallback, for the same
    /// reason as in [`ContainerPlan`]: a host that rebuilds its style objects
    /// every frame hands back a fresh `Arc` holding an identical style.
    pub(super) style: Option<Arc<nana_ui_core::LayoutStyle>>,
    /// The intrinsic size measured for this child, or `None` for a child the
    /// flow collection dropped (`display:none`, out of flow). A dropped child
    /// contributes nothing to the container's measurement, and it cannot start
    /// contributing without its own style changing -- which the style compare
    /// above catches.
    pub(super) intrinsic: PackedOptSize,
    /// The main size the line gave this child when it differs from the
    /// measured one, and the child measured at it (its cross size is the
    /// container's input then).
    pub(super) at_main: Option<Box<(f32, Size)>>,
    /// An atom on a line of an ordinary block: its ascent on the line
    /// ([`inline::ifc_atom_metrics`]). The line's height reads it, so it
    /// can move while the atom's size stays.
    pub(super) baseline: PackedOptF32,
}

const _: () = assert!(std::mem::size_of::<MeasuredChild>() <= 40);

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
/// `retained.intrinsics.remove` in `layout_document_with_frontier`. That removal is
/// still right and still happens: an affected node's memo holds entries for
/// constraint combinations this frame will not measure, and those really are
/// stale. What a plan caches is the container's result under ONE recorded
/// constraint, re-validated against the closure before it is used, so the two
/// do not overlap.
#[derive(Clone)]
pub(super) struct MeasurePlan {
    /// Constraint the container was measured against. This is the part of the
    /// per-pass cache key that the plan, keyed by id alone, has to carry.
    pub(super) available: Size,
    pub(super) parent_direction: Option<FlexDirection>,
    pub(super) parent_stretches: bool,
    pub(super) viewport: LayoutViewport,
    pub(super) parent_font_px: f32,
    /// The container's effective style, compared by pointer with a value
    /// fallback.
    pub(super) style: Arc<nana_ui_core::LayoutStyle>,
    /// The writing mode and direction the container measured in, inherited;
    /// see [`ContainerPlan::writing`].
    pub(super) writing: nana_ui_core::WritingContext,
    /// The container's child list, compared by pointer. A structural edit
    /// copy-on-writes this `Arc`, so a different pointer is a different list.
    pub(super) children: Arc<Vec<StableNodeId>>,
    /// The container's own content beside its children: `None` for a
    /// container with none (no text, strut or visual), as most are, so the
    /// plan stays small.
    pub(super) own: Option<Box<PlanOwnContent>>,
    /// Available size in-flow children were measured against. Flex uses one
    /// value for every child. A grid stores the content box here; each item's
    /// contribution lives on [`GridTrackPlan`], and a fill axis is applied
    /// only when that contribution is remeasured.
    pub(super) child_available: Size,
    /// Flow direction handed to each child as its `parent_direction`.
    pub(super) child_direction: FlexDirection,
    /// Sorted by child id.
    pub(super) entries: Vec<MeasuredChild>,
    /// What the measurement produced.
    pub(super) size: Size,
    /// Main size is the sum of child border boxes, margins, and gaps.
    pub(super) sequential: bool,
    /// On a sequential plan, that sum, exact: see `sequential_main_term`.
    pub(super) main_sum: f64,
    /// On a sequential plan, the in-flow children the sum holds, one gap
    /// between each two.
    pub(super) flow_len: usize,
    /// On a sequential plan, the widest in-flow child's cross extent with its
    /// margins: a child arriving within it leaves the cross size where it
    /// was, and so does one leaving while another is as wide.
    pub(super) children_cross: f32,
    /// How many in-flow children are that wide.
    pub(super) children_at_cross: usize,
    /// The cross-axis size the measurement handed its final step, before the
    /// container's own size rules. A sequential patch keeps it.
    pub(super) default_cross: f32,
    /// Present when this measurement is a grid track solution.
    pub(super) grid: Option<Box<GridTrackPlan>>,
    /// A child of the container changed in a pass that measured it under
    /// another constraint only: this plan still holds that child as it was,
    /// and re-checking the children a later pass reaches would not find it.
    /// It is never reused; a walk of the children records a fresh one.
    pub(super) stale: bool,
}

/// What a container measured of its own beside its children
/// ([`MeasurePlan::own`]).
#[derive(Clone, Copy, PartialEq)]
pub(super) struct PlanOwnContent {
    /// The container's own shaped text, which competes with the children for
    /// the content size.
    pub(super) text_metrics: Option<crate::TextMetrics>,
    /// That text's unwrapped width, when it wrapped narrower.
    pub(super) text_natural_width: Option<f32>,
    /// The width that text was last wrapped against.
    pub(super) text_wrap_limit: Option<f32>,
    /// The face metrics its line boxes started from (`UiWorld::block_strut`).
    pub(super) strut: Option<crate::FontStrut>,
    /// What the container's visual draws beside that text.
    pub(super) visual: VisualContent,
}

impl PlanOwnContent {
    /// Nothing at all, to the bit: what a plan leaves out (`own: None`).
    pub(super) fn is_none(&self) -> bool {
        self.text_metrics.is_none()
            && self.text_natural_width.is_none()
            && self.text_wrap_limit.is_none()
            && self.strut.is_none()
            && self.visual.is_nothing()
    }

    /// Boxed, or `None` when there is nothing.
    pub(super) fn boxed(self) -> Option<Box<Self>> {
        (!self.is_none()).then(|| Box::new(self))
    }
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
pub(super) struct MeasurePlanSlots {
    /// Boxed: most containers hold one plan, and the table stays small.
    pub(super) slots: [Option<Box<MeasurePlan>>; 2],
}

impl MeasurePlanSlots {
    pub(super) fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    /// Picks the slot recorded under this constraint. Selection only, not a
    /// correctness check: `MeasurePlan::inputs_match` compares `available`
    /// again, so handing back the wrong slot costs a recompute, never a stale
    /// answer.
    pub(super) fn get(&self, available: Size) -> Option<&MeasurePlan> {
        self.slots
            .iter()
            .flatten()
            .find(|plan| plan.available == available)
            .map(Box::as_ref)
    }

    pub(super) fn insert(&mut self, plan: impl Into<Box<MeasurePlan>>) {
        let plan = plan.into();
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

    pub(super) fn clear(&mut self) {
        self.slots = [None, None];
    }

    /// The plans this holder carries, most recent first.
    pub(super) fn into_plans(self) -> impl DoubleEndedIterator<Item = Box<MeasurePlan>> {
        self.slots.into_iter().flatten()
    }
}

impl MeasurePlan {
    /// Everything the measurement depends on other than the child list and
    /// the children. The caller compares the list: the same one re-checks the
    /// children the change closure reaches, a new one takes the edit (see
    /// [`ChildListEdit`]).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn inputs_match(
        &self,
        available: Size,
        parent_direction: Option<FlexDirection>,
        parent_stretches: bool,
        viewport: LayoutViewport,
        parent_font_px: f32,
        style: &Arc<nana_ui_core::LayoutStyle>,
        text_metrics: Option<crate::TextMetrics>,
        text_natural_width: Option<f32>,
        text_wrap_limit: Option<f32>,
        strut: Option<crate::FontStrut>,
        visual: VisualContent,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        let own_matches = match self.own.as_deref() {
            // Compared as the fields were: with `==`, against nothing.
            None => {
                strut.is_none()
                    && text_metrics.is_none()
                    && text_natural_width.is_none()
                    && text_wrap_limit.is_none()
                    && visual == VisualContent::default()
            }
            Some(own) => {
                own.strut == strut
                    && own.text_metrics == text_metrics
                    && own.text_natural_width == text_natural_width
                    && own.text_wrap_limit == text_wrap_limit
                    && own.visual == visual
            }
        };
        self.writing == writing
            && own_matches
            && self.available == available
            && self.parent_direction == parent_direction
            && self.parent_stretches == parent_stretches
            && viewport_basis(&self.style, self.viewport) == viewport_basis(style, viewport)
            && self.parent_font_px == parent_font_px
            && (Arc::ptr_eq(&self.style, style) || measure_inputs_equal(&self.style, style))
    }

    pub(super) fn text_metrics(&self) -> Option<crate::TextMetrics> {
        self.own.as_ref().and_then(|own| own.text_metrics)
    }

    pub(super) fn entry(&self, child: StableNodeId) -> Option<&MeasuredChild> {
        self.entries
            .binary_search_by_key(&child, |entry| entry.child)
            .ok()
            .map(|slot| &self.entries[slot])
    }
}

/// What a pass decided about its containers' plans: the ones it rebuilt, and
/// the containers whose retained plan it retires. A container is in one or
/// neither.
///
/// The rebuilt plans sit in a table of the retained cache's own type, so a
/// first layout hands the table over whole ([`Self::merge_into`]) instead of
/// copying every plan into a second table of the same size and freeing this
/// one: 3.1 MB a pass on a 21 001-box page, which the allocator then kept.
#[derive(Default)]
pub(super) struct PassContainerPlans {
    pub(super) rebuilt: HashMap<StableNodeId, ContainerPlan>,
    pub(super) retired: crate::NodeSet,
    /// What replays changed in a retained plan they kept, by container:
    /// applied when the pass commits, so the pass reads the previous pass's
    /// plans without writing them (see [`PlanPatch`]).
    pub(super) patched: HashMap<StableNodeId, PlanPatch>,
}

/// The rows a replay moved in a retained [`ContainerPlan`] it kept, and the
/// main extent it placed them against.
///
/// A pass reads the retained cache and writes only its own tables, which
/// the commit merges at the frame boundary: what a pass has not finished
/// leaves the previous pass's results whole (#399). A replay that keeps a
/// long list's plan and moves its tail records just the tail here, rather
/// than a copy of the plan, so a tail edit stays as cheap as the replay.
#[derive(Default)]
pub(super) struct PlanPatch {
    /// Replaced entries, by index, ascending.
    pub(super) rows: Vec<(u32, PlannedChild)>,
    pub(super) placed_main: Option<f32>,
}

impl PlanPatch {
    /// The entry at `index`, as the patch leaves it.
    pub(super) fn entry<'a>(&'a self, plan: &'a ContainerPlan, index: usize) -> &'a PlannedChild {
        match self
            .rows
            .binary_search_by_key(&(index as u32), |(row, _)| *row)
        {
            Ok(slot) => &self.rows[slot].1,
            Err(_) => &plan.entries[index],
        }
    }

    pub(super) fn apply(self, plan: &mut ContainerPlan) {
        for (index, entry) in self.rows {
            plan.entries[index as usize] = entry;
        }
        if let Some(placed_main) = self.placed_main {
            plan.placed_main = placed_main;
        }
    }
}

impl PassContainerPlans {
    pub(super) fn reserve(&mut self, containers: usize) {
        self.rebuilt.reserve(containers);
    }

    /// `plan` is `id`'s from this pass on. It replaces any patch the pass
    /// recorded for the retained one; see [`Self::patched_copy`].
    pub(super) fn rebuild(&mut self, id: StableNodeId, plan: ContainerPlan) {
        self.retired.remove(&id);
        self.patched.remove(&id);
        self.rebuilt.insert(id, plan);
    }

    /// `id` keeps no plan: neither one this pass rebuilt nor the retained one.
    pub(super) fn retire(&mut self, id: StableNodeId) {
        self.rebuilt.remove(&id);
        self.patched.remove(&id);
        self.retired.insert(id);
    }

    /// `id` keeps its retained plan, with `patch` applied when the pass
    /// commits.
    pub(super) fn patch(&mut self, id: StableNodeId, patch: PlanPatch) {
        self.rebuilt.remove(&id);
        self.retired.remove(&id);
        self.patched.insert(id, patch);
    }

    /// A copy of `retained`, `id`'s retained plan, with what this pass
    /// patched in it: where the pass rebuilds a plan it kept.
    pub(super) fn patched_copy(
        &mut self,
        id: StableNodeId,
        retained: &ContainerPlan,
    ) -> ContainerPlan {
        let mut plan = retained.clone();
        if let Some(patch) = self.patched.remove(&id) {
            patch.apply(&mut plan);
        }
        plan
    }

    /// What this pass decided for `id`: `Some(None)` when it retired the
    /// plan, `None` when it left the retained one alone.
    pub(super) fn get(&self, id: &StableNodeId) -> Option<Option<&ContainerPlan>> {
        match self.rebuilt.get(id) {
            Some(plan) => Some(Some(plan)),
            None => self.retired.contains(id).then_some(None),
        }
    }

    /// The plan this pass rebuilt for `id`, to add to.
    pub(super) fn rebuilt_mut(&mut self, id: &StableNodeId) -> Option<&mut ContainerPlan> {
        self.rebuilt.get_mut(id)
    }

    /// Merges the pass's decisions into the retained plans and the set of
    /// containers with an overlay placed outside them.
    pub(super) fn merge_into(
        self,
        retained: &mut HashMap<StableNodeId, ContainerPlan>,
        remote_overlays: &mut crate::NodeSet,
    ) {
        for id in self.retired {
            retained.remove(&id);
            remote_overlays.remove(&id);
        }
        for (id, patch) in self.patched {
            if let Some(plan) = retained.get_mut(&id) {
                patch.apply(plan);
            }
        }
        let mut rebuilt = self.rebuilt;
        for (id, plan) in &mut rebuilt {
            // Gate E: one plan per container, entries are the direct
            // participants recorded this pass. A repeat edit replaces
            // the previous plan instead of appending another.
            bound_container_plan(plan);
            if plan
                .overlay
                .iter()
                .any(|entry| !entry.fixed && !entry.in_container)
            {
                remote_overlays.insert(*id);
            } else {
                remote_overlays.remove(id);
            }
        }
        if retained.is_empty() {
            // A first layout: the pass's table is the retained one.
            *retained = rebuilt;
        } else {
            retained.extend(rebuilt);
        }
    }
}

pub(super) fn dedupe_plan_rows<T: Clone>(rows: &mut Vec<T>, child_of: impl Fn(&T) -> StableNodeId) {
    // Most plans hold a few children, and a plan moved with its container
    // holds the ones it was recorded with: compare those in place rather
    // than hash them into a set built for the purpose.
    if rows.len() <= 16
        && rows.iter().enumerate().all(|(at, row)| {
            rows[..at]
                .iter()
                .all(|kept| child_of(kept) != child_of(row))
        })
    {
        return;
    }
    let mut seen = crate::NodeSet::with_capacity_and_hasher(rows.len(), Default::default());
    if rows.iter().all(|row| seen.insert(child_of(row))) {
        return;
    }
    seen.clear();
    let mut kept = Vec::with_capacity(rows.len());
    for row in rows.drain(..) {
        if seen.insert(child_of(&row)) {
            kept.push(row);
        }
    }
    *rows = kept;
}

pub(super) const BY_CHILD_LINEAR_CAP: usize = 32;

pub(super) fn build_by_child_index(entries: &[PlannedChild]) -> Option<Box<[(StableNodeId, u32)]>> {
    if entries.len() <= BY_CHILD_LINEAR_CAP {
        return None;
    }
    let mut by_child: Vec<(StableNodeId, u32)> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.child, index as u32))
        .collect();
    by_child.sort_unstable_by_key(|(child, _)| *child);
    Some(by_child.into_boxed_slice())
}

pub(super) fn bound_container_plan(plan: &mut ContainerPlan) {
    let before = plan.entries.len();
    dedupe_plan_rows(&mut plan.entries, |entry| entry.child);
    if plan.entries.len() != before {
        plan.by_child = build_by_child_index(&plan.entries);
    }
    dedupe_plan_rows(&mut plan.overlay, |entry| entry.child);
    if let Some(grid) = plan.grid.as_mut() {
        dedupe_plan_rows(&mut grid.items, |item| item.child);
    }
}

pub(super) fn bound_measure_plan(plan: &mut MeasurePlan) {
    dedupe_plan_rows(&mut plan.entries, |entry| entry.child);
    if let Some(grid) = plan.grid.as_mut() {
        dedupe_plan_rows(&mut grid.items, |item| item.child);
    }
}
