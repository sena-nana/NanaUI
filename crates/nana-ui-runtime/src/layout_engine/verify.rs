//! The recompute-and-compare guard: after every retained pass, lay the
//! document out again from nothing and require the retained boxes to equal
//! it. Every test that lays out then also tests what the invalidation rules
//! skipped. On in tests and with the `layout-verify` feature.
#![cfg(any(test, feature = "layout-verify"))]

use super::*;

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
    let Some(cache) = retained.documents.get(&document) else {
        return;
    };
    #[cfg(any(test, feature = "benchmark"))]
    let counted = plan_stats::save();
    let full = engine.layout_document(world, document, viewport);
    #[cfg(any(test, feature = "benchmark"))]
    plan_stats::restore(counted);
    let Ok(full) = full else {
        return;
    };
    let mut wrong = Vec::new();
    for (id, expected) in full {
        let kept = cache.boxes.get(&id).copied().unwrap_or_default();
        if !same(kept, expected) {
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
