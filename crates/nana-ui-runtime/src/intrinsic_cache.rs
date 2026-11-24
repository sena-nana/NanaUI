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

    /// `(tag, values)` of [`Self::normalized`], read in one match: what
    /// equality and hashing compare, on the hot path of every cache lookup.
    pub(crate) fn canonical(self) -> (u8, (u32, u32)) {
        const UNCONSTRAINED: (u8, (u32, u32)) = (0, (0, 0));
        // A finite extent, clamped at zero, with one zero.
        let bits = |value: f32| {
            let value = value.max(0.0);
            if value == 0.0 { 0 } else { value.to_bits() }
        };
        let one = |tag: u8, value: f32| {
            if value.is_finite() {
                (tag, (bits(value), 0))
            } else {
                UNCONSTRAINED
            }
        };
        let two = |tag: u8, inline: f32, block: f32| {
            if inline.is_finite() && block.is_finite() {
                (tag, (bits(inline), bits(block)))
            } else {
                UNCONSTRAINED
            }
        };
        match self {
            Self::Unconstrained => UNCONSTRAINED,
            Self::MaxInline(value) => one(1, value),
            Self::ExactInline(value) => one(2, value),
            Self::MaxBlock(value) => one(3, value),
            Self::ExactSize { inline, block } => two(4, inline, block),
            Self::PercentageContainingBlock { inline, block }
            | Self::PercentageCb { inline, block }
            | Self::PercentageCB { inline, block } => two(5, inline, block),
            Self::Fill { inline, block } => two(6, inline, block),
            Self::AspectRatio { inline, block } => two(7, inline, block),
        }
    }
}

impl PartialEq for ConstraintClass {
    fn eq(&self, other: &Self) -> bool {
        self.canonical() == other.canonical()
    }
}
impl Eq for ConstraintClass {}
impl Hash for ConstraintClass {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let (tag, values) = self.canonical();
        tag.hash(state);
        values.hash(state);
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

/// Observable work counters for intrinsic measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IntrinsicCacheCounters {
    pub intrinsic_measure_requests: usize,
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
        self.intrinsic_measure_cache_hits = self
            .intrinsic_measure_cache_hits
            .saturating_add(other.intrinsic_measure_cache_hits);
        self.intrinsic_measure_cache_misses = self
            .intrinsic_measure_cache_misses
            .saturating_add(other.intrinsic_measure_cache_misses);
        self.intrinsic_measure_full_subtrees = self
            .intrinsic_measure_full_subtrees
            .saturating_add(other.intrinsic_measure_full_subtrees);
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

/// Which shared baseline to query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Baseline {
    First,
    Last,
}

/// One layout pass's intrinsic facts, laid out as an arena. A fact's key names
/// its node (`content` is the node id), so the facts of one node chain from
/// that node's slot in a table indexed by id, newest first, and the facts
/// themselves sit one after another in the order they were measured.
#[derive(Default)]
pub struct PassIntrinsicFacts {
    facts: Vec<PassFact>,
    /// Facts lookups see: the cache's entries.
    visible: usize,
    /// Index into `facts` of each node's newest fact.
    newest: crate::store::DenseNodeMap<u32>,
    /// Facts whose key names no node (`content` 0); none in practice.
    unnamed: Vec<u32>,
    /// Index into `facts` of the fact each node last measured or was
    /// seeded with: what its baseline is read from.
    latest: crate::store::DenseNodeMap<u32>,
    /// Recordings that differ from what their fact's lookups answer.
    records: Vec<IntrinsicMetrics>,
    counters: IntrinsicCacheCounters,
}

/// One fact of a pass. A pass writes one or two a node, so what a fact
/// holds is what a cold layout's working memory grows by: its recording
/// lives in `metrics` too, because what a lookup answers is the recording
/// normalized, and that is the recording itself for every measurement.
struct PassFact {
    key: IntrinsicCacheKey,
    /// What lookups answer, when `visible`; until then, the recording.
    metrics: IntrinsicMetrics,
    context: Option<FormattingContextId>,
    /// The latest metrics recorded for the retained cache:
    /// [`NOT_RECORDED`], [`RECORDED_AS_HELD`] (`metrics` itself), or where
    /// they differ from `metrics`, an index into
    /// [`PassIntrinsicFacts::records`].
    recorded: u32,
    /// The node's next older fact, or `u32::MAX`.
    older: u32,
    visible: bool,
}

// Two recordings in one fact was 184 bytes.
const _: () = assert!(std::mem::size_of::<PassFact>() <= 128);

const NO_FACT: u32 = u32::MAX;
const NOT_RECORDED: u32 = u32::MAX;
const RECORDED_AS_HELD: u32 = u32::MAX - 1;

impl PassIntrinsicFacts {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            facts: Vec::with_capacity(capacity),
            ..Self::default()
        }
    }

    /// Room for every id up to `highest` in the per-node indexes, which
    /// are about to hold `entries` nodes.
    pub(crate) fn reserve_through(&mut self, highest: crate::StableNodeId, entries: usize) {
        self.newest.reserve_through(highest, entries);
        self.latest.reserve_through(highest, entries);
    }

    fn find(&self, key: &IntrinsicCacheKey) -> Option<usize> {
        let mut index = match crate::StableNodeId::new(key.content) {
            Some(id) => self.newest.get(&id).copied().unwrap_or(NO_FACT),
            None => {
                return self
                    .unnamed
                    .iter()
                    .map(|&index| index as usize)
                    .find(|&index| self.facts[index].key == *key);
            }
        };
        while index != NO_FACT {
            let fact = &self.facts[index as usize];
            if fact.key == *key {
                return Some(index as usize);
            }
            index = fact.older;
        }
        None
    }

    /// Where the fact for `key` lies, appended (unseen, unrecorded) when
    /// there is none.
    fn slot_index(&mut self, key: IntrinsicCacheKey) -> usize {
        if let Some(index) = self.find(&key) {
            return index;
        }
        let index = u32::try_from(self.facts.len()).expect("a pass holds under 4G facts");
        let older = match crate::StableNodeId::new(key.content) {
            Some(id) => self.newest.insert(id, index).unwrap_or(NO_FACT),
            None => {
                self.unnamed.push(index);
                NO_FACT
            }
        };
        self.facts.push(PassFact {
            key,
            metrics: IntrinsicMetrics::new(0.0, 0.0, 0.0, None, UsedSize::new(0.0, 0.0)),
            context: None,
            recorded: NOT_RECORDED,
            older,
            visible: false,
        });
        index as usize
    }

    /// What `fact` holds for the retained cache.
    fn recording(&self, fact: &PassFact) -> Option<IntrinsicMetrics> {
        match fact.recorded {
            NOT_RECORDED => None,
            RECORDED_AS_HELD => Some(fact.metrics),
            aside => Some(self.records[aside as usize]),
        }
    }

    /// Make `metrics` fact `index`'s recording: in the fact itself while
    /// lookups see nothing or the same, aside otherwise. A fact keeps the
    /// place aside it once took, so there are never more than facts.
    fn hold_recording(&mut self, index: usize, metrics: IntrinsicMetrics) {
        let fact = &mut self.facts[index];
        if let Some(aside) = self.records.get_mut(fact.recorded as usize) {
            *aside = metrics;
        } else if !fact.visible || fact.metrics == metrics {
            if !fact.visible {
                fact.metrics = metrics;
            }
            fact.recorded = RECORDED_AS_HELD;
        } else {
            fact.recorded = u32::try_from(self.records.len()).expect("a pass holds under 4G facts");
            self.records.push(metrics);
        }
    }

    /// Make `metrics` what lookups of fact `index` answer, keeping its
    /// recording.
    fn show(
        &mut self,
        index: usize,
        metrics: IntrinsicMetrics,
        context: Option<FormattingContextId>,
    ) {
        let recording = self.recording(&self.facts[index]);
        let fact = &mut self.facts[index];
        let newly = !fact.visible;
        fact.metrics = metrics;
        fact.context = context;
        fact.visible = true;
        self.visible += usize::from(newly);
        if let Some(recording) = recording {
            self.hold_recording(index, recording);
        }
    }

    /// [`Self::insert`], [`Self::record`] and [`Self::note_latest`] for one
    /// measurement, finding its fact once: what a measured box tells the
    /// pass and the retained cache.
    pub(crate) fn insert_measured(
        &mut self,
        id: crate::StableNodeId,
        key: IntrinsicCacheKey,
        metrics: IntrinsicMetrics,
        context: Option<FormattingContextId>,
    ) {
        let normalized = IntrinsicCacheKey {
            constraints: key.constraints.normalized(),
            ..key
        };
        if normalized != key {
            // `record` keeps the key as given: two facts, as before.
            self.insert(key, metrics, context);
            self.record(key, metrics);
            self.note_latest(id, &key);
            return;
        }
        let mut shown = metrics.normalized();
        shown.generation = IntrinsicGeneration::default().get();
        let index = self.slot_index(key);
        let fact = &mut self.facts[index];
        let newly = !fact.visible;
        fact.metrics = shown;
        fact.context = context;
        fact.visible = true;
        self.visible += usize::from(newly);
        // The recording this replaces goes with it.
        self.hold_recording(index, metrics);
        self.latest.insert(id, index as u32);
    }

    /// Make `metrics` what lookups of `key` answer ([`IntrinsicCache::insert`]):
    /// a fact measured again replaces the one it held.
    pub fn insert(
        &mut self,
        key: IntrinsicCacheKey,
        mut metrics: IntrinsicMetrics,
        context: Option<FormattingContextId>,
    ) {
        let key = IntrinsicCacheKey {
            constraints: key.constraints.normalized(),
            ..key
        };
        metrics = metrics.normalized();
        metrics.generation = IntrinsicGeneration::default().get();
        let index = self.slot_index(key);
        self.show(index, metrics, context);
    }

    /// Make the fact for `key` the one `id`'s baseline is read from.
    pub(crate) fn note_latest(&mut self, id: crate::StableNodeId, key: &IntrinsicCacheKey) {
        let key = IntrinsicCacheKey {
            constraints: key.constraints.normalized(),
            ..*key
        };
        if let Some(index) = self.find(&key) {
            self.latest.insert(id, index as u32);
        }
    }

    /// The key [`Self::note_latest`] last named for `id`.
    pub(crate) fn latest_key(&self, id: crate::StableNodeId) -> Option<IntrinsicCacheKey> {
        self.latest
            .get(&id)
            .map(|&index| self.facts[index as usize].key)
    }

    /// Whether lookups of `key` find a fact.
    pub(crate) fn contains(&self, key: &IntrinsicCacheKey) -> bool {
        self.find(key)
            .is_some_and(|index| self.facts[index].visible)
    }

    /// Record `metrics` for `key` for the retained cache; the latest
    /// recording of a key wins.
    pub(crate) fn record(&mut self, key: IntrinsicCacheKey, metrics: IntrinsicMetrics) {
        let index = self.slot_index(key);
        self.hold_recording(index, metrics);
    }

    /// Every key's latest recording, in measuring order.
    #[cfg(test)]
    pub(crate) fn recorded(
        &self,
    ) -> impl Iterator<Item = (IntrinsicCacheKey, IntrinsicMetrics)> + '_ {
        self.facts
            .iter()
            .filter_map(|fact| Some((fact.key, self.recording(fact)?)))
    }

    pub(crate) fn has_recorded(&self, id: crate::StableNodeId) -> bool {
        let mut index = self.newest.get(&id).copied().unwrap_or(NO_FACT);
        while index != NO_FACT {
            let fact = &self.facts[index as usize];
            if fact.recorded != NOT_RECORDED {
                return true;
            }
            index = fact.older;
        }
        false
    }

    pub(crate) fn for_each_node_recorded(
        &self,
        mut visit: impl FnMut(crate::StableNodeId, NodeRecordedFacts<'_>),
    ) {
        for (id, &head) in self.newest.iter() {
            let iter = NodeRecordedFacts {
                facts: &self.facts,
                records: &self.records,
                next: head,
            };
            visit(id, iter);
        }
    }

    pub(crate) fn record_full_subtree(&mut self) {
        self.counters.intrinsic_measure_full_subtrees = self
            .counters
            .intrinsic_measure_full_subtrees
            .saturating_add(1);
    }

    pub(crate) fn record_baseline_miss(&mut self) {
        self.counters.baseline_queries = self.counters.baseline_queries.saturating_add(1);
    }

    fn lookup(
        &mut self,
        key: &IntrinsicCacheKey,
        context: Option<FormattingContextId>,
        account_measure: bool,
    ) -> Option<IntrinsicMetrics> {
        let key = IntrinsicCacheKey {
            constraints: key.constraints.normalized(),
            ..*key
        };
        let found = self
            .find(&key)
            .map(|index| &self.facts[index])
            .filter(|fact| fact.visible);
        if account_measure {
            let counters = &mut self.counters;
            counters.intrinsic_measure_requests =
                counters.intrinsic_measure_requests.saturating_add(1);
            match found {
                Some(fact) => {
                    counters.intrinsic_measure_cache_hits =
                        counters.intrinsic_measure_cache_hits.saturating_add(1);
                    if context.is_some() && fact.context.is_some() && context != fact.context {
                        counters.cross_context_hits = counters.cross_context_hits.saturating_add(1);
                    }
                }
                None => {
                    counters.intrinsic_measure_cache_misses =
                        counters.intrinsic_measure_cache_misses.saturating_add(1);
                    if context.is_some() {
                        counters.cross_context_misses =
                            counters.cross_context_misses.saturating_add(1);
                    }
                }
            }
        }
        found.map(|fact| fact.metrics)
    }

    pub fn get(
        &mut self,
        key: &IntrinsicCacheKey,
        context: Option<FormattingContextId>,
    ) -> Option<IntrinsicMetrics> {
        self.lookup(key, context, true)
    }

    pub fn baseline(
        &mut self,
        key: &IntrinsicCacheKey,
        context: Option<FormattingContextId>,
        which: Baseline,
    ) -> Option<f32> {
        self.counters.baseline_queries = self.counters.baseline_queries.saturating_add(1);
        self.lookup(key, context, false)
            .and_then(|metrics| match which {
                Baseline::First => metrics.first_baseline,
                Baseline::Last => metrics.last_baseline,
            })
    }

    pub fn counters(&self) -> IntrinsicCacheCounters {
        let mut counters = self.counters;
        counters.entries = self.visible;
        counters.bytes = self.visible.saturating_mul(std::mem::size_of::<PassFact>());
        counters
    }
}

#[derive(Clone)]
pub(crate) struct NodeRecordedFacts<'a> {
    facts: &'a [PassFact],
    records: &'a [IntrinsicMetrics],
    next: u32,
}

impl Iterator for NodeRecordedFacts<'_> {
    type Item = (IntrinsicCacheKey, IntrinsicMetrics);

    fn next(&mut self) -> Option<Self::Item> {
        while self.next != NO_FACT {
            let fact = &self.facts[self.next as usize];
            self.next = fact.older;
            let metrics = match fact.recorded {
                NOT_RECORDED => continue,
                RECORDED_AS_HELD => fact.metrics,
                aside => self.records[aside as usize],
            };
            return Some((fact.key, metrics));
        }
        None
    }
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn nonnegative(value: f32) -> f32 {
    finite(value).max(0.0)
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
    fn only_geometry_invalidations_bump_generation() {
        let mut generation = IntrinsicGeneration::default();
        assert!(!generation.invalidate(InvalidationReason::Paint));
        assert!(!generation.invalidate(InvalidationReason::Opacity));
        assert!(!generation.invalidate(InvalidationReason::Transform));
        assert_eq!(generation.get(), 0);
        assert!(generation.invalidate(InvalidationReason::ChildMetrics));
        assert_eq!(generation.get(), 1);
    }

    #[test]
    fn generation_saturation_does_not_report_a_false_bump() {
        let mut generation = IntrinsicGeneration::new(u64::MAX);
        assert!(!generation.invalidate(InvalidationReason::Content));
        assert_eq!(generation.get(), u64::MAX);
    }

    #[test]
    fn counters_accumulate_canonical_fields() {
        let mut total = IntrinsicCacheCounters::default();
        total.accumulate(IntrinsicCacheCounters {
            intrinsic_measure_cache_hits: 2,
            intrinsic_measure_cache_misses: 1,
            intrinsic_measure_full_subtrees: 3,
            ..IntrinsicCacheCounters::default()
        });
        assert_eq!(total.intrinsic_measure_cache_hits, 2);
        assert_eq!(total.intrinsic_measure_cache_misses, 1);
        assert_eq!(total.intrinsic_measure_full_subtrees, 3);
        total.accumulate(IntrinsicCacheCounters {
            intrinsic_measure_cache_hits: 4,
            ..IntrinsicCacheCounters::default()
        });
        assert_eq!(total.intrinsic_measure_cache_hits, 6);
    }

    /// A pass's facts answer and hand over what a fact holding what lookups
    /// answer and its recording side by side would: recordings that differ
    /// from the answer (a later generation, unnormalized values) and keys
    /// that normalize to another are interleaved with every write.
    #[test]
    fn pass_facts_hold_one_copy_and_answer_as_two() {
        #[derive(Default)]
        struct Model {
            order: Vec<IntrinsicCacheKey>,
            shown: Vec<Option<IntrinsicMetrics>>,
            recorded: Vec<Option<IntrinsicMetrics>>,
        }
        impl Model {
            fn slot(&mut self, key: IntrinsicCacheKey) -> usize {
                if let Some(index) = self.order.iter().position(|held| *held == key) {
                    return index;
                }
                self.order.push(key);
                self.shown.push(None);
                self.recorded.push(None);
                self.order.len() - 1
            }
            fn show(&mut self, key: IntrinsicCacheKey, metrics: IntrinsicMetrics) {
                let key = IntrinsicCacheKey {
                    constraints: key.constraints.normalized(),
                    ..key
                };
                let index = self.slot(key);
                self.shown[index] = Some(metrics.normalized().with_generation(0));
            }
            fn record(&mut self, key: IntrinsicCacheKey, metrics: IntrinsicMetrics) {
                let index = self.slot(key);
                self.recorded[index] = Some(metrics);
            }
            fn get(&self, key: &IntrinsicCacheKey) -> Option<IntrinsicMetrics> {
                let key = IntrinsicCacheKey {
                    constraints: key.constraints.normalized(),
                    ..*key
                };
                let index = self.order.iter().position(|held| *held == key)?;
                self.shown[index]
            }
        }

        let keys: Vec<IntrinsicCacheKey> = (1..=3)
            .flat_map(|content| {
                [
                    ConstraintClass::Unconstrained,
                    ConstraintClass::MaxInline(40.0),
                    // Normalizes to `Unconstrained`.
                    ConstraintClass::MaxInline(f32::INFINITY),
                ]
                .map(|constraints| IntrinsicCacheKey {
                    content,
                    style: 5,
                    constraints,
                })
            })
            .collect();
        let values = [
            metrics(),
            metrics().with_generation(3),
            IntrinsicMetrics::new(-4.0, 30.0, 2.0, 9.0, UsedSize::new(30.0, 9.0)),
            IntrinsicMetrics::new(6.0, 60.0, 8.0, None, UsedSize::new(60.0, 8.0)),
        ];
        let mut facts = PassIntrinsicFacts::default();
        let mut model = Model::default();
        let mut state = 0x2545_f491_u32;
        for step in 0..4_000 {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let key = keys[(state >> 8) as usize % keys.len()];
            let value = values[(state >> 16) as usize % values.len()];
            match (state >> 24) % 3 {
                0 => {
                    facts.insert(key, value, None);
                    model.show(key, value);
                }
                1 => {
                    facts.record(key, value);
                    model.record(key, value);
                }
                _ => {
                    let id = crate::StableNodeId::new(key.content).unwrap();
                    facts.insert_measured(id, key, value, None);
                    model.show(key, value);
                    model.record(key, value);
                }
            }
            for key in &keys {
                assert_eq!(facts.get(key, None), model.get(key), "step {step}, {key:?}");
            }
            let expected: Vec<_> = model
                .order
                .iter()
                .zip(&model.recorded)
                .filter_map(|(key, recorded)| Some((*key, (*recorded)?)))
                .collect();
            assert_eq!(
                facts.recorded().collect::<Vec<_>>(),
                expected,
                "step {step}"
            );
        }
        // Some recordings were held aside, never more than a place a fact.
        assert!(!facts.records.is_empty());
        assert!(facts.records.len() <= facts.facts.len());
    }
}
