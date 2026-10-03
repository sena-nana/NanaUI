//! Public contract checks for Issue #198's shared intrinsic cache.
//!
//! These tests deliberately exercise the cache through its public API.  A
//! formatting context may change from flex to grid to inline, but the key and
//! metrics remain owned by the node's content/style identities.

use nana_ui_runtime::{
    Baseline, ConstraintClass, FormattingContextId, IntrinsicCache, IntrinsicCacheBudget,
    IntrinsicCacheKey, IntrinsicMetrics, InvalidationReason, UsedSize,
};

fn metrics() -> IntrinsicMetrics {
    IntrinsicMetrics::new(10.0, 100.0, 4.0, 40.0, UsedSize::new(80.0, 20.0))
        .with_baselines(Some(12.0), Some(18.0))
        .with_aspect_ratio(Some(2.0))
}

fn key(class: ConstraintClass) -> IntrinsicCacheKey {
    IntrinsicCacheKey::new(0x7E57, 0x57_1E, class)
}

#[test]
fn one_entry_is_reused_across_flex_grid_and_inline_contexts() {
    let mut cache = IntrinsicCache::default();
    let key = key(ConstraintClass::Unconstrained);
    cache.insert(key, metrics(), Some(FormattingContextId::new(1))); // flex

    assert_eq!(
        cache.get(&key, Some(FormattingContextId::new(2))),
        Some(metrics())
    ); // grid
    assert_eq!(
        cache.get(&key, Some(FormattingContextId::new(3))),
        Some(metrics())
    ); // inline
    let counters = cache.counters();
    assert_eq!(counters.cross_context_hits, 2);
    assert_eq!(counters.entries, 1);
}

#[test]
fn relevant_constraint_classes_partition_entries_without_context_identity() {
    let mut cache = IntrinsicCache::default();
    let unconstrained = key(ConstraintClass::Unconstrained);
    let max_inline = key(ConstraintClass::MaxInline(320.0));
    let exact_inline = key(ConstraintClass::ExactInline(320.0));
    let percentage = key(ConstraintClass::percentage_cb(320.0, 200.0));
    cache.insert(unconstrained, metrics(), Some(FormattingContextId::new(1)));
    cache.insert(max_inline, metrics(), Some(FormattingContextId::new(1)));
    cache.insert(exact_inline, metrics(), Some(FormattingContextId::new(1)));
    cache.insert(percentage, metrics(), Some(FormattingContextId::new(1)));

    assert!(
        cache
            .get(
                &key(ConstraintClass::MaxInline(320.0)),
                Some(FormattingContextId::new(2))
            )
            .is_some()
    );
    assert!(
        cache
            .get(
                &key(ConstraintClass::ExactInline(320.0)),
                Some(FormattingContextId::new(2))
            )
            .is_some()
    );
    assert!(
        cache
            .get(
                &key(ConstraintClass::percentage_cb(320.0, 200.0)),
                Some(FormattingContextId::new(2))
            )
            .is_some()
    );
    assert_eq!(cache.counters().entries, 4);
}

#[test]
fn fill_and_aspect_dependencies_are_separate_constraint_classes() {
    let mut cache = IntrinsicCache::default();
    let fill = key(ConstraintClass::fill(320.0, 200.0));
    let aspect = key(ConstraintClass::aspect_ratio(320.0, 200.0));
    cache.insert(fill, metrics(), Some(FormattingContextId::new(1)));
    cache.insert(aspect, metrics(), Some(FormattingContextId::new(2)));

    assert!(
        cache
            .get(&fill, Some(FormattingContextId::new(3)))
            .is_some()
    );
    assert!(
        cache
            .get(&aspect, Some(FormattingContextId::new(3)))
            .is_some()
    );
    assert_eq!(cache.counters().entries, 2);
    assert_eq!(
        metrics().resolve(ConstraintClass::aspect_ratio(320.0, 200.0)),
        UsedSize::new(320.0, 200.0)
    );
}

#[test]
fn paint_only_state_keeps_metrics_but_resource_metadata_invalidates_them() {
    let mut cache = IntrinsicCache::default();
    let key = key(ConstraintClass::Unconstrained);
    cache.insert(key, metrics(), None);

    assert!(!cache.invalidate(InvalidationReason::Paint));
    assert_eq!(cache.get(&key, None), Some(metrics()));
    assert!(!cache.invalidate(InvalidationReason::Opacity));
    assert!(!cache.invalidate(InvalidationReason::Transform));
    assert!(!cache.invalidate(InvalidationReason::Hover));
    assert!(!cache.invalidate(InvalidationReason::Accessibility));
    assert_eq!(cache.get(&key, None), Some(metrics()));

    assert!(cache.invalidate(InvalidationReason::ResourceMetadata));
    assert!(cache.get(&key, None).is_none());
}

#[test]
fn baselines_are_shared_metrics_queries_and_used_size_does_not_mutate_them() {
    let mut cache = IntrinsicCache::default();
    let key = key(ConstraintClass::ExactSize {
        inline: 200.0,
        block: 80.0,
    });
    let original = metrics();
    cache.insert(key, original, Some(FormattingContextId::new(7)));

    assert_eq!(
        cache.baseline(&key, Some(FormattingContextId::new(8)), Baseline::First),
        Some(12.0)
    );
    assert_eq!(
        cache.baseline(&key, Some(FormattingContextId::new(8)), Baseline::Last),
        Some(18.0)
    );
    let used = original.resolve(ConstraintClass::ExactSize {
        inline: 200.0,
        block: 80.0,
    });
    assert_eq!(used, UsedSize::new(200.0, 80.0));
    assert_eq!(cache.get(&key, None), Some(original));
    assert_eq!(cache.counters().baseline_queries, 2);
}

#[test]
fn cache_budget_is_bounded_for_context_transitions() {
    let mut cache = IntrinsicCache::new(IntrinsicCacheBudget {
        max_entries: 2,
        max_bytes: usize::MAX,
    });
    for content in 0..3 {
        let key = IntrinsicCacheKey::new(content, 1, ConstraintClass::Unconstrained);
        cache.insert(key, metrics(), Some(FormattingContextId::new(content)));
    }
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.counters().evictions, 1);
}

#[test]
fn node_invalidation_does_not_flush_unrelated_intrinsic_facts() {
    let mut cache = IntrinsicCache::default();
    let first = IntrinsicCacheKey::new(1, 1, ConstraintClass::Unconstrained);
    let second = IntrinsicCacheKey::new(2, 1, ConstraintClass::Unconstrained);
    cache.insert(first, metrics(), None);
    cache.insert(second, metrics(), None);
    assert!(cache.invalidate_node(1, InvalidationReason::Content));
    assert!(cache.get(&first, None).is_none());
    assert!(cache.get(&second, None).is_some());
    assert!(!cache.invalidate_node(2, InvalidationReason::Paint));
}

#[test]
fn an_unbounded_block_maximum_is_explicit() {
    let facts = IntrinsicMetrics::new(1.0, 10.0, 2.0, None, UsedSize::new(8.0, 64.0));
    assert_eq!(facts.max_block, None);
    assert_eq!(facts.preferred_size(), UsedSize::new(8.0, 64.0));
}
