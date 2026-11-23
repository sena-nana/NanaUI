//! Bounded layout recursion helpers and stack-usage pricing.
//!
//! Measure runs iteratively on an explicit heap `Vec` work-stack in `measure.rs`,
//! so it uses O(1) call-stack depth regardless of tree depth.
//!
//! Placement descends at most [`SPAN`] levels before queuing remaining subtrees
//! as [`Boundary`] items, so placement call-stack depth is independent of tree
//! depth. A scroll container or a layout-isolated box is queued at any depth
//! (`placement::natural_boundary`): the units a scheduler can take apart.
//!
//! A pass reads the previous pass's retained cache and writes only its own
//! tables, merged when it commits; a replay that keeps a container's plan
//! records what it moved as a `PlanPatch`. So the previous pass's results
//! stay whole for whatever a pass does not finish.
//!
//! With a [`LayoutSliceBudget`] a pass drains its queue nearest the viewport
//! first and, once the budget is spent, leaves the out-of-view boundaries
//! that nothing retained is placed under for later passes: such a subtree
//! has no boxes until a pass places it, whole. See [`DeferredBoundary`]. Each pass records peak per-level stack usage in
//! `LayoutExecutionStats::{measure,placement}_stack_per_level`.

use super::{PassIntrinsicCache, Size};
use crate::StableNodeId;

/// Placement levels one call descends before the rest of a subtree waits in
/// the pass's queue: about 90 KiB of placement stack in either build, an
/// unoptimized level costing three times an optimized one. Tests run with
/// the smallest span, so every one of them places through boundaries.
pub(super) const SPAN: usize = if cfg!(test) {
    2
} else if cfg!(debug_assertions) {
    8
} else {
    24
};

/// A subtree whose root has its box and whose children wait to be placed:
/// where `place_node_body` resumes.
pub(super) struct Boundary {
    pub(super) id: StableNodeId,
    pub(super) size: Size,
    pub(super) containing: Size,
    pub(super) parent_font_px: f32,
    pub(super) inherited: Option<Box<super::grid::InheritedGridTracks>>,
    /// How far the root's box lies outside the viewport, on the page and
    /// scrolled, in px: `0` for one in view. Read only by a sliced pass.
    pub(super) distance: f32,
    /// A boundary an earlier pass deferred: nothing retained is placed
    /// under it, so a pass may defer it again.
    pub(super) resumed: bool,
}

/// How much placement a frame may do before it leaves what is out of view
/// to later frames (#399 stage 4). Off by default: a host opts in with
/// `AppContext::set_layout_slice_budget`.
///
/// Measurement is never sliced, so every box keeps the size its content
/// gives it. Placement is: the boundaries in view are always placed, then
/// the rest nearest the viewport first until the budget is spent, and at
/// least one of those each pass, so a sliced layout always finishes. A
/// boundary is deferred only while nothing retained is placed under it (a
/// first layout, or a subtree just mounted): its subtree then has no boxes
/// until a later pass places all of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutSliceBudget {
    /// Boxes a frame places before it defers.
    pub placement_nodes: Option<usize>,
    /// Time a frame's layout passes take before they defer, from the frame's
    /// start.
    pub time: Option<std::time::Duration>,
}

impl LayoutSliceBudget {
    /// A budget of `nodes` placed boxes a frame.
    pub const fn placement_nodes(nodes: usize) -> Self {
        Self {
            placement_nodes: Some(nodes),
            time: None,
        }
    }

    /// A budget of `time` a frame.
    pub const fn time(time: std::time::Duration) -> Self {
        Self {
            placement_nodes: None,
            time: Some(time),
        }
    }
}

/// What is left of a frame's budget for one pass.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SliceLimit {
    pub(crate) placement_nodes: Option<usize>,
    pub(crate) deadline: Option<std::time::Instant>,
}

/// A boundary a sliced pass left for a later one: the inputs its body is
/// placed from when no pass reaches its root's box again.
#[derive(Clone)]
pub(super) struct DeferredBoundary {
    pub(super) size: Size,
    pub(super) containing: Size,
    pub(super) parent_font_px: f32,
    pub(super) inherited: Option<Box<super::grid::InheritedGridTracks>>,
}

impl DeferredBoundary {
    pub(super) fn of(boundary: Boundary) -> (StableNodeId, Self) {
        (
            boundary.id,
            Self {
                size: boundary.size,
                containing: boundary.containing,
                parent_font_px: boundary.parent_font_px,
                inherited: boundary.inherited,
            },
        )
    }

    /// Whether a pass placing the root's box hands its body other inputs:
    /// what the deferred record held is no longer the work to do.
    pub(super) fn differs(&self, size: Size, containing: Size, parent_font_px: f32) -> bool {
        self.size != size
            || self.containing != containing
            || self.parent_font_px.to_bits() != parent_font_px.to_bits()
    }
}

/// A pass's slicing: what is left of the frame's budget, and the boundaries
/// deferred before it and by it.
#[derive(Default)]
pub(super) struct PassSlice {
    pub(super) limit: Option<SliceLimit>,
    /// The viewport's extent, which a boundary's distance is measured from.
    pub(super) viewport: Size,
    /// Out-of-view boundaries this pass placed.
    pub(super) placed_offscreen: usize,
    /// Boundaries earlier passes deferred that this pass has not reached.
    pub(super) pending: crate::NodeMap<DeferredBoundary>,
    /// Boundaries this pass deferred.
    pub(super) deferred: Vec<(StableNodeId, DeferredBoundary)>,
    /// Deferred boundaries dropped because their root left the document or
    /// stopped generating a box, or a pass placed it with other inputs.
    pub(super) cancelled: usize,
}

impl PassSlice {
    pub(super) fn active(&self) -> bool {
        self.limit.is_some()
    }

    /// Whether the budget is spent once `placed` boxes are placed.
    pub(super) fn spent(&self, placed: usize) -> bool {
        self.limit.is_some_and(|limit| {
            limit.placement_nodes.is_some_and(|nodes| placed >= nodes)
                || limit
                    .deadline
                    .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        })
    }

    /// `id` reached again with its box placed at these inputs: no longer a
    /// pending deferral but a boundary of this pass. Whether it was one.
    pub(super) fn reach(
        &mut self,
        id: StableNodeId,
        size: Size,
        containing: Size,
        parent_font_px: f32,
    ) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        let Some(record) = self.pending.remove(&id) else {
            return false;
        };
        if record.differs(size, containing, parent_font_px) {
            self.cancelled += 1;
        }
        true
    }
}

/// The pass's boundaries: first queued, first placed; a sliced pass places
/// the one nearest the viewport first.
#[derive(Default)]
pub(super) struct BoundaryQueue {
    fifo: std::collections::VecDeque<Boundary>,
    ranked: std::collections::BinaryHeap<Ranked>,
    queued: u64,
}

struct Ranked {
    distance: f32,
    order: u64,
    boundary: Boundary,
}

impl PartialEq for Ranked {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for Ranked {}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ranked {
    /// The nearest, then the first queued, is the greatest: what the heap
    /// pops first.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .distance
            .total_cmp(&self.distance)
            .then(other.order.cmp(&self.order))
    }
}

impl BoundaryQueue {
    pub(super) fn push(&mut self, boundary: Boundary, ranked: bool) {
        if ranked {
            self.queued += 1;
            self.ranked.push(Ranked {
                distance: boundary.distance,
                order: self.queued,
                boundary,
            });
        } else {
            self.fifo.push_back(boundary);
        }
    }

    pub(super) fn pop(&mut self) -> Option<Boundary> {
        match self.ranked.pop() {
            Some(ranked) => Some(ranked.boundary),
            None => self.fifo.pop_front(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Recursion {
    Placement,
}

/// The open levels of one recursion in a pass.
#[derive(Default)]
pub(super) struct LevelStack {
    /// Stack left at each open level's entry, or `None` where the platform
    /// cannot report it.
    open: Vec<Option<usize>>,
    /// Most stack one level took this pass, in bytes.
    pub(super) per_level: usize,
}

impl LevelStack {
    /// Levels open now.
    pub(super) fn depth(&self) -> usize {
        self.open.len()
    }

    fn enter(&mut self, remaining: Option<usize>) {
        if let (Some(Some(parent)), Some(here)) = (self.open.last(), remaining)
            && let Some(level) = parent.checked_sub(here)
        {
            self.per_level = self.per_level.max(level);
        }
        self.open.push(remaining);
    }

    fn exit(&mut self) {
        self.open.pop();
    }
}

/// Runs one level of `recursion` and prices its stack consumption.
pub(super) fn level<T>(
    cache: &mut PassIntrinsicCache,
    recursion: Recursion,
    run: impl FnOnce(&mut PassIntrinsicCache) -> T,
) -> T {
    let remaining = crate::stack::remaining();
    cache.levels(recursion).enter(remaining);
    let out = run(cache);
    cache.levels(recursion).exit();
    out
}
