//! The recompute-and-compare guard: after every retained pass, lay the
//! document out again from nothing and require the retained boxes to equal
//! it. Every test that lays out then also tests what the invalidation rules
//! skipped. On in tests and with the `layout-verify` feature.
#![cfg(any(test, feature = "layout-verify"))]

use std::cell::Cell;

use super::*;

thread_local! {
    static SKIPPED: Cell<bool> = const { Cell::new(false) };
    static REFERENCE: Cell<bool> = const { Cell::new(false) };
    static KEPT: std::cell::RefCell<Vec<StableNodeId>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// `id` only moved and kept its subtree this pass
/// (`placement::keeps_moved_subtree`): the guard requires every box under
/// it to be the full layout's to the bit, not just within its tolerance.
pub(super) fn note_kept_subtree(id: StableNodeId) {
    if !REFERENCE.with(Cell::get) {
        KEPT.with(|kept| kept.borrow_mut().push(id));
    }
}

/// Whether measurement may drop the offered block extent from the key of a
/// subtree that reads none of it. The guard's reference layout keys by the
/// real extent, so a read `reads_offered_block_extent` misses shows up as a
/// box that differs.
pub(super) fn honors_block_reads() -> bool {
    !REFERENCE.with(Cell::get)
}

/// Turns the guard off on this thread until the returned value drops. For a
/// soak whose frame count makes a full layout per frame unaffordable; such a
/// test compares against a cold layout itself.
#[cfg(test)]
pub(crate) fn skip_layout_verify() -> SkipLayoutVerify {
    SKIPPED.with(|skipped| skipped.set(true));
    SkipLayoutVerify(())
}

#[cfg(test)]
pub(crate) struct SkipLayoutVerify(());

#[cfg(test)]
impl Drop for SkipLayoutVerify {
    fn drop(&mut self) {
        SKIPPED.with(|skipped| skipped.set(false));
    }
}

fn same(a: LayoutBox, b: LayoutBox) -> bool {
    const EPSILON: f32 = 0.01;
    // An omitted box has no meaningful origin; only its zero extent.
    if a.width == 0.0 && a.height == 0.0 && b.width == 0.0 && b.height == 0.0 {
        return true;
    }
    (a.x - b.x).abs() <= EPSILON
        && (a.y - b.y).abs() <= EPSILON
        && (a.width - b.width).abs() <= EPSILON
        && (a.height - b.height).abs() <= EPSILON
}

pub(super) fn retained_matches_full_layout(
    engine: RuntimeLayoutEngine,
    world: &UiWorld,
    document: DocumentId,
    viewport: LayoutViewport,
    retained: &RetainedLayoutCache,
    seeds: &[LayoutFrontierSeed],
    force_full: bool,
) {
    let kept_subtrees = KEPT.with(|kept| std::mem::take(&mut *kept.borrow_mut()));
    if SKIPPED.with(Cell::get) {
        return;
    }
    let Some(cache) = retained.documents.get(&document) else {
        return;
    };
    // The full layout runs on its own pass cache: it counts nothing on the
    // retained document's execution stats, nor on a test's measure trace.
    world.check_block_extent_reads();
    #[cfg(test)]
    let _paused = super::measure_trace::pause();
    REFERENCE.with(|reference| reference.set(true));
    let full = engine.layout_document_local(world, document, viewport);
    REFERENCE.with(|reference| reference.set(false));
    let Ok(full) = full else {
        return;
    };
    // Under a box that kept its subtree, every box is the one the body would
    // have written: the full layout's, bit for bit.
    let mut exact = crate::NodeSet::default();
    let mut stack = kept_subtrees;
    while let Some(id) = stack.pop() {
        if exact.insert(id) {
            stack.extend(world.child_ids(id).iter().copied());
        }
    }
    let bits = |b: LayoutBox| {
        [
            b.x.to_bits(),
            b.y.to_bits(),
            b.width.to_bits(),
            b.height.to_bits(),
        ]
    };
    let mut wrong = Vec::new();
    for (id, expected, frame) in full {
        let omitted = expected.width == 0.0 && expected.height == 0.0;
        if exact.contains(&id)
            && !omitted
            && cache
                .boxes
                .get(&id)
                .is_none_or(|kept| bits(*kept) != bits(expected))
        {
            let kept = cache.boxes.get(&id).copied().unwrap_or_default();
            panic!(
                "{id:?} under a moved subtree kept {kept:?}, full layout {expected:?}: \
                 not the same to the bit"
            );
        }
        let kept = cache.boxes.get(&id).copied().unwrap_or_default();
        let kept_frame = cache.frames.get(&id).copied().unwrap_or_default();
        // A box with no extent has no frame that matters either.
        if !same(kept, expected) || (kept_frame != frame && !omitted) {
            wrong.push((id, kept, expected));
        }
    }
    if wrong.is_empty() {
        return;
    }
    let shown: Vec<String> = wrong
        .iter()
        .take(8)
        .map(|(id, kept, expected)| {
            let chain: Vec<_> =
                std::iter::successors(world.parent_id(*id), |id| world.parent_id(*id)).collect();
            format!("{id:?} kept {kept:?} full {expected:?} under {chain:?}")
        })
        .collect();
    panic!(
        "retained layout (force_full {force_full}) differs from a full layout at {} \
         nodes:\n{}\nseeds: {seeds:?}",
        wrong.len(),
        shown.join("\n")
    );
}

/// Every line-box answer `document`'s cache keeps equals a fresh walk's: the
/// recompute-and-compare guard for which seeds drop them
/// ([`super::may_move_line_boxes`]). Reading an answer checks it too
/// (`checked_line_boxes`), but only the ones a pass asks; this checks the
/// ones a later pass would read. One fresh memo serves every entry, so the
/// audit walks each box once. [`skip_layout_verify`] skips this full-document
/// walk as well so timing and 100k-node benchmarks keep O(edit) cost.
pub(super) fn retained_line_boxes_are_fresh(
    world: &UiWorld,
    document: DocumentId,
    retained: &RetainedLayoutCache,
) {
    if SKIPPED.with(Cell::get) {
        return;
    }
    let Some(cache) = retained.documents.get(&document) else {
        return;
    };
    // A box out of the document (detached, parked) is asked nothing until it
    // comes back, and it comes back with a structural seed that drops what
    // is kept below it.
    let mounted: crate::NodeSet = world.document_order(document).into_iter().collect();
    let mut fresh = crate::NodeMap::default();
    for (&id, &kept) in cache.line_boxes.iter() {
        if !mounted.contains(&id) || !world.presence_live(id) {
            continue;
        }
        assert_eq!(
            kept,
            world.holds_line_box_in(id, &mut fresh),
            "{id:?}: the retained line-box answer is stale after the pass"
        );
    }
}
