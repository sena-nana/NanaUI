//! Page origins remembered for one read-only pass.
//!
//! A box's page origin is its frames' local origins summed from the top down
//! ([`UiWorld::record_layout_origin`]), O(depth). Extraction and the
//! accessibility projection read it for every node they touch, so a subtree
//! mounted deep in the page paid its depth once per node: a chain of `n`
//! boxes cost `n²` additions. While a pass holds a [`PageOriginScope`], a
//! deep box's walk stops at the first frame it already summed and adds its
//! own locals onto that: summing from the top down, the frame's origin is
//! the running sum at that frame, so the result is the bit a full walk gives.
//! The scope borrows the world, so nothing writes a box while it is open.

use super::*;

/// Chains no deeper than this are walked directly: a lookup costs more than
/// the steps it saves. The other walks up a deep chain that remember what
/// they passed (the hit index's shifts and pointer events, accessibility's
/// chain answers) use it too.
pub(crate) const DIRECT_DEPTH: u16 = 32;

#[derive(Debug, Default)]
pub(super) struct PageOrigins {
    /// Open scopes: the memo holds while this is not zero.
    open: u32,
    /// Each frame summed in a deep walk: its page origin.
    origins: NodeMap<[f32; 2]>,
    /// The walk's frames and their locals, from the box up; reused.
    chain: Vec<(Option<StableNodeId>, [f32; 2])>,
}

/// Keeps [`UiWorld`]'s page origins remembered until dropped.
pub(crate) struct PageOriginScope<'a> {
    world: &'a UiWorld,
}

impl Drop for PageOriginScope<'_> {
    fn drop(&mut self) {
        let mut origins = self.world.page_origins.borrow_mut();
        origins.open -= 1;
        if origins.open == 0 {
            origins.origins.clear();
        }
    }
}

impl UiWorld {
    /// Remember page origins until the scope is dropped. Nothing can write
    /// a box meanwhile: the scope borrows the world.
    pub(crate) fn remember_page_origins(&self) -> PageOriginScope<'_> {
        self.page_origins.borrow_mut().open += 1;
        PageOriginScope { world: self }
    }

    /// `record`'s page origin through the remembered frames, when a scope is
    /// open, no component geometry is being derived and the box is deep.
    pub(super) fn remembered_page_origin(&self, record: &NodeRecord) -> Option<[f32; 2]> {
        if record.depth <= DIRECT_DEPTH || self.geometry_frame.get().is_some() {
            return None;
        }
        let mut memo = self.page_origins.try_borrow_mut().ok()?;
        if memo.open == 0 {
            return None;
        }
        let PageOrigins { origins, chain, .. } = &mut *memo;
        chain.clear();
        let mut base = None;
        let mut id = None;
        let mut cursor = Some(record);
        while let Some(node) = cursor {
            #[cfg(test)]
            steps::step(steps::Walk::PageOrigin);
            if let Some(origin) = id.and_then(|id| origins.get(&id)) {
                base = Some(*origin);
                break;
            }
            if matches!(node.kind.as_ref(), NodeKind::Document) {
                break;
            }
            chain.push((id, [node.local_layout.x, node.local_layout.y]));
            id = self.frame_node(node);
            cursor = id.and_then(|frame| self.nodes.get(frame));
        }
        // The same additions in the same order as the full walk: from the
        // top local (or the remembered sum down to a frame) down to the box.
        let mut rest = chain.iter().rev();
        let mut origin = match base {
            Some(origin) => origin,
            None => match rest.next() {
                Some(&(top, local)) => {
                    if let Some(top) = top {
                        origins.insert(top, local);
                    }
                    local
                }
                None => return Some([0.0, 0.0]),
            },
        };
        for &(frame, [dx, dy]) in rest {
            origin = [origin[0] + dx, origin[1] + dy];
            if let Some(frame) = frame {
                origins.insert(frame, origin);
            }
        }
        Some(origin)
    }
}

/// The steps each walk up a chain took, for the tests that bound them.
#[cfg(test)]
pub(crate) mod steps {
    use std::cell::Cell;

    /// How many walks there are.
    pub(crate) const WALKS: usize = 7;

    #[derive(Clone, Copy)]
    pub(crate) enum Walk {
        /// A page origin summed ([`super::UiWorld::record_layout_origin`]).
        PageOrigin,
        /// The hit index's scroll shift above an entry.
        HitShift,
        /// The hit index's depth of an entry whose bounds are refreshed.
        HitDepth,
        /// The pointer events a parent uses.
        PointerEvents,
        /// Layout's question whether an affected node is inside an island.
        Islands,
        /// Accessibility's answers for a record and every one above it.
        Records,
        /// An affected child a container's retained plan re-checks or
        /// patches (`layout_engine::ScopeContext::affected_children`).
        PlanChildren,
    }

    thread_local! {
        static STEPS: Cell<[u64; WALKS]> = const { Cell::new([0; WALKS]) };
    }

    pub(crate) fn step(walk: Walk) {
        STEPS.with(|steps| {
            let mut all = steps.get();
            all[walk as usize] += 1;
            steps.set(all);
        });
    }

    /// The steps each walk took since the last call, by [`Walk`], and
    /// start again.
    pub(crate) fn take_all() -> [u64; WALKS] {
        STEPS.with(|steps| steps.replace([0; WALKS]))
    }

    /// `check` without counting its steps: a check walks in full.
    pub(crate) fn uncounted<R>(check: impl FnOnce() -> R) -> R {
        let held = STEPS.with(Cell::get);
        let result = check();
        STEPS.with(|steps| steps.set(held));
        result
    }
}
