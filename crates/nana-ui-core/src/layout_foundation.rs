//! Canonical layout foundation contracts.
//!
//! The foundation deliberately contains data and lifecycle rules, rather than
//! a universal layout algorithm.  Flex, grid, inline and overlay algorithms
//! can all consume the same [`LayoutNode`] and publish a [`LayoutResult`].
//! A node's identity is retained while its participation and parent context
//! change; changing context therefore never requires a component wrapper or a
//! second business-state tree.

use std::collections::{HashMap, HashSet};

use crate::{DisplaySpec, LayoutIntent};

/// Stable identity owned by the retained UI tree.
///
/// Zero is reserved for an absent identity.  The foundation stores no
/// component state and does not allocate a new id when a node changes context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayoutNodeId(u64);

impl LayoutNodeId {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for LayoutNodeId {
    type Error = &'static str;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value).ok_or("layout node identity zero is reserved")
    }
}

/// The parent-owned algorithm that arranges a node's children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FormattingContext {
    /// `display:none`: the retained identity remains, but no box is emitted.
    None,
    /// `display:contents`: the identity remains, but its children participate
    /// directly in the nearest generated ancestor's formatting context.
    Contents,
    /// Ordinary block/flow arrangement.
    #[default]
    Flow,
    Flex,
    Grid,
    Inline,
    /// Overlapping children (the unambiguous meaning of an overlay Stack).
    Overlay,
}

impl FormattingContext {
    /// Lower a resolved display intent without making the display value a node
    /// type.  `Stack::row`/`Stack::column` callers should pass Flex and set the
    /// direction in their intent; only true overlap lowers to Overlay.
    pub const fn from_display(display: DisplaySpec) -> Self {
        match display {
            DisplaySpec::Flex | DisplaySpec::InlineFlex => Self::Flex,
            DisplaySpec::Grid | DisplaySpec::InlineGrid => Self::Grid,
            DisplaySpec::Inline | DisplaySpec::InlineBlock => Self::Inline,
            DisplaySpec::Contents => Self::Contents,
            DisplaySpec::None => Self::None,
            DisplaySpec::Block => Self::Flow,
        }
    }

    pub const fn is_inline(self) -> bool {
        matches!(self, Self::Inline)
    }

    /// Whether this context owns a layout box. `display:none` and
    /// `display:contents` retain identity for lifecycle/state purposes but are
    /// transparent to geometry consumers.
    pub const fn generates_box(self) -> bool {
        !matches!(self, Self::None | Self::Contents)
    }

    pub const fn is_transparent(self) -> bool {
        matches!(self, Self::Contents)
    }
}

/// Placement mode is orthogonal to the formatting context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlacementMode {
    #[default]
    NormalFlow,
    Absolute,
    Fixed,
    Sticky,
    Overlay,
}

impl PlacementMode {
    pub const fn is_out_of_flow(self) -> bool {
        matches!(self, Self::Absolute | Self::Fixed | Self::Overlay)
    }
}

/// A resource-backed atomic box can participate in every context without
/// becoming a new component type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplacedContent {
    pub intrinsic_size: Option<LayoutSize>,
    pub aspect_ratio: Option<f32>,
    pub baseline: BaselinePolicy,
    pub fit: ObjectFit,
    pub resource_generation: u64,
}

impl Default for ReplacedContent {
    fn default() -> Self {
        Self {
            intrinsic_size: None,
            aspect_ratio: None,
            baseline: BaselinePolicy::Bottom,
            fit: ObjectFit::Contain,
            resource_generation: 0,
        }
    }
}

/// Baseline fallback for atomic/replaced content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BaselinePolicy {
    #[default]
    Bottom,
    Center,
    FirstBaseline,
}

/// Object-fit policy is part of presentation sizing, not resource identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ObjectFit {
    #[default]
    Contain,
    Cover,
    Fill,
    None,
    /// Keep intrinsic size when it fits, otherwise contain within the box.
    ScaleDown,
}

/// The context-specific view of a retained node.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Participation {
    #[default]
    NormalFlow,
    FlexItem,
    GridItem,
    NativeTextInline,
    AtomicInlineBox,
    OutOfFlow(PlacementMode),
    Replaced(ReplacedContent),
    /// Unsupported data is kept visible to diagnostics and fails closed; it
    /// never silently creates a wrapper or changes component identity.
    Unsupported,
}

impl Participation {
    pub const fn placement(self) -> PlacementMode {
        match self {
            Self::OutOfFlow(mode) => mode,
            _ => PlacementMode::NormalFlow,
        }
    }

    pub const fn is_supported(self) -> bool {
        !matches!(self, Self::Unsupported)
    }

    pub const fn is_atomic(self) -> bool {
        matches!(self, Self::AtomicInlineBox | Self::Replaced(_))
    }
}

/// Orthogonal node behaviour.  A scroll/clip change affects presentation and
/// hit testing; it does not imply a new formatting context or intrinsic pass.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutBehavior {
    pub scroll_x: bool,
    pub scroll_y: bool,
    pub clip: bool,
    pub viewport: bool,
    pub scroll_offset: LayoutSize,
}

/// Canonical layout identity and its retained child sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutNode {
    pub id: LayoutNodeId,
    pub intent: LayoutIntent,
    pub formatting_context: FormattingContext,
    pub participation: Participation,
    pub placement: PlacementMode,
    pub behavior: LayoutBehavior,
    pub children: Vec<LayoutNodeId>,
    pub context_generation: u64,
}

impl LayoutNode {
    pub fn new(id: LayoutNodeId, intent: LayoutIntent, context: FormattingContext) -> Self {
        Self {
            id,
            intent,
            formatting_context: context,
            participation: Participation::default(),
            placement: PlacementMode::NormalFlow,
            behavior: LayoutBehavior::default(),
            children: Vec::new(),
            context_generation: 0,
        }
    }

    pub const fn identity(&self) -> LayoutNodeId {
        self.id
    }

    /// Return the view a parent context consumes.  This is a cheap value
    /// record, never a retained wrapper node.  Out-of-flow and replaced
    /// capabilities survive context changes; ordinary children receive the
    /// context-specific item kind.
    pub const fn participate(&self, parent: FormattingContext) -> Participation {
        if !self.formatting_context.generates_box() {
            return Participation::Unsupported;
        }
        if let Participation::OutOfFlow(mode) = self.participation {
            return Participation::OutOfFlow(mode);
        }
        if self.placement.is_out_of_flow() {
            return Participation::OutOfFlow(self.placement);
        }
        match self.participation {
            Participation::Replaced(content) => Participation::Replaced(content),
            Participation::Unsupported => Participation::Unsupported,
            Participation::NativeTextInline if parent.is_inline() => {
                Participation::NativeTextInline
            }
            Participation::AtomicInlineBox if parent.is_inline() => Participation::AtomicInlineBox,
            _ => match parent {
                FormattingContext::Flex => Participation::FlexItem,
                FormattingContext::Grid => Participation::GridItem,
                FormattingContext::Inline => Participation::AtomicInlineBox,
                FormattingContext::None
                | FormattingContext::Contents
                | FormattingContext::Flow
                | FormattingContext::Overlay => Participation::NormalFlow,
            },
        }
    }

    /// The context this node establishes for its own children.  Parent
    /// participation and child formatting are intentionally separate queries.
    pub const fn establish_formatting_context(&self) -> FormattingContext {
        self.formatting_context
    }

    /// Set a parent-owned participation record while retaining this node.
    pub fn set_participation(&mut self, participation: Participation) {
        // Placement is an orthogonal capability.  Replaced/atomic content may
        // still be absolutely/fixed/sticky positioned, and remapping its
        // resource participation must not silently reset that placement.
        // `OutOfFlow` remains the one participation variant that carries a
        // placement mode directly.
        if matches!(participation, Participation::OutOfFlow(_)) {
            self.placement = participation.placement();
        }
        self.participation = participation;
    }

    /// Change only the parent algorithm.  Children and component identity are
    /// deliberately untouched.
    pub fn set_formatting_context(&mut self, context: FormattingContext) -> bool {
        if self.formatting_context == context {
            return false;
        }
        self.formatting_context = context;
        self.context_generation = self.context_generation.saturating_add(1);
        true
    }
}

/// Finite logical size used by metrics and result contracts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutSize {
    pub inline: f32,
    pub block: f32,
}

impl LayoutSize {
    pub const ZERO: Self = Self {
        inline: 0.0,
        block: 0.0,
    };

    pub fn new(inline: f32, block: f32) -> Self {
        Self {
            inline: finite_non_negative(inline),
            block: finite_non_negative(block),
        }
    }

    pub fn is_finite(self) -> bool {
        self.inline.is_finite() && self.block.is_finite()
    }
}

/// Size chosen by a parent formatting context.  It is deliberately a
/// different type from [`LayoutSize`] (intrinsic facts) so a compressed used
/// size cannot accidentally be written back into the metrics cache.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UsedSize {
    pub inline: f32,
    pub block: f32,
}

impl UsedSize {
    pub fn new(inline: f32, block: f32) -> Self {
        Self {
            inline: finite_non_negative(inline),
            block: finite_non_negative(block),
        }
    }

    pub fn is_finite(self) -> bool {
        self.inline.is_finite() && self.block.is_finite()
    }
}

impl From<LayoutSize> for UsedSize {
    fn from(value: LayoutSize) -> Self {
        Self::new(value.inline, value.block)
    }
}

/// A finite logical rectangle.  Inputs from an untrusted style/host boundary
/// are normalized once here rather than being allowed to poison caches.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutRect {
    pub inline: f32,
    pub block: f32,
    pub size: LayoutSize,
}

impl LayoutRect {
    pub const ZERO: Self = Self {
        inline: 0.0,
        block: 0.0,
        size: LayoutSize::ZERO,
    };

    pub fn new(inline: f32, block: f32, size: LayoutSize) -> Self {
        Self {
            inline: finite(inline),
            block: finite(block),
            size: LayoutSize::new(size.inline, size.block),
        }
    }

    pub fn is_finite(self) -> bool {
        self.inline.is_finite() && self.block.is_finite() && self.size.is_finite()
    }
}

/// Shared intrinsic facts.  Used size is chosen by a parent context and is
/// never written back into these metrics.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct IntrinsicMetrics {
    pub min_inline: f32,
    pub max_inline: f32,
    pub min_block: f32,
    pub max_block: Option<f32>,
    pub preferred: LayoutSize,
    pub first_baseline: Option<f32>,
    pub last_baseline: Option<f32>,
    pub aspect_ratio: Option<f32>,
    pub generation: u64,
}

impl IntrinsicMetrics {
    pub fn new(preferred: LayoutSize) -> Self {
        Self {
            min_inline: 0.0,
            max_inline: preferred.inline,
            min_block: 0.0,
            max_block: Some(preferred.block),
            preferred,
            ..Self::default()
        }
    }

    pub fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    pub fn sanitized(mut self) -> Self {
        self.min_inline = finite_non_negative(self.min_inline);
        self.max_inline = finite_non_negative(self.max_inline).max(self.min_inline);
        self.min_block = finite_non_negative(self.min_block);
        self.max_block = self.max_block.map(finite_non_negative);
        self.preferred = LayoutSize::new(self.preferred.inline, self.preferred.block);
        self.first_baseline = self
            .first_baseline
            .filter(|v| v.is_finite())
            .map(|v| v.max(0.0));
        self.last_baseline = self
            .last_baseline
            .filter(|v| v.is_finite())
            .map(|v| v.max(0.0));
        self.aspect_ratio = self.aspect_ratio.filter(|v| v.is_finite() && *v > 0.0);
        self
    }
}

/// Constraint classes describe only dimensions that can change intrinsic
/// measurement.  The formatting-context name is intentionally absent.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ConstraintClass {
    #[default]
    Unconstrained,
    MaxInline(f32),
    ExactInline(f32),
    MaxBlock(f32),
    ExactSize {
        inline: f32,
        block: f32,
    },
    PercentageContainingBlock {
        inline: f32,
        block: f32,
    },
}

impl ConstraintClass {
    fn key(self) -> ConstraintKey {
        match self {
            Self::Unconstrained => ConstraintKey::new(0, 0.0, 0.0),
            Self::MaxInline(value) => ConstraintKey::new(1, finite_non_negative(value), 0.0),
            Self::ExactInline(value) => ConstraintKey::new(2, finite_non_negative(value), 0.0),
            Self::MaxBlock(value) => ConstraintKey::new(3, 0.0, finite_non_negative(value)),
            Self::ExactSize { inline, block } => {
                ConstraintKey::new(4, finite_non_negative(inline), finite_non_negative(block))
            }
            Self::PercentageContainingBlock { inline, block } => {
                ConstraintKey::new(5, finite_non_negative(inline), finite_non_negative(block))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ConstraintKey {
    kind: u8,
    first: u32,
    second: u32,
}

impl ConstraintKey {
    fn new(kind: u8, first: f32, second: f32) -> Self {
        Self {
            kind,
            first: first.to_bits(),
            second: second.to_bits(),
        }
    }
}

/// A placed child entry in a result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutPlacement {
    pub node: LayoutNodeId,
    pub bounds: LayoutRect,
    pub participation: Participation,
}

/// A renderer/input-facing fragment.  Fragments describe layout parts, not
/// GPU primitives; scene, hit test and accessibility can project the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FragmentKind {
    Box,
    Text,
    Atomic,
    Overlay,
    Viewport,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutFragment {
    pub node: LayoutNodeId,
    pub bounds: LayoutRect,
    pub kind: FragmentKind,
}

/// Authoritative geometry result for one canonical node.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutResult {
    pub node: LayoutNodeId,
    pub bounds: LayoutRect,
    pub content_box: LayoutRect,
    pub padding_box: LayoutRect,
    pub border_box: LayoutRect,
    pub overflow: LayoutRect,
    pub scroll_extent: UsedSize,
    pub clip: Option<LayoutRect>,
    pub containing_block: Option<LayoutNodeId>,
    pub used_size: UsedSize,
    pub first_baseline: Option<f32>,
    pub last_baseline: Option<f32>,
    pub children: Vec<LayoutPlacement>,
    pub fragments: Vec<LayoutFragment>,
    pub dependency_footprint: Vec<LayoutNodeId>,
    pub generation: u64,
}

impl LayoutResult {
    pub fn new(node: LayoutNodeId, bounds: LayoutRect, generation: u64) -> Self {
        Self {
            node,
            bounds,
            content_box: bounds,
            padding_box: bounds,
            border_box: bounds,
            overflow: bounds,
            scroll_extent: bounds.size.into(),
            clip: None,
            containing_block: None,
            used_size: bounds.size.into(),
            first_baseline: None,
            last_baseline: None,
            children: Vec::new(),
            fragments: Vec::new(),
            dependency_footprint: vec![node],
            generation,
        }
    }

    pub fn is_finite(&self) -> bool {
        self.bounds.is_finite()
            && self.content_box.is_finite()
            && self.padding_box.is_finite()
            && self.border_box.is_finite()
            && self.overflow.is_finite()
            && self.scroll_extent.is_finite()
            && self.used_size.is_finite()
            && self.clip.is_none_or(LayoutRect::is_finite)
            && self.first_baseline.is_none_or(f32::is_finite)
            && self.last_baseline.is_none_or(f32::is_finite)
            && self.children.iter().all(|child| child.bounds.is_finite())
            && self
                .fragments
                .iter()
                .all(|fragment| fragment.bounds.is_finite())
    }
}

/// Structural work counters for Foundation gates.  A zero value means no
/// foundation work was observed; no fake zero is emitted for an unsupported
/// renderer metric because these counters are CPU/layout-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutFoundationCounters {
    pub layout_nodes_considered: usize,
    pub layout_contexts_entered: usize,
    pub layout_context_transitions: usize,
    pub layout_participation_remaps: usize,
    pub intrinsic_measure_requests: usize,
    pub intrinsic_measure_cache_hits: usize,
    pub intrinsic_measure_cache_misses: usize,
    pub intrinsic_measure_cache_evictions: usize,
    pub intrinsic_measure_full_subtrees: usize,
    pub intrinsic_generation_bumps: usize,
    pub baseline_queries: usize,
    pub cross_context_measure_hits: usize,
    pub cross_context_measure_misses: usize,
    pub layout_results_created: usize,
    pub layout_results_reused: usize,
    pub layout_fragments_created: usize,
    pub layout_fragments_reused: usize,
    pub layout_nodes_placed: usize,
    pub layout_dependency_edges_visited: usize,
    pub layout_component_created: usize,
    pub layout_component_destroyed: usize,
    pub layout_wrapper_nodes_created: usize,
    pub layout_full_document_fallbacks: usize,
    pub layout_full_subtree_rebuilds: usize,
    pub scroll_viewport_updates: usize,
    pub scroll_content_extent_recomputes: usize,
    pub scroll_layout_reflows: usize,
    pub scroll_clip_updates: usize,
    pub replaced_resource_rebinds: usize,
}

impl LayoutFoundationCounters {
    pub fn accumulate(&mut self, other: Self) {
        macro_rules! add {
            ($($field:ident),+ $(,)?) => {
                $(
                self.$field = self.$field.saturating_add(other.$field);
                )+
            }
        }
        add!(
            layout_nodes_considered,
            layout_contexts_entered,
            layout_context_transitions,
            layout_participation_remaps,
            intrinsic_measure_requests,
            intrinsic_measure_cache_hits,
            intrinsic_measure_cache_misses,
            intrinsic_measure_cache_evictions,
            intrinsic_measure_full_subtrees,
            intrinsic_generation_bumps,
            baseline_queries,
            cross_context_measure_hits,
            cross_context_measure_misses,
            layout_results_created,
            layout_results_reused,
            layout_fragments_created,
            layout_fragments_reused,
            layout_nodes_placed,
            layout_dependency_edges_visited,
            layout_component_created,
            layout_component_destroyed,
            layout_wrapper_nodes_created,
            layout_full_document_fallbacks,
            layout_full_subtree_rebuilds,
            scroll_viewport_updates,
            scroll_content_extent_recomputes,
            scroll_layout_reflows,
            scroll_clip_updates,
            replaced_resource_rebinds,
        );
    }

    pub fn is_idle(self) -> bool {
        self == Self::default()
    }

    /// Return the work observed after `previous` in a cumulative counter
    /// snapshot.  Counters use saturating arithmetic, so a reset or a wrapped
    /// producer is treated as a fresh snapshot instead of underflowing.
    pub fn delta_since(self, previous: Self) -> Self {
        macro_rules! delta {
            ($($field:ident),+ $(,)?) => {
                Self {
                    $($field: self.$field.checked_sub(previous.$field).unwrap_or(self.$field),)+
                }
            }
        }
        delta!(
            layout_nodes_considered,
            layout_contexts_entered,
            layout_context_transitions,
            layout_participation_remaps,
            intrinsic_measure_requests,
            intrinsic_measure_cache_hits,
            intrinsic_measure_cache_misses,
            intrinsic_measure_cache_evictions,
            intrinsic_measure_full_subtrees,
            intrinsic_generation_bumps,
            baseline_queries,
            cross_context_measure_hits,
            cross_context_measure_misses,
            layout_results_created,
            layout_results_reused,
            layout_fragments_created,
            layout_fragments_reused,
            layout_nodes_placed,
            layout_dependency_edges_visited,
            layout_component_created,
            layout_component_destroyed,
            layout_wrapper_nodes_created,
            layout_full_document_fallbacks,
            layout_full_subtree_rebuilds,
            scroll_viewport_updates,
            scroll_content_extent_recomputes,
            scroll_layout_reflows,
            scroll_clip_updates,
            replaced_resource_rebinds,
        )
    }

    pub fn record_baseline_query(&mut self) {
        self.baseline_queries = self.baseline_queries.saturating_add(1);
    }

    pub fn record_full_subtree_measure(&mut self) {
        self.intrinsic_measure_full_subtrees =
            self.intrinsic_measure_full_subtrees.saturating_add(1);
    }

    pub fn record_placement(&mut self, dependency_edges: usize) {
        self.layout_nodes_placed = self.layout_nodes_placed.saturating_add(1);
        self.layout_dependency_edges_visited = self
            .layout_dependency_edges_visited
            .saturating_add(dependency_edges);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MetricsEntry {
    generation: u64,
    key: ConstraintKey,
}

/// Retained Foundation state.  Existing layout algorithms can use this as an
/// adapter without moving their implementation into this crate.
#[derive(Debug)]
pub struct LayoutFoundation {
    nodes: HashMap<LayoutNodeId, LayoutNode>,
    metrics: HashMap<(LayoutNodeId, ConstraintKey), IntrinsicMetrics>,
    metric_entries: HashMap<LayoutNodeId, HashSet<ConstraintKey>>,
    last_measure: HashMap<LayoutNodeId, MetricsEntry>,
    last_measure_context: HashMap<LayoutNodeId, FormattingContext>,
    results: HashMap<LayoutNodeId, LayoutResult>,
    metrics_generations: HashMap<LayoutNodeId, u64>,
    metric_budget: usize,
    generation: u64,
    counters: LayoutFoundationCounters,
}

impl Default for LayoutFoundation {
    fn default() -> Self {
        Self {
            nodes: HashMap::new(),
            metrics: HashMap::new(),
            metric_entries: HashMap::new(),
            last_measure: HashMap::new(),
            last_measure_context: HashMap::new(),
            results: HashMap::new(),
            metrics_generations: HashMap::new(),
            metric_budget: 4096,
            generation: 0,
            counters: LayoutFoundationCounters::default(),
        }
    }
}

impl LayoutFoundation {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bound retained intrinsic entries so context churn cannot grow memory
    /// without limit.  Zero disables retention while preserving correctness.
    pub fn set_metric_budget(&mut self, budget: usize) {
        self.metric_budget = budget;
        while self.metrics.len() > budget {
            self.evict_one_metric();
        }
    }

    pub fn metric_budget(&self) -> usize {
        self.metric_budget
    }

    pub fn node(&self, id: LayoutNodeId) -> Option<&LayoutNode> {
        self.nodes.get(&id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &LayoutNode> {
        self.nodes.values()
    }

    pub fn insert(&mut self, node: LayoutNode) -> bool {
        let id = node.id;
        if self.nodes.contains_key(&id) {
            return false;
        }
        self.nodes.insert(id, node);
        self.metrics_generations.insert(id, 0);
        self.counters.layout_nodes_considered =
            self.counters.layout_nodes_considered.saturating_add(1);
        true
    }

    /// Remove one retained identity.  This is reserved for a real tree
    /// removal; context changes use [`Self::transition_context`] instead.
    pub fn remove(&mut self, id: LayoutNodeId) -> Option<LayoutNode> {
        let removed = self.nodes.remove(&id)?;
        self.metrics.retain(|(node, _), _| *node != id);
        self.metric_entries.remove(&id);
        self.last_measure.remove(&id);
        self.last_measure_context.remove(&id);
        self.metrics_generations.remove(&id);
        self.results.remove(&id);
        self.counters.layout_component_destroyed =
            self.counters.layout_component_destroyed.saturating_add(1);
        Some(removed)
    }

    /// Keep only identities present in the current document.  The caller owns
    /// document scoping; this prevents metrics/results from one document being
    /// accidentally reused for another when a retained adapter is recycled.
    pub fn retain_nodes<I>(&mut self, ids: I)
    where
        I: IntoIterator<Item = LayoutNodeId>,
    {
        let live = ids.into_iter().collect::<HashSet<_>>();
        let stale = self
            .nodes
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect::<Vec<_>>();
        for id in stale {
            self.remove(id);
        }
    }

    /// Insert a canonical identity once, returning whether a new retained
    /// layout identity was created.  Repeated calls are lifecycle-neutral.
    pub fn ensure_node(
        &mut self,
        id: LayoutNodeId,
        intent: LayoutIntent,
        context: FormattingContext,
    ) -> bool {
        self.insert(LayoutNode::new(id, intent, context))
    }

    /// Upsert intent/children while preserving the canonical identity.
    pub fn upsert(&mut self, node: LayoutNode) -> bool {
        if self.nodes.contains_key(&node.id) {
            self.transition_context(node.id, node.formatting_context);
            self.remap_participation(node.id, node.participation);
            self.set_children(node.id, node.children);
            let existing = self.nodes.get_mut(&node.id).expect("checked above");
            let intent_changed = existing.intent != node.intent;
            let behavior_changed = existing.behavior != node.behavior;
            let placement_changed = existing.placement != node.placement;
            existing.intent = node.intent;
            existing.behavior = node.behavior;
            existing.placement = node.placement;
            // Clip/viewport/scroll-axis changes alter the presentation-facing
            // result even when intrinsic layout is unchanged.  A scroll
            // offset alone is updated through `update_scroll_offset` and is
            // intentionally paint-only, so it does not reach this path.
            if intent_changed || behavior_changed || placement_changed {
                self.invalidate_result(node.id);
            }
            return false;
        }
        self.insert(node)
    }

    pub fn set_children(&mut self, id: LayoutNodeId, children: Vec<LayoutNodeId>) -> bool {
        let Some(node) = self.nodes.get_mut(&id) else {
            return false;
        };
        if node.children == children {
            return false;
        }
        node.children = children;
        self.invalidate_result(id);
        true
    }

    pub fn transition_context(&mut self, id: LayoutNodeId, context: FormattingContext) -> bool {
        let Some(node) = self.nodes.get_mut(&id) else {
            return false;
        };
        if !node.set_formatting_context(context) {
            return false;
        }
        self.generation = self.generation.saturating_add(1);
        self.counters.layout_context_transitions =
            self.counters.layout_context_transitions.saturating_add(1);
        self.counters.layout_contexts_entered =
            self.counters.layout_contexts_entered.saturating_add(1);
        self.invalidate_result(id);
        true
    }

    /// Alias used by adapters that model a context change as a transition
    /// operation.  It has exactly the same zero-lifecycle semantics.
    pub fn transition(&mut self, id: LayoutNodeId, context: FormattingContext) -> bool {
        self.transition_context(id, context)
    }

    pub fn remap_participation(&mut self, id: LayoutNodeId, participation: Participation) -> bool {
        let Some(node) = self.nodes.get_mut(&id) else {
            return false;
        };
        if node.participation == participation {
            return false;
        }
        node.set_participation(participation);
        self.counters.layout_participation_remaps =
            self.counters.layout_participation_remaps.saturating_add(1);
        self.invalidate_result(id);
        true
    }

    /// Update a replaced resource's frame/handle without treating it as a
    /// participation transition.  Intrinsic metadata changes invalidate the
    /// result; a video/texture frame with the same metadata is presentation
    /// work only and keeps measure/placement at zero.
    pub fn set_replaced_content(
        &mut self,
        id: LayoutNodeId,
        content: ReplacedContent,
        intrinsic_metadata_changed: bool,
    ) -> bool {
        let Some(node) = self.nodes.get_mut(&id) else {
            return false;
        };
        let Participation::Replaced(previous) = node.participation else {
            return false;
        };
        if previous == content {
            return false;
        }
        node.participation = Participation::Replaced(content);
        self.counters.replaced_resource_rebinds =
            self.counters.replaced_resource_rebinds.saturating_add(1);
        if intrinsic_metadata_changed
            && (previous.intrinsic_size != content.intrinsic_size
                || previous.aspect_ratio != content.aspect_ratio
                || previous.baseline != content.baseline)
        {
            self.counters.intrinsic_generation_bumps =
                self.counters.intrinsic_generation_bumps.saturating_add(1);
            self.invalidate_result(id);
        }
        true
    }

    /// Scroll-only presentation update.  It never invalidates intrinsic
    /// metrics or child placement.
    pub fn update_scroll_offset(&mut self, id: LayoutNodeId, offset: LayoutSize) -> bool {
        let Some(node) = self.nodes.get_mut(&id) else {
            return false;
        };
        let offset = LayoutSize::new(offset.inline, offset.block);
        if node.behavior.scroll_offset == offset {
            return false;
        }
        node.behavior.scroll_offset = offset;
        self.counters.scroll_viewport_updates =
            self.counters.scroll_viewport_updates.saturating_add(1);
        if node.behavior.clip {
            self.counters.scroll_clip_updates = self.counters.scroll_clip_updates.saturating_add(1);
        }
        true
    }

    pub fn set_metrics(&mut self, id: LayoutNodeId, metrics: IntrinsicMetrics) -> bool {
        if !self.nodes.contains_key(&id) {
            return false;
        }
        let mut metrics = metrics.sanitized();
        let current_generation = self.metrics_generations.get(&id).copied().unwrap_or(0);
        // `set_metrics` publishes the node's canonical intrinsic metadata,
        // independent of the constraint-keyed cache entries.  Comparing all
        // entries with `all` is incorrect once a node has multiple keys: an
        // unchanged entry can mask a changed one.  The unconstrained entry is
        // the canonical value; absent means this is the first publication.
        let changed = self
            .metrics
            .get(&(id, ConstraintClass::Unconstrained.key()))
            .is_none_or(|existing| metrics_value_changed(*existing, metrics));
        let generation = if changed {
            metrics.generation.max(current_generation.saturating_add(1))
        } else {
            current_generation
        };
        metrics.generation = generation;
        self.metrics_generations.insert(id, generation);
        if changed {
            self.metrics.retain(|(node, _), _| *node != id);
            self.metric_entries.remove(&id);
            self.last_measure.remove(&id);
            self.last_measure_context.remove(&id);
            self.counters.intrinsic_generation_bumps =
                self.counters.intrinsic_generation_bumps.saturating_add(1);
            self.invalidate_result(id);
        }
        let key = ConstraintClass::Unconstrained.key();
        if changed || !self.metrics.contains_key(&(id, key)) {
            self.cache_metric(id, key, metrics);
        }
        changed
    }

    /// Measure once for a relevant constraint.  The context is intentionally
    /// absent from the key, so a context transition can reuse this entry.
    pub fn measure<F>(
        &mut self,
        id: LayoutNodeId,
        constraint: ConstraintClass,
        compute: F,
    ) -> Option<IntrinsicMetrics>
    where
        F: FnOnce() -> IntrinsicMetrics,
    {
        if !self.nodes.contains_key(&id) {
            return None;
        }
        self.counters.intrinsic_measure_requests =
            self.counters.intrinsic_measure_requests.saturating_add(1);
        let key = constraint.key();
        if let Some(value) = self.metrics.get(&(id, key)).copied() {
            self.counters.intrinsic_measure_cache_hits =
                self.counters.intrinsic_measure_cache_hits.saturating_add(1);
            let context = self.nodes.get(&id).map(|node| node.formatting_context);
            if context.is_some_and(|context| {
                self.last_measure_context
                    .get(&id)
                    .is_some_and(|previous| *previous != context)
            }) {
                self.counters.cross_context_measure_hits =
                    self.counters.cross_context_measure_hits.saturating_add(1);
            }
            self.last_measure.insert(
                id,
                MetricsEntry {
                    generation: value.generation,
                    key,
                },
            );
            if let Some(context) = context {
                self.last_measure_context.insert(id, context);
            }
            return Some(value);
        }
        self.counters.intrinsic_measure_cache_misses = self
            .counters
            .intrinsic_measure_cache_misses
            .saturating_add(1);
        if self.last_measure.contains_key(&id) {
            self.counters.cross_context_measure_misses =
                self.counters.cross_context_measure_misses.saturating_add(1);
        }
        let current_generation = self.metrics_generations.get(&id).copied().unwrap_or(0);
        let mut value = compute().sanitized();
        // A constrained measurement may not know the current canonical
        // generation. Never retain an older generation under a new key: that
        // would make the cache look fresh while its generation metadata is
        // stale relative to the node.
        value.generation = value.generation.max(current_generation);
        if value.generation > current_generation {
            self.metrics_generations.insert(id, value.generation);
        }
        if self.metric_budget == 0 {
            // Zero explicitly disables retention. Keep the miss observable,
            // but do not leave a private last-measure marker that would make
            // later uncached calls look like cross-context cache misses.
            self.last_measure.remove(&id);
            self.last_measure_context.remove(&id);
        } else {
            self.cache_metric(id, key, value);
            self.last_measure.insert(
                id,
                MetricsEntry {
                    generation: value.generation,
                    key,
                },
            );
            if let Some(node) = self.nodes.get(&id) {
                self.last_measure_context
                    .insert(id, node.formatting_context);
            }
        }
        Some(value)
    }

    pub fn metrics(
        &self,
        id: LayoutNodeId,
        constraint: ConstraintClass,
    ) -> Option<IntrinsicMetrics> {
        self.metrics.get(&(id, constraint.key())).copied()
    }

    pub fn metrics_generation(&self, id: LayoutNodeId) -> Option<u64> {
        self.metrics_generations.get(&id).copied()
    }

    /// Query a shared baseline instead of letting each formatting context
    /// invent an offset. `last` selects the last baseline when available and
    /// falls back to the first one.
    pub fn baseline(&mut self, id: LayoutNodeId, last: bool) -> Option<f32> {
        self.counters.record_baseline_query();
        let metrics = self
            .metrics
            .get(&(id, ConstraintClass::Unconstrained.key()))
            .copied()?;
        if last {
            metrics.last_baseline.or(metrics.first_baseline)
        } else {
            metrics.first_baseline.or(metrics.last_baseline)
        }
    }

    pub fn publish_result(&mut self, mut result: LayoutResult) -> bool {
        if !self.nodes.contains_key(&result.node) || !result.is_finite() {
            return false;
        }
        // A result publication is the boundary at which placement work is
        // observable.  Count the node and its retained dependency edges here
        // rather than in callers: both the runtime adapter and direct
        // Foundation users then report the same structural work.
        let previous_result = self.results.get(&result.node).cloned();
        let result_changed = previous_result.as_ref().is_none_or(|previous| {
            let mut previous = previous.clone();
            previous.generation = 0;
            let mut current = result.clone();
            current.generation = 0;
            previous != current
        });
        self.counters.record_placement(
            result
                .dependency_footprint
                .iter()
                .filter(|dependency| **dependency != result.node)
                .count(),
        );

        // Fragments are retained layout parts, so a changed bounds value does
        // not by itself require a new part. Match by node/kind in document
        // order and charge the unmatched current parts as creations.
        let previous_fragments = previous_result
            .as_ref()
            .map(|previous| previous.fragments.as_slice())
            .unwrap_or_default();
        let mut matched_previous = vec![false; previous_fragments.len()];
        let mut reused_fragments = 0;
        for fragment in &result.fragments {
            if let Some((index, _)) =
                previous_fragments
                    .iter()
                    .enumerate()
                    .find(|(index, previous)| {
                        !matched_previous[*index]
                            && previous.node == fragment.node
                            && previous.kind == fragment.kind
                    })
            {
                matched_previous[index] = true;
                reused_fragments += 1;
            }
        }
        self.counters.layout_fragments_reused = self
            .counters
            .layout_fragments_reused
            .saturating_add(reused_fragments);
        self.counters.layout_fragments_created = self
            .counters
            .layout_fragments_created
            .saturating_add(result.fragments.len().saturating_sub(reused_fragments));

        // Scroll offsets are presentation-only and are accounted by
        // `update_scroll_offset`. A published result for a scrolling node,
        // however, represents a real reflow; extent recomputation is charged
        // only when the extent actually changes (or on first publication).
        if result_changed
            && self
                .nodes
                .get(&result.node)
                .is_some_and(|node| node.behavior.scroll_x || node.behavior.scroll_y)
        {
            self.counters.scroll_layout_reflows =
                self.counters.scroll_layout_reflows.saturating_add(1);
            if previous_result
                .as_ref()
                .is_none_or(|previous| previous.scroll_extent != result.scroll_extent)
            {
                self.counters.scroll_content_extent_recomputes = self
                    .counters
                    .scroll_content_extent_recomputes
                    .saturating_add(1);
            }
            if previous_result
                .as_ref()
                .is_none_or(|previous| previous.clip != result.clip)
            {
                self.counters.scroll_clip_updates =
                    self.counters.scroll_clip_updates.saturating_add(1);
            }
        }
        if result_changed {
            self.generation = self.generation.saturating_add(1);
        }
        result.generation = self.generation;
        let replaced = self.results.insert(result.node, result).is_some();
        if replaced {
            self.counters.layout_results_reused =
                self.counters.layout_results_reused.saturating_add(1);
        } else {
            self.counters.layout_results_created =
                self.counters.layout_results_created.saturating_add(1);
        }
        true
    }

    pub fn result(&self, id: LayoutNodeId) -> Option<&LayoutResult> {
        self.results.get(&id)
    }

    pub fn results(&self) -> impl Iterator<Item = (&LayoutNodeId, &LayoutResult)> {
        self.results.iter()
    }

    /// Drop results for nodes that did not produce a box in the latest
    /// placement pass (`display:none`/`display:contents` included) while
    /// retaining all canonical identities and metrics.
    pub fn retain_results<I>(&mut self, ids: I)
    where
        I: IntoIterator<Item = LayoutNodeId>,
    {
        let live = ids.into_iter().collect::<HashSet<_>>();
        self.results.retain(|id, _| live.contains(id));
    }

    pub fn counters(&self) -> LayoutFoundationCounters {
        self.counters
    }

    /// Monotonic generation of published canonical results. Paint-only
    /// updates that do not republish geometry leave this value unchanged.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn take_counters(&mut self) -> LayoutFoundationCounters {
        std::mem::take(&mut self.counters)
    }

    pub fn clear_result(&mut self, id: LayoutNodeId) {
        self.results.remove(&id);
    }

    fn invalidate_result(&mut self, id: LayoutNodeId) {
        self.results.remove(&id);
    }

    fn cache_metric(&mut self, id: LayoutNodeId, key: ConstraintKey, value: IntrinsicMetrics) {
        if self.metric_budget == 0 {
            return;
        }
        if !self.metrics.contains_key(&(id, key)) && self.metrics.len() >= self.metric_budget {
            self.evict_one_metric();
        }
        self.metrics.insert((id, key), value);
        self.metric_entries.entry(id).or_default().insert(key);
    }

    fn evict_one_metric(&mut self) {
        let Some((node, key)) = self.metrics.keys().next().copied() else {
            return;
        };
        self.metrics.remove(&(node, key));
        if self
            .last_measure
            .get(&node)
            .is_some_and(|entry| entry.key == key)
        {
            self.last_measure.remove(&node);
            self.last_measure_context.remove(&node);
        }
        if let Some(keys) = self.metric_entries.get_mut(&node) {
            keys.remove(&key);
            if keys.is_empty() {
                self.metric_entries.remove(&node);
            }
        }
        self.counters.intrinsic_measure_cache_evictions = self
            .counters
            .intrinsic_measure_cache_evictions
            .saturating_add(1);
    }
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn finite_non_negative(value: f32) -> f32 {
    finite(value).max(0.0)
}

fn metrics_value_changed(mut existing: IntrinsicMetrics, mut incoming: IntrinsicMetrics) -> bool {
    existing.generation = 0;
    incoming.generation = 0;
    existing != incoming
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u64) -> LayoutNodeId {
        LayoutNodeId::new(value).unwrap()
    }

    fn intent() -> LayoutIntent {
        LayoutIntent::component_default(crate::LayoutOwnership::default())
    }

    #[test]
    fn context_transition_keeps_identity_children_and_participation_state() {
        let mut foundation = LayoutFoundation::new();
        let mut node = LayoutNode::new(id(1), intent(), FormattingContext::Flex);
        node.children = vec![id(2), id(3)];
        node.set_participation(Participation::Replaced(ReplacedContent {
            resource_generation: 7,
            ..ReplacedContent::default()
        }));
        assert!(foundation.insert(node));
        assert!(foundation.transition_context(id(1), FormattingContext::Grid));
        let node = foundation.node(id(1)).unwrap();
        assert_eq!(node.id, id(1));
        assert_eq!(node.children, vec![id(2), id(3)]);
        assert_eq!(node.participation.placement(), PlacementMode::NormalFlow);
        assert_eq!(node.context_generation, 1);
        assert_eq!(foundation.counters().layout_component_created, 0);
        assert_eq!(foundation.counters().layout_component_destroyed, 0);
    }

    #[test]
    fn upsert_preserves_orthogonal_placement_for_replaced_nodes() {
        let mut foundation = LayoutFoundation::new();
        let mut first = LayoutNode::new(id(1), intent(), FormattingContext::Flow);
        first.placement = PlacementMode::Absolute;
        first.participation = Participation::Replaced(ReplacedContent::default());
        assert!(foundation.insert(first));

        let mut second = LayoutNode::new(id(1), intent(), FormattingContext::Flow);
        second.placement = PlacementMode::NormalFlow;
        second.participation = Participation::Replaced(ReplacedContent {
            resource_generation: 2,
            ..ReplacedContent::default()
        });
        assert!(!foundation.upsert(second));
        let node = foundation.node(id(1)).unwrap();
        assert_eq!(node.placement, PlacementMode::NormalFlow);
        assert_eq!(
            node.participate(FormattingContext::Flow),
            Participation::Replaced(ReplacedContent {
                resource_generation: 2,
                ..ReplacedContent::default()
            })
        );
    }

    #[test]
    fn metrics_cache_is_shared_across_contexts_and_paint_does_not_bump_generation() {
        let mut foundation = LayoutFoundation::new();
        assert!(foundation.insert(LayoutNode::new(id(1), intent(), FormattingContext::Flex)));
        let mut calls = 0;
        let first = foundation
            .measure(id(1), ConstraintClass::ExactInline(120.0), || {
                calls += 1;
                IntrinsicMetrics::new(LayoutSize::new(120.0, 20.0)).with_generation(4)
            })
            .unwrap();
        foundation.transition_context(id(1), FormattingContext::Inline);
        let second = foundation
            .measure(id(1), ConstraintClass::ExactInline(120.0), || {
                calls += 1;
                IntrinsicMetrics::default()
            })
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(first, second);
        assert_eq!(foundation.counters().intrinsic_measure_cache_hits, 1);
        assert_eq!(foundation.counters().cross_context_measure_hits, 1);
    }

    #[test]
    fn result_is_finite_and_invalidated_by_context_or_children() {
        let mut foundation = LayoutFoundation::new();
        assert!(foundation.insert(LayoutNode::new(id(1), intent(), FormattingContext::Flex)));
        assert!(foundation.publish_result(LayoutResult::new(
            id(1),
            LayoutRect::new(0.0, 0.0, LayoutSize::new(20.0, 10.0)),
            0,
        )));
        assert!(foundation.result(id(1)).is_some());
        assert!(foundation.set_children(id(1), vec![id(2)]));
        assert!(foundation.result(id(1)).is_none());
        assert!(!foundation.publish_result(LayoutResult::new(
            id(1),
            LayoutRect {
                inline: f32::NAN,
                block: 0.0,
                size: LayoutSize::new(20.0, 10.0),
            },
            0,
        )));
        let mut invalid = LayoutResult::new(
            id(1),
            LayoutRect::new(0.0, 0.0, LayoutSize::new(20.0, 10.0)),
            0,
        );
        invalid.content_box.inline = f32::NAN;
        assert!(!foundation.publish_result(invalid));
    }

    #[test]
    fn display_facades_lower_to_parent_contexts() {
        assert_eq!(
            FormattingContext::from_display(DisplaySpec::Flex),
            FormattingContext::Flex
        );
        assert_eq!(
            FormattingContext::from_display(DisplaySpec::Grid),
            FormattingContext::Grid
        );
        assert_eq!(
            FormattingContext::from_display(DisplaySpec::Inline),
            FormattingContext::Inline
        );
        assert_eq!(
            FormattingContext::from_display(DisplaySpec::Block),
            FormattingContext::Flow
        );
        assert_eq!(
            FormattingContext::from_display(DisplaySpec::None),
            FormattingContext::None
        );
        assert!(!FormattingContext::from_display(DisplaySpec::Contents).generates_box());
        assert!(!FormattingContext::from_display(DisplaySpec::None).generates_box());
    }

    #[test]
    fn upsert_and_prune_are_the_only_lifecycle_changes() {
        let mut foundation = LayoutFoundation::new();
        assert!(foundation.ensure_node(id(1), intent(), FormattingContext::Flex));
        assert!(!foundation.upsert(LayoutNode::new(id(1), intent(), FormattingContext::Grid,)));
        assert_eq!(foundation.counters().layout_component_created, 0);
        assert_eq!(foundation.counters().layout_component_destroyed, 0);
        assert_eq!(foundation.counters().layout_context_transitions, 1);
        foundation.retain_nodes([id(1)]);
        assert!(foundation.node(id(1)).is_some());
        foundation.retain_nodes([]);
        assert!(foundation.node(id(1)).is_none());
        assert_eq!(foundation.counters().layout_component_destroyed, 1);
    }

    #[test]
    fn placement_and_behavior_are_orthogonal_to_context() {
        let mut node = LayoutNode::new(id(1), intent(), FormattingContext::Inline);
        node.placement = PlacementMode::Sticky;
        node.participation = Participation::NormalFlow;
        assert_eq!(
            node.participate(FormattingContext::Grid),
            Participation::GridItem
        );
        node.behavior = LayoutBehavior {
            scroll_x: true,
            scroll_y: false,
            clip: true,
            viewport: true,
            scroll_offset: LayoutSize::ZERO,
        };
        assert!(node.behavior.scroll_x && node.behavior.clip);
        assert_eq!(
            node.establish_formatting_context(),
            FormattingContext::Inline
        );
    }

    #[test]
    fn resource_frame_and_scroll_updates_do_not_remeasure() {
        let mut foundation = LayoutFoundation::new();
        let mut node = LayoutNode::new(id(1), intent(), FormattingContext::Flex);
        node.set_participation(Participation::Replaced(ReplacedContent {
            intrinsic_size: Some(LayoutSize::new(10.0, 10.0)),
            resource_generation: 1,
            ..ReplacedContent::default()
        }));
        assert!(foundation.insert(node));
        assert!(foundation.set_replaced_content(
            id(1),
            ReplacedContent {
                intrinsic_size: Some(LayoutSize::new(10.0, 10.0)),
                resource_generation: 2,
                ..ReplacedContent::default()
            },
            false,
        ));
        assert_eq!(foundation.counters().intrinsic_generation_bumps, 0);
        assert!(foundation.update_scroll_offset(id(1), LayoutSize::new(4.0, 2.0)));
        assert_eq!(foundation.counters().intrinsic_measure_requests, 0);
    }

    #[test]
    fn replaced_baseline_change_invalidates_the_published_result() {
        let mut foundation = LayoutFoundation::new();
        let mut node = LayoutNode::new(id(1), intent(), FormattingContext::Flow);
        node.participation = Participation::Replaced(ReplacedContent::default());
        assert!(foundation.insert(node));
        assert!(foundation.publish_result(LayoutResult::new(
            id(1),
            LayoutRect::new(0.0, 0.0, LayoutSize::new(20.0, 10.0)),
            0,
        )));
        assert!(foundation.set_replaced_content(
            id(1),
            ReplacedContent {
                baseline: BaselinePolicy::Center,
                ..ReplacedContent::default()
            },
            true,
        ));
        assert!(foundation.result(id(1)).is_none());
    }

    #[test]
    fn intrinsic_cache_budget_is_bounded_and_reports_evictions() {
        let mut foundation = LayoutFoundation::new();
        foundation.set_metric_budget(1);
        assert!(foundation.insert(LayoutNode::new(id(1), intent(), FormattingContext::Flex)));
        assert!(foundation.insert(LayoutNode::new(id(2), intent(), FormattingContext::Flex)));
        foundation.measure(id(1), ConstraintClass::Unconstrained, || {
            IntrinsicMetrics::new(LayoutSize::new(1.0, 1.0))
        });
        foundation.measure(id(2), ConstraintClass::Unconstrained, || {
            IntrinsicMetrics::new(LayoutSize::new(2.0, 2.0))
        });
        assert_eq!(foundation.metric_budget(), 1);
        assert_eq!(foundation.counters().intrinsic_measure_cache_evictions, 1);
    }

    #[test]
    fn unchanged_intrinsic_metrics_do_not_bump_generation() {
        let mut foundation = LayoutFoundation::new();
        assert!(foundation.insert(LayoutNode::new(id(1), intent(), FormattingContext::Flex)));
        let metrics = IntrinsicMetrics::new(LayoutSize::new(10.0, 4.0));
        assert!(foundation.set_metrics(id(1), metrics));
        let generation = foundation.metrics_generation(id(1));
        assert!(!foundation.set_metrics(id(1), metrics));
        assert_eq!(foundation.metrics_generation(id(1)), generation);
        assert_eq!(foundation.counters().intrinsic_generation_bumps, 1);
    }

    #[test]
    fn canonical_metrics_change_invalidates_all_constraint_keys() {
        let mut foundation = LayoutFoundation::new();
        assert!(foundation.insert(LayoutNode::new(id(1), intent(), FormattingContext::Flow)));
        let canonical = IntrinsicMetrics::new(LayoutSize::new(10.0, 4.0));
        assert!(foundation.set_metrics(id(1), canonical));
        let _ = foundation.measure(id(1), ConstraintClass::ExactInline(10.0), || {
            IntrinsicMetrics::new(LayoutSize::new(10.0, 4.0))
        });
        let changed = IntrinsicMetrics::new(LayoutSize::new(12.0, 4.0));
        assert!(foundation.set_metrics(id(1), changed));
        assert_eq!(
            foundation.metrics(id(1), ConstraintClass::ExactInline(10.0)),
            None
        );
        assert_eq!(foundation.counters().intrinsic_generation_bumps, 2);
    }

    #[test]
    fn non_finite_constraints_fail_closed_to_the_same_finite_cache_key() {
        let mut foundation = LayoutFoundation::new();
        assert!(foundation.insert(LayoutNode::new(id(1), intent(), FormattingContext::Flex)));
        let mut calls = 0;
        foundation.measure(id(1), ConstraintClass::MaxInline(f32::NAN), || {
            calls += 1;
            IntrinsicMetrics::new(LayoutSize::new(3.0, 2.0))
        });
        foundation.measure(id(1), ConstraintClass::MaxInline(0.0), || {
            calls += 1;
            IntrinsicMetrics::default()
        });
        assert_eq!(calls, 1);
    }

    #[test]
    fn intrinsic_and_rect_sanitizers_reject_non_finite_size_metadata() {
        let metrics = IntrinsicMetrics {
            min_inline: 4.0,
            max_inline: f32::INFINITY,
            min_block: 2.0,
            max_block: Some(f32::NAN),
            preferred: LayoutSize {
                inline: f32::NAN,
                block: -3.0,
            },
            ..IntrinsicMetrics::default()
        }
        .sanitized();
        assert_eq!(metrics.max_inline, 4.0);
        assert_eq!(metrics.max_block, Some(0.0));
        assert_eq!(metrics.preferred, LayoutSize::ZERO);

        let rect = LayoutRect::new(
            f32::NAN,
            f32::INFINITY,
            LayoutSize {
                inline: f32::NAN,
                block: -3.0,
            },
        );
        assert_eq!(rect.inline, 0.0);
        assert_eq!(rect.block, 0.0);
        assert_eq!(rect.size, LayoutSize::ZERO);
        assert!(rect.is_finite());
    }

    #[test]
    fn zero_metric_budget_does_not_retain_measure_markers() {
        let mut foundation = LayoutFoundation::new();
        foundation.set_metric_budget(0);
        assert!(foundation.insert(LayoutNode::new(id(1), intent(), FormattingContext::Flow)));
        let mut calls = 0;
        foundation.measure(id(1), ConstraintClass::Unconstrained, || {
            calls += 1;
            IntrinsicMetrics::new(LayoutSize::new(3.0, 2.0))
        });
        foundation.measure(id(1), ConstraintClass::Unconstrained, || {
            calls += 1;
            IntrinsicMetrics::new(LayoutSize::new(3.0, 2.0))
        });
        assert_eq!(calls, 2);
        assert_eq!(
            foundation.metrics(id(1), ConstraintClass::Unconstrained),
            None
        );
        assert_eq!(foundation.counters().cross_context_measure_misses, 0);
    }

    #[test]
    fn constrained_measurement_never_rewinds_canonical_generation() {
        let mut foundation = LayoutFoundation::new();
        assert!(foundation.insert(LayoutNode::new(id(1), intent(), FormattingContext::Flow)));
        assert!(foundation.set_metrics(
            id(1),
            IntrinsicMetrics::new(LayoutSize::new(8.0, 4.0)).with_generation(5),
        ));
        let measured = foundation
            .measure(id(1), ConstraintClass::ExactInline(8.0), || {
                IntrinsicMetrics::new(LayoutSize::new(8.0, 4.0)).with_generation(1)
            })
            .unwrap();
        assert_eq!(measured.generation, 5);
        assert_eq!(foundation.metrics_generation(id(1)), Some(5));
    }

    #[test]
    fn behavior_changes_invalidate_result_but_scroll_offsets_do_not() {
        let mut foundation = LayoutFoundation::new();
        assert!(foundation.insert(LayoutNode::new(id(1), intent(), FormattingContext::Flow)));
        assert!(foundation.publish_result(LayoutResult::new(
            id(1),
            LayoutRect::new(0.0, 0.0, LayoutSize::new(20.0, 10.0)),
            0,
        )));

        let mut clipped = LayoutNode::new(id(1), intent(), FormattingContext::Flow);
        clipped.behavior = LayoutBehavior {
            clip: true,
            viewport: true,
            ..LayoutBehavior::default()
        };
        assert!(!foundation.upsert(clipped));
        assert!(foundation.result(id(1)).is_none());

        assert!(foundation.publish_result(LayoutResult::new(
            id(1),
            LayoutRect::new(0.0, 0.0, LayoutSize::new(20.0, 10.0)),
            0,
        )));
        assert!(foundation.update_scroll_offset(id(1), LayoutSize::new(3.0, 2.0)));
        assert!(foundation.result(id(1)).is_some());
    }

    #[test]
    fn result_publication_counts_placement_dependencies_and_fragment_reuse() {
        let mut foundation = LayoutFoundation::new();
        assert!(foundation.insert(LayoutNode::new(id(1), intent(), FormattingContext::Flow,)));

        let bounds = LayoutRect::new(0.0, 0.0, LayoutSize::new(20.0, 10.0));
        let mut first = LayoutResult::new(id(1), bounds, 0);
        first.dependency_footprint = vec![id(1), id(2)];
        first.fragments.push(LayoutFragment {
            node: id(1),
            bounds,
            kind: FragmentKind::Box,
        });
        assert!(foundation.publish_result(first));
        let counters = foundation.counters();
        assert_eq!(counters.layout_nodes_placed, 1);
        assert_eq!(counters.layout_dependency_edges_visited, 1);
        assert_eq!(counters.layout_fragments_created, 1);
        assert_eq!(counters.layout_fragments_reused, 0);

        let mut second = LayoutResult::new(
            id(1),
            LayoutRect::new(0.0, 0.0, LayoutSize::new(24.0, 10.0)),
            0,
        );
        second.dependency_footprint = vec![id(1), id(2)];
        second.fragments.push(LayoutFragment {
            node: id(1),
            bounds: second.bounds,
            kind: FragmentKind::Box,
        });
        assert!(foundation.publish_result(second));
        let counters = foundation.counters();
        assert_eq!(counters.layout_nodes_placed, 2);
        assert_eq!(counters.layout_dependency_edges_visited, 2);
        assert_eq!(counters.layout_fragments_created, 1);
        assert_eq!(counters.layout_fragments_reused, 1);
    }

    #[test]
    fn scroll_result_work_is_distinct_from_scroll_offset_presentation_work() {
        let mut foundation = LayoutFoundation::new();
        let mut node = LayoutNode::new(id(1), intent(), FormattingContext::Flow);
        node.behavior = LayoutBehavior {
            scroll_x: true,
            clip: true,
            ..LayoutBehavior::default()
        };
        assert!(foundation.insert(node));

        let bounds = LayoutRect::new(0.0, 0.0, LayoutSize::new(20.0, 10.0));
        let mut first = LayoutResult::new(id(1), bounds, 0);
        first.scroll_extent = UsedSize::new(50.0, 10.0);
        first.clip = Some(bounds);
        assert!(foundation.publish_result(first));
        let counters = foundation.counters();
        assert_eq!(counters.scroll_layout_reflows, 1);
        assert_eq!(counters.scroll_content_extent_recomputes, 1);
        assert_eq!(counters.scroll_clip_updates, 1);

        assert!(foundation.update_scroll_offset(id(1), LayoutSize::new(2.0, 0.0)));
        let counters = foundation.counters();
        assert_eq!(counters.scroll_layout_reflows, 1);
        assert_eq!(counters.scroll_content_extent_recomputes, 1);
        assert_eq!(counters.scroll_clip_updates, 2);
    }
}
