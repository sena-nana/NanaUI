//! Dependency-aware dirty frontier construction for retained layout.
//!
//! The frontier is deliberately a per-pass work set.  It owns no tree and no
//! duplicate ancestor/descendant lists: callers provide the retained tree
//! relationships and receive the union of the work needed for this pass.

use std::collections::{HashMap, HashSet, VecDeque};

use nana_ui_core::{
    InvalidationKind, InvalidationReason, LayoutDependencyFootprint, LayoutInvalidation,
    LayoutInvalidationSource, LayoutMetricDelta,
};

use crate::StableNodeId;

/// A seed plus its node identity.  The contract crate keeps invalidation data
/// tree-independent; Runtime binds it to the retained node here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutFrontierSeed {
    pub node: StableNodeId,
    pub invalidation: LayoutInvalidation,
}

/// A dependency edge in a retained formatting context.  The graph stores
/// only local adjacency, so callers can describe parent constraints and
/// context-local coupling without materialising ancestor/descendant vectors
/// on every node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DependencyLink {
    target: StableNodeId,
    footprint: LayoutDependencyFootprint,
}

/// Bounded dependency index used by [`LayoutFrontier::from_dependency_graph`].
///
/// `add_parent_dependency(parent, child, footprint)` records both directions:
/// bottom-up metric export follows the parent link, while a parent constraint
/// change follows the child link. Context links are bidirectional and are
/// intended for flex/grid/inline sibling domains.
#[derive(Debug, Clone, Default)]
pub struct LayoutDependencyGraph {
    parents: HashMap<StableNodeId, Vec<DependencyLink>>,
    children: HashMap<StableNodeId, Vec<DependencyLink>>,
    contexts: HashMap<StableNodeId, Vec<DependencyLink>>,
    isolated: HashSet<StableNodeId>,
}

impl LayoutDependencyGraph {
    pub fn add_parent_dependency(
        &mut self,
        parent: StableNodeId,
        child: StableNodeId,
        footprint: LayoutDependencyFootprint,
    ) {
        self.add_parent_dependency_split(parent, child, footprint, footprint);
    }

    /// Add a parent/child edge with independent dependency footprints for the
    /// two propagation directions. A child metric export usually travels
    /// upward while a parent constraint is consumed downward; keeping those
    /// masks separate prevents a placement-only seed from widening both
    /// directions to the full subtree.
    pub fn add_parent_dependency_split(
        &mut self,
        parent: StableNodeId,
        child: StableNodeId,
        upward: LayoutDependencyFootprint,
        downward: LayoutDependencyFootprint,
    ) {
        Self::add_link(&mut self.parents, child, parent, upward);
        Self::add_link(&mut self.children, parent, child, downward);
    }

    /// Add a bidirectional edge for a local formatting-context domain (for
    /// example the siblings sharing a flex line or a grid track).
    pub fn add_context_dependency(
        &mut self,
        left: StableNodeId,
        right: StableNodeId,
        footprint: LayoutDependencyFootprint,
    ) {
        Self::add_link(&mut self.contexts, left, right, footprint);
        Self::add_link(&mut self.contexts, right, left, footprint);
    }

    /// Add a one-way context edge for block-flow sibling prefixes.
    pub fn add_context_dependency_forward(
        &mut self,
        source: StableNodeId,
        target: StableNodeId,
        footprint: LayoutDependencyFootprint,
    ) {
        Self::add_link(&mut self.contexts, source, target, footprint);
    }

    pub fn isolate(&mut self, node: StableNodeId) {
        self.isolated.insert(node);
    }

    fn parents(&self, node: StableNodeId) -> &[DependencyLink] {
        self.parents.get(&node).map(Vec::as_slice).unwrap_or(&[])
    }

    fn children(&self, node: StableNodeId) -> &[DependencyLink] {
        self.children.get(&node).map(Vec::as_slice).unwrap_or(&[])
    }

    fn contexts(&self, node: StableNodeId) -> &[DependencyLink] {
        self.contexts.get(&node).map(Vec::as_slice).unwrap_or(&[])
    }

    fn is_isolated(&self, node: StableNodeId) -> bool {
        self.isolated.contains(&node)
    }

    fn add_link(
        links: &mut HashMap<StableNodeId, Vec<DependencyLink>>,
        source: StableNodeId,
        target: StableNodeId,
        footprint: LayoutDependencyFootprint,
    ) {
        let entries = links.entry(source).or_default();
        if let Some(existing) = entries.iter_mut().find(|link| link.target == target) {
            existing.footprint = existing.footprint.union(footprint);
        } else {
            entries.push(DependencyLink { target, footprint });
        }
    }
}

impl LayoutFrontierSeed {
    pub const fn new(node: StableNodeId, invalidation: LayoutInvalidation) -> Self {
        Self { node, invalidation }
    }

    pub const fn layout(node: StableNodeId) -> Self {
        Self::new(
            node,
            LayoutInvalidation::new(
                LayoutInvalidationSource::Runtime,
                InvalidationReason::STYLE,
                InvalidationKind::MEASURE.union(InvalidationKind::PLACEMENT),
                nana_ui_core::LayoutFieldMask::ALL,
                LayoutDependencyFootprint::ALL,
            ),
        )
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct FrontierEntry {
    invalidation: LayoutInvalidation,
    measure: bool,
    placement: bool,
    context: bool,
    writing: bool,
    scroll: bool,
    topology: bool,
}

impl FrontierEntry {
    fn from_invalidation(invalidation: LayoutInvalidation) -> Self {
        let kind = invalidation.kind;
        Self {
            invalidation,
            measure: kind.intersects(InvalidationKind::MEASURE.union(InvalidationKind::TOPOLOGY)),
            placement: kind
                .intersects(InvalidationKind::PLACEMENT.union(InvalidationKind::TOPOLOGY)),
            context: kind.intersects(InvalidationKind::CONTEXT_REFLOW),
            writing: kind.intersects(InvalidationKind::WRITING_CONTEXT),
            scroll: kind.intersects(InvalidationKind::SCROLL_OVERFLOW),
            topology: kind.intersects(InvalidationKind::TOPOLOGY),
        }
    }

    fn merge(&mut self, invalidation: LayoutInvalidation) -> bool {
        // Reasons/source/changed-inputs are diagnostic metadata. They must be
        // retained on the entry, but changing only that metadata must not
        // trigger another ancestor walk: the same dependency class is already
        // represented in this frame. Propagation is needed only when a new
        // work kind or dependency footprint appears.
        let before_kind = self.invalidation.kind;
        let before_axes = self.invalidation.affected_axes;
        let before = (
            self.measure,
            self.placement,
            self.context,
            self.writing,
            self.scroll,
            self.topology,
        );
        self.invalidation = self.invalidation.merge(invalidation);
        let next = Self::from_invalidation(self.invalidation);
        self.measure |= next.measure;
        self.placement |= next.placement;
        self.context |= next.context;
        self.writing |= next.writing;
        self.scroll |= next.scroll;
        self.topology |= next.topology;
        let dependency_class_changed =
            before_kind != self.invalidation.kind || before_axes != self.invalidation.affected_axes;
        before
            != (
                self.measure,
                self.placement,
                self.context,
                self.writing,
                self.scroll,
                self.topology,
            )
            || dependency_class_changed
    }

    fn union_kind(self) -> InvalidationKind {
        let mut kind = InvalidationKind::NONE;
        if self.measure {
            kind = kind.union(InvalidationKind::MEASURE);
        }
        if self.placement {
            kind = kind.union(InvalidationKind::PLACEMENT);
        }
        if self.context {
            kind = kind.union(InvalidationKind::CONTEXT_REFLOW);
        }
        if self.writing {
            kind = kind.union(InvalidationKind::WRITING_CONTEXT);
        }
        if self.scroll {
            kind = kind.union(InvalidationKind::SCROLL_OVERFLOW);
        }
        if self.topology {
            kind = kind.union(InvalidationKind::TOPOLOGY);
        }
        kind
    }
}

/// Per-pass union of measure, placement, context, writing and scroll work.
#[derive(Debug, Clone, Default)]
pub struct LayoutFrontier {
    entries: HashMap<StableNodeId, FrontierEntry>,
    all: HashSet<StableNodeId>,
    measure: HashSet<StableNodeId>,
    placement: HashSet<StableNodeId>,
    contexts: HashSet<StableNodeId>,
    writing: HashSet<StableNodeId>,
    scroll: HashSet<StableNodeId>,
    seeds: usize,
    seed_merges: usize,
    dependency_edges_visited: usize,
    propagations_stopped: usize,
    local_subtree_fallbacks: usize,
    full_document_fallbacks: usize,
}

/// Compact snapshot copied onto the retained document cache after a pass.
/// Keeping the snapshot separate from the hash sets lets frame diagnostics
/// consume counters without retaining the frontier's scratch maps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayoutFrontierStats {
    pub seeds: usize,
    pub seed_merges: usize,
    pub nodes_measure: usize,
    pub nodes_placement: usize,
    pub contexts: usize,
    pub dependency_edges_visited: usize,
    pub propagations_stopped: usize,
    pub local_subtree_fallbacks: usize,
    pub full_document_fallbacks: usize,
}

impl LayoutFrontierStats {
    pub fn from_frontier(frontier: &LayoutFrontier) -> Self {
        Self {
            seeds: frontier.seeds(),
            seed_merges: frontier.seed_merges(),
            nodes_measure: frontier.measure_nodes().len(),
            nodes_placement: frontier.placement_nodes().len(),
            contexts: frontier.context_nodes().len(),
            dependency_edges_visited: frontier.dependency_edges_visited(),
            propagations_stopped: frontier.propagations_stopped(),
            local_subtree_fallbacks: frontier.local_subtree_fallbacks(),
            full_document_fallbacks: frontier.full_document_fallbacks(),
        }
    }
}

/// Child edges add measure when the carried footprint consumes a parent
/// constraint. Context edges place the neighbour and add measure only by
/// leaving an existing measure bit in place.
fn descend_kind(
    kind: InvalidationKind,
    measure_edge: bool,
    context_edge: bool,
) -> InvalidationKind {
    let mut kind = kind.union(InvalidationKind::PLACEMENT);
    if context_edge {
        kind = kind.union(InvalidationKind::CONTEXT_REFLOW);
    }
    if measure_edge && !context_edge {
        kind = kind.union(InvalidationKind::MEASURE);
    } else if !measure_edge {
        kind = kind.without(InvalidationKind::MEASURE);
    }
    kind
}

impl LayoutFrontier {
    /// Build the bottom-up ancestor frontier.  `parent_of` and `is_isolated`
    /// are O(1) retained-tree queries.  A shared edge is visited once per
    /// newly-added work class, so a batch does not perform repeated root walks.
    pub fn from_seeds<I, P, Iso>(seeds: I, mut parent_of: P, mut is_isolated: Iso) -> Self
    where
        I: IntoIterator<Item = LayoutFrontierSeed>,
        P: FnMut(StableNodeId) -> Option<StableNodeId>,
        Iso: FnMut(StableNodeId) -> bool,
    {
        let mut frontier = Self::default();
        for seed in seeds {
            if seed.invalidation.is_empty() {
                frontier.propagations_stopped = frontier.propagations_stopped.saturating_add(1);
                continue;
            }
            frontier.add_seed(seed, &mut parent_of, &mut is_isolated);
        }
        frontier
    }

    /// Build a frontier from explicit parent, descendant and context-local
    /// dependency edges. This is the typed path for parent-constraint,
    /// descendant, and sibling-prefix invalidations.
    pub fn from_dependency_graph<I>(seeds: I, graph: &LayoutDependencyGraph) -> Self
    where
        I: IntoIterator<Item = LayoutFrontierSeed>,
    {
        let mut frontier = Self::default();
        let mut pending = VecDeque::new();
        let seeds = seeds.into_iter().collect::<Vec<_>>();
        // An isolation boundary stops metrics exported by descendants, but a
        // seed on the isolated box itself (for example, a fixed island whose
        // own size changed) still has to reach its parent's placement. Keep
        // this compact set so the distinction does not widen descendant
        // propagation.
        let direct_seeds = seeds.iter().map(|seed| seed.node).collect::<HashSet<_>>();
        for seed in seeds {
            if seed.invalidation.is_empty() {
                frontier.propagations_stopped = frontier.propagations_stopped.saturating_add(1);
                continue;
            }
            frontier.seeds = frontier.seeds.saturating_add(1);
            let existed = frontier.entries.contains_key(&seed.node);
            let changed = frontier.insert_entry(seed.node, seed.invalidation);
            frontier.refresh_sets(seed.node);
            if existed {
                frontier.seed_merges = frontier.seed_merges.saturating_add(1);
            }
            if changed {
                pending.push_back((seed.node, seed.invalidation));
            } else {
                frontier.propagations_stopped = frontier.propagations_stopped.saturating_add(1);
            }
        }

        // Each queue item represents one merged dependency class. A class is
        // visited once, while distinct metric/context classes can still share
        // the same node and edge safely.
        // Parent-constraint and writing edges measure. Sibling-prefix and
        // origin movement only place.
        let measure_down = LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT
            .union(LayoutDependencyFootprint::CONSUMES_PARENT_BLOCK_CONSTRAINT)
            .union(LayoutDependencyFootprint::DEPENDS_ON_CONTAINING_BLOCK)
            .union(LayoutDependencyFootprint::DEPENDS_ON_WRITING_CONTEXT);
        let mut visited = HashSet::new();
        while let Some((node, invalidation)) = pending.pop_front() {
            let key = (
                node,
                invalidation.kind.bits(),
                invalidation.affected_axes.bits(),
            );
            if !visited.insert(key) {
                continue;
            }
            let axes = invalidation.affected_axes;
            // Topology changes invalidate the edge set itself. A producer may
            // provide a narrow footprint, while the retained graph still has
            // to visit every known direction; this keeps externally
            // constructed topology seeds safe too.
            let follow_all = axes == LayoutDependencyFootprint::ALL
                || invalidation.kind.intersects(InvalidationKind::TOPOLOGY);
            let follow_up = invalidation.kind.intersects(
                InvalidationKind::MEASURE
                    .union(InvalidationKind::PLACEMENT)
                    .union(InvalidationKind::CONTEXT_REFLOW)
                    .union(InvalidationKind::WRITING_CONTEXT)
                    .union(InvalidationKind::SCROLL_OVERFLOW)
                    .union(InvalidationKind::TOPOLOGY),
            );
            // An isolated formatting context owns its exported metrics. It
            // can still receive descendant/context work below, but a seed
            // inside it must not escape through a parent edge.
            if follow_up && (!graph.is_isolated(node) || direct_seeds.contains(&node)) {
                for link in graph.parents(node) {
                    frontier.dependency_edges_visited =
                        frontier.dependency_edges_visited.saturating_add(1);
                    // A NONE edge is a retained traversal path through a
                    // fixed-size ancestor. It keeps the layout walk rooted
                    // without claiming that the ancestor must be measured.
                    let structural_path = link.footprint.is_empty();
                    if !follow_all && !structural_path && !axes.intersects(link.footprint) {
                        continue;
                    }
                    let mut parent_invalidation = invalidation;
                    if structural_path {
                        parent_invalidation.kind = InvalidationKind::PLACEMENT;
                        parent_invalidation.affected_axes = LayoutDependencyFootprint::NONE;
                    }
                    // A child may consume several dependency classes, while
                    // this particular parent edge exports only a subset. Do
                    // not carry unrelated classes across the boundary: a
                    // fixed row's lateral placement dependency must not turn
                    // into a block-constraint seed that expands every sibling
                    // below the outer container.
                    if !follow_all && !structural_path {
                        let excluded = LayoutDependencyFootprint::ALL.without(link.footprint);
                        parent_invalidation.affected_axes = axes.without(excluded);
                        let metric_edges = LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE
                            .union(LayoutDependencyFootprint::EXPORTS_INTRINSIC_BLOCK)
                            .union(LayoutDependencyFootprint::EXPORTS_BASELINE)
                            .union(LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS);
                        if !link.footprint.intersects(metric_edges) {
                            parent_invalidation.kind =
                                parent_invalidation.kind.without(InvalidationKind::MEASURE);
                        }
                    }
                    if !invalidation.kind.intersects(InvalidationKind::MEASURE)
                        && !invalidation.kind.intersects(
                            InvalidationKind::CONTEXT_REFLOW
                                .union(InvalidationKind::WRITING_CONTEXT),
                        )
                        && !invalidation
                            .kind
                            .intersects(InvalidationKind::SCROLL_OVERFLOW)
                        && !invalidation.kind.intersects(InvalidationKind::TOPOLOGY)
                    {
                        parent_invalidation.kind = InvalidationKind::PLACEMENT;
                    }
                    // A lateral-only edge through a fixed-size ancestor
                    // replays that ancestor's child placement but does not
                    // invalidate its own intrinsic measurement. Preserve the
                    // narrow edge classification so retained measure plans
                    // remain reusable.
                    if !link.footprint.intersects(
                        LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE
                            .union(LayoutDependencyFootprint::EXPORTS_INTRINSIC_BLOCK)
                            .union(LayoutDependencyFootprint::EXPORTS_BASELINE)
                            .union(LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS),
                    ) && !invalidation.kind.intersects(InvalidationKind::TOPOLOGY)
                    {
                        parent_invalidation.kind = InvalidationKind::PLACEMENT;
                    }
                    let existed = frontier.entries.contains_key(&link.target);
                    let changed = frontier.insert_entry(link.target, parent_invalidation);
                    if existed {
                        frontier.seed_merges = frontier.seed_merges.saturating_add(1);
                    }
                    frontier.refresh_sets(link.target);
                    if changed && !graph.is_isolated(link.target) {
                        pending.push_back((link.target, parent_invalidation));
                    } else {
                        frontier.propagations_stopped =
                            frontier.propagations_stopped.saturating_add(1);
                    }
                }
            }

            let follow_down = axes.intersects(
                measure_down
                    .union(LayoutDependencyFootprint::DEPENDS_ON_SIBLING_PREFIX)
                    .union(LayoutDependencyFootprint::CONTEXT_LOCAL_COUPLING),
            ) || follow_all;
            if follow_down {
                for link in graph.children(node) {
                    frontier.dependency_edges_visited =
                        frontier.dependency_edges_visited.saturating_add(1);
                    if !follow_all && !axes.intersects(link.footprint) {
                        continue;
                    }
                    let carried = if follow_all {
                        link.footprint
                    } else {
                        axes.intersection(link.footprint)
                    };
                    let measure_edge = follow_all || carried.intersects(measure_down);
                    let mut child_invalidation = invalidation;
                    child_invalidation.kind =
                        descend_kind(child_invalidation.kind, measure_edge, false);
                    if !follow_all {
                        child_invalidation.affected_axes = carried;
                    }
                    if carried.intersects(LayoutDependencyFootprint::DEPENDS_ON_WRITING_CONTEXT)
                        || (follow_all
                            && axes
                                .intersects(LayoutDependencyFootprint::DEPENDS_ON_WRITING_CONTEXT))
                    {
                        child_invalidation.kind = child_invalidation
                            .kind
                            .union(InvalidationKind::WRITING_CONTEXT);
                    }
                    let existed = frontier.entries.contains_key(&link.target);
                    let changed = frontier.insert_entry(link.target, child_invalidation);
                    if existed {
                        frontier.seed_merges = frontier.seed_merges.saturating_add(1);
                    }
                    frontier.refresh_sets(link.target);
                    if changed {
                        pending.push_back((link.target, child_invalidation));
                    } else {
                        frontier.propagations_stopped =
                            frontier.propagations_stopped.saturating_add(1);
                    }
                }
            }

            let follow_context = axes.intersects(
                LayoutDependencyFootprint::DEPENDS_ON_SIBLING_PREFIX
                    .union(LayoutDependencyFootprint::CONTEXT_LOCAL_COUPLING),
            ) || follow_all;
            if follow_context {
                for link in graph.contexts(node) {
                    frontier.dependency_edges_visited =
                        frontier.dependency_edges_visited.saturating_add(1);
                    if !follow_all && !axes.intersects(link.footprint) {
                        continue;
                    }
                    let carried = if follow_all {
                        link.footprint
                    } else {
                        axes.intersection(link.footprint)
                    };
                    let mut context_invalidation = invalidation;
                    let measure_edge = follow_all || carried.intersects(measure_down);
                    context_invalidation.kind =
                        descend_kind(context_invalidation.kind, measure_edge, true);
                    if !follow_all {
                        context_invalidation.affected_axes = carried;
                    }
                    let existed = frontier.entries.contains_key(&link.target);
                    let changed = frontier.insert_entry(link.target, context_invalidation);
                    if existed {
                        frontier.seed_merges = frontier.seed_merges.saturating_add(1);
                    }
                    frontier.refresh_sets(link.target);
                    if changed {
                        pending.push_back((link.target, context_invalidation));
                    } else {
                        frontier.propagations_stopped =
                            frontier.propagations_stopped.saturating_add(1);
                    }
                }
            }
        }
        frontier
    }

    fn insert_entry(&mut self, node: StableNodeId, invalidation: LayoutInvalidation) -> bool {
        self.entries.entry(node).or_default().merge(invalidation)
    }

    fn add_seed<P, Iso>(
        &mut self,
        seed: LayoutFrontierSeed,
        parent_of: &mut P,
        is_isolated: &mut Iso,
    ) where
        P: FnMut(StableNodeId) -> Option<StableNodeId>,
        Iso: FnMut(StableNodeId) -> bool,
    {
        self.seeds = self.seeds.saturating_add(1);
        let already_seeded = self.entries.contains_key(&seed.node);
        let entry = self.entries.entry(seed.node).or_default();
        if already_seeded {
            self.seed_merges = self.seed_merges.saturating_add(1);
        }
        let seed_changed = entry.merge(seed.invalidation);
        self.refresh_sets(seed.node);
        // An identical seed has no new dependency class to carry upward.  It
        // is already represented by the existing node entry, so stop here
        // instead of paying one redundant parent-edge visit per duplicate.
        if already_seeded && !seed_changed {
            self.propagations_stopped = self.propagations_stopped.saturating_add(1);
            return;
        }
        // An established layout-isolation boundary owns its subtree. The
        // seed itself remains in the frontier, but its exported metrics do
        // not invalidate ancestors outside that boundary.
        if is_isolated(seed.node) {
            self.propagations_stopped = self.propagations_stopped.saturating_add(1);
            return;
        }

        let mut cursor = parent_of(seed.node);
        while let Some(parent) = cursor {
            self.dependency_edges_visited = self.dependency_edges_visited.saturating_add(1);
            let mut parent_invalidation = seed.invalidation;
            // An ancestor has to place its affected child.  It only needs a
            // fresh measure when the seed exports a metric or the context is
            // explicitly coupled.  This is the key distinction between a
            // placement-only delta and a measure frontier.
            if !seed.invalidation.kind.intersects(InvalidationKind::MEASURE)
                && !seed.invalidation.kind.intersects(
                    InvalidationKind::CONTEXT_REFLOW.union(InvalidationKind::WRITING_CONTEXT),
                )
                && !seed
                    .invalidation
                    .kind
                    .intersects(InvalidationKind::SCROLL_OVERFLOW)
                && !seed
                    .invalidation
                    .kind
                    .intersects(InvalidationKind::TOPOLOGY)
            {
                parent_invalidation.kind = InvalidationKind::PLACEMENT;
            }
            let parent_was_present = self.entries.contains_key(&parent);
            let existing = self.entries.entry(parent).or_default();
            let added = existing.merge(parent_invalidation);
            if parent_was_present {
                self.seed_merges = self.seed_merges.saturating_add(1);
            }
            self.refresh_sets(parent);
            if !added {
                self.propagations_stopped = self.propagations_stopped.saturating_add(1);
                break;
            }
            if is_isolated(parent) {
                self.propagations_stopped = self.propagations_stopped.saturating_add(1);
                break;
            }
            cursor = parent_of(parent);
        }
    }

    fn refresh_sets(&mut self, node: StableNodeId) {
        let Some(entry) = self.entries.get(&node).copied() else {
            return;
        };
        self.all.insert(node);
        if entry.measure {
            self.measure.insert(node);
        }
        if entry.placement {
            self.placement.insert(node);
        }
        if entry.context {
            self.contexts.insert(node);
        }
        if entry.writing {
            self.writing.insert(node);
        }
        if entry.scroll {
            self.scroll.insert(node);
        }
    }

    /// Seed the frontier from an exported metric result.  A `None` delta is a
    /// valid stable boundary and deliberately does not walk ancestors.
    pub fn propagate_metric_delta<P, Iso>(
        &mut self,
        node: StableNodeId,
        delta: LayoutMetricDelta,
        mut parent_of: P,
        mut is_isolated: Iso,
    ) where
        P: FnMut(StableNodeId) -> Option<StableNodeId>,
        Iso: FnMut(StableNodeId) -> bool,
    {
        if delta.is_none() {
            self.propagations_stopped = self.propagations_stopped.saturating_add(1);
            return;
        }
        let mut kind = InvalidationKind::PLACEMENT;
        if delta.propagates_measure() {
            kind = kind.union(InvalidationKind::MEASURE);
        }
        if delta.intersects(LayoutMetricDelta::WRITING_CONTEXT) {
            kind = kind.union(InvalidationKind::WRITING_CONTEXT);
        }
        if delta.intersects(LayoutMetricDelta::SCROLL_EXTENT.union(LayoutMetricDelta::OVERFLOW)) {
            kind = kind.union(InvalidationKind::SCROLL_OVERFLOW);
        }
        self.add_seed(
            LayoutFrontierSeed::new(
                node,
                LayoutInvalidation::new(
                    LayoutInvalidationSource::Runtime,
                    InvalidationReason::UNKNOWN,
                    kind,
                    nana_ui_core::LayoutFieldMask::INTRINSIC,
                    delta.affected_footprint(),
                ),
            ),
            &mut parent_of,
            &mut is_isolated,
        );
    }

    pub fn contains(&self, node: StableNodeId) -> bool {
        self.all.contains(&node)
    }

    pub fn nodes(&self) -> &HashSet<StableNodeId> {
        &self.all
    }

    pub fn measure_nodes(&self) -> &HashSet<StableNodeId> {
        &self.measure
    }

    pub fn placement_nodes(&self) -> &HashSet<StableNodeId> {
        &self.placement
    }

    pub fn context_nodes(&self) -> &HashSet<StableNodeId> {
        &self.contexts
    }

    pub fn writing_nodes(&self) -> &HashSet<StableNodeId> {
        &self.writing
    }

    pub fn scroll_nodes(&self) -> &HashSet<StableNodeId> {
        &self.scroll
    }

    pub fn invalidation(&self, node: StableNodeId) -> Option<LayoutInvalidation> {
        self.entries.get(&node).map(|entry| {
            LayoutInvalidation::new(
                entry.invalidation.source,
                entry.invalidation.reason,
                entry.union_kind(),
                entry.invalidation.changed_inputs,
                entry.invalidation.affected_axes,
            )
        })
    }

    pub fn seeds(&self) -> usize {
        self.seeds
    }
    pub fn seed_merges(&self) -> usize {
        self.seed_merges
    }
    pub fn dependency_edges_visited(&self) -> usize {
        self.dependency_edges_visited
    }
    pub fn propagations_stopped(&self) -> usize {
        self.propagations_stopped
    }
    pub fn local_subtree_fallbacks(&self) -> usize {
        self.local_subtree_fallbacks
    }
    pub fn full_document_fallbacks(&self) -> usize {
        self.full_document_fallbacks
    }
}
