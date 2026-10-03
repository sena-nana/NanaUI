//! Generation-aware intrinsic measurement cache shared by formatting contexts.
//!
//! Intrinsic metrics describe a content subtree.  They are deliberately kept
//! separate from [`UsedSize`], which is the result of resolving one particular
//! containing block.  The cache key therefore carries only the content/style
//! identity and the *class* of constraints; a formatting-context identity is
//! never part of the key.  A flex, grid, block, or inline context can answer
//! from the same entry when they ask the same question.
//!
//! This module is intentionally independent from the retained box cache.  A
//! retained box is a placement result, while an intrinsic entry can safely be
//! shared by contexts and documents as long as the producer gives content and
//! style identities that have the same lifetime.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// A finite size in the intrinsic/used-size contract.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UsedSize {
    pub inline: f32,
    pub block: f32,
}

impl UsedSize {
    pub const ZERO: Self = Self {
        inline: 0.0,
        block: 0.0,
    };

    #[must_use]
    pub fn new(inline: f32, block: f32) -> Self {
        Self {
            inline: nonnegative(inline),
            block: nonnegative(block),
        }
    }

    #[must_use]
    pub fn physical(self) -> (f32, f32) {
        (self.inline, self.block)
    }

    #[must_use]
    pub fn finite(self) -> Self {
        Self {
            inline: nonnegative(self.inline),
            block: nonnegative(self.block),
        }
    }
}

/// Intrinsic contribution of a subtree, before a containing block resolves a
/// used size.  Baselines are in the subtree's block-axis coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntrinsicMetrics {
    pub min_inline: f32,
    pub max_inline: f32,
    pub min_block: f32,
    pub max_block: Option<f32>,
    pub preferred: UsedSize,
    pub first_baseline: Option<f32>,
    pub last_baseline: Option<f32>,
    pub aspect_ratio: Option<f32>,
    /// Whether the content can be shortened when an exact/max constraint is
    /// applied (for example, an ellipsized label).
    pub truncatable: bool,
    /// Generation of the content/style inputs used to produce this result.
    pub generation: u64,
}

/// Constructor input accepted for the optional block maximum.  Keeping the
/// `f32` implementation preserves the ergonomic form used by existing
/// callers, while `Option<f32>` expresses an unbounded block axis directly.
pub trait MaxBlockInput {
    fn into_max_block(self, min_block: f32) -> Option<f32>;
}

impl MaxBlockInput for f32 {
    fn into_max_block(self, min_block: f32) -> Option<f32> {
        Some(nonnegative(self).max(nonnegative(min_block)))
    }
}

impl MaxBlockInput for Option<f32> {
    fn into_max_block(self, min_block: f32) -> Option<f32> {
        self.map(|value| nonnegative(value).max(nonnegative(min_block)))
    }
}

impl Default for IntrinsicMetrics {
    fn default() -> Self {
        Self {
            min_inline: 0.0,
            max_inline: 0.0,
            min_block: 0.0,
            max_block: None,
            preferred: UsedSize::ZERO,
            first_baseline: None,
            last_baseline: None,
            aspect_ratio: None,
            truncatable: false,
            generation: 0,
        }
    }
}

impl IntrinsicMetrics {
    #[must_use]
    pub fn new<M: MaxBlockInput>(
        min_inline: f32,
        max_inline: f32,
        min_block: f32,
        max_block: M,
        preferred_size: UsedSize,
    ) -> Self {
        let min_block = nonnegative(min_block);
        Self {
            min_inline: nonnegative(min_inline),
            max_inline: nonnegative(max_inline).max(nonnegative(min_inline)),
            min_block,
            max_block: max_block.into_max_block(min_block),
            preferred: preferred_size.finite(),
            ..Self::default()
        }
        .normalized()
    }

    #[must_use]
    pub fn with_baselines(mut self, first: Option<f32>, last: Option<f32>) -> Self {
        self.first_baseline = first.filter(|value| value.is_finite() && *value >= 0.0);
        self.last_baseline = last.filter(|value| value.is_finite() && *value >= 0.0);
        self
    }

    #[must_use]
    pub fn with_aspect_ratio(mut self, ratio: Option<f32>) -> Self {
        self.aspect_ratio = ratio.filter(|ratio| ratio.is_finite() && *ratio > 0.0);
        self
    }

    #[must_use]
    pub fn with_truncatable(mut self, truncatable: bool) -> Self {
        self.truncatable = truncatable;
        self
    }

    #[must_use]
    pub fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.min_inline = nonnegative(self.min_inline);
        self.max_inline = nonnegative(self.max_inline).max(self.min_inline);
        self.min_block = nonnegative(self.min_block);
        self.max_block = self
            .max_block
            .map(|value| nonnegative(value).max(self.min_block));
        self.preferred = self.preferred.finite();
        self.first_baseline = self
            .first_baseline
            .filter(|value| value.is_finite() && *value >= 0.0);
        self.last_baseline = self
            .last_baseline
            .filter(|value| value.is_finite() && *value >= 0.0);
        self.aspect_ratio = self
            .aspect_ratio
            .filter(|value| value.is_finite() && *value > 0.0);
        self.preferred.inline = clamp_inline(self.preferred.inline, self);
        self.preferred.block = clamp_block(self.preferred.block, self);
        self
    }

    /// Set an unbounded block contribution.  CSS `max-block: none` is
    /// represented explicitly rather than as a sentinel float.
    #[must_use]
    pub fn with_max_block(mut self, max_block: Option<f32>) -> Self {
        self.max_block = max_block.map(|value| nonnegative(value).max(self.min_block));
        self.normalized()
    }

    /// Compatibility accessor for callers that use the longer spelling.
    #[must_use]
    pub const fn preferred_size(self) -> UsedSize {
        self.preferred
    }

    /// Resolve an intrinsic result into a used size for a constraint class.
    /// This helper does not mutate the cache and is suitable for all
    /// formatting contexts.
    #[must_use]
    pub fn resolve(self, constraints: ConstraintClass) -> UsedSize {
        match constraints.normalized() {
            ConstraintClass::Unconstrained => self.preferred,
            ConstraintClass::MaxInline(inline) => UsedSize::new(
                clamp_inline(self.preferred.inline.min(inline.max(0.0)), self),
                clamp_block(self.preferred.block, self),
            ),
            ConstraintClass::ExactInline(inline) => UsedSize::new(
                clamp_inline(inline, self),
                clamp_block(self.preferred.block, self),
            ),
            ConstraintClass::MaxBlock(block) => UsedSize::new(
                clamp_inline(self.preferred.inline, self),
                clamp_block(self.preferred.block.min(block.max(0.0)), self),
            ),
            ConstraintClass::ExactSize { inline, block }
            | ConstraintClass::PercentageContainingBlock { inline, block }
            | ConstraintClass::PercentageCb { inline, block }
            | ConstraintClass::PercentageCB { inline, block } => {
                // Exact/containing-block constraints are the parent's used
                // size decision.  They may expand or compress the preferred
                // intrinsic size; writing them back as clamped metrics would
                // conflate UsedSize with the child's intrinsic truth.
                UsedSize::new(inline, block)
            }
            ConstraintClass::Fill { inline, block }
            | ConstraintClass::AspectRatio { inline, block } => UsedSize::new(inline, block),
        }
    }
}

fn clamp_inline(value: f32, metrics: IntrinsicMetrics) -> f32 {
    value.max(metrics.min_inline).min(metrics.max_inline)
}

fn clamp_block(value: f32, metrics: IntrinsicMetrics) -> f32 {
    let value = value.max(metrics.min_block);
    metrics.max_block.map_or(value, |max| value.min(max))
}

/// Constraint *shape* used in an intrinsic query.  Numeric values are
/// canonicalized for equality and hashing, so `-0.0` and `0.0` do not create
/// distinct entries and NaN is treated as an unconstrained value.
#[derive(Debug, Clone, Copy)]
pub enum ConstraintClass {
    Unconstrained,
    MaxInline(f32),
    ExactInline(f32),
    MaxBlock(f32),
    ExactSize {
        inline: f32,
        block: f32,
    },
    /// A size supplied by a percentage containing block.  It is a distinct
    /// class because a percentage basis can become definite later.
    PercentageContainingBlock {
        inline: f32,
        block: f32,
    },
    /// Spelling retained for callers that use the CSS abbreviation.
    PercentageCb {
        inline: f32,
        block: f32,
    },
    /// Upper-case abbreviation retained as an API convenience.
    PercentageCB {
        inline: f32,
        block: f32,
    },
    /// A fill/stretch dependency selected by the parent.
    Fill {
        inline: f32,
        block: f32,
    },
    /// An aspect-ratio transfer dependency. The ratio itself is carried by
    /// [`IntrinsicMetrics::aspect_ratio`].
    AspectRatio {
        inline: f32,
        block: f32,
    },
}

impl ConstraintClass {
    #[must_use]
    pub fn normalized(self) -> Self {
        let finite_extent = |value: f32| {
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            }
        };
        match self {
            Self::Unconstrained => Self::Unconstrained,
            Self::MaxInline(value) if !value.is_finite() => Self::Unconstrained,
            Self::ExactInline(value) if !value.is_finite() => Self::Unconstrained,
            Self::MaxBlock(value) if !value.is_finite() => Self::Unconstrained,
            Self::MaxInline(value) => Self::MaxInline(finite_extent(value)),
            Self::ExactInline(value) => Self::ExactInline(finite_extent(value)),
            Self::MaxBlock(value) => Self::MaxBlock(finite_extent(value)),
            Self::ExactSize { inline, block } if inline.is_finite() && block.is_finite() => {
                Self::ExactSize {
                    inline: finite_extent(inline),
                    block: finite_extent(block),
                }
            }
            Self::ExactSize { .. } => Self::Unconstrained,
            Self::PercentageContainingBlock { inline, block }
            | Self::PercentageCb { inline, block }
            | Self::PercentageCB { inline, block }
                if inline.is_finite() && block.is_finite() =>
            {
                Self::PercentageContainingBlock {
                    inline: finite_extent(inline),
                    block: finite_extent(block),
                }
            }
            Self::PercentageContainingBlock { .. }
            | Self::PercentageCb { .. }
            | Self::PercentageCB { .. } => Self::Unconstrained,
            Self::Fill { inline, block } if inline.is_finite() && block.is_finite() => Self::Fill {
                inline: finite_extent(inline),
                block: finite_extent(block),
            },
            Self::Fill { .. } => Self::Unconstrained,
            Self::AspectRatio { inline, block } if inline.is_finite() && block.is_finite() => {
                Self::AspectRatio {
                    inline: finite_extent(inline),
                    block: finite_extent(block),
                }
            }
            Self::AspectRatio { .. } => Self::Unconstrained,
        }
    }

    #[must_use]
    pub fn percentage_cb(inline: f32, block: f32) -> Self {
        Self::PercentageContainingBlock { inline, block }
    }

    #[must_use]
    pub fn exact_size(inline: f32, block: f32) -> Self {
        Self::ExactSize { inline, block }
    }

    #[must_use]
    pub fn fill(inline: f32, block: f32) -> Self {
        Self::Fill { inline, block }
    }

    #[must_use]
    pub fn aspect_ratio(inline: f32, block: f32) -> Self {
        Self::AspectRatio { inline, block }
    }

    #[must_use]
    pub fn is_unconstrained(self) -> bool {
        matches!(self.normalized(), Self::Unconstrained)
    }

    fn tag(self) -> u8 {
        match self {
            Self::Unconstrained => 0,
            Self::MaxInline(_) => 1,
            Self::ExactInline(_) => 2,
            Self::MaxBlock(_) => 3,
            Self::ExactSize { .. } => 4,
            Self::PercentageContainingBlock { .. }
            | Self::PercentageCb { .. }
            | Self::PercentageCB { .. } => 5,
            Self::Fill { .. } => 6,
            Self::AspectRatio { .. } => 7,
        }
    }

    fn values(self) -> (u32, u32) {
        let canonical = |value: f32| canonical_f32_bits(value);
        match self {
            Self::Unconstrained | Self::MaxInline(_) | Self::ExactInline(_) | Self::MaxBlock(_) => {
                let value = match self {
                    Self::MaxInline(value) | Self::ExactInline(value) | Self::MaxBlock(value) => {
                        value
                    }
                    _ => 0.0,
                };
                (canonical(value), 0)
            }
            Self::ExactSize { inline, block }
            | Self::PercentageContainingBlock { inline, block }
            | Self::PercentageCb { inline, block }
            | Self::PercentageCB { inline, block }
            | Self::Fill { inline, block }
            | Self::AspectRatio { inline, block } => (canonical(inline), canonical(block)),
        }
    }
}

impl PartialEq for ConstraintClass {
    fn eq(&self, other: &Self) -> bool {
        let left = self.normalized();
        let right = other.normalized();
        left.tag() == right.tag() && left.values() == right.values()
    }
}
impl Eq for ConstraintClass {}
impl Hash for ConstraintClass {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let normalized = self.normalized();
        normalized.tag().hash(state);
        normalized.values().hash(state);
    }
}

/// Stable identity of intrinsic inputs.  `context` is intentionally absent:
/// this key is valid across formatting contexts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntrinsicCacheKey {
    pub content: u64,
    pub style: u64,
    pub constraints: ConstraintClass,
}

impl IntrinsicCacheKey {
    #[must_use]
    pub fn new(content: u64, style: u64, constraints: ConstraintClass) -> Self {
        Self {
            content,
            style,
            constraints: constraints.normalized(),
        }
    }
}

/// A formatting-context identity used only for diagnostics.  It is not part
/// of [`IntrinsicCacheKey`], so using another context can still hit the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FormattingContextId(pub u64);

impl FormattingContextId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

/// Reasons that can change intrinsic geometry.  Paint-only state intentionally
/// does not bump the generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvalidationReason {
    Content,
    Intrinsic,
    ShapeStyle,
    PaddingBorder,
    MinMax,
    ResourceMetadata,
    ScaleFont,
    ChildMetrics,
    Paint,
    Opacity,
    Transform,
    Hover,
    Accessibility,
}

impl InvalidationReason {
    #[must_use]
    pub const fn affects_intrinsic(self) -> bool {
        matches!(
            self,
            Self::Content
                | Self::Intrinsic
                | Self::ShapeStyle
                | Self::PaddingBorder
                | Self::MinMax
                | Self::ResourceMetadata
                | Self::ScaleFont
                | Self::ChildMetrics
        )
    }
}

/// Monotonic generation for intrinsic inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IntrinsicGeneration {
    value: u64,
}

impl IntrinsicGeneration {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self { value }
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.value
    }

    /// Bump only for a geometry-affecting reason.  Returns whether the value
    /// changed.
    pub fn invalidate(&mut self, reason: InvalidationReason) -> bool {
        if !reason.affects_intrinsic() {
            return false;
        }
        let next = self.value.saturating_add(1).max(1);
        let changed = next != self.value;
        self.value = next;
        changed
    }
}

/// Bounded cache size.  Both limits are enforced so one huge subtree cannot
/// crowd out every small entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntrinsicCacheBudget {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for IntrinsicCacheBudget {
    fn default() -> Self {
        Self {
            max_entries: 4096,
            max_bytes: 4 * 1024 * 1024,
        }
    }
}

/// Observable work counters for intrinsic measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IntrinsicCacheCounters {
    pub intrinsic_measure_requests: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub full_subtrees: usize,
    pub intrinsic_measure_cache_hits: usize,
    pub intrinsic_measure_cache_misses: usize,
    pub intrinsic_measure_full_subtrees: usize,
    pub generation_bumps: usize,
    pub baseline_queries: usize,
    pub cross_context_hits: usize,
    pub cross_context_misses: usize,
    pub evictions: usize,
    pub entries: usize,
    pub bytes: usize,
}

impl IntrinsicCacheCounters {
    pub fn accumulate(&mut self, other: Self) {
        self.intrinsic_measure_requests = self
            .intrinsic_measure_requests
            .saturating_add(other.intrinsic_measure_requests);
        self.cache_hits = self.cache_hits.saturating_add(other.cache_hits);
        self.cache_misses = self.cache_misses.saturating_add(other.cache_misses);
        self.full_subtrees = self.full_subtrees.saturating_add(other.full_subtrees);
        // `cache_hits`/`cache_misses`/`full_subtrees` are compatibility names
        // used by the per-pass layout adapter.  New producers fill the
        // explicitly prefixed fields.  Prefer the canonical value when both
        // are present so folding does not count one lookup twice.
        self.intrinsic_measure_cache_hits =
            self.intrinsic_measure_cache_hits
                .saturating_add(prefer_canonical(
                    other.intrinsic_measure_cache_hits,
                    other.cache_hits,
                ));
        self.intrinsic_measure_cache_misses =
            self.intrinsic_measure_cache_misses
                .saturating_add(prefer_canonical(
                    other.intrinsic_measure_cache_misses,
                    other.cache_misses,
                ));
        self.intrinsic_measure_full_subtrees =
            self.intrinsic_measure_full_subtrees
                .saturating_add(prefer_canonical(
                    other.intrinsic_measure_full_subtrees,
                    other.full_subtrees,
                ));
        self.generation_bumps = self.generation_bumps.saturating_add(other.generation_bumps);
        self.baseline_queries = self.baseline_queries.saturating_add(other.baseline_queries);
        self.cross_context_hits = self
            .cross_context_hits
            .saturating_add(other.cross_context_hits);
        self.cross_context_misses = self
            .cross_context_misses
            .saturating_add(other.cross_context_misses);
        self.evictions = self.evictions.saturating_add(other.evictions);
        self.entries = other.entries;
        self.bytes = other.bytes;
    }
}

struct Entry {
    metrics: IntrinsicMetrics,
    global_generation: u64,
    node_generation: u64,
    context: Option<FormattingContextId>,
    bytes: usize,
    previous: Option<IntrinsicCacheKey>,
    next: Option<IntrinsicCacheKey>,
}

/// LRU intrinsic cache.  `get` accepts an optional context solely to report
/// cross-context reuse; omitting it is useful for hosts without context IDs.
pub struct IntrinsicCache {
    entries: HashMap<IntrinsicCacheKey, Entry>,
    /// Intrusive LRU links keep lookup and recency updates O(1).
    head: Option<IntrinsicCacheKey>,
    tail: Option<IntrinsicCacheKey>,
    budget: IntrinsicCacheBudget,
    generation: IntrinsicGeneration,
    node_generations: HashMap<u64, u64>,
    bytes: usize,
    counters: IntrinsicCacheCounters,
}

impl std::fmt::Debug for IntrinsicCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntrinsicCache")
            .field("entries", &self.entries.len())
            .field("bytes", &self.bytes())
            .field("generation", &self.generation.get())
            .finish()
    }
}

impl Default for IntrinsicCache {
    fn default() -> Self {
        Self::new(IntrinsicCacheBudget::default())
    }
}

impl IntrinsicCache {
    #[must_use]
    pub fn new(budget: IntrinsicCacheBudget) -> Self {
        Self {
            entries: HashMap::new(),
            head: None,
            tail: None,
            budget,
            generation: IntrinsicGeneration::default(),
            node_generations: HashMap::new(),
            bytes: 0,
            counters: IntrinsicCacheCounters::default(),
        }
    }

    #[must_use]
    pub fn generation(&self) -> IntrinsicGeneration {
        self.generation
    }

    #[must_use]
    pub fn node_generation(&self, node: u64) -> u64 {
        self.node_generations.get(&node).copied().unwrap_or(0)
    }

    /// Invalidates all entries and releases stale storage immediately. This
    /// keeps byte/entry gauges honest and prevents an old generation from
    /// consuming the new budget.
    pub fn invalidate(&mut self, reason: InvalidationReason) -> bool {
        let changed = self.generation.invalidate(reason);
        if changed {
            self.counters.generation_bumps = self.counters.generation_bumps.saturating_add(1);
            self.entries.clear();
            self.head = None;
            self.tail = None;
            self.bytes = 0;
        }
        changed
    }

    /// Invalidate one node's intrinsic facts without flushing siblings.  This
    /// is the normal path for child/content edits; a parent can then decide
    /// whether its own metrics need recomputation.  Paint-only reasons are a
    /// no-op by contract.
    pub fn invalidate_node(&mut self, node: u64, reason: InvalidationReason) -> bool {
        if !reason.affects_intrinsic() {
            return false;
        }
        let generation = self.node_generations.entry(node).or_default();
        let next_generation = generation.saturating_add(1).max(1);
        if next_generation == *generation {
            return false;
        }
        *generation = next_generation;
        let keys: Vec<_> = self
            .entries
            .keys()
            .copied()
            .filter(|key| key.content == node)
            .collect();
        for key in keys {
            self.remove_entry(&key);
        }
        self.counters.generation_bumps = self.counters.generation_bumps.saturating_add(1);
        true
    }

    /// Marks that a full-subtree walk was required by the caller.
    pub fn record_full_subtree(&mut self) {
        self.counters.full_subtrees = self.counters.full_subtrees.saturating_add(1);
        self.counters.intrinsic_measure_full_subtrees = self
            .counters
            .intrinsic_measure_full_subtrees
            .saturating_add(1);
    }

    /// Looks up metrics under the current generation.
    pub fn get(
        &mut self,
        key: &IntrinsicCacheKey,
        context: Option<FormattingContextId>,
    ) -> Option<IntrinsicMetrics> {
        self.lookup_with_accounting(key, context, true)
    }

    fn lookup_with_accounting(
        &mut self,
        key: &IntrinsicCacheKey,
        context: Option<FormattingContextId>,
        account_measure: bool,
    ) -> Option<IntrinsicMetrics> {
        // Callers may construct the public key struct directly instead of
        // using `IntrinsicCacheKey::new`; normalize here as well so malformed
        // float constraints cannot create an unreachable duplicate entry.
        let key = IntrinsicCacheKey {
            constraints: key.constraints.normalized(),
            ..*key
        };
        if account_measure {
            self.counters.intrinsic_measure_requests =
                self.counters.intrinsic_measure_requests.saturating_add(1);
        }
        let Some(entry) = self.entries.get(&key) else {
            if account_measure {
                self.counters.cache_misses = self.counters.cache_misses.saturating_add(1);
                self.counters.intrinsic_measure_cache_misses = self
                    .counters
                    .intrinsic_measure_cache_misses
                    .saturating_add(1);
                if context.is_some() {
                    self.counters.cross_context_misses =
                        self.counters.cross_context_misses.saturating_add(1);
                }
            }
            return None;
        };
        if entry.global_generation != self.generation.get()
            || entry.node_generation != self.node_generation(key.content)
        {
            if account_measure {
                self.counters.cache_misses = self.counters.cache_misses.saturating_add(1);
                self.counters.intrinsic_measure_cache_misses = self
                    .counters
                    .intrinsic_measure_cache_misses
                    .saturating_add(1);
                if context.is_some() {
                    self.counters.cross_context_misses =
                        self.counters.cross_context_misses.saturating_add(1);
                }
            }
            return None;
        }
        let cross = context.is_some() && entry.context.is_some() && context != entry.context;
        let metrics = entry.metrics;
        self.touch(&key);
        if account_measure {
            self.counters.cache_hits = self.counters.cache_hits.saturating_add(1);
            self.counters.intrinsic_measure_cache_hits =
                self.counters.intrinsic_measure_cache_hits.saturating_add(1);
            if cross {
                self.counters.cross_context_hits =
                    self.counters.cross_context_hits.saturating_add(1);
            }
        }
        Some(metrics)
    }

    /// Alias useful at call sites that want to make the intrinsic query
    /// explicit.
    pub fn lookup(
        &mut self,
        key: &IntrinsicCacheKey,
        context: Option<FormattingContextId>,
    ) -> Option<IntrinsicMetrics> {
        self.get(key, context)
    }

    /// Shared baseline query.  It counts separately while reusing the same
    /// generation-checked entry as ordinary intrinsic metrics.
    pub fn baseline(
        &mut self,
        key: &IntrinsicCacheKey,
        context: Option<FormattingContextId>,
        which: Baseline,
    ) -> Option<f32> {
        self.counters.baseline_queries = self.counters.baseline_queries.saturating_add(1);
        // Baseline lookup is observable work in its own right. Keep it out of
        // intrinsic measure hit/miss counters so a placement pass does not
        // report one measure request for every baseline alignment query.
        self.lookup_with_accounting(key, context, false)
            .and_then(|metrics| match which {
                Baseline::First => metrics.first_baseline,
                Baseline::Last => metrics.last_baseline,
            })
    }

    /// Inserts metrics.  A result from another generation is normalized to
    /// the cache's current generation; callers generally leave `generation`
    /// at the value returned by [`Self::generation`].
    pub fn insert(
        &mut self,
        key: IntrinsicCacheKey,
        mut metrics: IntrinsicMetrics,
        context: Option<FormattingContextId>,
    ) -> usize {
        let key = IntrinsicCacheKey {
            constraints: key.constraints.normalized(),
            ..key
        };
        metrics = metrics.normalized();
        metrics.generation = self.generation.get().max(self.node_generation(key.content));
        let bytes = entry_bytes();
        if self.budget.max_entries == 0 || bytes > self.budget.max_bytes {
            return 0;
        }
        self.remove_entry(&key);
        let mut evicted = 0;
        while self.entries.len().saturating_add(1) > self.budget.max_entries
            || self.bytes.saturating_add(bytes) > self.budget.max_bytes
        {
            let Some(oldest) = self.head else {
                break;
            };
            if self.remove_entry(&oldest).is_some() {
                evicted += 1;
            }
        }
        self.entries.insert(
            key,
            Entry {
                metrics,
                global_generation: self.generation.get(),
                node_generation: self.node_generation(key.content),
                context,
                bytes,
                previous: self.tail,
                next: None,
            },
        );
        if let Some(tail) = self.tail {
            self.entries.get_mut(&tail).expect("tail entry").next = Some(key);
        } else {
            self.head = Some(key);
        }
        self.tail = Some(key);
        self.bytes = self.bytes.saturating_add(bytes);
        self.counters.evictions = self.counters.evictions.saturating_add(evicted);
        evicted
    }

    pub fn set_budget(&mut self, budget: IntrinsicCacheBudget) -> usize {
        self.budget = budget;
        let mut evicted = 0;
        while (self.entries.len() > budget.max_entries || self.bytes() > budget.max_bytes)
            && self.evict_oldest()
        {
            evicted += 1;
        }
        self.counters.evictions = self.counters.evictions.saturating_add(evicted);
        evicted
    }

    #[must_use]
    pub fn budget(&self) -> IntrinsicCacheBudget {
        self.budget
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    #[must_use]
    pub fn counters(&self) -> IntrinsicCacheCounters {
        let mut counters = self.counters;
        counters.entries = self.len();
        counters.bytes = self.bytes();
        counters
    }

    pub fn reset_counters(&mut self) {
        self.counters = IntrinsicCacheCounters::default();
    }

    fn touch(&mut self, key: &IntrinsicCacheKey) {
        if self.tail == Some(*key) {
            return;
        }
        let Some((previous, next)) = self
            .entries
            .get(key)
            .map(|entry| (entry.previous, entry.next))
        else {
            return;
        };
        if let Some(previous) = previous {
            self.entries
                .get_mut(&previous)
                .expect("previous entry")
                .next = next;
        } else {
            self.head = next;
        }
        if let Some(next) = next {
            self.entries.get_mut(&next).expect("next entry").previous = previous;
        }
        let old_tail = self.tail;
        if let Some(old_tail) = old_tail {
            self.entries.get_mut(&old_tail).expect("tail entry").next = Some(*key);
        }
        if let Some(entry) = self.entries.get_mut(key) {
            entry.previous = old_tail;
            entry.next = None;
        }
        self.tail = Some(*key);
        if self.head.is_none() {
            self.head = Some(*key);
        }
    }

    fn evict_oldest(&mut self) -> bool {
        self.head
            .is_some_and(|oldest| self.remove_entry(&oldest).is_some())
    }

    fn remove_entry(&mut self, key: &IntrinsicCacheKey) -> Option<Entry> {
        let (previous, next) = self
            .entries
            .get(key)
            .map(|entry| (entry.previous, entry.next))?;
        if let Some(previous) = previous {
            self.entries
                .get_mut(&previous)
                .expect("previous entry")
                .next = next;
        } else {
            self.head = next;
        }
        if let Some(next) = next {
            self.entries.get_mut(&next).expect("next entry").previous = previous;
        } else {
            self.tail = previous;
        }
        let entry = self.entries.remove(key)?;
        self.bytes = self.bytes.saturating_sub(entry.bytes);
        Some(entry)
    }
}

/// Which shared baseline to query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Baseline {
    First,
    Last,
}

const fn entry_bytes() -> usize {
    std::mem::size_of::<IntrinsicCacheKey>() + std::mem::size_of::<Entry>()
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn nonnegative(value: f32) -> f32 {
    finite(value).max(0.0)
}

fn canonical_f32_bits(value: f32) -> u32 {
    let value = finite(value);
    if value == 0.0 {
        0.0f32.to_bits()
    } else {
        value.to_bits()
    }
}

fn prefer_canonical(canonical: usize, compatibility: usize) -> usize {
    if canonical != 0 {
        canonical
    } else {
        compatibility
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> IntrinsicMetrics {
        IntrinsicMetrics::new(10.0, 100.0, 4.0, 40.0, UsedSize::new(80.0, 20.0))
            .with_baselines(Some(12.0), Some(18.0))
            .with_aspect_ratio(Some(2.0))
            .with_truncatable(true)
    }

    #[test]
    fn constraint_classes_are_context_free_and_canonical() {
        let left = IntrinsicCacheKey::new(1, 2, ConstraintClass::MaxInline(-0.0));
        let right = IntrinsicCacheKey::new(1, 2, ConstraintClass::MaxInline(0.0));
        assert_eq!(left, right);
        assert_eq!(
            ConstraintClass::MaxInline(f32::NAN),
            ConstraintClass::Unconstrained
        );
        let a = IntrinsicCacheKey::new(1, 2, ConstraintClass::Unconstrained);
        let b = IntrinsicCacheKey::new(1, 2, ConstraintClass::Unconstrained);
        assert_eq!(a, b, "formatting context is deliberately absent from key");

        let invalid_baselines = metrics().with_baselines(Some(-1.0), Some(f32::NAN));
        assert_eq!(invalid_baselines.first_baseline, None);
        assert_eq!(invalid_baselines.last_baseline, None);
    }

    #[test]
    fn cross_context_lookup_reuses_entry_and_baselines() {
        let mut cache = IntrinsicCache::new(IntrinsicCacheBudget::default());
        let key = IntrinsicCacheKey::new(7, 9, ConstraintClass::Unconstrained);
        cache.insert(key, metrics(), Some(FormattingContextId::new(1)));
        assert_eq!(
            cache.get(&key, Some(FormattingContextId::new(2))),
            Some(metrics().with_generation(0))
        );
        assert_eq!(
            cache.baseline(&key, Some(FormattingContextId::new(3)), Baseline::Last),
            Some(18.0)
        );
        let counters = cache.counters();
        assert_eq!(counters.cache_hits, 1);
        // The baseline query shares the entry but is accounted separately
        // from intrinsic measure hits.
        assert_eq!(counters.cross_context_hits, 1);
        assert_eq!(counters.baseline_queries, 1);
        assert_eq!(counters.intrinsic_measure_requests, 1);
    }

    #[test]
    fn only_geometry_invalidations_bump_generation() {
        let mut cache = IntrinsicCache::default();
        assert!(!cache.invalidate(InvalidationReason::Paint));
        assert!(!cache.invalidate(InvalidationReason::Opacity));
        assert!(!cache.invalidate(InvalidationReason::Transform));
        assert_eq!(cache.generation().get(), 0);
        assert!(cache.invalidate(InvalidationReason::ChildMetrics));
        assert_eq!(cache.generation().get(), 1);
        assert_eq!(cache.counters().generation_bumps, 1);
    }

    #[test]
    fn generation_saturation_does_not_report_a_false_bump() {
        let mut generation = IntrinsicGeneration::new(u64::MAX);
        assert!(!generation.invalidate(InvalidationReason::Content));
        assert_eq!(generation.get(), u64::MAX);

        let mut cache = IntrinsicCache::default();
        cache.node_generations.insert(9, u64::MAX);
        assert!(!cache.invalidate_node(9, InvalidationReason::Content));
        assert_eq!(cache.counters().generation_bumps, 0);
    }

    #[test]
    fn stale_entries_miss_after_generation_bump_and_budget_evicts() {
        let budget = IntrinsicCacheBudget {
            max_entries: 1,
            max_bytes: usize::MAX,
        };
        let mut cache = IntrinsicCache::new(budget);
        let first = IntrinsicCacheKey::new(1, 1, ConstraintClass::Unconstrained);
        let second = IntrinsicCacheKey::new(2, 1, ConstraintClass::Unconstrained);
        cache.insert(first, metrics(), None);
        cache.insert(second, metrics(), None);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.counters().evictions, 1);
        assert!(cache.invalidate(InvalidationReason::Content));
        assert!(cache.get(&second, None).is_none());
        assert_eq!(cache.counters().cache_misses, 1);
    }

    #[test]
    fn hit_promotes_entry_before_the_next_lru_eviction() {
        let budget = IntrinsicCacheBudget {
            max_entries: 2,
            max_bytes: usize::MAX,
        };
        let mut cache = IntrinsicCache::new(budget);
        let first = IntrinsicCacheKey::new(1, 1, ConstraintClass::Unconstrained);
        let second = IntrinsicCacheKey::new(2, 1, ConstraintClass::Unconstrained);
        let third = IntrinsicCacheKey::new(3, 1, ConstraintClass::Unconstrained);
        cache.insert(first, metrics(), None);
        cache.insert(second, metrics(), None);
        assert!(cache.get(&first, None).is_some());
        cache.insert(third, metrics(), None);
        assert!(cache.get(&first, None).is_some());
        assert!(cache.get(&second, None).is_none());
        assert!(cache.get(&third, None).is_some());
    }

    #[test]
    fn counter_aliases_fold_into_required_names_once() {
        let mut total = IntrinsicCacheCounters::default();
        total.accumulate(IntrinsicCacheCounters {
            cache_hits: 2,
            cache_misses: 1,
            full_subtrees: 3,
            ..IntrinsicCacheCounters::default()
        });
        assert_eq!(total.intrinsic_measure_cache_hits, 2);
        assert_eq!(total.intrinsic_measure_cache_misses, 1);
        assert_eq!(total.intrinsic_measure_full_subtrees, 3);
        total.accumulate(IntrinsicCacheCounters {
            cache_hits: 99,
            intrinsic_measure_cache_hits: 4,
            ..IntrinsicCacheCounters::default()
        });
        assert_eq!(total.intrinsic_measure_cache_hits, 6);
    }

    #[test]
    fn byte_budget_tracks_replacements_and_evictions() {
        let bytes_per_entry = entry_bytes();
        let mut cache = IntrinsicCache::new(IntrinsicCacheBudget {
            max_entries: 4,
            max_bytes: bytes_per_entry * 2,
        });
        let first = IntrinsicCacheKey::new(1, 1, ConstraintClass::Unconstrained);
        let second = IntrinsicCacheKey::new(2, 1, ConstraintClass::Unconstrained);
        let third = IntrinsicCacheKey::new(3, 1, ConstraintClass::Unconstrained);
        cache.insert(first, metrics(), None);
        cache.insert(second, metrics(), None);
        assert_eq!(cache.bytes(), bytes_per_entry * 2);
        // Replacement reuses the same slot and does not evict another key.
        cache.insert(first, metrics(), None);
        assert_eq!(cache.bytes(), bytes_per_entry * 2);
        cache.insert(third, metrics(), None);
        assert_eq!(cache.bytes(), bytes_per_entry * 2);
        assert_eq!(cache.len(), 2);
    }
}
