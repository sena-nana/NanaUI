//! Issue #265: responsive rules read their container's size through an
//! index and move layout only by what their variants change.
//!
//! A page of filler groups ends in a panel 4000 px tall whose width the test
//! sets. A hundred rows sit in the panel, each 200x24 and each following a
//! rule on the panel's inline size: ten turn 36 px tall below 500 px, the
//! other ninety below 200 px.
//!
//! - Gate A: the panel's width moves: the hundred rules are evaluated and no
//!   other node is, the same at 10k and at 100k nodes.
//! - Gate B: 240 widths in one bucket: every rule is evaluated, none changes,
//!   and nothing is laid out for them.
//! - Gate C: the width crosses 500: the ten rules that break there change
//!   and seed one invalidation each; the ninety fixed rows measure nothing.
//! - Gate D: eight containers nested, each with fifty fixed leaves, each
//!   tightening its padding below 400 px of its parent: a resize evaluates
//!   the seven rules and no leaf, converges in at most four rounds, needs no
//!   fallback, and matches the same page laid out cold.
//! - Gate E: the index holds what was registered, and a frame's history is
//!   gone by the next frame.
//!
//! A rule whose variant moves its own container back across its breakpoint
//! settles in its frame: it keeps the bucket it reached, deterministically.

#![cfg(test)]

use std::sync::Arc;

use nana_ui_core::{ContainerType, FlexDirection, LayoutStyle, LengthSpec, WorkCounters};

use super::reflow_oracle::{assert_matches_cold, bundled_face_shaper, node, product_frame, styled};
use super::{DocumentId, NodeKind, StableNodeId};
use crate::layout_engine::verify::skip_layout_verify;
use crate::{
    AppContext, LayoutViewport, MutationQueue, NanaTextEngineShaper, NodeStyle, ResponsiveAxis,
    ResponsiveContainer, ResponsiveRule, StyleVariant,
};

pub(super) const RULES: u64 = 100;
const PANEL: u64 = 10_000_000;

fn viewport() -> LayoutViewport {
    LayoutViewport::new(1200.0, 800.0)
}

fn element(
    queue: &mut MutationQueue,
    document: DocumentId,
    parent: StableNodeId,
    id: StableNodeId,
    style: NodeStyle,
) {
    queue.create(id, document, NodeKind::Element { tag: "div".into() });
    queue.insert(parent, id, None);
    queue.set_style(id, style);
}

fn column(width: Option<f32>, height: Option<f32>) -> LayoutStyle {
    LayoutStyle {
        width: width.map(LengthSpec::Px),
        height: height.map(LengthSpec::Px),
        direction: Some(FlexDirection::Column),
        flex_shrink: Some(0.0),
        ..LayoutStyle::default()
    }
}

fn fixed(width: f32, height: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        flex_shrink: Some(0.0),
        ..LayoutStyle::default()
    }
}

/// The page under the document, and groups of a hundred fixed rows filling
/// it to about `nodes` nodes, numbered from `first`. The page holds groups,
/// not a hundred thousand children of its own.
fn page_with_filler(queue: &mut MutationQueue, document: DocumentId, nodes: u64, first: u64) {
    queue.create(node(1), document, NodeKind::Document);
    element(
        queue,
        document,
        node(1),
        node(2),
        styled(column(Some(1200.0), None)),
    );
    let groups = (nodes.saturating_sub(RULES + 3) / 100).max(1);
    let mut next = first;
    for _ in 0..groups {
        let group = node(next);
        next += 1;
        element(queue, document, node(2), group, styled(column(None, None)));
        for _ in 0..99 {
            element(
                queue,
                document,
                group,
                node(next),
                styled(fixed(100.0, 4.0)),
            );
            next += 1;
        }
    }
}

/// Row `index` of the panel.
pub(super) fn row(index: u64) -> StableNodeId {
    node(PANEL + 1 + index)
}

/// Where row `index` breaks: every tenth row at 500, the rest at 200.
pub(super) fn breakpoint(index: u64) -> f32 {
    if index.is_multiple_of(10) {
        500.0
    } else {
        200.0
    }
}

fn row_rule(index: u64) -> Arc<ResponsiveRule> {
    Arc::new(
        ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
            .below(breakpoint(index), |layout| {
                layout.height = Some(LengthSpec::Px(36.0))
            }),
    )
}

/// What responsive rules cost one frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Queries {
    pub(super) size_changes: usize,
    pub(super) evaluated: usize,
    pub(super) changed: usize,
    pub(super) unchanged: usize,
    pub(super) downstream: usize,
    pub(super) rounds: usize,
    pub(super) fallbacks: usize,
}

impl From<WorkCounters> for Queries {
    fn from(counters: WorkCounters) -> Self {
        Self {
            size_changes: counters.container_query_size_changes,
            evaluated: counters.container_query_rules_evaluated,
            changed: counters.container_query_results_changed,
            unchanged: counters.container_query_results_unchanged,
            downstream: counters.container_query_downstream_invalidations,
            rounds: counters.container_query_convergence_rounds,
            fallbacks: counters.container_query_cycle_fallbacks,
        }
    }
}

/// A page of about `nodes` nodes ending in a box whose width the test sets.
pub(super) struct Panel {
    pub(super) context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    guarded: bool,
    /// The box [`Self::resize`] restyles, and its layout at a width.
    resized: StableNodeId,
    layout_at: fn(f32) -> LayoutStyle,
}

impl Panel {
    /// The page ending in the panel of a hundred rows.
    pub(super) fn new(nodes: u64, width: f32, guarded: bool) -> Self {
        let document = DocumentId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        page_with_filler(&mut queue, document, nodes, 10);
        // The panel ends the page and keeps its height: a row that grows
        // moves nothing outside it.
        element(
            &mut queue,
            document,
            node(2),
            node(PANEL),
            styled(column(Some(width), Some(4000.0))),
        );
        for index in 0..RULES {
            element(
                &mut queue,
                document,
                node(PANEL),
                row(index),
                styled(fixed(200.0, 24.0)),
            );
            queue.set_responsive(row(index), Some(row_rule(index)));
        }
        Self::laid_out(document, queue, guarded, node(PANEL), |width| {
            column(Some(width), Some(4000.0))
        })
    }

    /// `queue` committed to a new context and laid out once.
    fn laid_out(
        document: DocumentId,
        queue: MutationQueue,
        guarded: bool,
        resized: StableNodeId,
        layout_at: fn(f32) -> LayoutStyle,
    ) -> Self {
        let mut context = AppContext::new();
        context.compat_world_mut().commit(queue).unwrap();
        let mut panel = Self {
            context,
            document,
            shaper: bundled_face_shaper(),
            guarded,
            resized,
            layout_at,
        };
        panel.frame();
        panel
    }

    pub(super) fn frame(&mut self) -> WorkCounters {
        let _unguarded = (!self.guarded).then(skip_layout_verify);
        product_frame(
            &mut self.context,
            self.document,
            viewport(),
            &mut self.shaper,
        )
    }

    /// One frame at `width`.
    pub(super) fn resize(&mut self, width: f32) -> WorkCounters {
        let mut queue = MutationQueue::new();
        queue.set_style(self.resized, styled((self.layout_at)(width)));
        self.context.commit_mutations(queue).unwrap();
        self.frame()
    }

    fn height(&self, index: u64) -> f32 {
        self.context.world().layout_box(row(index)).unwrap().height
    }

    fn footprint(&self) -> (usize, usize, usize, usize) {
        self.context.world().responsive.footprint()
    }
}

#[test]
fn issue265_gate_a_a_container_resize_evaluates_its_rules_and_nothing_else() {
    let mut costs = Vec::new();
    for nodes in [10_000, 100_000] {
        let mut panel = Panel::new(nodes, 800.0, false);
        for index in 0..RULES {
            assert_eq!(panel.height(index), 24.0, "{nodes}: row {index}");
        }
        let counters = panel.resize(790.0);
        let queries = Queries::from(counters);
        // The panel's inline size moved; its hundred rules read it.
        assert_eq!(queries.size_changes, 1, "{nodes}: {queries:?}");
        assert_eq!(queries.evaluated, RULES as usize, "{nodes}: {queries:?}");
        assert_eq!(queries.unchanged, RULES as usize);
        assert_eq!(queries.changed, 0);
        assert_eq!(counters.layout_full_document_fallbacks, 0);
        costs.push((
            queries,
            counters.layout_frontier_seeds,
            counters.layout_frontier_nodes_measure,
            counters.layout_frontier_nodes_placement,
            counters.layout_measure_nodes,
            counters.layout_placement_nodes,
            counters.layout_dependency_edges_visited,
        ));
    }
    // Ninety thousand more nodes outside the panel cost nothing.
    assert_eq!(costs[0], costs[1]);
}

#[test]
fn issue265_gate_b_a_resize_storm_in_one_bucket_changes_nothing() {
    let mut panel = Panel::new(1_000, 800.0, true);
    let footprint = panel.footprint();
    let mut total = Queries::default();
    for step in 0..240u32 {
        // Every width at or above 500: one bucket for every rule.
        let width = 510.0 + ((step * 37) % 290) as f32;
        let counters = panel.resize(width);
        let queries = Queries::from(counters);
        assert_eq!(queries.changed, 0, "step {step}: {queries:?}");
        assert_eq!(queries.downstream, 0, "step {step}: {queries:?}");
        total.evaluated += queries.evaluated;
        total.size_changes += queries.size_changes;
        // The panel lays out at its new width; no row measures.
        assert!(
            counters.layout_measure_nodes <= 1,
            "step {step}: {} measured",
            counters.layout_measure_nodes
        );
    }
    assert_eq!(total.size_changes, 240);
    assert_eq!(total.evaluated, 240 * RULES as usize);
    // Gate E: the index is what was registered, not what the frames did.
    assert_eq!(panel.footprint(), footprint);
    assert_eq!(footprint, (RULES as usize, RULES as usize, 1, 0));
}

#[test]
fn issue265_gate_c_one_breakpoint_changes_only_the_rules_it_breaks() {
    let mut panel = Panel::new(1_000, 520.0, true);
    let counters = panel.resize(480.0);
    let queries = Queries::from(counters);
    let breaking = (0..RULES)
        .filter(|index| breakpoint(*index) == 500.0)
        .count();
    assert_eq!(breaking, 10);
    assert_eq!(queries.evaluated, RULES as usize, "{queries:?}");
    assert_eq!(queries.changed, breaking, "{queries:?}");
    assert_eq!(queries.unchanged, RULES as usize - breaking);
    assert_eq!(queries.downstream, breaking, "one seed per variant");
    assert_eq!(queries.rounds, 1);
    assert_eq!(queries.fallbacks, 0);
    for index in 0..RULES {
        let expected = if breakpoint(index) == 500.0 {
            36.0
        } else {
            24.0
        };
        assert_eq!(panel.height(index), expected, "row {index}");
    }
    // The ten rows measure; the ninety fixed ones below them only move.
    assert!(
        counters.layout_measure_nodes <= breaking + 2,
        "{} measured",
        counters.layout_measure_nodes
    );
    // The frontier is the ten rows and the path from the panel to the root:
    // the panel, the page and the document.
    assert!(
        counters.layout_frontier_nodes_measure <= breaking + 3,
        "{} in the measure frontier",
        counters.layout_frontier_nodes_measure
    );
    // History of the frame that changed: the ten buckets they left. The
    // next frame starts over.
    assert_eq!(panel.footprint().3, breaking);
    let counters = panel.resize(470.0);
    assert_eq!(Queries::from(counters).changed, 0);
    assert_eq!(panel.footprint().3, 0);

    // And back.
    let queries = Queries::from(panel.resize(520.0));
    assert_eq!(queries.changed, breaking);
    for index in 0..RULES {
        assert_eq!(panel.height(index), 24.0, "row {index}");
    }
}

const DEPTH: u64 = 8;
const LEAVES: u64 = 50;

fn level(depth: u64) -> StableNodeId {
    node(100 + depth)
}

fn padded(width: LengthSpec, padding: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(width),
        padding: Some(LengthSpec::Px(padding)),
        direction: Some(FlexDirection::Column),
        flex_shrink: Some(0.0),
        ..LayoutStyle::default()
    }
}

/// The outermost container: `width` wide, its height its own.
fn outer(width: f32) -> LayoutStyle {
    LayoutStyle {
        height: Some(LengthSpec::Px(4000.0)),
        ..padded(LengthSpec::Px(width), 8.0)
    }
}

impl Panel {
    /// Eight containers nested at the end of a page of about `nodes` nodes,
    /// each filling its parent and holding the next and fifty fixed leaves;
    /// container `i > 0` tightens its padding below 400 px of its parent. The
    /// outermost keeps its height, so nothing outside it moves.
    pub(super) fn nest(width: f32, nodes: u64, guarded: bool) -> Self {
        let document = DocumentId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        page_with_filler(&mut queue, document, nodes, 20_000_000);
        element(
            &mut queue,
            document,
            node(2),
            level(0),
            styled(outer(width)),
        );
        let mut leaf = 1_000u64;
        for depth in 0..DEPTH {
            if depth > 0 {
                element(
                    &mut queue,
                    document,
                    level(depth - 1),
                    level(depth),
                    styled(padded(LengthSpec::Fill, 8.0)),
                );
                queue.set_responsive(
                    level(depth),
                    Some(Arc::new(
                        ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
                            .below(400.0, |layout| layout.padding = Some(LengthSpec::Px(2.0))),
                    )),
                );
            }
            for _ in 0..LEAVES {
                element(
                    &mut queue,
                    document,
                    level(depth),
                    node(leaf),
                    styled(fixed(20.0, 4.0)),
                );
                leaf += 1;
            }
        }
        Self::laid_out(document, queue, guarded, level(0), outer)
    }

    fn buckets(&self) -> Vec<Option<usize>> {
        (1..DEPTH)
            .map(|depth| self.context.world().responsive_bucket(level(depth)))
            .collect()
    }
}

#[test]
fn issue265_gate_d_nested_containers_converge_along_their_chain() {
    let mut nest = Panel::nest(800.0, 1_000, true);
    assert_eq!(nest.buckets(), vec![Some(1); 7]);
    let counters = nest.resize(300.0);
    let queries = Queries::from(counters);
    // Seven rules, one per level; no leaf has one to evaluate.
    assert!(queries.evaluated >= 7, "{queries:?}");
    assert!(queries.evaluated <= 7 * 4, "{queries:?}");
    assert_eq!(queries.changed, 7, "{queries:?}");
    assert!(queries.rounds <= 4, "{queries:?}");
    assert_eq!(queries.fallbacks, 0, "{queries:?}");
    assert_eq!(nest.buckets(), vec![Some(0); 7]);
    // The same page laid out from nothing at 300 px.
    let mut cold = Panel::nest(300.0, 1_000, true);
    assert_eq!(cold.buckets(), vec![Some(0); 7]);
    assert_matches_cold(&mut nest.context, &mut cold.context, nest.document);

    // Still below every breakpoint: rules are read, nothing changes.
    let queries = Queries::from(nest.resize(320.0));
    assert_eq!(queries.changed, 0, "{queries:?}");
    assert_eq!(queries.downstream, 0, "{queries:?}");
}

#[test]
fn issue265_a_rule_that_moves_its_own_container_settles_in_its_frame() {
    // A box sized by its one child, which is wide while the box is narrow
    // and narrow while it is wide: the rule cannot be satisfied.
    let run = || {
        let document = DocumentId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document, NodeKind::Document);
        // A row: the box in it is as wide as what it holds.
        element(
            &mut queue,
            document,
            node(1),
            node(2),
            styled(LayoutStyle {
                direction: Some(FlexDirection::Row),
                ..column(Some(1200.0), Some(800.0))
            }),
        );
        element(
            &mut queue,
            document,
            node(2),
            node(3),
            styled(column(None, None)),
        );
        element(
            &mut queue,
            document,
            node(3),
            node(4),
            styled(fixed(100.0, 20.0)),
        );
        queue.set_responsive(
            node(4),
            Some(Arc::new(
                ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
                    .below(200.0, |layout| layout.width = Some(LengthSpec::Px(300.0))),
            )),
        );
        let mut context = AppContext::new();
        context.compat_world_mut().commit(queue).unwrap();
        let mut shaper = bundled_face_shaper();
        let counters = product_frame(&mut context, document, viewport(), &mut shaper);
        let passes = context.layout_invocations();
        product_frame(&mut context, document, viewport(), &mut shaper);
        assert_eq!(
            context.layout_invocations(),
            passes,
            "the next frame is idle"
        );
        let world = context.world();
        (
            Queries::from(counters),
            world.responsive_bucket(node(4)),
            world.layout_box(node(3)),
            world.layout_box(node(4)),
        )
    };
    let (queries, bucket, container, child) = run();
    assert!(queries.fallbacks >= 1, "{queries:?}");
    assert!(queries.rounds <= 4, "{queries:?}");
    assert!(bucket.is_some());
    // The same inputs settle the same way.
    assert_eq!(run(), (queries, bucket, container, child));
}

#[test]
fn issue265_a_variant_hides_optional_chrome_and_style_follows_it() {
    // Below 300 px the panel drops its optional row: the row's computed
    // style, not only its box, follows the variant.
    let mut panel = Panel::new(1_000, 320.0, true);
    let chrome = row(RULES - 1);
    let mut queue = MutationQueue::new();
    queue.set_responsive(
        chrome,
        Some(Arc::new(
            ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
                .below(300.0, |layout| layout.hidden = true),
        )),
    );
    panel.context.commit_mutations(queue).unwrap();
    panel.frame();
    let visible = |panel: &Panel| {
        panel
            .context
            .world()
            .computed_style(chrome)
            .unwrap()
            .visible
    };
    assert!(visible(&panel));
    assert_eq!(panel.height(RULES - 1), 24.0);

    let queries = Queries::from(panel.resize(280.0));
    assert!(queries.changed >= 1, "{queries:?}");
    assert!(!visible(&panel), "the hidden variant hides the row");
    assert_eq!(panel.height(RULES - 1), 0.0, "and omits its box");

    panel.resize(320.0);
    assert!(visible(&panel));
    assert_eq!(panel.height(RULES - 1), 24.0);
}

#[test]
fn issue265_a_rule_follows_its_node_to_a_new_parent_and_can_be_dropped() {
    let mut panel = Panel::new(1_000, 520.0, true);
    let moved = row(0);
    assert_eq!(panel.height(0), 24.0, "520 px is at its breakpoint");
    // A narrow box beside the panel: the row reads it once it moves there.
    let narrow = node(PANEL - 1);
    let mut queue = MutationQueue::new();
    element(
        &mut queue,
        DocumentId::new(1).unwrap(),
        node(2),
        narrow,
        styled(column(Some(300.0), Some(100.0))),
    );
    panel.context.commit_mutations(queue).unwrap();
    panel.frame();
    let mut queue = MutationQueue::new();
    queue.insert(narrow, moved, None);
    panel.context.commit_mutations(queue).unwrap();
    panel.frame();
    assert_eq!(panel.height(0), 36.0, "300 px is below its breakpoint");
    assert_eq!(panel.context.world().responsive_bucket(moved), Some(0));

    // Without its rule the row is its own layout again.
    let mut queue = MutationQueue::new();
    queue.set_responsive(moved, None);
    panel.context.commit_mutations(queue).unwrap();
    panel.frame();
    assert_eq!(panel.height(0), 24.0);
    assert_eq!(panel.context.world().responsive_bucket(moved), None);
    assert_eq!(panel.footprint().0, RULES as usize - 1);
}

/// A variant made of data, as CSS writes one: the fields it changes and their
/// values. A paint-only variant paints and lays nothing out; the node keeps
/// the style it was authored with, and a later style write starts from that
/// and keeps the variant over it.
#[test]
fn issue265_a_data_variant_paints_without_layout_and_keeps_the_authored_style() {
    let mut panel = Panel::new(1_000, 320.0, true);
    let painted = row(RULES - 1);
    let red = [1.0, 0.0, 0.0, 1.0];
    let authored = panel.context.world().node_style(painted).unwrap().clone();
    let redder = LayoutStyle {
        background: Some(red),
        ..(*authored.layout).clone()
    };
    let rule = || {
        Arc::new(
            ResponsiveRule::from_buckets(
                ResponsiveContainer::Parent,
                ResponsiveAxis::Width,
                vec![300.0],
                vec![StyleVariant::between(&authored.layout, &redder), None],
            )
            .unwrap(),
        )
    };
    let installed = rule();
    let mut queue = MutationQueue::new();
    queue.set_responsive(painted, Some(Arc::clone(&installed)));
    panel.context.commit_mutations(queue).unwrap();
    panel.frame();
    let background = |panel: &Panel| {
        panel
            .context
            .world()
            .computed_style(painted)
            .unwrap()
            .background
    };
    assert_eq!(background(&panel), None);

    let queries = Queries::from(panel.resize(280.0));
    assert!(queries.changed >= 1, "{queries:?}");
    assert_eq!(background(&panel), Some(red), "the variant paints");
    assert_eq!(
        queries.downstream, 0,
        "a paint-only variant seeds no layout"
    );
    let world = panel.context.world();
    assert_eq!(
        world.node_style(painted),
        Some(&authored),
        "authored style kept"
    );
    // The same rule sent again is the rule it has.
    let mut queue = MutationQueue::new();
    queue.set_responsive(painted, Some(rule()));
    panel.context.commit_mutations(queue).unwrap();
    assert!(Arc::ptr_eq(
        panel.context.world().responsive_rule(painted).unwrap(),
        &installed
    ));

    // A write while in the bucket starts from the authored style; the
    // variant stays written over it.
    let taller = styled(fixed(200.0, 30.0));
    let mut queue = MutationQueue::new();
    queue.set_style(painted, taller.clone());
    panel.context.commit_mutations(queue).unwrap();
    panel.frame();
    assert_eq!(panel.height(RULES - 1), 30.0);
    assert_eq!(background(&panel), Some(red));
    assert_eq!(panel.context.world().node_style(painted), Some(&taller));

    panel.resize(320.0);
    assert_eq!(background(&panel), None);
    assert_eq!(panel.height(RULES - 1), 30.0);
}

/// `container-type` and `container-name`: a rule reads the nearest box above
/// it that answers its query, past boxes that do not, and keeps its authored
/// style while none does. A box that becomes or stops being a query
/// container is found, or lost, by the rules below it.
#[test]
fn issue265_a_rule_reads_the_nearest_query_container_by_name() {
    let document = DocumentId::new(1).unwrap();
    let card = node(PANEL + 500);
    let wrapper = node(PANEL + 501);
    let named = node(PANEL + 502);
    let unnamed = node(PANEL + 503);
    let elsewhere = node(PANEL + 504);
    let card_at = |width: f32| LayoutStyle {
        container_type: ContainerType::InlineSize,
        container_name: vec!["card".into()],
        ..column(Some(width), Some(400.0))
    };
    let mut queue = MutationQueue::new();
    page_with_filler(&mut queue, document, 1_000, 10);
    element(&mut queue, document, node(2), card, styled(card_at(320.0)));
    element(
        &mut queue,
        document,
        card,
        wrapper,
        styled(column(None, None)),
    );
    let tall = fixed(100.0, 24.0);
    let taller = fixed(100.0, 36.0);
    for (id, name, axis) in [
        (named, Some("card"), ResponsiveAxis::Width),
        (unnamed, None, ResponsiveAxis::Inline),
        (elsewhere, Some("sidebar"), ResponsiveAxis::Width),
    ] {
        element(&mut queue, document, wrapper, id, styled(tall.clone()));
        queue.set_responsive(
            id,
            Some(Arc::new(
                ResponsiveRule::from_buckets(
                    ResponsiveContainer::Nearest {
                        name: name.map(String::from),
                    },
                    axis,
                    vec![300.0],
                    vec![StyleVariant::between(&tall, &taller), None],
                )
                .unwrap(),
            )),
        );
    }
    let mut panel = Panel::laid_out(document, queue, true, card, card_at);
    let height = |panel: &Panel, id| panel.context.world().layout_box(id).unwrap().height;
    for id in [named, unnamed, elsewhere] {
        assert_eq!(height(&panel, id), 24.0);
    }

    panel.resize(280.0);
    assert_eq!(
        height(&panel, named),
        36.0,
        "found by name, past the wrapper"
    );
    assert_eq!(height(&panel, unnamed), 36.0, "an unnamed query takes any");
    assert_eq!(
        height(&panel, elsewhere),
        24.0,
        "no box answers to its name"
    );
    assert_eq!(panel.context.world().responsive_bucket(elsewhere), None);

    // The card stops being a query container: its rules keep their
    // authored style.
    let mut queue = MutationQueue::new();
    queue.set_style(card, styled(column(Some(280.0), Some(400.0))));
    panel.context.commit_mutations(queue).unwrap();
    panel.frame();
    assert_eq!(height(&panel, named), 24.0);
    assert_eq!(panel.context.world().responsive_bucket(named), None);

    // The wrapper becomes one, by that name: it is the nearest now.
    let mut queue = MutationQueue::new();
    queue.set_style(
        wrapper,
        styled(LayoutStyle {
            container_type: ContainerType::Size,
            container_name: vec!["card".into()],
            ..column(None, None)
        }),
    );
    panel.context.commit_mutations(queue).unwrap();
    panel.frame();
    assert_eq!(height(&panel, named), 36.0, "280 px wide: below 300");
    assert_eq!(height(&panel, elsewhere), 24.0);
}

/// A node in a bucket is the node authored with the variant written over its
/// style: the same computed style, the same box.
#[test]
fn issue265_a_node_in_a_bucket_matches_the_node_authored_that_way() {
    let mut panel = Panel::new(1_000, 280.0, true);
    let base = fixed(200.0, 24.0);
    let narrow = LayoutStyle {
        height: Some(LengthSpec::Px(40.0)),
        padding: Some(LengthSpec::Px(6.0)),
        background: Some([0.2, 0.4, 0.6, 1.0]),
        opacity: Some(0.5),
        font_size: Some(18.0),
        ..base.clone()
    };
    let following = row(RULES - 1);
    let authored = row(RULES - 2);
    let mut queue = MutationQueue::new();
    queue.set_responsive(
        following,
        Some(Arc::new(
            ResponsiveRule::from_buckets(
                ResponsiveContainer::Parent,
                ResponsiveAxis::Inline,
                vec![300.0],
                vec![StyleVariant::between(&base, &narrow), None],
            )
            .unwrap(),
        )),
    );
    queue.set_responsive(authored, None);
    queue.set_style(authored, styled(narrow));
    panel.context.commit_mutations(queue).unwrap();
    panel.frame();
    let world = panel.context.world();
    assert_eq!(
        world.computed_style(following),
        world.computed_style(authored)
    );
    let size = |id| {
        let layout = world.layout_box(id).unwrap();
        (layout.width, layout.height)
    };
    assert_eq!(size(following), size(authored));
    assert_eq!(size(following), (200.0, 40.0));
}
