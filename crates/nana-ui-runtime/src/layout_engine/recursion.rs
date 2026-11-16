//! Bounded layout recursion helpers and stack-usage pricing.
//!
//! Measure runs iteratively on an explicit heap `Vec` work-stack in `measure.rs`,
//! so it uses O(1) call-stack depth regardless of tree depth.
//!
//! Placement descends at most [`SPAN`] levels before queuing remaining subtrees
//! as [`Boundary`] items, so placement call-stack depth is independent of tree
//! depth. Each pass records peak per-level stack usage in
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
