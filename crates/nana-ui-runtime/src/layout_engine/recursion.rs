//! One level of a layout recursion, on a stack that cannot run out.
//!
//! Measure and placement recurse once per tree level. Each level first reads
//! how much stack it has left: below [`RED_ZONE`] it carries on in a fresh
//! [`SEGMENT`] (`stacker::grow`), so no tree is too deep for the thread laying
//! it out, whatever the build and whatever that thread's stack. Where the
//! platform cannot tell what is left (`stacker::remaining_stack` is `None`:
//! Android, iOS, wasm), the level runs where it is, as before.
//!
//! The same reading prices a level: the stack its entry has less than its
//! parent's entry had. The pass keeps the largest, per recursion
//! (`LayoutExecutionStats::{measure,placement}_stack_per_level`, published as
//! `WorkCounters::layout_{measure,placement}_stack_per_level`), and the
//! `stack_budget` tests hold it to a budget: the guard keeps a deep tree from
//! crashing, the budget keeps each level from growing back.

use super::PassIntrinsicCache;

/// Stack left below which a level moves to a new segment: room for one level
/// and the non-recursive work it calls (text, intrinsic facts) before the
/// next level checks again.
const RED_ZONE: usize = 256 << 10;
/// What a level that moves gets.
const SEGMENT: usize = 4 << 20;

#[derive(Clone, Copy)]
pub(super) enum Recursion {
    Measure,
    Placement,
}

/// The open levels of one recursion in a pass.
#[derive(Default)]
pub(super) struct LevelStack {
    /// Stack left at each open level's entry. `None` where the platform
    /// cannot tell, and for a level that moved: its children run on another
    /// segment and are not priced against it.
    open: Vec<Option<usize>>,
    /// Most stack one level took this pass, in bytes.
    pub(super) per_level: usize,
}

impl LevelStack {
    fn enter(&mut self, remaining: Option<usize>, moves: bool) {
        if let (Some(Some(parent)), Some(here)) = (self.open.last(), remaining)
            && let Some(level) = parent.checked_sub(here)
        {
            self.per_level = self.per_level.max(level);
        }
        self.open.push(if moves { None } else { remaining });
    }

    fn exit(&mut self) {
        self.open.pop();
    }
}

/// Runs one level of `recursion`: here, or on a new segment when this stack
/// is nearly used up.
pub(super) fn level<T>(
    cache: &mut PassIntrinsicCache,
    recursion: Recursion,
    run: impl FnOnce(&mut PassIntrinsicCache) -> T,
) -> T {
    let remaining = stacker::remaining_stack();
    let moves = remaining.is_some_and(|left| left < RED_ZONE);
    cache.levels(recursion).enter(remaining, moves);
    let out = if moves {
        stacker::grow(SEGMENT, || run(cache))
    } else {
        run(cache)
    };
    cache.levels(recursion).exit();
    out
}
